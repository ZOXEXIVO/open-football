//! Evidence-tier classification and the calibration it drives: soft caps,
//! team-result context damping, and the low-engagement penalty. All from a
//! single stat-line read so the three signals can't drift out of sync.

use super::{EvidenceTier, RatingContext, RatingMath};
use crate::PlayerFieldPositionGroup;

impl<'a> RatingContext<'a> {
    /// Classify the player's stat-line into an `EvidenceTier`. The
    /// classification is consumed by `apply_soft_caps_for`,
    /// `context_credit_factor`, and `engagement_penalty` — a single
    /// shared pivot so the three signals can't drift out of sync (a
    /// "passenger" for one must be a "passenger" for all three).
    ///
    /// Pure stat-line read: never inspects ability, CA, or any hidden
    /// flag. Tiered exactly as the legacy `apply_soft_caps` did so the
    /// upper-band behaviour is unchanged — the *passenger* tier is what
    /// the rebalance tightens, with the cap moving down to +0.20 (was
    /// +0.65) and engagement-penalty + context-damping firing on top.
    pub(super) fn evidence_tier(&self) -> EvidenceTier {
        let s = self.stats;
        let z = s.zone_stats;

        let goals = s.goals as u32;
        let assists = s.assists as u32;
        let major_contrib = goals + assists;
        let shot_or_chance =
            (s.shots_total + s.key_passes + s.passes_into_box + s.successful_dribbles) as u32;
        let defensive_volume =
            (s.tackles + s.interceptions + s.blocks + s.clearances + s.successful_pressures) as u32;
        let gk_volume = s.saves as u32 + z.gk_command_actions as u32;
        let total_volume = shot_or_chance + defensive_volume + gk_volume + (assists * 2);

        let minutes = s.minutes_played;
        let any_event =
            goals > 0 || s.errors_leading_to_goal > 0 || s.red_cards > 0 || s.own_goals > 0;

        // ──── Direct scoring events take precedence ────
        if goals >= 3 || major_contrib >= 3 {
            return EvidenceTier::HatTrick;
        }
        if goals >= 2 || major_contrib >= 2 {
            return EvidenceTier::TwoGoals;
        }
        if goals == 1 && total_volume < 6 {
            return EvidenceTier::OneGoalLowVolume;
        }
        if minutes < 30 && !any_event {
            return EvidenceTier::QuietCameo;
        }

        // ──── Non-G/A starters: evidence-based tier ladder ────
        if major_contrib == 0 && minutes >= 30 {
            let zone_impact = (z.tackles_own_box
                + z.tackles_own_six_yard
                + z.interceptions_own_box
                + z.interceptions_own_six_yard
                + z.blocks_own_box
                + z.blocks_own_six_yard
                + z.clearances_own_box
                + z.clearances_own_six_yard
                + z.pressures_won_final_third) as u32;
            let creative_strong = s
                .key_passes
                .max(s.shots_on_target)
                .max(s.successful_dribbles) as u32;
            let creative_any = (s.key_passes
                + s.passes_into_box
                + s.successful_dribbles
                + s.crosses_completed
                + s.shots_on_target
                + s.progressive_passes
                + s.progressive_carries) as u32;

            if self.is_goalkeeper() {
                let gk_busy = s.saves >= 4 || z.gk_command_actions >= 3 || s.xg_prevented > 0.5;
                if gk_busy {
                    return EvidenceTier::GkBusy;
                }
                if s.saves >= 2 || z.gk_command_actions >= 1 || s.xg_prevented > 0.0 {
                    return EvidenceTier::GkModest;
                }
                return EvidenceTier::GkPassenger;
            }

            // Forward-specific ladder: routine creative or dribbling
            // volume isn't enough to promote a goalless forward into
            // Strong. The bar is "direct goal threat plus a real
            // creative or dribbling footprint" — SOT≥2 combined with
            // KP+PB≥3 or drib≥3. Anything less caps at Modest, and a
            // completely flat attacking line drops to Passenger.
            //
            // This is the upstream half of the spec's "creative
            // forward without G/A doesn't look like a good performer":
            // the cap is tighter than the generic Strong cap and the
            // context_credit_factor damping below halves the
            // team-result lift.
            if self.pos == PlayerFieldPositionGroup::Forward {
                let kp_pb = (s.key_passes + s.passes_into_box) as u32;
                let drib = s.successful_dribbles as u32;
                let sot = s.shots_on_target as u32;
                let attacking_strong = sot >= 2 && (kp_pb >= 3 || drib >= 3);
                if attacking_strong {
                    return EvidenceTier::Strong;
                }
                let attacking_modest =
                    sot >= 1 || kp_pb >= 2 || drib >= 2 || s.xg >= 0.4 || s.crosses_completed >= 2;
                if attacking_modest {
                    return EvidenceTier::Modest;
                }
                return EvidenceTier::Passenger;
            }

            let big_def = zone_impact + (s.saves as u32) / 2;
            // Routine defensive footprint — the workhorse signal. A CB
            // / fullback / DM who put in 3+ honest defensive actions
            // (tackles + interceptions + blocks + clearances + won
            // pressures) is doing the job and shouldn't collapse to
            // Passenger just because the engine didn't happen to emit a
            // box-zone tag or a cross-completion event for them.
            let routine_def =
                (s.tackles + s.interceptions + s.blocks + s.clearances + s.successful_pressures)
                    as u32;
            // High-volume accurate passing — the midfielder recycler
            // signal. A DM who turned over 30+ completed passes at
            // 75%+ accuracy did real work even without progressing the
            // ball or creating a chance.
            let pct = if s.passes_attempted > 0 {
                s.passes_completed as f32 / s.passes_attempted as f32
            } else {
                0.0
            };
            let high_pass_volume = s.passes_completed >= 30 && pct >= 0.75;
            let big_pass_volume = s.passes_completed >= 50 && pct >= 0.80;

            if zone_impact >= 2
                || creative_strong >= 2
                || big_def >= 3
                || routine_def >= 7
                || big_pass_volume
                || s.crosses_completed >= 3
                || (s.key_passes + s.passes_into_box) >= 4
            {
                return EvidenceTier::Strong;
            }
            let creative_decisive = (s.key_passes
                + s.passes_into_box
                + s.successful_dribbles
                + s.crosses_completed
                + s.shots_on_target) as u32;
            if zone_impact >= 1
                || routine_def >= 3
                || high_pass_volume
                || creative_decisive >= 1
                || creative_any >= 3
            {
                return EvidenceTier::Modest;
            }
            return EvidenceTier::Passenger;
        }

        if minutes >= 60 && major_contrib == 0 && total_volume < 5 {
            return EvidenceTier::AnonymousStarter;
        }

        EvidenceTier::Uncapped
    }

    /// Apply the tier-appropriate soft cap to the cumulative positive
    /// delta. Caps preserved from the legacy `apply_soft_caps` so the
    /// strong-evidence / multi-goal tiers are unchanged. The passenger
    /// cap tightens to +0.20 (was +0.65) — combined with
    /// `engagement_penalty` and `context_credit_factor` it pulls a
    /// genuinely anonymous shift below 6.0 instead of pinning it at
    /// 6.3-6.6 like the legacy version did.
    pub(super) fn apply_soft_caps_for(&self, positive_delta: f32, tier: EvidenceTier) -> f32 {
        match tier {
            EvidenceTier::HatTrick => positive_delta,
            EvidenceTier::TwoGoals => RatingMath::soft_cap(positive_delta, 2.3, 0.45),
            EvidenceTier::OneGoalLowVolume => RatingMath::soft_cap(positive_delta, 1.6, 0.45),
            EvidenceTier::QuietCameo => RatingMath::soft_cap(positive_delta, 0.7, 0.25),
            EvidenceTier::Strong => RatingMath::soft_cap(positive_delta, 1.3, 0.40),
            EvidenceTier::Modest => {
                // Unified Modest cap at 0.95 for all outfield positions
                // (was forward-specific 0.80 → 0.65 → 0.80). The
                // forward-specific tighter cap was added to prevent
                // goalless forwards from drifting to 6.9+ season
                // averages; but the soft_cap slope 0.30 above the cap
                // already provides natural compression, and the broader
                // forward over-tightening of the prior round (ARE +
                // wasted-xG + context damping) is what was actually
                // doing that work. Restoring 0.95 lets an active
                // goalless forward's busy routine line register, while
                // ARE still drags the rating below the good band.
                RatingMath::soft_cap(positive_delta, 0.95, 0.30)
            }
            // Tightened: passenger routine volume alone is severely
            // bounded. The engagement penalty + context damping handle
            // the rest of the "showed up, did nothing" signal.
            EvidenceTier::Passenger => RatingMath::soft_cap(positive_delta, 0.20, 0.15),
            EvidenceTier::AnonymousStarter => RatingMath::soft_cap(positive_delta, 1.1, 0.25),
            // GK caps: previous tightening (0.75 / 1.30 / 0.50) was
            // calibrated against synthetic 7.x averages but went too
            // far — the engine emits modest shot volume so most TOP-GK
            // shifts land in GkModest, and combined with the halved
            // GkPassenger context factor a Maignan / Courtois /
            // Unai Simón quality keeper averaged ~6.3 over a season
            // (well below the WhoScored 6.8-7.0 reference band).
            // Caps lifted back toward an honest "did the job" ceiling
            // while keeping the second-tier-keeper guard from the
            // 2026-04 calibration pass.
            // GkBusy cap lifted 1.45 → 1.52 (FM-parity follow-up): the
            // clean-sheet credit lifts pushed a routine 3-save shutout
            // win to ~7.45, a dead heat with the heroic 8-save loss.
            // Keeping the barrage keeper's headroom above the
            // well-protected shutout preserves the "earned vs
            // organised" ordering FM shows.
            EvidenceTier::GkBusy => RatingMath::soft_cap(positive_delta, 1.52, 0.35),
            EvidenceTier::GkModest => RatingMath::soft_cap(positive_delta, 0.92, 0.30),
            // GkPassenger cap lifted 0.62 → 0.70 (FM-parity season
            // calibration): with the quiet-shutout credit at 0.25 the
            // old cap clipped a 1-save clean sheet's honest routine sum,
            // re-flattening exactly the matches the GK season band
            // needed back. Still well under GkModest, so an untested
            // keeper can't ride saves they never made.
            EvidenceTier::GkPassenger => RatingMath::soft_cap(positive_delta, 0.70, 0.20),
            EvidenceTier::Uncapped => positive_delta,
        }
    }

    /// Multiplier applied to win / clean-sheet bonuses. A passenger
    /// (no decisive evidence) only gets half the team-result credit —
    /// they were *on* the winning side but didn't contribute to the
    /// win. Real punditry distinguishes "rode the team's wave" from
    /// "earned the result"; this is the smallest stat-line proxy.
    ///
    /// Forwards get an additional damping when they have no G/A.
    /// The spec's principle: a striker who wasn't part of any goal
    /// has little claim on the team-result lift, regardless of how
    /// the rest of the side performed. The damping is asymmetric
    /// (positive context only) — being on the losing side still
    /// hits a goalless forward at full strength.
    pub(super) fn context_credit_factor(&self, tier: EvidenceTier) -> f32 {
        let base: f32 = match tier {
            EvidenceTier::Passenger | EvidenceTier::AnonymousStarter => 0.50,
            // Untested keeper is not "riding the wave" the way an
            // outfield passenger is. A GK who organised the back four
            // through a clean sheet — even without making a save —
            // is doing the job. Halving their CS/result credit to 0.50
            // double-penalised them on top of the GkPassenger cap and
            // pulled TOP-GK season averages into the 6.2–6.4 band.
            // 0.85 (lifted from 0.80 in the FM-parity season
            // calibration) keeps a meaningful "didn't make a single
            // decisive intervention" discount without collapsing the
            // 12-clean-sheet season the credit exists to reward.
            EvidenceTier::GkPassenger => 0.85,
            _ => 1.0,
        };
        if self.pos == PlayerFieldPositionGroup::Forward
            && self.stats.goals == 0
            && self.stats.assists == 0
            && self.stats.minutes_played >= 30
        {
            // Cap the goalless-forward damping at 0.65 (lifted from
            // 0.55 in the FM-parity season calibration). A 16-goal
            // striker spends ~60% of his season goalless; with the win
            // credit at 0.16 the 0.55 cap kept those matches reading
            // poor instead of ordinary and dragged the season average
            // below the believable band. 0.65 still meaningfully
            // discounts a goalless forward riding the team's win (no
            // full credit), and a true passenger stays at the 0.50
            // tier base below this cap.
            return base.min(0.65);
        }
        base
    }

    /// Engagement penalty for low-touch starters. Pure stat-line
    /// "anonymous shift" signal: a 60+ min outfield player whose total
    /// touches-per-minute fall well below the position-typical floor
    /// is observably uninvolved in the match. Returns a non-positive
    /// value (0 if engagement is healthy).
    ///
    /// Position floors are conservative — set just below the typical
    /// engaged starter so an *ordinary* routine player (the existing
    /// 6.0-6.7 band) doesn't trip the gate; only the genuine
    /// "didn't belong" case does.
    pub(super) fn engagement_penalty(&self) -> f32 {
        let s = self.stats;
        let minutes = s.minutes_played;
        if minutes < 60 {
            return 0.0;
        }
        if self.is_goalkeeper() {
            return 0.0;
        }
        // Total visible touches the engine emitted for this player.
        // Includes attempted dribbles whether successful or not — an
        // attempt is still a touch. Excludes pressures (those are
        // sustained-proximity events rather than discrete touches).
        let attempted_dribbles = s.attempted_dribbles as u32;
        let total_touches = (s.passes_attempted as u32)
            + (s.shots_total as u32)
            + (s.tackles as u32)
            + (s.interceptions as u32)
            + (s.blocks as u32)
            + (s.clearances as u32)
            + attempted_dribbles
            + (s.crosses_attempted as u32);
        // Zero-touches: treat as a synthetic / legacy stats bundle (the
        // engine always emits at least some events for a 60+ min
        // outfield starter). Leaving the rating at the unmodified
        // baseline so the "neutral player" reference invariant holds —
        // this is the explicit anchor that test fixtures use.
        if total_touches == 0 {
            return 0.0;
        }
        let touches_per_min = total_touches as f32 / (minutes as f32).max(1.0);
        // Position floor — chosen so the existing "ordinary routine"
        // archetypes (mid 40-45 passes / 90, def 30-35 passes / 90 with
        // a handful of defensive actions) sit at or above it. Below
        // the floor the penalty ramps up; well below saturates.
        let floor = match self.pos {
            PlayerFieldPositionGroup::Defender => 0.40,
            PlayerFieldPositionGroup::Midfielder => 0.50,
            // Forwards are exempt: the forward-specific
            // `attacking_role_expectation` drag already captures the
            // "anonymous shift" signal for a striker. Stacking the
            // engagement penalty on top of ARE double-bit a goalless
            // forward in a tough CL-away match, collapsing the rating
            // to ~5.2-5.4. ARE alone scales the drag with the
            // attacking footprint; that is the right primary signal.
            PlayerFieldPositionGroup::Forward => return 0.0,
            _ => return 0.0,
        };
        if touches_per_min >= floor {
            return 0.0;
        }
        // Smooth ramp: zero at the floor, grows as engagement drops.
        // Normalised on the floor so a player at 50% of the floor lands
        // mid-penalty, and a near-zero-touch starter saturates near the
        // bottom. Coefficient calibrated so the symptom case (a
        // low-skill starter at a possession-dominant club with ~25
        // passes / 90 ≈ 0.28 touches/min) lands around −0.7 to −0.9,
        // pulling 6.3 into the 5.0–5.5 "clear underperformance" band.
        let shortfall = (floor - touches_per_min) / floor;
        -RatingMath::sat(shortfall, 0.5) * 1.5
    }
}

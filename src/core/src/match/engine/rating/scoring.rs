//! Attacking-output rating components: scoring events, shooting threat,
//! chance creation, ball progression, retention, and touch quality.
//!
//! Each method returns a small signed value in "rating units"; magnitudes
//! are deliberately modest — they get multiplied by the position weight
//! (<= ~1.1) before contributing to the rating.

use super::{RatingContext, RatingMath};
use crate::PlayerFieldPositionGroup;

impl<'a> RatingContext<'a> {
    /// Direct decisive-event impact: goals + assists + clinical (over-xG)
    /// + decisive (the contribution won the match). Saturates so a
    /// hat-trick or multi-assist game is rewarded but not 3× a single
    /// event.
    ///
    /// Assists live here (not in creation()) because they are the same
    /// kind of decisive moment a goal is — punditry treats them as the
    /// primary creator's output. Routing the credit through the
    /// `scoring` profile weight makes the per-position dial coherent:
    /// the same dial that pays a striker for finishing pays a
    /// midfielder for setting one up.
    pub(super) fn scoring_event(&self) -> f32 {
        let s = self.stats;
        let g = s.goals as f32;
        let a = s.assists as f32;
        if g <= 0.0 && a <= 0.0 {
            return 0.0;
        }
        // sat(1, 1.6) ≈ 0.46; sat(2) ≈ 0.71; sat(3) ≈ 0.85.
        // Coefficient lifted 2.80 → 2.95 in the FM-parity season
        // calibration (see season_tests.rs): a 15-21 goal season has to
        // accumulate to 6.8-7.2 against ~60% goalless matches, and the
        // goal event is the only lever that lifts scorers without
        // lifting passengers. The OneGoalLowVolume soft cap absorbs
        // most of the single-match effect, so per-match bands move by
        // hundredths while the season aggregate gains the difference.
        let goal_raw = RatingMath::sat(g, 1.6) * 2.95;
        // Assists ≈ 55% of a goal — decisive but not as decisive as
        // putting it in. Lifted 1.55 → 1.65 in tandem for the same
        // reason (assist-match rating proportional to goal-match).
        let assist_raw = RatingMath::sat(a, 1.6) * 1.65;
        let raw = goal_raw + assist_raw;

        // Clinical-finisher bonus: goals beyond xG → premium for
        // converting tougher chances or being lethal in front of goal.
        let over = (g - s.xg).max(0.0);
        let clinical = RatingMath::sat(over, 1.0) * 0.15;

        // Decisive-event nudge — the goal or assist mattered to the
        // scoreline.
        let decisive = if self.team_goals > self.opponent_goals {
            0.08
        } else {
            0.0
        };

        raw + clinical + decisive
    }

    /// Shooting threat: xG generated, shots on target, with a wasted-
    /// xG penalty for high-quality chances missed and a shot-spam
    /// penalty for high-volume low-quality attempts.
    ///
    /// Forwards face a stricter calibration on the negative side: the
    /// wasted-xG threshold drops to 0.40 and the per-unit drag is
    /// heavier, and the no-SOT spam drag is heavier too. A forward
    /// who shoots without threatening the goal is observably failing
    /// at their primary role.
    pub(super) fn shooting(&self) -> f32 {
        let s = self.stats;
        if s.shots_total == 0 && s.xg <= 0.0 {
            return 0.0;
        }

        let is_forward = self.pos == PlayerFieldPositionGroup::Forward;

        // xG credit lifted 0.38 → 0.46 (FM-parity season calibration) —
        // chance creation is the active forward's secondary positive
        // signal when goals don't come. A 16-goal striker's blank
        // matches still carry 0.4-0.8 xG; those shifts have to read
        // ordinary (6.3-6.6), not poor, for the season band to hold.
        let xg_value = RatingMath::sat(s.xg, 1.8) * 0.46;
        // SoT credit lifted 0.34 → 0.42 in the same pass — putting a
        // shot on target IS the headline forward action even without
        // scoring. Together with the xG lift this moves an active
        // goalless shift (~2 SOT, 0.5 xG) by ≈ +0.07 while a no-SOT
        // passenger gains nothing.
        let sot_value = RatingMath::sat(s.shots_on_target as f32, 2.5) * 0.42;
        let mut shooting = xg_value + sot_value;

        // Wasted high xG: created premium chances, scored nothing.
        // Forwards: lower threshold (0.40) + heavier coefficient — a
        // striker squandering decent chances is the canonical bad
        // forward shift. Other positions: a stray 0.6+ xG miss still
        // drags, but proportionally to how unusual it is.
        if s.goals == 0 {
            // Forward threshold 0.75, coef 0.35 — final calibration
            // (was 0.40/0.90 → 0.55/0.70 → 0.55/0.50 → 0.75/0.35). The
            // dev_match 200-match benchmark showed forwards' goalless
            // tier at 5.65 average vs the 6.2 real-football reference;
            // the wasted-xG drag in shooting was firing for ~half of
            // goalless matches at any xg > 0.55. Reserve this drag for
            // genuine sitter-miss shifts (xg > 0.75 unconverted) and
            // keep the coef in line with the non-forward 0.55 so a
            // single bad finishing match doesn't double-bite via this
            // *and* the ARE wasted lane.
            let (threshold, coef) = if is_forward {
                (0.75, 0.35)
            } else {
                (0.60, 0.55)
            };
            if s.xg > threshold {
                shooting -= RatingMath::sat(s.xg - threshold, 1.2) * coef;
            }
        }

        // Shot accuracy band — small lift for hitting the target.
        // Gated to 2+ attempts: accuracy from a single shot is not a
        // signal (one speculative miss was reading as a -0.07 "bad
        // accuracy" verdict, one tidy finish as a +0.08 bonus on top
        // of the SoT credit it already earned).
        if s.shots_total >= 2 {
            let accuracy = s.shots_on_target as f32 / s.shots_total as f32;
            shooting += RatingMath::signed_sat(accuracy - 0.40, 0.30) * 0.08;
        }

        // Shot spam: a wasteful low-skill finisher who keeps launching
        // speculative attempts gets a visible drag. Coefficients
        // softened in 2026-06 round 5 after the dev_match benchmark
        // showed forward goalless tier at 5.65 — the shot-spam +
        // no-SoT spam + wasted-xG triplet was double/triple-biting on
        // the same bad-finishing forward.
        if s.shots_total >= 3 {
            let xg_per_shot = s.xg / s.shots_total as f32;
            if xg_per_shot < 0.10 {
                let spam_coef = if is_forward { 0.40 } else { 0.30 };
                shooting -= RatingMath::sat(s.shots_total as f32 - 2.0, 4.0) * spam_coef;
            }
        }

        // No-goal, no-SOT spammer: drag scales with raw shot volume
        // even when xG is small. Forward coef cut 0.30 → 0.24 — the
        // accuracy band above already reads the same 0-for-N signal
        // negatively at 2+ shots, and the double-bite was holding
        // quiet-forward shifts in the 5.5s instead of the FM-style
        // "poor/quiet ≈ 6.0" anchor.
        if s.goals == 0 && s.shots_on_target == 0 && s.shots_total >= 2 {
            let nosot_coef = if is_forward { 0.24 } else { 0.25 };
            shooting -= RatingMath::sat(s.shots_total as f32 - 1.0, 3.0) * nosot_coef;
        }

        shooting
    }

    /// Chance creation: key passes, passes/carries into the box,
    /// completed crosses, xG buildup, zone-aware lane bonuses.
    ///
    /// Assists deliberately do NOT live here — they are routed through
    /// [`Self::scoring_event`] alongside goals, so the same `scoring`
    /// profile weight drives all decisive attacking events. This keeps
    /// the per-position dial coherent (a striker's assist pays through
    /// the same channel as a goal) and prevents the creation soft-cap
    /// from accidentally suppressing assist credit.
    ///
    /// Coefficients are deliberately modest — a real "good creator"
    /// (3 KP + 3 box entries + 4 progressive) lands routine ~0.6,
    /// not the inflated ~1.1 that drove ordinary playmakers to 7.4
    /// on routine alone. The surrounding chain-building creates the
    /// lift, but doesn't take the player into the elite band without
    /// a goal contribution.
    pub(super) fn creation(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;

        let key = RatingMath::sat(s.key_passes as f32, 3.5) * 0.42;

        // Box entries — combine passes-into-box and carries-into-box so
        // the same delivery doesn't pay double if both fired.
        let box_entries = RatingMath::sat(s.passes_into_box as f32 + z.carries_into_box as f32, 5.0) * 0.30;

        // Cross output: completed crosses help, failed crosses drag.
        // Failed-cross penalty softened (was sat(failed, 5.0) * 0.22):
        // a routine fullback attempts 3-5 crosses per match and
        // completes 1-2 (real-football reference: ~25% completion).
        // The prior coefficient hit them with -0.07 to -0.14 routine
        // drag for normal workload, contributing to the Cambiaso 6.20
        // average. The gentler curve still drags a player who can't
        // hit a cross to save their life, but absorbs ordinary fullback
        // crossing volume.
        let cross_credit = RatingMath::sat(s.crosses_completed as f32, 3.5) * 0.13;
        let cross_failed = s.crosses_attempted.saturating_sub(s.crosses_completed) as f32;
        let cross_penalty = RatingMath::sat(cross_failed, 10.0) * 0.10;

        // xG buildup — chains the player participated in that ended
        // in a shot. Clean "made the chance happen" signal.
        let xg_chain = RatingMath::sat(s.xg_buildup.max(0.0), 1.2) * 0.30;

        // Zone-aware lane creation — smaller weights because the same
        // events typically tick `passes_into_box` / `key_passes` too.
        let lanes = RatingMath::sat(
            z.half_space_passes_into_box as f32
                + z.central_passes_into_box as f32
                + z.switches_of_play as f32,
            7.0,
        ) * 0.12;

        // Progressive into final third — chance build-up that didn't
        // reach the box.
        let into_final_third = RatingMath::sat(
            z.progressive_passes_into_final_third as f32
                + z.progressive_carries_into_final_third as f32,
            7.0,
        ) * 0.08;

        key + box_entries + cross_credit - cross_penalty
            + xg_chain
            + lanes
            + into_final_third
    }

    /// Ball progression and dribbling: progressive passes, progressive
    /// carries, carry distance, take-ons. Failed dribbles drag harder
    /// than success rewards — a low-skill dribbler who keeps trying
    /// 1v1s and losing is visibly costing the team.
    ///
    /// Coefficients are tuned so that "moved the ball forward" stats
    /// register but don't dominate. A progressive pass / carry is
    /// observable evidence — it earns Tier B in the soft-cap ladder —
    /// but the raw component contribution stays modest.
    pub(super) fn progression(&self) -> f32 {
        let s = self.stats;

        let pp = RatingMath::sat(s.progressive_passes as f32, 6.0) * 0.26;
        let pc = RatingMath::sat(s.progressive_carries as f32, 5.0) * 0.24;
        let cd = RatingMath::sat(s.carry_distance as f32 / 1000.0, 1.8) * 0.10;

        let drib_w = match self.pos {
            PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder => 0.26,
            _ => 0.14,
        };
        let dribbles = RatingMath::sat(s.successful_dribbles as f32, 3.5) * drib_w;

        let failed = s.attempted_dribbles.saturating_sub(s.successful_dribbles) as f32;
        // Failed-dribble drag is tighter saturation (3.0 vs 4.0) and
        // a heavier per-event weight so a poor 1v1 record visibly hurts.
        // Forwards still get a small discount because the position
        // expects them to take risks.
        let failed_w = if self.pos == PlayerFieldPositionGroup::Forward {
            0.26
        } else {
            0.34
        };
        let failed_drib = RatingMath::sat(failed, 3.0) * failed_w;

        pp + pc + cd + dribbles - failed_drib
    }

    /// Pass-completion quality × volume. A high-volume accurate passer
    /// in midfield is rewarded; a low-completion volume passer is
    /// dragged. Volume gates the magnitude (a 10-pass shift moves the
    /// retention component very little).
    ///
    /// First-touch quality enters here as a small drag from
    /// `miscontrols` and `heavy_touches`. The drag is conservative
    /// because the engine producers for those counters are still being
    /// wired up — once they fire reliably, every event registers as a
    /// visible loss of control without needing a coefficient bump.
    pub(super) fn retention(&self) -> f32 {
        let s = self.stats;
        let touch_drag = self.touch_quality();
        if s.passes_attempted < 10 {
            return touch_drag;
        }
        let pct = s.passes_completed as f32 / s.passes_attempted as f32;
        let volume = RatingMath::sat(s.passes_attempted as f32, 45.0); // saturates by ~90 attempts
        // 0.74 is the league-average baseline. Coefficient lifted
        // 0.30 → 0.50 in the FM-parity DEF/MID season pass: the
        // recycler archetype (60+ passes at ~90%) was accumulating to
        // ~6.38 against the believable 6.50-6.75 band — high-volume
        // accurate circulation is the role's primary output and FM
        // credits it as solid. The elite band still needs progression
        // / creation: the recycler guards (`safe_recycler...`,
        // `high_pass_completion...`) hold tidy volume below 7.0.
        let pass_signal = RatingMath::signed_sat(pct - 0.74, 0.18) * volume * 0.50;
        pass_signal + touch_drag
    }

    /// Touch-quality drag from miscontrols and heavy touches. Returns
    /// a non-positive value (0 if no events recorded). Saturating so
    /// a single bad touch isn't catastrophic but accumulating losses
    /// of control visibly drag the rating.
    ///
    /// The producer (`add_miscontrol` / `add_heavy_touch`) IS wired in
    /// `match/engine/player/events/players.rs` — it fires per receive
    /// roll against `first_touch_loss_probability`, which scales with
    /// (1 − first_touch_skill)² · pressure. A low-skill player under
    /// regular pressure will accumulate 3-5 events per 90 minutes,
    /// landing roughly −0.45 to −0.6 of rating drag.
    pub(super) fn touch_quality(&self) -> f32 {
        let s = self.stats;
        let m = s.miscontrols as f32;
        let h = s.heavy_touches as f32 * 0.5;
        if m + h <= 0.0 {
            return 0.0;
        }
        // sat(3, 5) ≈ 0.45 → ~ -0.38 rating units at three miscontrols;
        // sat(5, 5) ≈ 0.63 → ~ -0.54 at five. Strong enough that
        // low-first-touch players visibly drop, gentle enough that one
        // mishit doesn't define the match.
        -RatingMath::sat(m + h, 5.0) * 0.85
    }

    /// Forward role expectation drag — applied at the rating layer
    /// (alongside the team-result context and discipline deltas) for a
    /// forward who played meaningful minutes without producing a goal
    /// or an assist. Returns a non-positive value; outside the gate it
    /// returns 0 so other positions are unaffected.
    ///
    /// Pure stat-line read, smooth saturation. Two components:
    ///
    /// 1. **Lack-of-impact penalty** — a forward's primary job is
    ///    decisive attacking output. Without G/A we look for the
    ///    secondary footprint that real punditry rewards: shots on
    ///    target, xG generated, key passes, passes into the box,
    ///    successful dribbles. When that footprint is small, the
    ///    forward visibly hasn't done their job.
    ///
    /// 2. **Wasted high-xG penalty** — a forward who racked up clear
    ///    chances and converted none is the signature failed-striker
    ///    shift. This stacks with the wasted-xG drag inside
    ///    [`Self::shooting`]: that drag bites the *shooting* component
    ///    on its own scale; this one bites the *role expectation* on
    ///    the rating's scale.
    ///
    /// Calibration target (pure stat-line, no ability read):
    /// - anonymous forward (0/0/0/0 attacking evidence, 90 min): ≈ −0.85
    /// - creative no-G/A forward (3 KP + 3 PB + 3 drib, xG buildup):
    ///   ≈ −0.30 (only partly saturated — the creative line doesn't
    ///   substitute for a goal contribution)
    /// - wasteful high-xG no-goal striker (xG 2.5, 2 SOT): ≈ −0.50
    pub(super) fn attacking_role_expectation(&self) -> f32 {
        if self.pos != PlayerFieldPositionGroup::Forward {
            return 0.0;
        }
        let s = self.stats;
        if s.goals > 0 || s.assists > 0 {
            return 0.0;
        }
        let minutes = s.minutes_played;
        if minutes < 30 {
            return 0.0;
        }
        // Time-on-pitch factor — full strength from 90 minutes, ramps in
        // smoothly from 30. A short cameo with no G/A isn't a failed
        // shift; an 80-minute starter without a touch on goal is.
        let minute_factor = ((minutes as f32 - 30.0) / 60.0).clamp(0.0, 1.0);

        let sot = s.shots_on_target as f32;
        let xg = s.xg.max(0.0);
        let kp = s.key_passes as f32;
        let pbox = s.passes_into_box as f32;
        let dribs = s.successful_dribbles as f32;

        // Goal-threat evidence — what punditry calls "looked like
        // scoring": SOT and meaningful xG. These are the strongest
        // markers of a forward actually attempting their job.
        let threat = sot * 0.7 + xg * 1.0;
        // Creative evidence — secondary forward output. Lower weight
        // than threat, but not zero: a forward who repeatedly broke
        // the line for teammates has done something.
        let creative = kp * 0.5 + pbox * 0.3 + dribs * 0.3;
        // Combined footprint, with creative work counted at ~70% of
        // direct threat.
        let footprint = threat + creative * 0.7;

        // Final calibration anchored to .dev/match 200-match benchmark.
        // The 2.0 threshold from prior round only lifted goalless mean
        // by +0.09 (5.65 → 5.74); the dominant drag was the *stack*
        // of ARE + shot-spam + no-SoT spam + wasted-xG. Threshold cut
        // to 1.0 so any forward with even one SoT or 0.3 xG clears the
        // gate completely; ARE only fires for genuinely zero-footprint
        // shifts. Coef cut 0.20 → 0.15 in the FM-parity season pass:
        // the FM-style anchor for a quiet shift is "poor ≈ 6.0", and
        // the remaining drag lanes (no-SoT spam, accuracy, context
        // damping) already hold an anonymous forward below baseline —
        // a zero-footprint 90 still loses ≈ −0.06 here plus the rest.
        let shortfall = (1.0 - footprint).max(0.0);
        let lack_penalty = -RatingMath::sat(shortfall, 2.0) * 0.15;

        // Wasted big-chance drag (ARE lane, separate from shooting()).
        // Threshold 0.8, coef cut 0.30 → 0.20 — the 200-match benchmark
        // showed even this minor lane stacking with the shooting()
        // wasted-xG drag on the same xg signal. Reserve this for
        // 1.0+ xG unconverted (clear sitter case).
        let wasted = if xg > 0.8 {
            -RatingMath::sat(xg - 0.8, 1.2) * 0.20
        } else {
            0.0
        };

        (lack_penalty + wasted) * minute_factor
    }
}

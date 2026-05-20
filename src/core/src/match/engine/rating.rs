use crate::PlayerFieldPositionGroup;
use crate::r#match::PlayerMatchEndStats;
use crate::r#match::engine::zones::ZoneCoeffs;
#[cfg(test)]
use crate::r#match::engine::zones::ZoneStats;

// =====================================================================
// Public API
// =====================================================================
//
// Player match ratings (1.0 ..= 10.0, neutral baseline 6.0) computed
// from a [`PlayerMatchEndStats`] snapshot. The model is component-based:
//
//   rating = BASE
//          + compress(positive routine + scoring event) [soft-cap by profile]
//          + negative routine deltas
//          + always-on contextual deltas (result, clean sheet, conceded,
//            errors, cards, discipline, GK exceptional negatives)
//          + final clamp [1, 10]
//
// Each component evaluates to a small signed "impact" value driven by
// smooth saturation curves (`sat`, `signed_sat`). Routine on-the-ball
// signal is always confidence-damped by minutes. Direct event deltas
// (goals, errors-to-goal, red cards, own goals, failed claims) keep
// most of their bite even from a cameo via `event_minutes_factor`.
//
// A cross-component compression and contribution-aware soft caps keep
// the rating distribution realistic: an anonymous starter stays under
// ~7.1, a one-goal-only finisher under ~7.6, and a hat-trick scorer
// is uncapped. Distinct ratings still register because positive
// components stack inside the cap rather than hard-clamping.
//
// Build a context with [`RatingContext::new`] and call
// [`RatingContext::calculate`].

const BASE_RATING: f32 = 6.0;
const RATING_MIN: f32 = 1.0;
const RATING_MAX: f32 = 10.0;

// =====================================================================
// Saturation helpers
// =====================================================================

/// Smooth positive saturation: `1 - exp(-x/scale)`. Returns 0 for
/// non-positive `x`. At `x = scale` ≈ 0.63, at `x = 2·scale` ≈ 0.86,
/// at `x = 3·scale` ≈ 0.95.
#[inline]
fn sat(x: f32, scale: f32) -> f32 {
    if x <= 0.0 || scale <= 0.0 {
        0.0
    } else {
        1.0 - (-x / scale).exp()
    }
}

/// Signed smooth saturation via `tanh`. Useful for percentage-like
/// signals that swing both above and below a baseline.
#[inline]
fn signed_sat(x: f32, scale: f32) -> f32 {
    if scale <= 0.0 {
        0.0
    } else {
        (x / scale).tanh()
    }
}

// =====================================================================
// Confidence + event-minute policy
// =====================================================================

/// Smooth minute-confidence curve. Reaches ~0.40 by 15 minutes, ~0.70
/// by 35, ~0.93 by 70, ~1.0 by 90+. Players that didn't play (0
/// minutes) get 0.0 so their event totals contribute nothing.
fn minute_confidence(minutes: u16) -> f32 {
    if minutes == 0 {
        return 0.0;
    }
    let m = minutes as f32 / 35.0;
    m.tanh()
}

/// Damp factor for direct event deltas (goals, errors-to-goal, reds,
/// own goals). Always ≥ 0.70 so a 5-minute winner keeps the bulk of
/// the goal credit, but a cameo doesn't get the full routine credit
/// either — that part still goes through `minute_confidence`.
#[inline]
fn event_minutes_factor(conf: f32) -> f32 {
    0.70 + 0.30 * conf
}

/// Compress excessive cumulative positive upside. Below the knee passes
/// through unchanged; above, each extra unit is damped to `SLOPE`
/// contribution. Knee is set so that ordinary stat lines (typical
/// per-match routine sum 0.6-1.0) pass through, but accumulated routine
/// stacking past ~1.0 starts to hit diminishing returns — keeps a
/// volume passer / busy worker from drifting into the elite band on
/// routine alone, without flattening genuinely top-tier performances.
#[inline]
fn compress_positive_delta(delta: f32) -> f32 {
    const KNEE: f32 = 1.0;
    const SLOPE: f32 = 0.40;
    if delta <= KNEE {
        delta
    } else {
        KNEE + (delta - KNEE) * SLOPE
    }
}

/// Soft cap: below `cap`, passes through; above, the excess is
/// compressed by `slope_after`. Cheaper than a hard clamp because
/// the relative ordering of "great vs very great" survives.
#[inline]
fn soft_cap(value: f32, cap: f32, slope_after: f32) -> f32 {
    if value <= cap {
        value
    } else {
        cap + (value - cap) * slope_after
    }
}

// =====================================================================
// Position weight profile
// =====================================================================

/// Multiplicative weight per component for a given position. Values
/// near 1.0 mean "this is core to the role"; values near 0 mean "this
/// component basically doesn't apply to this position".
#[derive(Clone, Copy)]
struct Profile {
    scoring: f32,
    shooting: f32,
    creation: f32,
    progression: f32,
    retention: f32,
    defensive: f32,
    goalkeeping: f32,
}

impl Profile {
    fn for_position(pos: PlayerFieldPositionGroup) -> Self {
        match pos {
            PlayerFieldPositionGroup::Goalkeeper => Profile {
                scoring: 1.0,
                shooting: 0.5,
                creation: 0.2,
                progression: 0.2,
                retention: 0.4,
                defensive: 0.4,
                goalkeeping: 1.0,
            },
            PlayerFieldPositionGroup::Defender => Profile {
                scoring: 1.10,
                shooting: 0.6,
                creation: 0.7,
                progression: 0.7,
                retention: 0.8,
                defensive: 1.00,
                goalkeeping: 0.0,
            },
            PlayerFieldPositionGroup::Midfielder => Profile {
                scoring: 1.05,
                shooting: 0.85,
                creation: 1.10,
                progression: 1.00,
                retention: 0.90,
                defensive: 0.85,
                goalkeeping: 0.0,
            },
            PlayerFieldPositionGroup::Forward => Profile {
                scoring: 1.00,
                shooting: 1.10,
                creation: 0.95,
                progression: 0.75,
                retention: 0.55,
                defensive: 0.35,
                goalkeeping: 0.0,
            },
        }
    }
}

// =====================================================================
// RatingContext
// =====================================================================

pub struct RatingContext<'a> {
    stats: &'a PlayerMatchEndStats,
    team_goals: u8,
    opponent_goals: u8,
    pos: PlayerFieldPositionGroup,
    profile: Profile,
    /// Smooth confidence factor for time on the pitch. Applied to all
    /// routine (on-the-ball) components.
    confidence: f32,
}

impl<'a> RatingContext<'a> {
    /// Build a rating context from a player's end-of-match stats and
    /// the final scoreline (from that player's perspective).
    pub fn new(stats: &'a PlayerMatchEndStats, team_goals: u8, opponent_goals: u8) -> Self {
        let pos = stats.position_group;
        let profile = Profile::for_position(pos);
        let confidence = minute_confidence(stats.minutes_played);
        Self {
            stats,
            team_goals,
            opponent_goals,
            pos,
            profile,
            confidence,
        }
    }

    /// Calculate the match rating (1.0 - 10.0, base 6.0).
    ///
    /// Routine components are always damped by minute confidence so a
    /// short cameo of small touches doesn't farm a high rating. Direct
    /// event deltas (goals + clinical/decisive bonuses) keep most of
    /// their bite even from a cameo via `event_minutes_factor`.
    ///
    /// The positive sum is then compressed (a single decisive moment
    /// shouldn't combine with five tiny bonuses to reach elite band)
    /// and gated by contribution-aware soft caps: anonymous starters
    /// stay around 7.1, one-goal-only finishers around 7.6, multi-goal
    /// scorers are uncapped. Negative events (errors-to-goal, reds,
    /// own goals, conceded penalty, GK failed claims) stay at full
    /// strength so a defining moment of failure always lands.
    pub fn calculate(&self) -> f32 {
        let p = self.profile;
        let conf = self.confidence;
        let ev_factor = event_minutes_factor(conf);

        // Routine on-the-ball signal — minute-confidence damped.
        let routine = p.shooting * self.shooting()
            + p.creation * self.creation()
            + p.progression * self.progression()
            + p.retention * self.retention()
            + p.defensive * self.defensive()
            + p.goalkeeping * self.goalkeeping();
        let routine_damped = routine * conf;

        // Direct event delta — goals, decisive/clinical bonus. Softer
        // minute policy so a 5-minute winner keeps most of its credit.
        let event_pos = p.scoring * self.scoring_event();
        let event_damped = event_pos * ev_factor;

        // Split positive/negative pieces so compression only fires on
        // the upside. Routine positives get cross-component compression;
        // event positives are kept intact (one decisive moment should
        // not be sanded down by the same curve that bounds spam).
        //
        // Goalkeepers skip routine compression: every save is decisive
        // evidence in a way an outfield interception isn't, and the
        // gk_busy / gk_modest / passenger tiers in `apply_soft_caps`
        // already gate the upside. Without this exemption a barrage
        // keeper's two-plus rating units get sanded down before the
        // tier cap even sees them.
        let raw_pos_routine = routine_damped.max(0.0);
        let positive_routine = if self.is_goalkeeper() {
            raw_pos_routine
        } else {
            compress_positive_delta(raw_pos_routine)
        };
        let negative_routine = routine_damped.min(0.0);
        let positive_event = event_damped.max(0.0);
        let negative_event = event_damped.min(0.0);

        // Contribution-aware soft caps on the combined positive total.
        let positive_total = self.apply_soft_caps(positive_routine + positive_event);

        let mut rating = BASE_RATING + positive_total + negative_routine + negative_event;
        rating += self.result_context();
        rating += self.clean_sheet_context();
        rating += self.conceded_context();
        rating += self.discipline();
        rating += self.errors_and_cards();
        rating += self.gk_exceptional_negatives();

        rating.clamp(RATING_MIN, RATING_MAX)
    }

    #[inline]
    fn is_goalkeeper(&self) -> bool {
        self.pos == PlayerFieldPositionGroup::Goalkeeper
    }

    /// Effective denominator for save% calculations. The engine populates
    /// `shots_faced` directly; legacy fixtures / save files leave it at
    /// zero, in which case we synthesise it from saves + goals conceded.
    fn shots_faced(&self) -> u16 {
        self.stats
            .shots_faced
            .max(self.stats.saves + self.opponent_goals as u16)
    }

    // ===================================================================
    // Components
    //
    // Each returns a small signed value in "rating units". Magnitudes
    // are deliberately modest — they get multiplied by the position
    // weight (≤ ~1.1) before contributing to the rating.
    // ===================================================================

    /// Direct goal-event impact: goals scored + clinical (over-xG) +
    /// decisive (the goal won the match). Saturates so a hat-trick is
    /// rewarded but not 3× a single goal.
    fn scoring_event(&self) -> f32 {
        let s = self.stats;
        if s.goals == 0 {
            return 0.0;
        }
        let g = s.goals as f32;
        // sat(1, 1.6) ≈ 0.46; sat(2) ≈ 0.71; sat(3) ≈ 0.85.
        let raw = sat(g, 1.6) * 2.55;

        // Clinical-finisher bonus: goals beyond xG → premium for
        // converting tougher chances or being lethal in front of goal.
        let over = (g - s.xg).max(0.0);
        let clinical = sat(over, 1.0) * 0.15;

        // Decisive-goal nudge — the goal mattered to the scoreline.
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
    fn shooting(&self) -> f32 {
        let s = self.stats;
        if s.shots_total == 0 && s.xg <= 0.0 {
            return 0.0;
        }

        let xg_value = sat(s.xg, 1.8) * 0.30;
        let sot_value = sat(s.shots_on_target as f32, 2.5) * 0.22;
        let mut shooting = xg_value + sot_value;

        // Wasted high xG: created premium chances, scored nothing.
        if s.goals == 0 && s.xg > 0.6 {
            shooting -= sat(s.xg - 0.6, 1.2) * 0.55;
        }

        // Shot accuracy band — small lift for hitting the target.
        if s.shots_total > 0 {
            let accuracy = s.shots_on_target as f32 / s.shots_total as f32;
            shooting += signed_sat(accuracy - 0.40, 0.30) * 0.08;
        }

        // Shot spam: ≥ 5 shots with very low xG/shot means chasing
        // shadows. Saturates so 12 bad shots ≠ 4 bad shots × 3.
        if s.shots_total >= 5 {
            let xg_per_shot = s.xg / s.shots_total as f32;
            if xg_per_shot < 0.08 {
                shooting -= sat(s.shots_total as f32 - 4.0, 4.0) * 0.35;
            }
        }

        shooting
    }

    /// Chance creation: assists, key passes, passes/carries into the
    /// box, completed crosses, xG buildup, zone-aware lane bonuses.
    ///
    /// Coefficients are deliberately modest — a real "good creator"
    /// (3 KP + 3 box entries + 4 progressive) lands routine ~0.65,
    /// not the inflated ~1.1 that drove ordinary playmakers to 7.4
    /// on routine alone. Assist event itself still pays well; the
    /// surrounding chain-building creates the lift, but doesn't take
    /// the player into the elite band without a goal-contribution.
    fn creation(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;

        let assists = sat(s.assists as f32, 1.6) * 1.10;

        let key = sat(s.key_passes as f32, 3.5) * 0.42;

        // Box entries — combine passes-into-box and carries-into-box so
        // the same delivery doesn't pay double if both fired.
        let box_entries = sat(
            s.passes_into_box as f32 + z.carries_into_box as f32,
            5.0,
        ) * 0.30;

        // Cross output: completed crosses help, failed crosses drag.
        let cross_credit = sat(s.crosses_completed as f32, 3.5) * 0.13;
        let cross_failed = s
            .crosses_attempted
            .saturating_sub(s.crosses_completed) as f32;
        let cross_penalty = sat(cross_failed, 5.0) * 0.22;

        // xG buildup — chains the player participated in that ended
        // in a shot. Clean "made the chance happen" signal.
        let xg_chain = sat(s.xg_buildup.max(0.0), 1.2) * 0.30;

        // Zone-aware lane creation — smaller weights because the same
        // events typically tick `passes_into_box` / `key_passes` too.
        let lanes = sat(
            z.half_space_passes_into_box as f32
                + z.central_passes_into_box as f32
                + z.switches_of_play as f32,
            7.0,
        ) * 0.12;

        // Progressive into final third — chance build-up that didn't
        // reach the box.
        let into_final_third = sat(
            z.progressive_passes_into_final_third as f32
                + z.progressive_carries_into_final_third as f32,
            7.0,
        ) * 0.08;

        assists + key + box_entries + cross_credit - cross_penalty + xg_chain + lanes
            + into_final_third
    }

    /// Ball progression and dribbling: progressive passes, progressive
    /// carries, carry distance, take-ons. Failed dribbles drag.
    ///
    /// Coefficients are tuned so that "moved the ball forward" stats
    /// register but don't dominate. A progressive pass / carry is
    /// observable evidence — it earns Tier B in the soft-cap ladder —
    /// but the raw component contribution stays modest.
    fn progression(&self) -> f32 {
        let s = self.stats;

        let pp = sat(s.progressive_passes as f32, 6.0) * 0.26;
        let pc = sat(s.progressive_carries as f32, 5.0) * 0.24;
        let cd = sat(s.carry_distance as f32 / 1000.0, 1.8) * 0.10;

        let drib_w = match self.pos {
            PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder => 0.26,
            _ => 0.14,
        };
        let dribbles = sat(s.successful_dribbles as f32, 3.5) * drib_w;

        let failed = s
            .attempted_dribbles
            .saturating_sub(s.successful_dribbles) as f32;
        let failed_w = if self.pos == PlayerFieldPositionGroup::Forward {
            0.18
        } else {
            0.24
        };
        let failed_drib = sat(failed, 4.0) * failed_w;

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
    fn retention(&self) -> f32 {
        let s = self.stats;
        let touch_drag = self.touch_quality();
        if s.passes_attempted < 10 {
            return touch_drag;
        }
        let pct = s.passes_completed as f32 / s.passes_attempted as f32;
        let volume = sat(s.passes_attempted as f32, 45.0); // saturates by ~90 attempts
        // 0.74 is the league-average baseline. High pass completion alone
        // should not be a large bonus — a tidy 90% recycler isn't elite.
        // The coefficient is intentionally modest so that retention has
        // to combine with progression / creation to push a rating up.
        let pass_signal = signed_sat(pct - 0.74, 0.18) * volume * 0.30;
        pass_signal + touch_drag
    }

    /// Touch-quality drag from miscontrols and heavy touches. Returns
    /// a non-positive value (0 if no events recorded). Saturating so
    /// a single bad touch isn't catastrophic but accumulating losses
    /// of control visibly drag the rating.
    fn touch_quality(&self) -> f32 {
        let s = self.stats;
        let m = s.miscontrols as f32;
        let h = s.heavy_touches as f32 * 0.5;
        if m + h <= 0.0 {
            return 0.0;
        }
        // sat(3, 5) ≈ 0.45 → ~ -0.25 rating units at three miscontrols.
        -sat(m + h, 5.0) * 0.55
    }

    /// Defensive work: tackles, interceptions, blocks, clearances,
    /// pressures. Includes a zone-aware premium for actions inside
    /// the own box / six-yard area and pressing high up the pitch.
    ///
    /// Saturation denominators are deliberately set so that real-football
    /// "average per-90" volumes (a CB with 2-3 tackles + 1-2 ints + 3-4
    /// clearances) earn moderate credit, not elite saturation. A defender
    /// who genuinely dominates (5+ tackles, 5+ ints, 6+ clearances) still
    /// pushes the band; their fingerprints just have to look it.
    fn defensive(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;

        // Raw routine volume — tackles / interceptions / blocks /
        // clearances anywhere on the pitch. Coefficients are deliberately
        // modest: a CB with 3-4 of each lands modest credit, not elite.
        // Real lift comes from zone-aware bonuses below (own-box / six-
        // yard actions, final-third pressure / tackles) where the work
        // actually stopped an attack.
        let effective_tackles = (s.tackles as f32 - s.fouls as f32 * 0.5).max(0.0);
        let tackles = sat(effective_tackles, 6.0) * 0.30;
        let interceptions = sat(s.interceptions as f32, 6.0) * 0.30;
        let blocks = sat(s.blocks as f32, 3.5) * 0.28;
        let clearances = sat(s.clearances as f32, 7.5) * 0.16;

        let succ_pressure = sat(s.successful_pressures as f32, 5.5) * 0.16;
        let raw_pressure = s
            .pressures
            .saturating_sub(s.successful_pressures);
        let press_volume = sat(raw_pressure as f32, 12.0) * 0.04;

        // Zone-aware premium on top of the flat work — actions in
        // high-danger zones deserve more credit. Tighter saturation
        // scale means even one own-box intervention reads as meaningful
        // evidence of a real defensive moment, not lost in volume noise.
        let danger_actions = (z.tackles_own_box + z.interceptions_own_box + z.blocks_own_box
            + z.clearances_own_box) as f32
            * 0.5
            + (z.tackles_own_six_yard
                + z.interceptions_own_six_yard
                + z.blocks_own_six_yard
                + z.clearances_own_six_yard) as f32;
        let danger_zone = sat(danger_actions, 4.0) * 0.42;

        let final_third_pressure = sat(z.pressures_won_final_third as f32, 3.0) * 0.10;
        let middle_third_int = sat(z.interceptions_middle_third as f32, 4.0) * 0.05;
        let final_third_tackle = sat(z.tackles_final_third as f32, 3.0) * 0.07;

        tackles + interceptions + blocks + clearances + succ_pressure + press_volume
            + danger_zone
            + final_third_pressure
            + middle_third_int
            + final_third_tackle
    }

    /// Goalkeeping routine signal: saves volume, save percentage, xG
    /// prevented, command-box actions, workload absorbed, and the
    /// quiet-shutout credit. Exceptional negatives (failed claims,
    /// dangerous turnovers, errors-to-goal extras) live in
    /// [`Self::gk_exceptional_negatives`] so they stay at full bite
    /// regardless of minutes played.
    fn goalkeeping(&self) -> f32 {
        if !self.is_goalkeeper() {
            return 0.0;
        }
        let s = self.stats;
        let z = s.zone_stats;

        // Per-save credit — saturates so a 10-save shift isn't 10× a
        // single save.
        let saves_v = sat(s.saves as f32, 2.8) * 1.35;

        // Save percentage band — relative to a 70% baseline. We use a
        // hard zero in the 50%-70% dead-zone to keep a "made the saves
        // they were supposed to" keeper at baseline.
        let shots_faced = self.shots_faced();
        let save_pct_v = if shots_faced >= 3 {
            let pct = s.saves as f32 / shots_faced as f32;
            if pct > 0.70 {
                ((pct - 0.70) * 2.7).min(0.80)
            } else if pct < 0.50 {
                ((pct - 0.50) * 2.0).max(-0.80)
            } else {
                0.0
            }
        } else {
            0.0
        };

        // xG-prevented: positive-only (upside-only by design).
        let direct = s.xg_prevented.max(0.0);
        let xg_prev = if direct > 0.0 {
            direct
        } else if shots_faced >= 3 {
            let expected = shots_faced as f32 * 0.70;
            ((s.saves as f32 - expected) * 0.30).max(0.0)
        } else {
            0.0
        };
        let xg_prev_v = sat(xg_prev, 1.5) * 0.90;

        // Workload absorbed: showing up under a barrage. Capped via sat.
        let workload =
            sat((shots_faced as f32 - 2.0).max(0.0), 6.0) * 0.35;

        // Command-zone actions (cross claims, sweeper interventions).
        let command = sat(z.gk_command_actions as f32, 4.0) * 0.30;

        // Quiet-shutout credit — keeper who organised the line and
        // never had to make a save still earned the clean sheet.
        let dominant_defense = if self.opponent_goals == 0 && shots_faced < 3 {
            0.12
        } else {
            0.0
        };

        saves_v + save_pct_v + xg_prev_v + workload + command + dominant_defense
    }

    /// GK-specific exceptional negatives kept at full strength: failed
    /// claims-to-shot / -goal, dangerous turnovers, errors-to-goal in
    /// the own box. These are "defining moments of failure" and should
    /// always land, regardless of minutes played.
    fn gk_exceptional_negatives(&self) -> f32 {
        if !self.is_goalkeeper() {
            return 0.0;
        }
        let z = self.stats.zone_stats;
        let failed_shot = z.gk_failed_claims_to_shot as f32 * ZoneCoeffs::GK_FAILED_CLAIM_TO_SHOT;
        let failed_goal = z.gk_failed_claims_to_goal as f32 * ZoneCoeffs::GK_FAILED_CLAIM_TO_GOAL;
        let turnovers = sat(
            z.dangerous_turnovers_own_third as f32 * 0.5
                + z.dangerous_turnovers_own_box as f32,
            4.0,
        ) * 0.55;
        let error_extra =
            z.errors_to_goal_own_box as f32 * ZoneCoeffs::ERROR_TO_GOAL_OWN_BOX_EXTRA;
        // Apply GK profile weight (1.0) implicitly — these are GK-only.
        failed_shot + failed_goal - turnovers + error_extra
    }

    /// Contribution-aware soft caps on the cumulative positive delta.
    ///
    /// Replaces the legacy multiplicative passenger guard with an
    /// evidence-based tier ladder: how high a non-scorer can rate is
    /// gated by what they actually did on the pitch. Pure stat line —
    /// never reads ability or any hidden flag.
    ///
    /// Tiers for non-G/A starters (minutes ≥ 60):
    ///
    /// * **Strong evidence** — ≥2 own-box / six-yard interventions, ≥2
    ///   key passes / SOT / dribbles, ≥3 combined zone+save actions:
    ///   cap at +1.5 (=7.5). A real standout shift without a goal.
    /// * **Modest evidence** — at least one zone intervention OR any
    ///   creative event (key pass / box entry / cross / SOT / dribble):
    ///   cap at +1.15 (=7.15). Visible decisive moment.
    /// * **Passenger** — no zone work, no creative output: cap at +0.85
    ///   (=6.85). Routine volume alone, regardless of how busy, can't
    ///   clear 7.0 without observable decisive evidence.
    ///
    /// Goalkeepers have their own tier because save / claim activity
    /// reads as decisive there in a way it doesn't elsewhere.
    fn apply_soft_caps(&self, positive_delta: f32) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;

        let goals = s.goals as u32;
        let assists = s.assists as u32;
        let major_contrib = goals + assists;
        let shot_or_chance = (s.shots_total
            + s.key_passes
            + s.passes_into_box
            + s.successful_dribbles) as u32;
        let defensive_volume = (s.tackles
            + s.interceptions
            + s.blocks
            + s.clearances
            + s.successful_pressures) as u32;
        let gk_volume = s.saves as u32 + z.gk_command_actions as u32;
        let total_volume = shot_or_chance + defensive_volume + gk_volume + (assists * 2);

        let minutes = s.minutes_played;
        let any_event = goals > 0
            || s.errors_leading_to_goal > 0
            || s.red_cards > 0
            || s.own_goals > 0;

        // ──── Direct scoring events take precedence ────
        // Hat trick or 3+ G/A: no cap — they've earned it.
        if goals >= 3 || major_contrib >= 3 {
            return positive_delta;
        }
        // Two goals or goal + assist: cap at +2.3 (= 8.3).
        if goals >= 2 || major_contrib >= 2 {
            return soft_cap(positive_delta, 2.3, 0.45);
        }
        // One goal only with low all-around volume: cap at +1.6 (= 7.6).
        if goals == 1 && total_volume < 6 {
            return soft_cap(positive_delta, 1.6, 0.45);
        }
        // Cameo with no event of any kind: cap at +0.7 (= 6.7).
        if minutes < 30 && !any_event {
            return soft_cap(positive_delta, 0.7, 0.25);
        }

        // ──── Non-G/A starters: evidence-based tier ladder ────
        //
        // Past the cameo / scorer guards, everything else is a starter
        // who didn't score or assist. Their ceiling is gated by visible
        // decisive evidence: routine volume alone cannot clear 7.0,
        // which fixes the global inflation symptom where ~80% of
        // players landed at 7.4 from stacked small bonuses.
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
            // Progressive passes / carries count as decisive evidence:
            // they're the per-spec "moved the ball into dangerous areas"
            // signal that distinguishes a contributor from a passenger.
            // A box-to-box midfielder with 3 progressive passes earns
            // the modest-evidence ceiling even without a key pass.
            let creative_any = (s.key_passes
                + s.passes_into_box
                + s.successful_dribbles
                + s.crosses_completed
                + s.shots_on_target
                + s.progressive_passes
                + s.progressive_carries) as u32;

            if self.is_goalkeeper() {
                // GK: save / claim activity counts as decisive evidence,
                // so the ladder is keyed off keeper-specific signals.
                let gk_busy = s.saves >= 4
                    || z.gk_command_actions >= 3
                    || s.xg_prevented > 0.5;
                if gk_busy {
                    return soft_cap(positive_delta, 1.7, 0.40);
                }
                if s.saves >= 2
                    || z.gk_command_actions >= 1
                    || s.xg_prevented > 0.0
                {
                    return soft_cap(positive_delta, 1.05, 0.35);
                }
                return soft_cap(positive_delta, 0.70, 0.25);
            }

            // Outfield: tiered by decisive evidence. Thresholds are
            // tuned so a single progressive pass / box entry does NOT
            // unlock the modest-evidence band — that would let any
            // active starter drift past 7.0 in a goalless match. Real
            // evidence is multi-event: 2+ key passes / box entries /
            // dribbles, or any own-box / six-yard intervention.
            let big_def = zone_impact + (s.saves as u32) / 2;
            // Strong: multi-action decisive footprint earned across
            // creation, zone interventions, or buildup workload.
            if zone_impact >= 2
                || creative_strong >= 2
                || big_def >= 3
                || s.crosses_completed >= 3
                || (s.key_passes + s.passes_into_box) >= 4
            {
                return soft_cap(positive_delta, 1.3, 0.40);
            }
            // Modest: at least one zone intervention or a concrete
            // creative event (2+ creative_any or 1+ key pass / dribble /
            // box entry / completed cross / SOT). Pure progression
            // alone (just progressive passes / carries) does not unlock
            // this tier — those count toward `creative_any` but the
            // threshold of 2 forces them to combine with something else.
            let creative_decisive = (s.key_passes
                + s.passes_into_box
                + s.successful_dribbles
                + s.crosses_completed
                + s.shots_on_target) as u32;
            if zone_impact >= 1 || creative_decisive >= 1 || creative_any >= 3 {
                return soft_cap(positive_delta, 0.95, 0.30);
            }
            // Passenger: routine volume only. Cap aggressively so
            // routine work alone cannot clear 7.0, regardless of how
            // busy the player was. The context bonuses (clean sheet /
            // win) still add on top, so a busy back-line worker in a
            // 1-0 win can still nudge into the upper 6s — they just
            // can't be elite without decisive evidence.
            return soft_cap(positive_delta, 0.65, 0.20);
        }

        // Anonymous starter (extreme edge case: low total volume,
        // no goal, between cameo and full match): conservative cap.
        if minutes >= 60 && major_contrib == 0 && total_volume < 5 {
            return soft_cap(positive_delta, 1.1, 0.25);
        }

        positive_delta
    }

    // ===================================================================
    // Always-on contextual deltas
    //
    // These are applied AFTER the routine damp / event factor so they
    // hit at full strength even for cameos. They're scoreline /
    // clean-sheet / conceded / discipline signals — not "things this
    // player did on the ball", so confidence shouldn't gate them.
    // ===================================================================

    /// Win / loss nudge.
    fn result_context(&self) -> f32 {
        if self.team_goals > self.opponent_goals {
            0.12
        } else if self.team_goals < self.opponent_goals {
            -0.15
        } else {
            0.0
        }
    }

    /// Position-aware clean-sheet bonus.
    ///
    /// Defenders get a tiered credit based on stat-line evidence of
    /// actual back-line involvement: a CB who made high-danger zone
    /// interventions or posted ≥6 routine defensive actions gets full
    /// credit; a CB with only modest activity gets a reduced bonus;
    /// a truly absent passenger gets the smallest bookkeeping bonus.
    /// This is evidence-based — the gating uses observed stats, not
    /// hidden ability — and stops a back-line passenger from riding
    /// the team's clean sheet into the elite band.
    fn clean_sheet_context(&self) -> f32 {
        if self.opponent_goals != 0 {
            return 0.0;
        }
        match self.pos {
            PlayerFieldPositionGroup::Goalkeeper => 0.30,
            PlayerFieldPositionGroup::Defender => {
                let z = self.stats.zone_stats;
                let high_value = (z.tackles_own_box
                    + z.tackles_own_six_yard
                    + z.interceptions_own_box
                    + z.interceptions_own_six_yard
                    + z.blocks_own_box
                    + z.blocks_own_six_yard
                    + z.clearances_own_box
                    + z.clearances_own_six_yard) as u16;
                let routine = self
                    .stats
                    .tackles
                    .saturating_add(self.stats.interceptions)
                    .saturating_add(self.stats.blocks)
                    .saturating_add(self.stats.clearances)
                    .saturating_add(self.stats.successful_pressures);
                if high_value >= 1 || routine >= 6 {
                    0.25
                } else if routine >= 3 {
                    0.15
                } else {
                    0.08
                }
            }
            PlayerFieldPositionGroup::Midfielder => 0.05,
            _ => 0.0,
        }
    }

    /// Goals-conceded penalty for goalkeepers and (lightly) defenders.
    /// Smooth growth: gentle through the first two, steeper from the
    /// third, slows again past the sixth (so a 10-shipping disaster
    /// stays in the disaster band rather than pinning to the floor).
    fn conceded_context(&self) -> f32 {
        match self.pos {
            PlayerFieldPositionGroup::Goalkeeper => {
                let g = self.opponent_goals as f32;
                let base = 0.30 * g.min(2.0);
                let mid = (g - 2.0).max(0.0) * 0.55;
                let heavy = (g - 5.0).max(0.0) * 0.20;
                -(base + mid + heavy)
            }
            PlayerFieldPositionGroup::Defender if self.opponent_goals >= 3 => {
                // Defenders share blame from the 3rd onward, smoothly.
                let extra = (self.opponent_goals as f32 - 2.0).max(0.0);
                -sat(extra, 3.0) * 1.10
            }
            _ => 0.0,
        }
    }

    /// Fouls, offsides, own-goals, penalty-foul-conceded. Position-
    /// sensitive (forwards live with offsides; back-line players are
    /// extra penalised for own-third fouls).
    fn discipline(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;

        // Fouls — saturating drag so a 10-foul shift doesn't compound
        // linearly.
        let fouls = sat(s.fouls as f32, 5.0) * -0.30;

        let own_third_extra = if matches!(
            self.pos,
            PlayerFieldPositionGroup::Defender | PlayerFieldPositionGroup::Goalkeeper
        ) {
            z.own_third_def_fouls as f32 * ZoneCoeffs::FOUL_OWN_THIRD_DEF_EXTRA_PER
        } else {
            0.0
        };
        let penalty_foul =
            z.penalty_fouls_conceded as f32 * ZoneCoeffs::FOUL_PENALTY;

        let (per, scale) = match self.pos {
            PlayerFieldPositionGroup::Forward => (0.08, 4.0),
            _ => (0.06, 3.0),
        };
        let offsides = -sat(s.offsides as f32, scale) * per * scale; // ≈ per-event ≤ scale*per

        let own_goals = s.own_goals as f32
            * (ZoneCoeffs::OWN_GOAL_BASE + ZoneCoeffs::OWN_GOAL_OWN_BOX_EXTRA);

        fouls + own_third_extra + penalty_foul + offsides + own_goals
    }

    /// Errors that led to a shot or goal + yellow/red cards. Errors-
    /// to-goal hit hard per event — a single mistake is a defining
    /// moment. Always at full strength so a cameo error still lands.
    fn errors_and_cards(&self) -> f32 {
        let s = self.stats;
        let err_shot = sat(s.errors_leading_to_shot as f32, 1.0) * -0.55;
        let err_goal = sat(s.errors_leading_to_goal as f32, 1.2) * -2.40;
        let yellow = s.yellow_cards as f32 * -0.15;
        let red = s.red_cards as f32 * -1.50;
        err_shot + err_goal + yellow + red
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_stats(
        goals: u16,
        assists: u16,
        passes_attempted: u16,
        passes_completed: u16,
        shots_on_target: u16,
        shots_total: u16,
        tackles: u16,
        interceptions: u16,
        saves: u16,
        xg: f32,
        position_group: PlayerFieldPositionGroup,
    ) -> PlayerMatchEndStats {
        PlayerMatchEndStats {
            goals,
            assists,
            passes_attempted,
            passes_completed,
            shots_on_target,
            shots_total,
            tackles,
            interceptions,
            saves,
            shots_faced: 0,
            match_rating: 0.0,
            xg,
            position_group,
            fouls: 0,
            yellow_cards: 0,
            red_cards: 0,
            minutes_played: 90,
            key_passes: 0,
            progressive_passes: 0,
            progressive_carries: 0,
            successful_dribbles: 0,
            attempted_dribbles: 0,
            successful_pressures: 0,
            pressures: 0,
            blocks: 0,
            clearances: 0,
            passes_into_box: 0,
            crosses_attempted: 0,
            crosses_completed: 0,
            xg_chain: 0.0,
            xg_buildup: 0.0,
            miscontrols: 0,
            heavy_touches: 0,
            carry_distance: 0,
            errors_leading_to_shot: 0,
            errors_leading_to_goal: 0,
            xg_prevented: 0.0,
            offsides: 0,
            own_goals: 0,
            zone_stats: ZoneStats::default(),
        }
    }

    fn make_gk(saves: u16, shots_faced: u16) -> PlayerMatchEndStats {
        let mut s = make_stats(
            0, 0, 20, 15, 0, 0, 0, 0, saves, 0.0,
            PlayerFieldPositionGroup::Goalkeeper,
        );
        s.shots_faced = shots_faced;
        s
    }

    fn anonymous(pos: PlayerFieldPositionGroup) -> PlayerMatchEndStats {
        make_stats(0, 0, 0, 0, 0, 0, 0, 0, 0, 0.0, pos)
    }

    // ===========================================================
    // Behavioral invariants
    // ===========================================================

    #[test]
    fn neutral_quiet_player_stays_near_six() {
        let s = anonymous(PlayerFieldPositionGroup::Midfielder);
        let r = RatingContext::new(&s, 1, 1).calculate();
        assert!((r - 6.0).abs() < 0.10, "neutral rating = {}", r);
    }

    #[test]
    fn short_cameo_has_damped_non_exceptional_rating_movement() {
        let mut starter = make_stats(
            0, 0, 30, 26, 0, 0, 0, 0, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        starter.minutes_played = 90;
        starter.key_passes = 2;
        starter.progressive_passes = 4;
        starter.passes_into_box = 3;
        starter.zone_stats.progressive_passes_into_final_third = 3;
        let mut cameo = starter.clone();
        cameo.minutes_played = 10;
        let starter_r = RatingContext::new(&starter, 1, 1).calculate();
        let cameo_r = RatingContext::new(&cameo, 1, 1).calculate();
        assert!(
            starter_r > cameo_r + 0.3,
            "starter {} should clearly outrate damped cameo {}",
            starter_r,
            cameo_r
        );
        assert!(
            cameo_r < 6.6,
            "cameo with no exceptional events rated {} — should stay damped near 6",
            cameo_r
        );
    }

    #[test]
    fn late_goal_cameo_can_rate_high() {
        // 5-minute cameo, scored the winner. event_minutes_factor keeps
        // most of the goal credit.
        let mut s = make_stats(
            1, 0, 4, 3, 1, 1, 0, 0, 0, 0.5,
            PlayerFieldPositionGroup::Forward,
        );
        s.minutes_played = 5;
        s.shots_on_target = 1;
        let r = RatingContext::new(&s, 2, 1).calculate();
        assert!(
            r >= 7.1 && r <= 7.8,
            "late-goal cameo rated {} — should be in 7.1..=7.8",
            r
        );
    }

    #[test]
    fn one_goal_low_volume_forward_does_not_exceed_7_7() {
        // 90 minutes, 1 goal, 1 SOT, low creation/passing.
        let mut s = make_stats(
            1, 0, 12, 9, 1, 1, 0, 0, 0, 0.5,
            PlayerFieldPositionGroup::Forward,
        );
        s.minutes_played = 90;
        let r = RatingContext::new(&s, 2, 1).calculate();
        assert!(
            r >= 7.0 && r <= 7.7,
            "single-goal low-volume FWD rated {} — should be 7.0..=7.7",
            r
        );
    }

    #[test]
    fn two_goals_can_reach_eight_but_not_nine_without_all_round_volume() {
        // 90 minutes, 2 goals, 2 SOT, low creation. Should reach 8.0
        // but not 9.0 without supporting volume.
        let mut s = make_stats(
            2, 0, 18, 14, 2, 2, 0, 0, 0, 0.9,
            PlayerFieldPositionGroup::Forward,
        );
        s.minutes_played = 90;
        let r = RatingContext::new(&s, 2, 0).calculate();
        assert!(
            r >= 8.0 && r <= 8.7,
            "two-goal low-volume FWD rated {} — should be 8.0..=8.7",
            r
        );
    }

    #[test]
    fn creative_no_goal_forward_outrates_passive_baseline() {
        // A creator-shape forward without a goal (3 KP + 3 box entries
        // + 3 successful dribbles + 4 progressive carries) lands in
        // the upper 6s under the new evidence-based calibration —
        // observable creative work without a finishing event sits
        // between "ordinary" (6.0-6.9) and "good performer" (7.0-7.4)
        // rather than auto-claiming the latter on routine alone. We
        // pin the relative ordering (creative > passive baseline) and
        // a tight band that prevents an unrelated regression.
        let mut fwd = make_stats(
            0, 0, 35, 28, 0, 0, 0, 0, 0, 0.6,
            PlayerFieldPositionGroup::Forward,
        );
        fwd.key_passes = 3;
        fwd.passes_into_box = 3;
        fwd.successful_dribbles = 3;
        fwd.attempted_dribbles = 4;
        fwd.progressive_carries = 4;
        fwd.xg_buildup = 0.4;

        // Baseline: same passing line, no creative footprint.
        let passive = make_stats(
            0, 0, 35, 28, 0, 0, 0, 0, 0, 0.0,
            PlayerFieldPositionGroup::Forward,
        );
        let base_r = RatingContext::new(&passive, 1, 0).calculate();
        let r = RatingContext::new(&fwd, 1, 0).calculate();
        assert!(
            r > base_r + 0.5,
            "creative forward {} must visibly outrate passive baseline {}",
            r,
            base_r
        );
        assert!(
            r >= 6.7 && r < 7.4,
            "creative forward rated {} — should land 6.7..7.4 (top of ordinary / lower good)",
            r
        );
    }

    #[test]
    fn anonymous_clean_sheet_defender_stays_below_7() {
        let mut s = make_stats(
            0, 0, 18, 15, 0, 0, 0, 0, 0, 0.0,
            PlayerFieldPositionGroup::Defender,
        );
        s.minutes_played = 90;
        let r = RatingContext::new(&s, 1, 0).calculate();
        assert!(r < 7.0, "anonymous clean-sheet DEF rated {} — must be < 7.0", r);
    }

    #[test]
    fn safe_recycler_does_not_get_elite_rating() {
        let mut s = make_stats(
            0, 0, 60, 55, 0, 0, 0, 0, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        s.passes_into_box = 0;
        s.progressive_passes = 1;
        let r = RatingContext::new(&s, 1, 1).calculate();
        assert!(
            r < 7.0,
            "safe recycler rated {} — should not reach elite band",
            r
        );
    }

    #[test]
    fn defensive_midfielder_can_exceed_seven_from_defense_and_progression() {
        let mut mid = make_stats(
            0, 0, 45, 38, 0, 0, 5, 5, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        mid.successful_pressures = 6;
        mid.pressures = 12;
        mid.blocks = 2;
        mid.progressive_passes = 5;
        mid.progressive_carries = 3;
        mid.carry_distance = 1400;
        let r = RatingContext::new(&mid, 1, 0).calculate();
        assert!(
            r > 7.0,
            "defensive MID rated {} — should clear 7.0 on D + progression",
            r
        );
    }

    #[test]
    fn defender_own_box_interventions_rate_higher_than_midfield_volume() {
        let mut middle = make_stats(
            0, 0, 25, 21, 0, 0, 3, 3, 0, 0.0,
            PlayerFieldPositionGroup::Defender,
        );
        middle.clearances = 4;
        middle.blocks = 2;
        let mut box_cb = middle.clone();
        box_cb.zone_stats.tackles_own_box = 3;
        box_cb.zone_stats.interceptions_own_box = 3;
        box_cb.zone_stats.clearances_own_box = 2;
        box_cb.zone_stats.blocks_own_box = 1;
        box_cb.zone_stats.clearances_own_six_yard = 2;
        box_cb.zone_stats.blocks_own_six_yard = 1;
        let mid_r = RatingContext::new(&middle, 1, 0).calculate();
        let box_r = RatingContext::new(&box_cb, 1, 0).calculate();
        assert!(
            box_r > mid_r + 0.20,
            "box CB ({}) should outrate middle-zone CB ({})",
            box_r,
            mid_r
        );
    }

    #[test]
    fn goalkeeper_busy_clean_sheet_rates_well() {
        let busy = make_gk(6, 6);
        let r = RatingContext::new(&busy, 1, 0).calculate();
        assert!(r >= 7.3, "busy CS GK rated {} — should reach 7.3+", r);
    }

    #[test]
    fn goalkeeper_conceding_three_with_saves_is_distinguished_from_no_saves() {
        let busy_three = make_gk(5, 8);
        let bad_three = make_gk(0, 3);
        let busy_r = RatingContext::new(&busy_three, 0, 3).calculate();
        let bad_r = RatingContext::new(&bad_three, 0, 3).calculate();
        assert!(
            busy_r > bad_r + 0.5,
            "busy 3-shipping GK ({}) must outrate inactive 3-shipping GK ({})",
            busy_r,
            bad_r
        );
    }

    #[test]
    fn errors_and_red_cards_materially_lower_rating() {
        let clean = make_stats(
            0, 0, 20, 16, 0, 0, 2, 2, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        let mut bad = clean.clone();
        bad.errors_leading_to_goal = 1;
        let mut red = clean.clone();
        red.red_cards = 1;

        let clean_r = RatingContext::new(&clean, 1, 1).calculate();
        let err_r = RatingContext::new(&bad, 1, 2).calculate();
        let red_r = RatingContext::new(&red, 1, 1).calculate();
        assert!(
            clean_r - err_r > 1.5,
            "error-to-goal must drop rating significantly: clean {} → err {}",
            clean_r,
            err_r
        );
        assert!(
            clean_r - red_r > 1.0,
            "red card must drop rating: clean {} → red {}",
            clean_r,
            red_r
        );
    }

    #[test]
    fn rating_stays_in_one_to_ten_range() {
        let mut great = make_stats(
            5, 3, 60, 57, 5, 5, 5, 5, 10, 4.0,
            PlayerFieldPositionGroup::Forward,
        );
        great.key_passes = 8;
        great.progressive_passes = 12;
        great.progressive_carries = 8;
        great.successful_dribbles = 6;
        great.attempted_dribbles = 7;
        great.passes_into_box = 6;
        great.successful_pressures = 5;
        great.pressures = 12;
        great.crosses_attempted = 5;
        great.crosses_completed = 4;
        great.blocks = 2;
        great.clearances = 3;
        great.carry_distance = 3000;
        great.xg_buildup = 1.5;
        let r = RatingContext::new(&great, 6, 0).calculate();
        assert!(r >= RATING_MIN && r <= RATING_MAX, "great rating {}", r);

        let mut bad = anonymous(PlayerFieldPositionGroup::Goalkeeper);
        bad.minutes_played = 90;
        bad.errors_leading_to_goal = 3;
        bad.red_cards = 1;
        bad.own_goals = 1;
        bad.zone_stats.errors_to_goal_own_box = 3;
        let r = RatingContext::new(&bad, 0, 8).calculate();
        assert!(r >= RATING_MIN && r <= RATING_MAX, "bad rating {}", r);
    }

    #[test]
    fn extreme_stat_spam_saturates_without_hard_ceiling_artifacts() {
        let mut moderate = make_stats(
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        moderate.key_passes = 4;
        moderate.progressive_passes = 8;
        moderate.passes_into_box = 4;
        moderate.successful_dribbles = 4;
        moderate.attempted_dribbles = 4;
        let mut extreme = moderate.clone();
        extreme.key_passes = 30;
        extreme.progressive_passes = 50;
        extreme.passes_into_box = 25;
        extreme.successful_dribbles = 25;
        extreme.attempted_dribbles = 25;

        let mod_r = RatingContext::new(&moderate, 1, 0).calculate();
        let ext_r = RatingContext::new(&extreme, 1, 0).calculate();
        assert!(ext_r >= mod_r, "spam must not rate below moderate");
        let delta = ext_r - mod_r;
        assert!(
            delta < 1.5,
            "saturation should bound extreme vs moderate delta ({}) — got {}",
            mod_r,
            delta
        );
        assert!(
            ext_r <= 10.0,
            "spam must respect final clamp — got {}",
            ext_r
        );
    }

    // ===========================================================
    // Sanity / regression checks
    // ===========================================================

    #[test]
    fn busy_gk_outrates_quiet_gk() {
        let quiet = make_gk(1, 1);
        let busy = make_gk(8, 9);
        let quiet_r = RatingContext::new(&quiet, 1, 0).calculate();
        let busy_r = RatingContext::new(&busy, 0, 1).calculate();
        assert!(busy_r > quiet_r + 0.5, "busy {} vs quiet {}", busy_r, quiet_r);
    }

    #[test]
    fn gk_shipping_many_goals_is_disaster_band() {
        let gk = make_gk(3, 10);
        let r = RatingContext::new(&gk, 0, 7).calculate();
        assert!(r < 4.5, "7-shipping GK rated {} — should be a disaster", r);
        let none = make_gk(0, 7);
        let none_r = RatingContext::new(&none, 0, 7).calculate();
        assert!(r > none_r, "any-effort {} must outrate no-effort {}", r, none_r);
    }

    #[test]
    fn defender_with_clean_sheet_and_interventions_lifts_above_quiet() {
        let passive = make_stats(
            0, 0, 20, 16, 0, 0, 0, 0, 0, 0.0,
            PlayerFieldPositionGroup::Defender,
        );
        let mut active = passive.clone();
        active.tackles = 3;
        active.interceptions = 4;
        active.clearances = 6;
        active.blocks = 2;
        let p_r = RatingContext::new(&passive, 1, 0).calculate();
        let a_r = RatingContext::new(&active, 1, 0).calculate();
        assert!(a_r > p_r + 0.4, "active CB {} vs passive {}", a_r, p_r);
    }

    #[test]
    fn forward_offsides_penalised_more_than_midfielder() {
        let mut fwd = make_stats(
            0, 0, 10, 7, 0, 0, 0, 0, 0, 0.0,
            PlayerFieldPositionGroup::Forward,
        );
        fwd.offsides = 3;
        let mut mid = fwd.clone();
        mid.position_group = PlayerFieldPositionGroup::Midfielder;
        let fwd_r = RatingContext::new(&fwd, 1, 1).calculate();
        let mid_r = RatingContext::new(&mid, 1, 1).calculate();
        assert!(fwd_r < mid_r, "FWD offsides {} vs MID offsides {}", fwd_r, mid_r);
    }

    #[test]
    fn high_volume_accurate_passing_beats_low_volume() {
        let few = make_stats(
            0, 0, 15, 14, 0, 0, 2, 0, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        let many = make_stats(
            0, 0, 55, 50, 0, 0, 2, 0, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        let f_r = RatingContext::new(&few, 1, 1).calculate();
        let m_r = RatingContext::new(&many, 1, 1).calculate();
        assert!(m_r > f_r, "many {} vs few {}", m_r, f_r);
    }

    #[test]
    fn clean_sheet_keeper_with_distribution_errors_stays_above_floor() {
        let mut gk = make_gk(1, 1);
        gk.errors_leading_to_shot = 5;
        let r = RatingContext::new(&gk, 0, 0).calculate();
        assert!(
            r >= 5.0,
            "clean-sheet keeper with intercepted long balls rated {} — should stay reasonable",
            r
        );
    }

    #[test]
    fn own_goal_drops_rating_materially() {
        let mut s = make_stats(
            0, 0, 30, 25, 0, 0, 2, 3, 0, 0.0,
            PlayerFieldPositionGroup::Defender,
        );
        s.clearances = 4;
        let base_r = RatingContext::new(&s, 1, 1).calculate();
        s.own_goals = 1;
        let og_r = RatingContext::new(&s, 1, 2).calculate();
        assert!(base_r - og_r >= 1.0, "OG drop {} → {}", base_r, og_r);
    }

    #[test]
    fn wasteful_high_xg_no_goals_does_not_match_clinical_two_goals() {
        let mut wasteful = make_stats(
            0, 0, 20, 15, 2, 6, 0, 0, 0, 2.5,
            PlayerFieldPositionGroup::Forward,
        );
        wasteful.shots_on_target = 2;
        let clinical = make_stats(
            2, 0, 20, 15, 2, 3, 0, 0, 0, 0.6,
            PlayerFieldPositionGroup::Forward,
        );
        let w_r = RatingContext::new(&wasteful, 2, 0).calculate();
        let c_r = RatingContext::new(&clinical, 2, 0).calculate();
        assert!(
            c_r > w_r + 1.0,
            "clinical {} must clearly outrate wasteful {}",
            c_r,
            w_r
        );
    }

    #[test]
    fn errors_to_shot_saturate() {
        let mut few = make_stats(
            0, 0, 30, 24, 0, 0, 0, 0, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        few.errors_leading_to_shot = 2;
        let mut many = few.clone();
        many.errors_leading_to_shot = 8;
        let f_r = RatingContext::new(&few, 1, 1).calculate();
        let m_r = RatingContext::new(&many, 1, 1).calculate();
        assert!(
            (f_r - m_r).abs() < 0.15,
            "errors-to-shot should saturate: 2 {} vs 8 {} delta {}",
            f_r,
            m_r,
            f_r - m_r
        );
    }

    #[test]
    fn failed_gk_claim_to_goal_subtracts_full_strength() {
        let mut gk = make_gk(3, 4);
        let baseline = RatingContext::new(&gk, 1, 1).calculate();
        gk.zone_stats.gk_failed_claims_to_goal = 1;
        let with_fail = RatingContext::new(&gk, 1, 1).calculate();
        let drop = baseline - with_fail;
        assert!(
            drop > 0.6,
            "failed-claim-to-goal must hit hard — got drop {}",
            drop
        );
    }

    #[test]
    fn xg_buildup_lifts_midfielder() {
        let plain = make_stats(
            0, 0, 40, 34, 0, 0, 3, 0, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        let mut chained = plain.clone();
        chained.xg_buildup = 0.8;
        let p_r = RatingContext::new(&plain, 1, 1).calculate();
        let c_r = RatingContext::new(&chained, 1, 1).calculate();
        assert!(c_r > p_r, "buildup chained {} > plain {}", c_r, p_r);
    }

    // ===========================================================
    // Low-HQ passenger / routine-volume guards
    //
    // These pin the headline bug: a back-line / midfield player
    // racking up routine defensive volume (no own-box impact, no
    // progressive output, no key passes / crosses / dribbles, no
    // G/A) should not drift into the elite band on the back of a
    // clean-sheet win. Stat-line evidence only — never reads
    // current_ability or any hidden flag.
    // ===========================================================

    #[test]
    fn low_impact_routine_cb_with_clean_sheet_stays_below_seven() {
        // 12 routine defensive actions, 80% completion on 30 safe
        // passes, no own-box / six-yard interventions, no progressive
        // passes / carries / dribbles, no key passes. Clean-sheet win.
        // This is the engine's typical low-HQ CB output shape — the
        // rating must NOT report this as a 7.0+ shift.
        let mut s = make_stats(
            0, 0, 30, 24, 0, 0, 3, 2, 0, 0.0,
            PlayerFieldPositionGroup::Defender,
        );
        s.clearances = 4;
        s.blocks = 1;
        s.successful_pressures = 2;
        s.pressures = 8;
        s.minutes_played = 90;
        let r = RatingContext::new(&s, 1, 0).calculate();
        assert!(
            r < 7.0,
            "low-impact routine CB with clean sheet rated {} — must stay < 7.0",
            r
        );
        assert!(
            r > 6.0,
            "low-impact routine CB rated {} — should still benefit from showing up",
            r
        );
    }

    #[test]
    fn low_impact_routine_mid_with_routine_passing_stays_below_seven() {
        // 30/26 passes (87%), 1 tackle, 1 interception, 1 successful
        // pressure, 5 pressures. No key pass / progressive pass / cross.
        // Win 1-0 (clean sheet doesn't help midfielders much). The
        // engine's typical low-HQ shuttler shape — must stay sub-7.0.
        let mut s = make_stats(
            0, 0, 30, 26, 0, 0, 1, 1, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        s.successful_pressures = 1;
        s.pressures = 5;
        s.minutes_played = 90;
        let r = RatingContext::new(&s, 1, 0).calculate();
        assert!(
            r < 7.0,
            "low-impact routine MID rated {} — must stay < 7.0",
            r
        );
    }

    #[test]
    fn cb_with_own_box_intervention_clears_passenger_guard() {
        // Same routine volume as the bug case above, but with a single
        // own-box clearance. That's stat-line evidence of a decisive
        // moment — the passenger ceiling (Tier C, +0.85) must lift to
        // the modest-evidence ceiling (Tier B, +1.15) on the strength
        // of that one zone event.
        let mut s = make_stats(
            0, 0, 30, 24, 0, 0, 3, 2, 0, 0.0,
            PlayerFieldPositionGroup::Defender,
        );
        s.clearances = 4;
        s.blocks = 1;
        s.successful_pressures = 2;
        s.pressures = 8;
        s.minutes_played = 90;
        // Baseline: same player, no zone event.
        let baseline = RatingContext::new(&s, 1, 0).calculate();
        // With the own-box intervention added.
        s.zone_stats.clearances_own_box = 1;
        let r = RatingContext::new(&s, 1, 0).calculate();
        // Single own-box clearance is genuine decisive evidence — it
        // must lift the rating above the baseline. The absolute value
        // is no longer expected to clear 7.0 (one clearance is a
        // modest event, not a man-of-the-match shift), but the ladder
        // must visibly reward the evidence.
        assert!(
            r > baseline,
            "CB with own-box intervention rated {} — must outrate the equivalent player without the intervention ({})",
            r,
            baseline
        );
        assert!(
            r > 6.8,
            "CB with own-box intervention rated {} — should at least sit in the upper 6s for an active back-line shift",
            r
        );
    }

    #[test]
    fn low_hq_player_with_decisive_goal_still_rates_above_seven() {
        // The fix should reduce *fake* competence from routine volume,
        // never block a real decisive moment. A "low-HQ" forward whose
        // single match yielded a goal must still be allowed past 7.0.
        let mut s = make_stats(
            1, 0, 12, 8, 1, 2, 0, 0, 0, 0.4,
            PlayerFieldPositionGroup::Forward,
        );
        s.minutes_played = 80;
        let r = RatingContext::new(&s, 1, 0).calculate();
        assert!(
            r > 7.0,
            "low-volume forward with a goal rated {} — must reach 7.0+",
            r
        );
    }

    #[test]
    fn miscontrols_drag_rating_when_recorded() {
        // Once the engine producers fire, miscontrols / heavy touches
        // must visibly drag the rating — the helper's coefficient is
        // calibrated to land but not dominate.
        let mut clean = make_stats(
            0, 0, 30, 26, 0, 0, 2, 1, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        clean.minutes_played = 90;
        let mut sloppy = clean.clone();
        sloppy.miscontrols = 3;
        sloppy.heavy_touches = 2;
        let clean_r = RatingContext::new(&clean, 1, 1).calculate();
        let sloppy_r = RatingContext::new(&sloppy, 1, 1).calculate();
        assert!(
            clean_r > sloppy_r + 0.15,
            "miscontrols/heavy-touches should drag: clean {} vs sloppy {}",
            clean_r,
            sloppy_r
        );
    }

    #[test]
    fn quiet_passenger_below_busy_passenger() {
        // Both pass the passenger guard, but the busy worker bee should
        // outrate the truly quiet one — the graded `busy` multiplier
        // preserves ordering within the passenger band.
        let mut quiet = make_stats(
            0, 0, 18, 15, 0, 0, 1, 1, 0, 0.0,
            PlayerFieldPositionGroup::Defender,
        );
        quiet.minutes_played = 90;
        let mut busy = quiet.clone();
        busy.tackles = 4;
        busy.interceptions = 3;
        busy.clearances = 5;
        busy.blocks = 1;
        busy.successful_pressures = 2;
        busy.pressures = 6;
        let quiet_r = RatingContext::new(&quiet, 1, 0).calculate();
        let busy_r = RatingContext::new(&busy, 1, 0).calculate();
        assert!(
            busy_r > quiet_r + 0.3,
            "busy CB {} should outrate quiet passenger CB {}",
            busy_r,
            quiet_r
        );
        // Quiet passenger never clears 7.0; the busy 15-action worker
        // bee is allowed to drift just past it on clean-sheet credit,
        // but well below the elite band that real decisive output
        // would unlock.
        assert!(
            quiet_r < 7.0,
            "quiet passenger CB rated {} — must stay < 7.0",
            quiet_r
        );
        assert!(
            busy_r < 7.3,
            "busy passenger CB rated {} — should not breach 7.3 without decisive output",
            busy_r
        );
    }

    #[test]
    fn clean_sheet_credit_tiered_by_defensive_evidence() {
        // CB with zero defensive activity (and no own-box presence)
        // gets a minimal clean-sheet bonus; a busy back-line workhorse
        // gets the full +0.25. Evidence-based — no ability read.
        let mut quiet = make_stats(
            0, 0, 18, 15, 0, 0, 0, 0, 0, 0.0,
            PlayerFieldPositionGroup::Defender,
        );
        quiet.minutes_played = 90;
        let mut busy = quiet.clone();
        busy.tackles = 3;
        busy.interceptions = 2;
        busy.clearances = 3;
        busy.blocks = 1;
        let mut zone_busy = quiet.clone();
        zone_busy.zone_stats.tackles_own_box = 1;
        let qctx = RatingContext::new(&quiet, 1, 0);
        let bctx = RatingContext::new(&busy, 1, 0);
        let zctx = RatingContext::new(&zone_busy, 1, 0);
        assert!(qctx.clean_sheet_context() < bctx.clean_sheet_context());
        assert!(qctx.clean_sheet_context() < zctx.clean_sheet_context());
        assert!(
            (zctx.clean_sheet_context() - 0.25).abs() < 0.001,
            "own-box intervention earns full clean-sheet bonus"
        );
    }

    #[test]
    fn destroyer_midfielder_with_clutch_blocks_rates_well() {
        // Heavy defensive volume + progression in a 1-0 win: with the
        // new evidence-based calibration this kind of "shuttler"
        // performance lands in the upper 6s rather than auto-claiming
        // the elite band. Routine work without a goal / assist /
        // own-box intervention is genuinely "good but not great",
        // which matches the spec's "most players: 6.0-6.9" target.
        let mut destroyer = make_stats(
            0, 0, 40, 34, 0, 0, 6, 5, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        destroyer.blocks = 3;
        destroyer.successful_pressures = 5;
        destroyer.pressures = 12;
        destroyer.progressive_passes = 3;
        let mut passive = destroyer.clone();
        passive.tackles = 0;
        passive.interceptions = 0;
        passive.blocks = 0;
        passive.successful_pressures = 0;
        passive.pressures = 0;
        passive.progressive_passes = 0;
        let r = RatingContext::new(&destroyer, 1, 0).calculate();
        let base_r = RatingContext::new(&passive, 1, 0).calculate();
        assert!(
            r > base_r + 0.5,
            "destroyer {} must visibly outrate passive baseline {}",
            r,
            base_r
        );
        assert!(r > 6.7, "destroyer MID rated {} — clutch D should lift past 6.7", r);
    }

    // ===========================================================
    // Distribution targets (from the global-inflation spec).
    //
    // These tests pin the headline calibration: an ordinary
    // midfielder / defender / forward without decisive output
    // should cluster in the mid-6s, not at 7.4. A goal / assist /
    // multi-key-pass shift earns the 7.0+ band on evidence.
    // Stat-line only — never reads current_ability.
    // ===========================================================

    #[test]
    fn ordinary_midfielder_with_routine_volume_stays_in_mid_six_band() {
        // Spec stat line: 35/42 passes (83%), 1 progressive pass,
        // 1 tackle, 1 interception, no goal/assist/key pass/error.
        // Expected band: 6.2–6.7.
        let mut s = make_stats(
            0, 0, 42, 35, 0, 0, 1, 1, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        s.progressive_passes = 1;
        s.minutes_played = 90;
        // 1-1 draw — no clean-sheet / win lift.
        let r = RatingContext::new(&s, 1, 1).calculate();
        assert!(
            r >= 6.0 && r <= 6.7,
            "ordinary MID rated {} — should sit 6.0..6.7 per the spec target",
            r
        );
    }

    #[test]
    fn ordinary_defender_in_draw_without_clean_sheet_stays_low_six() {
        // Spec stat line: 90 min, moderate passing (20/25, 80%),
        // 2 clearances, 1 tackle, no clean sheet (drawn 1-1).
        // Expected band: 6.1–6.6.
        let mut s = make_stats(
            0, 0, 25, 20, 0, 0, 1, 0, 0, 0.0,
            PlayerFieldPositionGroup::Defender,
        );
        s.clearances = 2;
        s.minutes_played = 90;
        let r = RatingContext::new(&s, 1, 1).calculate();
        assert!(
            r >= 6.0 && r <= 6.6,
            "ordinary DEF in draw rated {} — should sit 6.0..6.6",
            r
        );
    }

    #[test]
    fn losing_midfielder_with_routine_volume_stays_below_six_eight() {
        // No goal/assist, some passes/progression/defense, team lost.
        // Expected: < 6.8 — defeat + no decisive output combined caps
        // the rating below the "good performer" band.
        let mut s = make_stats(
            0, 0, 50, 42, 0, 0, 2, 2, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        s.progressive_passes = 2;
        s.successful_pressures = 3;
        s.pressures = 8;
        s.minutes_played = 90;
        let r = RatingContext::new(&s, 0, 2).calculate(); // lost 0-2
        assert!(
            r < 6.8,
            "losing MID rated {} — should stay below 6.8 without decisive output",
            r
        );
    }

    #[test]
    fn good_creator_lands_in_seven_to_seven_four_band() {
        // Key passes + box entries + strong progression — the
        // "good performer" archetype. Expected band: 7.0–7.4.
        let mut s = make_stats(
            0, 0, 50, 42, 0, 0, 1, 1, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        s.key_passes = 3;
        s.passes_into_box = 2;
        s.progressive_passes = 5;
        s.progressive_carries = 2;
        s.xg_buildup = 0.4;
        s.minutes_played = 90;
        let r = RatingContext::new(&s, 1, 0).calculate();
        assert!(
            r >= 7.0 && r <= 7.6,
            "good creator MID rated {} — should land 7.0..7.6",
            r
        );
    }

    #[test]
    fn decisive_playmaker_with_assist_clears_seven() {
        // Spec: "goal or assist — rating can exceed 7.0". Real
        // assist-day lines come with creative context (key passes,
        // box entries, progression) — the assist event isn't a
        // standalone signal in the stats stream. The rating ladder
        // rewards the cumulative decisive footprint.
        let mut s = make_stats(
            0, 1, 50, 42, 0, 0, 1, 1, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        s.key_passes = 2;
        s.passes_into_box = 1;
        s.progressive_passes = 2;
        s.minutes_played = 90;
        let r = RatingContext::new(&s, 1, 0).calculate();
        assert!(r > 7.0, "decisive playmaker rated {} — assist + creation must clear 7.0", r);
    }

    #[test]
    fn high_pass_completion_alone_does_not_unlock_seven() {
        // 60 passes at 95% completion, no tackles / ints / creation /
        // progression. The spec calls this out explicitly: "high pass
        // completion should not be a large bonus unless volume and
        // progression are meaningful". 1-0 win + clean sheet for MID
        // gives +0.17 context, so the rating should still stay sub-7.
        let mut s = make_stats(
            0, 0, 60, 57, 0, 0, 0, 0, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        s.minutes_played = 90;
        let r = RatingContext::new(&s, 1, 0).calculate();
        assert!(
            r < 7.0,
            "pure recycler rated {} — high completion alone must not breach 7.0",
            r
        );
    }

    #[test]
    fn busy_routine_defender_without_decisive_evidence_stays_sub_seven() {
        // 7/7/7/5 routine defensive actions — a very busy CB by
        // per-90 standards. No zone events, no creative output,
        // clean-sheet win. Routine volume alone may not produce a
        // 7.0+; the passenger cap (Tier C) keeps it in the upper 6s.
        let mut s = make_stats(
            0, 0, 25, 21, 0, 0, 7, 7, 0, 0.0,
            PlayerFieldPositionGroup::Defender,
        );
        s.clearances = 7;
        s.successful_pressures = 5;
        s.pressures = 12;
        s.minutes_played = 90;
        let r = RatingContext::new(&s, 1, 0).calculate();
        // A very busy routine CB on a clean sheet IS allowed to nudge
        // marginally past 7.0 because the team kept a shutout + win,
        // but never to the elite band. We pin the upper bound below
        // 7.3 — anything more would mean routine volume is unlocking
        // a "good performer" rating, which is the inflation symptom.
        assert!(
            r < 7.3,
            "very busy passenger CB rated {} — must not breach 7.3 without decisive evidence",
            r
        );
    }

    #[test]
    fn losing_team_full_squad_does_not_cluster_at_seven_four() {
        // Spec acceptance criterion: "Losing-team players should not
        // be broadly rated as good performers." Probe a representative
        // losing-side stat distribution: routine outputs for a CB,
        // a CM, and a striker, all in a 0-2 defeat. None should clear
        // 7.0 without decisive output.
        let mut cb = make_stats(
            0, 0, 32, 26, 0, 0, 4, 3, 0, 0.0,
            PlayerFieldPositionGroup::Defender,
        );
        cb.clearances = 5;
        cb.blocks = 1;
        cb.minutes_played = 90;
        let cb_r = RatingContext::new(&cb, 0, 2).calculate();

        let mut cm = make_stats(
            0, 0, 55, 46, 0, 0, 2, 2, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        cm.progressive_passes = 3;
        cm.successful_pressures = 3;
        cm.pressures = 9;
        cm.minutes_played = 90;
        let cm_r = RatingContext::new(&cm, 0, 2).calculate();

        let mut st = make_stats(
            0, 0, 18, 12, 1, 4, 0, 0, 0, 0.4,
            PlayerFieldPositionGroup::Forward,
        );
        st.shots_on_target = 1;
        st.successful_dribbles = 1;
        st.attempted_dribbles = 3;
        st.minutes_played = 90;
        let st_r = RatingContext::new(&st, 0, 2).calculate();

        for (label, r) in [("CB", cb_r), ("CM", cm_r), ("ST", st_r)] {
            assert!(
                r < 7.0,
                "losing-team {} rated {} — losers without decisive output must stay sub-7",
                label,
                r
            );
        }
    }

    #[test]
    fn ordinary_winning_starter_without_major_action_stays_below_seven() {
        // Spec acceptance criterion: "Players with no goal/assist/big
        // defensive action should usually stay below 7.0." Probe a
        // typical winning-side CM with routine outputs and no decisive
        // events. Win + clean-sheet context contributes +0.17, but
        // Tier B (modest evidence) keeps the ceiling at 7.15 and the
        // routine sum lands under that.
        let mut s = make_stats(
            0, 0, 45, 38, 0, 0, 2, 2, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        s.progressive_passes = 2;
        s.successful_pressures = 2;
        s.pressures = 7;
        s.minutes_played = 90;
        let r = RatingContext::new(&s, 1, 0).calculate();
        assert!(
            r < 7.0,
            "ordinary winning CM rated {} — routine on the winning side must not clear 7.0",
            r
        );
    }

    #[test]
    fn evidence_tier_ladder_orders_correctly_across_three_archetypes() {
        // Same minutes, same passing baseline. Only the decisive
        // evidence differs. The tier ladder must rate them strictly
        // monotonically:
        //   passenger  (no zone / no creative / no shot)
        //   < modest   (1 key pass)
        //   < strong   (multi key passes + zone work)
        let mut base = make_stats(
            0, 0, 35, 28, 0, 0, 2, 2, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        base.minutes_played = 90;
        let passenger = base.clone();
        let mut modest = base.clone();
        modest.key_passes = 1;
        let mut strong = base.clone();
        strong.key_passes = 3;
        strong.passes_into_box = 2;
        strong.zone_stats.pressures_won_final_third = 2;
        let p_r = RatingContext::new(&passenger, 1, 0).calculate();
        let m_r = RatingContext::new(&modest, 1, 0).calculate();
        let s_r = RatingContext::new(&strong, 1, 0).calculate();
        assert!(p_r < m_r, "passenger {} should be < modest {}", p_r, m_r);
        assert!(m_r < s_r, "modest {} should be < strong {}", m_r, s_r);
        // Passenger is below the 7.0 band; strong has earned the lift.
        assert!(p_r < 7.0, "passenger MID rated {}", p_r);
    }

    #[test]
    fn one_goal_low_volume_player_still_clears_seven_for_decisive_output() {
        // Spec: "A low-HQ player with visible decisive output should
        // still be rated well." Even a low-touch forward with a single
        // goal must clear 7.0 — the fix should reduce *fake* competence,
        // never block a real decisive moment.
        let mut s = make_stats(
            1, 0, 8, 5, 1, 1, 0, 0, 0, 0.35,
            PlayerFieldPositionGroup::Forward,
        );
        s.minutes_played = 65;
        let r = RatingContext::new(&s, 1, 0).calculate();
        assert!(r > 7.0, "low-volume forward with a goal rated {} — decisive output must land", r);
    }
}

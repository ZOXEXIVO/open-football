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

/// Compress excessive cumulative positive upside. Below 1.6 rating
/// units passes through unchanged; above that, each extra unit is
/// damped to 0.55× contribution. Keeps a stat-spammer from stacking
/// six small bonuses into elite territory without a decisive moment.
#[inline]
fn compress_positive_delta(delta: f32) -> f32 {
    const KNEE: f32 = 1.6;
    const SLOPE: f32 = 0.55;
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
        let positive_routine = compress_positive_delta(routine_damped.max(0.0));
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
    fn creation(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;

        let assists = sat(s.assists as f32, 1.6) * 1.10;

        let key = sat(s.key_passes as f32, 3.5) * 0.50;

        // Box entries — combine passes-into-box and carries-into-box so
        // the same delivery doesn't pay double if both fired.
        let box_entries = sat(
            s.passes_into_box as f32 + z.carries_into_box as f32,
            5.0,
        ) * 0.38;

        // Cross output: completed crosses help, failed crosses drag.
        let cross_credit = sat(s.crosses_completed as f32, 3.5) * 0.16;
        let cross_failed = s
            .crosses_attempted
            .saturating_sub(s.crosses_completed) as f32;
        let cross_penalty = sat(cross_failed, 5.0) * 0.18;

        // xG buildup — chains the player participated in that ended
        // in a shot. Clean "made the chance happen" signal.
        let xg_chain = sat(s.xg_buildup.max(0.0), 1.2) * 0.36;

        // Zone-aware lane creation — smaller weights because the same
        // events typically tick `passes_into_box` / `key_passes` too.
        let lanes = sat(
            z.half_space_passes_into_box as f32
                + z.central_passes_into_box as f32
                + z.switches_of_play as f32,
            7.0,
        ) * 0.18;

        // Progressive into final third — chance build-up that didn't
        // reach the box.
        let into_final_third = sat(
            z.progressive_passes_into_final_third as f32
                + z.progressive_carries_into_final_third as f32,
            7.0,
        ) * 0.12;

        assists + key + box_entries + cross_credit - cross_penalty + xg_chain + lanes
            + into_final_third
    }

    /// Ball progression and dribbling: progressive passes, progressive
    /// carries, carry distance, take-ons. Failed dribbles drag.
    fn progression(&self) -> f32 {
        let s = self.stats;

        let pp = sat(s.progressive_passes as f32, 6.0) * 0.32;
        let pc = sat(s.progressive_carries as f32, 5.0) * 0.30;
        let cd = sat(s.carry_distance as f32 / 1000.0, 1.8) * 0.14;

        let drib_w = match self.pos {
            PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder => 0.32,
            _ => 0.18,
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
    fn retention(&self) -> f32 {
        let s = self.stats;
        if s.passes_attempted < 10 {
            return 0.0;
        }
        let pct = s.passes_completed as f32 / s.passes_attempted as f32;
        let volume = sat(s.passes_attempted as f32, 45.0); // saturates by ~90 attempts
        // 0.74 is the league-average baseline. Above 0.92 saturates near +0.46,
        // below 0.56 saturates near -0.46.
        signed_sat(pct - 0.74, 0.18) * volume * 0.46
    }

    /// Defensive work: tackles, interceptions, blocks, clearances,
    /// pressures. Includes a zone-aware premium for actions inside
    /// the own box / six-yard area and pressing high up the pitch.
    fn defensive(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;

        // Effective tackles = tackles minus a share of fouls (so the
        // player didn't "earn" a tackle by hacking the runner down).
        let effective_tackles = (s.tackles as f32 - s.fouls as f32 * 0.5).max(0.0);
        let tackles = sat(effective_tackles, 4.5) * 0.48;
        let interceptions = sat(s.interceptions as f32, 4.5) * 0.48;
        let blocks = sat(s.blocks as f32, 2.8) * 0.38;
        let clearances = sat(s.clearances as f32, 5.5) * 0.28;

        let succ_pressure = sat(s.successful_pressures as f32, 4.5) * 0.24;
        let raw_pressure = s
            .pressures
            .saturating_sub(s.successful_pressures);
        let press_volume = sat(raw_pressure as f32, 10.0) * 0.08;

        // Zone-aware premium on top of the flat work — actions in
        // high-danger zones deserve more credit.
        let danger_actions = (z.tackles_own_box + z.interceptions_own_box + z.blocks_own_box
            + z.clearances_own_box) as f32
            * 0.5
            + (z.tackles_own_six_yard
                + z.interceptions_own_six_yard
                + z.blocks_own_six_yard
                + z.clearances_own_six_yard) as f32;
        let danger_zone = sat(danger_actions, 5.5) * 0.38;

        let final_third_pressure = sat(z.pressures_won_final_third as f32, 3.0) * 0.18;
        let middle_third_int = sat(z.interceptions_middle_third as f32, 4.0) * 0.10;
        let final_third_tackle = sat(z.tackles_final_third as f32, 3.0) * 0.14;

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
    /// Prevents an anonymous starter from drifting into elite ratings,
    /// while leaving multi-goal scorers uncapped.
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

        // Hat trick or 3+ G/A: no cap.
        if goals >= 3 || major_contrib >= 3 {
            return positive_delta;
        }

        // Two goals or goal + assist: cap at +2.3 (= 8.3) with slope
        // 0.45 — a clear elite shift but not pinned to 10.
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

        // Anonymous starter: 60+ minutes, no G/A, low meaningful volume.
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
    fn clean_sheet_context(&self) -> f32 {
        if self.opponent_goals != 0 {
            return 0.0;
        }
        match self.pos {
            PlayerFieldPositionGroup::Goalkeeper => 0.30,
            PlayerFieldPositionGroup::Defender => 0.25,
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
    fn forward_without_goal_can_exceed_seven_when_creative() {
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
        let r = RatingContext::new(&fwd, 1, 0).calculate();
        assert!(
            r > 7.0 && r < 7.8,
            "creative no-goal forward rated {} — should be 7.0..7.8",
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

    #[test]
    fn destroyer_midfielder_with_clutch_blocks_rates_well() {
        let mut destroyer = make_stats(
            0, 0, 40, 34, 0, 0, 6, 5, 0, 0.0,
            PlayerFieldPositionGroup::Midfielder,
        );
        destroyer.blocks = 3;
        destroyer.successful_pressures = 5;
        destroyer.pressures = 12;
        destroyer.progressive_passes = 3;
        let r = RatingContext::new(&destroyer, 1, 0).calculate();
        assert!(r > 7.0, "destroyer MID rated {} — clutch D should lift", r);
    }
}

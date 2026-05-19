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
//          + Σ profile_weight[c] · component[c]
//          + always-on contextual deltas (result, clean sheet, conceded,
//            errors, cards, discipline)
//          + final clamp [1, 10]
//
// Each component evaluates to a small signed "impact" value driven by
// smooth saturation curves (`sat`, `signed_sat`) rather than by stacks
// of fixed `.min()` caps. Saturation lets exceptional performances
// register without runaway stacking. There are no per-position hard
// ceilings (e.g. "forward without a goal cannot exceed 7.0"). Low
// involvement naturally stays near 6.0 because the components produce
// small numbers, not because they are clamped.
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
/// at `x = 3·scale` ≈ 0.95. Used everywhere a counter should grow
/// quickly at first and then diminish.
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
    /// Smooth confidence factor for time on the pitch — full at ~70+
    /// minutes, ramps from ~0.25 for short cameos. Exceptional events
    /// (goals, red cards, errors-to-goal, own goals) bypass the damp
    /// since a 5-minute winner should still rate high.
    confidence: f32,
    exceptional: bool,
}

impl<'a> RatingContext<'a> {
    /// Build a rating context from a player's end-of-match stats and
    /// the final scoreline (from that player's perspective).
    pub fn new(stats: &'a PlayerMatchEndStats, team_goals: u8, opponent_goals: u8) -> Self {
        let pos = stats.position_group;
        let profile = Profile::for_position(pos);
        let exceptional = stats.goals > 0
            || stats.red_cards > 0
            || stats.errors_leading_to_goal > 0
            || stats.own_goals > 0;
        let confidence = minute_confidence(stats.minutes_played);
        Self {
            stats,
            team_goals,
            opponent_goals,
            pos,
            profile,
            confidence,
            exceptional,
        }
    }

    /// Calculate the match rating (1.0 - 10.0, base 6.0).
    ///
    /// Components evaluate to signed "impact" values. Each is multiplied
    /// by its position weight from [`Profile`] before summing. The
    /// per-component impact is then scaled by minute confidence (so
    /// short cameos move less for non-exceptional play) and added to
    /// the always-on contextual deltas (team result, clean sheet,
    /// conceded penalty, discipline). The final value is clamped to
    /// `[1.0, 10.0]`.
    pub fn calculate(&self) -> f32 {
        let p = self.profile;

        // Position-weighted components. These are all "rating units"
        // (signed deltas applied directly to base 6.0).
        let mut weighted_impact = 0.0_f32;
        weighted_impact += p.scoring * self.scoring();
        weighted_impact += p.shooting * self.shooting();
        weighted_impact += p.creation * self.creation();
        weighted_impact += p.progression * self.progression();
        weighted_impact += p.retention * self.retention();
        weighted_impact += p.defensive * self.defensive();
        weighted_impact += p.goalkeeping * self.goalkeeping();

        // Exceptional events ignore the confidence damp; routine event
        // volume from a short cameo is otherwise diluted so a 10-minute
        // sub stacking small contributions can't post 7.5+ off them.
        let confidence_applied = if self.exceptional {
            weighted_impact
        } else {
            weighted_impact * self.confidence
        };

        // Always-on contextual deltas — applied at full strength so a
        // cameo error-to-goal or a clean sheet for a back-line player
        // still registers properly.
        let mut rating = BASE_RATING + confidence_applied;
        rating += self.result_context();
        rating += self.clean_sheet_context();
        rating += self.conceded_context();
        rating += self.discipline();
        rating += self.errors_and_cards();

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

    /// Goals scored. Saturates so a hat-trick is rewarded but not 3×
    /// a single goal. A small over-conversion bonus rewards clinical
    /// finishing (more goals than xG suggested); under-conversion
    /// shows up via the shooting component's wasted-xG penalty.
    fn scoring(&self) -> f32 {
        let s = self.stats;
        if s.goals == 0 {
            return 0.0;
        }
        let g = s.goals as f32;
        // sat(1, 1.6) ≈ 0.46; sat(2) ≈ 0.71; sat(3) ≈ 0.85; sat(5) ≈ 0.96
        let raw = sat(g, 1.6) * 2.6;

        // Clinical-finisher bonus: goals beyond xG → premium for
        // converting tougher chances or being lethal in front of goal.
        let over = (g - s.xg).max(0.0);
        let clinical = sat(over, 1.0) * 0.25;

        // Decisive-goal nudge — the goal mattered to the scoreline.
        let decisive = if self.team_goals > self.opponent_goals {
            0.12
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

        // xG and shots on target both signal "you got into shooting
        // positions". Saturating both prevents a spam-shooter from
        // farming this component.
        let xg_value = sat(s.xg, 1.8) * 0.40;
        let sot_value = sat(s.shots_on_target as f32, 2.5) * 0.30;
        let mut shooting = xg_value + sot_value;

        // Wasted high xG: created premium chances, scored nothing.
        // Smooth penalty rather than a hard floor.
        if s.goals == 0 && s.xg > 0.7 {
            shooting -= sat(s.xg - 0.7, 1.2) * 0.50;
        }

        // Shot accuracy band — small lift for hitting the target,
        // small drag for spraying it wide.
        if s.shots_total > 0 {
            let accuracy = s.shots_on_target as f32 / s.shots_total as f32;
            shooting += signed_sat(accuracy - 0.40, 0.30) * 0.12;
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
    /// All inputs feed through `sat`, so a key-pass-and-box-entry-and-
    /// progressive-pass on the same play doesn't pay four independent
    /// bonuses — diminishing returns keep them honest.
    fn creation(&self) -> f32 {
        let s = self.stats;
        let z = s.zone_stats;

        let assists = sat(s.assists as f32, 1.6) * 1.50;

        let key = sat(s.key_passes as f32, 3.0) * 0.55;

        // Box entries — combine passes-into-box and carries-into-box so
        // the same delivery doesn't pay double if both fired.
        let box_entries = sat(
            s.passes_into_box as f32 + z.carries_into_box as f32,
            4.0,
        ) * 0.40;

        // Cross output: completed crosses help, failed crosses drag but
        // smoothly (winger spam is taxed without floors).
        let cross_credit = sat(s.crosses_completed as f32, 3.0) * 0.22;
        let cross_failed = s
            .crosses_attempted
            .saturating_sub(s.crosses_completed) as f32;
        let cross_penalty = sat(cross_failed, 5.0) * 0.18;

        // xG buildup — chains the player participated in that ended
        // in a shot. Excludes own shots/assists, so it's a clean
        // "made the chance happen" signal.
        let xg_chain = sat(s.xg_buildup.max(0.0), 1.0) * 0.40;

        // Zone-aware lane creation — smaller weights because the same
        // events typically tick `passes_into_box` / `key_passes` too.
        let lanes = sat(
            z.half_space_passes_into_box as f32
                + z.central_passes_into_box as f32
                + z.switches_of_play as f32,
            6.0,
        ) * 0.25;

        // Progressive into final third — chance build-up that didn't
        // reach the box.
        let into_final_third = sat(
            z.progressive_passes_into_final_third as f32
                + z.progressive_carries_into_final_third as f32,
            6.0,
        ) * 0.18;

        assists + key + box_entries + cross_credit - cross_penalty + xg_chain + lanes
            + into_final_third
    }

    /// Ball progression and dribbling: progressive passes, progressive
    /// carries, carry distance, take-ons. Failed dribbles drag.
    fn progression(&self) -> f32 {
        let s = self.stats;

        let pp = sat(s.progressive_passes as f32, 5.0) * 0.45;
        let pc = sat(s.progressive_carries as f32, 4.0) * 0.40;
        let cd = sat(s.carry_distance as f32 / 1000.0, 1.5) * 0.20;

        let drib_w = match self.pos {
            PlayerFieldPositionGroup::Forward | PlayerFieldPositionGroup::Midfielder => 0.40,
            _ => 0.25,
        };
        let dribbles = sat(s.successful_dribbles as f32, 3.0) * drib_w;

        let failed = s
            .attempted_dribbles
            .saturating_sub(s.successful_dribbles) as f32;
        let failed_w = if self.pos == PlayerFieldPositionGroup::Forward {
            0.18
        } else {
            0.25
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
        let volume = sat(s.passes_attempted as f32, 40.0); // saturates by ~80 attempts
        // 0.70 is the league-average baseline. Above 0.90 saturates to ~+0.55,
        // below 0.50 saturates to ~-0.55.
        signed_sat(pct - 0.70, 0.18) * volume * 0.65
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
        let tackles = sat(effective_tackles, 4.0) * 0.55;
        let interceptions = sat(s.interceptions as f32, 4.0) * 0.55;
        let blocks = sat(s.blocks as f32, 2.5) * 0.45;
        let clearances = sat(s.clearances as f32, 5.0) * 0.35;

        let succ_pressure = sat(s.successful_pressures as f32, 4.0) * 0.30;
        let raw_pressure = s
            .pressures
            .saturating_sub(s.successful_pressures);
        let press_volume = sat(raw_pressure as f32, 10.0) * 0.12;

        // Zone-aware premium on top of the flat work — actions in
        // high-danger zones deserve more credit. Smooth saturation
        // keeps the bonus from runaway-stacking when several events
        // happen to share a zone.
        let danger_actions = (z.tackles_own_box + z.interceptions_own_box + z.blocks_own_box
            + z.clearances_own_box) as f32
            * 0.5
            + (z.tackles_own_six_yard
                + z.interceptions_own_six_yard
                + z.blocks_own_six_yard
                + z.clearances_own_six_yard) as f32;
        let danger_zone = sat(danger_actions, 5.0) * 0.45;

        let final_third_pressure = sat(z.pressures_won_final_third as f32, 3.0) * 0.20;
        let middle_third_int = sat(z.interceptions_middle_third as f32, 4.0) * 0.10;
        let final_third_tackle = sat(z.tackles_final_third as f32, 3.0) * 0.15;

        tackles + interceptions + blocks + clearances + succ_pressure + press_volume
            + danger_zone
            + final_third_pressure
            + middle_third_int
            + final_third_tackle
    }

    /// Goalkeeping: saves volume, save percentage, xG prevented,
    /// command-box actions, workload absorbed, and the conceded curve
    /// (which lives here so it composes with the rest of the keeper
    /// signal under the position weight). Failed-claim coefficients
    /// stay at full strength regardless of the rest of the match.
    fn goalkeeping(&self) -> f32 {
        if !self.is_goalkeeper() {
            return 0.0;
        }
        let s = self.stats;
        let z = s.zone_stats;

        // Per-save credit — saturates so a 10-save shift isn't 10× a
        // single save.
        let saves_v = sat(s.saves as f32, 2.6) * 1.6;

        // Save percentage band — relative to a 70% baseline. We use a
        // hard zero in the 50%-70% dead-zone to keep a "made the saves
        // they were supposed to" keeper at baseline (matches WhoScored
        // references for league-average GK shifts).
        let shots_faced = self.shots_faced();
        let save_pct_v = if shots_faced >= 3 {
            let pct = s.saves as f32 / shots_faced as f32;
            if pct > 0.70 {
                ((pct - 0.70) * 3.0).min(0.95)
            } else if pct < 0.50 {
                ((pct - 0.50) * 2.0).max(-0.80)
            } else {
                0.0
            }
        } else {
            0.0
        };

        // xG-prevented: positive-only (upside-only by design). Engine
        // negative ledger from concessions is already taxed by the
        // conceded curve. If the live producer hasn't populated this,
        // synthesise from save volume vs. a 70% baseline.
        let direct = s.xg_prevented.max(0.0);
        let xg_prev = if direct > 0.0 {
            direct
        } else if shots_faced >= 3 {
            let expected = shots_faced as f32 * 0.70;
            ((s.saves as f32 - expected) * 0.30).max(0.0)
        } else {
            0.0
        };
        let xg_prev_v = sat(xg_prev, 1.5) * 1.05;

        // Workload absorbed: showing up under a barrage. Capped via sat
        // so 25 shots faced doesn't lift the rating to absurd values.
        let workload =
            sat((shots_faced as f32 - 2.0).max(0.0), 6.0) * 0.45;

        // Command-zone actions (cross claims, sweeper interventions).
        let command = sat(z.gk_command_actions as f32, 4.0) * 0.30;

        // Failed-claim penalties stay un-saturated and at full bite —
        // a botched cross that became a goal is a defining moment.
        let failed_shot =
            z.gk_failed_claims_to_shot as f32 * ZoneCoeffs::GK_FAILED_CLAIM_TO_SHOT;
        let failed_goal =
            z.gk_failed_claims_to_goal as f32 * ZoneCoeffs::GK_FAILED_CLAIM_TO_GOAL;

        // Dangerous-turnover hit for keepers distributing under pressure.
        // Saturates so 8 turnovers ≠ 8× one turnover.
        let turnovers = sat(
            z.dangerous_turnovers_own_third as f32 * 0.5
                + z.dangerous_turnovers_own_box as f32,
            4.0,
        ) * 0.55;
        let error_extra =
            z.errors_to_goal_own_box as f32 * ZoneCoeffs::ERROR_TO_GOAL_OWN_BOX_EXTRA;

        // Quiet-shutout credit — a keeper who organised the line and
        // never had to make a save still earned the clean sheet behind
        // them. Only fires on a clean sheet with < 3 shots faced so it
        // never doubles with the save stack.
        let dominant_defense = if self.opponent_goals == 0 && shots_faced < 3 {
            0.15
        } else {
            0.0
        };

        saves_v + save_pct_v + xg_prev_v + workload + command + failed_shot + failed_goal
            - turnovers + error_extra + dominant_defense
    }

    // ===================================================================
    // Always-on contextual deltas
    //
    // These are applied AFTER the confidence damp so they hit at full
    // strength even for cameos. They're scoreline / clean-sheet /
    // conceded / discipline signals — not "things this player did on
    // the ball", so confidence shouldn't gate them.
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

    /// Errors that led to a shot or goal + yellow/red cards.
    /// Saturating so a torrent of small giveaways can't push a clean-
    /// sheet keeper below a sensible floor; the worst events (errors-
    /// to-goal, reds) keep their full bite.
    fn errors_and_cards(&self) -> f32 {
        let s = self.stats;
        // Errors-to-shot saturates quickly: 1-2 cost meaningfully, 5-8
        // doesn't keep compounding. A keeper distributing under
        // pressure shouldn't be floored by repeated long-ball
        // interceptions when none turned into a goal.
        let err_shot = sat(s.errors_leading_to_shot as f32, 1.0) * -0.55;
        // Errors-to-goal hit hard per event — a single mistake is a
        // defining moment. Saturation is gentle so the second/third
        // still cost, but the first one already lands the heaviest
        // blow.
        let err_goal = sat(s.errors_leading_to_goal as f32, 1.2) * -2.40;
        let yellow = s.yellow_cards as f32 * -0.15;
        let red = s.red_cards as f32 * -1.50;
        err_shot + err_goal + yellow + red
    }
}

/// Smooth minute-confidence curve. Reaches ~0.40 by 15 minutes, ~0.70
/// by 35, ~0.93 by 70, ~1.0 by 90+. Players that didn't play (0
/// minutes) get 0.0 so their event totals contribute nothing.
fn minute_confidence(minutes: u16) -> f32 {
    if minutes == 0 {
        return 0.0;
    }
    // tanh hits ~0.99 at x≈2.65. Tuning: 70 minutes → tanh(2) ≈ 0.96.
    let m = minutes as f32 / 35.0;
    m.tanh()
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
    // Behavioral invariants (the 12 spec tests)
    // ===========================================================

    #[test]
    fn neutral_quiet_player_stays_near_six() {
        let s = anonymous(PlayerFieldPositionGroup::Midfielder);
        let r = RatingContext::new(&s, 1, 1).calculate();
        assert!((r - 6.0).abs() < 0.10, "neutral rating = {}", r);
    }

    #[test]
    fn short_cameo_has_damped_non_exceptional_rating_movement() {
        // Same modern stats, one starter, one cameo.
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
            starter_r > cameo_r + 0.5,
            "starter {} should clearly outrate damped cameo {}",
            starter_r,
            cameo_r
        );
        // Cameo without exceptional events should be near 6.0.
        assert!(
            cameo_r < 6.6,
            "cameo with no exceptional events rated {} — should stay damped near 6",
            cameo_r
        );
    }

    #[test]
    fn late_goal_cameo_can_rate_high() {
        // 5-minute cameo, scored the winner. The exception bypass
        // means this player keeps the full goal credit.
        let mut s = make_stats(
            1, 0, 4, 3, 1, 1, 0, 0, 0, 0.5,
            PlayerFieldPositionGroup::Forward,
        );
        s.minutes_played = 5;
        s.shots_on_target = 1;
        let r = RatingContext::new(&s, 2, 1).calculate();
        assert!(r >= 7.5, "late-goal cameo rated {} — should reach 7.5+", r);
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
            r > 7.0,
            "creative no-goal forward rated {} — should exceed 7.0",
            r
        );
    }

    #[test]
    fn safe_recycler_does_not_get_elite_rating() {
        // 60-pass shift, sideways only, no creation/defending/shooting.
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
            box_r > mid_r + 0.25,
            "box CB ({}) should outrate middle-zone CB ({})",
            box_r,
            mid_r
        );
    }

    #[test]
    fn goalkeeper_busy_clean_sheet_rates_well() {
        // Busy CS keeper sits in the strong-performance band.
        let busy = make_gk(6, 6);
        let r = RatingContext::new(&busy, 1, 0).calculate();
        assert!(r >= 7.5, "busy CS GK rated {} — should reach 7.5+", r);
    }

    #[test]
    fn goalkeeper_conceding_three_with_saves_is_distinguished_from_no_saves() {
        let busy_three = make_gk(5, 8);
        let bad_three = make_gk(0, 3);
        let busy_r = RatingContext::new(&busy_three, 0, 3).calculate();
        let bad_r = RatingContext::new(&bad_three, 0, 3).calculate();
        assert!(
            busy_r > bad_r + 0.7,
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
        // Best case + worst case both produce in-range values.
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
        // Comparing 50× volume vs 10× volume — saturation means the
        // 50× shift isn't 5× the rating delta.
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
    // Sanity / regression checks carried over (in spirit, not in
    // hardcoded thresholds). All assert behavioral relationships,
    // not exact numeric outputs.
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
        // But not pinned to the floor: another keeper with no saves at
        // all should rate lower.
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
        assert!(a_r > p_r + 0.6, "active CB {} vs passive {}", a_r, p_r);
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
        // Two events ≈ 8 events delta should be small (saturating cap).
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
        // The full coefficient applies (full strength under the GK
        // position weight 1.0).
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
        assert!(r > 7.2, "destroyer MID rated {} — clutch D should lift", r);
    }
}

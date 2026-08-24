//! The match engine itself: [`FootballEngine`], the type every simulated
//! match runs through, and the standalone types it produces. The engine
//! is stateless — it carries no fields — so everything below is either an
//! `impl FootballEngine` split by concern, or plain data.
//!
//! | Group             | Concern                                                        |
//! |-------------------|----------------------------------------------------------------|
//! | `types`           | The plain data: pitch, player store, clock, outcome            |
//! | `run`             | One match start to finish: `play()`, the shootout, the result  |
//! | `tick`            | One tick: the driver, its sub-phases, the plan refresh         |
//! | `resolve`         | What the tick does to the ball between those phases            |
//! | `diagnostics`     | The profiler and the `match-logs` censuses                     |
//!
//! Only the flat type names and `phase_prof` are re-exported below; the
//! groups themselves stay private, so the engine root's `pub use
//! engine::*` surfaces exactly what it did before the grouping.

use crate::r#match::PlayerSide;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::context::SubstitutionRecord;
use crate::r#match::engine::events::dispatcher::EventCollection;
use crate::r#match::engine::goal::{
    advance_goal_celebration, assign_kickoff, finish_goal_celebration, handle_goal_reset,
};
#[cfg(feature = "match-logs")]
use crate::r#match::engine::player::events::players::save_accounting_stats;
use crate::r#match::engine::rating::RatingContext;
use crate::r#match::engine::set_pieces::{
    penalty_conversion_prob, score_keeper_save, score_penalty_taker,
};
use crate::r#match::engine::substitutions::{Substitutions, process_substitutions};
use crate::r#match::events::EventDispatcher;
use crate::r#match::field::MatchField;
#[cfg(feature = "match-logs")]
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::result::ResultMatchPositionData;
use crate::r#match::{
    AttackPlan, AttackRefreshInputs, CoachInstruction, DefenceRefreshInputs, DefensivePlan,
    GameTickContext, MatchContext, MatchPlayer, MatchResultRaw, MatchSquad, MatchState,
    PenaltyShootoutKick, Score, ShapeRefreshInputs, StateManager, SubstitutionInfo,
    TacticalRefreshInputs, TeamShape, TeamTacticalState,
};
use crate::{MatchRuntime, PlayerFieldPositionGroup};
#[cfg(feature = "match-logs")]
use crate::{match_log_debug, match_log_info};
use std::time::Instant;

// ───────────────────────────────────────────────────────────────────────────────
// FootballEngine — match orchestration
// ───────────────────────────────────────────────────────────────────────────────

/// Per-team weighted accumulator for the skill-composite averages
/// fed into `TacticalRefreshInputs`. Each per-player contribution
/// carries a position weight so a midfielder counts less than a
/// forward in `attacking_quality` and less than a defender in
/// `defensive_quality` — reflects how much each role actually drives
/// that phase of play.
///
/// The previous version added `mid_attack * 0.5` while still
/// incrementing the slot count by 1, which quartered the AM
/// contribution: half via the value, half via the denominator. A
/// proper weighted denominator fixes the bias so attacking
/// midfielders contribute meaningfully without overpowering forwards.
///
/// Position weights (per spec):
///   * Forward      — attacking 1.00, build-up 0.00, press 0.80, defensive 0.35
///   * Midfielder   — attacking 0.50, build-up 1.00, press 1.00, defensive 0.80
///   * Defender     — attacking 0.00, build-up 0.70, press 0.70, defensive 1.00
///   * Goalkeeper   — gk 1.00 only
struct SkillAccumulator {
    build_up_sum: f32,
    build_up_weight: f32,
    press_sum: f32,
    press_weight: f32,
    defensive_sum: f32,
    defensive_weight: f32,
    attacking_sum: f32,
    attacking_weight: f32,
    gk_sum: f32,
    gk_count: u32,
    conc_team_sum: f32,
    conc_team_count: u32,
    top_leadership: f32,
    keeper_voice: f32,
}

impl SkillAccumulator {
    fn new() -> Self {
        Self {
            build_up_sum: 0.0,
            build_up_weight: 0.0,
            press_sum: 0.0,
            press_weight: 0.0,
            defensive_sum: 0.0,
            defensive_weight: 0.0,
            attacking_sum: 0.0,
            attacking_weight: 0.0,
            gk_sum: 0.0,
            gk_count: 0,
            conc_team_sum: 0.0,
            conc_team_count: 0,
            top_leadership: 0.0,
            keeper_voice: 0.0,
        }
    }

    #[inline]
    fn accumulate(sum: &mut f32, weight: &mut f32, value: f32, w: f32) {
        if w > 0.0 {
            *sum += value * w;
            *weight += w;
        }
    }

    fn add(&mut self, p: &MatchPlayer, minute: u32) {
        // The loudest organising voice on the pitch, keeper included — a
        // shouting keeper marshals a back four as well as any captain,
        // and `on_field_leadership` reads the same attribute for both.
        self.top_leadership = self.top_leadership.max(sc::on_field_leadership(p, minute));

        let group = p.tactical_position.current_position.position_group();
        match group {
            PlayerFieldPositionGroup::Goalkeeper => {
                // Per spec: shot_stopping 0.45 + aerial/claim_cross 0.30
                // + distribution 0.25. `gk_claim_cross` is the active
                // cross-claim composite; `gk_aerial` covers the broader
                // aerial duel. Blend them 50/50 for the aerial slot
                // so a strong cross-claimer with weak overall aerial
                // duel still carries weight.
                let aerial_blend = 0.5 * (sc::gk_claim_cross(p, minute) + sc::gk_aerial(p, minute));
                let g = sc::gk_shot_stopping(p, minute) * 0.45
                    + aerial_blend * 0.30
                    + sc::gk_distribution(p, minute) * 0.25;
                self.gk_sum += g;
                self.gk_count += 1;
                // …and his ORGANISING voice, kept apart from his shot-stopping
                // because it is not a property of how good a goalkeeper he
                // is, it is a property of how well the ten in front of him
                // play. `sc::gk_communication` was written whole for this
                // and had no live caller in the match engine at all.
                self.keeper_voice = self.keeper_voice.max(sc::gk_communication(p, minute));
                self.add_conc_team(p, minute);
            }
            PlayerFieldPositionGroup::Defender => {
                Self::accumulate(
                    &mut self.build_up_sum,
                    &mut self.build_up_weight,
                    sc::passing_execution(p, minute),
                    0.70,
                );
                // Defenders: defensive_duel + interception + positioning.
                // Adding `defensive_positioning` so a CB who reads the
                // game well lifts the team's defensive_quality even
                // without elite tackling.
                let d = (sc::defensive_duel(p, minute)
                    + sc::interception(p, minute)
                    + sc::defensive_positioning(p, minute))
                    / 3.0;
                Self::accumulate(&mut self.defensive_sum, &mut self.defensive_weight, d, 1.00);
                Self::accumulate(
                    &mut self.press_sum,
                    &mut self.press_weight,
                    sc::pressing(p, minute),
                    0.70,
                );
                self.add_conc_team(p, minute);
            }
            PlayerFieldPositionGroup::Midfielder => {
                Self::accumulate(
                    &mut self.build_up_sum,
                    &mut self.build_up_weight,
                    sc::passing_execution(p, minute),
                    1.00,
                );
                let d = (sc::defensive_duel(p, minute)
                    + sc::interception(p, minute)
                    + sc::defensive_positioning(p, minute))
                    / 3.0;
                Self::accumulate(&mut self.defensive_sum, &mut self.defensive_weight, d, 0.80);
                Self::accumulate(
                    &mut self.press_sum,
                    &mut self.press_weight,
                    sc::pressing(p, minute),
                    1.00,
                );
                // Attacking midfielders contribute partial attacking
                // quality so an AM-heavy side reads as more dangerous
                // than a holding-mid-heavy side. Weight 0.50 on the
                // weighted denominator (was a buggy *0.5 + count++).
                let mid_attack = (sc::shooting_medium(p, minute)
                    + sc::off_ball_attack(p, minute)
                    + sc::pass_selection(p, minute))
                    / 3.0;
                Self::accumulate(
                    &mut self.attacking_sum,
                    &mut self.attacking_weight,
                    mid_attack,
                    0.50,
                );
                self.add_conc_team(p, minute);
            }
            PlayerFieldPositionGroup::Forward => {
                let a = (sc::shooting_close(p, minute)
                    + sc::dribble_attack(p, minute)
                    + sc::off_ball_attack(p, minute))
                    / 3.0;
                Self::accumulate(&mut self.attacking_sum, &mut self.attacking_weight, a, 1.00);
                Self::accumulate(
                    &mut self.press_sum,
                    &mut self.press_weight,
                    sc::pressing(p, minute),
                    0.80,
                );
                // Forwards contribute to defensive_quality at lower
                // weight via pressing + positioning — a hard-working
                // striker raises the defensive baseline without being
                // a CB.
                let f_def = 0.5 * (sc::defensive_positioning(p, minute) + sc::pressing(p, minute));
                Self::accumulate(
                    &mut self.defensive_sum,
                    &mut self.defensive_weight,
                    f_def,
                    0.35,
                );
                self.add_conc_team(p, minute);
            }
        }
    }

    fn add_conc_team(&mut self, p: &MatchPlayer, minute: u32) {
        // Use effective skills for the concentration / teamwork
        // average. Without `effective_skill` an exhausted side would
        // still register as "well-organised" purely on paper skill.
        let mental = sc::EffActionContext::mental(minute);
        let conc = sc::n(sc::eff(p, mental, |q| q.skills.mental.concentration));
        let team = sc::n(sc::eff(p, mental, |q| q.skills.mental.teamwork));
        self.conc_team_sum += (conc + team) * 0.5;
        self.conc_team_count += 1;
    }

    fn finalize(self) -> TeamSkillAggregates {
        let weighted = |sum: f32, weight: f32, default: f32| -> f32 {
            if weight <= 0.0 {
                default
            } else {
                (sum / weight).clamp(0.0, 1.0)
            }
        };
        let avg = |sum: f32, count: u32, default: f32| -> f32 {
            if count == 0 {
                default
            } else {
                (sum / count as f32).clamp(0.0, 1.0)
            }
        };
        TeamSkillAggregates {
            build_up_quality: weighted(self.build_up_sum, self.build_up_weight, 0.5),
            press_quality: weighted(self.press_sum, self.press_weight, 0.5),
            defensive_quality: weighted(self.defensive_sum, self.defensive_weight, 0.5),
            attacking_quality: weighted(self.attacking_sum, self.attacking_weight, 0.5),
            gk_quality: avg(self.gk_sum, self.gk_count, 0.5),
            concentration_teamwork_avg: avg(self.conc_team_sum, self.conc_team_count, 0.5),
            // A team with nobody on the pitch reads neutral, not silent.
            top_leadership: if self.top_leadership > 0.0 {
                self.top_leadership.clamp(0.0, 1.0)
            } else {
                0.5
            },
            // A side with no keeper on the pitch is organising itself.
            keeper_voice: if self.keeper_voice > 0.0 {
                self.keeper_voice.clamp(0.0, 1.0)
            } else {
                TeamSkillAggregates::KEEPER_VOICE_REFERENCE
            },
        }
    }
}

/// Cumulative-metric snapshot fed into `FootballEngine::build_rolling_metrics`.
/// Bundling the seven inputs into a struct keeps the call signature
/// stable as we add more counters and makes the `evaluate_coaches`
/// site less error-prone (no positional confusion between xg_for /
/// xg_against and the like).
#[derive(Debug, Clone, Copy, Default)]
pub(in crate::r#match::engine::engine) struct RollingMetricsInput {
    pub cum_xg_for: f32,
    pub cum_xg_against: f32,
    pub cum_shots_for: u32,
    pub cum_pressures: u32,
    pub cum_successful_pressures: u32,
    pub cum_deep_entries: u32,
    pub cum_dangerous_turnovers: u32,
}

pub struct FootballEngine<const W: usize, const H: usize> {}

impl<const W: usize, const H: usize> Default for FootballEngine<W, H> {
    fn default() -> Self {
        Self::new()
    }
}

mod diagnostics;
mod resolve;
mod run;
mod tick;
mod types;

// `phase_prof` was a public child of this module before the grouping, and
// the engine root re-exports this module with a glob, so it has to stay
// reachable as `crate::r#match::phase_prof`.
pub use diagnostics::phase_prof;

// The flat type surface. Listed rather than globbed so that the names the
// engine root carries onward are visible here — and so that no group
// module name leaks into that root namespace alongside them.
pub use types::{
    BallSide, MATCH_EXTRA_TIME_MS, MATCH_HALF_TIME_MS, MATCH_TIME_MS, MatchEvent, MatchFieldSize,
    MatchPlayerCollection, MatchTime, PlayMatchStateResult, PlayerEntry, TeamsTactics,
};

use crate::r#match::TeamSkillAggregates;

#[cfg(test)]
mod tests;

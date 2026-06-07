//! Selection-explanation builder. Translates a finished
//! [`PlayerSelectionResult`] (XI + bench) plus the original available
//! pool into structured `MatchSelectionContext` records for important
//! omissions — KeyPlayers left out, regulars demoted to the bench,
//! force-selected players overlooked. Consumed downstream by
//! `Player::on_match_dropped_with_context` so the player-events feed
//! can describe *who* was preferred and *why* instead of falling back
//! to a generic "Dropped from match squad" line.
//!
//! Importance gate is intentionally narrow — building a context for
//! every reserve every match would spam the events log. Only players
//! who would normally have featured (squad status, recent form, force
//! flag) qualify.
use crate::club::staff::{CoachDecisionEngine, CoachDecisionReason, CoachSelectionContext};
use crate::club::{PlayerFieldPositionGroup, PlayerPositionType, Staff};
use crate::r#match::player::MatchPlayer;
use crate::utils::DateUtils;
use crate::{
    MatchSelectionContext, Player, PlayerSquadStatus, SelectionComparison, SelectionDecisionScope,
    SelectionOmissionReason, SelectionRole, Tactics,
};
use chrono::NaiveDate;

use super::balance::LineupBalanceScorer;
use super::bench_scenarios::{BenchScenarioPlan, BenchScenarioScorer};
use super::model::{EligibilityDecision, EligibilityEvaluator, MatchSelectionGameModel};
use super::role_duty::{OpponentMatchupScorer, RoleDutyFitScorer, TacticalDuty};
use super::{DomesticCupContext, SelectionCompetition};
use super::helpers;
use super::scoring::{ScoringEngine, SlotScoreBreakdown};
use crate::HappinessEventType;
use std::collections::HashMap;

/// Output of the omissions builder — one entry per important
/// omission. The simulator ultimately surfaces the carried context on
/// the player's events feed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OmittedPlayer {
    pub player_id: u32,
    pub context: MatchSelectionContext,
}

pub(crate) struct OmissionBuilder<'a> {
    pub available: &'a [&'a Player],
    pub main_squad: &'a [MatchPlayer],
    pub substitutes: &'a [MatchPlayer],
    pub staff: &'a Staff,
    pub tactics: &'a Tactics,
    pub engine: &'a ScoringEngine,
    pub date: NaiveDate,
    pub is_friendly: bool,
    pub match_importance: f32,
    /// Present only for domestic cup ties — gates `CupRotation` so a
    /// league dead rubber doesn't borrow the cup-rotation copy.
    pub cup: Option<&'a DomesticCupContext>,
    /// Coach decision engine for the selecting side — used to enrich
    /// the chosen omission reason with the coach-memory read where it
    /// outweighs the raw factor-gap fallback.
    pub coach: Option<&'a CoachDecisionEngine<'a>>,
    pub competition: SelectionCompetition,
    /// Richer per-fixture model. When present the omission breakdown
    /// surfaces opponent-matchup, role/duty, balance, scenario,
    /// medical and eligibility factors so the rendered reason can
    /// reach the new variants instead of falling back to a generic
    /// quality / tactical mismatch line.
    pub game_model: Option<&'a MatchSelectionGameModel>,
}

impl<'a> OmissionBuilder<'a> {
    pub fn build(self) -> Vec<OmittedPlayer> {
        let mut out: Vec<OmittedPlayer> = Vec::new();

        let starter_ids: Vec<u32> = self.main_squad.iter().map(|p| p.id).collect();
        let bench_ids: Vec<u32> = self.substitutes.iter().map(|p| p.id).collect();

        let starter_players: Vec<&Player> = self
            .available
            .iter()
            .copied()
            .filter(|p| starter_ids.contains(&p.id))
            .collect();
        let bench_players: Vec<&Player> = self
            .available
            .iter()
            .copied()
            .filter(|p| bench_ids.contains(&p.id))
            .collect();

        for &player in self.available.iter() {
            if starter_ids.contains(&player.id) {
                continue;
            }

            let on_bench = bench_ids.contains(&player.id);
            if !Self::is_important(player, on_bench) {
                continue;
            }

            let scope = if on_bench {
                if Self::expects_to_start(player) {
                    SelectionDecisionScope::DroppedToBench
                } else {
                    SelectionDecisionScope::UnusedSubstitute
                }
            } else if self.match_importance < 0.4 {
                if DateUtils::age(player.birth_date, self.date) <= 21 {
                    SelectionDecisionScope::Rotation
                } else {
                    SelectionDecisionScope::Rotation
                }
            } else if self.is_load_managed(player) {
                SelectionDecisionScope::Rested
            } else {
                SelectionDecisionScope::LeftOutOfMatchdaySquad
            };

            let role = preferred_role(player, self.tactics);
            let comparison = self.build_comparison(player, &starter_players, &bench_players);
            let reason = self.choose_reason(player, scope, comparison.as_ref());

            let ctx = MatchSelectionContext {
                scope,
                reason,
                comparison,
                role,
                match_importance: self.match_importance,
                repeated: Self::omission_repeated(player),
                is_friendly: self.is_friendly,
            };

            out.push(OmittedPlayer {
                player_id: player.id,
                context: ctx,
            });
        }

        out
    }

    /// Important-omission gate. Filters the noise so only players
    /// whose absence is football-newsworthy generate a contextual
    /// drop event.
    fn is_important(player: &Player, on_bench: bool) -> bool {
        if player.is_force_match_selection {
            return true;
        }
        let status = player
            .contract
            .as_ref()
            .map(|c| c.squad_status.clone())
            .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
        match status {
            PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular => true,
            PlayerSquadStatus::FirstTeamSquadRotation => true,
            PlayerSquadStatus::HotProspectForTheFuture => !on_bench,
            _ => {
                if player.happiness.is_established_starter {
                    return true;
                }
                let games = player.statistics.played + player.cup_statistics.played;
                // Sample-size-regressed: a 5-app 7.0+ raw is often
                // 6.7 once regression is applied — protects against a
                // tiny-sample squad-rotation player being treated as a
                // disgruntled regular over a single hot week.
                let pos = player.position().position_group();
                games >= 5 && player.statistics.average_rating_realistic(pos) >= 7.0
            }
        }
    }

    fn expects_to_start(player: &Player) -> bool {
        let status = player
            .contract
            .as_ref()
            .map(|c| c.squad_status.clone())
            .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
        matches!(
            status,
            PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
        ) || player.happiness.is_established_starter
    }

    fn is_load_managed(&self, player: &Player) -> bool {
        let load = player
            .load
            .physical_load_7
            .max(player.load.minutes_last_7 * 0.95);
        load >= 360.0 || player.load.recovery_debt >= 120.0
    }

    fn omission_repeated(player: &Player) -> bool {
        let recent_drops = player
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                matches!(e.event_type, HappinessEventType::MatchDropped) && e.days_ago <= 14
            })
            .count();
        recent_drops >= 2
    }

    fn build_comparison(
        &self,
        omitted: &Player,
        starters: &[&Player],
        bench: &[&Player],
    ) -> Option<SelectionComparison> {
        let preferred_pos = best_natural_position(omitted, self.tactics);
        let slot_pos = preferred_pos?;
        let slot_group = slot_pos.position_group();

        let omitted_breakdown = self.score_breakdown(omitted, slot_pos, slot_group);

        let same_slot_starter = self
            .main_squad
            .iter()
            .find(|mp| mp.tactical_position.current_position == slot_pos)
            .and_then(|mp| starters.iter().copied().find(|p| p.id == mp.id));

        let same_slot_bench = self
            .substitutes
            .iter()
            .find(|mp| mp.tactical_position.current_position == slot_pos)
            .and_then(|mp| bench.iter().copied().find(|p| p.id == mp.id));

        let (rival, was_starter) = if let Some(p) = same_slot_starter {
            (Some(p), true)
        } else if let Some(p) = same_slot_bench {
            (Some(p), false)
        } else {
            self.find_group_rival(starters, bench, slot_group)
        };

        let rival = rival?;
        let rival_breakdown = self.score_breakdown(rival, slot_pos, slot_group);
        let top_factors = rival_breakdown.top_factors_against(&omitted_breakdown, 4);

        Some(SelectionComparison {
            selected_player_id: rival.id,
            selected_was_starter: was_starter,
            slot: Some(coarse_role(slot_pos)),
            selected_score: rival_breakdown.total(),
            omitted_score: omitted_breakdown.total(),
            top_factors,
        })
    }

    fn find_group_rival<'p>(
        &self,
        starters: &'p [&'p Player],
        bench: &'p [&'p Player],
        group: PlayerFieldPositionGroup,
    ) -> (Option<&'p Player>, bool) {
        let any_starter = starters
            .iter()
            .copied()
            .find(|p| p.position().position_group() == group);
        if let Some(p) = any_starter {
            return (Some(p), true);
        }
        let any_bench = bench
            .iter()
            .copied()
            .find(|p| p.position().position_group() == group);
        (any_bench, false)
    }

    fn score_breakdown(
        &self,
        player: &Player,
        slot: PlayerPositionType,
        group: PlayerFieldPositionGroup,
    ) -> SlotScoreBreakdown {
        let (_, mut b) = self.engine.score_player_for_slot_with_breakdown(
            player,
            slot,
            group,
            self.staff,
            self.tactics,
            self.date,
            self.is_friendly,
            &[],
        );
        // Fold in the external adjustments the competitive selector applies
        // on top of the pure slot score, so the comparison can explain a
        // backup/prospect winning the slot on opportunity/development rather
        // than raw quality. The pure breakdown leaves these at zero.
        b.development_minutes = self
            .engine
            .development_minutes_bonus(player, self.match_importance);
        b.injury_risk =
            -self
                .engine
                .injury_risk_penalty(player, self.match_importance, self.is_friendly);
        if let Some(cup) = self.cup {
            b.domestic_cup_opportunity = self
                .engine
                .domestic_cup_opportunity_bonus(player, cup, true);
        }
        // Future-aware pathway nudge, so a prospect winning a slot on a
        // development / succession call is explained rather than reading as a
        // raw-quality upset. Same-role checks compare against the full pool.
        b.future_pathway = self.engine.future_pathway_adjustment(
            player,
            slot,
            self.match_importance,
            self.date,
            self.cup,
            self.available,
            true,
        );
        // New layered factors — populated only when a richer game model is
        // wired in so the comparison line can surface opponent / role-duty
        // / scenario reasoning. Without the model these stay at zero and
        // the comparison shape is exactly as before.
        if let Some(model) = self.game_model {
            b.opponent_matchup =
                OpponentMatchupScorer::score(player, slot, &model.opponent_profile);
            let duty = self.duty_for_slot(slot);
            let fit = RoleDutyFitScorer::score(player, slot, duty);
            b.role_duty_fit = ((fit - 0.55) * 3.6).clamp(-1.8, 1.8);
            match EligibilityEvaluator::evaluate(player, &model.competition_rules) {
                EligibilityDecision::Eligible => {}
                EligibilityDecision::SoftLimited { penalty, .. } => {
                    b.eligibility_rule = -penalty;
                }
                EligibilityDecision::HardBlocked { .. } => {
                    b.eligibility_rule = -50.0;
                }
            }
            b.medical_risk =
                -self
                    .engine
                    .injury_risk_penalty(player, self.match_importance, self.is_friendly)
                    * (model.coach_policy.medical_caution - 0.5).max(0.0)
                    * 0.6;
            // Lineup balance contribution — small per-player share of the
            // whole-XI band the rival's profile uplifts. Cheap proxy: a
            // single-player evaluation, normalised to a comparable scale.
            let single_squad: Vec<MatchPlayer> = vec![MatchPlayer::from_player(
                0, player, slot, false,
            )];
            let mut singleton = HashMap::new();
            singleton.insert(player.id, player);
            let band = LineupBalanceScorer::evaluate(&single_squad, &singleton);
            // Use only the bands most relevant to the slot's group as the
            // per-player contribution.
            let per_player = match group {
                PlayerFieldPositionGroup::Goalkeeper => band.aerial_security * 0.02,
                PlayerFieldPositionGroup::Defender => {
                    (band.defensive_security + band.aerial_security + band.pace_recovery) * 0.012
                }
                PlayerFieldPositionGroup::Midfielder => {
                    (band.ball_progression + band.pressing_capacity) * 0.012
                }
                PlayerFieldPositionGroup::Forward => {
                    (band.chance_creation + band.pressing_capacity) * 0.012
                }
            };
            b.lineup_balance = per_player.clamp(-2.5, 2.5);
            // Bench scenario contribution — only meaningful for bench candidates,
            // surfaced here so the comparison can also flag scenario-coverage on
            // bench-vs-bench omissions.
            let plan = BenchScenarioPlan::build(model.match_type, model.tactical_objective);
            let cover = plan.cover_score(|scenario| {
                BenchScenarioScorer::coverage(player, scenario, self.date)
            });
            b.bench_scenario = (cover * 3.0).clamp(0.0, 3.0);
        }
        b
    }

    fn duty_for_slot(&self, slot: PlayerPositionType) -> TacticalDuty {
        use PlayerPositionType::*;
        match slot {
            Goalkeeper
            | Sweeper
            | DefenderCenter
            | DefenderCenterLeft
            | DefenderCenterRight
            | DefenderLeft
            | DefenderRight
            | DefensiveMidfielder => TacticalDuty::Defend,
            WingbackLeft
            | WingbackRight
            | MidfielderCenter
            | MidfielderCenterLeft
            | MidfielderCenterRight => TacticalDuty::Support,
            _ => TacticalDuty::Attack,
        }
    }

    /// Pick the dominant football-realistic reason from the
    /// comparison and player state. Order of checks is hand-tuned —
    /// the first matching atom wins, so concrete situational reasons
    /// (no role, returning, fatigue) trump generic "rival was better"
    /// fallbacks. The coach-memory pass runs *after* the situational
    /// reasons but *before* the factor-gap fallback so a sustained-
    /// poor-form omission reads as the coach's read, not a raw
    /// quality gap that may have only marginal explanatory power.
    fn choose_reason(
        &self,
        omitted: &Player,
        scope: SelectionDecisionScope,
        comparison: Option<&SelectionComparison>,
    ) -> SelectionOmissionReason {
        if best_natural_position(omitted, self.tactics).is_none() {
            return SelectionOmissionReason::NoNaturalRoleInFormation;
        }
        if omitted.player_attributes.is_in_recovery() {
            return SelectionOmissionReason::ReturningFromInjury;
        }
        if matches!(scope, SelectionDecisionScope::Rested) {
            return SelectionOmissionReason::FatigueManagement;
        }
        if matches!(scope, SelectionDecisionScope::Rotation) {
            // CupRotation is reserved for actual domestic cup ties — a
            // low-importance league fixture (dead rubber, congestion) reads
            // as LowMatchImportanceRotation instead.
            if self.cup.is_some() {
                let age = DateUtils::age(omitted.birth_date, self.date);
                if age <= 21 {
                    return SelectionOmissionReason::YouthDevelopmentRotation;
                }
                return SelectionOmissionReason::CupRotation;
            }
            return SelectionOmissionReason::LowMatchImportanceRotation;
        }
        // Coach memory pass — a sustained-poor-form / low-tactical-
        // trust read trumps the raw factor gap as the explanation.
        if let Some(reason) = self.coach_memory_reason(omitted) {
            return reason;
        }
        if let Some(c) = comparison {
            if let Some(top) = c.top_factors.first() {
                use crate::SelectionScoreFactor as F;
                return match top {
                    F::PerceivedQuality => SelectionOmissionReason::TeammatePreferredOnAbility,
                    F::MatchReadiness => SelectionOmissionReason::TeammatePreferredOnFitness,
                    F::Fatigue => SelectionOmissionReason::TeammatePreferredOnFitness,
                    F::PositionFit => SelectionOmissionReason::PositionFitIssue,
                    F::TacticalFit | F::SideFootFit => {
                        SelectionOmissionReason::TeammatePreferredForTacticalBalance
                    }
                    F::CoachRelationship => SelectionOmissionReason::TeammatePreferredOnTrust,
                    F::Newcomer => SelectionOmissionReason::NewcomerStillIntegrating,
                    F::SquadStatus => SelectionOmissionReason::SquadStatusMismatch,
                    F::ForceSelection => SelectionOmissionReason::ManagerDoesNotTrustPlayer,
                    F::TrainingImpression => SelectionOmissionReason::PoorRecentForm,
                    F::Cohesion => SelectionOmissionReason::TacticalMismatch,
                    F::Reputation => SelectionOmissionReason::TeammatePreferredOnAbility,
                    F::YouthPreference => SelectionOmissionReason::YouthDevelopmentRotation,
                    F::ClubPhilosophy => SelectionOmissionReason::TacticalMismatch,
                    F::DevelopmentMinutes => SelectionOmissionReason::LowMatchImportanceRotation,
                    F::CupOpportunity => SelectionOmissionReason::CupRotation,
                    F::InjuryRisk => SelectionOmissionReason::FitnessProtection,
                    F::OpponentMatchup => SelectionOmissionReason::OpponentMatchupMismatch,
                    F::RoleDutyFit => SelectionOmissionReason::PositionFitIssue,
                    F::LineupBalance => SelectionOmissionReason::LineupBalanceCall,
                    F::BenchScenario => SelectionOmissionReason::BenchScenarioCoverage,
                    F::MedicalRisk => SelectionOmissionReason::MedicalRecurrenceRisk,
                    F::EligibilityRule => SelectionOmissionReason::EligibilityRuleBlock,
                };
            }
        }

        let load = omitted
            .load
            .physical_load_7
            .max(omitted.load.minutes_last_7 * 0.95);
        if load >= 360.0 {
            return SelectionOmissionReason::FatigueManagement;
        }
        // Diagnose "poor form" against the regressed season average so
        // a single-match dip doesn't trigger the wrong omission reason.
        let pos = omitted.position().position_group();
        let regressed = omitted.statistics.average_rating_realistic(pos);
        if regressed > 0.0 && regressed < 6.3 {
            return SelectionOmissionReason::PoorRecentForm;
        }
        SelectionOmissionReason::TacticalMismatch
    }

    /// Map the coach engine's dominant decision reason into a
    /// [`SelectionOmissionReason`], or `None` when memory isn't wired
    /// in / the read doesn't yield a strong-enough negative signal.
    fn coach_memory_reason(&self, omitted: &Player) -> Option<SelectionOmissionReason> {
        let coach = self.coach?;
        let slot = best_natural_position(omitted, self.tactics)?;
        let natural_role_fit =
            (helpers::position_fit_score(omitted, slot, slot.position_group()) / 20.0)
                .clamp(0.0, 1.0);
        let coach_ctx = CoachSelectionContext {
            date: self.date,
            match_importance: self.match_importance,
            is_friendly: self.is_friendly,
            is_cup: matches!(
                self.competition,
                SelectionCompetition::DomesticCup { .. } | SelectionCompetition::ContinentalCup
            ),
            is_derby: false,
            is_continental: matches!(self.competition, SelectionCompetition::ContinentalCup),
            natural_role_fit,
            is_succession_heir: &[],
        };
        let assessment = coach.assess_player_for_selection(omitted, &coach_ctx);
        // Only surface a coach-memory reason when the coach's
        // selection_confidence is actually negative on the player —
        // otherwise the omission is driven by the factor gap, not
        // memory, and the renderer's existing copy fits better.
        if assessment.drop_risk < 0.55 {
            return None;
        }
        let dominant = assessment.dominant_reason()?;
        OmissionReasonMap::map(dominant)
    }
}

/// Stateless namespace mapping a [`CoachDecisionReason`] onto the
/// closest [`SelectionOmissionReason`] variant. Kept as a struct so
/// the conversion stays declarative and tests can drive it without
/// constructing a full `OmissionBuilder`.
struct OmissionReasonMap;

impl OmissionReasonMap {
    fn map(reason: CoachDecisionReason) -> Option<SelectionOmissionReason> {
        let mapped = match reason {
            CoachDecisionReason::PoorRecentForm => SelectionOmissionReason::PoorRecentForm,
            CoachDecisionReason::SustainedPoorForm => SelectionOmissionReason::PoorRecentForm,
            CoachDecisionReason::LowTacticalTrust => {
                SelectionOmissionReason::ManagerDoesNotTrustPlayer
            }
            CoachDecisionReason::StickyDoubt => SelectionOmissionReason::ManagerDoesNotTrustPlayer,
            CoachDecisionReason::BigMatchFailure => {
                SelectionOmissionReason::ManagerDoesNotTrustPlayer
            }
            CoachDecisionReason::RoleMismatch => SelectionOmissionReason::PositionFitIssue,
            CoachDecisionReason::FatigueRisk => SelectionOmissionReason::FatigueManagement,
            CoachDecisionReason::InjuryRisk => SelectionOmissionReason::FitnessProtection,
            // Positive reasons don't generate an omission — only used for diagnostics.
            CoachDecisionReason::StrongRecentForm
            | CoachDecisionReason::HighTacticalTrust
            | CoachDecisionReason::BigMatchReliability
            | CoachDecisionReason::TrainingLevel
            | CoachDecisionReason::CoachRelationship
            | CoachDecisionReason::DevelopmentPathway
            | CoachDecisionReason::SuccessionPlanning
            | CoachDecisionReason::ProtectingStar
            | CoachDecisionReason::TacticalNeed
            | CoachDecisionReason::LiveMatchUnderperformance
            | CoachDecisionReason::CostlyError
            | CoachDecisionReason::CardRisk => return None,
        };
        Some(mapped)
    }
}

/// Find the best position in the formation for a player. `None` if
/// none of the formation slots match the player's known positions —
/// signals "no natural role" to the reason chooser.
pub(crate) fn best_natural_position(
    player: &Player,
    tactics: &Tactics,
) -> Option<PlayerPositionType> {
    let mut best: Option<(PlayerPositionType, u8)> = None;
    for &pos in tactics.positions() {
        let level = player.positions.get_level(pos);
        if level == 0 {
            continue;
        }
        match best {
            Some((_, lvl)) if lvl >= level => {}
            _ => best = Some((pos, level)),
        }
    }
    best.map(|(p, _)| p)
}

/// Map a fine-grained `PlayerPositionType` to the coarse
/// render-friendly `SelectionRole` used in the comparison line.
fn coarse_role(pos: PlayerPositionType) -> SelectionRole {
    use PlayerPositionType::*;
    match pos {
        Goalkeeper => SelectionRole::Goalkeeper,
        DefenderCenter | DefenderCenterLeft | DefenderCenterRight => SelectionRole::CentreBack,
        DefenderLeft | DefenderRight | WingbackLeft | WingbackRight => SelectionRole::Fullback,
        DefensiveMidfielder => SelectionRole::DefensiveMidfielder,
        MidfielderCenter | MidfielderCenterLeft | MidfielderCenterRight => {
            SelectionRole::CentralMidfielder
        }
        AttackingMidfielderCenter => SelectionRole::AttackingMidfielder,
        AttackingMidfielderLeft | AttackingMidfielderRight | MidfielderLeft | MidfielderRight => {
            SelectionRole::Winger
        }
        Striker | ForwardCenter | ForwardLeft | ForwardRight => SelectionRole::Striker,
        _ => SelectionRole::Other,
    }
}

fn preferred_role(player: &Player, tactics: &Tactics) -> SelectionRole {
    if let Some(pos) = best_natural_position(player, tactics) {
        return coarse_role(pos);
    }
    coarse_role(player.position())
}

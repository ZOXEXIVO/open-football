use crate::club::PlayerPositionType;
use crate::utils::DateUtils;
use crate::Player;
use chrono::NaiveDate;

use super::model::{MatchTypeSignal, TacticalObjective};

/// Match-state scenario the bench should be ready for. The bench plan
/// is built from a weighted basket of these — instead of fixed
/// "DefensiveCover / MidfieldControl / …" roles, each scenario rewards
/// players whose profile actually solves that match state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BenchScenario {
    BackupGoalkeeper,
    ProtectLead,
    ChaseGoal,
    TacticalSwitchWide,
    TacticalSwitchCentral,
    FreshLegsPress,
    AerialPlanB,
    PenaltyShootout,
    YouthCameo,
    InjuryCoverDefence,
    InjuryCoverMidfield,
    InjuryCoverAttack,
}

/// Per-fixture weighting over [`BenchScenario`]. The selector reads it
/// to pick the bench: each scenario contributes to a bench candidate's
/// score proportional to its weight and the candidate's coverage.
#[derive(Debug, Clone)]
pub struct BenchScenarioPlan {
    pub weights: Vec<(BenchScenario, f32)>,
}

impl BenchScenarioPlan {
    /// Build the per-fixture weighting from the match type and the
    /// resolved tactical objective. Defaults follow the spec's tables.
    pub fn build(match_type: MatchTypeSignal, objective: TacticalObjective) -> Self {
        let mut weights = match (match_type, objective) {
            (MatchTypeSignal::CupFinal, _) | (MatchTypeSignal::ContinentalKnockout, _) => vec![
                (BenchScenario::BackupGoalkeeper, 0.12),
                (BenchScenario::InjuryCoverDefence, 0.10),
                (BenchScenario::InjuryCoverMidfield, 0.10),
                (BenchScenario::InjuryCoverAttack, 0.08),
                (BenchScenario::ChaseGoal, 0.16),
                (BenchScenario::ProtectLead, 0.16),
                (BenchScenario::PenaltyShootout, 0.12),
                (BenchScenario::TacticalSwitchWide, 0.06),
                (BenchScenario::TacticalSwitchCentral, 0.06),
                (BenchScenario::FreshLegsPress, 0.04),
            ],
            (MatchTypeSignal::CupEarlyRound, _) => vec![
                (BenchScenario::BackupGoalkeeper, 0.12),
                (BenchScenario::YouthCameo, 0.18),
                (BenchScenario::InjuryCoverDefence, 0.08),
                (BenchScenario::InjuryCoverMidfield, 0.08),
                (BenchScenario::InjuryCoverAttack, 0.08),
                (BenchScenario::FreshLegsPress, 0.14),
                (BenchScenario::ChaseGoal, 0.10),
                (BenchScenario::ProtectLead, 0.10),
                (BenchScenario::TacticalSwitchWide, 0.06),
                (BenchScenario::TacticalSwitchCentral, 0.06),
            ],
            (_, TacticalObjective::UnderdogAway | TacticalObjective::ProtectLead) => vec![
                (BenchScenario::BackupGoalkeeper, 0.12),
                (BenchScenario::ProtectLead, 0.20),
                (BenchScenario::FreshLegsPress, 0.14),
                (BenchScenario::AerialPlanB, 0.12),
                (BenchScenario::InjuryCoverDefence, 0.10),
                (BenchScenario::InjuryCoverMidfield, 0.09),
                (BenchScenario::InjuryCoverAttack, 0.08),
                (BenchScenario::ChaseGoal, 0.09),
                (BenchScenario::TacticalSwitchWide, 0.03),
                (BenchScenario::TacticalSwitchCentral, 0.03),
            ],
            (_, TacticalObjective::ChaseGame | TacticalObjective::FavoriteHome) => vec![
                (BenchScenario::BackupGoalkeeper, 0.12),
                (BenchScenario::ChaseGoal, 0.18),
                (BenchScenario::FreshLegsPress, 0.14),
                (BenchScenario::TacticalSwitchWide, 0.12),
                (BenchScenario::TacticalSwitchCentral, 0.10),
                (BenchScenario::ProtectLead, 0.10),
                (BenchScenario::InjuryCoverDefence, 0.08),
                (BenchScenario::InjuryCoverMidfield, 0.08),
                (BenchScenario::InjuryCoverAttack, 0.08),
            ],
            _ => vec![
                (BenchScenario::BackupGoalkeeper, 0.12),
                (BenchScenario::ChaseGoal, 0.14),
                (BenchScenario::ProtectLead, 0.14),
                (BenchScenario::FreshLegsPress, 0.12),
                (BenchScenario::TacticalSwitchWide, 0.08),
                (BenchScenario::TacticalSwitchCentral, 0.08),
                (BenchScenario::InjuryCoverDefence, 0.08),
                (BenchScenario::InjuryCoverMidfield, 0.08),
                (BenchScenario::InjuryCoverAttack, 0.08),
                (BenchScenario::AerialPlanB, 0.04),
                (BenchScenario::PenaltyShootout, 0.04),
            ],
        };

        // Penalty-shootout coverage on knockout ties when no starter is
        // a reliable taker is left to the bench scorer's penalty value
        // term — the weight is already pre-set above for finals.
        weights.retain(|(_, w)| *w > 0.0);
        BenchScenarioPlan { weights }
    }

    /// Fold a [`BenchScenarioScorer::coverage`] map into a single 0..1
    /// scenario-coverage score for a candidate.
    pub fn cover_score(&self, coverage: impl Fn(BenchScenario) -> f32) -> f32 {
        let mut acc = 0.0;
        for (scenario, weight) in &self.weights {
            acc += coverage(*scenario) * *weight;
        }
        acc.clamp(0.0, 1.0)
    }
}

/// Per-(scenario, player) coverage scorer. Returns 0..1 — the bench
/// scorer multiplies it by the plan weight and adds to the candidate's
/// total.
pub struct BenchScenarioScorer;

impl BenchScenarioScorer {
    pub fn coverage(
        player: &Player,
        scenario: BenchScenario,
        date: NaiveDate,
    ) -> f32 {
        let t = &player.skills.technical;
        let m = &player.skills.mental;
        let p = &player.skills.physical;
        let positions = &player.positions;
        let age = DateUtils::age(player.birth_date, date);

        let has = |pos: PlayerPositionType| positions.get_level(pos) > 0;
        let has_any = |targets: &[PlayerPositionType]| targets.iter().any(|&p| has(p));

        match scenario {
            BenchScenario::BackupGoalkeeper => {
                if has(PlayerPositionType::Goalkeeper) {
                    1.0
                } else {
                    0.0
                }
            }
            BenchScenario::ProtectLead => {
                if has_any(&[
                    PlayerPositionType::DefenderCenter,
                    PlayerPositionType::DefenderCenterLeft,
                    PlayerPositionType::DefenderCenterRight,
                    PlayerPositionType::DefensiveMidfielder,
                    PlayerPositionType::DefenderLeft,
                    PlayerPositionType::DefenderRight,
                    PlayerPositionType::WingbackLeft,
                    PlayerPositionType::WingbackRight,
                ]) {
                    let attr = (t.tackling + t.marking + m.positioning + m.concentration) / 80.0;
                    (0.5 + attr * 0.5).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
            BenchScenario::ChaseGoal => {
                let attacking = (t.finishing + t.dribbling + m.flair + m.off_the_ball) / 80.0;
                let attacker_pos = has_any(&[
                    PlayerPositionType::Striker,
                    PlayerPositionType::ForwardCenter,
                    PlayerPositionType::ForwardLeft,
                    PlayerPositionType::ForwardRight,
                    PlayerPositionType::AttackingMidfielderCenter,
                ]);
                if attacker_pos {
                    (0.55 + attacking * 0.45).clamp(0.0, 1.0)
                } else {
                    attacking.clamp(0.0, 0.6)
                }
            }
            BenchScenario::TacticalSwitchWide => {
                if has_any(&[
                    PlayerPositionType::DefenderLeft,
                    PlayerPositionType::DefenderRight,
                    PlayerPositionType::WingbackLeft,
                    PlayerPositionType::WingbackRight,
                    PlayerPositionType::MidfielderLeft,
                    PlayerPositionType::MidfielderRight,
                    PlayerPositionType::AttackingMidfielderLeft,
                    PlayerPositionType::AttackingMidfielderRight,
                    PlayerPositionType::ForwardLeft,
                    PlayerPositionType::ForwardRight,
                ]) {
                    1.0
                } else {
                    0.0
                }
            }
            BenchScenario::TacticalSwitchCentral => {
                if has_any(&[
                    PlayerPositionType::DefensiveMidfielder,
                    PlayerPositionType::MidfielderCenter,
                    PlayerPositionType::AttackingMidfielderCenter,
                ]) {
                    1.0
                } else {
                    0.0
                }
            }
            BenchScenario::FreshLegsPress => {
                let pressing = (m.work_rate + p.stamina + p.pace + m.aggression) / 80.0;
                let condition = player.player_attributes.condition_percentage() as f32 / 100.0;
                (pressing * 0.7 + condition * 0.3).clamp(0.0, 1.0)
            }
            BenchScenario::AerialPlanB => {
                if has_any(&[
                    PlayerPositionType::DefenderCenter,
                    PlayerPositionType::DefenderCenterLeft,
                    PlayerPositionType::DefenderCenterRight,
                    PlayerPositionType::Striker,
                    PlayerPositionType::ForwardCenter,
                ]) {
                    let aerial = (t.heading + p.jumping + p.strength) / 60.0;
                    aerial.clamp(0.0, 1.0)
                } else {
                    0.0
                }
            }
            BenchScenario::PenaltyShootout => {
                let take = (t.penalty_taking + m.composure + t.technique) / 60.0;
                take.clamp(0.0, 1.0)
            }
            BenchScenario::YouthCameo => {
                if age <= 19 {
                    1.0
                } else if age <= 22 {
                    0.6
                } else {
                    0.0
                }
            }
            BenchScenario::InjuryCoverDefence => {
                if has_any(&[
                    PlayerPositionType::DefenderCenter,
                    PlayerPositionType::DefenderCenterLeft,
                    PlayerPositionType::DefenderCenterRight,
                    PlayerPositionType::DefenderLeft,
                    PlayerPositionType::DefenderRight,
                    PlayerPositionType::WingbackLeft,
                    PlayerPositionType::WingbackRight,
                    PlayerPositionType::Sweeper,
                ]) {
                    1.0
                } else {
                    0.0
                }
            }
            BenchScenario::InjuryCoverMidfield => {
                if has_any(&[
                    PlayerPositionType::DefensiveMidfielder,
                    PlayerPositionType::MidfielderCenter,
                    PlayerPositionType::MidfielderCenterLeft,
                    PlayerPositionType::MidfielderCenterRight,
                    PlayerPositionType::MidfielderLeft,
                    PlayerPositionType::MidfielderRight,
                    PlayerPositionType::AttackingMidfielderCenter,
                ]) {
                    1.0
                } else {
                    0.0
                }
            }
            BenchScenario::InjuryCoverAttack => {
                if has_any(&[
                    PlayerPositionType::Striker,
                    PlayerPositionType::ForwardCenter,
                    PlayerPositionType::ForwardLeft,
                    PlayerPositionType::ForwardRight,
                    PlayerPositionType::AttackingMidfielderLeft,
                    PlayerPositionType::AttackingMidfielderRight,
                ]) {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    /// Per-player penalty-taking value, 0..1. Used by the bench scorer
    /// to add a small set-piece premium so a knockout final's bench
    /// always carries at least one credible shootout taker.
    pub fn penalty_value(player: &Player) -> f32 {
        let t = &player.skills.technical;
        let m = &player.skills.mental;
        ((t.penalty_taking * 2.0 + m.composure + t.technique) / 80.0).clamp(0.0, 1.0)
    }
}

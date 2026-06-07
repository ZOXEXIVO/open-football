use crate::club::PlayerPositionType;
use crate::r#match::player::MatchPlayer;
use crate::Player;
use std::collections::HashMap;

use super::model::{MatchSelectionGameModel, TacticalObjective};

/// Per-axis XI balance read. Each band is normalised so 0..100 reads
/// like a coach's mental scouting grade — the per-objective weights in
/// [`LineupBalanceWeights`] roll them into a single 0..100 score.
#[derive(Debug, Clone, Copy, Default)]
pub struct LineupBalanceReading {
    pub defensive_security: f32,
    pub ball_progression: f32,
    pub chance_creation: f32,
    pub pressing_capacity: f32,
    pub aerial_security: f32,
    pub set_piece_quality: f32,
    pub leadership_spine: f32,
    pub pace_recovery: f32,
    pub left_right_symmetry: f32,
}

/// Objective-driven weights folded over a [`LineupBalanceReading`].
#[derive(Debug, Clone, Copy)]
pub struct LineupBalanceWeights {
    pub security: f32,
    pub progression: f32,
    pub creation: f32,
    pub pressing: f32,
    pub aerial: f32,
    pub set_piece: f32,
    pub leadership: f32,
    pace: f32,
    symmetry: f32,
}

impl LineupBalanceWeights {
    pub fn for_objective(objective: TacticalObjective) -> Self {
        match objective {
            TacticalObjective::WinNowBalanced => LineupBalanceWeights {
                security: 0.18,
                progression: 0.16,
                creation: 0.16,
                pressing: 0.12,
                aerial: 0.10,
                set_piece: 0.08,
                leadership: 0.08,
                pace: 0.08,
                symmetry: 0.04,
            },
            TacticalObjective::ProtectLead | TacticalObjective::UnderdogAway => {
                LineupBalanceWeights {
                    security: 0.26,
                    progression: 0.10,
                    creation: 0.06,
                    pressing: 0.10,
                    aerial: 0.16,
                    set_piece: 0.05,
                    leadership: 0.12,
                    pace: 0.12,
                    symmetry: 0.03,
                }
            }
            TacticalObjective::ChaseGame | TacticalObjective::FavoriteHome => LineupBalanceWeights {
                security: 0.10,
                progression: 0.18,
                creation: 0.24,
                pressing: 0.14,
                aerial: 0.06,
                set_piece: 0.10,
                leadership: 0.05,
                pace: 0.10,
                symmetry: 0.03,
            },
            TacticalObjective::DevelopmentFixture => LineupBalanceWeights {
                security: 0.12,
                progression: 0.14,
                creation: 0.14,
                pressing: 0.12,
                aerial: 0.10,
                set_piece: 0.08,
                leadership: 0.08,
                pace: 0.10,
                symmetry: 0.12,
            },
        }
    }

    pub fn fold(&self, r: &LineupBalanceReading) -> f32 {
        r.defensive_security * self.security
            + r.ball_progression * self.progression
            + r.chance_creation * self.creation
            + r.pressing_capacity * self.pressing
            + r.aerial_security * self.aerial
            + r.set_piece_quality * self.set_piece
            + r.leadership_spine * self.leadership
            + r.pace_recovery * self.pace
            + r.left_right_symmetry * self.symmetry
    }
}

/// Scorer producing a [`LineupBalanceReading`] from an XI mapped to its
/// player records. Reads only the assigned slot and the player's
/// attributes — no opponent / context coupling — so the same scorer
/// drives the live evaluation and the post-swap "did this trade help?"
/// check inside [`BalanceSwapPass`].
pub struct LineupBalanceScorer;

impl LineupBalanceScorer {
    pub fn evaluate(
        squad: &[MatchPlayer],
        player_by_id: &HashMap<u32, &Player>,
    ) -> LineupBalanceReading {
        let mut def_sum = 0.0;
        let mut def_count = 0.0;
        let mut prog_sum = 0.0;
        let mut prog_count = 0.0;
        let mut crea_sum = 0.0;
        let mut crea_count = 0.0;
        let mut press_sum = 0.0;
        let mut aerial_sum = 0.0;
        let mut aerial_count = 0.0;
        let mut setp_sum = 0.0;
        let mut setp_count = 0.0;
        let mut leader_sum = 0.0;
        let mut pace_sum = 0.0;
        let mut pace_count = 0.0;
        let mut left_strength = 0.0;
        let mut right_strength = 0.0;

        for mp in squad {
            let Some(player) = player_by_id.get(&mp.id).copied() else {
                continue;
            };
            let slot = mp.tactical_position.current_position;
            let t = &player.skills.technical;
            let m = &player.skills.mental;
            let p = &player.skills.physical;

            // Leadership spine — average across the XI.
            leader_sum += m.leadership;

            // Defensive security: defenders + DM + GK shot stopping.
            if Self::is_defensive(slot) {
                let s = (t.tackling + t.marking + m.positioning + m.concentration) / 4.0;
                def_sum += s;
                def_count += 1.0;
            }

            // Pressing capacity — work rate / stamina over outfielders.
            if slot != PlayerPositionType::Goalkeeper {
                press_sum += (m.work_rate + p.stamina + m.aggression) / 3.0;
            }

            // Progression: passing + technique across mids/CBs.
            if Self::is_progressor(slot) {
                prog_sum += (t.passing + t.technique + m.vision) / 3.0;
                prog_count += 1.0;
            }

            // Creation: attacking mid + forwards vision / flair.
            if Self::is_creator(slot) {
                crea_sum += (m.vision + m.flair + t.dribbling + t.first_touch) / 4.0;
                crea_count += 1.0;
            }

            // Aerial security: CBs + GK + tall midfielders.
            if Self::is_aerial_target(slot) {
                aerial_sum += (t.heading + p.jumping + p.strength) / 3.0;
                aerial_count += 1.0;
            }

            // Set-piece quality: corners / free kicks / penalties.
            setp_sum += (t.corners + t.free_kicks + t.penalty_taking) / 3.0;
            setp_count += 1.0;

            // Pace recovery: defenders' pace + goalkeeper sweeping.
            if Self::is_defensive(slot) {
                let pace = if slot == PlayerPositionType::Goalkeeper {
                    (player.skills.goalkeeping.rushing_out + p.acceleration) / 2.0
                } else {
                    (p.pace + p.acceleration) / 2.0
                };
                pace_sum += pace;
                pace_count += 1.0;
            }

            // Symmetry: accumulate side strength.
            let strength_score = (t.passing + p.pace + m.work_rate + t.crossing) / 4.0;
            match slot {
                PlayerPositionType::DefenderLeft
                | PlayerPositionType::WingbackLeft
                | PlayerPositionType::MidfielderLeft
                | PlayerPositionType::AttackingMidfielderLeft
                | PlayerPositionType::ForwardLeft => left_strength += strength_score,
                PlayerPositionType::DefenderRight
                | PlayerPositionType::WingbackRight
                | PlayerPositionType::MidfielderRight
                | PlayerPositionType::AttackingMidfielderRight
                | PlayerPositionType::ForwardRight => right_strength += strength_score,
                _ => {}
            }
        }

        let normalise = |s: f32, n: f32| if n > 0.0 { (s / n) * 5.0 } else { 0.0 };
        let xi = squad.len().max(1) as f32;
        let symmetry_band = if left_strength + right_strength <= 0.0 {
            60.0
        } else {
            let total = left_strength + right_strength;
            let lopsided = ((left_strength - right_strength).abs() / total).clamp(0.0, 1.0);
            (1.0 - lopsided) * 100.0
        };

        LineupBalanceReading {
            defensive_security: normalise(def_sum, def_count).clamp(0.0, 100.0),
            ball_progression: normalise(prog_sum, prog_count).clamp(0.0, 100.0),
            chance_creation: normalise(crea_sum, crea_count).clamp(0.0, 100.0),
            pressing_capacity: (press_sum / xi * 5.0).clamp(0.0, 100.0),
            aerial_security: normalise(aerial_sum, aerial_count).clamp(0.0, 100.0),
            set_piece_quality: normalise(setp_sum, setp_count).clamp(0.0, 100.0),
            leadership_spine: (leader_sum / xi * 5.0).clamp(0.0, 100.0),
            pace_recovery: normalise(pace_sum, pace_count).clamp(0.0, 100.0),
            left_right_symmetry: symmetry_band,
        }
    }

    pub fn score(
        squad: &[MatchPlayer],
        player_by_id: &HashMap<u32, &Player>,
        objective: TacticalObjective,
    ) -> f32 {
        let r = Self::evaluate(squad, player_by_id);
        let w = LineupBalanceWeights::for_objective(objective);
        w.fold(&r)
    }

    fn is_defensive(slot: PlayerPositionType) -> bool {
        matches!(
            slot,
            PlayerPositionType::Goalkeeper
                | PlayerPositionType::Sweeper
                | PlayerPositionType::DefenderCenter
                | PlayerPositionType::DefenderCenterLeft
                | PlayerPositionType::DefenderCenterRight
                | PlayerPositionType::DefenderLeft
                | PlayerPositionType::DefenderRight
                | PlayerPositionType::WingbackLeft
                | PlayerPositionType::WingbackRight
                | PlayerPositionType::DefensiveMidfielder
        )
    }

    fn is_progressor(slot: PlayerPositionType) -> bool {
        matches!(
            slot,
            PlayerPositionType::DefenderCenter
                | PlayerPositionType::DefenderCenterLeft
                | PlayerPositionType::DefenderCenterRight
                | PlayerPositionType::DefensiveMidfielder
                | PlayerPositionType::MidfielderCenter
                | PlayerPositionType::MidfielderCenterLeft
                | PlayerPositionType::MidfielderCenterRight
                | PlayerPositionType::AttackingMidfielderCenter
        )
    }

    fn is_creator(slot: PlayerPositionType) -> bool {
        matches!(
            slot,
            PlayerPositionType::AttackingMidfielderCenter
                | PlayerPositionType::AttackingMidfielderLeft
                | PlayerPositionType::AttackingMidfielderRight
                | PlayerPositionType::MidfielderLeft
                | PlayerPositionType::MidfielderRight
                | PlayerPositionType::ForwardCenter
                | PlayerPositionType::ForwardLeft
                | PlayerPositionType::ForwardRight
                | PlayerPositionType::Striker
        )
    }

    fn is_aerial_target(slot: PlayerPositionType) -> bool {
        matches!(
            slot,
            PlayerPositionType::Goalkeeper
                | PlayerPositionType::DefenderCenter
                | PlayerPositionType::DefenderCenterLeft
                | PlayerPositionType::DefenderCenterRight
                | PlayerPositionType::ForwardCenter
                | PlayerPositionType::Striker
        )
    }
}

/// Post-assignment XI swap pass. For each starter, looks at same-group
/// bench candidates and proposes the swap that lifts the balance score
/// by more than the slot-score it costs (within the spec's caps).
/// Returns up to one proposal per starter — the caller applies them in
/// order.
pub struct BalanceSwapPass {
    /// Max slot score the swap may concede in high-importance matches.
    pub max_slot_loss_high: f32,
    /// Max slot score it may concede in early cup rotations.
    pub max_slot_loss_cup_early: f32,
    /// Minimum balance gain required to flip.
    pub min_balance_gain: f32,
}

impl BalanceSwapPass {
    pub fn standard() -> Self {
        BalanceSwapPass {
            max_slot_loss_high: 1.25,
            max_slot_loss_cup_early: 3.5,
            min_balance_gain: 0.25,
        }
    }

    /// Score gap allowed for this fixture's importance / rotation context.
    pub fn allowed_slot_loss(&self, match_importance: f32, is_cup_early: bool) -> f32 {
        if is_cup_early {
            self.max_slot_loss_cup_early
        } else if match_importance >= 0.7 {
            self.max_slot_loss_high
        } else {
            (self.max_slot_loss_high + 0.6).min(self.max_slot_loss_cup_early)
        }
    }
}

/// Stateless helper that picks the objective from the game model when
/// present, defaulting to `WinNowBalanced`.
pub struct ObjectiveResolver;

impl ObjectiveResolver {
    pub fn resolve(game_model: Option<&MatchSelectionGameModel>) -> TacticalObjective {
        game_model
            .map(|m| m.tactical_objective)
            .unwrap_or(TacticalObjective::WinNowBalanced)
    }
}

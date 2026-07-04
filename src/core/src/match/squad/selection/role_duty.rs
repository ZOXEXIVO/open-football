use crate::Player;
use crate::club::PlayerPositionType;

use super::model::OpponentSelectionProfile;

/// Whether the slot expects a defensive, supporting, or attacking duty.
/// Same role profile can be set up differently (defensive fullback vs
/// overlapping fullback) — the duty steers the attribute blend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TacticalDuty {
    Defend,
    Support,
    Attack,
}

/// Per-slot role profile. Maps a `PlayerPositionType` (plus optional
/// duty inferred from the formation context) into a richer description
/// of what the manager actually wants in that slot — used by
/// [`RoleDutyFitScorer`] to evaluate role-attribute fit on top of the
/// raw position level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionRoleProfile {
    SweeperKeeper,
    LineKeeper,
    BallPlayingCentreBack,
    StopperCentreBack,
    DefensiveFullback,
    OverlappingFullback,
    HoldingMidfielder,
    BallWinner,
    DeepPlaymaker,
    BoxToBoxMidfielder,
    AdvancedPlaymaker,
    TouchlineWinger,
    InvertedWinger,
    PressingForward,
    TargetForward,
    Poacher,
}

/// Stateless resolver from `(PlayerPositionType, TacticalDuty)` to a
/// [`SelectionRoleProfile`]. The duty defaults to `Support` when the
/// caller doesn't carry an explicit per-slot duty in the tactic
/// definition yet — the result is a sensible neutral profile.
pub struct RoleProfileResolver;

impl RoleProfileResolver {
    pub fn resolve(slot: PlayerPositionType, duty: TacticalDuty) -> SelectionRoleProfile {
        use PlayerPositionType::*;
        match (slot, duty) {
            (Goalkeeper, TacticalDuty::Defend) => SelectionRoleProfile::LineKeeper,
            (Goalkeeper, _) => SelectionRoleProfile::SweeperKeeper,
            (DefenderCenter | DefenderCenterLeft | DefenderCenterRight, TacticalDuty::Support)
            | (DefenderCenter | DefenderCenterLeft | DefenderCenterRight, TacticalDuty::Attack) => {
                SelectionRoleProfile::BallPlayingCentreBack
            }
            (DefenderCenter | DefenderCenterLeft | DefenderCenterRight, TacticalDuty::Defend) => {
                SelectionRoleProfile::StopperCentreBack
            }
            (DefenderLeft | DefenderRight, TacticalDuty::Defend) => {
                SelectionRoleProfile::DefensiveFullback
            }
            (DefenderLeft | DefenderRight | WingbackLeft | WingbackRight, _) => {
                SelectionRoleProfile::OverlappingFullback
            }
            (DefensiveMidfielder, TacticalDuty::Defend) => SelectionRoleProfile::HoldingMidfielder,
            (DefensiveMidfielder, _) => SelectionRoleProfile::BallWinner,
            (
                MidfielderCenter | MidfielderCenterLeft | MidfielderCenterRight,
                TacticalDuty::Defend,
            ) => SelectionRoleProfile::HoldingMidfielder,
            (
                MidfielderCenter | MidfielderCenterLeft | MidfielderCenterRight,
                TacticalDuty::Support,
            ) => SelectionRoleProfile::BoxToBoxMidfielder,
            (
                MidfielderCenter | MidfielderCenterLeft | MidfielderCenterRight,
                TacticalDuty::Attack,
            ) => SelectionRoleProfile::DeepPlaymaker,
            (AttackingMidfielderCenter, _) => SelectionRoleProfile::AdvancedPlaymaker,
            (
                AttackingMidfielderLeft
                | AttackingMidfielderRight
                | MidfielderLeft
                | MidfielderRight,
                TacticalDuty::Defend,
            ) => SelectionRoleProfile::TouchlineWinger,
            (
                AttackingMidfielderLeft
                | AttackingMidfielderRight
                | MidfielderLeft
                | MidfielderRight,
                _,
            ) => SelectionRoleProfile::InvertedWinger,
            (Striker, TacticalDuty::Defend) => SelectionRoleProfile::PressingForward,
            (Striker, TacticalDuty::Support) => SelectionRoleProfile::TargetForward,
            (Striker | ForwardCenter, TacticalDuty::Attack) => SelectionRoleProfile::Poacher,
            (ForwardCenter, _) => SelectionRoleProfile::TargetForward,
            (ForwardLeft | ForwardRight, _) => SelectionRoleProfile::InvertedWinger,
            _ => SelectionRoleProfile::BoxToBoxMidfielder,
        }
    }
}

/// Computes role/duty-aware fit on top of the raw position level.
/// Output is on a 0..1 scale — callers fold it into the broader slot
/// score with their own weights. Composition follows the spec:
/// `0.45 * position_level + 0.40 * role_attribute_fit + 0.15 * duty_fit`.
pub struct RoleDutyFitScorer;

impl RoleDutyFitScorer {
    pub fn score(player: &Player, slot: PlayerPositionType, duty: TacticalDuty) -> f32 {
        let profile = RoleProfileResolver::resolve(slot, duty);
        let level = player.positions.get_level(slot) as f32 / 20.0;
        let attribute_fit = Self::role_attribute_fit(player, profile);
        let duty_fit = Self::duty_fit(player, duty);
        (level * 0.45 + attribute_fit * 0.40 + duty_fit * 0.15).clamp(0.0, 1.0)
    }

    /// 0..1 weighted attribute fit for the resolved role profile. The
    /// blend tables live here in named match arms — the spec wants
    /// declarative role recipes, not magic numbers buried inline at the
    /// scoring call site.
    pub fn role_attribute_fit(player: &Player, profile: SelectionRoleProfile) -> f32 {
        let t = &player.skills.technical;
        let m = &player.skills.mental;
        let p = &player.skills.physical;
        let gk = &player.skills.goalkeeping;
        let v = match profile {
            SelectionRoleProfile::SweeperKeeper => {
                gk.handling * 0.16
                    + gk.reflexes * 0.16
                    + gk.rushing_out * 0.16
                    + gk.kicking * 0.14
                    + gk.command_of_area * 0.10
                    + m.decisions * 0.14
                    + m.composure * 0.14
            }
            SelectionRoleProfile::LineKeeper => {
                gk.handling * 0.22
                    + gk.reflexes * 0.22
                    + gk.aerial_reach * 0.16
                    + gk.one_on_ones * 0.14
                    + m.positioning * 0.14
                    + m.concentration * 0.12
            }
            SelectionRoleProfile::BallPlayingCentreBack => {
                t.passing * 0.18
                    + t.technique * 0.10
                    + m.composure * 0.16
                    + m.decisions * 0.12
                    + m.positioning * 0.12
                    + t.heading * 0.10
                    + t.tackling * 0.12
                    + p.pace * 0.10
            }
            SelectionRoleProfile::StopperCentreBack => {
                t.tackling * 0.18
                    + t.marking * 0.16
                    + t.heading * 0.16
                    + p.strength * 0.14
                    + m.bravery * 0.12
                    + m.positioning * 0.12
                    + m.concentration * 0.12
            }
            SelectionRoleProfile::DefensiveFullback => {
                t.tackling * 0.18
                    + t.marking * 0.18
                    + m.positioning * 0.16
                    + p.pace * 0.12
                    + p.stamina * 0.12
                    + m.concentration * 0.12
                    + p.strength * 0.12
            }
            SelectionRoleProfile::OverlappingFullback => {
                p.pace * 0.18
                    + p.stamina * 0.16
                    + t.crossing * 0.16
                    + t.dribbling * 0.12
                    + m.work_rate * 0.14
                    + t.tackling * 0.12
                    + m.off_the_ball * 0.12
            }
            SelectionRoleProfile::HoldingMidfielder => {
                m.positioning * 0.18
                    + m.anticipation * 0.14
                    + t.tackling * 0.14
                    + m.decisions * 0.14
                    + m.teamwork * 0.12
                    + t.passing * 0.12
                    + p.stamina * 0.08
                    + p.strength * 0.08
            }
            SelectionRoleProfile::BallWinner => {
                t.tackling * 0.20
                    + m.aggression * 0.16
                    + m.bravery * 0.14
                    + p.stamina * 0.14
                    + m.work_rate * 0.14
                    + m.anticipation * 0.12
                    + p.strength * 0.10
            }
            SelectionRoleProfile::DeepPlaymaker => {
                t.passing * 0.22
                    + m.vision * 0.20
                    + t.technique * 0.16
                    + m.composure * 0.14
                    + m.decisions * 0.14
                    + t.first_touch * 0.14
            }
            SelectionRoleProfile::BoxToBoxMidfielder => {
                p.stamina * 0.16
                    + m.work_rate * 0.16
                    + t.passing * 0.14
                    + t.tackling * 0.12
                    + p.pace * 0.10
                    + m.off_the_ball * 0.12
                    + m.teamwork * 0.10
                    + t.finishing * 0.10
            }
            SelectionRoleProfile::AdvancedPlaymaker => {
                m.vision * 0.20
                    + t.passing * 0.18
                    + t.technique * 0.16
                    + t.first_touch * 0.14
                    + m.decisions * 0.12
                    + m.flair * 0.10
                    + m.composure * 0.10
            }
            SelectionRoleProfile::TouchlineWinger => {
                t.crossing * 0.20
                    + p.pace * 0.18
                    + p.acceleration * 0.14
                    + t.dribbling * 0.16
                    + p.stamina * 0.12
                    + m.work_rate * 0.10
                    + t.technique * 0.10
            }
            SelectionRoleProfile::InvertedWinger => {
                t.dribbling * 0.18
                    + t.long_shots * 0.14
                    + t.technique * 0.16
                    + m.flair * 0.12
                    + m.off_the_ball * 0.12
                    + p.pace * 0.14
                    + t.first_touch * 0.14
            }
            SelectionRoleProfile::PressingForward => {
                m.work_rate * 0.18
                    + p.stamina * 0.16
                    + p.pace * 0.14
                    + m.bravery * 0.12
                    + m.off_the_ball * 0.12
                    + m.teamwork * 0.10
                    + t.finishing * 0.10
                    + p.strength * 0.08
            }
            SelectionRoleProfile::TargetForward => {
                p.strength * 0.20
                    + t.heading * 0.18
                    + m.bravery * 0.14
                    + t.first_touch * 0.14
                    + t.finishing * 0.14
                    + m.off_the_ball * 0.10
                    + p.jumping * 0.10
            }
            SelectionRoleProfile::Poacher => {
                t.finishing * 0.24
                    + m.off_the_ball * 0.20
                    + m.anticipation * 0.16
                    + m.composure * 0.14
                    + p.acceleration * 0.14
                    + m.concentration * 0.12
            }
        };
        (v / 20.0).clamp(0.0, 1.0)
    }

    /// Lightweight duty fit: how well the player's work-rate / discipline
    /// profile matches the duty. Defensive duties reward positioning and
    /// teamwork; attack duties reward flair and finishing.
    pub fn duty_fit(player: &Player, duty: TacticalDuty) -> f32 {
        let m = &player.skills.mental;
        let v = match duty {
            TacticalDuty::Defend => {
                (m.positioning + m.teamwork + m.work_rate + m.concentration) / 4.0
            }
            TacticalDuty::Support => (m.teamwork + m.work_rate + m.decisions + m.vision) / 4.0,
            TacticalDuty::Attack => (m.flair + m.off_the_ball + m.work_rate + m.anticipation) / 4.0,
        };
        (v / 20.0).clamp(0.0, 1.0)
    }
}

/// Per-slot opponent matchup adjustment. Returns a small signed nudge
/// (capped to roughly +/-2.5 by the spec) that rewards selecting a
/// player whose physical / mental profile counters the opponent's
/// stated strength, and penalises a clear vulnerability.
pub struct OpponentMatchupScorer;

impl OpponentMatchupScorer {
    pub fn score(
        player: &Player,
        slot: PlayerPositionType,
        opponent: &OpponentSelectionProfile,
    ) -> f32 {
        let p = &player.skills.physical;
        let m = &player.skills.mental;
        let t = &player.skills.technical;
        let gk = &player.skills.goalkeeping;

        let mut sum = 0.0f32;

        // Pace threat — rewarded against quick defenders and central
        // backs, penalises slow ones in the same slots.
        if Self::is_defensive_outfield(slot) {
            let pace = (p.pace + p.acceleration + m.positioning) / 60.0;
            let bonus = ((opponent.pace_threat - 0.4) * pace * 2.4).clamp(-1.8, 1.4);
            sum += bonus;
        }

        // Aerial threat — rewards aerial-strong centre-backs and keepers.
        if matches!(
            slot,
            PlayerPositionType::Goalkeeper
                | PlayerPositionType::DefenderCenter
                | PlayerPositionType::DefenderCenterLeft
                | PlayerPositionType::DefenderCenterRight
        ) {
            let aerial = if slot == PlayerPositionType::Goalkeeper {
                (gk.aerial_reach + gk.command_of_area + p.jumping + p.strength) / 80.0
            } else {
                (t.heading + p.jumping + p.strength) / 60.0
            };
            let bonus = ((opponent.aerial_threat - 0.4) * aerial * 2.4).clamp(-1.5, 1.4);
            sum += bonus;
        }

        // High press — composure / first_touch / passing rewarded
        // everywhere on the pitch (it's an XI-wide test).
        let buildup = (m.composure + t.first_touch + t.passing + m.decisions) / 80.0;
        let press_bonus = ((opponent.pressing_intensity - 0.4) * buildup * 2.0).clamp(-1.2, 1.2);
        sum += press_bonus;

        // Low block — creators thrive, journeymen don't add much.
        if Self::is_attacking_outfield(slot) {
            let unlock = (m.vision + t.technique + t.long_shots + m.flair + t.crossing) / 100.0;
            let bonus = ((opponent.low_block_likelihood - 0.3) * unlock * 2.0).clamp(0.0, 1.1);
            sum += bonus;
        }

        // Wide threats — same-side fullback / wingback bonus.
        let wide_signal = match slot {
            PlayerPositionType::DefenderLeft | PlayerPositionType::WingbackLeft => {
                opponent.wide_threat_right
            }
            PlayerPositionType::DefenderRight | PlayerPositionType::WingbackRight => {
                opponent.wide_threat_left
            }
            _ => 0.0,
        };
        if wide_signal > 0.0 {
            let defensive = (t.tackling + m.positioning + p.pace) / 60.0;
            let bonus = ((wide_signal - 0.3) * defensive * 2.0).clamp(-1.0, 1.2);
            sum += bonus;
        }

        sum.clamp(-2.5, 2.5)
    }

    fn is_defensive_outfield(slot: PlayerPositionType) -> bool {
        matches!(
            slot,
            PlayerPositionType::DefenderCenter
                | PlayerPositionType::DefenderCenterLeft
                | PlayerPositionType::DefenderCenterRight
                | PlayerPositionType::DefenderLeft
                | PlayerPositionType::DefenderRight
                | PlayerPositionType::WingbackLeft
                | PlayerPositionType::WingbackRight
                | PlayerPositionType::Sweeper
                | PlayerPositionType::DefensiveMidfielder
        )
    }

    fn is_attacking_outfield(slot: PlayerPositionType) -> bool {
        matches!(
            slot,
            PlayerPositionType::AttackingMidfielderCenter
                | PlayerPositionType::AttackingMidfielderLeft
                | PlayerPositionType::AttackingMidfielderRight
                | PlayerPositionType::MidfielderLeft
                | PlayerPositionType::MidfielderRight
                | PlayerPositionType::Striker
                | PlayerPositionType::ForwardCenter
                | PlayerPositionType::ForwardLeft
                | PlayerPositionType::ForwardRight
        )
    }
}

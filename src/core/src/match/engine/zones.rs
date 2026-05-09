use crate::r#match::PlayerSide;
use crate::r#match::engine::context::PenaltyArea;
use nalgebra::Vector3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchZone {
    OwnSixYardBox,
    OwnPenaltyArea,
    DefensiveThird,
    MiddleThird,
    FinalThird,
    OppositionPenaltyArea,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LateralLane {
    WideLeft,
    HalfSpaceLeft,
    CentralLane,
    HalfSpaceRight,
    WideRight,
}

impl MatchZone {
    /// Classify a pitch position from the `side`'s point of view.
    /// Box variants take priority over third bands so an opponent-box
    /// shot inside the final third reads as `OppositionPenaltyArea`,
    /// not `FinalThird`. The six-yard zone is approximated as the
    /// inner third of the penalty area along the goal-line axis.
    pub fn classify(
        position: Vector3<f32>,
        side: PlayerSide,
        field_width: f32,
        own_box: PenaltyArea,
        opp_box: PenaltyArea,
    ) -> Self {
        if own_box.contains(&position) {
            if Self::is_inner_third_of_box(&position, &own_box, side) {
                return MatchZone::OwnSixYardBox;
            }
            return MatchZone::OwnPenaltyArea;
        }
        if opp_box.contains(&position) {
            return MatchZone::OppositionPenaltyArea;
        }
        let progress = side.attacking_progress_x(position.x, field_width);
        if progress >= 2.0 / 3.0 {
            MatchZone::FinalThird
        } else if progress >= 1.0 / 3.0 {
            MatchZone::MiddleThird
        } else {
            MatchZone::DefensiveThird
        }
    }

    /// Six-yard band: inner third of the penalty-area depth on the
    /// goal-line side. Approximation — engine has no explicit six-yard
    /// rectangle.
    fn is_inner_third_of_box(
        position: &Vector3<f32>,
        box_area: &PenaltyArea,
        side: PlayerSide,
    ) -> bool {
        let depth = box_area.max.x - box_area.min.x;
        let inner = depth / 3.0;
        match side {
            PlayerSide::Left => position.x <= box_area.min.x + inner,
            PlayerSide::Right => position.x >= box_area.max.x - inner,
        }
    }

    pub fn is_own_box(self) -> bool {
        matches!(self, MatchZone::OwnSixYardBox | MatchZone::OwnPenaltyArea)
    }

    pub fn is_own_six_yard(self) -> bool {
        matches!(self, MatchZone::OwnSixYardBox)
    }

    pub fn is_own_third(self) -> bool {
        matches!(
            self,
            MatchZone::OwnSixYardBox | MatchZone::OwnPenaltyArea | MatchZone::DefensiveThird
        )
    }

    pub fn is_middle_third(self) -> bool {
        matches!(self, MatchZone::MiddleThird)
    }

    pub fn is_final_third(self) -> bool {
        matches!(
            self,
            MatchZone::FinalThird | MatchZone::OppositionPenaltyArea
        )
    }

    pub fn is_opposition_box(self) -> bool {
        matches!(self, MatchZone::OppositionPenaltyArea)
    }
}

impl LateralLane {
    /// Five vertical lanes — wide / halfspace / central — split evenly
    /// along the pitch's y-axis. Used by event-classification code (cross
    /// candidacy, halfspace receivers).
    pub fn classify(y: f32, field_height: f32) -> Self {
        if field_height <= 0.0 {
            return LateralLane::CentralLane;
        }
        let frac = (y / field_height).clamp(0.0, 1.0);
        if frac < 0.20 {
            LateralLane::WideLeft
        } else if frac < 0.40 {
            LateralLane::HalfSpaceLeft
        } else if frac < 0.60 {
            LateralLane::CentralLane
        } else if frac < 0.80 {
            LateralLane::HalfSpaceRight
        } else {
            LateralLane::WideRight
        }
    }

    pub fn is_wide(self) -> bool {
        matches!(self, LateralLane::WideLeft | LateralLane::WideRight)
    }
}

/// Per-zone counters carried alongside the raw action totals on
/// `MatchPlayerStatistics`. Populated by engine event handlers that
/// know the action's pitch location (foul handler, tackle handler,
/// pass-completion handler, …); read by the rating helper to apply
/// per-zone multipliers without needing the full action stream at
/// scoring time.
///
/// Defaults are zero — legacy callers and the in-engine paths that
/// haven't been updated to record zones yet still produce sensible
/// ratings, just without the zone-aware bumps.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ZoneStats {
    pub tackles_own_box: u16,
    pub tackles_own_six_yard: u16,
    pub tackles_final_third: u16,
    pub interceptions_own_box: u16,
    pub interceptions_own_six_yard: u16,
    pub interceptions_middle_third: u16,
    pub blocks_own_box: u16,
    pub blocks_own_six_yard: u16,
    pub clearances_own_box: u16,
    pub clearances_own_six_yard: u16,
    pub pressures_won_final_third: u16,

    pub progressive_passes_into_final_third: u16,
    pub progressive_carries_into_final_third: u16,
    pub carries_into_box: u16,

    /// Completed passes into the opposition box originating from a
    /// half-space lane. Real football's most threatening creation
    /// channel — overlapping fullback / inverted winger / between-the-
    /// lines midfielder all live here.
    pub half_space_passes_into_box: u16,
    /// Completed passes into the opposition box originating from the
    /// central lane. Less common (defenders compress the middle), but
    /// when they happen they tend to produce high-quality chances.
    pub central_passes_into_box: u16,
    /// Switch-of-play passes: long lateral balls that travel from one
    /// wide / half-space lane to the OPPOSITE wide / half-space lane
    /// in a single pass, completing the change of point of attack.
    pub switches_of_play: u16,

    pub dangerous_turnovers_own_third: u16,
    pub dangerous_turnovers_own_box: u16,

    /// Errors-to-goal where the original giveaway happened inside the
    /// player's own penalty area. Layered on top of the base
    /// `errors_leading_to_goal` count so a goal-mouth howler hits harder
    /// than a midfield error that became a goal.
    pub errors_to_goal_own_box: u16,

    /// GK cross-claims, punches, sweeper interceptions and high
    /// claims — collectively "command-zone" actions worth a small
    /// per-event credit independent of saves.
    pub gk_command_actions: u16,
    /// Failed cross-claim attempts that immediately produced a shot
    /// or a goal. Helpers + rating coefficients are deliberately kept
    /// in place; the live producer is intentionally deferred until the
    /// GK state machine distinguishes "attempted to claim and missed"
    /// from "wisely stayed on the line". Default zero — rating impact
    /// is zero until that producer lands.
    pub gk_failed_claims_to_shot: u16,
    pub gk_failed_claims_to_goal: u16,

    pub penalty_fouls_conceded: u16,
    /// Defender / GK fouls inside the team's own third.
    pub own_third_def_fouls: u16,
}

/// Centralised coefficient lookup. Keeping the zone bumps and discipline
/// penalties in one struct makes rating tuning a single-file change and
/// keeps the rating helper readable as orchestration.
pub struct ZoneCoeffs;

impl ZoneCoeffs {
    pub const DEF_OWN_BOX_BONUS: f32 = 0.35;
    pub const DEF_OWN_SIX_YARD_BONUS: f32 = 0.60;
    pub const INTERCEPTION_MIDDLE_BONUS: f32 = 0.05;
    pub const TACKLE_FINAL_THIRD_BONUS: f32 = 0.15;
    pub const PRESSURE_FINAL_THIRD_BONUS: f32 = 0.15;

    pub const DEF_ZONE_BONUS_CAP: f32 = 0.60;

    pub const PROGRESSIVE_TO_FINAL_THIRD_PER: f32 = 0.03;
    pub const PROGRESSIVE_TO_FINAL_THIRD_CAP: f32 = 0.20;
    pub const BOX_ENTRY_PER: f32 = 0.05;
    pub const BOX_ENTRY_CAP: f32 = 0.25;

    /// Half-space pass into the opposition box — small per-event
    /// credit on top of the regular `passes_into_box` line.
    pub const HALF_SPACE_BOX_ENTRY_PER: f32 = 0.04;
    pub const HALF_SPACE_BOX_ENTRY_CAP: f32 = 0.20;
    /// Central-channel pass into the opposition box — slightly more
    /// per-event because central balls beat a more compact defence.
    pub const CENTRAL_BOX_ENTRY_PER: f32 = 0.05;
    pub const CENTRAL_BOX_ENTRY_CAP: f32 = 0.20;
    /// Switch of play (long cross-lane diagonal) — rewards a recycled
    /// attack and penalises the opposite side's defence.
    pub const SWITCH_OF_PLAY_PER: f32 = 0.025;
    pub const SWITCH_OF_PLAY_CAP: f32 = 0.15;

    pub const TURNOVER_OWN_THIRD: f32 = -0.20;
    pub const TURNOVER_OWN_BOX: f32 = -0.45;

    pub const ERROR_TO_GOAL_OWN_BOX_EXTRA: f32 = -0.35;

    pub const GK_COMMAND_PER: f32 = 0.04;
    pub const GK_COMMAND_CAP: f32 = 0.25;
    pub const GK_FAILED_CLAIM_TO_SHOT: f32 = -0.35;
    pub const GK_FAILED_CLAIM_TO_GOAL: f32 = -0.90;

    pub const FOUL_PER: f32 = -0.03;
    pub const FOUL_CAP: f32 = -0.18;
    pub const FOUL_OWN_THIRD_DEF_EXTRA_PER: f32 = -0.05;
    pub const FOUL_PENALTY: f32 = -0.35;

    pub const OFFSIDE_FORWARD_PER: f32 = -0.06;
    pub const OFFSIDE_FORWARD_CAP: f32 = -0.24;
    pub const OFFSIDE_OTHER_PER: f32 = -0.04;
    pub const OFFSIDE_OTHER_CAP: f32 = -0.12;

    pub const OWN_GOAL_BASE: f32 = -1.00;
    /// Extra penalty per OG inside own box. OGs are by definition in
    /// the player's own goal mouth, so this fires once per OG.
    pub const OWN_GOAL_OWN_BOX_EXTRA: f32 = -0.30;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pa(min_x: f32, max_x: f32, min_y: f32, max_y: f32) -> PenaltyArea {
        PenaltyArea::new(
            Vector3::new(min_x, min_y, 0.0),
            Vector3::new(max_x, max_y, 0.0),
        )
    }

    #[test]
    fn left_team_own_box_classifies_as_own_box() {
        let own_box = pa(0.0, 132.0, 200.0, 345.0);
        let opp_box = pa(708.0, 840.0, 200.0, 345.0);
        let zone = MatchZone::classify(
            Vector3::new(20.0, 270.0, 0.0),
            PlayerSide::Left,
            840.0,
            own_box,
            opp_box,
        );
        assert!(zone.is_own_box());
    }

    #[test]
    fn left_team_close_to_goal_classifies_as_six_yard() {
        let own_box = pa(0.0, 132.0, 200.0, 345.0);
        let opp_box = pa(708.0, 840.0, 200.0, 345.0);
        let zone = MatchZone::classify(
            Vector3::new(10.0, 270.0, 0.0),
            PlayerSide::Left,
            840.0,
            own_box,
            opp_box,
        );
        assert_eq!(zone, MatchZone::OwnSixYardBox);
    }

    #[test]
    fn left_team_attacking_third_classifies_as_final_third() {
        let own_box = pa(0.0, 132.0, 200.0, 345.0);
        let opp_box = pa(708.0, 840.0, 200.0, 345.0);
        let zone = MatchZone::classify(
            Vector3::new(640.0, 270.0, 0.0),
            PlayerSide::Left,
            840.0,
            own_box,
            opp_box,
        );
        assert_eq!(zone, MatchZone::FinalThird);
    }

    #[test]
    fn right_team_own_box_classifies_as_own_box() {
        let left_box = pa(0.0, 132.0, 200.0, 345.0);
        let right_box = pa(708.0, 840.0, 200.0, 345.0);
        // For a Right-side player, "own box" is the right-hand box.
        let zone = MatchZone::classify(
            Vector3::new(820.0, 270.0, 0.0),
            PlayerSide::Right,
            840.0,
            right_box,
            left_box,
        );
        assert_eq!(zone, MatchZone::OwnSixYardBox);
    }

    #[test]
    fn middle_third_for_left_team() {
        let own_box = pa(0.0, 132.0, 200.0, 345.0);
        let opp_box = pa(708.0, 840.0, 200.0, 345.0);
        let zone = MatchZone::classify(
            Vector3::new(420.0, 270.0, 0.0),
            PlayerSide::Left,
            840.0,
            own_box,
            opp_box,
        );
        assert_eq!(zone, MatchZone::MiddleThird);
    }

    #[test]
    fn lateral_lane_central_for_centre_y() {
        assert_eq!(
            LateralLane::classify(272.5, 545.0),
            LateralLane::CentralLane
        );
    }

    #[test]
    fn lateral_lane_wide_for_outer_y() {
        assert_eq!(LateralLane::classify(20.0, 545.0), LateralLane::WideLeft);
        assert_eq!(LateralLane::classify(530.0, 545.0), LateralLane::WideRight);
    }
}

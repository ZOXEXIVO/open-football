//! Trait → behaviour bias table.
//!
//! Many `PlayerTrait`s previously existed only as scouting flavour and
//! a couple of shooting/passing checks. This module turns them into
//! concrete numeric biases that movement, passing, and defensive
//! decisions can consult. Centralising the table here means rebalancing
//! a trait is a single-file edit, not a hunt across state machines.
//!
//! All deltas are additive offsets to scores in roughly [0.0, 1.0]
//! space. Callers blend them into existing decision scores. Position
//! frequency / probability deltas are also exposed so movement code
//! can directly adjust how often a player attempts a given action.

use crate::club::player::traits::PlayerTrait;
use crate::r#match::MatchPlayer;

#[derive(Debug, Clone, Copy, Default)]
pub struct MovementBias {
    /// Y-axis offset from the formation slot (units toward touchline).
    /// Positive = toward the nearer touchline.
    pub touchline_offset_units: f32,
    /// Multiplier on cut-inside probability.
    pub cut_inside_delta: f32,
    /// Multiplier on cross probability.
    pub cross_delta: f32,
    /// Score bonus on box-arrival (late run, GetsIntoOppositionArea).
    pub box_arrival_bonus: f32,
    /// Score bonus on late-run trigger (ArrivesLate / GetsInto).
    pub late_run_bonus: f32,
    /// Cap on how far the player runs forward from formation.
    pub forward_run_cap_delta: f32,
    /// Rest-defense weighting bonus.
    pub rest_defense_bonus: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PassingBias {
    pub ask_for_ball_bonus: f32,
    pub tempo_control_bonus: f32,
    pub risky_central_pass_bonus: f32,
    pub through_ball_bonus: f32,
    pub turnover_tolerance: f32,
    pub short_pass_bonus: f32,
    pub long_pass_bonus: f32,
    pub switch_pass_bonus: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DefendingBias {
    /// Subtracted from marking-distance target (negative = tighter).
    pub marking_distance_offset: f32,
    pub foul_risk_delta: f32,
    pub interception_risk_delta: f32,
    pub tackle_attempt_threshold_delta: f32,
    pub clean_tackle_bonus: f32,
    pub block_intercept_preference: f32,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PersonalityBias {
    pub yellow_after_protest_chance: f32,
    pub opponent_aggression_chance: f32,
    pub own_card_risk_delta: f32,
}

pub fn movement_bias(player: &MatchPlayer) -> MovementBias {
    let mut b = MovementBias::default();
    if player.has_trait(PlayerTrait::HugsLine) {
        b.touchline_offset_units += 18.0;
        b.cut_inside_delta -= 0.20;
        b.cross_delta += 0.12;
    }
    if player.has_trait(PlayerTrait::CutsInsideFromBothWings) {
        b.touchline_offset_units -= 22.0;
        b.cut_inside_delta += 0.18;
        b.cross_delta -= 0.08;
    }
    if player.has_trait(PlayerTrait::GetsIntoOppositionArea) {
        b.box_arrival_bonus += 0.16;
        b.late_run_bonus += 0.10;
    }
    if player.has_trait(PlayerTrait::ArrivesLateInOppositionArea) {
        b.late_run_bonus += 0.18;
    }
    if player.has_trait(PlayerTrait::StaysBack) {
        b.forward_run_cap_delta -= 0.22;
        b.rest_defense_bonus += 0.18;
    }
    if player.has_trait(PlayerTrait::RunsWithBallRarely) {
        b.forward_run_cap_delta -= 0.06;
    }
    b
}

pub fn passing_bias(player: &MatchPlayer) -> PassingBias {
    let mut b = PassingBias::default();
    if player.has_trait(PlayerTrait::Playmaker) {
        b.ask_for_ball_bonus += 0.12;
        b.tempo_control_bonus += 0.10;
        if player.skills.mental.vision >= 13.0 {
            b.risky_central_pass_bonus += 0.08;
        }
    }
    if player.has_trait(PlayerTrait::TriesThroughBalls)
        || player.has_trait(PlayerTrait::KillerBallOften)
    {
        b.through_ball_bonus += 0.14;
        b.turnover_tolerance += 0.08;
    }
    if player.has_trait(PlayerTrait::PlaysShortPasses) {
        b.short_pass_bonus += 0.12;
        b.long_pass_bonus -= 0.10;
    }
    if player.has_trait(PlayerTrait::PlaysLongPasses) {
        b.long_pass_bonus += 0.14;
        b.short_pass_bonus -= 0.06;
    }
    if player.has_trait(PlayerTrait::LikesToSwitchPlay) {
        b.switch_pass_bonus += 0.18;
    }
    if player.has_trait(PlayerTrait::LooksForPassRatherThanAttemptShot) {
        b.ask_for_ball_bonus += 0.06;
    }
    b
}

pub fn defending_bias(player: &MatchPlayer) -> DefendingBias {
    let mut b = DefendingBias::default();
    if player.has_trait(PlayerTrait::MarkTightly) {
        b.marking_distance_offset -= 2.5;
        b.foul_risk_delta += 0.03;
        b.interception_risk_delta += 0.04;
    }
    if player.has_trait(PlayerTrait::StaysOnFeet) {
        b.tackle_attempt_threshold_delta += 0.10;
        b.block_intercept_preference += 0.08;
    }
    if player.has_trait(PlayerTrait::DivesIntoTackles) {
        b.tackle_attempt_threshold_delta -= 0.12;
        b.clean_tackle_bonus += 0.04;
        b.foul_risk_delta += 0.08;
    }
    b
}

pub fn personality_bias(player: &MatchPlayer) -> PersonalityBias {
    let mut b = PersonalityBias::default();
    if player.has_trait(PlayerTrait::Argues) {
        b.yellow_after_protest_chance += 0.03;
    }
    if player.has_trait(PlayerTrait::WindsUpOpponents) {
        b.opponent_aggression_chance += 0.02;
        b.own_card_risk_delta += 0.02;
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerSkills;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
    };
    use chrono::NaiveDate;

    fn make(traits: Vec<PlayerTrait>) -> MatchPlayer {
        let attrs = PlayerAttributes::default();
        let mut skills = PlayerSkills::default();
        skills.mental.vision = 14.0;
        let mut player = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::ForwardLeft,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        player.traits = traits;
        MatchPlayer::from_player(1, &player, PlayerPositionType::ForwardLeft, false)
    }

    #[test]
    fn hugs_line_increases_touchline_offset_and_cross() {
        let p = make(vec![PlayerTrait::HugsLine]);
        let m = movement_bias(&p);
        assert!(m.touchline_offset_units > 10.0);
        assert!(m.cross_delta > 0.0);
        assert!(m.cut_inside_delta < 0.0);
    }

    #[test]
    fn cuts_inside_pulls_toward_central_lane() {
        let p = make(vec![PlayerTrait::CutsInsideFromBothWings]);
        let m = movement_bias(&p);
        assert!(m.touchline_offset_units < -10.0);
        assert!(m.cut_inside_delta > 0.0);
    }

    #[test]
    fn playmaker_asks_for_ball_more() {
        let p = make(vec![PlayerTrait::Playmaker]);
        let pb = passing_bias(&p);
        assert!(pb.ask_for_ball_bonus > 0.0);
        assert!(pb.tempo_control_bonus > 0.0);
        assert!(pb.risky_central_pass_bonus > 0.0); // vision >= 13
    }

    #[test]
    fn dives_into_tackles_raises_foul_and_attempt_rate() {
        let p = make(vec![PlayerTrait::DivesIntoTackles]);
        let d = defending_bias(&p);
        assert!(d.foul_risk_delta > 0.0);
        assert!(d.tackle_attempt_threshold_delta < 0.0);
    }
}

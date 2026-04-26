//! Position grouping for development purposes and per-position skill
//! weights. These weights serve TWO purposes:
//! 1. Per-skill CEILING = `base_ceiling * weight` (key skills can reach
//!    high, irrelevant stay low).
//! 2. Per-skill GROWTH RATE multiplier (key skills develop faster).
//!
//! Range: 0.3 (irrelevant) to 1.5 (core skill). Default: 0.8 for
//! unspecified skills.

use super::skills_array::*;
use crate::PlayerPositionType;

// IMPORTANT: This grouping intentionally diverges from
// `PlayerPositionType::position_group()` for `DefensiveMidfielder`.
// The canonical position group treats DM as a Defender (because they
// drop deep, screen the back four, and are evaluated using defensive
// weights). For *development*, however, a DM grows the same skill set
// as a central midfielder: passing, vision, stamina, decisions. Treating
// them as a defender for development would slow their growth on the
// skills that actually define their role. The divergence is contained
// to this module; ability calculations elsewhere keep using
// `position_group()`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum PosGroup {
    Goalkeeper,
    Defender,
    Midfielder,
    Forward,
}

pub(super) fn pos_group_from(pos: PlayerPositionType) -> PosGroup {
    match pos {
        PlayerPositionType::Goalkeeper => PosGroup::Goalkeeper,
        PlayerPositionType::Sweeper
        | PlayerPositionType::DefenderLeft
        | PlayerPositionType::DefenderCenterLeft
        | PlayerPositionType::DefenderCenter
        | PlayerPositionType::DefenderCenterRight
        | PlayerPositionType::DefenderRight => PosGroup::Defender,
        PlayerPositionType::WingbackLeft
        | PlayerPositionType::WingbackRight
        | PlayerPositionType::DefensiveMidfielder
        | PlayerPositionType::MidfielderLeft
        | PlayerPositionType::MidfielderCenterLeft
        | PlayerPositionType::MidfielderCenter
        | PlayerPositionType::MidfielderCenterRight
        | PlayerPositionType::MidfielderRight => PosGroup::Midfielder,
        PlayerPositionType::AttackingMidfielderLeft
        | PlayerPositionType::AttackingMidfielderCenter
        | PlayerPositionType::AttackingMidfielderRight
        | PlayerPositionType::ForwardLeft
        | PlayerPositionType::ForwardCenter
        | PlayerPositionType::ForwardRight
        | PlayerPositionType::Striker => PosGroup::Forward,
    }
}

pub(super) fn position_dev_weights(group: PosGroup) -> [f32; SKILL_COUNT] {
    let mut w = [0.8f32; SKILL_COUNT];

    // GK-specific skills default to 0 for outfield players: they don't train them.
    for i in SK_GK_AERIAL_REACH..=SK_GK_THROWING {
        w[i] = 0.0;
    }

    match group {
        PosGroup::Goalkeeper => {
            w[SK_POSITIONING] = 1.5;
            w[SK_CONCENTRATION] = 1.4;
            w[SK_AGILITY] = 1.4;
            w[SK_ANTICIPATION] = 1.3;
            w[SK_COMPOSURE] = 1.3;
            w[SK_JUMPING] = 1.3;
            w[SK_BRAVERY] = 1.2;
            w[SK_DECISIONS] = 1.1;
            w[SK_STRENGTH] = 1.0;
            // Modern GK
            w[SK_FIRST_TOUCH] = 1.0;
            w[SK_PASSING] = 1.0;
            w[SK_TECHNIQUE] = 0.9;
            w[SK_NATURAL_FITNESS] = 1.0;
            w[SK_PACE] = 0.8;
            w[SK_STAMINA] = 0.8;
            // Irrelevant outfield skills
            w[SK_FINISHING] = 0.3;
            w[SK_LONG_SHOTS] = 0.3;
            w[SK_CROSSING] = 0.3;
            w[SK_CORNERS] = 0.3;
            w[SK_FREE_KICKS] = 0.35;
            w[SK_HEADING] = 0.35;
            w[SK_OFF_THE_BALL] = 0.35;
            w[SK_DRIBBLING] = 0.4;
            w[SK_LONG_THROWS] = 0.5;
            w[SK_TACKLING] = 0.35;
            w[SK_MARKING] = 0.35;
            w[SK_WORK_RATE] = 0.5;
            w[SK_FLAIR] = 0.4;
            w[SK_ACCELERATION] = 0.6;
            // Goalkeeping-specific attributes
            w[SK_GK_HANDLING] = 1.5;
            w[SK_GK_REFLEXES] = 1.5;
            w[SK_GK_ONE_ON_ONES] = 1.4;
            w[SK_GK_AERIAL_REACH] = 1.3;
            w[SK_GK_COMMAND_OF_AREA] = 1.3;
            w[SK_GK_COMMUNICATION] = 1.3;
            w[SK_GK_RUSHING_OUT] = 1.2;
            w[SK_GK_PUNCHING] = 1.2;
            w[SK_GK_KICKING] = 1.1;
            w[SK_GK_THROWING] = 1.1;
            w[SK_GK_FIRST_TOUCH] = 1.0;
            w[SK_GK_PASSING] = 1.0;
            w[SK_GK_ECCENTRICITY] = 0.6;
        }
        PosGroup::Defender => {
            w[SK_TACKLING] = 1.4;
            w[SK_MARKING] = 1.4;
            w[SK_POSITIONING] = 1.4;
            w[SK_HEADING] = 1.3;
            w[SK_STRENGTH] = 1.2;
            w[SK_CONCENTRATION] = 1.2;
            w[SK_ANTICIPATION] = 1.2;
            w[SK_BRAVERY] = 1.2;
            w[SK_JUMPING] = 1.1;
            w[SK_PACE] = 1.0;
            w[SK_PASSING] = 1.0;
            w[SK_TEAMWORK] = 1.0;
            w[SK_DECISIONS] = 1.0;
            w[SK_COMPOSURE] = 1.0;
            w[SK_NATURAL_FITNESS] = 1.0;
            w[SK_STAMINA] = 1.0;
            // Less relevant for defenders
            w[SK_FINISHING] = 0.4;
            w[SK_DRIBBLING] = 0.5;
            w[SK_FLAIR] = 0.4;
            w[SK_LONG_SHOTS] = 0.4;
            w[SK_OFF_THE_BALL] = 0.5;
            w[SK_CORNERS] = 0.4;
            w[SK_FREE_KICKS] = 0.4;
        }
        PosGroup::Midfielder => {
            w[SK_PASSING] = 1.4;
            w[SK_VISION] = 1.3;
            w[SK_STAMINA] = 1.2;
            w[SK_TECHNIQUE] = 1.2;
            w[SK_FIRST_TOUCH] = 1.2;
            w[SK_DECISIONS] = 1.2;
            w[SK_TEAMWORK] = 1.2;
            w[SK_WORK_RATE] = 1.2;
            w[SK_DRIBBLING] = 1.0;
            w[SK_TACKLING] = 1.0;
            w[SK_POSITIONING] = 1.0;
            w[SK_COMPOSURE] = 1.0;
            w[SK_ANTICIPATION] = 1.0;
            w[SK_CONCENTRATION] = 1.0;
            w[SK_PACE] = 1.0;
            w[SK_ACCELERATION] = 1.0;
            w[SK_NATURAL_FITNESS] = 1.0;
            w[SK_BALANCE] = 1.0;
            // Less relevant
            w[SK_HEADING] = 0.6;
            w[SK_LONG_THROWS] = 0.5;
            w[SK_FINISHING] = 0.6;
        }
        PosGroup::Forward => {
            w[SK_FINISHING] = 1.5;
            w[SK_OFF_THE_BALL] = 1.4;
            w[SK_DRIBBLING] = 1.3;
            w[SK_PACE] = 1.2;
            w[SK_COMPOSURE] = 1.2;
            w[SK_FIRST_TOUCH] = 1.2;
            w[SK_ANTICIPATION] = 1.2;
            w[SK_ACCELERATION] = 1.2;
            w[SK_HEADING] = 1.0;
            w[SK_TECHNIQUE] = 1.0;
            w[SK_STRENGTH] = 1.0;
            w[SK_AGILITY] = 1.0;
            w[SK_BALANCE] = 1.0;
            w[SK_DECISIONS] = 1.0;
            w[SK_DETERMINATION] = 1.0;
            w[SK_BRAVERY] = 1.0;
            w[SK_NATURAL_FITNESS] = 1.0;
            // Less relevant
            w[SK_TACKLING] = 0.35;
            w[SK_MARKING] = 0.35;
            w[SK_POSITIONING] = 0.5;
            w[SK_CONCENTRATION] = 0.6;
            w[SK_LONG_THROWS] = 0.4;
        }
    }
    w
}

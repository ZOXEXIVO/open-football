use crate::club::player::position::PlayerPositionType;

pub const SKILL_COUNT: usize = 37;

// Technical (0..14)
pub const SK_CORNERS: usize = 0;
pub const SK_CROSSING: usize = 1;
pub const SK_DRIBBLING: usize = 2;
pub const SK_FINISHING: usize = 3;
pub const SK_FIRST_TOUCH: usize = 4;
pub const SK_FREE_KICKS: usize = 5;
pub const SK_HEADING: usize = 6;
pub const SK_LONG_SHOTS: usize = 7;
pub const SK_LONG_THROWS: usize = 8;
pub const SK_MARKING: usize = 9;
pub const SK_PASSING: usize = 10;
pub const SK_PENALTY_TAKING: usize = 11;
pub const SK_TACKLING: usize = 12;
pub const SK_TECHNIQUE: usize = 13;
// Mental (14..28)
pub const SK_AGGRESSION: usize = 14;
pub const SK_ANTICIPATION: usize = 15;
pub const SK_BRAVERY: usize = 16;
pub const SK_COMPOSURE: usize = 17;
pub const SK_CONCENTRATION: usize = 18;
pub const SK_DECISIONS: usize = 19;
pub const SK_DETERMINATION: usize = 20;
pub const SK_FLAIR: usize = 21;
pub const SK_LEADERSHIP: usize = 22;
pub const SK_OFF_THE_BALL: usize = 23;
pub const SK_POSITIONING: usize = 24;
pub const SK_TEAMWORK: usize = 25;
pub const SK_VISION: usize = 26;
pub const SK_WORK_RATE: usize = 27;
// Physical (28..36)
pub const SK_ACCELERATION: usize = 28;
pub const SK_AGILITY: usize = 29;
pub const SK_BALANCE: usize = 30;
pub const SK_JUMPING: usize = 31;
pub const SK_NATURAL_FITNESS: usize = 32;
pub const SK_PACE: usize = 33;
pub const SK_STAMINA: usize = 34;
pub const SK_STRENGTH: usize = 35;
// match_readiness (36) — state, not skill; weight stays 0.0 for CA scoring
pub const SK_MATCH_READINESS: usize = 36;

/// Per-exact-position attribute weight table (37 slots).
///
/// Weights are the single source of truth for two things:
///   1. Skill generation — how the CA target is shaped into individual attributes.
///   2. CA scoring — `PlayerSkills::calculate_ability_for_position` weighs each
///      attribute by its relevance to the role.
///
/// Range: 0.0 (irrelevant) .. ~2.0 (defining). The match_readiness slot is
/// always 0 — it's a fitness state, not a skill.
pub struct PositionWeights;

impl PositionWeights {
    /// Look up the 37-slot weight vector for an exact playing position.
    pub fn for_position(position: PlayerPositionType) -> [f32; SKILL_COUNT] {
        let mut w = [0.7f32; SKILL_COUNT];
        w[SK_MATCH_READINESS] = 0.0;
        use PlayerPositionType::*;
        match position {
            Goalkeeper => {
                // Goalkeeper outfield-attribute profile. Goalkeeping itself is
                // scored separately from the Goalkeeping struct; this table
                // covers the player's mental/physical/technical bedding.
                w[SK_POSITIONING] = 1.8;
                w[SK_CONCENTRATION] = 1.6;
                w[SK_AGILITY] = 1.7;
                w[SK_ANTICIPATION] = 1.5;
                w[SK_COMPOSURE] = 1.5;
                w[SK_JUMPING] = 1.5;
                w[SK_BRAVERY] = 1.4;
                w[SK_DECISIONS] = 1.3;
                w[SK_STRENGTH] = 1.1;
                w[SK_FIRST_TOUCH] = 1.1;
                w[SK_PASSING] = 1.1;
                w[SK_TECHNIQUE] = 1.0;
                w[SK_NATURAL_FITNESS] = 1.0;
                w[SK_PACE] = 0.7;
                w[SK_STAMINA] = 0.7;
                w[SK_LEADERSHIP] = 1.0;
                w[SK_BALANCE] = 1.0;
                w[SK_DETERMINATION] = 1.0;
                w[SK_TEAMWORK] = 1.0;
                w[SK_PENALTY_TAKING] = 0.3;
                w[SK_FINISHING] = 0.4;
                w[SK_LONG_SHOTS] = 0.4;
                w[SK_CROSSING] = 0.4;
                w[SK_CORNERS] = 0.4;
                w[SK_FREE_KICKS] = 0.5;
                w[SK_HEADING] = 0.5;
                w[SK_OFF_THE_BALL] = 0.4;
                w[SK_DRIBBLING] = 0.5;
                w[SK_LONG_THROWS] = 0.5;
                w[SK_TACKLING] = 0.5;
                w[SK_MARKING] = 0.5;
                w[SK_WORK_RATE] = 0.6;
                w[SK_FLAIR] = 0.4;
                w[SK_ACCELERATION] = 0.7;
                w[SK_AGGRESSION] = 0.6;
            }
            Sweeper => {
                w[SK_TACKLING] = 1.5;
                w[SK_MARKING] = 1.4;
                w[SK_POSITIONING] = 1.7;
                w[SK_ANTICIPATION] = 1.6;
                w[SK_DECISIONS] = 1.5;
                w[SK_COMPOSURE] = 1.4;
                w[SK_PASSING] = 1.3;
                w[SK_FIRST_TOUCH] = 1.2;
                w[SK_TECHNIQUE] = 1.2;
                w[SK_VISION] = 1.2;
                w[SK_HEADING] = 1.2;
                w[SK_STRENGTH] = 1.2;
                w[SK_CONCENTRATION] = 1.4;
                w[SK_BRAVERY] = 1.2;
                w[SK_PACE] = 1.0;
                w[SK_JUMPING] = 1.0;
                w[SK_TEAMWORK] = 1.2;
                w[SK_AGGRESSION] = 1.0;
                w[SK_NATURAL_FITNESS] = 1.0;
                w[SK_STAMINA] = 1.0;
                w[SK_FINISHING] = 0.3;
                w[SK_DRIBBLING] = 0.5;
                w[SK_FLAIR] = 0.5;
                w[SK_LONG_SHOTS] = 0.3;
                w[SK_OFF_THE_BALL] = 0.4;
                w[SK_CROSSING] = 0.4;
                w[SK_CORNERS] = 0.3;
                w[SK_FREE_KICKS] = 0.4;
            }
            DefenderCenter | DefenderCenterLeft | DefenderCenterRight => {
                w[SK_TACKLING] = 1.6;
                w[SK_MARKING] = 1.7;
                w[SK_POSITIONING] = 1.6;
                w[SK_HEADING] = 1.5;
                w[SK_STRENGTH] = 1.5;
                w[SK_CONCENTRATION] = 1.4;
                w[SK_ANTICIPATION] = 1.4;
                w[SK_BRAVERY] = 1.4;
                w[SK_AGGRESSION] = 1.2;
                w[SK_JUMPING] = 1.3;
                w[SK_PACE] = 1.0;
                w[SK_PASSING] = 1.0;
                w[SK_FIRST_TOUCH] = 0.9;
                w[SK_TEAMWORK] = 1.1;
                w[SK_DECISIONS] = 1.2;
                w[SK_COMPOSURE] = 1.1;
                w[SK_DETERMINATION] = 1.1;
                w[SK_NATURAL_FITNESS] = 1.0;
                w[SK_STAMINA] = 1.0;
                w[SK_FINISHING] = 0.2;
                w[SK_DRIBBLING] = 0.4;
                w[SK_FLAIR] = 0.3;
                w[SK_LONG_SHOTS] = 0.2;
                w[SK_OFF_THE_BALL] = 0.3;
                w[SK_VISION] = 0.6;
                w[SK_CROSSING] = 0.4;
                w[SK_CORNERS] = 0.3;
                w[SK_FREE_KICKS] = 0.3;
                w[SK_AGILITY] = 0.8;
                w[SK_ACCELERATION] = 0.9;
            }
            DefenderLeft | DefenderRight => {
                // Full-back: pace + crossing + stamina, less heading/strength
                w[SK_TACKLING] = 1.4;
                w[SK_MARKING] = 1.4;
                w[SK_POSITIONING] = 1.3;
                w[SK_PACE] = 1.5;
                w[SK_STAMINA] = 1.4;
                w[SK_ACCELERATION] = 1.4;
                w[SK_CROSSING] = 1.3;
                w[SK_WORK_RATE] = 1.4;
                w[SK_ANTICIPATION] = 1.2;
                w[SK_CONCENTRATION] = 1.2;
                w[SK_TEAMWORK] = 1.2;
                w[SK_DECISIONS] = 1.1;
                w[SK_AGILITY] = 1.2;
                w[SK_PASSING] = 1.1;
                w[SK_FIRST_TOUCH] = 1.1;
                w[SK_NATURAL_FITNESS] = 1.1;
                w[SK_BRAVERY] = 1.1;
                w[SK_DRIBBLING] = 1.0;
                w[SK_TECHNIQUE] = 1.0;
                w[SK_OFF_THE_BALL] = 0.9;
                w[SK_HEADING] = 0.9;
                w[SK_STRENGTH] = 0.9;
                w[SK_JUMPING] = 0.8;
                w[SK_FINISHING] = 0.4;
                w[SK_LONG_SHOTS] = 0.4;
                w[SK_FLAIR] = 0.7;
                w[SK_VISION] = 0.8;
                w[SK_LONG_THROWS] = 1.0;
                w[SK_CORNERS] = 0.5;
                w[SK_FREE_KICKS] = 0.5;
                w[SK_AGGRESSION] = 0.9;
                w[SK_COMPOSURE] = 1.0;
            }
            WingbackLeft | WingbackRight => {
                // Wing-back: very high pace + stamina, attack-oriented
                w[SK_PACE] = 1.6;
                w[SK_STAMINA] = 1.6;
                w[SK_ACCELERATION] = 1.5;
                w[SK_CROSSING] = 1.5;
                w[SK_WORK_RATE] = 1.5;
                w[SK_DRIBBLING] = 1.3;
                w[SK_OFF_THE_BALL] = 1.3;
                w[SK_TECHNIQUE] = 1.2;
                w[SK_AGILITY] = 1.3;
                w[SK_FIRST_TOUCH] = 1.2;
                w[SK_PASSING] = 1.2;
                w[SK_DECISIONS] = 1.1;
                w[SK_TACKLING] = 1.1;
                w[SK_MARKING] = 1.0;
                w[SK_POSITIONING] = 1.0;
                w[SK_ANTICIPATION] = 1.1;
                w[SK_CONCENTRATION] = 1.0;
                w[SK_TEAMWORK] = 1.1;
                w[SK_NATURAL_FITNESS] = 1.2;
                w[SK_BRAVERY] = 1.0;
                w[SK_FLAIR] = 1.0;
                w[SK_BALANCE] = 1.1;
                w[SK_VISION] = 0.9;
                w[SK_COMPOSURE] = 1.0;
                w[SK_HEADING] = 0.7;
                w[SK_STRENGTH] = 0.8;
                w[SK_JUMPING] = 0.7;
                w[SK_FINISHING] = 0.6;
                w[SK_LONG_SHOTS] = 0.6;
                w[SK_LONG_THROWS] = 1.1;
                w[SK_CORNERS] = 0.6;
                w[SK_FREE_KICKS] = 0.6;
                w[SK_AGGRESSION] = 0.8;
            }
            DefensiveMidfielder => {
                w[SK_TACKLING] = 1.5;
                w[SK_MARKING] = 1.3;
                w[SK_POSITIONING] = 1.5;
                w[SK_ANTICIPATION] = 1.4;
                w[SK_PASSING] = 1.3;
                w[SK_COMPOSURE] = 1.3;
                w[SK_WORK_RATE] = 1.4;
                w[SK_DECISIONS] = 1.4;
                w[SK_CONCENTRATION] = 1.3;
                w[SK_STAMINA] = 1.3;
                w[SK_TEAMWORK] = 1.3;
                w[SK_FIRST_TOUCH] = 1.2;
                w[SK_TECHNIQUE] = 1.2;
                w[SK_AGGRESSION] = 1.2;
                w[SK_BRAVERY] = 1.2;
                w[SK_VISION] = 1.1;
                w[SK_STRENGTH] = 1.2;
                w[SK_DETERMINATION] = 1.2;
                w[SK_NATURAL_FITNESS] = 1.0;
                w[SK_PACE] = 0.9;
                w[SK_ACCELERATION] = 0.9;
                w[SK_HEADING] = 1.1;
                w[SK_JUMPING] = 1.0;
                w[SK_FINISHING] = 0.4;
                w[SK_DRIBBLING] = 0.7;
                w[SK_FLAIR] = 0.6;
                w[SK_LONG_SHOTS] = 0.7;
                w[SK_OFF_THE_BALL] = 0.7;
                w[SK_CROSSING] = 0.5;
                w[SK_CORNERS] = 0.5;
                w[SK_FREE_KICKS] = 0.6;
            }
            MidfielderCenter | MidfielderCenterLeft | MidfielderCenterRight => {
                w[SK_PASSING] = 1.5;
                w[SK_VISION] = 1.4;
                w[SK_TECHNIQUE] = 1.4;
                w[SK_FIRST_TOUCH] = 1.4;
                w[SK_DECISIONS] = 1.4;
                w[SK_TEAMWORK] = 1.3;
                w[SK_STAMINA] = 1.3;
                w[SK_WORK_RATE] = 1.3;
                w[SK_ANTICIPATION] = 1.2;
                w[SK_COMPOSURE] = 1.2;
                w[SK_OFF_THE_BALL] = 1.2;
                w[SK_DRIBBLING] = 1.1;
                w[SK_TACKLING] = 1.0;
                w[SK_POSITIONING] = 1.0;
                w[SK_CONCENTRATION] = 1.1;
                w[SK_PACE] = 1.0;
                w[SK_ACCELERATION] = 1.0;
                w[SK_NATURAL_FITNESS] = 1.0;
                w[SK_BALANCE] = 1.0;
                w[SK_FLAIR] = 1.0;
                w[SK_DETERMINATION] = 1.1;
                w[SK_BRAVERY] = 1.0;
                w[SK_LONG_SHOTS] = 1.0;
                w[SK_HEADING] = 0.6;
                w[SK_LONG_THROWS] = 0.4;
                w[SK_FINISHING] = 0.6;
                w[SK_MARKING] = 0.7;
                w[SK_STRENGTH] = 0.8;
                w[SK_AGGRESSION] = 0.9;
                w[SK_CROSSING] = 0.7;
                w[SK_CORNERS] = 0.9;
                w[SK_FREE_KICKS] = 0.9;
            }
            MidfielderLeft | MidfielderRight => {
                // Wide midfielder: pace + crossing + work rate
                w[SK_PACE] = 1.4;
                w[SK_CROSSING] = 1.4;
                w[SK_STAMINA] = 1.4;
                w[SK_DRIBBLING] = 1.3;
                w[SK_WORK_RATE] = 1.4;
                w[SK_TECHNIQUE] = 1.2;
                w[SK_OFF_THE_BALL] = 1.2;
                w[SK_ACCELERATION] = 1.3;
                w[SK_FIRST_TOUCH] = 1.2;
                w[SK_PASSING] = 1.2;
                w[SK_AGILITY] = 1.2;
                w[SK_FLAIR] = 1.1;
                w[SK_DECISIONS] = 1.1;
                w[SK_TEAMWORK] = 1.2;
                w[SK_TACKLING] = 0.9;
                w[SK_BALANCE] = 1.1;
                w[SK_NATURAL_FITNESS] = 1.1;
                w[SK_VISION] = 1.0;
                w[SK_FINISHING] = 0.8;
                w[SK_LONG_SHOTS] = 0.9;
                w[SK_ANTICIPATION] = 1.0;
                w[SK_HEADING] = 0.6;
                w[SK_MARKING] = 0.7;
                w[SK_STRENGTH] = 0.8;
                w[SK_POSITIONING] = 0.9;
                w[SK_CONCENTRATION] = 0.9;
                w[SK_CORNERS] = 1.0;
                w[SK_FREE_KICKS] = 0.8;
                w[SK_LONG_THROWS] = 0.9;
                w[SK_BRAVERY] = 0.9;
                w[SK_COMPOSURE] = 1.0;
                w[SK_JUMPING] = 0.7;
                w[SK_AGGRESSION] = 0.8;
            }
            AttackingMidfielderCenter => {
                w[SK_PASSING] = 1.4;
                w[SK_VISION] = 1.5;
                w[SK_TECHNIQUE] = 1.5;
                w[SK_FIRST_TOUCH] = 1.5;
                w[SK_DECISIONS] = 1.3;
                w[SK_DRIBBLING] = 1.3;
                w[SK_OFF_THE_BALL] = 1.4;
                w[SK_FLAIR] = 1.3;
                w[SK_COMPOSURE] = 1.3;
                w[SK_ANTICIPATION] = 1.2;
                w[SK_LONG_SHOTS] = 1.2;
                w[SK_FINISHING] = 1.0;
                w[SK_AGILITY] = 1.2;
                w[SK_BALANCE] = 1.1;
                w[SK_ACCELERATION] = 1.1;
                w[SK_PACE] = 1.0;
                w[SK_TEAMWORK] = 1.0;
                w[SK_WORK_RATE] = 0.9;
                w[SK_STAMINA] = 1.0;
                w[SK_NATURAL_FITNESS] = 1.0;
                w[SK_TACKLING] = 0.4;
                w[SK_MARKING] = 0.4;
                w[SK_POSITIONING] = 0.7;
                w[SK_HEADING] = 0.6;
                w[SK_STRENGTH] = 0.7;
                w[SK_CONCENTRATION] = 0.9;
                w[SK_AGGRESSION] = 0.6;
                w[SK_BRAVERY] = 0.9;
                w[SK_DETERMINATION] = 1.0;
                w[SK_CROSSING] = 0.9;
                w[SK_CORNERS] = 1.0;
                w[SK_FREE_KICKS] = 1.1;
                w[SK_LEADERSHIP] = 0.8;
                w[SK_LONG_THROWS] = 0.4;
            }
            AttackingMidfielderLeft | AttackingMidfielderRight => {
                // Inside forward / wide attacking mid: dribbling, pace, finishing
                w[SK_PACE] = 1.4;
                w[SK_DRIBBLING] = 1.5;
                w[SK_TECHNIQUE] = 1.4;
                w[SK_OFF_THE_BALL] = 1.3;
                w[SK_FIRST_TOUCH] = 1.4;
                w[SK_FLAIR] = 1.3;
                w[SK_ACCELERATION] = 1.4;
                w[SK_AGILITY] = 1.3;
                w[SK_CROSSING] = 1.3;
                w[SK_FINISHING] = 1.2;
                w[SK_LONG_SHOTS] = 1.1;
                w[SK_PASSING] = 1.1;
                w[SK_DECISIONS] = 1.1;
                w[SK_COMPOSURE] = 1.1;
                w[SK_BALANCE] = 1.2;
                w[SK_ANTICIPATION] = 1.1;
                w[SK_VISION] = 1.0;
                w[SK_STAMINA] = 1.1;
                w[SK_WORK_RATE] = 1.1;
                w[SK_NATURAL_FITNESS] = 1.0;
                w[SK_TACKLING] = 0.4;
                w[SK_MARKING] = 0.4;
                w[SK_POSITIONING] = 0.7;
                w[SK_HEADING] = 0.6;
                w[SK_STRENGTH] = 0.8;
                w[SK_CONCENTRATION] = 0.9;
                w[SK_AGGRESSION] = 0.7;
                w[SK_BRAVERY] = 0.9;
                w[SK_TEAMWORK] = 1.0;
                w[SK_DETERMINATION] = 1.0;
                w[SK_JUMPING] = 0.7;
                w[SK_CORNERS] = 0.9;
                w[SK_FREE_KICKS] = 0.9;
                w[SK_LONG_THROWS] = 0.5;
            }
            Striker | ForwardCenter => {
                w[SK_FINISHING] = 1.7;
                w[SK_OFF_THE_BALL] = 1.5;
                w[SK_COMPOSURE] = 1.4;
                w[SK_ANTICIPATION] = 1.4;
                w[SK_FIRST_TOUCH] = 1.4;
                w[SK_PACE] = 1.3;
                w[SK_ACCELERATION] = 1.3;
                w[SK_DRIBBLING] = 1.2;
                w[SK_TECHNIQUE] = 1.2;
                w[SK_HEADING] = 1.2;
                w[SK_STRENGTH] = 1.1;
                w[SK_AGILITY] = 1.1;
                w[SK_BALANCE] = 1.1;
                w[SK_DECISIONS] = 1.0;
                w[SK_LONG_SHOTS] = 1.0;
                w[SK_FLAIR] = 1.0;
                w[SK_DETERMINATION] = 1.1;
                w[SK_BRAVERY] = 1.0;
                w[SK_NATURAL_FITNESS] = 1.0;
                w[SK_JUMPING] = 1.0;
                w[SK_STAMINA] = 1.0;
                w[SK_PASSING] = 0.9;
                w[SK_VISION] = 0.9;
                w[SK_WORK_RATE] = 1.0;
                w[SK_TACKLING] = 0.2;
                w[SK_MARKING] = 0.2;
                w[SK_POSITIONING] = 0.4;
                w[SK_CONCENTRATION] = 0.8;
                w[SK_LONG_THROWS] = 0.3;
                w[SK_AGGRESSION] = 0.8;
                w[SK_TEAMWORK] = 0.8;
                w[SK_PENALTY_TAKING] = 1.1;
                w[SK_CORNERS] = 0.4;
                w[SK_FREE_KICKS] = 0.7;
                w[SK_CROSSING] = 0.6;
            }
            ForwardLeft | ForwardRight => {
                // Wide forward: pace + dribbling + finishing
                w[SK_PACE] = 1.5;
                w[SK_DRIBBLING] = 1.4;
                w[SK_FINISHING] = 1.3;
                w[SK_ACCELERATION] = 1.4;
                w[SK_OFF_THE_BALL] = 1.3;
                w[SK_TECHNIQUE] = 1.3;
                w[SK_FIRST_TOUCH] = 1.3;
                w[SK_FLAIR] = 1.2;
                w[SK_AGILITY] = 1.3;
                w[SK_COMPOSURE] = 1.2;
                w[SK_ANTICIPATION] = 1.1;
                w[SK_BALANCE] = 1.2;
                w[SK_CROSSING] = 1.2;
                w[SK_LONG_SHOTS] = 1.0;
                w[SK_PASSING] = 1.0;
                w[SK_DECISIONS] = 1.0;
                w[SK_STAMINA] = 1.1;
                w[SK_WORK_RATE] = 1.0;
                w[SK_NATURAL_FITNESS] = 1.0;
                w[SK_VISION] = 0.9;
                w[SK_DETERMINATION] = 1.0;
                w[SK_TACKLING] = 0.3;
                w[SK_MARKING] = 0.3;
                w[SK_POSITIONING] = 0.6;
                w[SK_HEADING] = 0.8;
                w[SK_STRENGTH] = 0.8;
                w[SK_CONCENTRATION] = 0.8;
                w[SK_AGGRESSION] = 0.7;
                w[SK_BRAVERY] = 0.9;
                w[SK_JUMPING] = 0.8;
                w[SK_TEAMWORK] = 0.9;
                w[SK_LONG_THROWS] = 0.4;
                w[SK_CORNERS] = 0.7;
                w[SK_FREE_KICKS] = 0.8;
            }
        }
        w
    }

    /// Sum of weights, used to normalise a weighted average.
    pub fn total(w: &[f32; SKILL_COUNT]) -> f32 {
        let mut total = 0.0;
        for v in w.iter() {
            total += *v;
        }
        total
    }
}

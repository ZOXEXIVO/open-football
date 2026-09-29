//! CA scoring calibrated against filled source profiles, separately from
//! the archetype weights used to generate attribute shapes. See
//! .dev/ability/README.md for the cohort, holdout and limitations.

use crate::PlayerPositionType;

// Registry order: technical (14), mental (14), physical (9), goalkeeping (13).
const ATTACKING_MIDFIELDER_CENTER: [f32; 50] = [
    // technical
    0.0133, 0.0000, 0.0563, 0.0449, 0.0790, 0.0000, 0.0089, 0.0065, 0.0130, 0.0181, 0.0259, 0.0181,
    0.0002, 0.0213, // mental
    0.0000, 0.0353, 0.0113, 0.0604, 0.0073, 0.0597, 0.0070, 0.0093, 0.0045, 0.0677, 0.0274, 0.0138,
    0.0303, 0.0054, // physical
    0.0717, 0.0622, 0.0150, 0.0254, 0.0000, 0.0739, 0.0535, 0.0533, 0.0000,
    // goalkeeping
    0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0.0000,
];

const DEFENDER_CENTER: [f32; 50] = [
    // technical
    0.0129, 0.0028, 0.0159, 0.0179, 0.0302, 0.0070, 0.0345, 0.0120, 0.0057, 0.0767, 0.0176, 0.0090,
    0.0366, 0.0105, // mental
    0.0000, 0.0434, 0.0154, 0.0196, 0.0325, 0.0987, 0.0011, 0.0008, 0.0170, 0.0091, 0.0798, 0.0130,
    0.0102, 0.0271, // physical
    0.0521, 0.0571, 0.0177, 0.0584, 0.0000, 0.0541, 0.0281, 0.0755, 0.0000,
    // goalkeeping
    0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0.0000,
];

const DEFENSIVE_MIDFIELDER: [f32; 50] = [
    // technical
    0.0065, 0.0087, 0.0237, 0.0354, 0.0533, 0.0104, 0.0095, 0.0255, 0.0074, 0.0231, 0.0512, 0.0128,
    0.0272, 0.0473, // mental
    0.0000, 0.0511, 0.0103, 0.0276, 0.0240, 0.0752, 0.0051, 0.0048, 0.0080, 0.0125, 0.0237, 0.0273,
    0.0428, 0.0333, // physical
    0.0613, 0.0638, 0.0248, 0.0106, 0.0000, 0.0452, 0.0658, 0.0409, 0.0000,
    // goalkeeping
    0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0.0000,
];

const FULLBACK: [f32; 50] = [
    // technical
    0.0000, 0.0031, 0.0198, 0.0194, 0.0465, 0.0019, 0.0169, 0.0121, 0.0000, 0.0000, 0.0421, 0.0094,
    0.0193, 0.0401, // mental
    0.0000, 0.0529, 0.0120, 0.0230, 0.0278, 0.0744, 0.0125, 0.0167, 0.0154, 0.0034, 0.0263, 0.0252,
    0.0151, 0.0460, // physical
    0.0951, 0.0499, 0.0185, 0.0250, 0.0039, 0.0925, 0.0887, 0.0453, 0.0000,
    // goalkeeping
    0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0.0000,
];

const GOALKEEPER: [f32; 50] = [
    // technical
    0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0.0000, 0.0000, // mental
    0.0000, 0.0253, 0.0605, 0.0223, 0.0499, 0.1007, 0.0000, 0.0000, 0.0000, 0.0000, 0.0477, 0.0000,
    0.0000, 0.0000, // physical
    0.0642, 0.0696, 0.0000, 0.0000, 0.0000, 0.0388, 0.0000, 0.0478, 0.0000,
    // goalkeeping
    0.0529, 0.0550, 0.0499, 0.0000, 0.0363, 0.0637, 0.0419, 0.0316, 0.0305, 0.0000, 0.0821, 0.0000,
    0.0292,
];

const MIDFIELDER_CENTER: [f32; 50] = [
    // technical
    0.0000, 0.0317, 0.0471, 0.0461, 0.0462, 0.0135, 0.0073, 0.0104, 0.0102, 0.0158, 0.0414, 0.0167,
    0.0164, 0.0537, // mental
    0.0000, 0.0302, 0.0065, 0.0438, 0.0135, 0.0556, 0.0000, 0.0103, 0.0107, 0.0295, 0.0047, 0.0236,
    0.0379, 0.0354, // physical
    0.0799, 0.0584, 0.0182, 0.0129, 0.0009, 0.0776, 0.0583, 0.0358, 0.0000,
    // goalkeeping
    0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0.0000,
];

const STRIKER: [f32; 50] = [
    // technical
    0.0122, 0.0210, 0.0359, 0.0904, 0.0609, 0.0113, 0.0456, 0.0185, 0.0104, 0.0208, 0.0025, 0.0078,
    0.0055, 0.0336, // mental
    0.0000, 0.0490, 0.0000, 0.0444, 0.0126, 0.0309, 0.0081, 0.0096, 0.0108, 0.0469, 0.0073, 0.0112,
    0.0158, 0.0133, // physical
    0.0892, 0.0335, 0.0213, 0.0425, 0.0000, 0.0699, 0.0515, 0.0560, 0.0000,
    // goalkeeping
    0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0.0000,
];

const WIDE_ATTACKER: [f32; 50] = [
    // technical
    0.0065, 0.0359, 0.0486, 0.0429, 0.0514, 0.0075, 0.0184, 0.0166, 0.0112, 0.0094, 0.0169, 0.0065,
    0.0116, 0.0529, // mental
    0.0033, 0.0251, 0.0084, 0.0317, 0.0070, 0.0511, 0.0089, 0.0182, 0.0096, 0.0316, 0.0005, 0.0118,
    0.0234, 0.0281, // physical
    0.1104, 0.0534, 0.0185, 0.0189, 0.0020, 0.0995, 0.0681, 0.0341, 0.0000,
    // goalkeeping
    0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0.0000,
];

const WIDE_MIDFIELDER: [f32; 50] = [
    // technical
    0.0040, 0.0329, 0.0516, 0.0458, 0.0406, 0.0114, 0.0198, 0.0147, 0.0061, 0.0068, 0.0036, 0.0210,
    0.0054, 0.0412, // mental
    0.0000, 0.0210, 0.0148, 0.0347, 0.0088, 0.0500, 0.0044, 0.0094, 0.0097, 0.0184, 0.0113, 0.0252,
    0.0331, 0.0316, // physical
    0.1358, 0.0616, 0.0152, 0.0074, 0.0019, 0.1057, 0.0564, 0.0387, 0.0000,
    // goalkeeping
    0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0.0000,
];

const WINGBACK: [f32; 50] = [
    // technical
    0.0074, 0.0102, 0.0336, 0.0217, 0.0428, 0.0069, 0.0070, 0.0152, 0.0000, 0.0000, 0.0453, 0.0146,
    0.0089, 0.0536, // mental
    0.0000, 0.0408, 0.0081, 0.0194, 0.0243, 0.0693, 0.0083, 0.0373, 0.0141, 0.0063, 0.0000, 0.0228,
    0.0194, 0.0471, // physical
    0.1091, 0.0540, 0.0081, 0.0141, 0.0076, 0.1095, 0.0834, 0.0298, 0.0000,
    // goalkeeping
    0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000, 0.0000,
    0.0000,
];

pub(super) struct AbilityWeights;

impl AbilityWeights {
    pub(super) fn for_position(position: PlayerPositionType) -> &'static [f32; 50] {
        use PlayerPositionType::*;
        match position {
            Goalkeeper => &GOALKEEPER,
            Sweeper | DefenderCenter | DefenderCenterLeft | DefenderCenterRight => &DEFENDER_CENTER,
            DefenderLeft | DefenderRight => &FULLBACK,
            WingbackLeft | WingbackRight => &WINGBACK,
            DefensiveMidfielder => &DEFENSIVE_MIDFIELDER,
            MidfielderCenter | MidfielderCenterLeft | MidfielderCenterRight => &MIDFIELDER_CENTER,
            MidfielderLeft | MidfielderRight => &WIDE_MIDFIELDER,
            AttackingMidfielderCenter => &ATTACKING_MIDFIELDER_CENTER,
            AttackingMidfielderLeft | AttackingMidfielderRight | ForwardLeft | ForwardRight => {
                &WIDE_ATTACKER
            }
            Striker | ForwardCenter => &STRIKER,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerSkills;

    #[derive(serde::Deserialize)]
    struct Fixture {
        id: u32,
        position: PlayerPositionType,
        ca: u8,
        skills: PlayerSkills,
    }

    #[test]
    fn held_out_source_profiles_match_recorded_ability() {
        let fixtures: Vec<Fixture> =
            serde_json::from_str(include_str!("calibration_fixtures.json")).unwrap();
        let mut absolute_error = 0;
        for fixture in &fixtures {
            assert_eq!(fixture.id % 5, 0, "calibration fixtures must be held out");
            let got = fixture
                .skills
                .calculate_ability_for_position(fixture.position);
            absolute_error += got.abs_diff(fixture.ca) as u32;
        }
        let mae = absolute_error as f32 / fixtures.len() as f32;
        assert!(mae < 5.0, "source-profile MAE {mae}");
    }

    #[test]
    fn flat_ability_round_trips_for_every_position_and_budget() {
        use PlayerPositionType::*;
        for position in [
            Goalkeeper,
            Sweeper,
            DefenderCenter,
            DefenderLeft,
            DefenderRight,
            WingbackLeft,
            WingbackRight,
            DefensiveMidfielder,
            MidfielderCenter,
            MidfielderLeft,
            MidfielderRight,
            AttackingMidfielderCenter,
            AttackingMidfielderLeft,
            AttackingMidfielderRight,
            ForwardLeft,
            ForwardCenter,
            ForwardRight,
            Striker,
        ] {
            let weights = AbilityWeights::for_position(position);
            assert!(weights.iter().all(|w| *w >= 0.0));
            assert!((weights.iter().sum::<f32>() - 1.0).abs() < 0.002);
            assert_eq!(weights[36], 0.0, "readiness has no CA cost");
            for target in 1..=200 {
                let skills = PlayerSkills::flat_for_ability(target);
                assert_eq!(skills.calculate_ability_for_position(position), target);
                assert_eq!(skills.calculate_ability(), target);
            }
        }
    }
}

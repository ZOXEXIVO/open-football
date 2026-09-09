//! Moved verbatim out of `helpers.rs` — see that file's `mod tier_helper_tests`.

use crate::transfers::pipeline::PipelineProcessor;
use crate::{PlayerFieldPositionGroup, ReputationLevel};

const TIERS: [ReputationLevel; 6] = [
    ReputationLevel::Elite,
    ReputationLevel::Continental,
    ReputationLevel::National,
    ReputationLevel::Regional,
    ReputationLevel::Local,
    ReputationLevel::Amateur,
];

const GROUPS: [PlayerFieldPositionGroup; 4] = [
    PlayerFieldPositionGroup::Goalkeeper,
    PlayerFieldPositionGroup::Defender,
    PlayerFieldPositionGroup::Midfielder,
    PlayerFieldPositionGroup::Forward,
];

/// The tier band under test, resolved at each enum midpoint.
struct TierFx;

impl TierFx {
    /// Tier midpoint reputation score — used to validate that the
    /// continuous curve hits the calibrated values at the centre of
    /// each enum band.
    fn level_midpoint_score(level: &ReputationLevel) -> f32 {
        match level {
            ReputationLevel::Elite => 0.900,
            ReputationLevel::Continental => 0.725,
            ReputationLevel::National => 0.575,
            ReputationLevel::Regional => 0.400,
            ReputationLevel::Local => 0.225,
            ReputationLevel::Amateur => 0.075,
        }
    }

    fn baseline(level: &ReputationLevel, group: PlayerFieldPositionGroup) -> u8 {
        PipelineProcessor::tier_starter_ca_score(Self::level_midpoint_score(level), group)
    }

    fn ceiling(level: &ReputationLevel, group: PlayerFieldPositionGroup) -> u8 {
        PipelineProcessor::tier_target_ceiling_score(Self::level_midpoint_score(level), group)
    }
}

#[test]
fn baseline_is_strictly_decreasing_by_tier_within_each_group() {
    for group in GROUPS {
        let baselines: Vec<u8> = TIERS.iter().map(|t| TierFx::baseline(t, group)).collect();
        for window in baselines.windows(2) {
            assert!(
                window[0] > window[1],
                "tier baselines must be strictly decreasing for {:?}: {:?}",
                group,
                baselines
            );
        }
    }
}

#[test]
fn ceiling_is_at_least_baseline_for_every_tier_and_group() {
    for tier in &TIERS {
        for group in GROUPS {
            let b = TierFx::baseline(tier, group);
            let c = TierFx::ceiling(tier, group);
            assert!(
                c >= b,
                "ceiling {} below baseline {} for {:?}/{:?}",
                c,
                b,
                tier,
                group
            );
        }
    }
}

#[test]
fn elite_continental_can_reach_world_class() {
    // Elite-tier scouts must be allowed to recommend genuine
    // world-class players (180+); Continental at minimum top-bracket
    // (~155+). Calibration regression guard.
    let elite_fwd_ceiling =
        TierFx::ceiling(&ReputationLevel::Elite, PlayerFieldPositionGroup::Forward);
    assert!(
        elite_fwd_ceiling >= 180,
        "elite forward ceiling = {}",
        elite_fwd_ceiling
    );

    let cont_fwd_ceiling = TierFx::ceiling(
        &ReputationLevel::Continental,
        PlayerFieldPositionGroup::Forward,
    );
    assert!(
        cont_fwd_ceiling >= 155,
        "continental forward ceiling = {}",
        cont_fwd_ceiling
    );
}

#[test]
fn small_clubs_disciplined_below_world_class() {
    // Local / Amateur clubs should never reach top-class players
    // through the tier window — ensures the listed-star sweep won't
    // route Mbappé to a Sunday-league suitor.
    let local_ceiling = TierFx::ceiling(&ReputationLevel::Local, PlayerFieldPositionGroup::Forward);
    let amateur_ceiling =
        TierFx::ceiling(&ReputationLevel::Amateur, PlayerFieldPositionGroup::Forward);
    assert!(
        local_ceiling < 100,
        "local forward ceiling = {}",
        local_ceiling
    );
    assert!(
        amateur_ceiling < 80,
        "amateur forward ceiling = {}",
        amateur_ceiling
    );
}

#[test]
fn goalkeepers_score_below_outfield_at_same_tier() {
    for tier in &TIERS {
        let gk = TierFx::baseline(tier, PlayerFieldPositionGroup::Goalkeeper);
        let mid = TierFx::baseline(tier, PlayerFieldPositionGroup::Midfielder);
        assert!(
            gk < mid,
            "GK baseline {} not below MID baseline {} at {:?}",
            gk,
            mid,
            tier
        );
    }
}

#[test]
fn quality_tolerance_decreases_with_reputation() {
    // Top clubs upgrade aggressively (small tolerance); small clubs
    // patient (large tolerance). Monotonic in score.
    let mut prev = PipelineProcessor::tier_quality_tolerance_score(0.0);
    for step in 1..=10 {
        let s = step as f32 / 10.0;
        let cur = PipelineProcessor::tier_quality_tolerance_score(s);
        assert!(
            cur <= prev,
            "tolerance must be non-increasing as reputation rises (s={}: {} > prev {})",
            s,
            cur,
            prev
        );
        prev = cur;
    }
    let elite = PipelineProcessor::tier_quality_tolerance_score(0.95);
    let amateur = PipelineProcessor::tier_quality_tolerance_score(0.05);
    assert!(
        amateur > elite,
        "amateur {} should exceed elite {}",
        amateur,
        elite
    );
}

#[test]
fn baseline_score_curve_pins_tier_anchors() {
    // Anchor calibration regression guard: midpoint of each tier
    // returns the calibrated value the rest of the pipeline assumes.
    let cases = [
        (ReputationLevel::Elite, 145i16),
        (ReputationLevel::Continental, 130),
        (ReputationLevel::National, 110),
        (ReputationLevel::Regional, 88),
        (ReputationLevel::Local, 70),
        (ReputationLevel::Amateur, 55),
    ];
    for (tier, expected_mid_baseline) in &cases {
        let s = TierFx::level_midpoint_score(tier);
        // Midfielder offset is 0 — direct calibration check.
        let baseline =
            PipelineProcessor::tier_starter_ca_score(s, PlayerFieldPositionGroup::Midfielder);
        assert_eq!(
            baseline as i16, *expected_mid_baseline,
            "midpoint baseline for {:?}: expected {}, got {}",
            tier, expected_mid_baseline, baseline
        );
    }
}

#[test]
fn baseline_score_curve_is_monotonic_in_score() {
    for group in GROUPS {
        let mut prev = PipelineProcessor::tier_starter_ca_score(0.0, group);
        for step in 1..=20 {
            let s = step as f32 / 20.0;
            let cur = PipelineProcessor::tier_starter_ca_score(s, group);
            assert!(
                cur >= prev,
                "score baseline not monotonic at {}/{:?}: {} < {}",
                s,
                group,
                cur,
                prev
            );
            prev = cur;
        }
    }
}

#[test]
fn position_evaluation_ability_is_canonical_alias() {
    // The helper must return exactly what
    // `skills.calculate_ability_for_position(player.position())`
    // produces — it's a naming alias, not a separate calculation.
    // Construction goes through `PlayerGenerator::generate` (the
    // single source of truth for Player init), then we assert the
    // helper agrees with the direct call.
    use crate::club::player::generators::PlayerGenerator;
    use crate::{PeopleNameGeneratorData, PlayerPositionType};
    use chrono::NaiveDate;

    let names = PeopleNameGeneratorData {
        first_names: vec!["Tier".to_string()],
        last_names: vec!["Tester".to_string()],
        nicknames: Vec::new(),
    };
    let bd = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();
    let player =
        PlayerGenerator::generate(1, bd, PlayerPositionType::MidfielderCenter, 150, &names);
    let direct = player
        .skills
        .calculate_ability_for_position(player.position());
    let via_helper = PipelineProcessor::position_evaluation_ability(&player);
    assert_eq!(
        via_helper, direct,
        "position_evaluation_ability must mirror calculate_ability_for_position"
    );
}

#[test]
fn continental_weak_gk_clears_quality_upgrade_threshold() {
    // A Continental-tier club with a 110-CA starting goalkeeper
    // should fall below `baseline - tolerance` and so be flagged
    // for QualityUpgrade. Calibration regression guard for the
    // Spartak-style scenario.
    let cont_score = TierFx::level_midpoint_score(&ReputationLevel::Continental);
    let baseline =
        PipelineProcessor::tier_starter_ca_score(cont_score, PlayerFieldPositionGroup::Goalkeeper);
    let tolerance = PipelineProcessor::tier_quality_tolerance_score(cont_score);
    let threshold = baseline as i16 - tolerance;

    assert!(
        (110_i16) < threshold,
        "weak GK (CA=110) must be below upgrade threshold {} for Continental tier (baseline={}, tolerance={})",
        threshold,
        baseline,
        tolerance
    );

    // Symmetrically, a tier-fit GK at baseline must NOT trigger.
    assert!(
        (baseline as i16) >= threshold,
        "at-tier GK (CA={}) must clear threshold {}",
        baseline,
        threshold
    );
}

#[test]
fn local_ceiling_cannot_reach_world_class_targets() {
    // Local / Amateur clubs must not have CA windows wide enough
    // to chase 160+ players via the listed-sweep tier window.
    // Prevents impossible signings being shortlisted.
    for tier in &[ReputationLevel::Local, ReputationLevel::Amateur] {
        for group in GROUPS {
            let c = TierFx::ceiling(tier, group);
            assert!(
                c < 110,
                "{:?} {:?} ceiling {} would let CA-160 stars through the gate",
                tier,
                group,
                c
            );
        }
    }
}

#[test]
fn continental_window_admits_realistic_targets_blocks_unattainable() {
    // Continental tier should comfortably absorb a 130-CA listed
    // player (i.e. Mikhailov-class), but reject a 175-CA superstar
    // through the ceiling.
    let cont_score = TierFx::level_midpoint_score(&ReputationLevel::Continental);
    let ceiling_fwd =
        PipelineProcessor::tier_target_ceiling_score(cont_score, PlayerFieldPositionGroup::Forward);
    let baseline_fwd =
        PipelineProcessor::tier_starter_ca_score(cont_score, PlayerFieldPositionGroup::Forward);
    let floor_fwd = baseline_fwd.saturating_sub(20);

    assert!(
        130 >= floor_fwd && 130 <= ceiling_fwd,
        "Continental window [{}..={}] must contain CA 130",
        floor_fwd,
        ceiling_fwd
    );
    assert!(
        175 > ceiling_fwd,
        "Continental ceiling {} must reject CA 175 (out-of-tier)",
        ceiling_fwd
    );
}

#[test]
fn elite_window_reaches_world_class_targets() {
    // Elite clubs must be able to chase 175+ targets via the
    // tier window — the original bug masked these from elite
    // scouts because the squad-mean cap was too low.
    let elite_score = TierFx::level_midpoint_score(&ReputationLevel::Elite);
    let ceiling_fwd = PipelineProcessor::tier_target_ceiling_score(
        elite_score,
        PlayerFieldPositionGroup::Forward,
    );
    assert!(
        175 <= ceiling_fwd,
        "Elite ceiling {} must admit CA 175 world-class forward",
        ceiling_fwd
    );
}

#[test]
fn within_tier_continuous_score_differentiates_clubs() {
    // Mid-Continental and top-of-Continental clubs should NOT get
    // the same baseline — that's the whole point of the score
    // path. Tests the continuous calibration is genuinely
    // differentiating, not silently snapping to enum buckets.
    let mid_cont =
        PipelineProcessor::tier_starter_ca_score(0.68, PlayerFieldPositionGroup::Midfielder);
    let top_cont =
        PipelineProcessor::tier_starter_ca_score(0.79, PlayerFieldPositionGroup::Midfielder);
    assert!(
        top_cont > mid_cont,
        "top-of-Continental baseline ({}) must exceed mid-Continental ({})",
        top_cont,
        mid_cont
    );
}

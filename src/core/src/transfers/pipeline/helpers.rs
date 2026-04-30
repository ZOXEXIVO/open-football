use chrono::{Datelike, NaiveDate};

use crate::shared::CurrencyValue;
use crate::transfers::pipeline::processor::{PipelineProcessor, PlayerSummary};
use crate::transfers::pipeline::{
    DetailedScoutingReport, ReportRiskFlag, ScoutingRecommendation, TransferNeedReason,
    TransferRequest,
};
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{
    Club, Country, Person, Player, PlayerFieldPositionGroup, PlayerStatusType, ReputationLevel,
    StaffPosition, TeamType,
};

impl PipelineProcessor {
    pub(super) fn is_january_window(date: NaiveDate) -> bool {
        date.month() == 1
    }

    /// Full reset only at transfer window opening dates
    pub(super) fn is_window_start(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        (month == 5 && day == 31) || (month == 6 && day == 1) || (month == 1 && day == 1)
    }

    /// Re-evaluate during transfer windows.
    /// Daily during the first week of each window for fast pipeline startup,
    /// then weekly (Monday) for the rest of the window.
    pub(super) fn should_evaluate(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();

        // First week of summer window (June 1-7) or winter window (Jan 1-7): daily
        if (month == 6 && day <= 7) || (month == 1 && day <= 7) {
            return true;
        }

        // Rest of window: weekly on Monday
        ((month >= 6 && month <= 8) || month == 1) && date.weekday() == chrono::Weekday::Mon
    }

    pub fn transfer_need_reason_text(reason: &TransferNeedReason) -> &'static str {
        match reason {
            TransferNeedReason::FormationGap => "Formation gap — no player for required position",
            TransferNeedReason::QualityUpgrade => {
                "Quality upgrade — current player below squad level"
            }
            TransferNeedReason::DepthCover => "Squad depth — need backup for position group",
            TransferNeedReason::SuccessionPlanning => {
                "Succession planning — aging key player needs replacement"
            }
            TransferNeedReason::DevelopmentSigning => {
                "Development signing — young prospect with high potential"
            }
            TransferNeedReason::StaffRecommendation => "Staff recommendation",
            TransferNeedReason::LoanToFillSquad => "Loan to fill squad — cannot afford to buy",
            TransferNeedReason::ExperiencedHead => {
                "Experienced head — need senior player for leadership"
            }
            TransferNeedReason::SquadPadding => "Squad padding — too few players to compete",
            TransferNeedReason::CheapReinforcement => {
                "Cheap reinforcement — affordable quality improvement"
            }
            TransferNeedReason::InjuryCoverLoan => "Injury cover — loan to replace injured player",
            TransferNeedReason::OpportunisticLoanUpgrade => {
                "Opportunistic loan — player better than current options"
            }
        }
    }

    /// Build a transfer reason string from the transfer request and optional scout report.
    pub(super) fn build_transfer_reason(
        request: Option<&TransferRequest>,
        report: Option<&DetailedScoutingReport>,
    ) -> String {
        let need_reason = request.map(|r| Self::transfer_need_reason_text(&r.reason));

        let scout_reason = report.map(|r| {
            let rec = match r.recommendation {
                ScoutingRecommendation::StrongBuy => "Strong buy",
                ScoutingRecommendation::Buy => "Buy",
                ScoutingRecommendation::Consider => "Consider",
                ScoutingRecommendation::Pass => "Pass",
            };
            let ability_label = Self::ability_label(r.assessed_ability);
            let potential_label = Self::ability_label(r.assessed_potential);
            format!(
                "Scout: {} (ability: {}, potential: {}, confidence: {:.0}%)",
                rec,
                ability_label,
                potential_label,
                r.confidence * 100.0
            )
        });

        match (need_reason, scout_reason) {
            (Some(need), Some(scout)) => format!("{} — {}", need, scout),
            (Some(need), None) => need.to_string(),
            (None, Some(scout)) => scout,
            (None, None) => String::new(),
        }
    }

    /// Convert a raw assessed ability value to a qualitative label.
    fn ability_label(value: u8) -> &'static str {
        match value {
            0..=30 => "Very poor",
            31..=60 => "Poor",
            61..=80 => "Below average",
            81..=100 => "Average",
            101..=120 => "Decent",
            121..=140 => "Good",
            141..=160 => "Very good",
            161..=180 => "Excellent",
            181..=200 => "World class",
            _ => "Unknown",
        }
    }

    pub(super) fn find_player_in_country<'a>(
        country: &'a Country,
        player_id: u32,
    ) -> Option<&'a Player> {
        for club in &country.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.find(player_id) {
                    return Some(player);
                }
            }
        }
        None
    }

    /// Resolve player full name and selling club name from the country data.
    pub(super) fn resolve_player_and_club_name(
        country: &Country,
        player_id: u32,
        club_id: u32,
    ) -> (String, String) {
        let player_name = country
            .clubs
            .iter()
            .flat_map(|c| c.teams.iter())
            .find_map(|t| t.players.find(player_id))
            .map(|p| p.full_name.to_string())
            .unwrap_or_default();

        let club_name = country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        (player_name, club_name)
    }

    pub(super) fn find_player_in_club<'a>(club: &'a Club, player_id: u32) -> Option<&'a Player> {
        for team in &club.teams.teams {
            if let Some(player) = team.players.find(player_id) {
                return Some(player);
            }
        }
        None
    }

    pub(super) fn find_player_summary_in_country(
        country: &Country,
        player_id: u32,
        date: NaiveDate,
    ) -> Option<PlayerSummary> {
        for club in &country.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.find(player_id) {
                    let skill_ability = Self::position_evaluation_ability(player);
                    // Blended reputation, not just `world` — keeps domestic
                    // strength visible in valuation for clubs whose home
                    // standing exceeds their international footprint.
                    let (league_reputation, club_reputation) =
                        PlayerValuationCalculator::seller_context(country, club);
                    let estimated_value =
                        PlayerValuationCalculator::calculate_value_with_price_level(
                            player,
                            date,
                            country.settings.pricing.price_level,
                            league_reputation,
                            club_reputation,
                        )
                        .amount;
                    let (contract_months_remaining, salary) = player
                        .contract
                        .as_ref()
                        .map(|c| {
                            let days = (c.expiration - date).num_days().max(0);
                            ((days / 30).min(i16::MAX as i64) as i16, c.salary)
                        })
                        .unwrap_or((0, 0));
                    return Some(PlayerSummary {
                        player_id: player.id,
                        club_id: club.id,
                        country_id: country.id,
                        continent_id: country.continent_id,
                        country_code: country.code.clone(),
                        player_name: player.full_name.to_string(),
                        club_name: club.name.clone(),
                        position: player.position(),
                        position_group: player.position().position_group(),
                        age: player.age(date),
                        estimated_value,
                        is_listed: player.statuses.get().contains(&PlayerStatusType::Lst),
                        is_loan_listed: player.statuses.get().contains(&PlayerStatusType::Loa),
                        skill_ability,
                        average_rating: player.statistics.average_rating,
                        goals: player.statistics.goals,
                        assists: player.statistics.assists,
                        appearances: player.statistics.total_games(),
                        determination: player.skills.mental.determination,
                        work_rate: player.skills.mental.work_rate,
                        composure: player.skills.mental.composure,
                        anticipation: player.skills.mental.anticipation,
                        technical_avg: player.skills.technical.average(),
                        mental_avg: player.skills.mental.average(),
                        physical_avg: player.skills.physical.average(),
                        current_reputation: player.player_attributes.current_reputation,
                        home_reputation: player.player_attributes.home_reputation,
                        world_reputation: player.player_attributes.world_reputation,
                        country_reputation: country.reputation,
                        is_injured: player.player_attributes.is_injured,
                        contract_months_remaining,
                        salary,
                    });
                }
            }
        }
        None
    }

    /// Derive risk flags for a scouted player from their observable signals.
    /// Buyer rep is passed in so we can flag wage demands that blow the budget.
    /// Thresholds (determination floor, age cutoff, contract-month window,
    /// rep gap) live in `ScoutingConfig::risk_flags`.
    pub(super) fn evaluate_risk_flags(
        is_injured: bool,
        determination: f32,
        age: u8,
        contract_months_remaining: i16,
        player_world_rep: i16,
        buyer_world_rep: i16,
    ) -> Vec<ReportRiskFlag> {
        super::scouting_config::ScoutingConfig::default().risk_flags_for(
            is_injured,
            determination,
            age,
            contract_months_remaining,
            player_world_rep,
            buyer_world_rep,
        )
    }

    pub(super) fn club_world_reputation(club: &Club) -> i16 {
        club.teams
            .iter()
            .find(|t| matches!(t.team_type, TeamType::Main))
            .map(|t| t.reputation.world as i16)
            .unwrap_or(0)
    }

    /// Best `judging_player_data` across the club's scouting staff.
    /// Drives how aggressively the data department narrows the scout pool.
    /// Defaults from `ScoutingConfig::data_prefilter::default_data_skill`
    /// when the club has no scouts at all.
    pub(super) fn club_data_analysis_skill(club: &Club) -> u8 {
        let default_skill = super::scouting_config::ScoutingConfig::default()
            .data_prefilter
            .default_data_skill;
        club.teams
            .iter()
            .flat_map(|t| t.staffs.iter())
            .filter(|s| {
                s.contract
                    .as_ref()
                    .map(
                        |c| matches!(c.position, StaffPosition::Scout | StaffPosition::ChiefScout,),
                    )
                    .unwrap_or(false)
            })
            .map(|s| s.staff_attributes.data_analysis.judging_player_data)
            .max()
            .unwrap_or(default_skill)
    }

    /// Performance-adjusted data score used as a pre-scouting filter.
    /// Weights ability, form (rating × appearances), and raw output (G+A).
    pub(super) fn player_data_score(p: &PlayerSummary) -> f32 {
        let ability = p.skill_ability as f32 * 0.4;
        let form = p.average_rating * (p.appearances.min(40) as f32 / 4.0);
        let output = ((p.goals + p.assists).min(30)) as f32 * 0.3;
        ability + form + output
    }

    pub(super) fn get_scout_skills(club: &Club, scout_id: u32) -> (u8, u8) {
        for team in &club.teams.teams {
            if let Some(staff) = team.staffs.find(scout_id) {
                return (
                    staff.staff_attributes.knowledge.judging_player_ability,
                    staff.staff_attributes.knowledge.judging_player_potential,
                );
            }
        }
        // Pointer is stale (staff was removed mid-tick or assignment was
        // never tied to a real scout). Use the configured "missing staff"
        // defaults rather than panic — quality silently downgrades.
        let cfg = super::scouting_config::ScoutingConfig::default();
        (
            cfg.observation.default_judging_when_staff_missing,
            cfg.observation.default_judging_when_staff_missing,
        )
    }

    /// Estimate a player's growth potential from observable attributes.
    /// Scouts can't see PA — they judge ceiling from age, character, and current skill level.
    /// Young players with strong determination, work rate, composure show higher ceiling.
    pub(super) fn estimate_growth_potential(
        age: u8,
        determination: f32,
        work_rate: f32,
        composure: f32,
        anticipation: f32,
        current_skill_ability: u8,
    ) -> u8 {
        // Mental quality score: how much this player's character suggests growth (0.0-1.0)
        let mental_quality =
            ((determination + work_rate + composure + anticipation) / 4.0 - 1.0) / 19.0;
        let mental_factor = mental_quality.clamp(0.0, 1.0);

        // Age-based growth window: younger = more room to grow
        let base_growth = match age {
            0..=17 => 35.0,
            18 => 30.0,
            19 => 25.0,
            20 => 20.0,
            21 => 15.0,
            22 => 12.0,
            23 => 8.0,
            24 => 5.0,
            25 => 3.0,
            26..=27 => 1.0,
            _ => 0.0,
        };

        // Players already at high skill level have less room to grow
        let ceiling_factor = if current_skill_ability > 160 {
            0.3
        } else if current_skill_ability > 120 {
            0.6
        } else {
            1.0
        };

        (base_growth * mental_factor * ceiling_factor) as u8
    }

    pub(super) fn calculate_asking_price(
        player: &Player,
        country: &Country,
        club: &Club,
        date: NaiveDate,
        price_level: f32,
    ) -> CurrencyValue {
        // Selling clubs anchor on their own market context — a Serie A
        // club asking the same fee as a Maltese side for an identical
        // player is an obvious flatness bug. Pull the seller's blended
        // league + club reputation so the base value reflects who is
        // actually selling.
        let (league_rep, club_rep) = PlayerValuationCalculator::seller_context(country, club);
        let base_value = PlayerValuationCalculator::calculate_value_with_price_level(
            player,
            date,
            price_level,
            league_rep,
            club_rep,
        );

        let multiplier = if club.finance.balance.balance < 0 {
            0.9
        } else {
            1.1
        };

        CurrencyValue {
            amount: FormattingUtils::round_fee(base_value.amount * multiplier),
            currency: base_value.currency,
        }
    }

    pub(super) fn get_club_reputation(country: &Country, club_id: u32) -> f32 {
        country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .and_then(|c| c.teams.teams.first())
            .map(|t| t.reputation.attractiveness_factor())
            .unwrap_or(0.3)
    }

    pub(super) fn get_club_reputation_level(country: &Country, club_id: u32) -> ReputationLevel {
        country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .and_then(|c| c.teams.teams.first())
            .map(|t| t.reputation.level())
            .unwrap_or(ReputationLevel::Amateur)
    }

    pub(super) fn get_player_negotiation_data(
        country: &Country,
        player_id: u32,
        date: NaiveDate,
    ) -> (u8, f32) {
        Self::find_player_in_country(country, player_id)
            .map(|p| (p.age(date), p.attributes.ambition))
            .unwrap_or((25, 0.5))
    }

    pub(super) fn rep_level_value(level: &ReputationLevel) -> u8 {
        match level {
            ReputationLevel::Elite => 5,
            ReputationLevel::Continental => 4,
            ReputationLevel::National => 3,
            ReputationLevel::Regional => 2,
            ReputationLevel::Local => 1,
            ReputationLevel::Amateur => 0,
        }
    }

    /// Canonical "evaluate this player against a tier baseline" ability.
    /// Returns the position-weighted ability (1..200 scale) used
    /// throughout the transfer pipeline — squad evaluation, scout
    /// reach, listed-star sweeps. Both `current_ability` and this
    /// helper share the 1..200 scale and are kept in sync by the
    /// training and development paths
    /// (`development/tick.rs:190`, `training/result.rs:88`), which
    /// always recompute CA from `calculate_ability_for_position`.
    /// Naming the helper so call-sites can't accidentally mix it up
    /// with raw skill averages prevents the CA-vs-skill drift the
    /// audit flagged.
    pub(crate) fn position_evaluation_ability(player: &Player) -> u8 {
        player.skills.calculate_ability_for_position(player.position())
    }

    /// Linear-interpolated lookup of base baseline CA from a continuous
    /// reputation score. Anchors are calibrated so that the midpoint of
    /// each enum tier reproduces the bucketed baseline the rest of the
    /// pipeline expects. Score is `Reputation::overall_score()` (0..1).
    fn baseline_anchor_curve(score: f32) -> f32 {
        const ANCHORS: [(f32, f32); 7] = [
            (0.000, 50.0),
            (0.075, 55.0),
            (0.225, 70.0),
            (0.400, 88.0),
            (0.575, 110.0),
            (0.725, 130.0),
            (0.900, 145.0),
        ];
        let s = score.clamp(0.0, 1.0);
        // Above the top anchor we keep climbing — top-of-Elite (e.g. a
        // generational Real Madrid side) demands more than mid-Elite.
        if s >= ANCHORS[ANCHORS.len() - 1].0 {
            let (s_top, b_top) = ANCHORS[ANCHORS.len() - 1];
            let extrapolation = (s - s_top) * (162.0 - b_top) / (1.0 - s_top).max(1e-6);
            return b_top + extrapolation;
        }
        for window in ANCHORS.windows(2) {
            let (s0, b0) = window[0];
            let (s1, b1) = window[1];
            if s >= s0 && s <= s1 {
                let t = (s - s0) / (s1 - s0).max(1e-6);
                return b0 + (b1 - b0) * t;
            }
        }
        ANCHORS[0].1
    }

    /// Linear-interpolated headroom (max CA above baseline a club can
    /// realistically pursue). Anchored at the same enum-tier midpoints
    /// as [`baseline_anchor_curve`].
    fn headroom_anchor_curve(score: f32) -> f32 {
        const ANCHORS: [(f32, f32); 7] = [
            (0.000, 6.0),
            (0.075, 8.0),
            (0.225, 10.0),
            (0.400, 14.0),
            (0.575, 22.0),
            (0.725, 35.0),
            (0.900, 55.0),
        ];
        let s = score.clamp(0.0, 1.0);
        if s >= ANCHORS[ANCHORS.len() - 1].0 {
            let (s_top, h_top) = ANCHORS[ANCHORS.len() - 1];
            let extrapolation = (s - s_top) * (65.0 - h_top) / (1.0 - s_top).max(1e-6);
            return h_top + extrapolation;
        }
        for window in ANCHORS.windows(2) {
            let (s0, h0) = window[0];
            let (s1, h1) = window[1];
            if s >= s0 && s <= s1 {
                let t = (s - s0) / (s1 - s0).max(1e-6);
                return h0 + (h1 - h0) * t;
            }
        }
        ANCHORS[0].1
    }

    /// Per-group offset applied on top of the base baseline. Goalkeepers
    /// naturally score lower on the unified CA scale (fewer outfield-
    /// style attributes feed the rating); forwards a touch higher. Kept
    /// here as the single source of truth — neither evaluation nor
    /// recommendations carries its own per-group adjustment.
    fn group_baseline_offset(group: PlayerFieldPositionGroup) -> i16 {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => -8,
            PlayerFieldPositionGroup::Defender => -3,
            PlayerFieldPositionGroup::Midfielder => 0,
            PlayerFieldPositionGroup::Forward => 2,
        }
    }

    /// Expected current-ability of an at-tier starter for a club whose
    /// reputation `overall_score` is `score` (0..1). The continuous
    /// version of [`tier_starter_ca`] — a club mid-Continental gets a
    /// different baseline from a top-of-Continental club, instead of
    /// snapping to the same enum bucket. Position offsets are applied
    /// uniformly via [`group_baseline_offset`].
    pub(crate) fn tier_starter_ca_score(
        score: f32,
        group: PlayerFieldPositionGroup,
    ) -> u8 {
        let base = Self::baseline_anchor_curve(score);
        let offset = Self::group_baseline_offset(group);
        (base.round() as i16 + offset).clamp(20, 200) as u8
    }

    /// Continuous-score counterpart of [`tier_target_ceiling`].
    pub(crate) fn tier_target_ceiling_score(
        score: f32,
        group: PlayerFieldPositionGroup,
    ) -> u8 {
        let baseline = Self::tier_starter_ca_score(score, group);
        let headroom = Self::headroom_anchor_curve(score).round() as i16;
        (baseline as i16 + headroom).clamp(20, 200) as u8
    }

    /// Continuous-score counterpart of [`tier_quality_tolerance`]. Top
    /// clubs upgrade aggressively (small tolerance); small clubs
    /// patient. Linear in 1 - score so that going up the reputation
    /// ladder reduces tolerance smoothly, no enum cliff.
    pub(crate) fn tier_quality_tolerance_score(score: f32) -> i16 {
        let s = score.clamp(0.0, 1.0);
        let raw = 4.0 + (1.0 - s) * 11.0; // 4 at top, 15 at the bottom
        raw.round() as i16
    }

}

#[cfg(test)]
mod tier_helper_tests {
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
        PipelineProcessor::tier_starter_ca_score(level_midpoint_score(level), group)
    }

    fn ceiling(level: &ReputationLevel, group: PlayerFieldPositionGroup) -> u8 {
        PipelineProcessor::tier_target_ceiling_score(level_midpoint_score(level), group)
    }

    #[test]
    fn baseline_is_strictly_decreasing_by_tier_within_each_group() {
        for group in GROUPS {
            let baselines: Vec<u8> = TIERS.iter().map(|t| baseline(t, group)).collect();
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
                let b = baseline(tier, group);
                let c = ceiling(tier, group);
                assert!(c >= b, "ceiling {} below baseline {} for {:?}/{:?}", c, b, tier, group);
            }
        }
    }

    #[test]
    fn elite_continental_can_reach_world_class() {
        // Elite-tier scouts must be allowed to recommend genuine
        // world-class players (180+); Continental at minimum top-bracket
        // (~155+). Calibration regression guard.
        let elite_fwd_ceiling =
            ceiling(&ReputationLevel::Elite, PlayerFieldPositionGroup::Forward);
        assert!(elite_fwd_ceiling >= 180, "elite forward ceiling = {}", elite_fwd_ceiling);

        let cont_fwd_ceiling =
            ceiling(&ReputationLevel::Continental, PlayerFieldPositionGroup::Forward);
        assert!(cont_fwd_ceiling >= 155, "continental forward ceiling = {}", cont_fwd_ceiling);
    }

    #[test]
    fn small_clubs_disciplined_below_world_class() {
        // Local / Amateur clubs should never reach top-class players
        // through the tier window — ensures the listed-star sweep won't
        // route Mbappé to a Sunday-league suitor.
        let local_ceiling =
            ceiling(&ReputationLevel::Local, PlayerFieldPositionGroup::Forward);
        let amateur_ceiling =
            ceiling(&ReputationLevel::Amateur, PlayerFieldPositionGroup::Forward);
        assert!(local_ceiling < 100, "local forward ceiling = {}", local_ceiling);
        assert!(amateur_ceiling < 80, "amateur forward ceiling = {}", amateur_ceiling);
    }

    #[test]
    fn goalkeepers_score_below_outfield_at_same_tier() {
        for tier in &TIERS {
            let gk = baseline(tier, PlayerFieldPositionGroup::Goalkeeper);
            let mid = baseline(tier, PlayerFieldPositionGroup::Midfielder);
            assert!(gk < mid, "GK baseline {} not below MID baseline {} at {:?}", gk, mid, tier);
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
        assert!(amateur > elite, "amateur {} should exceed elite {}", amateur, elite);
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
            let s = level_midpoint_score(tier);
            // Midfielder offset is 0 — direct calibration check.
            let baseline = PipelineProcessor::tier_starter_ca_score(
                s,
                PlayerFieldPositionGroup::Midfielder,
            );
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
                    s, group, cur, prev
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
        let player = PlayerGenerator::generate(
            1,
            bd,
            PlayerPositionType::MidfielderCenter,
            150,
            &names,
        );
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
        let cont_score = level_midpoint_score(&ReputationLevel::Continental);
        let baseline =
            PipelineProcessor::tier_starter_ca_score(cont_score, PlayerFieldPositionGroup::Goalkeeper);
        let tolerance = PipelineProcessor::tier_quality_tolerance_score(cont_score);
        let threshold = baseline as i16 - tolerance;

        assert!(
            (110_i16) < threshold,
            "weak GK (CA=110) must be below upgrade threshold {} for Continental tier (baseline={}, tolerance={})",
            threshold, baseline, tolerance
        );

        // Symmetrically, a tier-fit GK at baseline must NOT trigger.
        assert!(
            (baseline as i16) >= threshold,
            "at-tier GK (CA={}) must clear threshold {}",
            baseline, threshold
        );
    }

    #[test]
    fn local_ceiling_cannot_reach_world_class_targets() {
        // Local / Amateur clubs must not have CA windows wide enough
        // to chase 160+ players via the listed-sweep tier window.
        // Prevents impossible signings being shortlisted.
        for tier in &[ReputationLevel::Local, ReputationLevel::Amateur] {
            for group in GROUPS {
                let c = ceiling(tier, group);
                assert!(
                    c < 110,
                    "{:?} {:?} ceiling {} would let CA-160 stars through the gate",
                    tier, group, c
                );
            }
        }
    }

    #[test]
    fn continental_window_admits_realistic_targets_blocks_unattainable() {
        // Continental tier should comfortably absorb a 130-CA listed
        // player (i.e. Mikhailov-class), but reject a 175-CA superstar
        // through the ceiling.
        let cont_score = level_midpoint_score(&ReputationLevel::Continental);
        let ceiling_fwd = PipelineProcessor::tier_target_ceiling_score(
            cont_score,
            PlayerFieldPositionGroup::Forward,
        );
        let baseline_fwd = PipelineProcessor::tier_starter_ca_score(
            cont_score,
            PlayerFieldPositionGroup::Forward,
        );
        let floor_fwd = baseline_fwd.saturating_sub(20);

        assert!(
            130 >= floor_fwd && 130 <= ceiling_fwd,
            "Continental window [{}..={}] must contain CA 130",
            floor_fwd, ceiling_fwd
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
        let elite_score = level_midpoint_score(&ReputationLevel::Elite);
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
        let mid_cont = PipelineProcessor::tier_starter_ca_score(
            0.68,
            PlayerFieldPositionGroup::Midfielder,
        );
        let top_cont = PipelineProcessor::tier_starter_ca_score(
            0.79,
            PlayerFieldPositionGroup::Midfielder,
        );
        assert!(
            top_cont > mid_cont,
            "top-of-Continental baseline ({}) must exceed mid-Continental ({})",
            top_cont, mid_cont
        );
    }
}

#[cfg(test)]
mod group_need_tests {
    use crate::transfers::pipeline::evaluation::{
        compute_group_needs, group_depth_requirement, GroupNeed, NeedKind,
    };
    use crate::transfers::pipeline::processor::SquadPlayerInfo;
    use crate::{MatchTacticType, PlayerFieldPositionGroup, PlayerPositionType, TACTICS_POSITIONS};
    use std::collections::HashMap;

    fn t442_positions() -> &'static [PlayerPositionType; 11] {
        let (_, positions) = TACTICS_POSITIONS
            .iter()
            .find(|(t, _)| *t == MatchTacticType::T442)
            .expect("T442 tactic must exist");
        positions
    }

    fn squad_player(
        id: u32,
        primary: PlayerPositionType,
        ca: u8,
    ) -> SquadPlayerInfo {
        let mut levels: HashMap<PlayerPositionType, u8> = HashMap::new();
        levels.insert(primary, 20);
        SquadPlayerInfo {
            player_id: id,
            primary_position: primary,
            current_ability: ca,
            potential_ability: ca,
            age: 26,
            position_levels: levels,
            appearances: 10,
            is_injured: false,
            recovery_days: 0,
            injury_days: 0,
        }
    }

    /// Build position_coverage with each formation slot covered by the
    /// best-fit squad player. Mirrors the production logic enough to
    /// drive the detector deterministically.
    fn coverage_from_squad(
        squad: &[SquadPlayerInfo],
        formation: &[PlayerPositionType; 11],
    ) -> Vec<(PlayerPositionType, Option<u32>, u8)> {
        let mut used: Vec<u32> = Vec::new();
        let mut out = Vec::new();
        for &slot in formation.iter() {
            let pick = squad
                .iter()
                .filter(|p| !used.contains(&p.player_id))
                .filter(|p| p.primary_position.position_group() == slot.position_group())
                .max_by_key(|p| p.current_ability);
            match pick {
                Some(p) => {
                    used.push(p.player_id);
                    out.push((slot, Some(p.player_id), p.current_ability));
                }
                None => out.push((slot, None, 0)),
            }
        }
        out
    }

    fn continental_score() -> f32 {
        0.725
    }

    fn continental_tolerance() -> i16 {
        crate::transfers::pipeline::PipelineProcessor::tier_quality_tolerance_score(
            continental_score(),
        )
    }

    #[test]
    fn weak_gk_at_continental_club_triggers_quality_upgrade() {
        // Continental tier squad: every outfield slot at-baseline,
        // GK well below tier baseline. Detector must produce exactly
        // one QualityUpgrade need targeting the goalkeeper group.
        let formation = t442_positions();
        let mut squad = Vec::new();
        squad.push(squad_player(1, PlayerPositionType::Goalkeeper, 110));
        squad.push(squad_player(2, PlayerPositionType::Goalkeeper, 95));
        // Outfield: at-tier defenders / mids / forwards
        let outfield_positions = [
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::MidfielderLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardRight,
        ];
        for (i, pos) in outfield_positions.iter().enumerate() {
            squad.push(squad_player(10 + i as u32, *pos, 132));
        }
        // Add a couple of bench outfielders so depth checks pass
        squad.push(squad_player(50, PlayerPositionType::DefenderCenterLeft, 120));
        squad.push(squad_player(51, PlayerPositionType::DefenderCenterRight, 120));
        squad.push(squad_player(52, PlayerPositionType::MidfielderCenterLeft, 120));
        squad.push(squad_player(53, PlayerPositionType::MidfielderCenterRight, 120));
        squad.push(squad_player(54, PlayerPositionType::ForwardLeft, 118));

        let coverage = coverage_from_squad(&squad, formation);
        let needs: Vec<GroupNeed> = compute_group_needs(
            &squad,
            &coverage,
            formation,
            continental_score(),
            continental_tolerance(),
        );

        let gk_needs: Vec<&GroupNeed> = needs
            .iter()
            .filter(|n| n.group == PlayerFieldPositionGroup::Goalkeeper)
            .collect();
        assert_eq!(gk_needs.len(), 1, "expected exactly one GK need, got {:?}", needs);
        assert_eq!(
            gk_needs[0].kind,
            NeedKind::QualityUpgrade,
            "expected QualityUpgrade for weak GK, got {:?}",
            gk_needs[0].kind
        );
    }

    #[test]
    fn duplicate_formation_slots_emit_one_group_need() {
        // 4-back formation has four defender slots — if all are gaps,
        // detector must collapse to ONE FormationGap defender entry,
        // not four. This is the budget-distortion bug being pinned.
        let formation = t442_positions();
        let mut squad = Vec::new();
        squad.push(squad_player(1, PlayerPositionType::Goalkeeper, 130));
        squad.push(squad_player(2, PlayerPositionType::Goalkeeper, 125));
        // No defenders at all
        // At-tier mids / fwds
        for (i, pos) in [
            PlayerPositionType::MidfielderLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardRight,
        ]
        .iter()
        .enumerate()
        {
            squad.push(squad_player(20 + i as u32, *pos, 135));
        }

        let coverage = coverage_from_squad(&squad, formation);
        let needs = compute_group_needs(
            &squad,
            &coverage,
            formation,
            continental_score(),
            continental_tolerance(),
        );

        let defender_needs: Vec<&GroupNeed> = needs
            .iter()
            .filter(|n| n.group == PlayerFieldPositionGroup::Defender)
            .collect();
        assert_eq!(
            defender_needs.len(),
            1,
            "four empty defender slots must collapse to one need (got {})",
            defender_needs.len()
        );
        assert_eq!(defender_needs[0].kind, NeedKind::FormationGap);
    }

    #[test]
    fn fully_at_tier_squad_yields_no_needs() {
        // A balanced squad at-tier in every group: no FormationGap,
        // no QualityUpgrade, no DepthCover. Universal calibration
        // sanity — over-firing here would create phantom requests.
        let formation = t442_positions();
        let mut squad = Vec::new();
        squad.push(squad_player(1, PlayerPositionType::Goalkeeper, 130));
        squad.push(squad_player(2, PlayerPositionType::Goalkeeper, 125));
        let outfield = [
            PlayerPositionType::DefenderLeft,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::DefenderRight,
            PlayerPositionType::MidfielderLeft,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::MidfielderRight,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardRight,
        ];
        for (i, pos) in outfield.iter().enumerate() {
            squad.push(squad_player(10 + i as u32, *pos, 138));
        }
        // Bench depth so depth-cover doesn't fire
        squad.push(squad_player(40, PlayerPositionType::DefenderCenterLeft, 130));
        squad.push(squad_player(41, PlayerPositionType::DefenderCenterRight, 130));
        squad.push(squad_player(42, PlayerPositionType::MidfielderCenterLeft, 130));
        squad.push(squad_player(43, PlayerPositionType::MidfielderCenterRight, 130));
        squad.push(squad_player(44, PlayerPositionType::ForwardLeft, 128));

        let coverage = coverage_from_squad(&squad, formation);
        let needs = compute_group_needs(
            &squad,
            &coverage,
            formation,
            continental_score(),
            continental_tolerance(),
        );

        assert!(
            needs.is_empty(),
            "balanced at-tier squad should not generate any need (got {:?})",
            needs
        );
    }

    #[test]
    fn sweep_realistic_continental_acceptance_and_realism_gates() {
        use crate::transfers::pipeline::recommendations::{
            evaluate_listed_target, BuyerContext, ListedRejectReason, ListedTargetVerdict,
            ListedTargetView,
        };
        use crate::PlayerFieldPositionGroup;

        // Continental club — Spartak-like context.
        let buyer = |open_request: bool, weak_group: bool| BuyerContext {
            buyer_rep_score: 0.72,
            buyer_world_rep: 5800,
            buyer_league_reputation: 5500,
            buyer_total_wages: 30_000_000,
            buyer_wage_budget: 60_000_000,
            plan_total_budget: 30_000_000.0,
            max_recommend_value: 60_000_000.0,
            // Weak group: starter at 105 (under tier baseline).
            // Otherwise: starter at 130 (tier baseline).
            buyer_best_in_group: if weak_group { 105 } else { 130 },
            has_open_request: open_request,
            has_aging_starter: false,
        };

        // Mikhailov-class candidate: 14M, CA 130, listed, age 25.
        let mikhailov_class = ListedTargetView {
            ability: 130,
            estimated_potential: 138,
            age: 25,
            estimated_value: 14_000_000.0,
            position_group: PlayerFieldPositionGroup::Forward,
            is_listed: false,
            is_transfer_requested: true,
            is_unhappy: true,
            world_reputation: 5200,
            current_reputation: 5000,
            ambition: 0.7,
            parent_club_score: 0.40, // smaller club
            parent_club_in_debt: false,
        };

        // Acceptance: weak group + an actual upgrade
        let v = evaluate_listed_target(&mikhailov_class, &buyer(false, true));
        match v {
            ListedTargetVerdict::Accept(score) => {
                assert!(score > 10.0, "expected meaningful score, got {}", score);
            }
            ListedTargetVerdict::Reject(r) => panic!("expected Accept, got Reject({:?})", r),
        }

        // Open request also unlocks the path even when group is at-tier
        let v2 = evaluate_listed_target(&mikhailov_class, &buyer(true, false));
        assert!(matches!(v2, ListedTargetVerdict::Accept(_)));

        // No need + only a marginal upgrade → NotAnUpgrade reject
        let mut marginal = mikhailov_class;
        marginal.ability = 132;
        let buyer_no_need = buyer(false, false); // best=130
        let v3 = evaluate_listed_target(&marginal, &buyer_no_need);
        assert_eq!(v3, ListedTargetVerdict::Reject(ListedRejectReason::NotAnUpgrade));
    }

    #[test]
    fn sweep_rejects_unaffordable_fee() {
        use crate::transfers::pipeline::recommendations::{
            evaluate_listed_target, BuyerContext, ListedRejectReason, ListedTargetVerdict,
            ListedTargetView,
        };
        use crate::PlayerFieldPositionGroup;

        let small_buyer = BuyerContext {
            buyer_rep_score: 0.40,
            buyer_world_rep: 2400,
            buyer_league_reputation: 3000,
            buyer_total_wages: 1_000_000,
            buyer_wage_budget: 1_500_000,
            plan_total_budget: 500_000.0,
            max_recommend_value: 1_000_000.0,
            buyer_best_in_group: 75,
            has_open_request: true,
            has_aging_starter: false,
        };

        // Asking 5M when budget allows ~700k → UnaffordableFee
        let pricey = ListedTargetView {
            ability: 95,
            estimated_potential: 100,
            age: 26,
            estimated_value: 5_000_000.0,
            position_group: PlayerFieldPositionGroup::Midfielder,
            is_listed: true,
            is_transfer_requested: false,
            is_unhappy: false,
            world_reputation: 2500,
            current_reputation: 1500,
            ambition: 0.5,
            parent_club_score: 0.55,
            parent_club_in_debt: false,
        };
        assert_eq!(
            evaluate_listed_target(&pricey, &small_buyer),
            ListedTargetVerdict::Reject(ListedRejectReason::UnaffordableFee)
        );
    }

    #[test]
    fn sweep_rejects_unaffordable_wage_when_headroom_is_exhausted() {
        use crate::transfers::pipeline::recommendations::{
            evaluate_listed_target, BuyerContext, ListedRejectReason, ListedTargetVerdict,
            ListedTargetView,
        };
        use crate::PlayerFieldPositionGroup;

        // Wage budget barely above current spend → almost no headroom.
        // Even an at-tier player at this club would exceed the wage cap.
        let cap_strapped = BuyerContext {
            buyer_rep_score: 0.40,
            buyer_world_rep: 2400,
            buyer_league_reputation: 3000,
            buyer_total_wages: 1_000_000,
            buyer_wage_budget: 1_010_000, // 10k headroom × 1.3 = 13k cap
            plan_total_budget: 5_000_000.0,
            max_recommend_value: 10_000_000.0,
            buyer_best_in_group: 75,
            has_open_request: true,
            has_aging_starter: false,
        };

        let in_tier_listed = ListedTargetView {
            ability: 90,
            estimated_potential: 95,
            age: 27,
            estimated_value: 200_000.0, // fee comfortably affordable
            position_group: PlayerFieldPositionGroup::Midfielder,
            is_listed: true,
            is_transfer_requested: false,
            is_unhappy: false,
            world_reputation: 2200,
            current_reputation: 800,
            ambition: 0.5,
            parent_club_score: 0.55,
            parent_club_in_debt: false,
        };

        assert_eq!(
            evaluate_listed_target(&in_tier_listed, &cap_strapped),
            ListedTargetVerdict::Reject(ListedRejectReason::UnaffordableWage)
        );
    }

    #[test]
    fn sweep_rejects_world_class_target_for_local_club() {
        use crate::transfers::pipeline::recommendations::{
            evaluate_listed_target, BuyerContext, ListedRejectReason, ListedTargetVerdict,
            ListedTargetView,
        };
        use crate::PlayerFieldPositionGroup;

        let local_buyer = BuyerContext {
            buyer_rep_score: 0.20,
            buyer_world_rep: 1500,
            buyer_league_reputation: 2000,
            buyer_total_wages: 200_000,
            buyer_wage_budget: 600_000,
            plan_total_budget: 300_000.0,
            max_recommend_value: 600_000.0,
            buyer_best_in_group: 60,
            has_open_request: true, // even with explicit demand, world-class is out of reach
            has_aging_starter: false,
        };

        let world_class = ListedTargetView {
            ability: 175,
            estimated_potential: 180,
            age: 28,
            estimated_value: 200_000.0, // dirt-cheap to bypass fee gate
            position_group: PlayerFieldPositionGroup::Forward,
            is_listed: true,
            is_transfer_requested: false,
            is_unhappy: false,
            world_reputation: 9500,
            current_reputation: 9000,
            ambition: 0.7,
            parent_club_score: 0.85,
            parent_club_in_debt: false,
        };

        let v = evaluate_listed_target(&world_class, &local_buyer);
        // Tier window or reputation gap blocks well before scoring.
        match v {
            ListedTargetVerdict::Reject(
                ListedRejectReason::OutOfTierWindow
                | ListedRejectReason::ReputationGapTooLarge,
            ) => {}
            other => panic!("expected window / rep-gap reject, got {:?}", other),
        }
    }

    #[test]
    fn sweep_rejects_when_no_need_and_no_request() {
        use crate::transfers::pipeline::recommendations::{
            evaluate_listed_target, BuyerContext, ListedRejectReason, ListedTargetVerdict,
            ListedTargetView,
        };
        use crate::PlayerFieldPositionGroup;

        // Continental club, perfectly fine in this group, no aging
        // starter, no open request — sweep must NOT add filler.
        let buyer = BuyerContext {
            buyer_rep_score: 0.72,
            buyer_world_rep: 5500,
            buyer_league_reputation: 5500,
            buyer_total_wages: 20_000_000,
            buyer_wage_budget: 50_000_000,
            plan_total_budget: 25_000_000.0,
            max_recommend_value: 50_000_000.0,
            buyer_best_in_group: 135, // above tier baseline
            has_open_request: false,
            has_aging_starter: false,
        };

        let modest_listed = ListedTargetView {
            ability: 128,
            estimated_potential: 130,
            age: 26,
            estimated_value: 8_000_000.0,
            position_group: PlayerFieldPositionGroup::Midfielder,
            is_listed: true,
            is_transfer_requested: false,
            is_unhappy: false,
            world_reputation: 4500,
            current_reputation: 4000,
            ambition: 0.5,
            parent_club_score: 0.55,
            parent_club_in_debt: false,
        };

        let v = evaluate_listed_target(&modest_listed, &buyer);
        assert_eq!(
            v,
            ListedTargetVerdict::Reject(ListedRejectReason::NoSquadNeed),
            "club with no need must not add filler — got {:?}",
            v
        );
    }

    #[test]
    fn sweep_rejects_player_without_listing_status() {
        use crate::transfers::pipeline::recommendations::{
            evaluate_listed_target, BuyerContext, ListedRejectReason, ListedTargetVerdict,
            ListedTargetView,
        };
        use crate::PlayerFieldPositionGroup;

        let buyer = BuyerContext {
            buyer_rep_score: 0.72,
            buyer_world_rep: 5500,
            buyer_league_reputation: 5500,
            buyer_total_wages: 20_000_000,
            buyer_wage_budget: 50_000_000,
            plan_total_budget: 25_000_000.0,
            max_recommend_value: 50_000_000.0,
            buyer_best_in_group: 105,
            has_open_request: true,
            has_aging_starter: false,
        };

        let happy_player = ListedTargetView {
            ability: 130,
            estimated_potential: 135,
            age: 25,
            estimated_value: 8_000_000.0,
            position_group: PlayerFieldPositionGroup::Forward,
            is_listed: false,
            is_transfer_requested: false,
            is_unhappy: false,
            world_reputation: 5000,
            current_reputation: 4500,
            ambition: 0.5,
            parent_club_score: 0.50,
            parent_club_in_debt: false,
        };

        // The sweep is the listed-star path — players without any
        // public listing flag aren't routed through it.
        assert_eq!(
            evaluate_listed_target(&happy_player, &buyer),
            ListedTargetVerdict::Reject(ListedRejectReason::NotListed)
        );
    }

    #[test]
    fn depth_requirement_scales_with_formation_footprint() {
        // The bench-depth helper is pure and used by the detector.
        // Pin its calibration so behaviour stays stable.
        let t442 = t442_positions();
        assert_eq!(
            group_depth_requirement(t442, PlayerFieldPositionGroup::Goalkeeper),
            2,
            "GK depth is fixed at 2 regardless of formation"
        );
        // 4-4-2 has 4 defenders → 4+2 = 6
        assert_eq!(
            group_depth_requirement(t442, PlayerFieldPositionGroup::Defender),
            6
        );
        // 4-4-2 has 4 mids → 4+1 = 5
        assert_eq!(
            group_depth_requirement(t442, PlayerFieldPositionGroup::Midfielder),
            5
        );
        // 4-4-2 has 2 forwards → 2+1 = 3
        assert_eq!(
            group_depth_requirement(t442, PlayerFieldPositionGroup::Forward),
            3
        );
    }
}

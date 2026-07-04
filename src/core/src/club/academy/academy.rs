use crate::club::academy::result::ClubAcademyResult;
use crate::club::academy::settings::AcademySettings;
use crate::club::academy::tuning::{AcademyTier, AcademyTuning};
use crate::context::GlobalContext;
use crate::{
    Person, Player, PlayerCollection, PlayerFieldPositionGroup, PlayerPositionType, StaffCollection,
};
use chrono::{Datelike, NaiveDate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcademyDevelopmentIdentity {
    Balanced,
    TechnicalSchool,
    TacticalSchool,
    AthleticDevelopment,
    PlayerTrading,
}

/// Coarse age-bucket label exposed for the UI / tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcademyPlayerPhase {
    Foundation,
    Development,
    Professional,
}

impl AcademyPlayerPhase {
    pub fn from_age(age: u8) -> Self {
        match age {
            0..=11 => AcademyPlayerPhase::Foundation,
            12..=14 => AcademyPlayerPhase::Development,
            _ => AcademyPlayerPhase::Professional,
        }
    }

    /// 0/1/2 — matches `sessions_for_phase_and_tier`'s phase index.
    pub fn index(self) -> u8 {
        match self {
            AcademyPlayerPhase::Foundation => 0,
            AcademyPlayerPhase::Development => 1,
            AcademyPlayerPhase::Professional => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AcademyPlayerPhase::Foundation => "Foundation",
            AcademyPlayerPhase::Development => "Development",
            AcademyPlayerPhase::Professional => "Professional",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AcademyPathwayPolicy {
    pub min_graduation_age: u8,
    /// 0..100 readiness threshold — see `pathway_readiness_score` for the
    /// score definition. Tiers above 7 push the bar higher because the
    /// resident U18 is already strong; tiers 1-3 graduate sooner so the
    /// pathway doesn't stall.
    pub readiness_threshold: i16,
    pub protect_late_developers: bool,
    pub max_group_imbalance: usize,
    /// 1..10 pathway tier. Drives the *age-relative* CA expectation in
    /// the readiness scorer: a strong academy generates higher-CA youth,
    /// so its "ready for youth football" CA bar sits higher than a small
    /// academy's. Paired with `readiness_threshold` it keeps the bar both
    /// reachable for each tier's realistic output and harder at the top.
    pub tier: u8,
}

impl AcademyPathwayPolicy {
    /// `level` is the academy facility rating (1..20, matches
    /// `FacilityLevel::to_rating`). Internally we collapse to a 1..10
    /// pathway tier so the policy thresholds stay readable.
    pub fn for_level(level: u8) -> Self {
        let tier = AcademyTier::from_level(level);
        AcademyPathwayPolicy {
            min_graduation_age: if tier.value() >= 8 { 14 } else { 15 },
            readiness_threshold: tier.readiness_threshold(),
            protect_late_developers: tier.value() >= 4,
            max_group_imbalance: if tier.value() >= 8 { 2 } else { 3 },
            tier: tier.value(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AcademyPipelineHealth {
    pub foundation_players: usize,
    pub development_players: usize,
    pub professional_players: usize,
    pub ready_for_youth: usize,
    pub elite_prospects: usize,
    pub at_risk_players: usize,
    pub group_counts: [usize; 4],
    pub total_players: usize,
    pub years_since_last_graduate: u16,
}

#[derive(Debug, Clone)]
pub struct ClubAcademy {
    pub(super) settings: AcademySettings,
    pub(super) tuning: AcademyTuning,
    pub players: PlayerCollection,
    pub staff: StaffCollection,
    pub(super) level: u8,
    pub(super) last_production_year: Option<i32>,
    /// Total players graduated to youth teams over the academy's history.
    pub graduates_produced: u16,
    pub(super) last_graduation_year: Option<i32>,
    /// Football identity the academy is trying to produce. It affects
    /// training emphasis and long-term recruitment balance, not match tactics.
    pub development_identity: AcademyDevelopmentIdentity,
    /// Rules for deciding when an academy player is ready for the U18/U19
    /// pathway rather than being held back inside the academy pool.
    pub pathway_policy: AcademyPathwayPolicy,
    /// Internal-precision pathway reputation. Stored as f32 so the
    /// monthly delta can move fractionally; exposed as u8 0..100 via
    /// `pathway_reputation`.
    pub(super) pathway_reputation_f: f32,
    /// 0..100 internal reputation for the pathway. Strong intakes, balanced
    /// age groups, and graduates lift it; bloated/blocked pathways reduce it.
    pub pathway_reputation: u8,
    /// Position groups under-supplied in the academy. Annual intakes use this
    /// as a recruitment brief before falling back to generic position odds.
    pub recruitment_priorities: Vec<PlayerFieldPositionGroup>,
    last_pathway_review: Option<NaiveDate>,
}

impl ClubAcademy {
    pub fn new(level: u8) -> Self {
        let starting_rep = (35.0 + AcademyTier::from_level(level).value() as f32 * 5.0).min(90.0);
        ClubAcademy {
            settings: AcademySettings::default(),
            tuning: AcademyTuning::default(),
            players: PlayerCollection::new(Vec::new()),
            staff: StaffCollection::new(Vec::new()),
            level,
            last_production_year: None,
            graduates_produced: 0,
            last_graduation_year: None,
            development_identity: AcademyDevelopmentIdentity::Balanced,
            pathway_policy: AcademyPathwayPolicy::for_level(level),
            pathway_reputation_f: starting_rep,
            pathway_reputation: starting_rep as u8,
            recruitment_priorities: Vec::new(),
            last_pathway_review: None,
        }
    }

    pub fn tuning(&self) -> &AcademyTuning {
        &self.tuning
    }

    /// 1..10 pathway tier — short scale used by every academy formula.
    pub fn tier(&self) -> AcademyTier {
        AcademyTier::from_level(self.level)
    }

    /// Raw academy level (1..20 facility rating).
    pub fn level(&self) -> u8 {
        self.level
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> ClubAcademyResult {
        // Academy players go through the same per-player lifecycle as
        // everyone else (injury recovery, condition, mailbox, …) but
        // **skip natural skill development**. That signal is owned by
        // the academy-specific weekly training tick — running both in
        // the same week would double-develop every prospect.
        let players_result = self
            .players
            .simulate_skip_development(ctx.with_player(None));

        self.run_pathway_review(&ctx);

        // Weekly academy training: the core development driver.
        self.train_academy_players(&ctx);

        let produce_result = self.produce_youth_players(ctx.clone());

        for player in produce_result.players {
            self.players.add(player);
        }

        // Ensure academy always has minimum players from settings.
        self.ensure_minimum_players(ctx);

        ClubAcademyResult::new(players_result)
    }

    fn run_pathway_review(&mut self, ctx: &GlobalContext<'_>) {
        if !ctx.simulation.is_month_beginning() {
            return;
        }

        let date = ctx.simulation.date.date();
        if self
            .last_pathway_review
            .map(|d| d.year() == date.year() && d.month() == date.month())
            .unwrap_or(false)
        {
            return;
        }

        let health = self.pipeline_health(date);
        self.recruitment_priorities = self.identify_recruitment_priorities(&health);
        self.apply_pathway_reputation_delta(&health, date);
        self.calibrate_player_count_range(&health);
        self.apply_player_welfare_controls(date);
        self.last_pathway_review = Some(date);
    }

    pub(super) fn pipeline_health(&self, date: NaiveDate) -> AcademyPipelineHealth {
        let mut health = AcademyPipelineHealth::default();
        health.total_players = self.players.players.len();

        let elite_pa = self.tuning.elite_pa_threshold;
        for player in &self.players.players {
            let age = player.age(date);
            match AcademyPlayerPhase::from_age(age) {
                AcademyPlayerPhase::Foundation => health.foundation_players += 1,
                AcademyPlayerPhase::Development => health.development_players += 1,
                AcademyPlayerPhase::Professional => health.professional_players += 1,
            }

            let group = player.position().position_group();
            health.group_counts[group_index(group)] += 1;

            // "Ready for youth" is *eligibility for the youth pathway*, not a
            // quality bar: an old-enough, healthy, non-exhausted prospect
            // counts even at low CA (see `is_graduation_eligible`). Readiness
            // only ranks who graduates first when capacity is limited.
            if self.is_graduation_eligible(player, date) {
                health.ready_for_youth += 1;
            }
            let readiness = self.pathway_readiness_score(player, date);
            if player.player_attributes.potential_ability >= elite_pa && readiness >= 60 {
                health.elite_prospects += 1;
            }
            if player.player_attributes.jadedness > 5500
                || player.player_attributes.condition < 5500
                || player.player_attributes.injury_proneness >= 17
            {
                health.at_risk_players += 1;
            }
        }

        health.years_since_last_graduate = self
            .last_graduation_year
            .map(|y| (date.year() - y).max(0) as u16)
            .unwrap_or(0);

        health
    }

    fn identify_recruitment_priorities(
        &self,
        health: &AcademyPipelineHealth,
    ) -> Vec<PlayerFieldPositionGroup> {
        let total = self
            .players
            .players
            .len()
            .max(health.group_counts.iter().sum::<usize>())
            .max(1);
        let targets = [
            (total as f32 * 0.10).ceil() as usize,
            (total as f32 * 0.30).ceil() as usize,
            (total as f32 * 0.38).ceil() as usize,
            (total as f32 * 0.22).ceil() as usize,
        ];
        let groups = [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ];

        let mut gaps: Vec<(PlayerFieldPositionGroup, usize)> = groups
            .into_iter()
            .enumerate()
            .filter_map(|(idx, group)| {
                let deficit = targets[idx].saturating_sub(health.group_counts[idx]);
                if deficit >= self.pathway_policy.max_group_imbalance.min(2) {
                    let urgency = deficit * recruitment_group_urgency(group);
                    Some((group, urgency))
                } else {
                    None
                }
            })
            .collect();

        gaps.sort_unstable_by(|a, b| b.1.cmp(&a.1));
        gaps.into_iter().map(|(group, _)| group).collect()
    }

    /// Returns `true` if `health.group_counts` is within
    /// `max_group_imbalance` of the per-group target proportions.
    pub(super) fn age_groups_balanced(&self, health: &AcademyPipelineHealth) -> bool {
        let total = health.total_players.max(1) as f32;
        // Foundation (8-11): 0-20%, Dev (12-14): 35-50%, Pro (15+): 30-50%.
        // Real academies skew toward Dev/Pro but Foundation isn't zero.
        let foundation_pct = health.foundation_players as f32 / total;
        let dev_pct = health.development_players as f32 / total;
        let pro_pct = health.professional_players as f32 / total;
        foundation_pct <= 0.25
            && dev_pct >= 0.25
            && dev_pct <= 0.55
            && pro_pct >= 0.25
            && pro_pct <= 0.55
    }

    pub(super) fn positional_balance_ok(&self, health: &AcademyPipelineHealth) -> bool {
        let total = health.total_players.max(1) as f32;
        let gk_pct = health.group_counts[0] as f32 / total;
        let def_pct = health.group_counts[1] as f32 / total;
        let mid_pct = health.group_counts[2] as f32 / total;
        let fwd_pct = health.group_counts[3] as f32 / total;
        gk_pct >= 0.05
            && gk_pct <= 0.18
            && def_pct >= 0.22
            && def_pct <= 0.40
            && mid_pct >= 0.30
            && mid_pct <= 0.48
            && fwd_pct >= 0.15
            && fwd_pct <= 0.32
    }

    /// Apply the monthly pathway-reputation delta. The score moves slowly
    /// (≤ ±2 per month) so a single bad / good month can't flip the
    /// pathway's standing.
    fn apply_pathway_reputation_delta(&mut self, health: &AcademyPipelineHealth, _date: NaiveDate) {
        let mut delta: f32 = 0.0;

        if health.ready_for_youth >= 4 {
            delta += 0.8;
        }
        if health.elite_prospects >= 1 {
            delta += 1.0;
        }
        if self.age_groups_balanced(health) {
            delta += 0.4;
        }
        if self.positional_balance_ok(health) {
            delta += 0.4;
        }

        let total = health.total_players.max(1) as f32;
        let at_risk_pct = health.at_risk_players as f32 / total;
        if at_risk_pct >= 0.20 {
            delta -= 0.8;
        }

        // "Professional-phase overcrowded" = pro_count is both an outright
        // majority and crowds out younger prospects. Real academies that
        // hoard 17-year-olds and never refresh the foundation stall.
        if total > 0.0 && (health.professional_players as f32 / total) > 0.55 {
            delta -= 0.6;
        }

        if health.years_since_last_graduate >= 2 {
            delta -= 0.5;
        }

        if health.total_players > self.tuning.max_academy_players {
            delta -= 0.4;
        }

        let delta = delta.clamp(-2.0, 2.0);
        self.pathway_reputation_f = (self.pathway_reputation_f + delta).clamp(0.0, 100.0);
        self.pathway_reputation = self.pathway_reputation_f.round() as u8;
    }

    fn calibrate_player_count_range(&mut self, _health: &AcademyPipelineHealth) {
        let (min, max) = self.tuning.target_size(AcademyTier::from_level(self.level));
        self.settings.players_count_range = min..max;
    }

    fn apply_player_welfare_controls(&mut self, date: NaiveDate) {
        let policy = self.pathway_policy.clone();
        let pathway_rep = self.pathway_reputation;

        for player in &mut self.players.players {
            if player.player_attributes.is_injured {
                continue;
            }

            if player.player_attributes.condition < 6500 {
                player.player_attributes.rest(700);
                player.player_attributes.jadedness =
                    player.player_attributes.jadedness.saturating_sub(250);
            }

            if player.age(date) <= 12 {
                player.player_attributes.jadedness =
                    player.player_attributes.jadedness.saturating_sub(200);
            }

            let readiness = AcademyReadinessScorer::new(pathway_rep, &policy).score(player, date);
            if readiness >= policy.readiness_threshold {
                player.player_attributes.update_reputation(1, 2, 0);
            }
        }
    }

    /// 0..100 readiness score combining current ability, runway, personality,
    /// age, fitness, pathway prestige, and risk penalties. Wraps
    /// [`AcademyReadinessScorer`] so callers don't reach into the free
    /// helper directly.
    pub fn pathway_readiness_score(&self, player: &Player, date: NaiveDate) -> i16 {
        AcademyReadinessScorer::new(self.pathway_reputation, &self.pathway_policy)
            .score(player, date)
    }

    /// Whether an academy player is *eligible to enter the youth-team
    /// pathway* this season. This is a welfare/age gate, deliberately
    /// independent of ability: old enough, under the academy age-out, fit,
    /// and not exhausted. Quality (CA/PA/personality) only *ranks*
    /// eligible players via [`pathway_readiness_score`] — it never blocks
    /// graduation. A low-CA but fit 16-year-old is a valid graduate.
    pub fn is_graduation_eligible(&self, player: &Player, date: NaiveDate) -> bool {
        let age = player.age(date);
        age >= self.pathway_policy.min_graduation_age
            && age < 18
            && !player.player_attributes.is_injured
            && player.player_attributes.condition >= 5000
            && player.player_attributes.jadedness <= 7500
    }

    /// Academy players eligible for youth-team graduation this season,
    /// each paired with their readiness score for ranking and UI display.
    /// The count of this list is the headline "ready for youth" figure —
    /// it reflects pathway eligibility, not a high-CA quality threshold.
    pub fn graduation_candidates(&self, date: NaiveDate) -> Vec<(u32, i16)> {
        self.players
            .players
            .iter()
            .filter(|p| self.is_graduation_eligible(p, date))
            .map(|p| (p.id, self.pathway_readiness_score(p, date)))
            .collect()
    }

    pub(super) fn recruitment_priority_position(
        &self,
        intake_index: usize,
    ) -> Option<PlayerPositionType> {
        let group = *self.recruitment_priorities.get(intake_index)?;
        Some(position_for_priority_group(group, intake_index))
    }
}

/// 0..100 readiness scorer. Owns every weight, axis, and penalty so
/// the scoring formula has one logical home rather than a constellation
/// of free helpers. Pathway *reputation* drives the prestige axis — not
/// the raw facility tier — because graduating into a well-respected U18
/// pathway is what actually translates an academy prospect to a senior
/// career.
pub struct AcademyReadinessScorer<'a> {
    pathway_reputation: u8,
    policy: &'a AcademyPathwayPolicy,
}

impl<'a> AcademyReadinessScorer<'a> {
    /// Build a scorer from the academy's pathway reputation (0..100).
    pub fn new(pathway_reputation: u8, policy: &'a AcademyPathwayPolicy) -> Self {
        AcademyReadinessScorer {
            pathway_reputation: pathway_reputation.min(100),
            policy,
        }
    }

    /// 0..100 readiness score. Below the policy minimum-age this is a
    /// hard zero; otherwise it is a *ranking* signal, not a pass/fail gate.
    ///
    /// Readiness means "ready to enter the youth-team pathway", which in
    /// real football a fit, old-enough teenager is — regardless of how
    /// high their current ability is. So the axes are dominated by age,
    /// welfare and attitude, with CA folded in only as an age-relative
    /// rank tiebreak (not an absolute quality bar). The split is:
    ///
    ///   age / proximity to graduation  25
    ///   fitness / welfare              20
    ///   personality / attitude         15
    ///   PA runway                      15
    ///   CA relative to age             15
    ///   pathway reputation             10
    ///   − injury/condition/jadedness penalties
    pub fn score(&self, player: &Player, date: NaiveDate) -> i16 {
        let age = player.age(date);
        if age < self.policy.min_graduation_age {
            return 0;
        }

        let ca = player.player_attributes.current_ability as f32;
        let pa = player.player_attributes.potential_ability as f32;

        // Age / time-in-pathway (25). The dominant axis: an older prospect
        // has had more development and is closer to youth-team football. A
        // 17-year-old maxes it; a freshly-eligible 15-year-old still earns
        // a solid base so "fit and old enough" carries real weight.
        let age_score = match age {
            a if a >= 17 => 1.00,
            16 => 0.80,
            15 => 0.62,
            14 => 0.46,
            _ => 0.46,
        } * 25.0;

        // Fitness / welfare (20). Condition dominates; jadedness trims it.
        let condition = (player.player_attributes.condition as f32 / 10000.0).clamp(0.0, 1.0);
        let jaded = (player.player_attributes.jadedness as f32 / 10000.0).clamp(0.0, 1.0);
        let fitness_score = (0.65 * condition + 0.35 * (1.0 - jaded)) * 20.0;

        // Personality / training attitude (15). Professionalism dominates
        // because it best predicts academy → senior translation.
        let personality_raw = 0.40 * player.attributes.professionalism
            + 0.25 * player.skills.mental.determination
            + 0.20 * player.skills.mental.work_rate
            + 0.15 * player.attributes.ambition;
        let personality_score = (personality_raw / 20.0 * 15.0).clamp(0.0, 15.0);

        // PA runway (15). Headroom between current and potential ability —
        // separates the high-ceiling prospects from the finished article.
        let pa_runway_score = (((pa - ca).max(0.0)) / 70.0 * 15.0).clamp(0.0, 15.0);

        // CA relative to age (15). NOT an absolute quality bar: scored
        // against the CA a strong-for-their-age prospect of this tier
        // actually reaches, so a low-CA-but-on-track teenager still ranks
        // respectably while a genuinely advanced one edges ahead. Academy
        // youth top out far below the ~140 senior scale.
        let expected_ca = {
            let base = match age {
                0..=14 => 58.0,
                15 => 66.0,
                16 => 74.0,
                17 => 82.0,
                _ => 88.0,
            };
            (base + (self.policy.tier as f32 - 4.0) * 2.0).max(45.0)
        };
        let ca_rel_score = (ca / expected_ca * 15.0).clamp(0.0, 15.0);

        // Pathway prestige (10). Driven by *reputation*, not facility tier:
        // a well-respected pathway translates prospects to senior careers.
        let pathway_score = self.pathway_reputation as f32 / 100.0 * 10.0;

        // Penalties keep at-risk prospects (injury-prone, under-conditioned,
        // exhausted) lower in the ranking even when age-eligible.
        let mut penalty = 0.0_f32;
        if player.player_attributes.injury_proneness >= 17 {
            penalty += 6.0;
        }
        if player.player_attributes.condition < 6000 {
            penalty += 6.0;
        }
        if player.player_attributes.jadedness > 6000 {
            penalty += 6.0;
        }
        if age <= 14 && self.policy.protect_late_developers && (pa - ca) < 35.0 {
            penalty += 3.0;
        }

        let score = age_score
            + fitness_score
            + personality_score
            + pa_runway_score
            + ca_rel_score
            + pathway_score
            - penalty;
        score.clamp(0.0, 100.0) as i16
    }
}

fn position_for_priority_group(
    group: PlayerFieldPositionGroup,
    intake_index: usize,
) -> PlayerPositionType {
    match group {
        PlayerFieldPositionGroup::Goalkeeper => PlayerPositionType::Goalkeeper,
        PlayerFieldPositionGroup::Defender => match intake_index % 4 {
            0 => PlayerPositionType::DefenderCenter,
            1 => PlayerPositionType::DefenderLeft,
            2 => PlayerPositionType::DefenderRight,
            _ => PlayerPositionType::DefensiveMidfielder,
        },
        PlayerFieldPositionGroup::Midfielder => match intake_index % 5 {
            0 => PlayerPositionType::MidfielderCenter,
            1 => PlayerPositionType::MidfielderLeft,
            2 => PlayerPositionType::MidfielderRight,
            3 => PlayerPositionType::AttackingMidfielderCenter,
            _ => PlayerPositionType::DefensiveMidfielder,
        },
        PlayerFieldPositionGroup::Forward => match intake_index % 4 {
            0 => PlayerPositionType::Striker,
            1 => PlayerPositionType::ForwardLeft,
            2 => PlayerPositionType::ForwardRight,
            _ => PlayerPositionType::ForwardCenter,
        },
    }
}

fn group_index(group: PlayerFieldPositionGroup) -> usize {
    match group {
        PlayerFieldPositionGroup::Goalkeeper => 0,
        PlayerFieldPositionGroup::Defender => 1,
        PlayerFieldPositionGroup::Midfielder => 2,
        PlayerFieldPositionGroup::Forward => 3,
    }
}

fn recruitment_group_urgency(group: PlayerFieldPositionGroup) -> usize {
    match group {
        PlayerFieldPositionGroup::Goalkeeper => 3,
        PlayerFieldPositionGroup::Defender => 1,
        PlayerFieldPositionGroup::Midfielder => 1,
        PlayerFieldPositionGroup::Forward => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pathway_policy_scales_with_academy_level() {
        let modest = AcademyPathwayPolicy::for_level(4);
        let elite = AcademyPathwayPolicy::for_level(18);

        assert!(elite.protect_late_developers);
        assert!(elite.readiness_threshold > modest.readiness_threshold);
        assert!(elite.min_graduation_age <= modest.min_graduation_age);
    }

    #[test]
    fn recruitment_priorities_target_missing_position_groups() {
        let academy = ClubAcademy::new(8);
        let health = AcademyPipelineHealth {
            group_counts: [0, 3, 14, 8],
            total_players: 25,
            ..AcademyPipelineHealth::default()
        };

        let priorities = academy.identify_recruitment_priorities(&health);

        assert_eq!(
            priorities.first(),
            Some(&PlayerFieldPositionGroup::Goalkeeper)
        );
        assert!(priorities.contains(&PlayerFieldPositionGroup::Defender));
    }

    #[test]
    fn priority_position_maps_group_to_real_position() {
        assert_eq!(
            position_for_priority_group(PlayerFieldPositionGroup::Goalkeeper, 0),
            PlayerPositionType::Goalkeeper
        );
        assert!(position_for_priority_group(PlayerFieldPositionGroup::Forward, 1).is_forward());
    }

    #[test]
    fn academy_tier_maps_facility_levels() {
        assert_eq!(AcademyTier::from_level(1).value(), 1);
        assert_eq!(AcademyTier::from_level(2).value(), 1);
        assert_eq!(AcademyTier::from_level(3).value(), 2);
        assert_eq!(AcademyTier::from_level(20).value(), 10);
    }

    #[test]
    fn readiness_score_stays_in_0_to_100() {
        use chrono::NaiveDate;

        let policy = AcademyPathwayPolicy::for_level(8);
        // Realistic mid-level pathway reputation for the test.
        let scorer = AcademyReadinessScorer::new(60, &policy);

        // Score a synthetic player across the entire CA/PA × age cube;
        // the score must stay inside 0..=100 for every combination.
        let date = NaiveDate::from_ymd_opt(2025, 7, 15).unwrap();
        for ca in (40..=180).step_by(20) {
            for pa in (ca..=200).step_by(20) {
                for age in 14..=17 {
                    let player = synthetic_academy_player(age, ca as u8, pa as u8, date);
                    let s = scorer.score(&player, date);
                    assert!(
                        s >= 0 && s <= 100,
                        "readiness out of range: CA={}, PA={}, age={} → {}",
                        ca,
                        pa,
                        age,
                        s
                    );
                }
            }
        }
    }

    /// Build a minimal Player suitable for readiness scoring. Lives in
    /// the test module so we don't expose the constructor anywhere else.
    fn synthetic_academy_player(age: u8, ca: u8, pa: u8, today: NaiveDate) -> Player {
        use crate::PeopleNameGeneratorData;
        use crate::PlayerGenerator;
        let names = PeopleNameGeneratorData {
            first_names: vec!["Test".to_string()],
            last_names: vec!["Player".to_string()],
            nicknames: vec![],
        };
        let mut player =
            PlayerGenerator::generate(1, today, PlayerPositionType::MidfielderCenter, 10, &names);
        // Override CA/PA and birth_date to match the test fixture
        player.player_attributes.current_ability = ca;
        player.player_attributes.potential_ability = pa;
        player.birth_date = NaiveDate::from_ymd_opt(today.year() - age as i32, 6, 15).unwrap();
        player
    }

    /// Readiness-test prospect with deterministic personality and
    /// condition. `personality` (0..20) is applied to the four traits the
    /// scorer reads so the band assertions don't depend on the generator's
    /// random attitude rolls.
    fn academy_prospect(
        age: u8,
        ca: u8,
        pa: u8,
        personality: f32,
        condition: i16,
        today: NaiveDate,
    ) -> Player {
        let mut player = synthetic_academy_player(age, ca, pa, today);
        player.attributes.professionalism = personality;
        player.attributes.ambition = personality;
        player.skills.mental.determination = personality;
        player.skills.mental.work_rate = personality;
        player.player_attributes.condition = condition;
        player.player_attributes.jadedness = 0;
        player.player_attributes.injury_proneness = 5;
        player
    }

    #[test]
    fn readiness_under_min_age_is_zero() {
        let date = NaiveDate::from_ymd_opt(2025, 7, 15).unwrap();
        let policy = AcademyPathwayPolicy::for_level(8); // min_grad 15
        let scorer = AcademyReadinessScorer::new(55, &policy);
        assert!(14 < policy.min_graduation_age, "test premise");
        // Even an outstanding 14-year-old scores a hard zero on age alone —
        // the only absolute gate left in the readiness model.
        let prospect = academy_prospect(14, 95, 160, 18.0, 9000, date);
        assert_eq!(scorer.score(&prospect, date), 0);
    }

    #[test]
    fn readiness_ranks_elite_above_low_ca_but_both_are_substantial() {
        // Readiness is a *ranking* signal now, not a quality gate: a fit,
        // age-eligible low-CA teenager is genuinely ready (non-zero, solid
        // score), while a high-PA peer simply ranks above them.
        let date = NaiveDate::from_ymd_opt(2025, 7, 15).unwrap();
        let policy = AcademyPathwayPolicy::for_level(8);
        let scorer = AcademyReadinessScorer::new(55, &policy);
        let low_ca = academy_prospect(16, 50, 60, 10.0, 8500, date);
        let elite = academy_prospect(16, 80, 165, 16.0, 8500, date);
        let low = scorer.score(&low_ca, date);
        let top = scorer.score(&elite, date);
        assert!(low > 0, "low-CA fit teen must not score zero: {}", low);
        assert!(
            top > low,
            "elite prospect must rank above low-CA: {top} <= {low}"
        );
    }

    #[test]
    fn readiness_blocks_no_one_for_low_ca_only() {
        // The same fit 17-year-old at CA 50 and at CA 90 should both be
        // clearly "ready" prospects; CA shifts the rank, never the floor.
        let date = NaiveDate::from_ymd_opt(2025, 7, 15).unwrap();
        let policy = AcademyPathwayPolicy::for_level(8);
        let scorer = AcademyReadinessScorer::new(55, &policy);
        let weak_ca = scorer.score(&academy_prospect(17, 50, 55, 9.0, 8500, date), date);
        let strong_ca = scorer.score(&academy_prospect(17, 90, 140, 9.0, 8500, date), date);
        assert!(
            weak_ca >= 50,
            "fit, old-enough teen must rank well: {weak_ca}"
        );
        assert!(strong_ca > weak_ca);
    }

    #[test]
    fn pipeline_health_counts_low_ca_eligible_players_as_ready() {
        let date = NaiveDate::from_ymd_opt(2025, 7, 15).unwrap();
        let mut academy = ClubAcademy::new(8); // tier 4, min_grad 15
        // Low-CA but fit, age-eligible teenagers — all "ready for youth".
        academy
            .players
            .add(academy_prospect(15, 48, 60, 9.0, 8000, date));
        academy
            .players
            .add(academy_prospect(16, 55, 70, 8.0, 8200, date));
        academy
            .players
            .add(academy_prospect(17, 52, 58, 7.0, 8500, date));
        // NOT ready: underage (even with huge CA)...
        academy
            .players
            .add(academy_prospect(14, 95, 160, 16.0, 9000, date));
        // ...and an exhausted teenager over the jadedness eligibility cap.
        let mut exhausted = academy_prospect(16, 70, 110, 12.0, 9000, date);
        exhausted.player_attributes.jadedness = 8000; // > 7500 cap
        academy.players.add(exhausted);

        let health = academy.pipeline_health(date);
        assert_eq!(
            health.ready_for_youth, 3,
            "the three fit, age-eligible low-CA teens are ready; underage and exhausted are not"
        );
    }

    #[test]
    fn graduation_does_not_overfill_youth() {
        // Even with a full eligible pool, the recommended count never
        // pushes the youth roster past the soft-max of 30.
        let academy = ClubAcademy::new(10);
        // Youth already at the soft-max → no graduates regardless of pool.
        let cap = academy.graduation_ceiling(30, 10, 2);
        assert_eq!(cap, 0, "ceiling violated soft-max");
        // Youth at 24 with 10 eligible → only 6 slots of room are usable.
        assert_eq!(academy.recommended_graduates(24, 10), 6);
    }

    #[test]
    fn pathway_reputation_delta_is_capped_per_month() {
        // No matter how lopsided the health, monthly delta stays within
        // ±2. This is the guarantee that callers rely on for "the
        // pathway reputation moves slowly".
        let mut academy = ClubAcademy::new(10);
        academy.pathway_reputation_f = 50.0;
        let health = AcademyPipelineHealth {
            ready_for_youth: 12,
            elite_prospects: 6,
            at_risk_players: 0,
            total_players: 40,
            development_players: 14,
            professional_players: 14,
            foundation_players: 12,
            group_counts: [4, 11, 15, 10],
            years_since_last_graduate: 0,
        };
        academy
            .apply_pathway_reputation_delta(&health, NaiveDate::from_ymd_opt(2025, 7, 1).unwrap());
        let after_good = academy.pathway_reputation_f;
        assert!(after_good - 50.0 <= 2.0001);
        assert!(after_good - 50.0 >= 1.0); // at least *some* lift

        let mut academy = ClubAcademy::new(10);
        academy.pathway_reputation_f = 50.0;
        let bad_health = AcademyPipelineHealth {
            at_risk_players: 30,
            professional_players: 30,
            total_players: 40,
            years_since_last_graduate: 5,
            ..AcademyPipelineHealth::default()
        };
        academy.apply_pathway_reputation_delta(
            &bad_health,
            NaiveDate::from_ymd_opt(2025, 7, 1).unwrap(),
        );
        let after_bad = academy.pathway_reputation_f;
        assert!(50.0 - after_bad <= 2.0001);
        assert!(50.0 - after_bad >= 1.0);
    }
}

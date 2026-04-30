use crate::club::academy::result::ClubAcademyResult;
use crate::club::academy::settings::AcademySettings;
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

#[derive(Debug, Clone)]
pub struct AcademyPathwayPolicy {
    pub min_graduation_age: u8,
    pub readiness_threshold: i16,
    pub protect_late_developers: bool,
    pub max_group_imbalance: usize,
}

impl AcademyPathwayPolicy {
    /// `level` is the academy facility rating (1..20, matches
    /// `FacilityLevel::to_rating`). Internally we collapse to a 1..10
    /// pathway tier so the policy thresholds stay readable.
    pub fn for_level(level: u8) -> Self {
        let tier = academy_tier(level);
        AcademyPathwayPolicy {
            min_graduation_age: if tier >= 8 { 14 } else { 15 },
            readiness_threshold: 70 + (tier as i16 * 3),
            protect_late_developers: tier >= 6,
            max_group_imbalance: if tier >= 8 { 2 } else { 3 },
        }
    }
}

/// Collapse the 1..20 facility-rating scale into a 1..10 pathway tier.
/// Used everywhere pathway logic wants "how strong is this academy" on a
/// short scale; keeps the storage scale (1..20) consistent.
pub(crate) fn academy_tier(level: u8) -> u8 {
    let lvl = level.clamp(1, 20) as u16;
    (((lvl + 1) / 2) as u8).clamp(1, 10)
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
}

#[derive(Debug, Clone)]
pub struct ClubAcademy {
    pub(super) settings: AcademySettings,
    pub players: PlayerCollection,
    pub staff: StaffCollection,
    pub(super) level: u8,
    pub(super) last_production_year: Option<i32>,
    /// Total players graduated to youth teams over the academy's history.
    pub graduates_produced: u16,
    /// Football identity the academy is trying to produce. It affects
    /// training emphasis and long-term recruitment balance, not match tactics.
    pub development_identity: AcademyDevelopmentIdentity,
    /// Rules for deciding when an academy player is ready for the U18/U19
    /// pathway rather than being held back inside the academy pool.
    pub pathway_policy: AcademyPathwayPolicy,
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
        ClubAcademy {
            settings: AcademySettings::default(),
            players: PlayerCollection::new(Vec::new()),
            staff: StaffCollection::new(Vec::new()),
            level,
            last_production_year: None,
            graduates_produced: 0,
            development_identity: AcademyDevelopmentIdentity::Balanced,
            pathway_policy: AcademyPathwayPolicy::for_level(level),
            pathway_reputation: (35 + academy_tier(level).saturating_mul(5)).min(90),
            recruitment_priorities: Vec::new(),
            last_pathway_review: None,
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> ClubAcademyResult {
        let players_result = self.players.simulate(ctx.with_player(None));

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
        self.pathway_reputation = self.calculate_pathway_reputation(&health);
        self.calibrate_player_count_range(&health);
        self.apply_player_welfare_controls(date);
        self.last_pathway_review = Some(date);
    }

    pub(super) fn pipeline_health(&self, date: NaiveDate) -> AcademyPipelineHealth {
        let mut health = AcademyPipelineHealth::default();

        for player in &self.players.players {
            let age = player.age(date);
            if age <= 11 {
                health.foundation_players += 1;
            } else if age <= 14 {
                health.development_players += 1;
            } else {
                health.professional_players += 1;
            }

            let group = player.position().position_group();
            health.group_counts[group_index(group)] += 1;

            let readiness = self.pathway_readiness_score(player, date);
            if readiness >= self.pathway_policy.readiness_threshold {
                health.ready_for_youth += 1;
            }
            if player.player_attributes.potential_ability >= 150 && readiness >= 75 {
                health.elite_prospects += 1;
            }
            if player.player_attributes.jadedness > 5500
                || player.player_attributes.condition < 5500
                || player.player_attributes.injury_proneness >= 17
            {
                health.at_risk_players += 1;
            }
        }

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

    fn calculate_pathway_reputation(&self, health: &AcademyPipelineHealth) -> u8 {
        let base = 30 + (academy_tier(self.level).saturating_mul(5)) as i16;
        let graduate_bonus = (self.graduates_produced.min(50) / 5) as i16;
        let ready_bonus = (health.ready_for_youth.min(8) as i16) * 2;
        let elite_bonus = (health.elite_prospects.min(5) as i16) * 3;
        let risk_drag = (health.at_risk_players.min(10) as i16) * 2;
        (base + graduate_bonus + ready_bonus + elite_bonus - risk_drag).clamp(0, 100) as u8
    }

    fn calibrate_player_count_range(&mut self, health: &AcademyPipelineHealth) {
        let base_min = 24 + academy_tier(self.level);
        let mut min_players = base_min;
        if health.ready_for_youth >= 6 {
            min_players = min_players.saturating_add(2);
        }
        if health.at_risk_players >= 8 {
            min_players = min_players.saturating_sub(2);
        }
        let max_players = min_players.saturating_add(18).min(58);
        self.settings.players_count_range = min_players..max_players;
    }

    fn apply_player_welfare_controls(&mut self, date: NaiveDate) {
        let policy = self.pathway_policy.clone();
        let level = self.level;

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

            let readiness = pathway_readiness_score_for(level, &policy, player, date);
            if readiness >= policy.readiness_threshold {
                player.player_attributes.update_reputation(1, 2, 0);
            }
        }
    }

    pub(super) fn pathway_readiness_score(&self, player: &Player, date: NaiveDate) -> i16 {
        pathway_readiness_score_for(self.level, &self.pathway_policy, player, date)
    }

    pub(super) fn recruitment_priority_position(
        &self,
        intake_index: usize,
    ) -> Option<PlayerPositionType> {
        let group = *self.recruitment_priorities.get(intake_index)?;
        Some(position_for_priority_group(group, intake_index))
    }
}

fn pathway_readiness_score_for(
    level: u8,
    policy: &AcademyPathwayPolicy,
    player: &Player,
    date: NaiveDate,
) -> i16 {
    let age = player.age(date);
    if age < policy.min_graduation_age {
        return 0;
    }

    let ca = player.player_attributes.current_ability as i16;
    let pa = player.player_attributes.potential_ability as i16;
    let potential_gap = (pa - ca).max(0);
    let personality = (player.attributes.professionalism * 0.45
        + player.skills.mental.determination * 0.30
        + player.skills.mental.work_rate * 0.25) as i16;
    let age_bonus = ((age.saturating_sub(policy.min_graduation_age)) as i16) * 3;
    let academy_bonus = academy_tier(level) as i16;
    let late_dev_bonus = if policy.protect_late_developers && potential_gap >= 35 {
        8
    } else {
        0
    };
    let risk_drag = if player.player_attributes.injury_proneness >= 17 {
        8
    } else {
        0
    };

    ca + potential_gap / 3 + personality + age_bonus + academy_bonus + late_dev_bonus - risk_drag
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
        // ClubAcademy.level is on the 1..20 facility-rating scale; the
        // pathway policy collapses internally to a 1..10 tier. Use facility
        // ratings directly here so the test mirrors how the field is
        // populated in production (FacilityLevel::to_rating).
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
}

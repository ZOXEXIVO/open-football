//! Synthetic-squad generation for nations whose real player pool is
//! too thin to field a credible 23. Produces fully-formed `Player`
//! records whose ability tracks the country's reputation, so a weak
//! nation faces opponents at roughly the right level instead of
//! conceding 17-0 with an empty net.

use super::NationalTeam;
use super::types::{NationalSelectionPolicy, NationalTeamLevel, SQUAD_SIZE, SYNTHETIC_POSITIONS};
use crate::club::player::interaction::ManagerInteractionLog;
use crate::club::player::load::PlayerLoad;
use crate::club::player::rapport::PlayerRapport;
use crate::shared::FullName;
use crate::utils::IntegerUtils;
use crate::{
    Mental, PersonAttributes, PersonBehaviour, PersonBehaviourState, Physical, Player,
    PlayerAttributes, PlayerDecisionHistory, PlayerFoots, PlayerHappiness, PlayerMailbox,
    PlayerPosition, PlayerPositionType, PlayerPositions, PlayerPreferredFoot, PlayerSkills,
    PlayerStatistics, PlayerStatisticsHistory, PlayerStatus, PlayerTraining, PlayerTrainingHistory,
    Relations, Technical,
};
use chrono::{Datelike, NaiveDate};

impl NationalTeam {
    /// Generate synthetic players for countries without enough club players.
    /// Ability is derived from country reputation. Senior-level default;
    /// U21 callers go through [`generate_synthetic_squad_with_policy`].
    pub(super) fn generate_synthetic_squad(&mut self, date: NaiveDate) {
        self.generate_synthetic_squad_with_policy(date, &NationalSelectionPolicy::senior());
    }

    /// Policy-driven synthetic-squad generation. Fills up to the policy's
    /// target size; synthetic player ages come from the policy's range
    /// (22-34 senior, 17-21 U21) and the id band is offset per level so a
    /// country's senior and U21 stand-ins never collide.
    pub(super) fn generate_synthetic_squad_with_policy(
        &mut self,
        date: NaiveDate,
        policy: &NationalSelectionPolicy,
    ) {
        self.generated_squad.clear();

        let target = match policy.level {
            NationalTeamLevel::Senior => SQUAD_SIZE,
            NationalTeamLevel::Under21 => policy.target_squad_size,
        };
        let slots_needed = target.saturating_sub(self.squad.len());
        if slots_needed == 0 {
            return;
        }

        // Derive ability from reputation (0-1000 reputation -> ~40-180 ability).
        // U21 stand-ins are pitched a touch below the senior baseline.
        let base_ability = {
            let senior = (self.reputation as f32 / 1000.0) * 140.0 + 40.0;
            match policy.level {
                NationalTeamLevel::Senior => senior as u8,
                NationalTeamLevel::Under21 => (senior * 0.85).clamp(30.0, 180.0) as u8,
            }
        };

        let (age_min, age_max) = policy.synthetic_age_range();
        // Offset U21 ids into a separate band so they can't collide with
        // the same country's senior synthetic players.
        let id_base = match policy.level {
            NationalTeamLevel::Senior => 900_000,
            NationalTeamLevel::Under21 => 1_000_000,
        };

        let positions_to_fill = &SYNTHETIC_POSITIONS[..slots_needed.min(SYNTHETIC_POSITIONS.len())];

        for (idx, &position) in positions_to_fill.iter().enumerate() {
            // Vary ability slightly per player
            let ability_variation = IntegerUtils::random(-10, 10) as i16;
            let ability = (base_ability as i16 + ability_variation).clamp(30, 200) as u8;

            let player = Self::generate_synthetic_player_aged(
                self.country_id,
                date,
                position,
                ability,
                idx as u32,
                age_min,
                age_max,
                id_base,
            );
            self.generated_squad.push(player);
        }
    }

    /// Generate a single synthetic player with senior defaults (age 22-34,
    /// id band 900_000). Used by the synthetic friendly-opponent builder.
    pub(super) fn generate_synthetic_player(
        country_id: u32,
        now: NaiveDate,
        position: PlayerPositionType,
        ability: u8,
        seed_offset: u32,
    ) -> Player {
        Self::generate_synthetic_player_aged(
            country_id,
            now,
            position,
            ability,
            seed_offset,
            22,
            34,
            900_000,
        )
    }

    /// Generate a single synthetic player with an explicit age range and
    /// id band, so U21 stand-ins are both age-appropriate and id-distinct
    /// from senior ones.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn generate_synthetic_player_aged(
        country_id: u32,
        now: NaiveDate,
        position: PlayerPositionType,
        ability: u8,
        seed_offset: u32,
        age_min: i32,
        age_max: i32,
        id_base: u32,
    ) -> Player {
        // `IntegerUtils::random` is exclusive of `max`, matching the
        // original senior call of `random(22, 34)` (ages 22-33).
        let age = IntegerUtils::random(age_min, age_max);
        let year = now.year() - age;
        let month = ((country_id + seed_offset) % 12 + 1) as u32;
        let day = ((country_id + seed_offset * 7) % 28 + 1) as u32;

        // Use deterministic ID based on country + position + offset
        let id = id_base + country_id * 100 + seed_offset;

        // Scale skills based on ability (ability 0-200 -> skill factor 0.25-1.0)
        let skill_factor = (ability as f32 / 200.0).clamp(0.25, 1.0);
        let base_skill = skill_factor * 20.0;

        let position_level = (skill_factor * 20.0) as u8;

        Player {
            id,
            full_name: FullName::with_full(
                format!("NT{}", seed_offset),
                format!("Player{}", country_id),
                String::new(),
            ),
            birth_date: NaiveDate::from_ymd_opt(year, month, day)
                .unwrap_or(NaiveDate::from_ymd_opt(year, 1, 1).unwrap()),
            country_id,
            nationality_continent_id: 0,
            behaviour: PersonBehaviour {
                state: PersonBehaviourState::Normal,
            },
            attributes: PersonAttributes {
                adaptability: base_skill,
                ambition: base_skill,
                controversy: 5.0,
                loyalty: base_skill,
                pressure: base_skill,
                professionalism: base_skill,
                sportsmanship: base_skill,
                temperament: base_skill,
                consistency: base_skill,
                important_matches: base_skill,
                dirtiness: 5.0,
            },
            happiness: PlayerHappiness::new(),
            statuses: PlayerStatus { statuses: vec![] },
            skills: PlayerSkills {
                technical: Technical {
                    corners: base_skill,
                    crossing: base_skill,
                    dribbling: base_skill,
                    finishing: base_skill,
                    first_touch: base_skill,
                    free_kicks: base_skill,
                    heading: base_skill,
                    long_shots: base_skill,
                    long_throws: base_skill,
                    marking: base_skill,
                    passing: base_skill,
                    penalty_taking: base_skill,
                    tackling: base_skill,
                    technique: base_skill,
                },
                mental: Mental {
                    aggression: base_skill,
                    anticipation: base_skill,
                    bravery: base_skill,
                    composure: base_skill,
                    concentration: base_skill,
                    decisions: base_skill,
                    determination: base_skill,
                    flair: base_skill,
                    leadership: base_skill,
                    off_the_ball: base_skill,
                    positioning: base_skill,
                    teamwork: base_skill,
                    vision: base_skill,
                    work_rate: base_skill,
                },
                physical: Physical {
                    acceleration: base_skill,
                    agility: base_skill,
                    balance: base_skill,
                    jumping: base_skill,
                    natural_fitness: base_skill,
                    pace: base_skill,
                    stamina: base_skill,
                    strength: base_skill,
                    match_readiness: 15.0,
                },
                goalkeeping: Default::default(),
            },
            contract: None,
            contract_loan: None,
            positions: PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: position_level,
                }],
            },
            preferred_foot: PlayerPreferredFoot::Right,
            foots: PlayerFoots::from_preferred(&PlayerPreferredFoot::Right),
            player_attributes: PlayerAttributes {
                is_banned: false,
                is_injured: false,
                condition: 10000,
                fitness: 0,
                jadedness: 0,
                weight: 75,
                height: 180,
                value: 0,
                current_reputation: (ability as i16) * 5,
                home_reputation: 1000,
                world_reputation: (ability as i16) * 3,
                current_ability: ability,
                potential_ability: ability,
                international_apps: 0,
                international_goals: 0,
                under_21_international_apps: 0,
                under_21_international_goals: 0,
                injury_days_remaining: 0,
                injury_type: None,
                injury_proneness: 10,
                recovery_days_remaining: 0,
                last_injury_body_part: 0,
                injury_count: 0,
                days_since_last_match: 0,
                suspension_matches: 0,
                yellow_card_running: 0,
            },
            mailbox: PlayerMailbox::new(),
            training: PlayerTraining::new(),
            training_history: PlayerTrainingHistory::new(),
            relations: Relations::new(),
            statistics: PlayerStatistics::default(),
            friendly_statistics: PlayerStatistics::default(),
            friendly_source_slug: None,
            cup_statistics: PlayerStatistics::default(),
            cup_statistics_by_competition: Vec::new(),
            statistics_history: PlayerStatisticsHistory::new(),
            decision_history: PlayerDecisionHistory::new(),
            individual_training: None,
            languages: Vec::new(),
            last_transfer_date: None,
            plan: None,
            favorite_clubs: Vec::new(),
            sold_from: None,
            sell_on_obligations: Vec::new(),
            traits: Vec::new(),
            is_force_match_selection: false,
            rapport: PlayerRapport::new(),
            promises: Vec::new(),
            interactions: ManagerInteractionLog::new(),
            pending_signing: None,
            generated: true,
            retired: false,
            load: PlayerLoad::new(),
            pending_contract_ask: None,
            pending_pre_contract: None,
            last_intl_caps_paid: 0,
            free_agent_state: None,
            availability_market: None,
            squad_social_view: None,
            transfer_request_reasons: Vec::new(),
            // Synthetic seniors (age 22-34) are established pros, not debutants.
            made_senior_debut: true,
            awards_count: Default::default(),
            release_reason: None,
        }
    }
}

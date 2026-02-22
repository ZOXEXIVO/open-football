use crate::country::PeopleNameGeneratorData;
use crate::r#match::engine::FootballEngine;
use crate::r#match::{MatchPlayer, MatchSquad};
use crate::shared::FullName;
use crate::utils::IntegerUtils;
use crate::{
    Club, MatchTacticType, Mental, PersonAttributes, PersonBehaviour, PersonBehaviourState,
    Physical, Player, PlayerAttributes, PlayerHappiness, PlayerMailbox, PlayerPosition,
    PlayerPositionType, PlayerPositions, PlayerPreferredFoot, PlayerSkills, PlayerStatistics,
    PlayerStatisticsHistory, PlayerStatus, PlayerStatusType, PlayerTraining, PlayerTrainingHistory,
    Relations, Tactics, TeamType, Technical,
};
use chrono::{Datelike, NaiveDate};
use log::info;

pub struct NationalTeam {
    pub country_id: u32,
    pub country_name: String,
    pub staff: Vec<NationalTeamStaffMember>,
    pub squad: Vec<NationalSquadPlayer>,
    pub generated_squad: Vec<Player>,
    pub tactics: Tactics,
    pub reputation: u16,
    pub elo_rating: u16,
    pub schedule: Vec<NationalTeamFixture>,
    pub results: Vec<NationalTeamMatchResult>,
}

pub struct NationalTeamStaffMember {
    pub first_name: String,
    pub last_name: String,
    pub role: NationalTeamStaffRole,
    pub country_id: u32,
    pub birth_year: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NationalTeamStaffRole {
    Manager,
    AssistantManager,
    Coach,
    GoalkeeperCoach,
    FitnessCoach,
}

impl NationalTeamStaffRole {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            NationalTeamStaffRole::Manager => "staff_manager",
            NationalTeamStaffRole::AssistantManager => "staff_assistant_manager",
            NationalTeamStaffRole::Coach => "staff_coach",
            NationalTeamStaffRole::GoalkeeperCoach => "staff_goalkeeper_coach",
            NationalTeamStaffRole::FitnessCoach => "staff_fitness_coach",
        }
    }
}

pub struct NationalSquadPlayer {
    pub player_id: u32,
    pub club_id: u32,
    pub team_id: u32,
}

pub struct NationalTeamFixture {
    pub date: NaiveDate,
    pub opponent_country_id: u32,
    pub is_home: bool,
    pub result: Option<NationalTeamMatchResult>,
}

#[derive(Clone)]
pub struct NationalTeamMatchResult {
    pub home_score: u8,
    pub away_score: u8,
    pub date: NaiveDate,
    pub opponent_country_id: u32,
}

/// Break windows matching League::is_international_break:
/// Sep 4-12, Oct 9-17, Nov 13-21, Mar 20-28
const BREAK_WINDOWS: [(u32, u32, u32); 4] = [
    (9, 4, 12),
    (10, 9, 17),
    (11, 13, 21),
    (3, 20, 28),
];

/// Tournament window: June-July for World Cup / Euro finals
const TOURNAMENT_WINDOW: (u32, u32, u32, u32) = (6, 10, 7, 15);

const DEFAULT_STAFF_ROLES: [NationalTeamStaffRole; 5] = [
    NationalTeamStaffRole::Manager,
    NationalTeamStaffRole::AssistantManager,
    NationalTeamStaffRole::Coach,
    NationalTeamStaffRole::GoalkeeperCoach,
    NationalTeamStaffRole::FitnessCoach,
];

/// Minimum number of real club players before generating synthetic ones
const MIN_REAL_PLAYERS: usize = 16;

/// Default squad call-up size
const SQUAD_SIZE: usize = 23;

/// Minimum country reputation to simulate friendlies (skips ~147 small nations)
const MIN_REPUTATION_FOR_FRIENDLIES: u16 = 4000;

/// Positions template for generating a balanced synthetic squad
const SYNTHETIC_POSITIONS: [PlayerPositionType; 23] = [
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::DefenderLeft,
    PlayerPositionType::DefenderCenterLeft,
    PlayerPositionType::DefenderCenter,
    PlayerPositionType::DefenderCenterRight,
    PlayerPositionType::DefenderRight,
    PlayerPositionType::DefenderCenter,
    PlayerPositionType::MidfielderLeft,
    PlayerPositionType::MidfielderCenterLeft,
    PlayerPositionType::MidfielderCenter,
    PlayerPositionType::MidfielderCenterRight,
    PlayerPositionType::MidfielderRight,
    PlayerPositionType::MidfielderCenter,
    PlayerPositionType::AttackingMidfielderCenter,
    PlayerPositionType::ForwardLeft,
    PlayerPositionType::ForwardCenter,
    PlayerPositionType::ForwardRight,
    PlayerPositionType::Striker,
    PlayerPositionType::DefenderCenter,
    PlayerPositionType::MidfielderCenter,
    PlayerPositionType::ForwardCenter,
    PlayerPositionType::Striker,
];

impl NationalTeam {
    pub fn new(country_id: u32, names: &PeopleNameGeneratorData) -> Self {
        let staff = Self::generate_staff(country_id, names);

        NationalTeam {
            country_id,
            country_name: String::new(),
            staff,
            squad: Vec::new(),
            generated_squad: Vec::new(),
            tactics: Tactics::new(MatchTacticType::T442),
            reputation: 0,
            elo_rating: 1500,
            schedule: Vec::new(),
            results: Vec::new(),
        }
    }

    fn generate_staff(
        country_id: u32,
        names: &PeopleNameGeneratorData,
    ) -> Vec<NationalTeamStaffMember> {
        DEFAULT_STAFF_ROLES
            .iter()
            .map(|&role| {
                let first_name = Self::random_name(&names.first_names);
                let last_name = Self::random_name(&names.last_names);
                let birth_year = IntegerUtils::random(1960, 1990);

                NationalTeamStaffMember {
                    first_name,
                    last_name,
                    role,
                    country_id,
                    birth_year,
                }
            })
            .collect()
    }

    fn random_name(names: &[String]) -> String {
        if names.is_empty() {
            return "Unknown".to_string();
        }
        let idx = IntegerUtils::random(0, names.len() as i32) as usize;
        names
            .get(idx)
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// Main simulation entry point — called each day from Country::simulate
    pub fn simulate(&mut self, clubs: &mut [Club], date: NaiveDate, country_id: u32) {
        if self.reputation < MIN_REPUTATION_FOR_FRIENDLIES {
            return;
        }

        // Handle international break call-ups
        if Self::is_break_start(date) {
            self.call_up_squad(clubs, date, country_id);
        }

        // Handle tournament period call-ups (June-July)
        if Self::is_tournament_start(date) && self.squad.is_empty() {
            self.call_up_squad(clubs, date, country_id);
        }

        // Play scheduled friendly fixtures (non-competition matches)
        if let Some(fixture_idx) = self
            .schedule
            .iter()
            .position(|f| f.date == date && f.result.is_none())
        {
            self.play_match(clubs, fixture_idx, date);
        }

        // Release squad at break end
        if Self::is_break_end(date) {
            self.release_squad(clubs);
        }

        // Release squad at tournament end
        if Self::is_tournament_end(date) && !self.squad.is_empty() {
            self.release_squad(clubs);
        }
    }

    /// Call up squad filtering by nationality (country_id match)
    pub fn call_up_squad(&mut self, clubs: &mut [Club], date: NaiveDate, country_id: u32) {
        self.squad.clear();
        self.schedule.clear();

        // Collect eligible players from Main teams — filter by nationality
        let mut candidates: Vec<(u32, u32, u32, u8)> = Vec::new();

        for club in clubs.iter() {
            for team in club.teams.teams.iter() {
                if team.team_type != TeamType::Main {
                    continue;
                }
                for player in team.players.players.iter() {
                    // Nationality filter: only players from this country
                    if player.country_id != country_id {
                        continue;
                    }
                    if player.player_attributes.is_injured
                        || player.player_attributes.is_banned
                        || player.statuses.get().contains(&PlayerStatusType::Loa)
                    {
                        continue;
                    }
                    candidates.push((
                        player.id,
                        club.id,
                        team.id,
                        player.player_attributes.current_ability,
                    ));
                }
            }
        }

        // Sort by ability descending, pick top 23
        candidates.sort_by(|a, b| b.3.cmp(&a.3));
        candidates.truncate(SQUAD_SIZE);

        for (player_id, club_id, team_id, _) in &candidates {
            self.squad.push(NationalSquadPlayer {
                player_id: *player_id,
                club_id: *club_id,
                team_id: *team_id,
            });
        }

        // If fewer than MIN_REAL_PLAYERS found, generate synthetic squad
        if candidates.len() < MIN_REAL_PLAYERS {
            self.generate_synthetic_squad(date);
        }

        // Set Int status on called-up players in clubs
        for club in clubs.iter_mut() {
            for team in club.teams.teams.iter_mut() {
                for player in team.players.players.iter_mut() {
                    if self.squad.iter().any(|s| s.player_id == player.id) {
                        player.statuses.add(date, PlayerStatusType::Int);
                    }
                }
            }
        }

        // Generate 2 friendly fixtures within the break window (if not in tournament period)
        if !Self::is_in_tournament_period(date) {
            let (_, start_day, end_day) = Self::current_break_window(date)
                .unwrap_or((date.month(), date.day(), date.day() + 8));

            let match_day_1 = start_day + 2;
            let match_day_2 = end_day - 2;

            let year = date.year();
            let month = date.month();

            if let (Some(d1), Some(d2)) = (
                NaiveDate::from_ymd_opt(year, month, match_day_1),
                NaiveDate::from_ymd_opt(year, month, match_day_2),
            ) {
                let opponent_1 = Self::random_opponent(country_id);
                let opponent_2 = Self::random_opponent(country_id);

                self.schedule.push(NationalTeamFixture {
                    date: d1,
                    opponent_country_id: opponent_1,
                    is_home: true,
                    result: None,
                });
                self.schedule.push(NationalTeamFixture {
                    date: d2,
                    opponent_country_id: opponent_2,
                    is_home: false,
                    result: None,
                });
            }
        }

        info!(
            "National team {} (country {}) called up {} players ({} from clubs, {} synthetic)",
            self.country_name,
            country_id,
            self.squad.len() + self.generated_squad.len(),
            self.squad.len(),
            self.generated_squad.len()
        );
    }

    /// Generate synthetic players for countries without enough club players.
    /// Ability is derived from country reputation.
    fn generate_synthetic_squad(&mut self, date: NaiveDate) {
        self.generated_squad.clear();

        let slots_needed = SQUAD_SIZE.saturating_sub(self.squad.len());
        if slots_needed == 0 {
            return;
        }

        // Derive ability from reputation (0-1000 reputation -> ~40-180 ability)
        let base_ability = ((self.reputation as f32 / 1000.0) * 140.0 + 40.0) as u8;

        let positions_to_fill = &SYNTHETIC_POSITIONS[..slots_needed.min(SYNTHETIC_POSITIONS.len())];

        for (idx, &position) in positions_to_fill.iter().enumerate() {
            // Vary ability slightly per player
            let ability_variation = IntegerUtils::random(-10, 10) as i16;
            let ability = (base_ability as i16 + ability_variation).clamp(30, 200) as u8;

            let player = Self::generate_synthetic_player(
                self.country_id,
                date,
                position,
                ability,
                idx as u32,
            );
            self.generated_squad.push(player);
        }
    }

    /// Generate a single synthetic player with the given attributes
    fn generate_synthetic_player(
        country_id: u32,
        now: NaiveDate,
        position: PlayerPositionType,
        ability: u8,
        seed_offset: u32,
    ) -> Player {
        let age = IntegerUtils::random(22, 34);
        let year = now.year() - age;
        let month = ((country_id + seed_offset) % 12 + 1) as u32;
        let day = ((country_id + seed_offset * 7) % 28 + 1) as u32;

        // Use deterministic ID based on country + position + offset
        let id = 900_000 + country_id * 100 + seed_offset;

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
            },
            contract: None,
            positions: PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: position_level,
                }],
            },
            preferred_foot: PlayerPreferredFoot::Right,
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
            },
            mailbox: PlayerMailbox::new(),
            training: PlayerTraining::new(),
            training_history: PlayerTrainingHistory::new(),
            relations: Relations::new(),
            statistics: PlayerStatistics::default(),
            statistics_history: PlayerStatisticsHistory::new(),
        }
    }

    /// Build a MatchSquad from the called-up squad + generated players
    pub fn build_match_squad(&self, clubs: &[Club]) -> MatchSquad {
        let team_id = self.country_id;
        let team_name = self.country_name.clone();

        // Collect real players from clubs
        let mut all_players: Vec<&Player> = Vec::new();

        for squad_player in &self.squad {
            for club in clubs.iter() {
                for team in club.teams.teams.iter() {
                    for player in team.players.players.iter() {
                        if player.id == squad_player.player_id {
                            all_players.push(player);
                        }
                    }
                }
            }
        }

        // Add generated synthetic players
        for player in &self.generated_squad {
            all_players.push(player);
        }

        // Select starting 11 and substitutes
        let tactics = &self.tactics;
        let required_positions = tactics.positions();

        let mut main_squad: Vec<MatchPlayer> = Vec::with_capacity(11);
        let mut used_ids: Vec<u32> = Vec::new();

        // Pick goalkeeper
        if let Some(gk) = all_players
            .iter()
            .filter(|p| {
                p.positions
                    .positions
                    .iter()
                    .any(|pos| pos.position == PlayerPositionType::Goalkeeper)
            })
            .max_by_key(|p| p.player_attributes.current_ability)
        {
            main_squad.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        }

        // Fill outfield positions
        for &pos in required_positions.iter() {
            if pos == PlayerPositionType::Goalkeeper {
                continue;
            }
            if main_squad.len() >= 11 {
                break;
            }

            let best = all_players
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| {
                    !p.positions
                        .positions
                        .iter()
                        .any(|pp| pp.position == PlayerPositionType::Goalkeeper)
                })
                .max_by_key(|p| {
                    let pos_fit = p.positions.get_level(pos) as u16;
                    let ability = p.player_attributes.current_ability as u16;
                    pos_fit * 3 + ability
                });

            if let Some(player) = best {
                main_squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
        }

        // Fill any remaining starting slots
        while main_squad.len() < 11 {
            let best = all_players
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by_key(|p| p.player_attributes.current_ability);

            match best {
                Some(player) => {
                    let pos = player.position();
                    main_squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        // Select substitutes (up to 7)
        let mut substitutes: Vec<MatchPlayer> = Vec::with_capacity(7);
        let remaining: Vec<&&Player> = all_players
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .collect();

        // Backup GK first
        if let Some(gk) = remaining
            .iter()
            .filter(|p| {
                p.positions
                    .positions
                    .iter()
                    .any(|pos| pos.position == PlayerPositionType::Goalkeeper)
            })
            .max_by_key(|p| p.player_attributes.current_ability)
        {
            substitutes.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        }

        // Fill rest of bench
        let mut bench_remaining: Vec<&&Player> = remaining
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .copied()
            .collect();
        bench_remaining.sort_by(|a, b| {
            b.player_attributes
                .current_ability
                .cmp(&a.player_attributes.current_ability)
        });

        for player in bench_remaining.iter().take(6) {
            let pos = player.position();
            substitutes.push(MatchPlayer::from_player(team_id, player, pos, false));
        }

        MatchSquad {
            team_id,
            team_name,
            tactics: self.tactics.clone(),
            main_squad,
            substitutes,
            captain_id: None,
            vice_captain_id: None,
            penalty_taker_id: None,
            free_kick_taker_id: None,
        }
    }

    /// Play a friendly match using the FootballEngine
    fn play_match(&mut self, clubs: &mut [Club], fixture_idx: usize, date: NaiveDate) {
        let fixture = &self.schedule[fixture_idx];
        let opponent_id = fixture.opponent_country_id;
        let is_home = fixture.is_home;

        // Build our squad
        let our_squad = self.build_match_squad(clubs);

        // Build opponent squad (synthetic since we don't have cross-country access here)
        let opponent_squad = Self::build_synthetic_opponent_squad(opponent_id);

        // Run match through the engine
        let (home_squad, away_squad) = if is_home {
            (our_squad, opponent_squad)
        } else {
            (opponent_squad, our_squad)
        };

        let match_result = FootballEngine::<840, 545>::play(home_squad, away_squad);

        let score = match_result.score.as_ref().expect("match should have score");
        let home_score = score.home_team.get();
        let away_score = score.away_team.get();

        let result = NationalTeamMatchResult {
            home_score,
            away_score,
            date,
            opponent_country_id: opponent_id,
        };

        // Update player stats from actual match data
        let squad_player_ids: Vec<u32> = self.squad.iter().map(|s| s.player_id).collect();

        for club in clubs.iter_mut() {
            for team in club.teams.teams.iter_mut() {
                for player in team.players.players.iter_mut() {
                    if squad_player_ids.contains(&player.id) {
                        player.player_attributes.international_apps += 1;

                        // Check if this player scored in the actual match
                        if let Some(stats) = match_result.player_stats.get(&player.id) {
                            player.player_attributes.international_goals += stats.goals as u16;
                        }
                    }
                }
            }
        }

        // Update Elo rating
        let (our_score, opp_score) = if is_home {
            (home_score, away_score)
        } else {
            (away_score, home_score)
        };
        self.update_elo(our_score, opp_score, 1500); // Assume opponent Elo 1500

        self.schedule[fixture_idx].result = Some(result.clone());
        self.results.push(result);

        info!(
            "International match: {} vs country {} - {}:{}",
            self.country_name, opponent_id, home_score, away_score
        );
    }

    /// Play a competition match between two national teams.
    /// Returns (home_score, away_score, player_goals: HashMap<player_id, goals>).
    /// This is called from the continent level for cross-country matches.
    pub fn play_competition_match(
        home_squad: MatchSquad,
        away_squad: MatchSquad,
    ) -> (u8, u8, std::collections::HashMap<u32, u16>) {
        let match_result = FootballEngine::<840, 545>::play(home_squad, away_squad);

        let score = match_result
            .score
            .as_ref()
            .expect("match should have score");
        let home_score = score.home_team.get();
        let away_score = score.away_team.get();

        // Collect player goals
        let player_goals: std::collections::HashMap<u32, u16> = match_result
            .player_stats
            .iter()
            .filter(|(_, stats)| stats.goals > 0)
            .map(|(&id, stats)| (id, stats.goals))
            .collect();

        (home_score, away_score, player_goals)
    }

    /// Build a synthetic opponent squad for friendly matches
    fn build_synthetic_opponent_squad(opponent_country_id: u32) -> MatchSquad {
        let team_id = opponent_country_id;

        // Generate 18 synthetic players with moderate ability
        let now = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let positions = &SYNTHETIC_POSITIONS[..18];

        let mut players: Vec<Player> = Vec::new();
        for (idx, &pos) in positions.iter().enumerate() {
            let ability = IntegerUtils::random(80, 140) as u8;
            let player = Self::generate_synthetic_player(
                opponent_country_id,
                now,
                pos,
                ability,
                idx as u32 + 50, // offset to avoid ID collision
            );
            players.push(player);
        }

        let tactics = Tactics::new(MatchTacticType::T442);
        let required_positions = tactics.positions();

        let mut main_squad: Vec<MatchPlayer> = Vec::with_capacity(11);
        let mut used_ids: Vec<u32> = Vec::new();

        // GK
        if let Some(gk) = players.iter().find(|p| {
            p.positions
                .positions
                .iter()
                .any(|pos| pos.position == PlayerPositionType::Goalkeeper)
        }) {
            main_squad.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        }

        // Outfield
        for &pos in required_positions.iter() {
            if pos == PlayerPositionType::Goalkeeper || main_squad.len() >= 11 {
                continue;
            }
            if let Some(player) = players
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by_key(|p| p.positions.get_level(pos) as u16 + p.player_attributes.current_ability as u16)
            {
                main_squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
        }

        // Subs
        let substitutes: Vec<MatchPlayer> = players
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .take(7)
            .map(|p| {
                let pos = p.position();
                MatchPlayer::from_player(team_id, p, pos, false)
            })
            .collect();

        MatchSquad {
            team_id,
            team_name: format!("Country {}", opponent_country_id),
            tactics,
            main_squad,
            substitutes,
            captain_id: None,
            vice_captain_id: None,
            penalty_taker_id: None,
            free_kick_taker_id: None,
        }
    }

    fn release_squad(&mut self, clubs: &mut [Club]) {
        let squad_player_ids: Vec<u32> = self.squad.iter().map(|s| s.player_id).collect();

        for club in clubs.iter_mut() {
            for team in club.teams.teams.iter_mut() {
                for player in team.players.players.iter_mut() {
                    if squad_player_ids.contains(&player.id) {
                        player.statuses.remove(PlayerStatusType::Int);
                    }
                }
            }
        }

        info!(
            "National team {} (country {}) released {} players from international duty",
            self.country_name,
            self.country_id,
            self.squad.len()
        );

        self.squad.clear();
        self.schedule.clear();
        self.generated_squad.clear();
    }

    /// Update Elo rating after a match
    pub fn update_elo(&mut self, our_score: u8, opponent_score: u8, opponent_elo: u16) {
        let k: f32 = 20.0;
        let expected = 1.0 / (1.0 + 10.0_f32.powf((opponent_elo as f32 - self.elo_rating as f32) / 400.0));

        let actual = if our_score > opponent_score {
            1.0
        } else if our_score == opponent_score {
            0.5
        } else {
            0.0
        };

        let change = (k * (actual - expected)) as i16;
        self.elo_rating = (self.elo_rating as i16 + change).clamp(500, 2500) as u16;
    }

    pub fn is_break_start(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        BREAK_WINDOWS
            .iter()
            .any(|(m, start, _)| month == *m && day == *start)
    }

    pub fn is_break_end(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        BREAK_WINDOWS
            .iter()
            .any(|(m, _, end)| month == *m && day == *end)
    }

    pub fn is_in_break(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        BREAK_WINDOWS
            .iter()
            .any(|(m, start, end)| month == *m && day >= *start && day <= *end)
    }

    fn is_tournament_start(date: NaiveDate) -> bool {
        date.month() == TOURNAMENT_WINDOW.0 && date.day() == TOURNAMENT_WINDOW.1
    }

    fn is_tournament_end(date: NaiveDate) -> bool {
        date.month() == TOURNAMENT_WINDOW.2 && date.day() == TOURNAMENT_WINDOW.3
    }

    fn is_in_tournament_period(date: NaiveDate) -> bool {
        let month = date.month();
        (month == TOURNAMENT_WINDOW.0 && date.day() >= TOURNAMENT_WINDOW.1)
            || (month == TOURNAMENT_WINDOW.2 && date.day() <= TOURNAMENT_WINDOW.3)
    }

    fn current_break_window(date: NaiveDate) -> Option<(u32, u32, u32)> {
        let month = date.month();
        BREAK_WINDOWS
            .iter()
            .find(|(m, _, _)| month == *m)
            .copied()
    }

    fn random_opponent(exclude_country_id: u32) -> u32 {
        loop {
            let id = IntegerUtils::random(1, 200) as u32;
            if id != exclude_country_id {
                return id;
            }
        }
    }
}

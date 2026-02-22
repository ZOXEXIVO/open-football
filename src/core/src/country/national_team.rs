use crate::country::PeopleNameGeneratorData;
use crate::utils::IntegerUtils;
use crate::{Club, PlayerStatusType, TeamType};
use chrono::{Datelike, NaiveDate};
use log::info;

pub struct NationalTeam {
    pub country_id: u32,
    pub staff: Vec<NationalTeamStaffMember>,
    pub squad: Vec<NationalSquadPlayer>,
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

const DEFAULT_STAFF_ROLES: [NationalTeamStaffRole; 5] = [
    NationalTeamStaffRole::Manager,
    NationalTeamStaffRole::AssistantManager,
    NationalTeamStaffRole::Coach,
    NationalTeamStaffRole::GoalkeeperCoach,
    NationalTeamStaffRole::FitnessCoach,
];

impl NationalTeam {
    pub fn new(country_id: u32, names: &PeopleNameGeneratorData) -> Self {
        let staff = Self::generate_staff(country_id, names);

        NationalTeam {
            country_id,
            staff,
            squad: Vec::new(),
            schedule: Vec::new(),
            results: Vec::new(),
        }
    }

    fn generate_staff(country_id: u32, names: &PeopleNameGeneratorData) -> Vec<NationalTeamStaffMember> {
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
        names.get(idx).cloned().unwrap_or_else(|| "Unknown".to_string())
    }

    pub fn simulate(&mut self, clubs: &mut [Club], date: NaiveDate, country_id: u32) {
        if Self::is_break_start(date) {
            self.call_up_squad(clubs, date, country_id);
        }

        if let Some(fixture_idx) = self.schedule.iter().position(|f| f.date == date && f.result.is_none()) {
            self.play_match(clubs, fixture_idx, date);
        }

        if Self::is_break_end(date) {
            self.release_squad(clubs);
        }
    }

    fn call_up_squad(&mut self, clubs: &mut [Club], date: NaiveDate, country_id: u32) {
        self.squad.clear();
        self.schedule.clear();

        // Collect eligible players from Main teams across all clubs
        let mut candidates: Vec<(u32, u32, u32, u8)> = Vec::new(); // (player_id, club_id, team_id, ability)

        for club in clubs.iter() {
            for team in club.teams.teams.iter() {
                if team.team_type != TeamType::Main {
                    continue;
                }
                for player in team.players.players.iter() {
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
        candidates.truncate(23);

        for (player_id, club_id, team_id, _) in &candidates {
            self.squad.push(NationalSquadPlayer {
                player_id: *player_id,
                club_id: *club_id,
                team_id: *team_id,
            });
        }

        // Set Int status on called-up players
        for club in clubs.iter_mut() {
            for team in club.teams.teams.iter_mut() {
                for player in team.players.players.iter_mut() {
                    if self.squad.iter().any(|s| s.player_id == player.id) {
                        player.statuses.add(date, PlayerStatusType::Int);
                    }
                }
            }
        }

        // Generate 2 fixtures within the break window
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

        info!(
            "🏴 National team (country {}) called up {} players for international break",
            country_id,
            self.squad.len()
        );
    }

    fn play_match(&mut self, clubs: &mut [Club], fixture_idx: usize, date: NaiveDate) {
        let fixture = &self.schedule[fixture_idx];
        let opponent_id = fixture.opponent_country_id;

        // Simple score simulation
        let home_score = IntegerUtils::random(0, 4) as u8;
        let away_score = IntegerUtils::random(0, 3) as u8;

        let result = NationalTeamMatchResult {
            home_score,
            away_score,
            date,
            opponent_country_id: opponent_id,
        };

        // Increment stats for squad players
        let squad_player_ids: Vec<u32> = self.squad.iter().map(|s| s.player_id).collect();

        for club in clubs.iter_mut() {
            for team in club.teams.teams.iter_mut() {
                for player in team.players.players.iter_mut() {
                    if squad_player_ids.contains(&player.id) {
                        player.player_attributes.international_apps += 1;

                        // Random chance for goals (roughly 20% per player)
                        if IntegerUtils::random(0, 100) < 20 {
                            player.player_attributes.international_goals += 1;
                        }
                    }
                }
            }
        }

        self.schedule[fixture_idx].result = Some(result.clone());
        self.results.push(result);

        info!(
            "🏴 International match: country {} vs country {} — {} : {}",
            self.country_id, opponent_id, home_score, away_score
        );
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
            "🏴 National team (country {}) released {} players from international duty",
            self.country_id,
            self.squad.len()
        );

        self.squad.clear();
        self.schedule.clear();
    }

    fn is_break_start(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        BREAK_WINDOWS.iter().any(|(m, start, _)| month == *m && day == *start)
    }

    fn is_break_end(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        BREAK_WINDOWS.iter().any(|(m, _, end)| month == *m && day == *end)
    }

    fn current_break_window(date: NaiveDate) -> Option<(u32, u32, u32)> {
        let month = date.month();
        BREAK_WINDOWS.iter()
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

use crate::club::player::load::PlayerLoad;
use crate::club::player::rapport::PlayerRapport;
use crate::country::PeopleNameGeneratorData;
use crate::r#match::{MatchPlayer, MatchSquad};
use crate::shared::FullName;
use crate::utils::IntegerUtils;
use crate::{
    Club, MatchTacticType, Mental, PersonAttributes, PersonBehaviour, PersonBehaviourState,
    Physical, Player, PlayerAttributes, PlayerDecisionHistory, PlayerFieldPositionGroup,
    PlayerHappiness, PlayerMailbox, PlayerPosition, PlayerPositionType, PlayerPositions,
    PlayerPreferredFoot, PlayerSkills, PlayerStatistics, PlayerStatisticsHistory, PlayerStatus,
    PlayerStatusType, PlayerTraining, PlayerTrainingHistory, Relations, Tactics, TeamType,
    Technical,
};
use crate::Country;
use chrono::{Datelike, NaiveDate};
use log::{debug};
use std::collections::HashMap;

#[derive(Clone)]
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
}

#[derive(Clone)]
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

#[derive(Clone)]
pub struct NationalSquadPlayer {
    pub player_id: u32,
    pub club_id: u32,
    pub team_id: u32,
}

#[derive(Clone)]
pub struct NationalTeamFixture {
    pub date: NaiveDate,
    pub opponent_country_id: u32,
    pub opponent_country_name: String,
    pub is_home: bool,
    pub competition_name: String,
    pub match_id: String,
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
pub(crate) const MIN_REPUTATION_FOR_FRIENDLIES: u16 = 4000;

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

/// Data collected from a candidate player for call-up scoring
pub(crate) struct CallUpCandidate {
    player_id: u32,
    club_id: u32,
    team_id: u32,
    current_ability: u8,
    potential_ability: u8,
    age: i32,
    condition_pct: f32,
    match_readiness: f32,
    average_rating: f32,
    played: u16,
    international_apps: u16,
    international_goals: u16,
    leadership: f32,
    composure: f32,
    teamwork: f32,
    determination: f32,
    pressure_handling: f32,
    world_reputation: i16,
    /// League reputation where the player competes (0-1000, higher = stronger league)
    league_reputation: u16,
    position_levels: Vec<(PlayerPositionType, u8)>,
    position_group: PlayerFieldPositionGroup,
    // Season performance events
    goals: u16,
    assists: u16,
    player_of_the_match: u8,
    clean_sheets: u16,
    yellow_cards: u8,
    red_cards: u8,
}

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

    /// Returns the fixture index of a pending friendly for today, if any.
    pub fn pending_friendly(&self, date: NaiveDate) -> Option<usize> {
        self.schedule
            .iter()
            .position(|f| f.date == date && f.result.is_none())
    }

    /// Apply the result of a friendly match that was played externally (in parallel).
    pub fn apply_friendly_result(
        &mut self,
        clubs: &mut [Club],
        fixture_idx: usize,
        match_result: &crate::r#match::MatchResultRaw,
        date: NaiveDate,
    ) {
        let fixture = &self.schedule[fixture_idx];
        let opponent_id = fixture.opponent_country_id;
        let opponent_name = fixture.opponent_country_name.clone();
        let is_home = fixture.is_home;

        let score = match_result
            .score
            .as_ref()
            .expect("match should have score");
        let home_score = score.home_team.get();
        let away_score = score.away_team.get();

        let result = NationalTeamMatchResult {
            home_score,
            away_score,
            date,
            opponent_country_id: opponent_id,
        };

        // Update player stats
        let squad_player_ids: Vec<u32> = self.squad.iter().map(|s| s.player_id).collect();

        for club in clubs.iter_mut() {
            for team in club.teams.iter_mut() {
                for player in team.players.iter_mut() {
                    if squad_player_ids.contains(&player.id) {
                        player.player_attributes.international_apps += 1;

                        if let Some(stats) = match_result.player_stats.get(&player.id) {
                            player.player_attributes.international_goals +=
                                stats.goals as u16;
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
        self.update_elo(our_score, opp_score, 1500);

        self.schedule[fixture_idx].result = Some(result);

        debug!(
            "International friendly: {} vs {} - {}:{}",
            self.country_name, opponent_name, home_score, away_score
        );
    }

    /// Collect eligible national team candidates from clubs.
    /// Filters out players from very low divisions and those below minimum ability.
    /// National coaches primarily select from top divisions.
    /// Maximum candidate pool size returned to the squad selection stage.
    /// The coach scouts broadly but narrows down to a shortlist.
    const MAX_CANDIDATE_POOL: usize = 60;

    /// Collect eligible candidates from all clubs across all countries, grouped by nationality.
    /// Used at the continent level to find players playing abroad.
    pub(crate) fn collect_all_candidates_by_country(
        countries: &[Country],
        date: NaiveDate,
    ) -> HashMap<u32, Vec<CallUpCandidate>> {
        let mut map: HashMap<u32, Vec<CallUpCandidate>> = HashMap::new();

        for country in countries {
            for club in &country.clubs {
                for team in &club.teams.teams {
                    if team.team_type != TeamType::Main {
                        continue;
                    }

                    let league_rep = team.league_id
                        .map(|_| team.reputation.world)
                        .unwrap_or(0);

                    for player in &team.players.players {
                        if player.player_attributes.is_injured
                            || player.player_attributes.is_banned
                            || player.statuses.get().contains(&PlayerStatusType::Loa)
                            || player.player_attributes.condition < 5000
                        {
                            continue;
                        }

                        if let Some(candidate) = Self::build_candidate(player, club.id, team.id, league_rep, date) {
                            map.entry(player.country_id).or_default().push(candidate);
                        }
                    }
                }
            }
        }

        // Rank and trim each country's pool independently
        for candidates in map.values_mut() {
            let trimmed = Self::rank_and_trim_candidates(std::mem::take(candidates));
            *candidates = trimmed;
        }

        map
    }

    /// Build a CallUpCandidate from a player, if the player is worth scouting.
    /// Only rejects players who have never played a match and have negligible ability.
    fn build_candidate(
        player: &Player,
        club_id: u32,
        team_id: u32,
        league_rep: u16,
        date: NaiveDate,
    ) -> Option<CallUpCandidate> {
        let ability = player.player_attributes.current_ability;
        let potential = player.player_attributes.potential_ability;
        let age = date.year() - player.birth_date.year();
        let total_games = player.statistics.played + player.statistics.played_subs;

        // Skip players who haven't proven themselves at club level:
        // - Must have played at least a few matches this season
        // - Must have reasonable ability (or be a high-potential youth)
        let is_promising_youth = age <= 21 && potential >= 80 && total_games >= 3;
        if total_games < 5 && !is_promising_youth {
            return None;
        }
        if ability < 40 && !is_promising_youth {
            return None;
        }

        let condition_pct =
            (player.player_attributes.condition as f32 / 10000.0) * 100.0;

        let position_levels: Vec<(PlayerPositionType, u8)> = player
            .positions
            .positions
            .iter()
            .map(|pp| (pp.position, pp.level))
            .collect();

        let position_group = player
            .positions
            .positions
            .iter()
            .max_by_key(|p| p.level)
            .map(|p| p.position.position_group())
            .unwrap_or(PlayerFieldPositionGroup::Midfielder);

        Some(CallUpCandidate {
            player_id: player.id,
            club_id,
            team_id,
            current_ability: ability,
            potential_ability: potential,
            age,
            condition_pct,
            match_readiness: player.skills.physical.match_readiness,
            average_rating: player.statistics.average_rating,
            played: total_games,
            international_apps: player.player_attributes.international_apps,
            international_goals: player.player_attributes.international_goals,
            leadership: player.skills.mental.leadership,
            composure: player.skills.mental.composure,
            teamwork: player.skills.mental.teamwork,
            determination: player.skills.mental.determination,
            pressure_handling: player.attributes.pressure,
            world_reputation: player.player_attributes.world_reputation,
            league_reputation: league_rep,
            position_levels,
            position_group,
            goals: player.statistics.goals,
            assists: player.statistics.assists,
            player_of_the_match: player.statistics.player_of_the_match,
            clean_sheets: player.statistics.clean_sheets,
            yellow_cards: player.statistics.yellow_cards,
            red_cards: player.statistics.red_cards,
        })
    }

    /// Rank candidates by a scouting score (ability + reputation + match results)
    /// and trim to MAX_CANDIDATE_POOL. This ensures weaker nations still produce
    /// a full candidate pool with their best available players.
    fn rank_and_trim_candidates(mut candidates: Vec<CallUpCandidate>) -> Vec<CallUpCandidate> {
        if candidates.len() <= Self::MAX_CANDIDATE_POOL {
            return candidates;
        }

        candidates.sort_by(|a, b| {
            let score_a = Self::scouting_score(a);
            let score_b = Self::scouting_score(b);
            score_b.partial_cmp(&score_a).unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates.truncate(Self::MAX_CANDIDATE_POOL);
        candidates
    }

    /// Quick scouting score used to rank candidates before detailed squad selection.
    /// Combines current ability, match performance, and reputation.
    fn scouting_score(c: &CallUpCandidate) -> f32 {
        // Ability: 0-200 scale, primary factor
        let ability = c.current_ability as f32;

        // Match performance: players who play regularly and perform well
        let games_factor = (c.played as f32).min(30.0) / 30.0;
        let rating_factor = if c.average_rating > 0.0 {
            c.average_rating / 10.0
        } else {
            0.4
        };
        let performance = games_factor * 40.0 + rating_factor * 30.0;

        // Reputation: world reputation is a strong signal coaches use
        let reputation = (c.world_reputation.max(0) as f32 / 100.0).min(50.0);

        // Goals and assists boost for proven performers
        let goals_bonus = (c.international_goals as f32).min(20.0);

        // League level: playing in a stronger league is a signal
        let league_bonus = (c.league_reputation as f32 / 50.0).min(20.0);

        // Youth potential bonus: young players with high ceiling
        let youth_bonus = if c.age <= 23 {
            (c.potential_ability as f32 - c.current_ability as f32).max(0.0) * 0.3
        } else {
            0.0
        };

        ability + performance + reputation + goals_bonus + league_bonus + youth_bonus
    }

    /// Call up squad using weighted scoring — considers ability, tactical fit,
    /// form, experience, mentality, and age. Friendly breaks allow more
    /// experimentation; tournament periods favour proven performers.
    pub(crate) fn call_up_squad(
        &mut self,
        own_clubs: &mut [Club],
        candidates: Vec<CallUpCandidate>,
        date: NaiveDate,
        country_id: u32,
        country_ids: &[(u32, String)],
    ) {
        self.squad.clear();
        self.schedule.clear();

        let is_tournament = Self::is_in_tournament_period(date);

        // Score each candidate and select a balanced squad
        let selected_indices =
            Self::select_balanced_squad(&candidates, &self.tactics, is_tournament, country_id);

        for idx in &selected_indices {
            let c = &candidates[*idx];
            self.squad.push(NationalSquadPlayer {
                player_id: c.player_id,
                club_id: c.club_id,
                team_id: c.team_id,
            });
        }

        // If fewer than MIN_REAL_PLAYERS found, generate synthetic squad
        if self.squad.len() < MIN_REAL_PLAYERS {
            self.generate_synthetic_squad(date);
        }

        // Set Int status on called-up players in own clubs
        for club in own_clubs.iter_mut() {
            for team in club.teams.iter_mut() {
                for player in team.players.iter_mut() {
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
                let (opp_id_1, opp_name_1) = Self::random_opponent(country_id, country_ids);
                let (opp_id_2, opp_name_2) = Self::random_opponent(country_id, country_ids);

                self.schedule.push(NationalTeamFixture {
                    date: d1,
                    opponent_country_id: opp_id_1,
                    opponent_country_name: opp_name_1,
                    is_home: true,
                    competition_name: "Friendly".to_string(),
                    match_id: String::new(),
                    result: None,
                });
                self.schedule.push(NationalTeamFixture {
                    date: d2,
                    opponent_country_id: opp_id_2,
                    opponent_country_name: opp_name_2,
                    is_home: false,
                    competition_name: "Friendly".to_string(),
                    match_id: String::new(),
                    result: None,
                });
            }
        }

        debug!(
            "National team {} (country {}) called up {} players ({} from clubs, {} synthetic)",
            self.country_name,
            country_id,
            self.squad.len() + self.generated_squad.len(),
            self.squad.len(),
            self.generated_squad.len()
        );
    }

    /// Score a candidate player for national team selection.
    ///
    /// National team AI selection logic:
    /// - Ability is the primary factor (you must be good enough)
    /// - Playing at a high level (top division) is strongly favored
    /// - World reputation matters (coaches watch famous players)
    /// - Current form/match rating matters more than raw stats
    /// - International experience gives proven reliability
    /// - Age profile: tournaments prefer prime, friendlies prefer youth
    fn score_candidate(
        candidate: &CallUpCandidate,
        tactics: &Tactics,
        is_tournament: bool,
        country_id: u32,
    ) -> f32 {
        // 1. Ability (0-100) — the core factor
        let ability_score = (candidate.current_ability as f32 / 200.0) * 100.0;

        // 2. League level bonus (0-100) — playing in a top league is a major signal
        // Serie A (rep ~800+) = 80-100, Championship (~500) = 50, Serie C (~200) = 20
        // This is the key factor that prevents Serie C players from being selected
        let league_score = (candidate.league_reputation as f32 / 10.0).clamp(0.0, 100.0);

        // 3. Tactical fit — best match to any required position (0-100)
        let required_positions = tactics.positions();
        let tactical_score = required_positions
            .iter()
            .filter_map(|&pos| {
                candidate
                    .position_levels
                    .iter()
                    .find(|(p, _)| *p == pos)
                    .map(|(_, level)| *level as f32)
            })
            .fold(0.0f32, |acc, x| acc.max(x))
            / 20.0
            * 100.0;

        // 4. Form & match readiness (0-100)
        // Heavily penalize players who aren't match-fit or haven't been playing
        let condition_norm = candidate.condition_pct.clamp(0.0, 100.0);
        let readiness_norm = (candidate.match_readiness / 20.0).clamp(0.0, 1.0) * 100.0;
        let rating_norm = if candidate.average_rating > 0.0 {
            (candidate.average_rating / 10.0).clamp(0.0, 1.0) * 100.0
        } else {
            30.0  // No rating = below average assumption
        };
        // Games played this season: 0 games = 0 bonus, 15+ games = full bonus
        let games_norm = (candidate.played as f32).min(15.0) / 15.0 * 100.0;
        let form_score =
            condition_norm * 0.25 + readiness_norm * 0.25 + rating_norm * 0.30 + games_norm * 0.20;

        // 5. Reputation & international experience (0-100)
        // World reputation is on 0-10000 scale — top players are 5000+
        let rep_norm = (candidate.world_reputation.max(0) as f32 / 8000.0).clamp(0.0, 1.0) * 60.0;
        let apps_norm = (candidate.international_apps as f32).min(80.0) / 80.0 * 25.0;
        let goals_bonus = (candidate.international_goals as f32).min(40.0) / 40.0 * 15.0;
        let experience_score = (rep_norm + apps_norm + goals_bonus).min(100.0);

        // 6. Mental & personality (0-100)
        let mental_avg = (candidate.leadership
            + candidate.composure
            + candidate.teamwork
            + candidate.determination
            + candidate.pressure_handling)
            / 5.0;
        let mental_score = (mental_avg / 20.0).clamp(0.0, 1.0) * 100.0;

        // 7. Age profile (0-100)
        let age_score = if is_tournament {
            match candidate.age {
                ..=20 => 40.0,
                21..=23 => 65.0,
                24..=29 => 90.0,
                30..=32 => 75.0,
                33..=35 => 50.0,
                _ => 30.0,
            }
        } else {
            match candidate.age {
                ..=20 => 75.0,
                21..=23 => 85.0,
                24..=29 => 70.0,
                30..=32 => 45.0,
                33..=35 => 30.0,
                _ => 15.0,
            }
        };

        // 8. Season impact — goals, assists, PoM awards, clean sheets (0-100)
        // A striker scoring 15+ goals or a midfielder with 10+ assists stands out.
        // Position-aware: goals matter more for forwards, clean sheets for defenders/GKs.
        let total_games_f = (candidate.played as f32).max(1.0);
        let goals_per_game = candidate.goals as f32 / total_games_f;
        let assists_per_game = candidate.assists as f32 / total_games_f;
        let pom_norm = (candidate.player_of_the_match as f32).min(8.0) / 8.0 * 30.0;
        let discipline_penalty = candidate.red_cards as f32 * 10.0
            + candidate.yellow_cards as f32 * 1.5;

        let impact_score = match candidate.position_group {
            PlayerFieldPositionGroup::Forward => {
                let goal_score = (goals_per_game * 80.0).min(40.0);
                let assist_score = (assists_per_game * 60.0).min(15.0);
                (goal_score + assist_score + pom_norm - discipline_penalty).clamp(0.0, 100.0)
            }
            PlayerFieldPositionGroup::Midfielder => {
                let goal_score = (goals_per_game * 60.0).min(20.0);
                let assist_score = (assists_per_game * 80.0).min(30.0);
                (goal_score + assist_score + pom_norm - discipline_penalty).clamp(0.0, 100.0)
            }
            PlayerFieldPositionGroup::Defender => {
                let cs_per_game = candidate.clean_sheets as f32 / total_games_f;
                let cs_score = (cs_per_game * 80.0).min(35.0);
                let goal_score = (goals_per_game * 50.0).min(10.0);
                (cs_score + goal_score + pom_norm - discipline_penalty).clamp(0.0, 100.0)
            }
            PlayerFieldPositionGroup::Goalkeeper => {
                let cs_per_game = candidate.clean_sheets as f32 / total_games_f;
                let cs_score = (cs_per_game * 100.0).min(45.0);
                (cs_score + pom_norm - discipline_penalty).clamp(0.0, 100.0)
            }
        };

        // 9. Potential (only meaningful in friendlies)
        let potential_score = (candidate.potential_ability as f32 / 200.0) * 100.0;

        // 10. Coach bias — deterministic per country
        let coach_bias = match country_id % 4 {
            0 => (candidate.international_apps as f32).min(80.0) / 80.0 * 5.0,
            1 => if candidate.age <= 24 { 5.0 } else { 0.0 },
            2 => (candidate.world_reputation.max(0) as f32 / 5000.0).clamp(0.0, 1.0) * 5.0,
            _ => (candidate.leadership / 20.0).clamp(0.0, 1.0) * 5.0,
        };

        // Apply context-dependent weights
        // Tournament: proven quality + match fitness + season impact + experience
        // Friendly: experiment with youth + potential, but still need fitness
        let weighted = if is_tournament {
            ability_score * 0.22
                + league_score * 0.08
                + tactical_score * 0.10
                + form_score * 0.18
                + impact_score * 0.15
                + experience_score * 0.12
                + mental_score * 0.08
                + age_score * 0.07
        } else {
            let youth_bonus = if candidate.age <= 23 && candidate.international_apps < 10 {
                5.0
            } else {
                0.0
            };
            ability_score * 0.16
                + league_score * 0.06
                + tactical_score * 0.08
                + form_score * 0.18
                + impact_score * 0.12
                + experience_score * 0.05
                + mental_score * 0.06
                + age_score * 0.08
                + potential_score * 0.11
                + youth_bonus
        };

        weighted + coach_bias
    }

    /// Select a balanced squad respecting positional quotas.
    /// Returns indices into the `candidates` slice.
    fn select_balanced_squad(
        candidates: &[CallUpCandidate],
        tactics: &Tactics,
        is_tournament: bool,
        country_id: u32,
    ) -> Vec<usize> {
        if candidates.is_empty() {
            return Vec::new();
        }

        // Score all candidates
        let mut scored: Vec<(usize, f32)> = candidates
            .iter()
            .enumerate()
            .map(|(idx, c)| {
                (
                    idx,
                    Self::score_candidate(c, tactics, is_tournament, country_id),
                )
            })
            .collect();

        // Determine positional quotas from the tactic
        let [gk_quota, def_quota, mid_quota, fwd_quota] = Self::positional_quotas(tactics);

        let desc =
            |a: &(usize, f32), b: &(usize, f32)| {
                b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
            };

        // Partition by position group and sort each by score descending
        let mut gk: Vec<(usize, f32)> = scored
            .iter()
            .filter(|(i, _)| candidates[*i].position_group == PlayerFieldPositionGroup::Goalkeeper)
            .copied()
            .collect();
        let mut def: Vec<(usize, f32)> = scored
            .iter()
            .filter(|(i, _)| candidates[*i].position_group == PlayerFieldPositionGroup::Defender)
            .copied()
            .collect();
        let mut mid: Vec<(usize, f32)> = scored
            .iter()
            .filter(|(i, _)| candidates[*i].position_group == PlayerFieldPositionGroup::Midfielder)
            .copied()
            .collect();
        let mut fwd: Vec<(usize, f32)> = scored
            .iter()
            .filter(|(i, _)| candidates[*i].position_group == PlayerFieldPositionGroup::Forward)
            .copied()
            .collect();

        gk.sort_by(&desc);
        def.sort_by(&desc);
        mid.sort_by(&desc);
        fwd.sort_by(&desc);

        let mut selected: Vec<usize> = Vec::with_capacity(SQUAD_SIZE);

        // Pick from each positional group up to its quota
        for &(idx, _) in gk.iter().take(gk_quota) {
            selected.push(idx);
        }
        for &(idx, _) in def.iter().take(def_quota) {
            selected.push(idx);
        }
        for &(idx, _) in mid.iter().take(mid_quota) {
            selected.push(idx);
        }
        for &(idx, _) in fwd.iter().take(fwd_quota) {
            selected.push(idx);
        }

        // Fill any remaining slots from the best unselected candidates
        if selected.len() < SQUAD_SIZE {
            scored.sort_by(&desc);
            for &(idx, _) in &scored {
                if selected.len() >= SQUAD_SIZE {
                    break;
                }
                if !selected.contains(&idx) {
                    selected.push(idx);
                }
            }
        }

        selected.truncate(SQUAD_SIZE);
        selected
    }

    /// Positional quotas for a 23-man squad based on the tactic's shape.
    /// Returns [GK, DEF, MID, FWD].
    fn positional_quotas(tactics: &Tactics) -> [usize; 4] {
        let def_count = tactics.defender_count();
        if def_count >= 5 {
            [3, 8, 7, 5]
        } else if def_count == 3 {
            [3, 6, 8, 6]
        } else {
            [3, 7, 7, 6]
        }
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
            friendly_statistics: PlayerStatistics::default(),
            cup_statistics: PlayerStatistics::default(),
            statistics_history: PlayerStatisticsHistory::new(),
            decision_history: PlayerDecisionHistory::new(),
            languages: Vec::new(),
            last_transfer_date: None,
            plan: None,
            favorite_clubs: Vec::new(),
            sold_from: None,
            traits: Vec::new(),
            rapport: PlayerRapport::new(),
            promises: Vec::new(),
            pending_signing: None,
            generated: true,
            load: PlayerLoad::new(),
        }
    }

    /// Build a MatchSquad from the called-up squad + generated players
    pub fn build_match_squad(&self, clubs: &[Club]) -> MatchSquad {
        let club_refs: Vec<&Club> = clubs.iter().collect();
        self.build_match_squad_from_refs(&club_refs)
    }

    /// Build a MatchSquad searching across all provided clubs (including foreign).
    /// This variant accepts refs so the caller can collect clubs from multiple countries.
    pub fn build_match_squad_from_refs(&self, clubs: &[&Club]) -> MatchSquad {
        let team_id = self.country_id;
        let team_name = self.country_name.clone();

        // Collect real players from clubs (may span multiple countries)
        let mut all_players: Vec<&Player> = Vec::new();

        for squad_player in &self.squad {
            for club in clubs.iter() {
                for team in club.teams.iter() {
                    if let Some(player) = team.players.find(squad_player.player_id) {
                        all_players.push(player);
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


    /// Build a synthetic opponent squad for friendly matches
    pub fn build_synthetic_opponent_squad(opponent_country_id: u32, opponent_name: &str) -> MatchSquad {
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
            team_name: opponent_name.to_string(),
            tactics,
            main_squad,
            substitutes,
            captain_id: None,
            vice_captain_id: None,
            penalty_taker_id: None,
            free_kick_taker_id: None,
        }
    }

    /// Remove Int status from players but keep squad data for display
    pub(crate) fn release_player_status(&mut self, clubs: &mut [Club]) {
        let squad_player_ids: Vec<u32> = self.squad.iter().map(|s| s.player_id).collect();
        for club in clubs.iter_mut() {
            for team in club.teams.iter_mut() {
                for player in team.players.iter_mut() {
                    if squad_player_ids.contains(&player.id) {
                        player.statuses.remove(PlayerStatusType::Int);
                    }
                }
            }
        }
    }

    #[allow(dead_code)]
    fn release_squad(&mut self, clubs: &mut [Club]) {
        let squad_player_ids: Vec<u32> = self.squad.iter().map(|s| s.player_id).collect();

        for club in clubs.iter_mut() {
            for team in club.teams.iter_mut() {
                for player in team.players.iter_mut() {
                    if squad_player_ids.contains(&player.id) {
                        player.statuses.remove(PlayerStatusType::Int);
                    }
                }
            }
        }

        debug!(
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

    pub fn is_tournament_start(date: NaiveDate) -> bool {
        date.month() == TOURNAMENT_WINDOW.0 && date.day() == TOURNAMENT_WINDOW.1
    }

    pub fn is_tournament_end(date: NaiveDate) -> bool {
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

    fn random_opponent(exclude_country_id: u32, country_ids: &[(u32, String)]) -> (u32, String) {
        let candidates: Vec<&(u32, String)> = country_ids
            .iter()
            .filter(|(id, _)| *id != exclude_country_id)
            .collect();

        if candidates.is_empty() {
            return (exclude_country_id, String::new());
        }

        let idx = IntegerUtils::random(0, candidates.len() as i32) as usize;
        candidates[idx.min(candidates.len() - 1)].clone()
    }
}

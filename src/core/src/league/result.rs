use crate::club::player::injury::InjuryType;
use crate::league::{LeagueTableResult, ScheduleItem};
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{FieldSquad, GoalDetail, MatchResult, MatchResultRaw, Score, TeamScore};
use crate::simulator::SimulatorData;
use crate::utils::DateUtils;
use crate::{HappinessEventType, MatchHistoryItem, PlayerStatusType, SimulationResult};
use chrono::NaiveDateTime;
use std::collections::HashMap;

pub struct LeagueResult {
    pub league_id: u32,
    pub table_result: LeagueTableResult,
    pub match_results: Option<Vec<MatchResult>>,
    pub new_season_started: bool,
}

impl LeagueResult {
    pub fn new(league_id: u32, table_result: LeagueTableResult) -> Self {
        LeagueResult {
            league_id,
            table_result,
            match_results: None,
            new_season_started: false,
        }
    }

    pub fn with_match_result(
        league_id: u32,
        table_result: LeagueTableResult,
        match_results: Vec<MatchResult>,
    ) -> Self {
        LeagueResult {
            league_id,
            table_result,
            match_results: Some(match_results),
            new_season_started: false,
        }
    }

    pub fn process(self, data: &mut SimulatorData, result: &mut SimulationResult) {
        if let Some(match_results) = self.match_results {
            for mut match_result in match_results {
                Self::process_match_results(&mut match_result, data);

                result.match_results.push(match_result);
            }
        }
    }

    fn process_match_results(result: &mut MatchResult, data: &mut SimulatorData) {
        let now = data.date;

        // Update league schedule (skip for friendlies without a league)
        if let Some(league) = data.league_mut(result.league_id) {
            league.schedule.update_match_result(
                &result.id,
                &result.score,
            );
        }

        let home_team = data.team_mut(result.score.home_team.team_id).unwrap();
        home_team.match_history.add(MatchHistoryItem::new(
            now,
            result.score.home_team.team_id,
            (
                TeamScore::from(&result.score.home_team),
                TeamScore::from(&result.score.away_team),
            ),
        ));

        let away_team = data.team_mut(result.score.away_team.team_id).unwrap();
        away_team.match_history.add(MatchHistoryItem::new(
            now,
            result.score.away_team.team_id,
            (
                TeamScore::from(&result.score.away_team),
                TeamScore::from(&result.score.home_team),
            ),
        ));

        Self::process_match_events(result, data);
    }

    fn process_match_events(result: &mut MatchResult, data: &mut SimulatorData) {
        let details = match &result.details {
            Some(d) => d,
            None => return,
        };

        // Look up friendly flag before mutable borrows
        let is_friendly = data.league(result.league_id)
            .map(|l| l.friendly)
            .unwrap_or(false);

        // Helper macro to select the correct statistics field
        macro_rules! stats {
            ($player:expr) => {
                if is_friendly { &mut $player.friendly_statistics } else { &mut $player.statistics }
            };
        }

        // Mark players as played (main squad) or played_subs (substitutes)
        for player_id in &details.left_team_players.main {
            if let Some(player) = data.player_mut(*player_id) {
                stats!(player).played += 1;
            }
        }
        for player_id in &details.left_team_players.substitutes_used {
            if let Some(player) = data.player_mut(*player_id) {
                stats!(player).played_subs += 1;
            }
        }
        for player_id in &details.right_team_players.main {
            if let Some(player) = data.player_mut(*player_id) {
                stats!(player).played += 1;
            }
        }
        for player_id in &details.right_team_players.substitutes_used {
            if let Some(player) = data.player_mut(*player_id) {
                stats!(player).played_subs += 1;
            }
        }

        // Goals and assists from score details
        for detail in &result.score.details {
            match detail.stat_type {
                MatchStatisticType::Goal => {
                    if let Some(player) = data.player_mut(detail.player_id) {
                        stats!(player).goals += 1;
                    }
                }
                MatchStatisticType::Assist => {
                    if let Some(player) = data.player_mut(detail.player_id) {
                        stats!(player).assists += 1;
                    }
                }
            }
        }

        // Per-player stats (shots, passes, tackles, rating)
        let mut best_rating: f32 = 0.0;
        let mut best_player_id: Option<u32> = None;

        for (player_id, stats_data) in &details.player_stats {
            if let Some(player) = data.player_mut(*player_id) {
                let s = stats!(player);
                s.shots_on_target += stats_data.shots_on_target as f32;
                s.tackling += stats_data.tackles as f32;
                if stats_data.passes_attempted > 0 {
                    let match_pct = (stats_data.passes_completed as f32 / stats_data.passes_attempted as f32 * 100.0) as u8;
                    let games = s.played + s.played_subs;
                    if games <= 1 {
                        s.passes = match_pct;
                    } else {
                        let prev = s.passes as f32;
                        s.passes = ((prev * (games - 1) as f32 + match_pct as f32) / games as f32) as u8;
                    }
                }

                // Update running average rating
                let games = s.played + s.played_subs;
                if games <= 1 {
                    s.average_rating = stats_data.match_rating;
                } else {
                    let prev = s.average_rating;
                    s.average_rating =
                        (prev * (games - 1) as f32 + stats_data.match_rating) / games as f32;
                }

                // Track best rating for player of the match
                if stats_data.match_rating > best_rating {
                    best_rating = stats_data.match_rating;
                    best_player_id = Some(*player_id);
                }
            }
        }

        // Award player of the match
        if let Some(motm_id) = best_player_id {
            if let Some(player) = data.player_mut(motm_id) {
                stats!(player).player_of_the_match += 1;
                player.happiness.add_event(HappinessEventType::PlayerOfTheMatch, 4.0);
            }
        }

        // Goalkeeper stats: conceded goals and clean sheets
        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();
        let home_team_id = result.score.home_team.team_id;

        // Find starting goalkeepers by checking main squad players' positions
        for &gk_id in details.left_team_players.main.iter() {
            if let Some(player) = data.player_mut(gk_id) {
                if player.position().is_goalkeeper() {
                    let goals_against = if details.left_team_players.team_id == home_team_id {
                        away_goals
                    } else {
                        home_goals
                    };
                    stats!(player).conceded += goals_against as u16;
                    if goals_against == 0 {
                        stats!(player).clean_sheets += 1;
                    }
                }
            }
        }
        for &gk_id in details.right_team_players.main.iter() {
            if let Some(player) = data.player_mut(gk_id) {
                if player.position().is_goalkeeper() {
                    let goals_against = if details.right_team_players.team_id == home_team_id {
                        away_goals
                    } else {
                        home_goals
                    };
                    stats!(player).conceded += goals_against as u16;
                    if goals_against == 0 {
                        stats!(player).clean_sheets += 1;
                    }
                }
            }
        }

        // Apply physical effects from match participation (always, regardless of friendly flag)
        Self::apply_post_match_physical_effects(details, data);

        // Update player reputations based on match performance
        let league_reputation = data.league(result.league_id)
            .map(|l| l.reputation)
            .unwrap_or(500) as f32;
        let league_weight = (league_reputation / 1000.0 + 0.5).clamp(0.5, 1.5);

        for (player_id, stats_data) in &details.player_stats {
            let rating_delta = (stats_data.match_rating - 6.0) * 20.0;
            let goal_bonus = stats_data.goals.min(3) as f32 * 15.0;
            let assist_bonus = stats_data.assists.min(3) as f32 * 8.0;
            let motm_bonus = if best_player_id == Some(*player_id) { 25.0 } else { 0.0 };
            let raw_delta = rating_delta + goal_bonus + assist_bonus + motm_bonus;

            if is_friendly {
                let home_delta = (raw_delta * 0.4 * league_weight) as i16;
                if let Some(player) = data.player_mut(*player_id) {
                    player.player_attributes.update_reputation(0, home_delta, 0);
                }
            } else {
                let current_delta = (raw_delta * league_weight) as i16;
                let home_delta = (raw_delta * 0.6 * league_weight) as i16;
                let world_delta = (raw_delta * 0.2 * league_weight) as i16;
                if let Some(player) = data.player_mut(*player_id) {
                    player.player_attributes.update_reputation(current_delta, home_delta, world_delta);
                }
            }
        }

        // Save PoM to match result
        if let Some(details_mut) = &mut result.details {
            details_mut.player_of_the_match_id = best_player_id;
        }
    }

    fn apply_post_match_physical_effects(details: &MatchResultRaw, data: &mut SimulatorData) {
        let now = data.date.date();

        // Build substitution lookup: player_id -> time in ms they were subbed out/in
        let mut subbed_out_at: HashMap<u32, u64> = HashMap::new();
        let mut subbed_in_at: HashMap<u32, u64> = HashMap::new();
        for sub in &details.substitutions {
            subbed_out_at.insert(sub.player_out_id, sub.match_time_ms);
            subbed_in_at.insert(sub.player_in_id, sub.match_time_ms);
        }

        let teams = [&details.left_team_players, &details.right_team_players];

        for team in teams {
            Self::apply_physical_effects_for_team(team, &subbed_out_at, &subbed_in_at, data, now);
        }
    }

    fn apply_physical_effects_for_team(
        team: &FieldSquad,
        subbed_out_at: &HashMap<u32, u64>,
        subbed_in_at: &HashMap<u32, u64>,
        data: &mut SimulatorData,
        now: chrono::NaiveDate,
    ) {
        // Process starters
        for &player_id in &team.main {
            let minutes = if let Some(&out_time_ms) = subbed_out_at.get(&player_id) {
                (out_time_ms / 60000) as f32
            } else {
                90.0
            };

            Self::apply_player_physical_effects(player_id, minutes, data, now);
        }

        // Process used substitutes
        for &player_id in &team.substitutes_used {
            let minutes = if let Some(&in_time_ms) = subbed_in_at.get(&player_id) {
                90.0 - (in_time_ms / 60000) as f32
            } else {
                30.0 // fallback
            };

            Self::apply_player_physical_effects(player_id, minutes, data, now);
        }

        // Process unused substitutes — frustration
        for &player_id in &team.substitutes {
            if !team.substitutes_used.contains(&player_id) {
                if let Some(player) = data.player_mut(player_id) {
                    player.happiness.add_event(HappinessEventType::MatchDropped, -1.5);
                }
            }
        }
    }

    fn apply_player_physical_effects(
        player_id: u32,
        minutes: f32,
        data: &mut SimulatorData,
        now: chrono::NaiveDate,
    ) {
        if let Some(player) = data.player_mut(player_id) {
            let age = DateUtils::age(player.birth_date, now);
            let stamina = player.skills.physical.stamina;
            let natural_fitness = player.skills.physical.natural_fitness;

            // 1. Condition drop
            let base_drop = minutes / 90.0 * 3000.0;
            let age_factor = if age > 30 {
                1.0 + (age as f32 - 30.0) * 0.08
            } else if age < 23 {
                0.9
            } else {
                1.0
            };
            let stamina_factor = 1.5 - (stamina / 20.0);
            let fitness_factor = 1.3 - (natural_fitness / 20.0) * 0.6;

            let mut total_drop = base_drop * age_factor * stamina_factor * fitness_factor;
            // Clamp full-90 equivalent to 1500-4500
            let scale = minutes / 90.0;
            total_drop = total_drop.clamp(1500.0 * scale, 4500.0 * scale);

            player.player_attributes.condition =
                (player.player_attributes.condition - total_drop as i16).max(0);

            // 2. Match readiness boost (minimum 15 minutes)
            if minutes >= 15.0 {
                let readiness_boost = minutes / 90.0 * 3.0;
                player.skills.physical.match_readiness =
                    (player.skills.physical.match_readiness + readiness_boost).min(20.0);
            }

            // 3. Jadedness accumulation
            if minutes > 60.0 {
                player.player_attributes.jadedness += 400;
            } else if minutes >= 30.0 {
                player.player_attributes.jadedness += 200;
            }

            if player.player_attributes.jadedness > 7000 {
                if !player.statuses.get().contains(&PlayerStatusType::Rst) {
                    player.statuses.add(now, PlayerStatusType::Rst);
                }
            }

            // 4. Reset days since last match
            player.player_attributes.days_since_last_match = 0;

            // 5. Match selection morale boost
            player.happiness.add_event(HappinessEventType::MatchSelection, 2.0);

            // 6. Match injury generation (~2.5% per 90 minutes base rate)
            if !player.player_attributes.is_injured {
                let injury_proneness = player.player_attributes.injury_proneness;
                let proneness_modifier = injury_proneness as f32 / 10.0;
                let condition_pct = player.player_attributes.condition_percentage();

                // Base injury chance: 0.5% per 90 minutes, scaled by minutes played
                let mut injury_chance: f32 = 0.005 * (minutes / 90.0);

                // Age >30: +0.1% per year over 30
                if age > 30 {
                    injury_chance += (age as f32 - 30.0) * 0.001;
                }

                // Low condition (<40%): +0.2-0.4%
                if condition_pct < 40 {
                    injury_chance += (40.0 - condition_pct as f32) * 0.0001;
                }

                // High jadedness (>7000): +0.2%
                if player.player_attributes.jadedness > 7000 {
                    injury_chance += 0.002;
                }

                // Low natural fitness (<8): +0.1%
                if natural_fitness < 8.0 {
                    injury_chance += 0.001;
                }

                // Injury proneness multiplier
                injury_chance *= proneness_modifier;

                // Recently recovered: +0.2% recurring injury risk
                if player.player_attributes.last_injury_body_part != 0 {
                    injury_chance += 0.002;
                }

                if rand::random::<f32>() < injury_chance {
                    let injury = InjuryType::random_match_injury(
                        minutes,
                        age,
                        condition_pct,
                        natural_fitness,
                        injury_proneness,
                    );
                    player.player_attributes.set_injury(injury);
                    player.statuses.add(now, PlayerStatusType::Inj);
                }
            }
        }
    }
}

pub struct LeagueMatch {
    pub id: String,

    pub league_id: u32,
    pub league_slug: String,

    pub date: NaiveDateTime,

    pub home_team_id: u32,
    pub away_team_id: u32,

    pub result: Option<LeagueMatchResultResult>,
}

pub struct LeagueMatchResultResult {
    pub home: TeamScore,
    pub away: TeamScore,
    pub details: Vec<GoalDetail>,
}

impl LeagueMatchResultResult {
    pub fn from_score(score: &Score) -> Self {
        LeagueMatchResultResult {
            home: TeamScore::from(&score.home_team),
            away: TeamScore::from(&score.away_team),
            details: score.detail().to_vec(),
        }
    }
}

impl From<ScheduleItem> for LeagueMatch {
    fn from(item: ScheduleItem) -> Self {
        let mut result = LeagueMatch {
            id: item.id.clone(),
            league_id: item.league_id,
            league_slug: item.league_slug,
            date: item.date,
            home_team_id: item.home_team_id,
            away_team_id: item.away_team_id,
            result: None,
        };

        if let Some(res) = item.result {
            result.result = Some(LeagueMatchResultResult::from_score(&res));
        }

        result
    }
}

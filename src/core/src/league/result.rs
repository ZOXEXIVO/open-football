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
}

impl LeagueResult {
    pub fn new(league_id: u32, table_result: LeagueTableResult) -> Self {
        LeagueResult {
            league_id,
            table_result,
            match_results: None,
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
        }
    }

    pub fn process(self, data: &mut SimulatorData, result: &mut SimulationResult) {
        if let Some(match_results) = self.match_results {
            for match_result in match_results {
                Self::process_match_results(&match_result, data);

                result.match_results.push(match_result);
            }
        }
    }

    fn process_match_results(result: &MatchResult, data: &mut SimulatorData) {
        let now = data.date;

        let league = data.league_mut(result.league_id).unwrap();

        league.schedule.update_match_result(
            &result.id,
            &result.score,
        );

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

    fn process_match_events(result: &MatchResult, data: &mut SimulatorData) {
        let details = match &result.details {
            Some(d) => d,
            None => return,
        };

        // Mark players as played (main squad) or played_subs (substitutes)
        for player_id in &details.left_team_players.main {
            if let Some(player) = data.player_mut(*player_id) {
                player.statistics.played += 1;
            }
        }
        for player_id in &details.left_team_players.substitutes_used {
            if let Some(player) = data.player_mut(*player_id) {
                player.statistics.played_subs += 1;
            }
        }
        for player_id in &details.right_team_players.main {
            if let Some(player) = data.player_mut(*player_id) {
                player.statistics.played += 1;
            }
        }
        for player_id in &details.right_team_players.substitutes_used {
            if let Some(player) = data.player_mut(*player_id) {
                player.statistics.played_subs += 1;
            }
        }

        // Goals and assists from score details
        for detail in &result.score.details {
            match detail.stat_type {
                MatchStatisticType::Goal => {
                    if let Some(player) = data.player_mut(detail.player_id) {
                        player.statistics.goals += 1;
                    }
                }
                MatchStatisticType::Assist => {
                    if let Some(player) = data.player_mut(detail.player_id) {
                        player.statistics.assists += 1;
                    }
                }
            }
        }

        // Per-player stats (shots, passes, tackles, rating)
        let mut best_rating: f32 = 0.0;
        let mut best_player_id: Option<u32> = None;

        for (player_id, stats) in &details.player_stats {
            if let Some(player) = data.player_mut(*player_id) {
                player.statistics.shots_on_target += stats.shots_on_target as f32;
                player.statistics.tackling += stats.tackles as f32;
                if stats.passes_attempted > 0 {
                    let match_pct = (stats.passes_completed as f32 / stats.passes_attempted as f32 * 100.0) as u8;
                    let games = player.statistics.played + player.statistics.played_subs;
                    if games <= 1 {
                        player.statistics.passes = match_pct;
                    } else {
                        let prev = player.statistics.passes as f32;
                        player.statistics.passes = ((prev * (games - 1) as f32 + match_pct as f32) / games as f32) as u8;
                    }
                }

                // Update running average rating
                let games = player.statistics.played + player.statistics.played_subs;
                if games <= 1 {
                    player.statistics.average_rating = stats.match_rating;
                } else {
                    let prev = player.statistics.average_rating;
                    player.statistics.average_rating =
                        (prev * (games - 1) as f32 + stats.match_rating) / games as f32;
                }

                // Track best rating for player of the match
                if stats.match_rating > best_rating {
                    best_rating = stats.match_rating;
                    best_player_id = Some(*player_id);
                }
            }
        }

        // Award player of the match
        if let Some(motm_id) = best_player_id {
            if let Some(player) = data.player_mut(motm_id) {
                player.statistics.player_of_the_match += 1;
            }
        }

        // Apply physical effects from match participation
        Self::apply_post_match_physical_effects(details, data);
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

use crate::continent::competitions::{CHAMPIONS_LEAGUE_ID, EUROPA_LEAGUE_ID, CONFERENCE_LEAGUE_ID};
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::MatchResult;
use crate::simulator::SimulatorData;
use crate::HappinessEventType;
use super::LeagueResult;

impl LeagueResult {
    pub(super) fn process_match_events(result: &mut MatchResult, data: &mut SimulatorData) {
        let details = match &result.details {
            Some(d) => d,
            None => return,
        };

        // Look up match type flags before mutable borrows
        // Continental cup matches (CL/EL/etc.) use a reserved ID range starting at 900_000_000
        let is_cup = result.league_id >= 900_000_000;
        let is_friendly = if is_cup {
            false
        } else {
            data.league(result.league_id)
                .map(|l| l.friendly)
                .unwrap_or(false)
        };

        // Helper macro to select the correct statistics field
        macro_rules! stats {
            ($player:expr) => {
                if is_cup { &mut $player.cup_statistics }
                else if is_friendly { &mut $player.friendly_statistics }
                else { &mut $player.statistics }
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
        //
        // Continental competitions (CL/EL/Conference) use reserved league_id >= 900_000_000:
        //   Champions League:    900_000_001
        //   Europa League:       900_000_002
        //   Conference League:   900_000_003
        //
        // These get special reputation weights — especially for world reputation,
        // since playing in European competition is the primary driver of global recognition.
        let (league_weight, world_weight) = if result.league_id == CHAMPIONS_LEAGUE_ID {
            // Champions League: highest prestige, massive world reputation boost
            (1.5, 1.2)
        } else if result.league_id == EUROPA_LEAGUE_ID {
            // Europa League: high prestige
            (1.3, 0.8)
        } else if result.league_id == CONFERENCE_LEAGUE_ID {
            // Conference League: moderate prestige
            (1.1, 0.5)
        } else if is_cup {
            // Other cup competitions
            (1.0, 0.3)
        } else {
            let league_reputation = data.league(result.league_id)
                .map(|l| l.reputation)
                .unwrap_or(500) as f32;
            let w = (league_reputation / 1000.0 + 0.5).clamp(0.5, 1.5);
            (w, 0.2)
        };

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
                let world_delta = (raw_delta * world_weight * league_weight) as i16;
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
}

//! Post-match world-wide writes: caps/goals/reputation, Elo, schedule.
//!
//! Each helper walks every continent so foreign-based squad members
//! receive their stat bumps regardless of where the match was played
//! or which competition it belonged to. Reused by both the
//! continental qualifier orchestrator and the global tournament
//! processor — single source of truth for "what happens after a
//! national-team match".

use chrono::NaiveDate;
use std::collections::HashMap;

use super::lookups::world_country_elo;
use crate::continent::Continent;
use crate::country::national_team::{NationalTeamFixture, NationalTeamMatchResult};

/// Update apps/goals/reputation for every player on either side's
/// squad, no matter which continent their club sits on.
///
/// Country-weighted reputation gains: stronger nations push the
/// reputation needle further (a goal at a World Cup for Brazil counts
/// for more than the same goal for a Tier-3 nation), bounded at 0.5x
/// to 2.0x so the curve doesn't explode at extremes.
pub fn apply_world_international_stats(
    continents: &mut [Continent],
    home_country_id: u32,
    away_country_id: u32,
    player_goals: &HashMap<u32, u16>,
) {
    let mut squad_player_ids: Vec<u32> = Vec::new();
    let mut country_weights: HashMap<u32, f32> = HashMap::new();

    for continent in continents.iter() {
        for country in continent.countries.iter() {
            if country.id != home_country_id && country.id != away_country_id {
                continue;
            }
            let country_rep = country.reputation as f32;
            let country_weight = (country_rep / 500.0).clamp(0.5, 2.0);
            for s in &country.national_team.squad {
                squad_player_ids.push(s.player_id);
                country_weights.insert(s.player_id, country_weight);
            }
        }
    }

    for continent in continents.iter_mut() {
        for country in continent.countries.iter_mut() {
            for club in country.clubs.iter_mut() {
                for team in club.teams.iter_mut() {
                    for player in team.players.iter_mut() {
                        if !squad_player_ids.contains(&player.id) {
                            continue;
                        }
                        let country_weight =
                            country_weights.get(&player.id).copied().unwrap_or(1.0);

                        player.player_attributes.international_apps += 1;

                        let mut goal_bonus: f32 = 0.0;
                        if let Some(&goals) = player_goals.get(&player.id) {
                            player.player_attributes.international_goals += goals;
                            goal_bonus = goals.min(3) as f32 * 20.0;
                        }

                        let base = 15.0;
                        let raw = base + goal_bonus;
                        let current_delta = (raw * 0.6 * country_weight) as i16;
                        let home_delta = (raw * 0.8 * country_weight) as i16;
                        let world_delta = (raw * 1.0 * country_weight) as i16;

                        player.player_attributes.update_reputation(
                            current_delta,
                            home_delta,
                            world_delta,
                        );
                    }
                }
            }
        }
    }
}

/// Update Elo for both countries after a national-team match.
/// Operates across the entire world so a continental qualifier and a
/// global tournament both go through the same Elo path.
pub fn apply_world_elo(
    continents: &mut [Continent],
    home_country_id: u32,
    away_country_id: u32,
    home_score: u8,
    away_score: u8,
) {
    let home_elo = world_country_elo(continents, home_country_id);
    let away_elo = world_country_elo(continents, away_country_id);

    for continent in continents.iter_mut() {
        for country in continent.countries.iter_mut() {
            if country.id == home_country_id {
                country
                    .national_team
                    .update_elo(home_score, away_score, away_elo);
            } else if country.id == away_country_id {
                country
                    .national_team
                    .update_elo(away_score, home_score, home_elo);
            }
        }
    }
}

/// Push a fixture entry into each country's national-team schedule so
/// the web layer can render the match in both teams' fixture lists.
#[allow(clippy::too_many_arguments)]
pub fn record_world_country_schedule(
    continents: &mut [Continent],
    date: NaiveDate,
    home_country_id: u32,
    away_country_id: u32,
    home_name: &str,
    away_name: &str,
    home_score: u8,
    away_score: u8,
    competition_name: &str,
    match_id: &str,
) {
    for continent in continents.iter_mut() {
        for country in continent.countries.iter_mut() {
            if country.id == home_country_id {
                country.national_team.schedule.push(NationalTeamFixture {
                    date,
                    opponent_country_id: away_country_id,
                    opponent_country_name: away_name.to_string(),
                    is_home: true,
                    competition_name: competition_name.to_string(),
                    match_id: match_id.to_string(),
                    result: Some(NationalTeamMatchResult {
                        home_score,
                        away_score,
                        date,
                        opponent_country_id: away_country_id,
                    }),
                });
            } else if country.id == away_country_id {
                country.national_team.schedule.push(NationalTeamFixture {
                    date,
                    opponent_country_id: home_country_id,
                    opponent_country_name: home_name.to_string(),
                    is_home: false,
                    competition_name: competition_name.to_string(),
                    match_id: match_id.to_string(),
                    result: Some(NationalTeamMatchResult {
                        home_score: away_score,
                        away_score: home_score,
                        date,
                        opponent_country_id: home_country_id,
                    }),
                });
            }
        }
    }
}

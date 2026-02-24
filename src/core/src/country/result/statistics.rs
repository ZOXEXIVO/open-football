use chrono::Datelike;
use log::info;
use std::collections::HashMap;
use super::CountryResult;
use crate::league::Season;
use crate::simulator::SimulatorData;

impl CountryResult {
    /// Snapshot all player statistics into history when a new season starts.
    /// Called from result processing AFTER match stats have been applied,
    /// so all player statistics are up-to-date.
    pub(super) fn snapshot_player_season_statistics(data: &mut SimulatorData, country_id: u32) {
        let date = data.date.date();

        // Season that just ended: if we're in Aug+, the season was the previous year
        let season_year = date.year() as u16 - 1;
        let season = Season::new(season_year);

        info!("📋 New season snapshot: saving player statistics for season {} (country {})", season_year, country_id);

        let country = match data.country_mut(country_id) {
            Some(c) => c,
            None => return,
        };

        // Build league lookup so we can resolve team.league_id -> (name, slug)
        let league_lookup: HashMap<u32, (String, String)> = country.leagues.leagues.iter()
            .map(|l| (l.id, (l.name.clone(), l.slug.clone())))
            .collect();

        for club in &mut country.clubs {
            // Find main team's league as fallback for youth/reserve teams without league_id
            let main_team_league = club.teams.teams.iter()
                .find(|t| t.team_type == crate::TeamType::Main)
                .and_then(|t| t.league_id)
                .and_then(|lid| league_lookup.get(&lid))
                .cloned()
                .unwrap_or_default();

            for team in &mut club.teams.teams {
                let team_name = team.name.clone();
                let team_slug = team.slug.clone();

                let (league_name, league_slug) = team.league_id
                    .and_then(|lid| league_lookup.get(&lid))
                    .cloned()
                    .unwrap_or_else(|| main_team_league.clone());

                for player in &mut team.players.players {
                    player.snapshot_season_statistics(
                        season.clone(),
                        &team_name,
                        &team_slug,
                        &league_name,
                        &league_slug,
                        date,
                    );
                }
            }
        }
    }
}

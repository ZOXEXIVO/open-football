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

        // Season that just ended — use the canonical Season::from_date calculation.
        // The snapshot fires when the NEW season starts (Aug+), so we need the
        // PREVIOUS season: subtract one year from the from_date result.
        let current_season = Season::from_date(date);
        let ended_season = Season::new(current_season.start_year.saturating_sub(1));

        info!("📋 New season snapshot: saving player statistics for season {} (country {})", ended_season.start_year, country_id);

        let country = match data.country_mut(country_id) {
            Some(c) => c,
            None => return,
        };

        // Build league lookup so we can resolve team.league_id -> (name, slug)
        let league_lookup: HashMap<u32, (String, String)> = country.leagues.leagues.iter()
            .map(|l| (l.id, (l.name.clone(), l.slug.clone())))
            .collect();

        for club in &mut country.clubs {
            // Get main team info — used for all teams in player history
            // so history always shows "Juventus" instead of "Juventus B" / "Juventus U20"
            let main_team_info: Option<(String, String, u16)> = club.teams.teams.iter()
                .find(|t| t.team_type == crate::TeamType::Main)
                .map(|t| (t.name.clone(), t.slug.clone(), t.reputation.world));

            let main_team_league = club.teams.teams.iter()
                .find(|t| t.team_type == crate::TeamType::Main)
                .and_then(|t| t.league_id)
                .and_then(|lid| league_lookup.get(&lid))
                .cloned()
                .unwrap_or_default();

            for team in &mut club.teams.teams {
                // Always use main team info in history (show club name, not sub-team)
                let (team_name, team_slug, team_reputation) = match (&team.team_type, &main_team_info) {
                    (crate::TeamType::Main, _) | (_, None) => {
                        (team.name.clone(), team.slug.clone(), team.reputation.world)
                    }
                    (_, Some((name, slug, rep))) => {
                        (name.clone(), slug.clone(), *rep)
                    }
                };

                // Always use main team's league in history
                let (league_name, league_slug) = main_team_league.clone();

                for player in &mut team.players.players {
                    player.snapshot_season_statistics(
                        ended_season.clone(),
                        &team_name,
                        &team_slug,
                        team_reputation,
                        &league_name,
                        &league_slug,
                        date,
                    );
                }
            }
        }
    }
}

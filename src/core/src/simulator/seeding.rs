use crate::club::Team;
use crate::{Club, Country, TeamInfo};
use std::collections::HashMap;

/// Collect every team id that participates in `league_id` across the
/// given clubs. Extracted so `init_league_tables`'s outer loop reads as
/// "for each league, install a table from the team ids" without an inline
/// `flat_map` chain.
pub(super) fn team_ids_for_league(clubs: &[Club], league_id: u32) -> Vec<u32> {
    clubs
        .iter()
        .flat_map(|c| c.teams.with_league(league_id))
        .collect()
}

/// Per-country `league_id -> (name, slug)` cache. Built once at the start
/// of a country's seeding sweep so the per-club main-team lookup is O(1).
pub(super) fn build_league_lookup(country: &Country) -> HashMap<u32, (String, String)> {
    country
        .leagues
        .leagues
        .iter()
        .map(|l| (l.id, (l.name.clone(), l.slug.clone())))
        .collect()
}

/// True if any team in the club has at least one player needing a current-
/// season seed entry. Cheap traversal — exits as soon as one is found.
pub(super) fn club_has_players_needing_seed(club: &Club) -> bool {
    club.teams.iter().any(|t| team_has_players_needing_seed(t))
}

pub(super) fn team_has_players_needing_seed(team: &Team) -> bool {
    team.players
        .iter()
        .any(|p| p.statistics_history.needs_current_season_seed())
}

/// Snapshot of the club's main-team identity for stats-seeding purposes.
/// Resolved once per club so youth teams (U18-U23) and Reserve inherit
/// the main brand consistently across all their players. Senior reserves
/// (B, Second) keep their own identity because they compete in real
/// lower divisions and players' histories should show that.
pub(super) struct ClubSeedingContext {
    main_name: Option<String>,
    main_slug: Option<String>,
    main_reputation: u16,
    main_league_name: String,
    main_league_slug: String,
    league_lookup: HashMap<u32, (String, String)>,
}

impl ClubSeedingContext {
    pub(super) fn resolve(club: &Club, league_lookup: &HashMap<u32, (String, String)>) -> Self {
        let main_team = club.teams.main();
        let main_name = main_team.map(|t| t.name.clone());
        let main_slug = main_team.map(|t| t.slug.clone());
        let main_reputation = main_team.map(|t| t.reputation.world).unwrap_or(0);
        let (main_league_name, main_league_slug) = main_team
            .and_then(|t| t.league_id)
            .and_then(|lid| league_lookup.get(&lid))
            .map(|(n, s)| (n.clone(), s.clone()))
            .unwrap_or_default();
        ClubSeedingContext {
            main_name,
            main_slug,
            main_reputation,
            main_league_name,
            main_league_slug,
            league_lookup: league_lookup.clone(),
        }
    }

    /// Build the `TeamInfo` that the seeder writes onto the player's
    /// history. Main, B and Second teams keep their own identity (each
    /// competes in a real league); youth and Reserve squads inherit the
    /// main brand so the player always has a "career home" row pointing
    /// at the parent club's main team — even if they only ever play for
    /// a non-owning squad.
    pub(super) fn team_info_for(&self, team: &Team) -> TeamInfo {
        let keeps_own_identity = team.team_type.is_own_team();
        if keeps_own_identity {
            let (league_name, league_slug) = team
                .league_id
                .and_then(|lid| self.league_lookup.get(&lid))
                .cloned()
                .unwrap_or_else(|| (self.main_league_name.clone(), self.main_league_slug.clone()));
            TeamInfo {
                name: team.name.clone(),
                slug: team.slug.clone(),
                reputation: team.reputation.world,
                league_name,
                league_slug,
            }
        } else if self.main_name.is_some() {
            TeamInfo {
                name: self.main_name.clone().unwrap_or_default(),
                slug: self.main_slug.clone().unwrap_or_default(),
                reputation: self.main_reputation,
                league_name: self.main_league_name.clone(),
                league_slug: self.main_league_slug.clone(),
            }
        } else {
            // Club has no main team at all — fall back to the team's own info.
            TeamInfo {
                name: team.name.clone(),
                slug: team.slug.clone(),
                reputation: team.reputation.world,
                league_name: self.main_league_name.clone(),
                league_slug: self.main_league_slug.clone(),
            }
        }
    }
}

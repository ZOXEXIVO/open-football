use crate::club::Team;
use crate::{Club, Country, TeamInfo};
use std::collections::HashMap;

/// Which badge a squad's career history hangs under.
///
/// Resolved once per club so youth teams (U18-U23) and Reserve inherit
/// the main brand consistently across all their players. Senior reserves
/// (B, Second) keep their own identity because they compete in real
/// lower divisions and players' histories should show that.
pub struct ClubIdentity {
    main_name: Option<String>,
    main_slug: Option<String>,
    main_reputation: u16,
    main_league_name: String,
    main_league_slug: String,
    league_lookup: HashMap<u32, (String, String)>,
}

impl ClubIdentity {
    /// Per-country `league_id -> (name, slug)` cache. Built once at the
    /// start of a country's sweep so the per-club main-team lookup is O(1).
    pub fn league_lookup(country: &Country) -> HashMap<u32, (String, String)> {
        country
            .leagues
            .leagues
            .iter()
            .map(|l| (l.id, (l.name.clone(), l.slug.clone())))
            .collect()
    }

    pub fn resolve(club: &Club, league_lookup: &HashMap<u32, (String, String)>) -> Self {
        let main_team = club.teams.main();
        let main_name = main_team.map(|t| t.name.clone());
        let main_slug = main_team.map(|t| t.slug.clone());
        let main_reputation = main_team.map(|t| t.reputation.world).unwrap_or(0);
        let (main_league_name, main_league_slug) = main_team
            .and_then(|t| t.league_id)
            .and_then(|lid| league_lookup.get(&lid))
            .map(|(n, s)| (n.clone(), s.clone()))
            .unwrap_or_default();
        ClubIdentity {
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
    pub fn team_info_for(&self, team: &Team) -> TeamInfo {
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

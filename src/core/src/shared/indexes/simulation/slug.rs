use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Clone, Default, PartialEq)]
pub struct SlugIndexes {
    country_slug_index: FxHashMap<String, u32>,
    league_slug_index: FxHashMap<String, u32>,
    team_slug_index: FxHashMap<String, u32>,
}

impl SlugIndexes {
    pub(super) fn add_country_slug(&mut self, slug: &str, country_id: u32) {
        self.country_slug_index.insert(slug.into(), country_id);
    }
    pub fn get_country_by_slug(&self, slug: &str) -> Option<u32> {
        self.country_slug_index.get(slug).copied()
    }

    pub(super) fn add_league_slug(&mut self, slug: &str, league_id: u32) {
        self.league_slug_index.insert(slug.into(), league_id);
    }
    pub fn get_league_by_slug(&self, slug: &str) -> Option<u32> {
        self.league_slug_index.get(slug).copied()
    }

    pub(super) fn add_team_slug(&mut self, slug: &str, team_id: u32) {
        self.team_slug_index.insert(slug.into(), team_id);
    }
    pub fn get_team_by_slug(&self, slug: &str) -> Option<u32> {
        self.team_slug_index.get(slug).copied()
    }

    /// Keep only the slugs of countries, leagues and teams still in the world.
    pub(super) fn retain(
        &mut self,
        countries: &FxHashSet<u32>,
        leagues: &FxHashSet<u32>,
        teams: &FxHashSet<u32>,
    ) {
        self.country_slug_index
            .retain(|_, id| countries.contains(id));
        self.league_slug_index.retain(|_, id| leagues.contains(id));
        self.team_slug_index.retain(|_, id| teams.contains(id));
    }
}

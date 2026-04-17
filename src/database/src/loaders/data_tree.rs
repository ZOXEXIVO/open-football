//! Pulls leagues, clubs, and country-name pools out of the compiled database
//! and resolves path-derived `country_id` from each record's baked-in
//! `country_code`.

use super::club::ClubEntity;
use super::compiled::{compiled, country_id_for_code};
use super::country::CountryEntity;
use super::league::LeagueEntity;
use super::names::NamesByCountryEntity;

pub struct DataTreeResult {
    pub leagues: Vec<LeagueEntity>,
    pub clubs: Vec<ClubEntity>,
    pub names_by_country: Vec<NamesByCountryEntity>,
}

pub struct DataTreeLoader;

impl DataTreeLoader {
    /// Build leagues/clubs/names_by_country from the embedded compiled DB.
    /// Disabled leagues and their clubs are filtered out to match the prior
    /// on-disk loader's behaviour.
    ///
    /// The `countries` parameter is accepted for backward compatibility but
    /// unused — id resolution uses the compiled doc's own country list.
    pub fn load(_countries: &[CountryEntity]) -> DataTreeResult {
        let db = compiled();

        let mut leagues: Vec<LeagueEntity> = db
            .leagues
            .iter()
            .filter(|l| l.enabled)
            .cloned()
            .map(|mut l| {
                l.country_id = country_id_for_code(&l.country_code);
                l
            })
            .collect();

        // Which league ids survived filtering — clubs of disabled leagues get dropped.
        let enabled_ids: std::collections::HashSet<u32> =
            leagues.iter().map(|l| l.id).collect();

        let mut clubs: Vec<ClubEntity> = db
            .clubs
            .iter()
            .cloned()
            .filter_map(|mut c| {
                // Keep clubs whose Main team points at an enabled league.
                let main_league_id = c
                    .teams
                    .iter()
                    .find(|t| t.team_type == "Main")
                    .and_then(|t| t.league_id);
                match main_league_id {
                    Some(lid) if enabled_ids.contains(&lid) => {
                        c.country_id = country_id_for_code(&c.country_code);
                        Some(c)
                    }
                    _ => None,
                }
            })
            .collect();

        let names_by_country: Vec<NamesByCountryEntity> = db
            .names
            .iter()
            .cloned()
            .map(|mut n| {
                n.country_id = country_id_for_code(&n.country_code);
                n
            })
            .collect();

        leagues.sort_by_key(|l| l.id);
        clubs.sort_by_key(|c| c.id);

        DataTreeResult { leagues, clubs, names_by_country }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loaders::country::CountryLoader;

    #[test]
    fn embedded_tree_loads_leagues_and_clubs() {
        let countries = CountryLoader::load();
        let tree = DataTreeLoader::load(&countries);
        // Snapshot counts of enabled leagues and their clubs in the compiled data.
        assert_eq!(tree.leagues.len(), 65, "enabled league count changed");
        assert_eq!(tree.clubs.len(), 999, "enabled club count changed");
    }
}

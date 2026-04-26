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
        assert_eq!(tree.leagues.len(), 73, "enabled league count changed");
        assert_eq!(tree.clubs.len(), 1076, "enabled club count changed");
    }

    #[test]
    fn russian_b_team_satellites_fold_into_parents() {
        let countries = CountryLoader::load();
        let tree = DataTreeLoader::load(&countries);

        // Satellite directories under russian-second-division-* should not
        // appear as standalone clubs — they're folded into their parents.
        let satellite_ids: &[u32] = &[
            58126843, 479504, 2000032541, 8066991, 58098003, 495359, 476302,
            58135242, 8064339,
        ];
        for sid in satellite_ids {
            assert!(
                !tree.clubs.iter().any(|c| c.id == *sid),
                "satellite club {} leaked into clubs list",
                sid
            );
        }

        // Mapping: (parent_id, expected sub-team team_type, enclosing league id).
        // Ural alone uses "B" because Ural's club.json predeclares a hand-named
        // B slot ("ural-b"); every other parent gets "Second", the canonical
        // "{Club} 2" reserve type.
        let expected: &[(u32, &str, u32)] = &[
            (1533, "Second", 2000272306),     // Ural   → russian-second-division-b-group-4
            (1520, "Second", 2000272298),     // Dinamo Moscow → russian-second-division-a-gold
            (58126754, "Second", 2000272300), // Rodina → russian-second-division-a-silver
            (1301106, "Second", 2000272303),  // Baltika → russian-second-division-b-group-2
            (1529, "Second", 2000272303),     // Spartak Moscow → russian-second-division-b-group-2
            (1301108, "Second", 2000272303), // Zenit  → russian-second-division-b-group-2
            (130501, "Second", 2000272305),   // Arsenal Tula → russian-second-division-b-group-3
            (58127493, "Second", 2000272306), // Orenburg → russian-second-division-b-group-4
            (130509, "Second", 2000272306),   // Rubin  → russian-second-division-b-group-4
        ];
        let enabled_league_ids: std::collections::HashSet<u32> =
            tree.leagues.iter().map(|l| l.id).collect();

        for (parent_id, want_type, want_league) in expected {
            let parent = tree
                .clubs
                .iter()
                .find(|c| c.id == *parent_id)
                .unwrap_or_else(|| panic!("parent club {} missing", parent_id));
            let sub_team = parent
                .teams
                .iter()
                .find(|t| t.team_type == *want_type)
                .unwrap_or_else(|| {
                    panic!(
                        "parent club {} has no {} team after satellite merge",
                        parent_id, want_type
                    )
                });
            let league_id = sub_team.league_id.unwrap_or_else(|| {
                panic!(
                    "{} team on parent {} has no league_id stamped",
                    want_type, parent_id
                )
            });
            assert_eq!(
                league_id, *want_league,
                "{} team on parent {} stamped with wrong league",
                want_type, parent_id
            );
            assert!(
                enabled_league_ids.contains(&league_id),
                "{} team on parent {} points at non-enabled league {}",
                want_type, parent_id, league_id
            );
        }
    }

    #[test]
    fn real_sociedad_b_folds_into_real_sociedad() {
        let countries = CountryLoader::load();
        let tree = DataTreeLoader::load(&countries);

        // The Real Sociedad B satellite (id 1743) must not appear standalone.
        assert!(
            !tree.clubs.iter().any(|c| c.id == 1743),
            "Real Sociedad B leaked into clubs list"
        );

        // Real Sociedad parent must now own a B sub-team stamped with the
        // Spanish second division league id.
        let parent = tree
            .clubs
            .iter()
            .find(|c| c.id == 1742)
            .expect("Real Sociedad parent missing");
        let b = parent
            .teams
            .iter()
            .find(|t| t.team_type == "B")
            .expect("Real Sociedad has no B team after satellite merge");
        assert_eq!(b.id, 1743, "B team kept the satellite's id");
        assert_eq!(b.name, "Real Sociedad B");
        assert_eq!(b.slug, "real-sociedad-b");
        assert_eq!(b.league_id, Some(91));
    }
}

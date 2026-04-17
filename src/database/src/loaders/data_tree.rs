use include_dir::{include_dir, Dir};

use super::club::ClubEntity;
use super::country::CountryEntity;
use super::league::LeagueEntity;
use super::names::NamesByCountryEntity;

/// Embedded data directory tree:
///   data/{country_code}/names.json
///   data/{country_code}/{league_slug}/league.json
///   data/{country_code}/{league_slug}/{club_slug}/club.json
///   data/{country_code}/{league_slug}/{club_slug}/players/*.json  (optional, populated later)
static DATA_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/src/data");

pub struct DataTreeResult {
    pub leagues: Vec<LeagueEntity>,
    pub clubs: Vec<ClubEntity>,
    pub names_by_country: Vec<NamesByCountryEntity>,
}

pub struct DataTreeLoader;

impl DataTreeLoader {
    /// Load all leagues, clubs and names from the tree structure.
    /// country_id, league_id are populated from directory paths.
    pub fn load(countries: &[CountryEntity]) -> DataTreeResult {
        let mut leagues = Vec::new();
        let mut clubs = Vec::new();
        let mut names_by_country = Vec::new();

        // Walk data/{country_code}/ directories
        for country_dir in DATA_DIR.dirs() {
            let country_code = country_dir.path().file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");

            let country_id = match countries.iter().find(|c| c.code == country_code) {
                Some(c) => c.id,
                None => continue,
            };

            // Load names.json if present
            if let Some(names_file) = country_dir.get_file(country_dir.path().join("names.json")) {
                if let Some(json) = names_file.contents_utf8() {
                    match serde_json::from_str::<NamesByCountryEntity>(json) {
                        Ok(mut names) => {
                            names.country_id = country_id;
                            names_by_country.push(names);
                        }
                        Err(e) => eprintln!("Failed to parse {}/names.json: {}", country_code, e),
                    }
                }
            }

            // Walk data/{country_code}/{league_slug}/ directories
            for league_dir in country_dir.dirs() {
                let league_slug = league_dir.path().file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");

                // Load league.json
                let league_file = match league_dir.get_file(league_dir.path().join("league.json")) {
                    Some(f) => f,
                    None => continue,
                };

                let league_json = match league_file.contents_utf8() {
                    Some(s) => s,
                    None => continue,
                };

                let mut league: LeagueEntity = match serde_json::from_str(league_json) {
                    Ok(l) => l,
                    Err(e) => {
                        eprintln!("Failed to parse league {}/{}/league.json: {}", country_code, league_slug, e);
                        continue;
                    }
                };
                league.country_id = country_id;

                if !league.enabled {
                    continue; // Skip disabled leagues and their clubs
                }

                let league_id = league.id;
                leagues.push(league);

                // Each club lives in its own subdirectory: {club_slug}/club.json
                for club_dir in league_dir.dirs() {
                    let club_slug = club_dir.path().file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");

                    let club_file = match club_dir.get_file(club_dir.path().join("club.json")) {
                        Some(f) => f,
                        None => continue,
                    };

                    let club_json = match club_file.contents_utf8() {
                        Some(s) => s,
                        None => continue,
                    };

                    let mut club: ClubEntity = match serde_json::from_str(club_json) {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("Failed to parse club {}/{}/{}/club.json: {}", country_code, league_slug, club_slug, e);
                            continue;
                        }
                    };

                    club.country_id = country_id;

                    // Set league_id on the Main team (others stay None)
                    for team in &mut club.teams {
                        if team.team_type == "Main" {
                            team.league_id = Some(league_id);
                        }
                    }

                    clubs.push(club);
                }
            }
        }

        leagues.sort_by_key(|l| l.id);

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
        // Snapshot counts of enabled leagues and their clubs in the embedded data.
        assert_eq!(tree.leagues.len(), 63, "enabled league count changed");
        assert_eq!(tree.clubs.len(), 969, "enabled club count changed");
    }
}

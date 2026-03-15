use include_dir::{include_dir, Dir};

use super::club::ClubEntity;
use super::country::CountryEntity;
use super::league::LeagueEntity;
use super::names::NamesByCountryEntity;

/// Embedded data directory tree: data/{country_code}/{league_slug}/{club|league}.json
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

                // Load data/league.json
                let league_file = match league_dir.get_file(league_dir.path().join("data/league.json")) {
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

                let league_id = league.id;
                leagues.push(league);

                // Load all other .json files as clubs
                for file in league_dir.files() {
                    let file_name = file.path().file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");

                    if file_name == "league.json" || !file_name.ends_with(".json") {
                        continue;
                    }

                    let club_json = match file.contents_utf8() {
                        Some(s) => s,
                        None => continue,
                    };

                    let mut club: ClubEntity = match serde_json::from_str(club_json) {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("Failed to parse club {}/{}/{}: {}", country_code, league_slug, file_name, e);
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

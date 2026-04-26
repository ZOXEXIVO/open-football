use crate::generators::{PlayerGenerator, PositionType};
use crate::{DatabaseEntity, ForeignPlayerEntry};
use core::PlayerGenerator as CorePlayerGenerator;
use core::PeopleNameGeneratorData;
use core::utils::IntegerUtils;
use core::{Player, TeamType};
use chrono::Local;

use super::DatabaseGenerator;

impl DatabaseGenerator {
    pub(super) fn generate_players(
        player_generator: &PlayerGenerator,
        country_id: u32,
        team_reputation: u16,
        country_reputation: u16,
        team_type: &TeamType,
        league_id: Option<u32>,
        data: &DatabaseEntity,
        academy_level: u8,
        youth_quality: f32,
        academy_quality: f32,
        recruitment_quality: f32,
    ) -> Vec<Player> {
        let mut players = Vec::with_capacity(100);

        // Youth teams (U18/U19) use the academy generator — same logic as in-game academy intake
        // but with age range matching the team type. This ensures consistent quality between
        // initial squad generation and ongoing academy production.
        if matches!(team_type, TeamType::U18 | TeamType::U19) {
            let people_names = data.names_by_country.iter()
                .find(|n| n.country_id == country_id)
                .map(|n| PeopleNameGeneratorData {
                    first_names: n.first_names.clone(),
                    last_names: n.last_names.clone(),
                    nicknames: n.nicknames.clone(),
                })
                .unwrap_or_else(|| PeopleNameGeneratorData {
                    first_names: Vec::new(),
                    last_names: Vec::new(),
                    nicknames: Vec::new(),
                });

            let now = Local::now().date_naive();

            let (min_age, max_age) = match team_type {
                TeamType::U18 => (14, 18),
                _ => (14, 19),
            };

            let (gk_range, def_range, mid_range, st_range) = ((3, 5), (4, 9), (6, 10), (3, 6));

            for _ in 0..IntegerUtils::random(gk_range.0, gk_range.1) {
                players.push(CorePlayerGenerator::generate_for_age_range(
                    country_id, now, core::PlayerPositionType::Goalkeeper,
                    academy_level, &people_names, youth_quality, academy_quality, recruitment_quality,
                    min_age, max_age,
                ));
            }
            for _ in 0..IntegerUtils::random(def_range.0, def_range.1) {
                let pos = match IntegerUtils::random(0, 4) {
                    0 => core::PlayerPositionType::DefenderLeft,
                    1 => core::PlayerPositionType::DefenderRight,
                    _ => core::PlayerPositionType::DefenderCenter,
                };
                players.push(CorePlayerGenerator::generate_for_age_range(
                    country_id, now, pos,
                    academy_level, &people_names, youth_quality, academy_quality, recruitment_quality,
                    min_age, max_age,
                ));
            }
            for _ in 0..IntegerUtils::random(mid_range.0, mid_range.1) {
                let pos = match IntegerUtils::random(0, 3) {
                    0 => core::PlayerPositionType::DefensiveMidfielder,
                    1 => core::PlayerPositionType::MidfielderLeft,
                    2 => core::PlayerPositionType::MidfielderRight,
                    _ => core::PlayerPositionType::MidfielderCenter,
                };
                players.push(CorePlayerGenerator::generate_for_age_range(
                    country_id, now, pos,
                    academy_level, &people_names, youth_quality, academy_quality, recruitment_quality,
                    min_age, max_age,
                ));
            }
            for _ in 0..IntegerUtils::random(st_range.0, st_range.1) {
                let pos = match IntegerUtils::random(0, 2) {
                    0 => core::PlayerPositionType::Striker,
                    1 => core::PlayerPositionType::ForwardLeft,
                    _ => core::PlayerPositionType::ForwardRight,
                };
                players.push(CorePlayerGenerator::generate_for_age_range(
                    country_id, now, pos,
                    academy_level, &people_names, youth_quality, academy_quality, recruitment_quality,
                    min_age, max_age,
                ));
            }

            return players;
        }

        // Age range based on team type
        let (min_age, max_age) = match team_type {
            TeamType::U20 => (16, 20),
            TeamType::U21 => (16, 21),
            TeamType::U23 => (17, 23),
            TeamType::B | TeamType::Second | TeamType::Reserve => (17, 28),
            TeamType::Main => (17, 35),
            _ => (15, 18),
        };

        let is_youth = false;

        // Youth gem system: 10-20% of non-main team players get boosted reputation
        let is_non_main = *team_type != TeamType::Main;
        let gem_rep = (team_reputation as f32 * 2.5).min(10000.0) as u16;

        let foreign_players: &[ForeignPlayerEntry] = league_id
            .and_then(|lid| data.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.foreign_players.as_slice())
            .unwrap_or(&[]);

        let total_foreign_weight: i32 = foreign_players.iter().map(|fp| fp.weight as i32).sum();

        let domestic_continent_id = data.countries.iter()
            .find(|c| c.id == country_id)
            .map(|c| c.continent_id)
            .unwrap_or(1);

        let generate_one = |pos: PositionType| -> Player {
            // 10-20% chance this player is a youth gem with boosted skills
            let effective_rep = if is_non_main && IntegerUtils::random(0, 100) < 15 {
                gem_rep
            } else {
                team_reputation
            };

            if total_foreign_weight > 0 {
                let roll = IntegerUtils::random(0, 100);
                if roll < total_foreign_weight {
                    // Pick a foreign country via weighted random walk
                    let mut acc = 0i32;
                    for fp in foreign_players {
                        acc += fp.weight as i32;
                        if roll < acc {
                            let names = data.names_by_country.iter().find(|n| n.country_id == fp.country_id);
                            let people_names = match names {
                                Some(n) => PeopleNameGeneratorData {
                                    first_names: n.first_names.clone(),
                                    last_names: n.last_names.clone(),
                                    nicknames: n.nicknames.clone(),
                                },
                                None => PeopleNameGeneratorData {
                                    first_names: Vec::new(),
                                    last_names: Vec::new(),
                                    nicknames: Vec::new(),
                                },
                            };
                            let foreign_gen = PlayerGenerator::with_people_names(&people_names);
                            let foreign_country = data.countries.iter()
                                .find(|c| c.id == fp.country_id);
                            let foreign_country_rep = foreign_country.map(|c| c.reputation).unwrap_or(3000);
                            let foreign_continent_id = foreign_country.map(|c| c.continent_id).unwrap_or(1);
                            return foreign_gen.generate(fp.country_id, foreign_continent_id, pos, effective_rep, foreign_country_rep, min_age, max_age, is_youth);
                        }
                    }
                }
            }
            player_generator.generate(country_id, domestic_continent_id, pos, effective_rep, country_reputation, min_age, max_age, is_youth)
        };

        // Main teams need larger squads to avoid fielding fewer than 11 after
        // injuries, bans, international duty, and condition drops.
        let (gk_range, def_range, mid_range, st_range) = match team_type {
            TeamType::Main => ((3, 5), (6, 9), (7, 10), (5, 8)),
            _ => ((3, 5), (4, 9), (6, 10), (3, 6)),
        };

        for _ in 0..IntegerUtils::random(gk_range.0, gk_range.1) {
            players.push(generate_one(PositionType::Goalkeeper));
        }
        for _ in 0..IntegerUtils::random(def_range.0, def_range.1) {
            players.push(generate_one(PositionType::Defender));
        }
        for _ in 0..IntegerUtils::random(mid_range.0, mid_range.1) {
            players.push(generate_one(PositionType::Midfielder));
        }
        for _ in 0..IntegerUtils::random(st_range.0, st_range.1) {
            players.push(generate_one(PositionType::Striker));
        }

        // Ensure main teams always have at least 25 players
        if *team_type == TeamType::Main {
            let positions = [PositionType::Defender, PositionType::Midfielder, PositionType::Striker];
            let mut pos_idx = 0;
            while players.len() < 25 {
                players.push(generate_one(positions[pos_idx % positions.len()]));
                pos_idx += 1;
            }
        }

        players
    }
}

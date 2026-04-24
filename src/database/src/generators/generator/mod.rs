mod countries;
mod leagues;
mod clubs;
mod players;
mod staffs;

use chrono::{Datelike, Local, NaiveDate, NaiveDateTime};
use core::context::NaiveTime;
use core::continent::Continent;
use core::competitions::GlobalCompetitions;
use core::{
    seed_core_player_id_sequence, CompetitionScope, NationalCompetitionConfig, SimulatorData,
};
use crate::DatabaseEntity;
use crate::generators::convert::convert_national_competition;
use crate::generators::player::seed_player_id_sequence;
use rayon::prelude::*;

pub struct DatabaseGenerator;

impl DatabaseGenerator {
    pub fn generate(data: &DatabaseEntity) -> SimulatorData {
        // Seed the procedural id sequence past every ODB-supplied player id
        // so generated players for non-ODB clubs cannot collide with the
        // hand-curated records in `players.odb`. Both generators share the
        // same namespace (the simulator indexes all players by global id),
        // so both must be seeded — otherwise the core generator's counter
        // still starts at 100_000 and collides with ODB ids above that,
        // producing "click Giovanni Pavone, see Ibrahim Al-Hafith" bugs
        // when the global index resolves the shared id to the ODB record.
        if let Some(max_odb_id) = data.players_odb.as_ref().and_then(|o| o.max_player_id()) {
            seed_player_id_sequence(max_odb_id);
            seed_core_player_id_sequence(max_odb_id);
        }

        let current_date = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(Local::now().year(), 8, 1).unwrap(),
            NaiveTime::default(),
        );

        // Convert all national competition entities to runtime configs
        let all_configs: Vec<NationalCompetitionConfig> = data
            .national_competitions
            .iter()
            .map(|e| convert_national_competition(e))
            .collect();

        // Separate global configs for GlobalCompetitions
        let global_configs: Vec<NationalCompetitionConfig> = all_configs
            .iter()
            .filter(|c| c.scope == CompetitionScope::Global)
            .cloned()
            .collect();

        let global_competitions = GlobalCompetitions::new(global_configs);

        // Parallelise at the continent level too: only ~6 continents, but
        // each one drives an independent par_iter over its countries, so the
        // overall tree is rayon-parallel at three levels (continent →
        // country → club). Rayon's work-stealing keeps all cores saturated
        // even when one continent's country/club mix dwarfs the others.
        let continents = data
            .continents
            .par_iter()
            .map(|continent| {
                // Filter configs relevant to this continent:
                // - continental configs where continent_id matches
                // - global configs that have a qualifying zone for this continent
                let continent_configs: Vec<NationalCompetitionConfig> = all_configs
                    .iter()
                    .filter(|config| {
                        match config.scope {
                            CompetitionScope::Continental => {
                                config.continent_id == Some(continent.id)
                            }
                            CompetitionScope::Global => {
                                config.qualifying.zones.iter().any(|z| z.continent_id == continent.id)
                            }
                        }
                    })
                    .cloned()
                    .collect();

                Continent::new(
                    continent.id,
                    continent.name.clone(),
                    Self::generate_countries(continent, data),
                    continent_configs,
                )
            }).collect();

        let mut simulator_data = SimulatorData::new(current_date, continents, global_competitions);

        // Register ALL countries so nationality lookups always succeed
        // (simulation only loads countries with active leagues)
        for country in &data.countries {
            simulator_data.add_country_info(
                country.id,
                country.code.clone(),
                country.slug.clone(),
                country.name.clone(),
            );
        }

        simulator_data
    }
}

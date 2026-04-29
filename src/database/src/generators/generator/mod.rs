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
use crate::generators::PlayerGenerator;
use crate::generators::convert::convert_national_competition;
use log::info;
use rayon::prelude::*;

pub struct DatabaseGenerator;

impl DatabaseGenerator {
    pub fn generate(data: &DatabaseEntity) -> SimulatorData {
        // Seed the procedural id sequence past every ODB-supplied player
        // id so generated players for non-ODB clubs cannot collide with
        // the hand-curated records in `players.odb`. Both the database
        // loader (this pass) and the core generator (runtime academy
        // intake) draw from the same single counter via `next_player_id`
        // — one source of truth, one stream of monotonic ids. After
        // generation finishes we re-seed from the actual world (see end
        // of this function) as belt-and-suspenders against any id source
        // we don't know about today.
        if let Some(max_odb_id) = data.players_odb.as_ref().and_then(|o| o.max_player_id()) {
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
                country.continent_id,
                country.reputation,
            );
        }

        // Hydrate clubless players from `data/{cc}/free_agents/` directly
        // into `SimulatorData.free_agents` — same pool the runtime uses for
        // released players, so the existing free-agent matching pipeline
        // can sign them without any new plumbing.
        if let Some(odb) = data.players_odb.as_ref() {
            let records = odb.free_agents();
            if !records.is_empty() {
                let hydrated: Vec<core::Player> = records
                    .par_iter()
                    .map(|r| {
                        let (continent_id, country_code) = data
                            .countries
                            .iter()
                            .find(|c| c.id == r.country_id)
                            .map(|c| (c.continent_id, c.code.clone()))
                            .unwrap_or((1, String::new()));
                        PlayerGenerator::generate_from_odb(
                            r,
                            continent_id,
                            &country_code,
                            data,
                        )
                    })
                    .collect();
                let count = hydrated.len();
                simulator_data.free_agents.extend(hydrated);
                info!("free agents hydrated from compiled DB: {count}");
            }
        }

        // Final seed: walk the fully-populated world and bump the counter
        // past the highest id we actually placed. ODB ids, procedurally
        // generated club fillers, hydrated free agents, retired-pool
        // veterans — anything that ended up in the simulator. Runtime
        // academy intake then cannot mint a colliding id no matter what
        // mix of sources fed the world.
        simulator_data.seed_player_id_sequence();

        simulator_data
    }
}

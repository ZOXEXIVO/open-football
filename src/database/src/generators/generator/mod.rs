mod clubs;
mod countries;
mod leagues;
mod players;
pub(super) mod staffs;

use std::collections::HashMap;

use crate::DatabaseEntity;
use crate::generators::PlayerGenerator;
use crate::generators::convert::{
    CountryTransferProfileConverter, NationalCompetitionConverter, UefaU21Championship,
};
use crate::generators::world_start::WorldStart;
use chrono::NaiveDateTime;
use core::competitions::GlobalCompetitions;
use core::context::NaiveTime;
use core::continent::Continent;
use core::{CompetitionScope, NationalCompetitionConfig, PlayerIdSequence, SimulatorData};
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
            PlayerIdSequence::seed(max_odb_id);
        }

        let current_date = NaiveDateTime::new(WorldStart::date(), NaiveTime::default());

        // Country code → id, built once. The transfer cards name each other
        // by code (that is what the data tree's directories are), and every
        // corridor list has to be resolved into ids before the simulator can
        // read it.
        let country_id_by_code: HashMap<String, u32> = data
            .countries
            .iter()
            .map(|c| (c.code.to_ascii_lowercase(), c.id))
            .collect();

        // Convert all national competition entities to runtime configs.
        // The compiled database predates the `team_level` field, so the
        // bundled UEFA U21 Championship is appended here to give the U21
        // layer a competition to contest until the database ships one.
        let all_configs = Self::national_competition_configs(data);

        // Separate global configs for GlobalCompetitions
        let global_competitions = Self::global_competitions(&all_configs);

        let continents = Self::build_continents(data, &all_configs);

        let mut simulator_data = SimulatorData::new(current_date, continents, global_competitions);

        Self::register_country_info(&mut simulator_data, data, &country_id_by_code);

        // The market map was built from the participating countries alone;
        // rebuild it now that all 224 nationalities carry their card, so a
        // Senegalese export corridor exists even though Senegal runs no
        // league in this save.
        simulator_data.rebuild_market_map();

        // Hydrate clubless players from `data/{cc}/free_agents/` directly
        // into `SimulatorData.free_agents` — same pool the runtime uses for
        // released players, so the existing free-agent matching pipeline
        // can sign them without any new plumbing.
        Self::hydrate_free_agents(&mut simulator_data, data);

        // Final seed: walk the fully-populated world and bump the counter
        // past the highest id we actually placed. ODB ids, procedurally
        // generated club fillers, hydrated free agents, retired-pool
        // veterans — anything that ended up in the simulator. Runtime
        // academy intake then cannot mint a colliding id no matter what
        // mix of sources fed the world.
        simulator_data.seed_player_id_sequence();

        simulator_data
    }

    fn national_competition_configs(data: &DatabaseEntity) -> Vec<NationalCompetitionConfig> {
        let mut all_configs: Vec<NationalCompetitionConfig> = data
            .national_competitions
            .iter()
            .map(NationalCompetitionConverter::convert)
            .collect();
        all_configs.push(UefaU21Championship::config());
        all_configs
    }

    fn global_competitions(all_configs: &[NationalCompetitionConfig]) -> GlobalCompetitions {
        let global_configs: Vec<NationalCompetitionConfig> = all_configs
            .iter()
            .filter(|c| c.scope == CompetitionScope::Global)
            .cloned()
            .collect();
        GlobalCompetitions::new(global_configs)
    }

    fn build_continents(
        data: &DatabaseEntity,
        all_configs: &[NationalCompetitionConfig],
    ) -> Vec<Continent> {
        // Parallelise at the continent level too: only ~6 continents, but
        // each one drives an independent par_iter over its countries, so the
        // overall tree is rayon-parallel at three levels (continent →
        // country → club). Rayon's work-stealing keeps all cores saturated
        // even when one continent's country/club mix dwarfs the others.
        data.continents
            .par_iter()
            .map(|continent| {
                let continent_configs = Self::continent_configs(all_configs, continent);
                Continent::new(
                    continent.id,
                    continent.name.clone(),
                    Self::generate_countries(continent, data),
                    continent_configs,
                )
            })
            .collect()
    }

    fn continent_configs(
        all_configs: &[NationalCompetitionConfig],
        continent: &crate::ContinentEntity,
    ) -> Vec<NationalCompetitionConfig> {
        // Filter configs relevant to this continent:
        // - continental configs where continent_id matches
        // - global configs that have a qualifying zone for this continent
        all_configs
            .iter()
            .filter(|config| match config.scope {
                CompetitionScope::Continental => config.continent_id == Some(continent.id),
                CompetitionScope::Global => config
                    .qualifying
                    .zones
                    .iter()
                    .any(|z| z.continent_id == continent.id),
            })
            .cloned()
            .collect()
    }

    fn register_country_info(
        simulator_data: &mut SimulatorData,
        data: &DatabaseEntity,
        country_id_by_code: &HashMap<String, u32>,
    ) {
        let converter = CountryTransferProfileConverter::new(country_id_by_code);
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
                converter.convert(country.transfers.as_ref()),
            );
        }
    }

    fn hydrate_free_agents(simulator_data: &mut SimulatorData, data: &DatabaseEntity) {
        let Some(odb) = data.players_odb.as_ref() else {
            return;
        };
        let records = odb.free_agents();
        if records.is_empty() {
            return;
        }
        let hydrated: Vec<core::Player> = records
            .par_iter()
            .map(|r| {
                let (continent_id, country_code) = data
                    .countries
                    .iter()
                    .find(|c| c.id == r.country_id)
                    .map(|c| (c.continent_id, c.code.clone()))
                    .unwrap_or((1, String::new()));
                PlayerGenerator::generate_from_odb(r, continent_id, &country_code, data)
            })
            .collect();
        simulator_data.admit_opening_free_agents(hydrated);
    }
}

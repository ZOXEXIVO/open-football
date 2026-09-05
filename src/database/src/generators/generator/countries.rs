use std::collections::HashMap;

use crate::DatabaseEntity;
use crate::generators::convert::convert_country_transfers;
use crate::generators::{PlayerGenerator, StaffGenerator};
use crate::loaders::ContinentEntity;
use core::league::LeagueCollection;
use core::transfers::ScoutingRegion;
use core::{
    Country, CountryGeneratorData, CountryPricing, CountryRegulations, CountrySettings,
    SkinColorDistribution,
};
use rayon::prelude::*;

use super::DatabaseGenerator;
use super::staffs::{ScoutMarketPrior, ScoutMarketSeed};

impl DatabaseGenerator {
    pub(super) fn generate_countries(
        continent: &ContinentEntity,
        data: &DatabaseEntity,
    ) -> Vec<Country> {
        // Collect all country IDs that have clubs — scouts can know these regions
        let _all_country_ids: Vec<u32> = data
            .countries
            .iter()
            .filter(|c| data.clubs.iter().any(|cl| cl.country_id == c.id))
            .map(|c| c.id)
            .collect();

        // Country code → id for the transfer cards, which name each other by
        // code. Built once per continent rather than per country: the lists
        // are read for every country in the walk below.
        let country_id_by_code: HashMap<String, u32> = data
            .countries
            .iter()
            .map(|c| (c.code.to_ascii_lowercase(), c.id))
            .collect();

        // Each country is fully independent: its own name pools, its own
        // clubs, and no shared mutable state — perfect shape for par_iter.
        // Inner club generation also parallelises, so the nested split
        // keeps cores busy even when one continent has few countries but
        // big leagues (e.g. Europe: 50 countries, but Spain/Italy/England
        // carry the bulk of the per-country work).
        data.countries
            .par_iter()
            .filter(|cn| cn.continent_id == continent.id)
            .filter(|cn| data.leagues.iter().any(|l| l.country_id == cn.id))
            .map(|country| {
                let generator_data = match data
                    .names_by_country
                    .iter()
                    .find(|c| c.country_id == country.id)
                {
                    Some(names) => CountryGeneratorData::new(
                        names.first_names.clone(),
                        names.last_names.clone(),
                        names.nicknames.clone(),
                    ),
                    None => CountryGeneratorData::empty(),
                };

                let player_generator =
                    PlayerGenerator::with_people_names(&generator_data.people_names);

                let staff_generator =
                    StaffGenerator::with_people_names(&generator_data.people_names);

                let transfer_profile =
                    convert_country_transfers(country.transfers.as_ref(), &country_id_by_code);

                // The markets this country's scouting departments start in.
                // Read straight off the country's own import card, so a
                // Turkish club's scouts begin knowing Brazil and Nigeria
                // rather than "South America" and "West Africa" — the
                // strictest day-0 condition available, and the one that
                // makes the shipped world its own evidence.
                let scout_priors: Vec<ScoutMarketPrior> = transfer_profile
                    .import
                    .iter()
                    .filter_map(|corridor| {
                        let source = data
                            .countries
                            .iter()
                            .find(|c| c.id == corridor.country_id)?;
                        Some(ScoutMarketPrior {
                            country_id: corridor.country_id,
                            weight: corridor.weight,
                            region: ScoutingRegion::from_country(source.continent_id, &source.code),
                        })
                    })
                    .collect();
                let scout_seed = ScoutMarketSeed {
                    country_id: country.id,
                    continent_id: continent.id,
                    country_code: &country.code,
                    import_priors: &scout_priors,
                };

                let mut clubs = Self::generate_clubs(
                    &scout_seed,
                    country.reputation,
                    data,
                    &player_generator,
                    &staff_generator,
                );

                let mut leagues_vec = Self::generate_leagues(country.id, country.reputation, data);
                // Build the domestic cup from the real leagues (before youth
                // sub-leagues are appended) so the tier-1 season window is
                // picked up cleanly.
                let domestic_cup = Self::generate_domestic_cup(country, &leagues_vec);
                // Playoffs for grouped competitions (MLS Cup, …), also built
                // from the real leagues before youth sub-leagues are added.
                let playoffs = Self::generate_playoffs(country, &leagues_vec);
                Self::create_subteams_leagues(country.id, &mut clubs, &mut leagues_vec, data);
                let leagues = LeagueCollection::new(leagues_vec);

                let settings = CountrySettings {
                    pricing: CountryPricing {
                        price_level: country.settings.pricing.price_level,
                    },
                    skin_colors: SkinColorDistribution {
                        white: country.skin_colors.white,
                        black: country.skin_colors.black,
                        metis: country.skin_colors.metis,
                    },
                };

                Country::builder()
                    .id(country.id)
                    .code(country.code.clone())
                    .slug(country.slug.clone())
                    .name(country.name.clone())
                    .background_color(country.background_color.clone())
                    .foreground_color(country.foreground_color.clone())
                    .continent_id(continent.id)
                    .leagues(leagues)
                    .domestic_cup(domestic_cup)
                    .playoffs(playoffs)
                    .clubs(clubs)
                    .reputation(country.reputation)
                    .settings(settings)
                    // Squad-registration rules from the country's own
                    // public regulations — foreigner quotas where the rule
                    // counts passports, homegrown minimums where it counts
                    // them the way this model can read. `None` everywhere
                    // else, so nothing is invented.
                    .regulations(CountryRegulations::for_country_code(&country.code))
                    // The country's transfer-market card: shipped priors,
                    // resolved from codes into ids and normalised. Empty
                    // when the data does not name this country, in which
                    // case every pair it is part of derives instead.
                    .transfer_profile(transfer_profile)
                    .generator_data(generator_data)
                    .build()
                    .expect("Failed to build Country")
            })
            .collect()
    }
}

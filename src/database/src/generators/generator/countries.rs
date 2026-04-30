use crate::DatabaseEntity;
use crate::generators::{PlayerGenerator, StaffGenerator};
use crate::loaders::ContinentEntity;
use core::league::LeagueCollection;
use core::{Country, CountryGeneratorData, CountryPricing, CountrySettings, SkinColorDistribution};
use rayon::prelude::*;

use super::DatabaseGenerator;

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

                let mut clubs = Self::generate_clubs(
                    country.id,
                    continent.id,
                    &country.code,
                    country.reputation,
                    data,
                    &player_generator,
                    &staff_generator,
                );

                let mut leagues_vec = Self::generate_leagues(country.id, country.reputation, data);
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
                    .clubs(clubs)
                    .reputation(country.reputation)
                    .settings(settings)
                    .generator_data(generator_data)
                    .build()
                    .expect("Failed to build Country")
            })
            .collect()
    }
}

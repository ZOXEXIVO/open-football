use std::collections::HashMap;

use crate::DatabaseEntity;
use crate::generators::convert::CountryTransferProfileConverter;
use crate::generators::{PlayerGenerator, StaffGenerator};
use crate::loaders::{ContinentEntity, CountryEntity};
use core::league::{DomesticCup, LeagueCollection, LeaguePlayoff};
use core::transfers::{CountryTransferProfile, ScoutingRegion};
use core::{
    Club, Country, CountryGeneratorData, CountryPricing, CountryRegulations, CountrySettings,
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
        let factory = CountryFactory {
            continent,
            data,
            country_id_by_code: &country_id_by_code,
        };

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
            .map(|country| factory.build(country))
            .collect()
    }
}

/// Everything one country needs from its continent and database while the
/// per-country world is assembled.
struct CountryFactory<'a> {
    continent: &'a ContinentEntity,
    data: &'a DatabaseEntity,
    country_id_by_code: &'a HashMap<String, u32>,
}

impl CountryFactory<'_> {
    fn build(&self, country: &CountryEntity) -> Country {
        let generator_data = self.generator_data(country);
        let player_generator = PlayerGenerator::with_people_names(&generator_data.people_names);
        let staff_generator = StaffGenerator::with_people_names(&generator_data.people_names);
        let transfer_profile = self.transfer_profile(country);

        // The markets this country's scouting departments start in. Read
        // straight off the country's own import card, so a Turkish club's
        // scouts begin knowing Brazil and Nigeria rather than "South
        // America" and "West Africa" — the strictest day-0 condition
        // available, and the one that makes the shipped world its own
        // evidence.
        let scout_priors = self.scout_priors(&transfer_profile);
        let scout_seed = ScoutMarketSeed {
            country_id: country.id,
            continent_id: self.continent.id,
            country_code: &country.code,
            import_priors: &scout_priors,
        };

        let mut clubs = DatabaseGenerator::generate_clubs(
            &scout_seed,
            country.reputation,
            self.data,
            &player_generator,
            &staff_generator,
        );

        let mut leagues =
            DatabaseGenerator::generate_leagues(country.id, country.reputation, self.data);
        // Build the domestic cup from the real leagues (before youth
        // sub-leagues are appended) so the tier-1 season window is
        // picked up cleanly.
        let domestic_cup = DatabaseGenerator::generate_domestic_cup(country, &leagues);
        // Playoffs for grouped competitions (MLS Cup, …), also built
        // from the real leagues before youth sub-leagues are added.
        let playoffs = DatabaseGenerator::generate_playoffs(country, &leagues);
        DatabaseGenerator::create_subteams_leagues(country.id, &mut clubs, &mut leagues, self.data);

        self.build_country(
            country,
            CountryParts {
                generator_data,
                leagues: LeagueCollection::new(leagues),
                domestic_cup,
                playoffs,
                clubs,
                transfer_profile,
                settings: Self::settings(country),
            },
        )
    }

    fn generator_data(&self, country: &CountryEntity) -> CountryGeneratorData {
        match self
            .data
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
        }
    }

    fn transfer_profile(&self, country: &CountryEntity) -> CountryTransferProfile {
        CountryTransferProfileConverter::new(self.country_id_by_code)
            .convert(country.transfers.as_ref())
    }

    fn scout_priors(&self, transfer_profile: &CountryTransferProfile) -> Vec<ScoutMarketPrior> {
        transfer_profile
            .import
            .iter()
            .filter_map(|corridor| {
                let source = self
                    .data
                    .countries
                    .iter()
                    .find(|c| c.id == corridor.country_id)?;
                Some(ScoutMarketPrior {
                    country_id: corridor.country_id,
                    weight: corridor.weight,
                    region: ScoutingRegion::from_country(source.continent_id, &source.code),
                })
            })
            .collect()
    }

    fn settings(country: &CountryEntity) -> CountrySettings {
        CountrySettings {
            pricing: CountryPricing {
                price_level: country.settings.pricing.price_level,
            },
            skin_colors: SkinColorDistribution {
                white: country.skin_colors.white,
                black: country.skin_colors.black,
                metis: country.skin_colors.metis,
            },
        }
    }

    fn build_country(&self, country: &CountryEntity, parts: CountryParts) -> Country {
        Country::builder()
            .id(country.id)
            .code(country.code.clone())
            .slug(country.slug.clone())
            .name(country.name.clone())
            .background_color(country.background_color.clone())
            .foreground_color(country.foreground_color.clone())
            .continent_id(self.continent.id)
            .leagues(parts.leagues)
            .domestic_cup(parts.domestic_cup)
            .playoffs(parts.playoffs)
            .clubs(parts.clubs)
            .reputation(country.reputation)
            .settings(parts.settings)
            // Squad-registration rules from the country's own public
            // regulations — foreigner quotas where the rule counts
            // passports, homegrown minimums where it counts them the way
            // this model can read. `None` everywhere else, so nothing is
            // invented.
            .regulations(CountryRegulations::for_country_code(&country.code))
            // The country's transfer-market card: shipped priors, resolved
            // from codes into ids and normalised. Empty when the data does
            // not name this country, in which case every pair it is part of
            // derives instead.
            .transfer_profile(parts.transfer_profile)
            .generator_data(parts.generator_data)
            .build()
            .expect("Failed to build Country")
    }
}

struct CountryParts {
    generator_data: CountryGeneratorData,
    leagues: LeagueCollection,
    domestic_cup: Option<DomesticCup>,
    playoffs: Vec<LeaguePlayoff>,
    clubs: Vec<Club>,
    transfer_profile: CountryTransferProfile,
    settings: CountrySettings,
}

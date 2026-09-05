use serde::Deserialize;

use super::compiled::compiled;
use super::domestic_cup::DomesticCupEntity;

#[derive(Deserialize, Clone)]
pub struct CountryEntity {
    pub id: u32,
    pub code: String,
    pub slug: String,
    pub name: String,
    pub background_color: String,
    pub foreground_color: String,
    pub continent_id: u32,
    pub reputation: u16,
    pub settings: CountrySettingsEntity,
    #[serde(default)]
    pub skin_colors: SkinColorsEntity,
    /// The country's named domestic cup, resolved from the compiled
    /// `domestic_cups` table by `CountryLoader::load`. Not present in
    /// countries.json (hence `skip_deserializing`); `None` means the
    /// runtime generator falls back to a "{Country} Cup".
    #[serde(skip_deserializing, default)]
    pub domestic_cup: Option<DomesticCupEntity>,
    /// The country's transfer-market card, resolved from the compiled
    /// `country_transfers` table by `CountryLoader::load` the same way the
    /// cup is. `None` for a country the data does not name — every pair
    /// involving it then falls to the derived corridor prior.
    #[serde(skip_deserializing, default)]
    pub transfers: Option<CountryTransferProfileEntity>,
}

/// One country's transfer-market priors as the compiler emits them:
/// directional corridor lists with relative weights, the diaspora shares the
/// squads cannot show, and the top division's typical foreign share.
///
/// Weights arrive on whatever scale the data files were authored with; the
/// runtime normalises each list by its own maximum, so only the SHAPE of a
/// list matters and two countries' cards stay comparable.
#[derive(Deserialize, Clone, Debug)]
pub struct CountryTransferProfileEntity {
    pub code: String,
    /// `"derived"` (read off the shipped squads) or `"authored"` (a human
    /// corrected the draft). Diagnostics only.
    #[serde(default)]
    pub source: String,
    /// Where this country's CLUBS buy foreigners from.
    #[serde(default)]
    pub import: Vec<CountryCorridorEntity>,
    /// Where this country's NATIONALS go.
    #[serde(default)]
    pub export: Vec<CountryCorridorEntity>,
    #[serde(default)]
    pub diaspora: Vec<CountryDiasporaEntity>,
    #[serde(default)]
    pub foreign_share: f32,
}

#[derive(Deserialize, Clone, Debug)]
pub struct CountryCorridorEntity {
    pub country: String,
    pub weight: f32,
    /// `"money"` marks a wage-led landing; absent means an ordinary talent
    /// corridor.
    #[serde(default)]
    pub kind: Option<String>,
}

impl CountryCorridorEntity {
    pub fn is_money(&self) -> bool {
        self.kind.as_deref() == Some("money")
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct CountryDiasporaEntity {
    pub country: String,
    pub share: f32,
}

#[derive(Deserialize, Clone)]
pub struct CountrySettingsEntity {
    pub pricing: CountryPricingEntity,
}

#[derive(Deserialize, Clone)]
pub struct CountryPricingEntity {
    pub price_level: f32,
}

#[derive(Deserialize, Clone)]
pub struct SkinColorsEntity {
    pub white: u8,
    pub black: u8,
    pub metis: u8,
}

impl Default for SkinColorsEntity {
    fn default() -> Self {
        SkinColorsEntity {
            white: 50,
            black: 20,
            metis: 30,
        }
    }
}

pub struct CountryLoader;

impl CountryLoader {
    pub fn load() -> Vec<CountryEntity> {
        let db = compiled();
        db.countries
            .iter()
            .cloned()
            .map(|mut country| {
                // Attach the named cup (if configured) by country slug.
                // Matching is case-insensitive on the trimmed slug so a
                // stray space or capitalisation in the data doesn't drop
                // the cup — the fallback generator covers any misses.
                let key = country.slug.trim().to_ascii_lowercase();
                country.domestic_cup = db
                    .domestic_cups
                    .iter()
                    .find(|c| c.country_slug.trim().to_ascii_lowercase() == key)
                    .cloned();
                // Same pattern for the transfer card, keyed by code rather
                // than slug — the code is what the data tree's directories
                // and every corridor list name each other by.
                let code = country.code.trim().to_ascii_lowercase();
                country.transfers = db
                    .country_transfers
                    .iter()
                    .find(|t| t.code.trim().to_ascii_lowercase() == code)
                    .cloned();
                country
            })
            .collect()
    }

    /// Look up a country code by its ID. Always returns lowercase ASCII —
    /// the loaded data is lowercase ("br", "nl", "jp"), and consumers
    /// (`Language::from_country_code`, `country_skill_bias`,
    /// `PhysicalProfile::country_height_offset`) all match on lowercase.
    /// Forcing the cast here makes the contract explicit and immune to any
    /// future data file accidentally storing mixed case.
    pub fn code_for_id(country_id: u32) -> String {
        compiled()
            .countries
            .iter()
            .find(|c| c.id == country_id)
            .map(|c| c.code.to_ascii_lowercase())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::CountryLoader;

    #[test]
    fn named_domestic_cups_resolve_onto_countries() {
        let countries = CountryLoader::load();
        let cup_name = |slug: &str| {
            countries
                .iter()
                .find(|c| c.slug == slug)
                .unwrap_or_else(|| panic!("country {slug} missing"))
                .domestic_cup
                .as_ref()
                .map(|c| c.name.as_str())
        };

        assert_eq!(cup_name("england"), Some("FA Cup"));
        assert_eq!(cup_name("spain"), Some("Copa del Rey"));
        assert_eq!(cup_name("italy"), Some("Coppa Italia"));
        assert_eq!(cup_name("germany"), Some("DFB-Pokal"));

        // A country with no configured cup resolves to `None`; the runtime
        // generator gives it a "{Country} Cup" fallback.
        assert_eq!(cup_name("afghanistan"), None);
    }

    /// Every country the data tree models must ship a transfer card. A
    /// missing one is silent at runtime — the pairs simply derive — so the
    /// only place it can be caught is here.
    #[test]
    fn every_modelled_country_ships_a_transfer_card() {
        let countries = CountryLoader::load();
        let modelled: Vec<&str> = [
            "ae", "al", "am", "ar", "at", "au", "az", "be", "bg", "br", "by", "ch", "cl", "cm",
            "co", "cy", "cz", "de", "dk", "dz", "ee", "eg", "es", "fi", "fj", "fr", "gb", "ge",
            "gh", "gr", "hr", "hu", "id", "il", "ir", "is", "it", "jp", "ke", "kz", "lt", "lv",
            "ma", "ml", "mt", "mx", "ng", "nl", "no", "nz", "pe", "pl", "pt", "py", "ro", "rs",
            "ru", "sa", "se", "si", "sk", "td", "tr", "ua", "us", "uy", "uz", "ve", "za",
        ]
        .to_vec();
        for code in modelled {
            let country = countries
                .iter()
                .find(|c| c.code == code)
                .unwrap_or_else(|| panic!("country {code} missing from the database"));
            let card = country
                .transfers
                .as_ref()
                .unwrap_or_else(|| panic!("country {code} ships no transfers block"));
            assert!(
                !card.import.is_empty() || !card.export.is_empty(),
                "country {code} ships an empty transfer card"
            );
            assert!(
                (0.0..=1.0).contains(&card.foreign_share),
                "country {code} foreign_share out of range"
            );
        }
    }

    /// The corridors have to be asymmetric, or the whole model collapses into
    /// "these two countries trade players". Russia → Turkey is one of the
    /// biggest corridors in football; Turkey → Russia is a trickle.
    #[test]
    fn corridors_are_directional() {
        let countries = CountryLoader::load();
        let card = |code: &str| {
            countries
                .iter()
                .find(|c| c.code == code)
                .and_then(|c| c.transfers.as_ref())
                .unwrap_or_else(|| panic!("no card for {code}"))
        };
        let export_weight = |from: &str, to: &str| {
            card(from)
                .export
                .iter()
                .find(|e| e.country == to)
                .map(|e| e.weight)
                .unwrap_or(0.0)
        };
        assert!(
            export_weight("ru", "tr") > export_weight("tr", "ru"),
            "Russia exports to Turkey far more than the reverse"
        );
        assert!(
            export_weight("br", "pt") > export_weight("pt", "br"),
            "Brazil exports to Portugal far more than the reverse"
        );
    }

    /// The end-to-end read: shipped cards through the converter into a
    /// live `MarketMap`, asserting the two moves the whole feature exists
    /// to separate.
    ///
    /// Deliberately here rather than in `core`: the core tests run against
    /// a hand-built miniature world, and a card that is right in a fixture
    /// and wrong in `database.db` would pass both.
    #[test]
    fn the_shipped_cards_price_the_named_corridors() {
        use std::collections::HashMap;

        use crate::generators::convert::convert_country_transfers;
        use core::transfers::{
            MarketAffinity, MarketAffinityInputs, MarketCountryFacts, MarketMap, MoveKind,
            ScoutingRegion,
        };

        let countries = CountryLoader::load();
        let by_code: HashMap<String, u32> = countries
            .iter()
            .map(|c| (c.code.to_ascii_lowercase(), c.id))
            .collect();
        let id = |code: &str| *by_code.get(code).unwrap_or_else(|| panic!("no {code}"));

        let profiles = countries
            .iter()
            .map(|c| (c.id, convert_country_transfers(c.transfers.as_ref(), &by_code)))
            .collect();
        let facts = countries
            .iter()
            .map(|c| {
                (
                    c.id,
                    MarketCountryFacts {
                        id: c.id,
                        code: c.code.clone(),
                        continent_id: c.continent_id,
                        region: ScoutingRegion::from_country(c.continent_id, &c.code),
                        reputation: c.reputation,
                        // No leagues loaded in a bare loader test; the
                        // country's own reputation is the closest stand-in
                        // and it only feeds the ladder term.
                        top_flight_reputation: c.reputation,
                        median_top_flight_wage: 0,
                    },
                )
            })
            .collect();
        let map = MarketMap::new(profiles, facts);

        let affinity = |nationality: &str, playing_in: &str, buyer: &str| {
            MarketAffinity::affinity(
                &map,
                MarketAffinityInputs {
                    buyer_country_id: id(buyer),
                    nationality_country_id: id(nationality),
                    current_country_id: id(playing_in),
                    kind: MoveKind::Talent,
                    benefactor: 0.0,
                },
            )
        };

        let to_turkey = affinity("ru", "ru", "tr");
        let to_brazil = affinity("ru", "ru", "br");
        let to_cameroon = affinity("ru", "ru", "cm");
        assert!(
            to_turkey > 4.0 * to_brazil,
            "Russia -> Turkey ({to_turkey:.3}) must dwarf Russia -> Brazil ({to_brazil:.3})"
        );
        assert!(
            to_cameroon < 0.12,
            "Russia -> Cameroon must sit near the floor, was {to_cameroon:.3}"
        );

        // A Brazilian at Porto is a Portugal-market player: Spain sees him
        // more readily than the same man at Flamengo.
        let at_porto = affinity("br", "pt", "es");
        let at_home = affinity("br", "br", "es");
        assert!(
            at_porto > at_home,
            "the Porto shop window ({at_porto:.3}) must beat the Brasileirao ({at_home:.3})"
        );

        // Going home is always plausible, whatever the league.
        assert_eq!(affinity("cm", "es", "cm"), 1.0);
    }
}

use core::shared::fullname::slug_from_display;
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
                // The data writes slugs as display text ("czech republic",
                // "côte d'ivoire"). Folded once here, every URL and the slug
                // index carry the same URL-safe form. The cup table is keyed
                // by the same display text, so it is matched folded too.
                country.slug = slug_from_display(&country.slug);
                country.domestic_cup = db
                    .domestic_cups
                    .iter()
                    .find(|c| slug_from_display(&c.country_slug) == country.slug)
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

    /// Slugs go into URLs, so the data's display text ("south africa",
    /// "st. vincent / grenadines") must come out URL-safe — and folding must
    /// not merge two countries the data tells apart.
    #[test]
    fn country_slugs_are_url_safe_and_stay_distinct() {
        use std::collections::HashSet;

        let countries = CountryLoader::load();
        for country in &countries {
            assert!(
                !country.slug.is_empty()
                    && country
                        .slug
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "country {} has a slug unfit for a URL: {:?}",
                country.id,
                country.slug
            );
        }

        let raw: HashSet<&str> = super::compiled()
            .countries
            .iter()
            .map(|c| c.slug.as_str())
            .collect();
        let folded: HashSet<&str> = countries.iter().map(|c| c.slug.as_str()).collect();
        assert_eq!(folded.len(), raw.len(), "folding merged two country slugs");

        let slug_of = |code: &str| {
            countries
                .iter()
                .find(|c| c.code == code)
                .map(|c| c.slug.as_str())
        };
        assert_eq!(slug_of("za"), Some("south-africa"));
        assert_eq!(slug_of("ci"), Some("cote-d-ivoire"));
    }

    /// The cup table names its country by the data's display text, so a cup
    /// whose country slug folds must still land on it.
    #[test]
    fn named_cups_follow_their_country_through_the_fold() {
        let countries = CountryLoader::load();
        let czech = countries
            .iter()
            .find(|c| c.slug == "czech-republic")
            .expect("czech republic folds to czech-republic");

        assert!(czech.domestic_cup.is_some(), "the Czech cup was dropped");
    }

    /// Every country the data tree models must ship a transfer card. A
    /// missing one is silent at runtime — the pairs simply derive — so the
    /// only place it can be caught is here.
    #[test]
    fn every_modelled_country_ships_a_transfer_card() {
        let countries = CountryLoader::load();
        let modelled: Vec<&str> = [
            "ae", "al", "am", "ar", "at", "au", "az", "be", "bg", "br", "by", "ch", "cl", "cm",
            "cn", "co", "cy", "cz", "de", "dk", "dz", "ee", "eg", "es", "fi", "fj", "fr", "gb",
            "ge", "gh", "gr", "hr", "hu", "id", "il", "ir", "is", "it", "jp", "ke", "kz", "lt",
            "lv", "ma", "ml", "mt", "mx", "ng", "nl", "no", "nz", "pe", "pl", "pt", "py", "ro",
            "rs", "ru", "sa", "se", "si", "sk", "td", "tr", "ua", "us", "uy", "uz", "ve", "za",
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
            .map(|c| {
                (
                    c.id,
                    convert_country_transfers(c.transfers.as_ref(), &by_code),
                )
            })
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

        // Exercise the production offer builder and acceptance decision, not
        // just the geography score. Change ONLY the export preference for a
        // Portuguese player offered a job in Brazil. Fluency (or missing
        // language data) used to make both offers identical here.
        use core::club::player::language::{Language, LanguageProfile};
        use core::country::core::builder::CountryBuilder;
        use core::transfers::gate::{
            AppraisalConfig, OfferKind, OfferViewBuilder, PlayerOfferAppraisal, PlayerStance,
        };

        let buyer = CountryBuilder::new()
            .id(id("br"))
            .code("br".into())
            .slug("brazil".into())
            .name("Brazil".into())
            .continent_id(3)
            .leagues(core::league::LeagueCollection::new(vec![]))
            .clubs(vec![])
            .build()
            .unwrap();
        let mut changed_profile = map.profile(id("pt")).clone();
        let preference = changed_profile
            .export
            .iter_mut()
            .find(|row| row.country_id == id("br"))
            .expect("Portugal's shipped export list includes Brazil");
        assert!(preference.weight < 0.5);
        preference.weight = 1.0;
        let preferred_map = MarketMap::new(
            HashMap::from([
                (id("pt"), changed_profile),
                (id("br"), map.profile(id("br")).clone()),
            ]),
            HashMap::from([
                (id("pt"), map.facts(id("pt")).unwrap().clone()),
                (id("br"), map.facts(id("br")).unwrap().clone()),
            ]),
        );
        let cfg = AppraisalConfig::default();
        for language_profile in [
            LanguageProfile {
                fluent_mask: Language::country_language_mask("pt"),
                conversational_mask: 0,
            },
            LanguageProfile::default(),
        ] {
            let stance = PlayerStance {
                nationality_country_id: id("pt"),
                seller_country_id: id("pt"),
                seller_continent_id: 1,
                language_profile,
                ..PlayerStance::neutral()
            };
            for kind in [OfferKind::Permanent, OfferKind::Loan] {
                let mut ordinary = OfferViewBuilder::build(
                    kind,
                    &buyer,
                    1,
                    &stance,
                    stance.current_wage,
                    None,
                    0.0,
                    0.5,
                    &map,
                );
                let mut preferred = OfferViewBuilder::build(
                    kind,
                    &buyer,
                    1,
                    &stance,
                    stance.current_wage,
                    None,
                    0.0,
                    0.5,
                    &preferred_map,
                );
                assert_eq!(ordinary.language_affinity, 1.0);
                assert!(ordinary.place_familiarity < preferred.place_familiarity);
                let assess = |offer: &core::transfers::gate::OfferView| {
                    PlayerOfferAppraisal::appraise(&stance, offer, 0.0, &cfg)
                };
                let ordinary_demand = assess(&ordinary).reservation_wage;
                let preferred_demand = assess(&preferred).reservation_wage;
                assert!(ordinary_demand > preferred_demand);
                let wage = (ordinary_demand as f64 + preferred_demand as f64) / 2.0;
                ordinary.offered_wage = wage;
                preferred.offered_wage = wage;
                assert!(!assess(&ordinary).accepts(), "weak route at the same wage");
                assert!(
                    assess(&preferred).accepts(),
                    "preferred route at the same wage"
                );
                ordinary.offered_wage = ordinary_demand as f64 * 1.02;
                assert!(assess(&ordinary).accepts(), "a preference is not a ban");
            }
        }
    }
}

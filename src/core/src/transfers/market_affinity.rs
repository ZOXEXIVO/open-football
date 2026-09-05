//! `MarketAffinity` — how plausible a place is for a player, on one 0..1
//! scale that every transfer path reads.
//!
//! A move has three geographies, and the model needs all three: where the
//! player is FROM (his passport, his language, his diaspora), where he PLAYS
//! (a Brazilian at Porto is a Portugal-market player and a Brazilian at
//! Flamengo is not), and where the buyer is (its own country's habits, and
//! whether it is a league that buys names at all). This module folds them
//! into one number.
//!
//! The number only weights and gates. It never chooses: no path may sort
//! destinations by affinity and take the top one, or the country cards stop
//! being priors and become the static routing table the design forbids.
//!
//! Gates read truth, rankings read belief — so the affinity itself is
//! deterministic, and the belief noise that stops twenty Turkish clubs
//! signing the same Russian lives in the callers' existing `ClubOpinion`
//! machinery, not here.

use crate::transfers::market_map::{AFFINITY_FLOOR, MarketMap};

/// What kind of move is being priced. A wage-led landing answers to the
/// destination's capacity to buy names rather than to any corridor: the Gulf,
/// MLS and Japan sign from everywhere, and that is the exception the model
/// must preserve rather than flatten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveKind {
    /// The ordinary transfer: a club recruiting a footballer.
    Talent,
    /// A wage-led signing. Set by callers that know the buyer is spending an
    /// owner's money (`WagePower`); also inferred from a `kind: "money"`
    /// mark on either country's card.
    Money,
}

/// One move's geography, as the market reads it.
#[derive(Debug, Clone, Copy)]
pub struct MarketAffinityInputs {
    /// The buying club's country.
    pub buyer_country_id: u32,
    /// The player's nationality.
    pub nationality_country_id: u32,
    /// The country he currently plays in. Equal to the nationality for a
    /// player who never left home; `0` when he is between clubs and has no
    /// recorded last league.
    pub current_country_id: u32,
    pub kind: MoveKind,
}

/// The read API over [`MarketMap`]. A unit struct rather than free functions
/// — one namespace for the whole geography read, per the project's
/// no-global-helpers rule.
pub struct MarketAffinity;

impl MarketAffinity {
    /// How much the market he PLAYS in can add on top of the corridor his
    /// passport already opens.
    ///
    /// An ADDITION, not a blend. The design specified a 0.65 : 0.35 average
    /// of the two corridors, and measured against the shipped cards that
    /// turns the shop window into a penalty: Spain's card imports Brazilians
    /// (0.375) more heavily than it imports out of the Portuguese league
    /// (0.262), so averaging made a Brazilian at Porto LESS plausible for
    /// Spain than the same man at Flamengo. Playing at Porto cannot make a
    /// Brazilian harder to sign. So the current market lifts, never drags,
    /// and the lift diminishes as the nationality corridor approaches 1 —
    /// a shop window adds nothing to a corridor that is already wide open.
    const CURRENT_MARKET_LIFT: f32 = 0.35;
    /// A corridor only one of the two cards names is real but thinner
    /// evidence than one both name.
    const MISSING_SIDE_DISCOUNT: f32 = 0.7;

    /// How plausible this destination is for this player, 0.02..1.
    ///
    /// `1.0` for a man going home — there is no such thing as an
    /// implausible return to your own country, whatever the league.
    pub fn affinity(map: &MarketMap, inputs: MarketAffinityInputs) -> f32 {
        if inputs.nationality_country_id != 0
            && inputs.nationality_country_id == inputs.buyer_country_id
        {
            return 1.0;
        }

        let nationality = map.corridor(inputs.nationality_country_id, inputs.buyer_country_id);
        let corridor_nationality = Self::blend_corridor(&nationality);

        // The shop window. Where he plays now enters as an IMPORT question
        // only — the buyer is shopping in that market, and what that
        // market's own nationals do is beside the point — and it enters
        // only for a man playing ABROAD: a Brazilian in Brazil has no shop
        // window, he is simply a Brazilian.
        let shop_window = if inputs.current_country_id == 0
            || inputs.current_country_id == inputs.nationality_country_id
        {
            0.0
        } else if inputs.current_country_id == inputs.buyer_country_id {
            // Already playing in the buying country: nothing about him is
            // hard to see.
            1.0
        } else {
            let current = map.corridor(inputs.current_country_id, inputs.buyer_country_id);
            current.data_import.unwrap_or(current.derived.import)
        };
        let lift = Self::CURRENT_MARKET_LIFT * shop_window * (1.0 - corridor_nationality);

        let diaspora = map.diaspora_link(inputs.nationality_country_id, inputs.buyer_country_id);

        let mut geo = corridor_nationality + lift + diaspora;

        // Money reaches where corridors do not. The floor, not a bonus: a
        // corridor that is already stronger than the buyer's import capacity
        // keeps its own value.
        if inputs.kind == MoveKind::Money || nationality.money {
            geo = geo.max(map.import_capacity(inputs.buyer_country_id));
        }

        geo.clamp(AFFINITY_FLOOR, 1.0)
    }

    /// The player's OWN map of a destination — what he knows of the place,
    /// as distinct from what the buyer's market knows of him. Used by the
    /// personal-terms appraisal, where the question is whether HE would go.
    ///
    /// Deliberately the maximum rather than a blend: one strong reason is
    /// enough. His compatriots go there, or his diaspora lives there, or
    /// they speak a language he has. Any of the three makes a place
    /// familiar; needing all three would make every foreign move strange.
    pub fn player_affinity(
        map: &MarketMap,
        nationality_country_id: u32,
        buyer_country_id: u32,
        language_affinity: f32,
    ) -> f32 {
        if nationality_country_id != 0 && nationality_country_id == buyer_country_id {
            return 1.0;
        }
        let corridor = map.corridor(nationality_country_id, buyer_country_id);
        let export = corridor.data_export.unwrap_or(corridor.derived.export);
        let diaspora = map.diaspora_link(nationality_country_id, buyer_country_id);
        export
            .max(diaspora)
            .max(language_affinity.clamp(0.0, 1.0))
            .clamp(AFFINITY_FLOOR, 1.0)
    }

    /// Geometric mean of the two sides when both cards name the corridor;
    /// the named side discounted when only one does; the derived pair when
    /// neither does.
    ///
    /// The geometric mean is what makes a corridor need agreement: a
    /// destination that says it buys Brazilians and a Brazil card that says
    /// its nationals go there is a corridor, and either one alone is a
    /// claim.
    fn blend_corridor(reading: &super::market_map::CorridorReading) -> f32 {
        match (reading.data_import, reading.data_export) {
            (Some(import), Some(export)) => (import * export).sqrt(),
            (Some(import), None) => import * Self::MISSING_SIDE_DISCOUNT,
            (None, Some(export)) => export * Self::MISSING_SIDE_DISCOUNT,
            (None, None) => (reading.derived.import * reading.derived.export).sqrt(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::transfers::ScoutingRegion;
    use crate::transfers::market_map::{
        CorridorWeight, CountryTransferProfile, DiasporaShare, MarketCountryFacts,
    };

    const BR: u32 = 1;
    const RU: u32 = 2;
    const TR: u32 = 3;
    const PT: u32 = 4;
    const CM: u32 = 5;
    const SA: u32 = 6;
    const ES: u32 = 7;
    const DE: u32 = 8;

    fn facts(
        id: u32,
        code: &str,
        continent: u32,
        rep: u16,
        top: u16,
        wage: u32,
    ) -> MarketCountryFacts {
        MarketCountryFacts {
            id,
            code: code.to_string(),
            continent_id: continent,
            region: ScoutingRegion::from_country(continent, code),
            reputation: rep,
            top_flight_reputation: top,
            median_top_flight_wage: wage,
        }
    }

    fn weight(country_id: u32, weight: f32) -> CorridorWeight {
        CorridorWeight {
            country_id,
            weight,
            money: false,
        }
    }

    fn money(country_id: u32, weight: f32) -> CorridorWeight {
        CorridorWeight {
            country_id,
            weight,
            money: true,
        }
    }

    /// A miniature world with the corridors the design argues about.
    fn world() -> MarketMap {
        let mut facts_map = HashMap::new();
        for f in [
            facts(BR, "br", 3, 8000, 7800, 500_000),
            facts(RU, "ru", 1, 6500, 6500, 900_000),
            facts(TR, "tr", 1, 6700, 7000, 900_000),
            facts(PT, "pt", 1, 8000, 7500, 700_000),
            facts(CM, "cm", 0, 4000, 2000, 20_000),
            facts(SA, "sa", 4, 6000, 6200, 4_000_000),
            facts(ES, "es", 1, 9200, 9200, 2_500_000),
            facts(DE, "de", 1, 9300, 9300, 2_600_000),
        ] {
            facts_map.insert(f.id, f);
        }

        let mut profiles = HashMap::new();
        profiles.insert(
            TR,
            CountryTransferProfile {
                import: vec![weight(BR, 1.0), weight(RU, 0.17), weight(PT, 0.4)],
                export: vec![weight(DE, 1.0)],
                diaspora: vec![],
                foreign_share: 0.58,
                authored: true,
            },
        );
        profiles.insert(
            RU,
            CountryTransferProfile {
                import: vec![weight(BR, 1.0)],
                export: vec![weight(TR, 1.0), money(SA, 0.1)],
                diaspora: vec![],
                foreign_share: 0.35,
                authored: true,
            },
        );
        profiles.insert(
            BR,
            CountryTransferProfile {
                import: vec![],
                export: vec![weight(PT, 1.0), weight(TR, 0.27), money(SA, 0.27)],
                diaspora: vec![],
                foreign_share: 0.07,
                authored: true,
            },
        );
        profiles.insert(
            PT,
            CountryTransferProfile {
                import: vec![weight(BR, 1.0)],
                export: vec![weight(ES, 0.6)],
                diaspora: vec![],
                foreign_share: 0.60,
                authored: true,
            },
        );
        profiles.insert(
            CM,
            CountryTransferProfile {
                import: vec![],
                export: vec![],
                diaspora: vec![],
                foreign_share: 0.06,
                authored: true,
            },
        );
        profiles.insert(
            SA,
            CountryTransferProfile {
                import: vec![money(BR, 1.0)],
                export: vec![],
                diaspora: vec![],
                foreign_share: 0.28,
                authored: true,
            },
        );
        profiles.insert(
            DE,
            CountryTransferProfile {
                import: vec![],
                export: vec![],
                diaspora: vec![DiasporaShare {
                    country_id: TR,
                    share: 0.09,
                }],
                foreign_share: 0.56,
                authored: true,
            },
        );
        MarketMap::new(profiles, facts_map)
    }

    fn affinity(map: &MarketMap, nationality: u32, current: u32, buyer: u32) -> f32 {
        MarketAffinity::affinity(
            map,
            MarketAffinityInputs {
                buyer_country_id: buyer,
                nationality_country_id: nationality,
                current_country_id: current,
                kind: MoveKind::Talent,
            },
        )
    }

    #[test]
    fn going_home_is_always_plausible() {
        let map = world();
        assert_eq!(affinity(&map, CM, ES, CM), 1.0);
    }

    #[test]
    fn the_russia_turkey_corridor_beats_russia_brazil_by_an_order_of_magnitude() {
        let map = world();
        let to_turkey = affinity(&map, RU, RU, TR);
        let to_brazil = affinity(&map, RU, RU, BR);
        assert!(
            to_turkey > 4.0 * to_brazil,
            "Russia → Turkey {to_turkey} must dwarf Russia → Brazil {to_brazil}"
        );
    }

    #[test]
    fn a_russian_reaching_cameroon_sits_on_the_floor() {
        let map = world();
        let to_cameroon = affinity(&map, RU, RU, CM);
        assert!(
            to_cameroon < 0.12,
            "Russia → Cameroon must be near the floor, was {to_cameroon}"
        );
    }

    #[test]
    fn a_brazilian_at_porto_is_a_portugal_market_player() {
        let map = world();
        let at_home = affinity(&map, BR, BR, ES);
        let at_porto = affinity(&map, BR, PT, ES);
        assert!(
            at_porto > at_home,
            "Porto shop window {at_porto} must beat the Brasileirão {at_home}"
        );
    }

    #[test]
    fn money_corridors_reach_where_talent_ones_do_not() {
        let map = world();
        let talent = affinity(&map, CM, CM, SA);
        let wage_led = MarketAffinity::affinity(
            &map,
            MarketAffinityInputs {
                buyer_country_id: SA,
                nationality_country_id: CM,
                current_country_id: CM,
                kind: MoveKind::Money,
            },
        );
        assert!(
            wage_led > talent,
            "the Gulf buys names from anywhere: {wage_led} vs {talent}"
        );
        assert!(wage_led >= map.import_capacity(SA) - 0.001);
    }

    #[test]
    fn money_capacity_does_not_open_a_poor_destination() {
        let map = world();
        let wage_led = MarketAffinity::affinity(
            &map,
            MarketAffinityInputs {
                buyer_country_id: CM,
                nationality_country_id: RU,
                current_country_id: RU,
                kind: MoveKind::Money,
            },
        );
        assert!(
            wage_led < 0.15,
            "Cameroon has no capacity to import a name, was {wage_led}"
        );
    }

    #[test]
    fn diaspora_opens_a_channel_both_ways() {
        let map = world();
        // Germany carries a large Turkish diaspora, so a Turk is a plausible
        // German signing even with no corridor entry either way.
        let with_diaspora = affinity(&map, TR, TR, DE);
        let without = affinity(&map, PT, PT, DE);
        assert!(
            with_diaspora > without,
            "diaspora {with_diaspora} must beat the silent pair {without}"
        );
    }

    #[test]
    fn affinity_never_leaves_the_band() {
        let map = world();
        for nationality in [BR, RU, TR, PT, CM, SA, ES, DE, 999] {
            for buyer in [BR, RU, TR, PT, CM, SA, ES, DE, 999] {
                let value = affinity(&map, nationality, nationality, buyer);
                assert!(
                    (AFFINITY_FLOOR..=1.0).contains(&value),
                    "{nationality} → {buyer} produced {value}"
                );
            }
        }
    }

    #[test]
    fn a_players_own_map_reads_his_export_list_not_the_buyers() {
        let map = world();
        // Brazil's card says its nationals go to Portugal; Portugal's says it
        // buys them. A Brazilian's own map of Portugal is his export list.
        let brazilian_to_portugal = MarketAffinity::player_affinity(&map, BR, PT, 0.0);
        let brazilian_to_cameroon = MarketAffinity::player_affinity(&map, BR, CM, 0.0);
        assert!(brazilian_to_portugal > 0.9);
        assert!(brazilian_to_cameroon < 0.15);
    }

    #[test]
    fn language_alone_makes_a_place_familiar_to_a_player() {
        let map = world();
        let mute = MarketAffinity::player_affinity(&map, CM, RU, 0.0);
        let speaks = MarketAffinity::player_affinity(&map, CM, RU, 0.8);
        assert!(speaks > mute);
        assert!(speaks >= 0.8);
    }
}

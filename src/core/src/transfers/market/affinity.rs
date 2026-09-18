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

use crate::club::board::ownership::ClubBenefactor;
use crate::transfers::market::map::{AFFINITY_FLOOR, CorridorReading, MarketMap};
use crate::transfers::pipeline::trace::MarketSwitches;

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
    /// The BUYER's owner funding, 0..1 —
    /// [`crate::club::board::ownership::ClubOwnership::benefactor`].
    ///
    /// The money corridor used to be a card property only: unless a country
    /// card marked the pair `kind: "money"`, no caller ever passed
    /// [`MoveKind::Money`], so a state-backed club buying out of a
    /// nationality its country's card does not name was priced as an
    /// ordinary talent move and could fall to the reach floor. Owner money
    /// is already a number on the buying club; reading it here makes the
    /// exception continuous rather than a data flag — a 0.3 benefactor gets
    /// 60 % of the relief a fully state-backed one gets.
    ///
    /// `0.0` for every caller that does not know (a country-grain read, a
    /// fixture): the term is a FLOOR, so not knowing costs nothing.
    pub benefactor: f32,
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
    /// How much of the derived prior a named pair may not read below. See
    /// [`Self::blend_corridor`] for why a floor exists at all.
    ///
    /// The knee of a 0 / ¼ / ½ / ¾ / 1 sweep, two full seasons each against
    /// the unfloored baseline (`OF_CORRIDOR_FLOOR_OFF`).
    ///
    /// What the floor costs is measurable and small: `corridor_overlap`
    /// against the top-8 export list falls monotonically, 0.500 at zero to
    /// 0.465 at the whole prior. Every other census number — foreign share
    /// against the card, the free-agent and permanent overlaps, the loan
    /// route bands — sits inside run-to-run noise at n=2.
    ///
    /// What it BUYS cannot be seen in any of them, because a move
    /// `thresholds::MARKET_REACH_FLOOR` closes never becomes a row anywhere.
    /// Counted directly off the cards, for a buyer with no particular
    /// knowledge of the market, that floor shuts 2353 of the 4556 ordered
    /// pairs between countries running a league. A half share reopens 300 of
    /// them for 0.020 of overlap; a quarter reopens 36 for the same 0.020;
    /// the whole prior reopens 478 for 0.035. Half is where the curve turns.
    const CORRIDOR_PRIOR_FLOOR_SHARE: f32 = 0.5;

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
        //
        // Two things can declare a move wage-led — the CARD (a country pair
        // marked `kind: "money"`, the Gulf and MLS corridors) and the BUYER
        // (an owner writing the cheques). The card is a step; the buyer is a
        // dial, so it enters as a share of the same capacity floor and a
        // half-funded club gets half the relief. Both are capped by the
        // destination's own capacity to buy names, which is why this does
        // not open Yaoundé to anybody however rich its owner is.
        let owner_share = if inputs.kind == MoveKind::Money || nationality.money {
            1.0
        } else {
            (inputs.benefactor.clamp(0.0, 1.0) / ClubBenefactor::STATE_BACKED_BAR).clamp(0.0, 1.0)
        };
        if owner_share > 0.0 {
            geo = geo.max(owner_share * map.import_capacity(inputs.buyer_country_id));
        }

        geo.clamp(AFFINITY_FLOOR, 1.0)
    }

    /// How plausible this destination is for a LOAN of this player, 0.02..1.
    ///
    /// A permanent signing is an acquisition, and [`Self::affinity`] prices
    /// it correctly: the buyer answers to the player's own corridor, and the
    /// league he happens to sit in is a shop window on top of it. A loan is
    /// an agreement between two CLUBS that outlives the signature — one hands
    /// over an asset, plays it for a season and gives it back — so the route
    /// between the two leagues is not a bonus on the passport, it is half the
    /// question. Japan signs Brazilians and does not borrow from Russia, and
    /// a passport-only read cannot tell those two apart: it scores a
    /// Brazilian at Zenit exactly as it scores the same man at Flamengo.
    ///
    /// The geometric mean, for the reason [`Self::blend_corridor`] uses one:
    /// a loan needs both sides to be real, and either alone is a claim.
    pub fn loan_affinity(map: &MarketMap, inputs: MarketAffinityInputs) -> f32 {
        let player = Self::affinity(map, inputs);
        if MarketSwitches::loan_route_off() {
            return player;
        }
        // No second league in the deal, so there is no route to price. He is
        // going home (his own federation is a route of its own — that is what
        // keeps the loan-home pathway untouched), he already plays here, his
        // club's country IS his passport's so the route and the corridor are
        // one corridor read twice, or nobody knows where he plays.
        if inputs.nationality_country_id == inputs.buyer_country_id
            || inputs.current_country_id == 0
            || inputs.current_country_id == inputs.buyer_country_id
            || inputs.current_country_id == inputs.nationality_country_id
        {
            return player;
        }
        // The model's own read, so the corridor-floor arm moves this term
        // with every other one. `corridor_strength` is the census ruler and
        // deliberately ignores that arm.
        let route =
            Self::blend_corridor(&map.corridor(inputs.current_country_id, inputs.buyer_country_id));
        (player * route).sqrt().clamp(AFFINITY_FLOOR, 1.0)
    }

    /// What two markets are worth to each other, 0..1, with no player in the
    /// middle — the one number a census can read to say whether two leagues
    /// do business at all.
    ///
    /// Always floored at the derived prior, whatever
    /// [`MarketSwitches::corridor_floor_off`] says, because this is a RULER
    /// and the arm is a model. A census that read the arm would move its own
    /// measurement with the thing it is measuring, and the route bands would
    /// improve by definition rather than by behaviour.
    pub fn corridor_strength(map: &MarketMap, from_country: u32, to_country: u32) -> f32 {
        let reading = map.corridor(from_country, to_country);
        let prior = (reading.derived.import * reading.derived.export).sqrt();
        Self::blend_corridor(&reading).max(prior)
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
        // A man who speaks the place fluently already has his one strong
        // reason; the maximum below cannot beat 1.0, so reading the corridor
        // first is work with no consequence.
        let language = language_affinity.clamp(0.0, 1.0);
        if language >= 1.0 {
            return 1.0;
        }
        let corridor = map.corridor(nationality_country_id, buyer_country_id);
        let export = corridor.data_export.unwrap_or(corridor.derived.export);
        let diaspora = map.diaspora_link(nationality_country_id, buyer_country_id);
        export
            .max(diaspora)
            .max(language)
            .clamp(AFFINITY_FLOOR, 1.0)
    }

    /// Geometric mean of the two sides when both cards name the corridor;
    /// the named side discounted when only one does; the derived pair when
    /// neither does — and never less than that derived pair.
    ///
    /// The geometric mean is what makes a corridor need agreement: a
    /// destination that says it buys Brazilians and a Brazil card that says
    /// its nationals go there is a corridor, and either one alone is a
    /// claim.
    ///
    /// The floor is there because a card weight and a corridor are not the
    /// same quantity. A weight is a VOLUME SHARE, normalised by its own
    /// list's maximum; a corridor is a PLAUSIBILITY. Portugal importing
    /// fifteen Brazilians for every Italian is the card being right, and it
    /// does not make an Italian at a Portuguese club implausible — but the
    /// tail of every card normalises to 0.03, the geometric mean squares that
    /// to 0.023, and `thresholds::MARKET_REACH_FLOOR` is 0.05. Measured
    /// across the shipped cards, 1501 of the 2264 ordered pairs some card
    /// names read BELOW their own derived prior, England's 25-country import
    /// list undercutting it 69 times: naming a pair was making it less of a
    /// corridor than never mentioning it. Data raises a prior; it does not
    /// lower one.
    ///
    /// A SHARE of the prior rather than all of it, because the two readings
    /// disagree for a reason. A pair the cards name faintly is a pair the
    /// world has looked at and found little traffic on, and that is worth
    /// something against the prior's structural guess — the floor is there to
    /// stop the card reading as a denial, not to make it say nothing at all.
    /// See [`Self::CORRIDOR_PRIOR_FLOOR_SHARE`].
    fn blend_corridor(reading: &CorridorReading) -> f32 {
        let prior = (reading.derived.import * reading.derived.export).sqrt();
        let data = match (reading.data_import, reading.data_export) {
            (Some(import), Some(export)) => (import * export).sqrt(),
            (Some(import), None) => import * Self::MISSING_SIDE_DISCOUNT,
            (None, Some(export)) => export * Self::MISSING_SIDE_DISCOUNT,
            (None, None) => return prior,
        };
        if MarketSwitches::corridor_floor_off() {
            return data;
        }
        data.max(prior * Self::CORRIDOR_PRIOR_FLOOR_SHARE)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::transfers::ScoutingRegion;
    use crate::transfers::market::map::{
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
    const JP: u32 = 9;

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
            facts(JP, "jp", 4, 6500, 6200, 1_500_000),
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
                export: vec![
                    weight(PT, 1.0),
                    weight(JP, 0.33),
                    weight(TR, 0.27),
                    money(SA, 0.27),
                ],
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
        profiles.insert(
            JP,
            CountryTransferProfile {
                import: vec![weight(BR, 1.0)],
                export: vec![weight(DE, 1.0)],
                diaspora: vec![],
                foreign_share: 0.15,
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
                benefactor: 0.0,
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

    fn loan(map: &MarketMap, nationality: u32, current: u32, buyer: u32) -> f32 {
        MarketAffinity::loan_affinity(
            map,
            MarketAffinityInputs {
                buyer_country_id: buyer,
                nationality_country_id: nationality,
                current_country_id: current,
                kind: MoveKind::Talent,
                benefactor: 0.0,
            },
        )
    }

    #[test]
    fn a_loan_is_priced_on_the_route_between_the_two_clubs() {
        let map = world();
        // Japan imports Brazilians heavily and does no business at all with
        // Russia. Read on the passport alone the two are the same player, so
        // a J-League club borrowed a Brazilian from Zenit as readily as from
        // Flamengo — which is the move this exists to stop.
        let from_home = loan(&map, BR, BR, JP);
        let from_russia = loan(&map, BR, RU, JP);
        assert!(
            from_home > 2.0 * from_russia,
            "Flamengo {from_home} must dwarf Zenit {from_russia}"
        );
        // …and the permanent read, which answers a different question, still
        // cannot tell them apart. That difference is the whole point.
        let permanent = affinity(&map, BR, RU, JP);
        assert!(permanent > 2.0 * from_russia, "was {permanent}");
    }

    #[test]
    fn a_loan_along_a_worked_route_is_untouched() {
        let map = world();
        // Russia's card exports to Turkey at the top of its list, so a
        // Brazilian at a Russian club is a Turkish club's ordinary loan-in
        // and must not pay for the border twice.
        let brazilian_to_turkey = loan(&map, BR, RU, TR);
        assert!(brazilian_to_turkey > 0.4, "was {brazilian_to_turkey}");
    }

    #[test]
    fn a_faint_card_entry_never_reads_below_knowing_nothing() {
        let map = world();
        // Turkey's card imports Russians at 0.17 — a real but minor market,
        // and the two are neighbours on the reputation ladder, so the derived
        // prior speaks louder than the share does. The card must raise that
        // prior or stay out of its way; measured on the shipped data it did
        // neither, and Italy → Portugal read 0.047 against a 0.31 prior.
        let named = MarketAffinity::corridor_strength(&map, RU, TR);
        let silent = MarketAffinity::corridor_strength(&map, ES, PT);
        let prior = {
            let reading = map.corridor(RU, TR);
            (reading.derived.import * reading.derived.export).sqrt()
        };
        assert!(
            named >= prior,
            "card {named} must not undercut prior {prior}"
        );
        assert!(silent > 0.2, "two Western European neighbours: {silent}");
    }

    #[test]
    fn the_model_floors_a_named_pair_at_its_share_of_the_prior() {
        let map = world();
        // The MODEL path, as distinct from the ruler above it: a named pair
        // may sit below the prior — the cards finding little traffic is
        // evidence — but not arbitrarily far below it.
        for from in [BR, RU, TR, PT, CM, SA, ES, DE, JP] {
            for to in [BR, RU, TR, PT, CM, SA, ES, DE, JP] {
                let reading = map.corridor(from, to);
                let prior = (reading.derived.import * reading.derived.export).sqrt();
                let blended = MarketAffinity::blend_corridor(&reading);
                let bar = prior * MarketAffinity::CORRIDOR_PRIOR_FLOOR_SHARE;
                assert!(blended >= bar - 1e-6, "{from} → {to}: {blended} < {bar}");
            }
        }
    }

    #[test]
    fn the_corridor_floor_only_ever_raises_a_pair() {
        let map = world();
        // Every pair, both directions: the floor is a MAX, so nothing the
        // cards say can come out lower than it went in. The arm that disarms
        // it is the A/B baseline, not a second model.
        for from in [BR, RU, TR, PT, CM, SA, ES, DE, JP] {
            for to in [BR, RU, TR, PT, CM, SA, ES, DE, JP] {
                let reading = map.corridor(from, to);
                let prior = (reading.derived.import * reading.derived.export).sqrt();
                let blended = MarketAffinity::corridor_strength(&map, from, to);
                assert!(
                    blended >= prior - 1e-6,
                    "{from} → {to}: {blended} < {prior}"
                );
            }
        }
    }

    #[test]
    fn the_route_never_touches_a_loan_home() {
        let map = world();
        // The loan-home pathway is the one cross-border route that answers to
        // the player rather than to the two leagues: Russia and Brazil do no
        // business, and a Brazilian at a Russian club still goes home.
        assert_eq!(loan(&map, BR, RU, BR), 1.0);
        assert_eq!(loan(&map, CM, ES, CM), 1.0);
    }

    #[test]
    fn a_man_at_home_reads_one_corridor_not_two() {
        let map = world();
        // His club's country IS his passport's, so the route and the
        // nationality corridor are the same corridor. Squaring it would make
        // every loan out of a player's own country implausible.
        for buyer in [TR, PT, ES, JP, SA] {
            assert_eq!(loan(&map, BR, BR, buyer), affinity(&map, BR, BR, buyer));
        }
    }

    #[test]
    fn loan_affinity_never_leaves_the_band() {
        let map = world();
        for nationality in [BR, RU, TR, PT, CM, SA, ES, DE, JP, 999] {
            for current in [BR, RU, CM, ES, JP, 0] {
                for buyer in [BR, RU, TR, PT, CM, SA, ES, DE, JP, 999] {
                    let value = loan(&map, nationality, current, buyer);
                    assert!(
                        (AFFINITY_FLOOR..=1.0).contains(&value),
                        "{nationality} @ {current} → {buyer} produced {value}"
                    );
                }
            }
        }
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
                benefactor: 0.0,
            },
        );
        assert!(
            wage_led > talent,
            "the Gulf buys names from anywhere: {wage_led} vs {talent}"
        );
        assert!(wage_led >= map.import_capacity(SA) - 0.001);
    }

    #[test]
    fn owner_money_reaches_where_the_card_does_not_and_scales_with_the_owner() {
        let map = world();
        // A Cameroonian to Saudi Arabia with no `Money` kind and no money
        // mark reachable from the CM side. Before the benefactor was read
        // here, no caller ever passed `MoveKind::Money`, so a state-backed
        // club buying out of a nationality its card does not name was
        // priced as an ordinary talent move.
        let with_owner = |benefactor: f32| {
            MarketAffinity::affinity(
                &map,
                MarketAffinityInputs {
                    buyer_country_id: SA,
                    nationality_country_id: CM,
                    current_country_id: CM,
                    kind: MoveKind::Talent,
                    benefactor,
                },
            )
        };
        let none = with_owner(0.0);
        let half = with_owner(0.25);
        let full = with_owner(ClubBenefactor::STATE_BACKED_BAR);
        assert!(half > none, "an owner opens a door: {half} vs {none}");
        assert!(full > half, "and a bigger one opens it wider: {full}");
        // Fully state-backed reads the same as the card's own money mark.
        assert!((full - map.import_capacity(SA)).abs() < 0.001);
        // Past the bar it saturates rather than compounding.
        assert!((with_owner(1.0) - full).abs() < 0.001);
    }

    #[test]
    fn owner_money_cannot_open_a_destination_with_no_capacity() {
        let map = world();
        // The floor is the DESTINATION's capacity to buy names. However
        // rich a Cameroonian club's owner, Yaoundé does not become a market
        // that signs Russians.
        let wage_led = MarketAffinity::affinity(
            &map,
            MarketAffinityInputs {
                buyer_country_id: CM,
                nationality_country_id: RU,
                current_country_id: RU,
                kind: MoveKind::Talent,
                benefactor: 1.0,
            },
        );
        assert!(wage_led < 0.15, "was {wage_led}");
    }

    #[test]
    fn a_fluent_speaker_reads_a_place_as_familiar_without_the_corridor() {
        let map = world();
        // The short-circuit: nothing the corridor could say beats 1.0, so
        // fluency answers on its own.
        assert_eq!(MarketAffinity::player_affinity(&map, RU, CM, 1.0), 1.0);
        assert!(MarketAffinity::player_affinity(&map, RU, CM, 0.0) < 0.15);
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
                benefactor: 0.0,
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

//! The world's transfer GEOGRAPHY: who moves where, and how well anyone
//! could know it.
//!
//! Football's transfer market is not a global list sorted by need. It is a
//! set of corridors — Brazil → Portugal, francophone Africa → France, Russia
//! → Turkey — each one directional, each one a product of language, empire,
//! diaspora and money rather than of proximity or league strength. A model
//! with no notion of corridors sends a Russian to a Brazilian club because
//! nothing in the arithmetic knows that this has never happened.
//!
//! The corridors themselves live in the data (`data/{cc}/country.json`,
//! compiled into the `country_transfers` table), because they are facts about
//! the real world and not parameters of a simulation. This module turns that
//! table into the runtime shape every transfer path reads:
//!
//!   * [`CountryTransferProfile`] — one country's card, normalised;
//!   * [`MarketMap`] — the whole world's cards plus the facts (reputation,
//!     region, wage level) the derived fallback and `import_capacity` need;
//!   * [`CorridorPrior::derive`] — what a pair the data does not name is
//!     worth, computed from language, region and the reputation ladder, so a
//!     data hole fails to a considered number and never to 1.0.
//!
//! The priors only WEIGHT and GATE. Nothing here picks a destination: no path
//! may iterate an export list to choose where a player goes, or the card
//! stops being a prior and becomes a routing table.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::club::player::personality::Language;
use crate::transfers::ScoutingRegion;

/// Never zero. A world must be able to open a corridor that has never
/// existed — a scout is hired, a signing works, and twenty years later the
/// save has a Venezuela pipeline the shipped data never named.
pub const AFFINITY_FLOOR: f32 = 0.02;

/// One entry in a country's `import` or `export` list.
#[derive(Debug, Clone, Copy)]
pub struct CorridorWeight {
    pub country_id: u32,
    /// Normalised to 0..1 by the list's own maximum, so a country's top
    /// corridor is 1.0 and everything else is a share of it. Comparable
    /// across countries with very different list lengths.
    pub weight: f32,
    /// A wage-led landing rather than a talent corridor — Riyadh, not
    /// Rotterdam. Money moves answer to the wage term, and the geography
    /// term yields to the destination's import capacity.
    pub money: bool,
}

/// Share of a nationality's professional pool raised in another country.
/// Read both ways: the host country imports the nationality cheaply, and the
/// home country recruits its own diaspora back.
#[derive(Debug, Clone, Copy)]
pub struct DiasporaShare {
    pub country_id: u32,
    pub share: f32,
}

/// One country's transfer-market card.
#[derive(Debug, Clone, Default)]
pub struct CountryTransferProfile {
    /// Where this country's CLUBS buy foreigners from (source nationality →
    /// clubs of this country).
    pub import: Vec<CorridorWeight>,
    /// Where this country's NATIONALS go (this nationality → clubs of that
    /// country).
    pub export: Vec<CorridorWeight>,
    pub diaspora: Vec<DiasporaShare>,
    /// Typical foreign share of the top division's registered players, 0..1.
    pub foreign_share: f32,
    /// True when a human corrected the derived draft. Diagnostics only.
    pub authored: bool,
}

impl CountryTransferProfile {
    pub fn import_weight(&self, country_id: u32) -> Option<f32> {
        Self::lookup(&self.import, country_id)
    }

    pub fn export_weight(&self, country_id: u32) -> Option<f32> {
        Self::lookup(&self.export, country_id)
    }

    /// True when either side of the pair is marked as a wage-led landing.
    pub fn is_money_corridor(&self, country_id: u32) -> bool {
        self.import
            .iter()
            .chain(self.export.iter())
            .any(|c| c.country_id == country_id && c.money)
    }

    pub fn diaspora_share(&self, country_id: u32) -> f32 {
        self.diaspora
            .iter()
            .find(|d| d.country_id == country_id)
            .map(|d| d.share)
            .unwrap_or(0.0)
    }

    fn lookup(list: &[CorridorWeight], country_id: u32) -> Option<f32> {
        list.iter()
            .find(|c| c.country_id == country_id)
            .map(|c| c.weight)
    }
}

/// The facts about one country the geography model needs, denormalised so a
/// per-country borrow can answer questions about a country it cannot reach.
#[derive(Debug, Clone)]
pub struct MarketCountryFacts {
    pub id: u32,
    pub code: String,
    pub continent_id: u32,
    pub region: ScoutingRegion,
    /// Football-ecosystem reputation, 0..10000.
    pub reputation: u16,
    /// Reputation of the strongest league this country runs, 0 when it runs
    /// none in this save.
    pub top_flight_reputation: u16,
    /// Median annual wage in the country's top division, 0 when unknown.
    /// The money axis of [`MarketMap::import_capacity`].
    pub median_top_flight_wage: u32,
}

/// What a country pair is worth when the data does not name it.
#[derive(Debug, Clone, Copy)]
pub struct CorridorPrior {
    /// Destination buys from source.
    pub import: f32,
    /// Source's nationals go to destination.
    pub export: f32,
}

impl CorridorPrior {
    /// Weight of a shared language.
    const LANGUAGE: f32 = 0.35;
    /// Weight of sitting in the same scouting region.
    const SAME_REGION: f32 = 0.25;
    /// Weight of the hand-authored region corridor table, normalised.
    const REGION_CORRIDOR: f32 = 0.20;
    /// Weight of the reputation ladder — the only asymmetric term.
    const LADDER: f32 = 0.20;
    /// Reputation gap at which the ladder term saturates. Roughly the
    /// distance from a mid-table European league to the Premier League.
    const LADDER_SPAN: f32 = 3000.0;
    /// A corridor that crosses continents is worth half as much before any
    /// of the above; oceans are the strongest filter in real transfer data
    /// after language.
    const CROSS_CONTINENT: f32 = 0.5;

    /// The prior for a pair no card names. Continuous, deterministic, and
    /// the same for everyone — two clubs in the same country derive the
    /// same number for the same source market.
    ///
    /// Only the ladder term is asymmetric, and it is the term that makes the
    /// asymmetry real: nationals move UP the ladder (export), and clubs buy
    /// at or BELOW their own level (import). Everything else about a pair —
    /// language, region, continent — is shared by both directions.
    pub fn derive(from: &MarketCountryFacts, to: &MarketCountryFacts) -> CorridorPrior {
        let shared_language = {
            let from_mask = Language::country_language_mask(&from.code);
            let to_mask = Language::country_language_mask(&to.code);
            if from_mask != 0 && to_mask != 0 && from_mask & to_mask != 0 {
                1.0
            } else {
                0.0
            }
        };
        let same_region = if from.region == to.region { 1.0 } else { 0.0 };
        let region_corridor = Self::region_corridor_weight(to.region, from.region);

        let base = Self::LANGUAGE * shared_language
            + Self::SAME_REGION * same_region
            + Self::REGION_CORRIDOR * region_corridor;

        // +1 when the destination is far stronger, −1 when far weaker. Read
        // off the LEAGUE standard rather than the ecosystem reputation: the
        // question a move answers is what competition he will play in.
        let ladder = ((to.top_flight_reputation as f32 - from.top_flight_reputation as f32)
            / Self::LADDER_SPAN)
            .clamp(-1.0, 1.0);

        let scale = if from.continent_id == to.continent_id {
            1.0
        } else {
            Self::CROSS_CONTINENT
        };

        CorridorPrior {
            import: ((base + Self::LADDER * (0.5 - 0.5 * ladder)) * scale)
                .clamp(AFFINITY_FLOOR, 1.0),
            export: ((base + Self::LADDER * (0.5 + 0.5 * ladder)) * scale)
                .clamp(AFFINITY_FLOOR, 1.0),
        }
    }

    /// The existing hand-authored region corridor table, normalised to 0..1
    /// by the destination region's own heaviest corridor. Reused rather than
    /// replaced: it is a coarse but real reading of where scouting networks
    /// point, and it is the only geography the engine had before the country
    /// cards existed.
    ///
    /// Read in the direction the table is written: `X.transfer_corridors()`
    /// lists the regions X's CLUBS look in, so the entry that describes a
    /// move from S to D is `S` inside `D`'s list — one number for the whole
    /// flow, in one direction. Reading it the other way round is what would
    /// make Russia → Brazil a corridor: Eastern European clubs do scout South
    /// America, and that says nothing about where Russians go.
    fn region_corridor_weight(destination: ScoutingRegion, source: ScoutingRegion) -> f32 {
        let corridors = destination.transfer_corridors();
        let max = corridors.iter().map(|(_, w)| *w).max().unwrap_or(0);
        if max == 0 {
            return 0.0;
        }
        corridors
            .iter()
            .find(|(region, _)| *region == source)
            .map(|(_, weight)| *weight as f32 / max as f32)
            .unwrap_or(0.0)
    }
}

/// Every country's card plus the facts the derived fallback needs, built
/// once at world load and refreshed when the wage world has moved.
///
/// Sized for the whole planet (224 nationalities × 224) but never
/// materialised as a matrix: the cards are sparse lists and the fallback is a
/// pure function, so the map is a few hundred kilobytes and a lookup is a
/// short linear scan of one country's list.
#[derive(Debug, Clone, Default)]
pub struct MarketMap {
    profiles: HashMap<u32, CountryTransferProfile>,
    facts: HashMap<u32, MarketCountryFacts>,
    /// Precomputed per country — the money axis needs a world median, so it
    /// cannot be answered one country at a time.
    import_capacity: HashMap<u32, f32>,
    /// The neutral profile handed out for a country with no card at all.
    empty: CountryTransferProfile,
}

impl MarketMap {
    /// Foreign share at which the "this league imports" axis saturates.
    /// Portugal and Cyprus live above it; England sits near it.
    const FOREIGN_SHARE_SPAN: f32 = 0.55;
    /// Multiple of the world median top-flight wage at which the money axis
    /// saturates. The Gulf and the Premier League clear it; a league paying
    /// the world median scores 0.4.
    const WAGE_SPAN: f32 = 2.5;

    pub fn new(
        profiles: HashMap<u32, CountryTransferProfile>,
        facts: HashMap<u32, MarketCountryFacts>,
    ) -> MarketMap {
        let mut map = MarketMap {
            profiles,
            facts,
            import_capacity: HashMap::new(),
            empty: CountryTransferProfile::default(),
        };
        map.recompute_import_capacity();
        map
    }

    pub fn is_empty(&self) -> bool {
        self.facts.is_empty()
    }

    pub fn profile(&self, country_id: u32) -> &CountryTransferProfile {
        self.profiles.get(&country_id).unwrap_or(&self.empty)
    }

    pub fn facts(&self, country_id: u32) -> Option<&MarketCountryFacts> {
        self.facts.get(&country_id)
    }

    /// How readily this country's clubs sign names from outside their own
    /// corridors, 0..1. Money and an established habit of importing, in
    /// equal measure — the Gulf, MLS and Japan run high, Turkey and Russia
    /// mid, Cameroon near zero.
    ///
    /// This is the discriminator the free-agent gates were missing: "leagues
    /// that recruit across regions chase names" is true of Riyadh and false
    /// of Yaoundé, and standing alone cannot tell them apart.
    pub fn import_capacity(&self, country_id: u32) -> f32 {
        self.import_capacity
            .get(&country_id)
            .copied()
            .unwrap_or(0.0)
    }

    /// The corridor prior for a pair, data first and the derived fallback
    /// behind it. `None` on the data side is meaningful — see
    /// [`crate::transfers::MarketAffinity`], which discounts a corridor only
    /// one of the two cards names.
    pub fn corridor(&self, from_country: u32, to_country: u32) -> CorridorReading {
        let data_import = self.profile(to_country).import_weight(from_country);
        let data_export = self.profile(from_country).export_weight(to_country);
        let money = self.profile(to_country).is_money_corridor(from_country)
            || self.profile(from_country).is_money_corridor(to_country);
        let derived = match (self.facts(from_country), self.facts(to_country)) {
            (Some(from), Some(to)) => CorridorPrior::derive(from, to),
            // A country the world does not know at all fails to the floor
            // rather than to a guess. Mirrors the free-agent snapshot's
            // fail-closed handling of an unknown nationality.
            _ => CorridorPrior {
                import: AFFINITY_FLOOR,
                export: AFFINITY_FLOOR,
            },
        };
        CorridorReading {
            data_import,
            data_export,
            derived,
            money,
        }
    }

    /// Diaspora of `nationality` living in `host`, plus half the reverse —
    /// the channel runs both ways, but a country recruiting its own diaspora
    /// abroad is the weaker half of it.
    pub fn diaspora_link(&self, nationality: u32, host: u32) -> f32 {
        self.profile(nationality).diaspora_share(host)
            + 0.5 * self.profile(host).diaspora_share(nationality)
    }

    /// Rebuild the per-country import capacity from the current facts. Cheap
    /// (one pass plus a median), so it can be re-run whenever the wage world
    /// has moved — the transfer-window boundaries are the natural cadence.
    pub fn recompute_import_capacity(&mut self) {
        let mut wages: Vec<u32> = self
            .facts
            .values()
            .map(|f| f.median_top_flight_wage)
            .filter(|w| *w > 0)
            .collect();
        wages.sort_unstable();
        let world_median = wages.get(wages.len() / 2).copied().unwrap_or(0) as f32;

        self.import_capacity = self
            .facts
            .iter()
            .map(|(id, facts)| {
                let foreign_axis = (self
                    .profiles
                    .get(id)
                    .map(|p| p.foreign_share)
                    .unwrap_or(0.0)
                    / Self::FOREIGN_SHARE_SPAN)
                    .clamp(0.0, 1.0);
                let wage_axis = if world_median > 0.0 {
                    (facts.median_top_flight_wage as f32 / world_median / Self::WAGE_SPAN)
                        .clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (*id, (0.5 * foreign_axis + 0.5 * wage_axis).clamp(0.0, 1.0))
            })
            .collect();
    }

    /// Overwrite one country's wage level and rebuild the capacities. Used
    /// by the periodic refresh, which reads the live world.
    pub fn set_median_wage(&mut self, country_id: u32, wage: u32) {
        if let Some(facts) = self.facts.get_mut(&country_id) {
            facts.median_top_flight_wage = wage;
        }
    }

    /// Region prestige computed from the loaded world: the strongest top
    /// division in each region, on the same 0..1 scale the hand table used.
    /// Regions with no loaded league keep their authored value.
    ///
    /// This is what corrects the inversion the hand table shipped with —
    /// Turkey (MiddleEastEurope, 0.40) ranked below Russia (EasternEurope,
    /// 0.50) while the Süper Lig ships at 7000 against the RPL's 6500, and
    /// every prestige gate in the transfer market read the wrong order.
    pub fn region_prestige_table(&self) -> [f32; ScoutingRegion::COUNT] {
        let mut best = [0u16; ScoutingRegion::COUNT];
        for facts in self.facts.values() {
            let slot = &mut best[facts.region.index()];
            *slot = (*slot).max(facts.top_flight_reputation);
        }
        let mut table = [0.0f32; ScoutingRegion::COUNT];
        for (index, region) in ScoutingRegion::all().iter().enumerate() {
            table[index] = if best[index] > 0 {
                (best[index] as f32 / 10_000.0).clamp(0.05, 1.0)
            } else {
                region.authored_league_prestige()
            };
        }
        table
    }
}

/// One pair, as the map reads it: what each card says (if anything) and what
/// the fallback would say.
#[derive(Debug, Clone, Copy)]
pub struct CorridorReading {
    /// Destination's card names the source as an import market.
    pub data_import: Option<f32>,
    /// Source's card names the destination as an export market.
    pub data_export: Option<f32>,
    pub derived: CorridorPrior,
    /// Either card marks the pair as a wage-led landing.
    pub money: bool,
}

/// Region prestige, published once per world load and read from everywhere.
///
/// The gates that price a step down (free-agent clearing, the loan market,
/// the player's own place term) sit inside per-country borrows that cannot
/// reach `SimulatorData`, and there are two dozen of them. Threading a table
/// through every one would be a large refactor for a value that is constant
/// for the life of a save, so it is published here the way the player-id
/// sequence already is — seeded at load, read with a relaxed atomic load,
/// and falling back to the authored constants when nothing has seeded it
/// (unit tests, fixtures, the pre-load window).
pub struct RegionPrestigeTable;

/// f32 bits per region; `0` (which is `+0.0`, never a valid prestige) means
/// "not seeded, use the authored constant".
static REGION_PRESTIGE_BITS: [AtomicU32; ScoutingRegion::COUNT] =
    [const { AtomicU32::new(0) }; ScoutingRegion::COUNT];

impl RegionPrestigeTable {
    /// Publish a table computed from the loaded world.
    pub fn publish(table: [f32; ScoutingRegion::COUNT]) {
        for (slot, value) in REGION_PRESTIGE_BITS.iter().zip(table.iter()) {
            slot.store(value.to_bits(), Ordering::Relaxed);
        }
    }

    /// Drop back to the authored constants. Test-facing; a world load always
    /// republishes.
    pub fn clear() {
        for slot in REGION_PRESTIGE_BITS.iter() {
            slot.store(0, Ordering::Relaxed);
        }
    }

    /// Prestige for one region — the loaded world's answer when there is
    /// one, the authored constant otherwise.
    pub fn get(region: ScoutingRegion) -> f32 {
        let bits = REGION_PRESTIGE_BITS[region.index()].load(Ordering::Relaxed);
        if bits == 0 {
            region.authored_league_prestige()
        } else {
            f32::from_bits(bits)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(
        id: u32,
        code: &str,
        continent_id: u32,
        reputation: u16,
        top_flight: u16,
    ) -> MarketCountryFacts {
        MarketCountryFacts {
            id,
            code: code.to_string(),
            continent_id,
            region: ScoutingRegion::from_country(continent_id, code),
            reputation,
            top_flight_reputation: top_flight,
            median_top_flight_wage: 0,
        }
    }

    #[test]
    fn derived_prior_is_directional_on_the_ladder() {
        let brazil = facts(1, "br", 3, 8000, 7800);
        let england = facts(2, "gb", 1, 9500, 9500);
        let up = CorridorPrior::derive(&brazil, &england);
        let down = CorridorPrior::derive(&england, &brazil);
        // A Brazilian going to England is a step UP: his export prior beats
        // England's prior for buying from him? No — both are real. What must
        // hold is that the ladder tilts each direction the right way.
        assert!(
            up.export > up.import,
            "nationals move up the ladder: {up:?}"
        );
        assert!(
            down.import > down.export,
            "clubs buy at or below their own level: {down:?}"
        );
    }

    #[test]
    fn shared_language_beats_a_silent_pair() {
        let portugal = facts(1, "pt", 1, 8000, 7500);
        let brazil = facts(2, "br", 3, 8000, 7800);
        let poland = facts(3, "pl", 1, 6000, 5500);
        let lusophone = CorridorPrior::derive(&brazil, &portugal);
        let silent = CorridorPrior::derive(&brazil, &poland);
        assert!(
            lusophone.export > silent.export,
            "Brazil → Portugal must outrank Brazil → Poland: {lusophone:?} vs {silent:?}"
        );
    }

    #[test]
    fn crossing_continents_halves_the_prior() {
        let serbia = facts(1, "rs", 1, 6000, 5200);
        let croatia = facts(2, "hr", 1, 6200, 5400);
        let kenya = facts(3, "ke", 0, 3000, 2400);
        let near = CorridorPrior::derive(&serbia, &croatia);
        let far = CorridorPrior::derive(&serbia, &kenya);
        assert!(near.export > 2.0 * far.export, "{near:?} vs {far:?}");
    }

    #[test]
    fn an_unknown_pair_never_derives_to_one() {
        let map = MarketMap::default();
        let reading = map.corridor(404, 405);
        assert_eq!(reading.derived.import, AFFINITY_FLOOR);
        assert_eq!(reading.derived.export, AFFINITY_FLOOR);
        assert!(reading.data_import.is_none());
    }

    #[test]
    fn import_capacity_separates_the_gulf_from_west_africa() {
        let mut facts_map = HashMap::new();
        let mut saudi = facts(1, "sa", 4, 6000, 6200);
        saudi.median_top_flight_wage = 4_000_000;
        let mut cameroon = facts(2, "cm", 0, 4000, 2000);
        cameroon.median_top_flight_wage = 20_000;
        let mut england = facts(3, "gb", 1, 9500, 9500);
        england.median_top_flight_wage = 3_000_000;
        let mut turkey = facts(4, "tr", 1, 6700, 7000);
        turkey.median_top_flight_wage = 900_000;
        for f in [saudi, cameroon, england, turkey] {
            facts_map.insert(f.id, f);
        }
        let mut profiles = HashMap::new();
        for (id, share) in [(1u32, 0.28f32), (2, 0.06), (3, 0.60), (4, 0.58)] {
            profiles.insert(
                id,
                CountryTransferProfile {
                    foreign_share: share,
                    ..Default::default()
                },
            );
        }
        let map = MarketMap::new(profiles, facts_map);
        assert!(map.import_capacity(1) > 0.5, "Saudi must buy names");
        assert!(map.import_capacity(2) < 0.15, "Cameroon must not");
        assert!(map.import_capacity(3) > 0.6, "England must buy names");
    }

    #[test]
    fn region_prestige_table_reads_the_world_not_the_hand_table() {
        let mut facts_map = HashMap::new();
        // Süper Lig above the RPL — the inversion the authored table shipped.
        for f in [
            facts(1, "tr", 1, 6700, 7000),
            facts(2, "ru", 1, 6500, 6500),
            facts(3, "br", 3, 8000, 7800),
        ] {
            facts_map.insert(f.id, f);
        }
        let map = MarketMap::new(HashMap::new(), facts_map);
        let table = map.region_prestige_table();
        assert!(
            table[ScoutingRegion::MiddleEastEurope.index()]
                > table[ScoutingRegion::EasternEurope.index()],
            "Turkey must outrank Russia once the data speaks"
        );
        assert!(
            (table[ScoutingRegion::SouthAmerica.index()] - 0.78).abs() < 0.001,
            "Brazil's 7800 must read as 0.78"
        );
        // A region with no loaded league keeps its authored value.
        assert_eq!(
            table[ScoutingRegion::SouthAsia.index()],
            ScoutingRegion::SouthAsia.authored_league_prestige()
        );
    }

    #[test]
    fn published_prestige_wins_over_the_authored_constant() {
        RegionPrestigeTable::clear();
        assert_eq!(
            RegionPrestigeTable::get(ScoutingRegion::EastAsia),
            ScoutingRegion::EastAsia.authored_league_prestige()
        );
        let mut table = [0.0f32; ScoutingRegion::COUNT];
        for (index, region) in ScoutingRegion::all().iter().enumerate() {
            table[index] = region.authored_league_prestige();
        }
        table[ScoutingRegion::EastAsia.index()] = 0.62;
        RegionPrestigeTable::publish(table);
        assert_eq!(RegionPrestigeTable::get(ScoutingRegion::EastAsia), 0.62);
        RegionPrestigeTable::clear();
    }
}

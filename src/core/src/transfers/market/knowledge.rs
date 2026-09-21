//! What ONE club can see of a foreign market.
//!
//! A club's foreign signings cluster in two to five source countries, and the
//! cluster is stable for years because it is made of relationships — a scout
//! who lived there, an agent, an ex-player, a friendly club. Udinese and
//! Watford share the Pozzo network; Shakhtar has been buying Brazilians for
//! twenty years; Brighton found Ecuador and Paraguay and Japan. A new
//! corridor opens when a club hires the person who knows the market, and it
//! fades over years when that person leaves.
//!
//! Knowledge is by COUNTRY, never by region: knowing Colombia is not knowing
//! Peru, and a Spartak scout who "knows South America" is not a person who
//! exists. Three sources feed it, each on its own timescale:
//!
//!   * the club's own **ledger** — whom it has actually bought from, decaying
//!     on a four-year half-life, strengthened by signings that deliver;
//!   * its **scouts'** per-country levels, which grow with assignment days;
//!   * its country's **prior**, at half weight — a Turkish club knows Brazil
//!     is a market its league buys from, even with no scout there. That is
//!     enough to BUY a Brazilian who is offered; it is not enough to go and
//!     FIND one.
//!
//! On day one every club's knowledge is exactly this: its country's card,
//! its own squad's nationalities, and its scouts' seeded countries. That is
//! what makes the 2026 world look like 2026 without anything being routed.
//!
//! A loan runs the same relationship the other way, so the LENDING side
//! lives here too: [`LoanPlacementLedger`] is where a club has sent its
//! own, [`LoanPlacementKnowledge`] what that is worth against a
//! destination, and [`PlacementReach`] the shape a borrowing country can
//! read it in.

use chrono::NaiveDate;
use rustc_hash::FxHashMap;

use crate::Club;
use crate::transfers::market::affinity::MarketAffinity;
use crate::transfers::market::map::MarketMap;

/// One country a club has done business in.
#[derive(Debug, Clone)]
pub struct MarketLedgerEntry {
    pub country_id: u32,
    /// Signings made from this market, including the day-0 bootstrap from
    /// the shipped squad.
    pub signings: u16,
    /// The most recent one. Decay runs from here.
    pub last_signing: NaiveDate,
    /// How well this corridor has served the club, 0..1, seeded neutral at
    /// 0.5 and nudged by what the signings went on to do. A corridor that
    /// delivers starters strengthens; one that delivers reserves does not.
    pub outcomes: f32,
    /// True while every signing on this row came from the day-0 bootstrap
    /// (the shipped squad) rather than from business done in the save.
    ///
    /// Read only by the knowledge census, which counts corridors a save has
    /// OPENED for itself. Without it every club reads as having opened one
    /// on day zero, because the bootstrap stamps the world-start date on
    /// every row it writes.
    pub bootstrapped: bool,
}

/// A club's memory of the markets it works in.
#[derive(Debug, Clone, Default)]
pub struct ClubMarketLedger {
    entries: Vec<MarketLedgerEntry>,
}

impl ClubMarketLedger {
    /// Signings at which the ledger term saturates. Five men from one country
    /// is a corridor; the sixth does not make it more of one.
    const SATURATION: f32 = 5.0;
    /// Half-life of an unworked corridor, in days. Four years — the span
    /// over which a departed scout's network stops returning calls.
    const HALF_LIFE_DAYS: f32 = 4.0 * 365.0;
    /// Neutral starting judgement of a corridor.
    const NEUTRAL_OUTCOME: f32 = 0.5;

    pub fn entries(&self) -> &[MarketLedgerEntry] {
        &self.entries
    }

    /// Record a completed signing from `country_id`.
    pub fn record_signing(&mut self, country_id: u32, date: NaiveDate) {
        match self.entries.iter_mut().find(|e| e.country_id == country_id) {
            // NOT `bootstrapped = false`: the flag records where the ROW came
            // from, not when it was last touched. Clearing it on every
            // increment made "clubs that opened a NEW market" count every
            // club that did ordinary business in a market it already had —
            // 43% of the world in one season, against a 3-8% band.
            Some(entry) => {
                entry.signings = entry.signings.saturating_add(1);
                entry.last_signing = date;
            }
            None => self.entries.push(MarketLedgerEntry {
                country_id,
                signings: 1,
                last_signing: date,
                outcomes: Self::NEUTRAL_OUTCOME,
                bootstrapped: false,
            }),
        }
    }

    /// Bootstrap an entry from the shipped world — a squad place today is a
    /// signing the club made at some point before the save began, so it dates
    /// from the world's start rather than from nothing.
    pub fn bootstrap(&mut self, country_id: u32, signings: u16, date: NaiveDate) {
        if signings == 0 {
            return;
        }
        match self.entries.iter_mut().find(|e| e.country_id == country_id) {
            Some(entry) => entry.signings = entry.signings.saturating_add(signings),
            None => self.entries.push(MarketLedgerEntry {
                country_id,
                signings,
                last_signing: date,
                outcomes: Self::NEUTRAL_OUTCOME,
                bootstrapped: true,
            }),
        }
    }

    /// Nudge a corridor's standing by how a signing from it worked out.
    /// `share` is the player's first-season selection share, 0..1.
    pub fn record_outcome(&mut self, country_id: u32, share: f32) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.country_id == country_id) {
            entry.outcomes = (entry.outcomes * 0.8 + share.clamp(0.0, 1.0) * 0.2).clamp(0.0, 1.0);
        }
    }

    /// This ledger's contribution to knowing `country_id`, 0..1.
    ///
    /// Volume, decayed by time since the last piece of business and scaled by
    /// how the corridor has served. A club that bought five Brazilians last
    /// summer reads 1.0; the same club fifteen years later, having bought
    /// none since, reads near zero and has to re-open the market like anyone
    /// else.
    pub fn knowledge(&self, country_id: u32, today: NaiveDate) -> f32 {
        let Some(entry) = self.entries.iter().find(|e| e.country_id == country_id) else {
            return 0.0;
        };
        let volume = (entry.signings as f32 / Self::SATURATION).clamp(0.0, 1.0);
        let days = (today - entry.last_signing).num_days().max(0) as f32;
        let decay = 0.5_f32.powf(days / Self::HALF_LIFE_DAYS);
        // Outcomes move the corridor within a band rather than gating it:
        // even a corridor that produced nothing leaves the club knowing the
        // market, which is the thing being measured.
        let quality = 0.7 + 0.6 * entry.outcomes.clamp(0.0, 1.0);
        (volume * decay * quality).clamp(0.0, 1.0)
    }
}

/// Where a club has SENT its loanees, and how each destination served
/// them.
///
/// A loan is the other half of the same relationship [`ClubMarketLedger`]
/// records, read from the lending end: Chelsea knows the Eredivisie
/// because Chelsea has had men there, and a season that delivered minutes
/// is a reason to go back. Entries are per COUNTRY for the same reason
/// imports are — placing a boy in Belgium is not knowing Portugal — and
/// the volume, decay and outcome arithmetic is the same memory, so it is
/// the same type underneath.
#[derive(Debug, Clone, Default)]
pub struct LoanPlacementLedger {
    countries: ClubMarketLedger,
}

impl LoanPlacementLedger {
    /// Record a completed placement into `country_id`.
    pub fn record_placement(&mut self, country_id: u32, date: NaiveDate) {
        self.countries.record_signing(country_id, date);
    }

    /// Seed from the shipped world — a loanee sitting at a club abroad
    /// today is a placement this club made before the save began.
    pub fn bootstrap(&mut self, country_id: u32, placements: u16, date: NaiveDate) {
        self.countries.bootstrap(country_id, placements, date);
    }

    /// Nudge a destination's standing by how much football he got there.
    pub fn record_outcome(&mut self, country_id: u32, start_share: f32) {
        self.countries.record_outcome(country_id, start_share);
    }

    pub fn knowledge(&self, country_id: u32, today: NaiveDate) -> f32 {
        self.countries.knowledge(country_id, today)
    }

    pub fn entries(&self) -> &[MarketLedgerEntry] {
        self.countries.entries()
    }
}

/// How well a LENDING club knows a destination country, 0..1.
///
/// The mirror of [`ClubMarketKnowledge`], read from the other end of the
/// deal. Two things differ. The prior is the parent country's EXPORT card
/// — where this country's players go, not where its clubs buy — and a
/// pair no card names falls to the corridor rather than to nothing, so a
/// save can open a placement route the shipped world never had.
pub struct LoanPlacementKnowledge;

impl LoanPlacementKnowledge {
    /// A country's own export prior counts for this much of its clubs'
    /// placement knowledge — half, exactly as the import prior does.
    const COUNTRY_PRIOR_SHARE: f32 = 0.5;

    pub fn of(
        map: &MarketMap,
        parent_country_id: u32,
        ledger: &LoanPlacementLedger,
        best_scout_level: u8,
        borrower_country_id: u32,
        today: NaiveDate,
    ) -> f32 {
        if borrower_country_id == 0 || borrower_country_id == parent_country_id {
            return 1.0;
        }
        let scouted = best_scout_level as f32 / 100.0;
        let ledger_term = ledger.knowledge(borrower_country_id, today);
        let card = map
            .profile(parent_country_id)
            .export_weight(borrower_country_id)
            .unwrap_or_else(|| {
                MarketAffinity::blend_corridor(
                    &map.corridor(parent_country_id, borrower_country_id),
                )
            });
        scouted
            .max(ledger_term)
            .max(Self::COUNTRY_PRIOR_SHARE * card)
            .clamp(0.0, 1.0)
    }
}

/// One lending club's placement map, in the shape a BORROWING country can
/// read it.
///
/// A borrower's per-country borrow cannot reach the club that owns the
/// player, so the three things the placement read needs — the parent's
/// country, its ledger and its scouts' coverage — are staged per CLUB
/// beside the world pool. Per club and not per player: one club's ledger
/// is a dozen rows and a hundred summaries share it.
#[derive(Debug, Clone, Default)]
pub struct PlacementReach {
    country_id: u32,
    ledger: LoanPlacementLedger,
    /// Countries the club's scouts cover, and how well.
    scouts: Vec<(u32, u8)>,
}

impl PlacementReach {
    pub fn of(club: &Club, country_id: u32) -> Self {
        let mut scouts: Vec<(u32, u8)> = Vec::new();
        for staff in club
            .teams
            .teams
            .iter()
            .flat_map(|team| team.staffs.staffs.iter())
        {
            for known in &staff.staff_attributes.knowledge.known_countries {
                match scouts.iter_mut().find(|(id, _)| *id == known.country_id) {
                    Some(entry) => entry.1 = entry.1.max(known.level),
                    None => scouts.push((known.country_id, known.level)),
                }
            }
        }
        PlacementReach {
            country_id,
            ledger: club.loan_placements.clone(),
            scouts,
        }
    }

    /// True when this club has nothing the country-level prior does not
    /// already say. Staging it would be a pair of allocations carrying no
    /// information.
    pub fn is_silent(&self) -> bool {
        self.ledger.entries().is_empty() && self.scouts.is_empty()
    }

    pub fn trust(&self, map: &MarketMap, borrower_country_id: u32, today: NaiveDate) -> f32 {
        let scout_level = self
            .scouts
            .iter()
            .find(|(id, _)| *id == borrower_country_id)
            .map(|(_, level)| *level)
            .unwrap_or(0);
        LoanPlacementKnowledge::of(
            map,
            self.country_id,
            &self.ledger,
            scout_level,
            borrower_country_id,
            today,
        )
    }
}

/// Every lending club's placement map, keyed by club id.
///
/// Clubs with nothing to say are absent and read from their country's
/// export card alone, which is what the whole world said about them
/// anyway.
#[derive(Debug, Clone, Default)]
pub struct PlacementReachIndex {
    by_club: FxHashMap<u32, PlacementReach>,
}

impl PlacementReachIndex {
    pub fn from_clubs(clubs: impl IntoIterator<Item = (u32, PlacementReach)>) -> Self {
        PlacementReachIndex {
            by_club: clubs
                .into_iter()
                .filter(|(_, reach)| !reach.is_silent())
                .collect(),
        }
    }

    /// How far this parent's own network reaches into the borrower's
    /// country, 0..1.
    pub fn trust(
        &self,
        map: &MarketMap,
        parent_club_id: u32,
        parent_country_id: u32,
        borrower_country_id: u32,
        today: NaiveDate,
    ) -> f32 {
        match self.by_club.get(&parent_club_id) {
            Some(reach) => reach.trust(map, borrower_country_id, today),
            None => LoanPlacementKnowledge::of(
                map,
                parent_country_id,
                &LoanPlacementLedger::default(),
                0,
                borrower_country_id,
                today,
            ),
        }
    }
}

/// Writes to a club's ledger from the execution paths. A unit struct so the
/// two call sites read as one named operation rather than three
/// conditionals copied twice.
pub struct MarketLedgerUpdate;

impl MarketLedgerUpdate {
    /// Record a completed signing. Both halves of the move count and for
    /// different reasons: buying FROM a league puts a club in touch with its
    /// clubs and agents, and buying a NATIONALITY puts it in touch with the
    /// people who represent it. A domestic signing of a domestic player
    /// teaches nothing about any market and is skipped.
    pub fn on_signing(
        club: &mut Club,
        buyer_country_id: u32,
        source_country_id: u32,
        nationality_country_id: u32,
        date: NaiveDate,
    ) {
        if source_country_id != 0 && source_country_id != buyer_country_id {
            club.market_ledger.record_signing(source_country_id, date);
        }
        if nationality_country_id != 0
            && nationality_country_id != buyer_country_id
            && nationality_country_id != source_country_id
        {
            club.market_ledger
                .record_signing(nationality_country_id, date);
        }
    }
}

/// The three sources folded into one number.
pub struct ClubMarketKnowledge;

impl ClubMarketKnowledge {
    /// A country's own import prior counts for this much of its clubs'
    /// knowledge. Half — enough to answer the phone about a Brazilian,
    /// not enough to find one.
    const COUNTRY_PRIOR_SHARE: f32 = 0.5;
    /// Scout level at which a market counts as covered. Below it the club
    /// wants a specialist (see the scout market).
    pub const COVERED_LEVEL: u8 = 30;
    /// Knowledge at which a market counts as one the club can work in — the
    /// bar the knowledge census counts against.
    pub const WORKING_KNOWLEDGE: f32 = 0.3;

    /// How well `club_country` + this club know `source_country`, 0..1.
    ///
    /// The maximum of the three channels rather than a sum: they are three
    /// ways of knowing the same thing, and a club with a Brazil specialist
    /// does not know Brazil better for also having a Brazilian on the books.
    pub fn knowledge(
        map: &MarketMap,
        club_country_id: u32,
        ledger: &ClubMarketLedger,
        best_scout_level: u8,
        source_country_id: u32,
        today: NaiveDate,
    ) -> f32 {
        if source_country_id == club_country_id {
            return 1.0;
        }
        let scouted = best_scout_level as f32 / 100.0;
        let ledger_term = ledger.knowledge(source_country_id, today);
        let prior = Self::COUNTRY_PRIOR_SHARE
            * map
                .profile(club_country_id)
                .import_weight(source_country_id)
                .unwrap_or(0.0);
        scouted.max(ledger_term).max(prior).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn a_worked_corridor_reads_high_and_fades_over_years() {
        let mut ledger = ClubMarketLedger::default();
        for _ in 0..5 {
            ledger.record_signing(1, day(2026, 7, 1));
        }
        let fresh = ledger.knowledge(1, day(2026, 8, 1));
        assert!(fresh > 0.9, "five signings last month: {fresh}");
        let stale = ledger.knowledge(1, day(2038, 8, 1));
        assert!(stale < 0.15, "twelve years untouched: {stale}");
        assert!(stale > 0.0, "a corridor is never wholly forgotten");
    }

    #[test]
    fn an_unworked_country_is_unknown_to_the_ledger() {
        let ledger = ClubMarketLedger::default();
        assert_eq!(ledger.knowledge(7, day(2026, 8, 1)), 0.0);
    }

    #[test]
    fn outcomes_move_a_corridor_within_a_band_and_never_close_it() {
        let mut good = ClubMarketLedger::default();
        let mut bad = ClubMarketLedger::default();
        for _ in 0..5 {
            good.record_signing(1, day(2026, 7, 1));
            bad.record_signing(1, day(2026, 7, 1));
        }
        for _ in 0..8 {
            good.record_outcome(1, 1.0);
            bad.record_outcome(1, 0.0);
        }
        let today = day(2026, 8, 1);
        assert!(good.knowledge(1, today) > bad.knowledge(1, today));
        assert!(
            bad.knowledge(1, today) > 0.5,
            "a disappointing corridor is still a known market"
        );
    }

    #[test]
    fn a_country_prior_alone_buys_but_does_not_find() {
        use std::collections::HashMap;

        use crate::transfers::ScoutingRegion;
        use crate::transfers::market::map::{
            CorridorWeight, CountryTransferProfile, MarketCountryFacts,
        };

        const TR: u32 = 1;
        const BR: u32 = 2;
        const PE: u32 = 3;
        let mut facts = HashMap::new();
        for (id, code, continent) in [(TR, "tr", 1u32), (BR, "br", 3), (PE, "pe", 3)] {
            facts.insert(
                id,
                MarketCountryFacts {
                    id,
                    code: code.to_string(),
                    continent_id: continent,
                    region: ScoutingRegion::from_country(continent, code),
                    reputation: 6000,
                    top_flight_reputation: 6000,
                    median_top_flight_wage: 100_000,
                },
            );
        }
        let mut profiles = HashMap::new();
        profiles.insert(
            TR,
            CountryTransferProfile {
                import: vec![CorridorWeight {
                    country_id: BR,
                    weight: 1.0,
                    money: false,
                }],
                ..Default::default()
            },
        );
        let map = MarketMap::new(profiles, facts);
        let ledger = ClubMarketLedger::default();
        let today = day(2026, 8, 1);

        let brazil = ClubMarketKnowledge::knowledge(&map, TR, &ledger, 0, BR, today);
        let peru = ClubMarketKnowledge::knowledge(&map, TR, &ledger, 0, PE, today);
        assert!(
            (brazil - 0.5).abs() < 0.001,
            "the country prior is worth half: {brazil}"
        );
        assert_eq!(peru, 0.0, "a market the country does not work is unknown");
        assert_eq!(
            ClubMarketKnowledge::knowledge(&map, TR, &ledger, 0, TR, today),
            1.0,
            "a club knows its own country"
        );
    }
}

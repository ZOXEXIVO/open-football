use crate::Person;
use crate::club::news::{
    ClubTargetsWeek, ClubTransferWeek, PursuitStage, TargetPursuit, WindowWeek,
};
use crate::transfers::{
    NegotiationPhase, NegotiationRejectionReason, NegotiationStatus, TransferCalendar,
};
use crate::world::SimulatorData;
use chrono::{Datelike, Duration, NaiveDate};
use rustc_hash::FxHashMap;

/// The week's completed transfer business, bucketed by club. Both sides
/// of a deal are recorded, so a club hears about its own departures even
/// though the player has already left the roster.
#[derive(Default)]
pub(super) struct WeeklyMarket {
    by_club: FxHashMap<u32, ClubTransferWeek>,
    /// Every club's live pursuits, read from the buyer's side of the
    /// negotiation table — bids in, fees agreed, medicals booked and
    /// the deals that fell over. Keyed by buying club.
    targets: FxHashMap<u32, ClubTargetsWeek>,
    /// Countries whose registration window opened or shut this week.
    windows: FxHashMap<u32, WindowWeek>,
    /// Arrivals and departures per club since the shut window opened —
    /// only tallied on the weeks a window actually shut.
    window_tally: FxHashMap<u32, (u8, u8)>,
}

impl WeeklyMarket {
    /// A rejection is news for this long after the phase it died in
    /// began. The negotiation map keeps settled rows for a month, so
    /// without a freshness gate a first edition would report every
    /// dead deal of the last thirty days as this week's.
    const REJECTION_IS_NEWS_FOR_DAYS: i64 = 14;

    /// How far past the week the backwards scan of the transfer log
    /// keeps reading before it gives up. Bounds the work while leaving
    /// room for rows filed out of date order — see [`Self::gather`].
    const SCAN_MARGIN_DAYS: i64 = 31;

    pub(super) fn from_world(
        data: &SimulatorData,
        week_start: NaiveDate,
        week_end: NaiveDate,
    ) -> Self {
        let mut market = WeeklyMarket {
            by_club: Self::gather(data, week_start, week_end),
            targets: Self::gather_targets(data, week_start, week_end),
            windows: Self::gather_windows(data, week_start, week_end),
            window_tally: FxHashMap::default(),
        };
        market.tally_windows(data, week_end);
        market.enrich_arrivals(data, week_end);
        market
    }

    /// The club's own business, before it is done.
    ///
    /// Every negotiation in the world is a row in some country's market
    /// — cross-border deals live under the buying country — so the walk
    /// covers every market and buckets by buyer. Only the stage a paper
    /// would print is kept: a bid is news the week it went in, a fee or
    /// a medical is news while it stands, and an ending is news for a
    /// fortnight after the phase it happened in began.
    fn gather_targets(
        data: &SimulatorData,
        week_start: NaiveDate,
        week_end: NaiveDate,
    ) -> FxHashMap<u32, ClubTargetsWeek> {
        let mut by_buyer: FxHashMap<u32, ClubTargetsWeek> = FxHashMap::default();
        let ending_floor = week_start - Duration::days(Self::REJECTION_IS_NEWS_FOR_DAYS);

        for continent in &data.continents {
            for country in &continent.countries {
                for negotiation in country.transfer_market.negotiations.values() {
                    let stage = match negotiation.status {
                        NegotiationStatus::Pending | NegotiationStatus::Countered => {
                            match negotiation.phase {
                                NegotiationPhase::InitialApproach { .. }
                                | NegotiationPhase::ClubNegotiation { .. } => {
                                    let lodged = negotiation.created_date;
                                    if lodged < week_start || lodged >= week_end {
                                        continue;
                                    }
                                    PursuitStage::BidLodged
                                }
                                NegotiationPhase::PersonalTerms { .. } => PursuitStage::FeeAgreed,
                                NegotiationPhase::MedicalAndFinalization { .. } => {
                                    PursuitStage::MedicalBooked
                                }
                            }
                        }
                        NegotiationStatus::Rejected => {
                            if Self::phase_started(&negotiation.phase) < ending_floor {
                                continue;
                            }
                            match negotiation.rejection_reason {
                                Some(NegotiationRejectionReason::AskingPriceTooHigh) => {
                                    PursuitStage::PricedOut
                                }
                                Some(
                                    NegotiationRejectionReason::SellerRefusedToNegotiate
                                    | NegotiationRejectionReason::PlayerTooImportant
                                    | NegotiationRejectionReason::ReputationGapTooLarge,
                                ) => PursuitStage::NotForSale,
                                Some(
                                    NegotiationRejectionReason::PlayerRejectedPersonalTerms
                                    | NegotiationRejectionReason::SalaryDemandsUnmet,
                                ) => PursuitStage::PlayerSaidNo,
                                Some(NegotiationRejectionReason::MedicalFailed) => {
                                    PursuitStage::MedicalFailed
                                }
                                Some(NegotiationRejectionReason::WindowClosed) => {
                                    PursuitStage::WindowShut
                                }
                                // A route the simulation itself refuses
                                // is not a deal that fell over, and a
                                // rejection with no reason recorded is
                                // not one the paper can explain.
                                Some(NegotiationRejectionReason::CountryPairRouteBlocked)
                                | None => continue,
                            }
                        }
                        // Talks that timed out without anybody saying
                        // no. Nothing happened, and the paper says so
                        // by saying nothing.
                        NegotiationStatus::Accepted | NegotiationStatus::Expired => continue,
                    };

                    by_buyer
                        .entry(negotiation.buying_club_id)
                        .or_default()
                        .pursuits
                        .push(TargetPursuit {
                            player_id: negotiation.player_id,
                            selling_club_id: negotiation.selling_club_id,
                            fee: if negotiation.is_loan {
                                0
                            } else {
                                negotiation.current_offer.base_fee.amount.max(0.0) as i64
                            },
                            is_loan: negotiation.is_loan,
                            stage,
                        });
                }
            }
        }

        // The negotiation map is a hash map; the page is not allowed to
        // depend on its iteration order.
        for week in by_buyer.values_mut() {
            week.pursuits
                .sort_by_key(|pursuit| (pursuit.player_id, pursuit.selling_club_id));
        }

        by_buyer
    }

    /// When the phase a negotiation is in began — the nearest thing a
    /// resolved row has to a date of resolution.
    fn phase_started(phase: &NegotiationPhase) -> NaiveDate {
        match phase {
            NegotiationPhase::InitialApproach { started }
            | NegotiationPhase::ClubNegotiation { started, .. }
            | NegotiationPhase::PersonalTerms { started, .. }
            | NegotiationPhase::MedicalAndFinalization { started } => *started,
        }
    }

    /// Which countries saw a registration window open or shut inside
    /// the week. Read off the same calendar the market itself runs on.
    fn gather_windows(
        data: &SimulatorData,
        week_start: NaiveDate,
        week_end: NaiveDate,
    ) -> FxHashMap<u32, WindowWeek> {
        let mut windows: FxHashMap<u32, WindowWeek> = FxHashMap::default();
        let inside = |day: NaiveDate| day >= week_start && day < week_end;

        for continent in &data.continents {
            for country in &continent.countries {
                let calendar = TransferCalendar::for_country(&country.code, week_end);
                for (opens, shuts) in [calendar.summer_window, calendar.winter_window] {
                    if opens > shuts {
                        continue;
                    }
                    let opened = inside(opens);
                    let closed = inside(shuts);
                    if opened || closed {
                        let entry = windows.entry(country.id).or_default();
                        entry.opened |= opened;
                        entry.closed |= closed;
                    }
                }
            }
        }

        windows
    }

    /// Arrivals and departures per club over the window that has just
    /// shut. Walks every market, because a cross-border departure is
    /// filed under the buyer's country; bounded by the earliest window
    /// start among the countries that shut one this week, and skipped
    /// entirely on the fifty weeks a year when none did.
    fn tally_windows(&mut self, data: &SimulatorData, week_end: NaiveDate) {
        let mut floor: Option<NaiveDate> = None;
        for continent in &data.continents {
            for country in &continent.countries {
                if !self
                    .windows
                    .get(&country.id)
                    .is_some_and(|week| week.closed)
                {
                    continue;
                }
                let calendar = TransferCalendar::for_country(&country.code, week_end);
                for (opens, shuts) in [calendar.summer_window, calendar.winter_window] {
                    if shuts < week_end && opens <= shuts {
                        floor = Some(floor.map_or(opens, |day: NaiveDate| day.min(opens)));
                    }
                }
            }
        }
        let Some(floor) = floor else {
            return;
        };

        for continent in &data.continents {
            for country in &continent.countries {
                for transfer in country
                    .transfer_market
                    .transfer_history
                    .iter()
                    .rev()
                    .take_while(|transfer| transfer.transfer_date >= floor)
                {
                    if transfer.transfer_date >= week_end {
                        continue;
                    }
                    if transfer.to_club_id != 0 {
                        let entry = self.window_tally.entry(transfer.to_club_id).or_default();
                        entry.0 = entry.0.saturating_add(1);
                    }
                    if transfer.from_club_id != 0 {
                        let entry = self.window_tally.entry(transfer.from_club_id).or_default();
                        entry.1 = entry.1.saturating_add(1);
                    }
                }
            }
        }
    }

    pub(super) fn for_targets(&self, club_id: u32) -> Option<&ClubTargetsWeek> {
        self.targets.get(&club_id)
    }

    /// What the calendar did this week for one club: whether its
    /// country's window moved, and — if it shut — the club's own count.
    pub(super) fn window_for(&self, country_id: u32, club_id: u32) -> Option<WindowWeek> {
        let week = *self.windows.get(&country_id)?;
        let (arrivals, departures) = self.window_tally.get(&club_id).copied().unwrap_or_default();
        Some(WindowWeek {
            arrivals: if week.closed { arrivals } else { 0 },
            departures: if week.closed { departures } else { 0 },
            ..week
        })
    }

    fn gather(
        data: &SimulatorData,
        week_start: NaiveDate,
        week_end: NaiveDate,
    ) -> FxHashMap<u32, ClubTransferWeek> {
        let mut by_club: FxHashMap<u32, ClubTransferWeek> = FxHashMap::default();

        // The history is append-ordered and never trimmed, so the walk
        // starts at the newest entry and stops once it is behind the
        // window — a decade of completed deals is not rescanned every
        // Monday. The stop is held a month back rather than on the
        // window's own edge: the log is appended from several points in
        // the tick (academy graduations, contract expiries, the market
        // pipeline, and cross-border deals filed under the *buying*
        // country), and a single row landing a day out of order would
        // otherwise cut the walk short and take that country's entire
        // week of transfer business off every front page in it, silently.
        let scan_floor = week_start - Duration::days(Self::SCAN_MARGIN_DAYS);

        for continent in &data.continents {
            for country in &continent.countries {
                for transfer in country
                    .transfer_market
                    .transfer_history
                    .iter()
                    .rev()
                    .take_while(|transfer| transfer.transfer_date >= scan_floor)
                {
                    if transfer.transfer_date < week_start || transfer.transfer_date >= week_end {
                        continue;
                    }
                    if transfer.to_club_id != 0 {
                        by_club
                            .entry(transfer.to_club_id)
                            .or_default()
                            .absorb(transfer.to_club_id, transfer);
                    }
                    if transfer.from_club_id != 0 {
                        by_club
                            .entry(transfer.from_club_id)
                            .or_default()
                            .absorb(transfer.from_club_id, transfer);
                    }
                }
            }
        }

        by_club
    }

    /// Fills in the facts about each arrival that decide which story he
    /// is: his age, and whether he has worn the buying club's colours
    /// before. The desk cannot look this up itself — a `TransferMove`
    /// deliberately carries identifiers only — and without it every
    /// returning favourite and every teenage signing prints as the same
    /// anonymous "new signing" line.
    fn enrich_arrivals(&mut self, data: &SimulatorData, today: NaiveDate) {
        for (club_id, week) in self.by_club.iter_mut() {
            if week.arrivals.is_empty() {
                continue;
            }
            let Some(club) = data.club(*club_id) else {
                continue;
            };
            let slugs: Vec<&str> = club.teams.iter().map(|team| team.slug.as_str()).collect();

            for arrival in week.arrivals.iter_mut() {
                let Some(player) = data.player(arrival.player_id) else {
                    continue;
                };
                arrival.age = player.age(today);

                let ledger = &player.statistics_history.season_ledger;
                arrival.returning = ledger
                    .iter()
                    .any(|entry| slugs.contains(&entry.team_slug.as_str()));
                // "Was here on loan" means the loan is the reason the
                // reader already knows him: his newest spell at this
                // club was a loan, and a recent one — a borrowed season
                // from years ago is a homecoming, not a buy-out.
                arrival.was_loan_here = ledger
                    .iter()
                    .rev()
                    .find(|entry| slugs.contains(&entry.team_slug.as_str()))
                    .is_some_and(|entry| {
                        entry.is_loan && i32::from(entry.season_start_year) >= today.year() - 1
                    });
            }
        }
    }

    pub(super) fn for_club(&self, club_id: u32) -> Option<&ClubTransferWeek> {
        self.by_club.get(&club_id)
    }
}

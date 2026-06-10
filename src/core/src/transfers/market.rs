use crate::shared::CurrencyValue;
use crate::transfers::negotiation::{NegotiationStatus, TransferNegotiation};
use crate::transfers::offer::TransferOffer;
use crate::transfers::{CompletedTransfer, TransferType};
use chrono::Duration;
use chrono::{Datelike, NaiveDate};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct TransferMarket {
    pub listings: Vec<TransferListing>,
    pub negotiations: HashMap<u32, TransferNegotiation>,
    pub transfer_window_open: bool,
    pub transfer_history: Vec<CompletedTransfer>,
    pub next_negotiation_id: u32,
    /// Pending obligations from clauses on completed transfers —
    /// installment payments, performance add-ons (appearances, goals,
    /// promotion), and triggerable bonuses that pay out *after* the
    /// player has moved. Resolved daily via [`Self::settle_due_clauses`]
    /// at country tick time. Empty for clubs that haven't bought
    /// anyone with installment-style deals.
    pub pending_clauses: Vec<PendingTransferClause>,
}

/// A future financial obligation arising from a clause that fires
/// *after* a player's transfer is complete: a tranche payment, a
/// performance bonus, a promotion top-up. Stored on the buying
/// country's `TransferMarket` (where the buyer lives) so the daily
/// settlement walk only touches the country whose club owes money.
///
/// The trigger model is intentionally simple — each clause carries the
/// information needed to resolve it without re-reading the original
/// `TransferClause` enum. Resolving an installment is a date check;
/// resolving an appearance / goal / promotion bonus is a counter check
/// the caller threads in.
#[derive(Debug, Clone)]
pub struct PendingTransferClause {
    /// Unique id within this market — used for cancellation / debug.
    pub id: u32,
    /// Buying club that owes the future payment.
    pub buying_club_id: u32,
    /// Selling club that receives the payment. Same club id is used
    /// even for installments owed back to the seller: the routing logic
    /// lives in `settle_due_clauses` which credits whichever side the
    /// clause names.
    pub selling_club_id: u32,
    /// Player the obligation tracks. Used by performance triggers to
    /// look up appearances/goals.
    pub player_id: u32,
    /// What the obligation pays for.
    pub trigger: ClauseTrigger,
    /// Per-fire payment amount. Total cost over time = `amount × max_fires`.
    pub amount: f64,
    /// Maximum number of times this clause can pay out before being
    /// retired. Installments cap at the agreed tranche count; one-off
    /// bonuses cap at 1.
    pub max_fires: u8,
    /// How many times the clause has fired so far.
    pub fires_so_far: u8,
    /// Last calendar date on which the clause can still fire. When `None`
    /// the clause has no end date — the `max_fires` cap retires it.
    pub expires_on: Option<NaiveDate>,
}

#[derive(Debug, Clone)]
pub enum ClauseTrigger {
    /// Fire on a specific calendar date. The settler compares
    /// `today >= scheduled_date` and `fires_so_far < max_fires`.
    Installment { scheduled_date: NaiveDate },
    /// Fire when the player crosses `target_appearances` total at the
    /// buying club. The settler reads the player's appearance counter
    /// at the buying club and compares.
    AppearanceMilestone { target_appearances: u32 },
    /// Fire when the player crosses `target_goals` total at the buying
    /// club.
    GoalMilestone { target_goals: u32 },
    /// Fire when the buying club gets promoted out of its current
    /// league tier. The settler reads the buying club's league position
    /// at end-of-season.
    Promotion,
}

#[derive(Debug, Clone)]
pub struct TransferListing {
    pub player_id: u32,
    pub club_id: u32,
    pub team_id: u32,
    pub asking_price: CurrencyValue,
    /// Kept separate from `asking_price` so decay steps have a reference
    /// point and can't drift below a sensible floor.
    pub original_asking_price: CurrencyValue,
    pub listed_date: NaiveDate,
    /// Last date a decay step was applied. Equal to `listed_date` when
    /// freshly created. Used to gate "one decay per week" cadence.
    pub last_decay_date: NaiveDate,
    pub listing_type: TransferListingType,
    pub status: TransferListingStatus,
    /// Why this listing exists. Synthetic listings created to back an
    /// unsolicited approach must NOT confer the "player is listed"
    /// acceptance bonus when the negotiation resolver evaluates the
    /// seller's willingness to deal — that bonus is only earned when
    /// the parent club actually advertised the player.
    pub origin: TransferListingOrigin,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransferListingType {
    Transfer,
    Loan,
    EndOfContract,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferListingOrigin {
    /// Selling club listed the player for permanent transfer.
    SellerListed,
    /// Selling club listed the player for loan-out.
    LoanOutListed,
    /// Contract ran out — listing exists so the player surfaces in the
    /// free-agent market.
    EndOfContract,
    /// Created on-the-fly by the pipeline so a buyer's unsolicited
    /// approach has something to negotiate against. Does not represent
    /// a genuine willingness to sell — must not earn the listed bonus.
    SyntheticUnsolicited,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransferListingStatus {
    Available,
    InNegotiation,
    Completed,
    Cancelled,
}

impl TransferListing {
    pub fn new(
        player_id: u32,
        club_id: u32,
        team_id: u32,
        asking_price: CurrencyValue,
        listed_date: NaiveDate,
        listing_type: TransferListingType,
    ) -> Self {
        let origin = match listing_type {
            TransferListingType::Transfer => TransferListingOrigin::SellerListed,
            TransferListingType::Loan => TransferListingOrigin::LoanOutListed,
            TransferListingType::EndOfContract => TransferListingOrigin::EndOfContract,
        };
        Self::new_with_origin(
            player_id,
            club_id,
            team_id,
            asking_price,
            listed_date,
            listing_type,
            origin,
        )
    }

    /// Build a listing with an explicit origin tag. Used by the
    /// pipeline to mark synthetic listings backing unsolicited bids so
    /// the negotiation resolver doesn't grant them the listed bonus.
    pub fn new_with_origin(
        player_id: u32,
        club_id: u32,
        team_id: u32,
        asking_price: CurrencyValue,
        listed_date: NaiveDate,
        listing_type: TransferListingType,
        origin: TransferListingOrigin,
    ) -> Self {
        TransferListing {
            player_id,
            club_id,
            team_id,
            asking_price: asking_price.clone(),
            original_asking_price: asking_price,
            listed_date,
            last_decay_date: listed_date,
            listing_type,
            status: TransferListingStatus::Available,
            origin,
        }
    }

    /// True when this listing reflects a genuine seller-side decision
    /// to part with the player (Lst/Loa or EndOfContract). Synthetic
    /// listings backing unsolicited approaches return false.
    pub fn is_seller_advertised(&self) -> bool {
        !matches!(self.origin, TransferListingOrigin::SyntheticUnsolicited)
    }
}

impl TransferMarket {
    pub fn new() -> Self {
        TransferMarket {
            listings: Vec::new(),
            negotiations: HashMap::new(),
            transfer_window_open: false,
            transfer_history: Vec::new(),
            next_negotiation_id: 1,
            pending_clauses: Vec::new(),
        }
    }

    /// Schedule installment tranches for a permanent transfer fee.
    /// `years` tranches are scheduled one calendar year apart, starting
    /// `1 year` after `start_date`. The total over the schedule equals
    /// `amount` (the *deferred* portion of the headline fee — callers
    /// should subtract the upfront portion before calling).
    pub fn schedule_installments(
        &mut self,
        buying_club_id: u32,
        selling_club_id: u32,
        player_id: u32,
        amount: f64,
        years: u8,
        start_date: NaiveDate,
    ) {
        if years == 0 || amount <= 0.0 {
            return;
        }
        let per_tranche = amount / years as f64;
        let id = self.next_pending_clause_id();
        let mut next_id = id;
        for i in 1..=years as i32 {
            let target_year = start_date.year() + i;
            let scheduled_date = start_date
                .with_year(target_year)
                .unwrap_or_else(|| {
                    start_date
                        .checked_add_signed(Duration::days(365 * i as i64))
                        .unwrap_or(start_date)
                });
            self.pending_clauses.push(PendingTransferClause {
                id: next_id,
                buying_club_id,
                selling_club_id,
                player_id,
                trigger: ClauseTrigger::Installment { scheduled_date },
                amount: per_tranche,
                max_fires: 1,
                fires_so_far: 0,
                expires_on: None,
            });
            next_id += 1;
        }
    }

    /// Schedule a performance-bonus add-on (appearance/goal milestone)
    /// that fires when the player crosses `threshold` of the named
    /// counter at the buying club. `expires_on` lets the buyer cap the
    /// obligation by a date (e.g. end-of-contract); pass `None` for
    /// open-ended obligations.
    pub fn schedule_performance_addon(
        &mut self,
        buying_club_id: u32,
        selling_club_id: u32,
        player_id: u32,
        trigger: ClauseTrigger,
        amount: f64,
        expires_on: Option<NaiveDate>,
    ) {
        if amount <= 0.0 {
            return;
        }
        // Performance add-ons fire at most once each — they reward a
        // single milestone crossing.
        let id = self.next_pending_clause_id();
        self.pending_clauses.push(PendingTransferClause {
            id,
            buying_club_id,
            selling_club_id,
            player_id,
            trigger,
            amount,
            max_fires: 1,
            fires_so_far: 0,
            expires_on,
        });
    }

    /// Drain pending clauses whose calendar trigger has now passed.
    /// Returns the list of due installments so the caller can route
    /// the money through the buying/selling club finances (the market
    /// doesn't hold club references). Drops fully-fired and expired
    /// clauses from the queue as a side effect.
    ///
    /// Performance triggers (appearances/goals/promotion) are NOT
    /// settled here — those need counters that live outside the
    /// market and are handled by [`Self::resolve_performance_clauses`].
    pub fn drain_due_installments(&mut self, today: NaiveDate) -> Vec<PendingTransferClause> {
        let mut due: Vec<PendingTransferClause> = Vec::new();
        let mut keep: Vec<PendingTransferClause> = Vec::with_capacity(self.pending_clauses.len());
        for mut clause in self.pending_clauses.drain(..) {
            // Drop expired entries silently — the player may have left
            // the buying club before the milestone could be reached.
            if let Some(deadline) = clause.expires_on {
                if today > deadline {
                    continue;
                }
            }
            if let ClauseTrigger::Installment { scheduled_date } = clause.trigger {
                if clause.fires_so_far < clause.max_fires && today >= scheduled_date {
                    clause.fires_so_far = clause.fires_so_far.saturating_add(1);
                    due.push(clause);
                    continue;
                }
            }
            keep.push(clause);
        }
        self.pending_clauses = keep;
        due
    }

    /// Settle performance-triggered add-ons that have crossed their
    /// threshold given the latest counters. Caller supplies a closure
    /// that resolves the player's appearances/goals at the buying club
    /// (the market doesn't carry that state). Returns the list of
    /// triggered clauses so the caller can route the payouts.
    pub fn resolve_performance_clauses<F, G>(
        &mut self,
        today: NaiveDate,
        mut appearance_count: F,
        mut goal_count: G,
    ) -> Vec<PendingTransferClause>
    where
        F: FnMut(u32, u32) -> Option<u32>, // (player_id, buying_club_id) -> apps
        G: FnMut(u32, u32) -> Option<u32>, // (player_id, buying_club_id) -> goals
    {
        let mut fired: Vec<PendingTransferClause> = Vec::new();
        let mut keep: Vec<PendingTransferClause> = Vec::with_capacity(self.pending_clauses.len());
        for mut clause in self.pending_clauses.drain(..) {
            if let Some(deadline) = clause.expires_on {
                if today > deadline {
                    continue;
                }
            }
            let crosses_threshold = match clause.trigger {
                ClauseTrigger::AppearanceMilestone { target_appearances } => {
                    appearance_count(clause.player_id, clause.buying_club_id)
                        .map(|apps| apps >= target_appearances)
                        .unwrap_or(false)
                }
                ClauseTrigger::GoalMilestone { target_goals } => {
                    goal_count(clause.player_id, clause.buying_club_id)
                        .map(|goals| goals >= target_goals)
                        .unwrap_or(false)
                }
                _ => false,
            };
            if crosses_threshold && clause.fires_so_far < clause.max_fires {
                clause.fires_so_far = clause.fires_so_far.saturating_add(1);
                fired.push(clause);
                continue;
            }
            keep.push(clause);
        }
        self.pending_clauses = keep;
        fired
    }

    /// Trigger promotion add-ons when the buying club has been promoted
    /// — the caller passes in the set of club ids that won promotion at
    /// season end. Cheap O(N) walk over pending clauses; clubs not in
    /// the promoted set leave their clauses untouched.
    pub fn resolve_promotion_clauses(
        &mut self,
        today: NaiveDate,
        promoted_clubs: &[u32],
    ) -> Vec<PendingTransferClause> {
        let mut fired: Vec<PendingTransferClause> = Vec::new();
        let mut keep: Vec<PendingTransferClause> = Vec::with_capacity(self.pending_clauses.len());
        for mut clause in self.pending_clauses.drain(..) {
            if let Some(deadline) = clause.expires_on {
                if today > deadline {
                    continue;
                }
            }
            if matches!(clause.trigger, ClauseTrigger::Promotion)
                && clause.fires_so_far < clause.max_fires
                && promoted_clubs.contains(&clause.buying_club_id)
            {
                clause.fires_so_far = clause.fires_so_far.saturating_add(1);
                fired.push(clause);
                continue;
            }
            keep.push(clause);
        }
        self.pending_clauses = keep;
        fired
    }

    fn next_pending_clause_id(&self) -> u32 {
        self.pending_clauses
            .iter()
            .map(|c| c.id)
            .max()
            .map(|m| m + 1)
            .unwrap_or(1)
    }

    pub fn add_listing(&mut self, listing: TransferListing) {
        // Check for duplicates
        if !self.listings.iter().any(|l| {
            l.player_id == listing.player_id
                && l.club_id == listing.club_id
                && l.listing_type == listing.listing_type
                && l.status != TransferListingStatus::Completed
                && l.status != TransferListingStatus::Cancelled
        }) {
            self.listings.push(listing);
        }
    }

    pub fn get_available_listings(&self) -> Vec<&TransferListing> {
        self.listings
            .iter()
            .filter(|l| l.status == TransferListingStatus::Available)
            .collect()
    }

    pub fn get_listing_by_player(&self, player_id: u32) -> Option<&TransferListing> {
        self.listings.iter().find(|l| {
            l.player_id == player_id
                && (l.status == TransferListingStatus::Available
                    || l.status == TransferListingStatus::InNegotiation)
        })
    }

    /// Mark all listings for a player as completed (after transfer/loan executes).
    pub fn complete_listings_for_player(&mut self, player_id: u32) {
        for listing in &mut self.listings {
            if listing.player_id == player_id {
                listing.status = TransferListingStatus::Completed;
            }
        }
    }

    pub fn start_negotiation(
        &mut self,
        player_id: u32,
        buying_club_id: u32,
        offer: TransferOffer,
        current_date: NaiveDate,
        selling_reputation: f32,
        buying_reputation: f32,
        player_age: u8,
        player_ambition: f32,
    ) -> Option<u32> {
        if self.has_active_negotiation_for(player_id, buying_club_id) {
            return None;
        }

        // Find the listing. InNegotiation remains eligible so several clubs
        // can bid for the same player while the seller compares offers.
        if let Some(listing_index) = self.listings.iter().position(|l| {
            l.player_id == player_id
                && (l.status == TransferListingStatus::Available
                    || l.status == TransferListingStatus::InNegotiation)
        }) {
            let listing = &mut self.listings[listing_index];

            // Never negotiate with yourself
            if listing.club_id == buying_club_id {
                return None;
            }

            // Create a negotiation
            let negotiation_id = self.next_negotiation_id;
            self.next_negotiation_id += 1;

            let negotiation = TransferNegotiation::new(
                negotiation_id,
                player_id,
                listing_index as u32,
                listing.club_id,
                buying_club_id,
                offer,
                current_date,
                selling_reputation,
                buying_reputation,
                player_age,
                player_ambition,
            );

            listing.status = TransferListingStatus::InNegotiation;

            // Store the negotiation
            self.negotiations.insert(negotiation_id, negotiation);

            Some(negotiation_id)
        } else {
            None
        }
    }

    pub fn complete_transfer(
        &mut self,
        negotiation_id: u32,
        current_date: NaiveDate,
        player_name: String,
        from_team_name: String,
        to_team_name: String,
    ) -> Option<CompletedTransfer> {
        if let Some(negotiation) = self.negotiations.get(&negotiation_id) {
            if negotiation.status != NegotiationStatus::Accepted {
                return None;
            }

            let listing_idx = negotiation.listing_id as usize;
            if listing_idx >= self.listings.len() {
                return None;
            }

            if let Some(listing) = self.listings.get_mut(listing_idx) {
                listing.status = TransferListingStatus::Completed;
            }

            let listing = self.listings.get(listing_idx).unwrap();
            let from_team_id = listing.team_id;
            let player_id = negotiation.player_id;

            // Use negotiation's is_loan flag (not listing type) since a buying club
            // may approach a Transfer-listed player as a loan or vice versa.
            //
            // For loans, use the explicit `loan_duration_months` field —
            // the legacy code accidentally reused `contract_length` (which
            // documents itself as YEARS for permanent deals) and treated
            // it as months, producing a 30-day loan end for "1 year".
            // Falling back to ~6 months keeps the market-history record
            // sensible when no explicit duration is staged; the parent
            // contract's `loan_to_club_id` end date is computed separately
            // off the parent league's season calendar in `execution.rs`.
            let transfer_type = if negotiation.is_loan {
                let loan_end = negotiation
                    .current_offer
                    .loan_duration_months
                    .map(|months| {
                        current_date
                            .checked_add_signed(Duration::days(months as i64 * 30))
                            .unwrap_or(current_date)
                    })
                    .unwrap_or_else(|| {
                        current_date
                            .checked_add_signed(Duration::days(180))
                            .unwrap_or(current_date)
                    });
                TransferType::Loan(loan_end)
            } else if listing.listing_type == TransferListingType::EndOfContract {
                TransferType::Free
            } else {
                TransferType::Permanent
            };

            let reason = negotiation.reason.clone();

            let completed = CompletedTransfer::new(
                negotiation.player_id,
                player_name,
                negotiation.selling_club_id,
                from_team_id,
                from_team_name,
                negotiation.buying_club_id,
                to_team_name,
                current_date,
                negotiation.current_offer.base_fee.clone(),
                transfer_type,
            )
            .with_reason(reason);

            self.transfer_history.push(completed.clone());

            // Clear all remaining active negotiations for this player
            self.cancel_negotiations_for_player(player_id, negotiation_id);

            // Cancel all other listings for this player
            for listing in &mut self.listings {
                if listing.player_id == player_id
                    && listing.status != TransferListingStatus::Completed
                {
                    listing.status = TransferListingStatus::Cancelled;
                }
            }

            Some(completed)
        } else {
            None
        }
    }

    /// Cancel all active negotiations for a player, except the completed
    /// one. Public because the free-agent pool completion path closes a
    /// deal without going through [`Self::complete_transfer`] (the
    /// history row is written by the deferred executor instead) and
    /// still needs the losing bids swept.
    pub fn cancel_negotiations_for_player(&mut self, player_id: u32, except_negotiation_id: u32) {
        for (id, negotiation) in &mut self.negotiations {
            if negotiation.player_id == player_id
                && *id != except_negotiation_id
                && (negotiation.status == NegotiationStatus::Pending
                    || negotiation.status == NegotiationStatus::Countered)
            {
                negotiation.status = NegotiationStatus::Rejected;
            }
        }
    }

    pub fn update(&mut self, current_date: NaiveDate) -> Vec<(u32, u32)> {
        // Stale-listing decay: an Available listing sitting without a bid
        // loses 5% of its asking every 7 days, down to 60% of the original
        // ask. Gives sellers a natural market-clearing signal and keeps
        // listings from sitting forever at an unrealistic price.
        for listing in self.listings.iter_mut() {
            if listing.status != TransferListingStatus::Available {
                continue;
            }
            let days_since_decay = (current_date - listing.last_decay_date).num_days();
            if days_since_decay < 7 {
                continue;
            }
            let steps = (days_since_decay / 7) as i32;
            let floor = listing.original_asking_price.amount * 0.6;
            let multiplier = 0.95_f64.powi(steps);
            let decayed = (listing.asking_price.amount * multiplier).max(floor);
            listing.asking_price.amount = decayed;
            listing.last_decay_date = listing
                .last_decay_date
                .checked_add_signed(Duration::days(steps as i64 * 7))
                .unwrap_or(current_date);
        }

        // Check for expired negotiations
        let expired_ids: Vec<u32> = self
            .negotiations
            .iter_mut()
            .filter_map(|(id, negotiation)| {
                if negotiation.check_expired(current_date) {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();

        // Collect (buying_club_id, player_id) for pipeline cleanup
        let mut expired_info: Vec<(u32, u32)> = Vec::new();

        // Update listings for expired negotiations
        for id in expired_ids {
            if let Some((buying_club_id, player_id)) = self
                .negotiations
                .get(&id)
                .map(|n| (n.buying_club_id, n.player_id))
            {
                expired_info.push((buying_club_id, player_id));
                self.reopen_listing_if_no_active_bids(player_id);
            }
        }

        expired_info
    }

    fn reopen_listing_if_no_active_bids(&mut self, player_id: u32) {
        let has_other_active = self.negotiations.values().any(|n| {
            n.player_id == player_id
                && (n.status == NegotiationStatus::Pending
                    || n.status == NegotiationStatus::Countered)
        });

        if !has_other_active {
            for listing in &mut self.listings {
                if listing.player_id == player_id
                    && listing.status == TransferListingStatus::InNegotiation
                {
                    listing.status = TransferListingStatus::Available;
                }
            }
        }
    }

    /// Check if a specific player already has an active negotiation from a given buyer.
    pub fn has_active_negotiation_for(&self, player_id: u32, buying_club_id: u32) -> bool {
        self.negotiations.values().any(|n| {
            n.player_id == player_id
                && n.buying_club_id == buying_club_id
                && (n.status == NegotiationStatus::Pending
                    || n.status == NegotiationStatus::Countered)
        })
    }

    /// Count how many active negotiations a specific club currently has as a buyer.
    pub fn active_negotiation_count_for_club(&self, club_id: u32) -> u32 {
        self.negotiations
            .values()
            .filter(|n| {
                n.buying_club_id == club_id
                    && (n.status == NegotiationStatus::Pending
                        || n.status == NegotiationStatus::Countered)
            })
            .count() as u32
    }

    pub fn check_transfer_window(&mut self, is_open: bool) {
        self.transfer_window_open = is_open;

        // Interest, listings, and agreed talks survive closed windows. The
        // window controls when new pipeline activity and registrations happen;
        // it should not erase the market's memory between windows.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::{Currency, CurrencyValue};

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn money(amount: f64) -> CurrencyValue {
        CurrencyValue {
            amount,
            currency: Currency::Usd,
        }
    }

    fn offer(buyer_id: u32, amount: f64) -> TransferOffer {
        TransferOffer::new(money(amount), buyer_id, d(2026, 7, 1))
    }

    #[test]
    fn allows_multiple_buyers_to_bid_for_same_listing() {
        let mut market = TransferMarket::new();
        market.add_listing(TransferListing::new(
            10,
            1,
            100,
            money(1_000_000.0),
            d(2026, 7, 1),
            TransferListingType::Transfer,
        ));

        let first =
            market.start_negotiation(10, 2, offer(2, 850_000.0), d(2026, 7, 1), 0.5, 0.6, 24, 0.5);
        let second =
            market.start_negotiation(10, 3, offer(3, 900_000.0), d(2026, 7, 1), 0.5, 0.7, 24, 0.5);

        assert!(first.is_some());
        assert!(second.is_some());
        assert_eq!(market.negotiations.len(), 2);
        assert_eq!(
            market.listings[0].status,
            TransferListingStatus::InNegotiation
        );
    }

    #[test]
    fn rejects_duplicate_active_bid_from_same_buyer() {
        let mut market = TransferMarket::new();
        market.add_listing(TransferListing::new(
            10,
            1,
            100,
            money(1_000_000.0),
            d(2026, 7, 1),
            TransferListingType::Transfer,
        ));

        assert!(
            market
                .start_negotiation(10, 2, offer(2, 850_000.0), d(2026, 7, 1), 0.5, 0.6, 24, 0.5)
                .is_some()
        );
        assert!(
            market
                .start_negotiation(10, 2, offer(2, 900_000.0), d(2026, 7, 2), 0.5, 0.6, 24, 0.5)
                .is_none()
        );
    }

    #[test]
    fn closing_window_preserves_market_interest() {
        let mut market = TransferMarket::new();
        market.add_listing(TransferListing::new(
            10,
            1,
            100,
            money(1_000_000.0),
            d(2026, 7, 1),
            TransferListingType::Transfer,
        ));
        let negotiation_id = market
            .start_negotiation(10, 2, offer(2, 850_000.0), d(2026, 7, 1), 0.5, 0.6, 24, 0.5)
            .unwrap();

        market.check_transfer_window(false);

        assert_eq!(
            market.listings[0].status,
            TransferListingStatus::InNegotiation
        );
        assert_eq!(
            market.negotiations.get(&negotiation_id).map(|n| &n.status),
            Some(&NegotiationStatus::Pending),
        );
    }
}

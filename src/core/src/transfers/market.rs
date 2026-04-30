use crate::shared::CurrencyValue;
use crate::transfers::negotiation::{NegotiationStatus, TransferNegotiation};
use crate::transfers::offer::TransferOffer;
use crate::transfers::{CompletedTransfer, TransferType};
use chrono::NaiveDate;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct TransferMarket {
    pub listings: Vec<TransferListing>,
    pub negotiations: HashMap<u32, TransferNegotiation>,
    pub transfer_window_open: bool,
    pub transfer_history: Vec<CompletedTransfer>,
    pub next_negotiation_id: u32,
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
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransferListingType {
    Transfer,
    Loan,
    EndOfContract,
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
        }
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
        }
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
            // may approach a Transfer-listed player as a loan or vice versa
            let transfer_type = if negotiation.is_loan {
                let loan_end = negotiation
                    .current_offer
                    .contract_length
                    .map(|months| {
                        current_date
                            .checked_add_signed(chrono::Duration::days(months as i64 * 30))
                            .unwrap_or(current_date)
                    })
                    .unwrap_or_else(|| {
                        current_date
                            .checked_add_signed(chrono::Duration::days(180))
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

    /// Cancel all active negotiations for a player, except the completed one
    fn cancel_negotiations_for_player(&mut self, player_id: u32, except_negotiation_id: u32) {
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
                .checked_add_signed(chrono::Duration::days(steps as i64 * 7))
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

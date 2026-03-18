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
    pub listed_date: NaiveDate,
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
            asking_price,
            listed_date,
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
        if !self.listings.iter().any(|l| l.player_id == listing.player_id
            && l.status == TransferListingStatus::Available) {
            self.listings.push(listing);
        }
    }

    pub fn get_available_listings(&self) -> Vec<&TransferListing> {
        self.listings.iter()
            .filter(|l| l.status == TransferListingStatus::Available)
            .collect()
    }

    pub fn get_listing_by_player(&self, player_id: u32) -> Option<&TransferListing> {
        self.listings.iter()
            .find(|l| l.player_id == player_id && l.status == TransferListingStatus::Available)
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
        // Find the listing
        if let Some(listing_index) = self.listings.iter().position(|l|
            l.player_id == player_id &&
                l.status == TransferListingStatus::Available) {
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

            // Update listing status
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
                let loan_end = negotiation.current_offer.contract_length
                    .map(|months| {
                        current_date.checked_add_signed(chrono::Duration::days(months as i64 * 30))
                            .unwrap_or(current_date)
                    })
                    .unwrap_or_else(|| {
                        current_date.checked_add_signed(chrono::Duration::days(180))
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
            ).with_reason(reason);

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
        // Check for expired negotiations
        let expired_ids: Vec<u32> = self.negotiations.iter_mut()
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
            if let Some(negotiation) = self.negotiations.get(&id) {
                expired_info.push((negotiation.buying_club_id, negotiation.player_id));

                let listing_idx = negotiation.listing_id as usize;
                if listing_idx < self.listings.len() {
                    let listing = &mut self.listings[listing_idx];
                    listing.status = TransferListingStatus::Available;
                }
            }
        }

        expired_info
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

        // If window closes, cancel all active listings and negotiations
        if !is_open {
            // Mark all available listings as cancelled
            for listing in &mut self.listings {
                if listing.status == TransferListingStatus::Available ||
                    listing.status == TransferListingStatus::InNegotiation {
                    listing.status = TransferListingStatus::Cancelled;
                }
            }

            // Mark all pending negotiations as expired
            for (_, negotiation) in &mut self.negotiations {
                if negotiation.status == NegotiationStatus::Pending ||
                    negotiation.status == NegotiationStatus::Countered {
                    negotiation.status = NegotiationStatus::Expired;
                }
            }
        }
    }
}
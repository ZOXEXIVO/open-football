use std::collections::HashMap;
use chrono::NaiveDate;
use crate::shared::CurrencyValue;
use crate::transfers::{CompletedTransfer, TransferType};
use crate::transfers::negotiation::{NegotiationStatus, TransferNegotiation};
use crate::transfers::offer::TransferOffer;

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

    pub fn start_negotiation(
        &mut self,
        player_id: u32,
        buying_club_id: u32,
        offer: TransferOffer,
        current_date: NaiveDate,
    ) -> Option<u32> {
        // Find the listing
        if let Some(listing_index) = self.listings.iter().position(|l|
            l.player_id == player_id &&
                l.status == TransferListingStatus::Available) {

            let listing = &mut self.listings[listing_index];

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

    pub fn complete_transfer(&mut self, negotiation_id: u32, current_date: NaiveDate) -> Option<CompletedTransfer> {
        // Find the negotiation
        if let Some(negotiation) = self.negotiations.get(&negotiation_id) {
            if negotiation.status != NegotiationStatus::Accepted {
                return None;
            }

            // Get the listing index
            let listing_idx = negotiation.listing_id as usize;
            if listing_idx >= self.listings.len() {
                return None;
            }

            // Update listing status
            if let Some(listing) = self.listings.get_mut(listing_idx) {
                listing.status = TransferListingStatus::Completed;
            }

            // Create a completed transfer record
            let transfer_type = match self.listings.get(listing_idx).unwrap().listing_type {
                TransferListingType::Loan => {
                    // Assume 6-month loan if contract length not specified
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
                },
                TransferListingType::EndOfContract => TransferType::Free,
                _ => TransferType::Permanent,
            };

            let completed = CompletedTransfer::new(
                negotiation.player_id,
                negotiation.selling_club_id,
                negotiation.buying_club_id,
                current_date,
                negotiation.current_offer.base_fee.clone(),
                transfer_type,
            );

            // Add to history
            self.transfer_history.push(completed.clone());

            Some(completed)
        } else {
            None
        }
    }

    pub fn update(&mut self, current_date: NaiveDate) {
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

        // Update listings for expired negotiations
        for id in expired_ids {
            if let Some(negotiation) = self.negotiations.get(&id) {
                let listing_idx = negotiation.listing_id as usize;
                if listing_idx < self.listings.len() {
                    let listing = &mut self.listings[listing_idx];
                    listing.status = TransferListingStatus::Available;
                }
            }
        }
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
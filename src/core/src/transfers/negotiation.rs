use chrono::NaiveDate;
use crate::transfers::offer::TransferOffer;

#[derive(Debug, Clone)]
pub struct TransferNegotiation {
    pub id: u32,
    pub player_id: u32,
    pub listing_id: u32,
    pub selling_club_id: u32,
    pub buying_club_id: u32,
    pub current_offer: TransferOffer,
    pub counter_offers: Vec<TransferOffer>,
    pub status: NegotiationStatus,
    pub expiry_date: NaiveDate,
    pub created_date: NaiveDate,
}

#[derive(Debug, PartialEq, Clone)]
pub enum NegotiationStatus {
    Pending,
    Accepted,
    Rejected,
    Countered,
    Expired,
}

impl TransferNegotiation {
    pub fn new(
        id: u32,
        player_id: u32,
        listing_id: u32,
        selling_club_id: u32,
        buying_club_id: u32,
        initial_offer: TransferOffer,
        created_date: NaiveDate,
    ) -> Self {
        // Negotiations expire after 3 days by default
        let expiry_date = created_date.checked_add_signed(chrono::Duration::days(3))
            .unwrap_or(created_date);

        TransferNegotiation {
            id,
            player_id,
            listing_id,
            selling_club_id,
            buying_club_id,
            current_offer: initial_offer,
            counter_offers: Vec::new(),
            status: NegotiationStatus::Pending,
            expiry_date,
            created_date,
        }
    }

    pub fn counter_offer(&mut self, counter: TransferOffer) {
        self.counter_offers.push(self.current_offer.clone());
        self.current_offer = counter;
        self.status = NegotiationStatus::Countered;
    }

    pub fn accept(&mut self) {
        self.status = NegotiationStatus::Accepted;
    }

    pub fn reject(&mut self) {
        self.status = NegotiationStatus::Rejected;
    }

    pub fn check_expired(&mut self, current_date: NaiveDate) -> bool {
        if current_date >= self.expiry_date && self.status == NegotiationStatus::Pending {
            self.status = NegotiationStatus::Expired;
            return true;
        }
        false
    }
}
use crate::transfers::offer::TransferOffer;
use crate::utils::IntegerUtils;
use chrono::NaiveDate;

#[derive(Debug, Clone)]
pub enum NegotiationPhase {
    /// Selling club decides whether to engage (3-7 days)
    InitialApproach { started: NaiveDate },
    /// Fee negotiation with counter-offers, up to 3 rounds (7-21 days per round)
    ClubNegotiation { started: NaiveDate, round: u8 },
    /// Player evaluates the move (5-14 days)
    PersonalTerms { started: NaiveDate },
    /// Medical exam and paperwork (3-5 days)
    MedicalAndFinalization { started: NaiveDate },
}

#[derive(Debug, Clone, PartialEq)]
pub enum NegotiationRejectionReason {
    SellerRefusedToNegotiate,
    AskingPriceTooHigh,
    PlayerTooImportant,
    PlayerRejectedPersonalTerms,
    ReputationGapTooLarge,
    SalaryDemandsUnmet,
    MedicalFailed,
    WindowClosed,
}

#[derive(Debug, PartialEq, Clone)]
pub enum NegotiationStatus {
    Pending,
    Accepted,
    Rejected,
    Countered,
    Expired,
}

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
    pub is_loan: bool,
    pub is_unsolicited: bool,

    // Phased negotiation fields
    pub phase: NegotiationPhase,
    pub phase_expiry: NaiveDate,
    pub offered_salary: Option<u32>,
    pub selling_club_reputation: f32,
    pub buying_club_reputation: f32,
    pub player_age: u8,
    pub player_ambition: f32,
    pub rejection_reason: Option<NegotiationRejectionReason>,
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
        selling_club_reputation: f32,
        buying_club_reputation: f32,
        player_age: u8,
        player_ambition: f32,
    ) -> Self {
        // Overall deadline: 60 days
        let expiry_date = created_date
            .checked_add_signed(chrono::Duration::days(60))
            .unwrap_or(created_date);

        // Initial approach phase: 3-7 days
        let phase_duration = IntegerUtils::random(3, 7) as i64;
        let phase_expiry = created_date
            .checked_add_signed(chrono::Duration::days(phase_duration))
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
            is_loan: false,
            is_unsolicited: false,
            phase: NegotiationPhase::InitialApproach {
                started: created_date,
            },
            phase_expiry,
            offered_salary: None,
            selling_club_reputation,
            buying_club_reputation,
            player_age,
            player_ambition,
            rejection_reason: None,
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

    /// Check if the current phase is ready to be resolved
    pub fn is_phase_ready(&self, current_date: NaiveDate) -> bool {
        current_date >= self.phase_expiry
            && (self.status == NegotiationStatus::Pending
                || self.status == NegotiationStatus::Countered)
    }

    /// Old compatibility: ready to resolve means the current phase is ready
    pub fn is_ready_to_resolve(&self, current_date: NaiveDate) -> bool {
        self.is_phase_ready(current_date)
    }

    pub fn advance_to_club_negotiation(&mut self, current_date: NaiveDate) {
        let duration = IntegerUtils::random(7, 21) as i64;
        self.phase = NegotiationPhase::ClubNegotiation {
            started: current_date,
            round: 1,
        };
        self.phase_expiry = current_date
            .checked_add_signed(chrono::Duration::days(duration))
            .unwrap_or(current_date);
    }

    pub fn advance_club_negotiation_round(&mut self, current_date: NaiveDate) {
        if let NegotiationPhase::ClubNegotiation { round, .. } = self.phase {
            let duration = IntegerUtils::random(7, 14) as i64;
            self.phase = NegotiationPhase::ClubNegotiation {
                started: current_date,
                round: round + 1,
            };
            self.phase_expiry = current_date
                .checked_add_signed(chrono::Duration::days(duration))
                .unwrap_or(current_date);
        }
    }

    pub fn advance_to_personal_terms(&mut self, current_date: NaiveDate) {
        let duration = IntegerUtils::random(5, 14) as i64;
        self.phase = NegotiationPhase::PersonalTerms {
            started: current_date,
        };
        self.phase_expiry = current_date
            .checked_add_signed(chrono::Duration::days(duration))
            .unwrap_or(current_date);
    }

    pub fn advance_to_medical(&mut self, current_date: NaiveDate) {
        let duration = IntegerUtils::random(3, 5) as i64;
        self.phase = NegotiationPhase::MedicalAndFinalization {
            started: current_date,
        };
        self.phase_expiry = current_date
            .checked_add_signed(chrono::Duration::days(duration))
            .unwrap_or(current_date);
    }

    pub fn reject_with_reason(&mut self, reason: NegotiationRejectionReason) {
        self.rejection_reason = Some(reason);
        self.status = NegotiationStatus::Rejected;
    }

    pub fn check_expired(&mut self, current_date: NaiveDate) -> bool {
        if current_date >= self.expiry_date
            && (self.status == NegotiationStatus::Pending
                || self.status == NegotiationStatus::Countered)
        {
            self.rejection_reason = Some(NegotiationRejectionReason::WindowClosed);
            self.status = NegotiationStatus::Expired;
            return true;
        }
        false
    }
}

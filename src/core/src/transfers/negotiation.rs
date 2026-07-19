use crate::PlayerFieldPositionGroup;
use crate::transfers::offer::TransferOffer;
use crate::utils::IntegerUtils;
use chrono::Duration;
use chrono::NaiveDate;

#[derive(Debug, Clone)]
pub enum NegotiationPhase {
    /// Selling club decides whether to engage (1-3 days)
    InitialApproach { started: NaiveDate },
    /// Fee negotiation with counter-offers, up to 3 rounds (3-7 days per round)
    ClubNegotiation { started: NaiveDate, round: u8 },
    /// Player evaluates the move (2-5 days). `round` counts wage-negotiation
    /// passes: a deal that stalls purely on money lets the buyer improve its
    /// offer and the player re-evaluate, up to a small cap.
    PersonalTerms { started: NaiveDate, round: u8 },
    /// Medical exam and paperwork (1-3 days)
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
    /// The simulation refuses this cross-country move because the buyer
    /// and seller countries sit on the
    /// [`crate::transfers::TransferRoutePolicy`] route block list on the
    /// current sim date (today: Russia ↔ Ukraine after 2022-02-24).
    CountryPairRouteBlocked,
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
    /// True when the buyer proposed Loan-with-option-to-buy. Recorded as a
    /// contractual option on the resulting loan contract.
    pub has_option_to_buy: bool,
    pub is_unsolicited: bool,
    /// Buying club's league reputation at negotiation start (0–10000).
    /// Anchors the player's reservation wage during PersonalTerms.
    pub buying_league_reputation: u16,

    // Phased negotiation fields
    pub phase: NegotiationPhase,
    pub phase_expiry: NaiveDate,
    /// Annual wage the buying club has staged for the player. None until the
    /// pipeline fills it at negotiation start.
    pub offered_salary: Option<u32>,
    /// The player's reservation wage captured at negotiation start, for
    /// negotiations whose player can't be re-read at resolution time
    /// (foreign moves, global-pool free agents). Foreign moves stage it
    /// ABOVE the opening wage — the relocation premium a player expects for
    /// crossing a border — which is what gives the iterative wage rounds a
    /// real gap to close. Domestic club-to-club deals leave it `None` and
    /// recompute the reservation live from the player.
    pub staged_reservation_wage: Option<u32>,
    pub selling_club_reputation: f32,
    pub buying_club_reputation: f32,
    pub player_age: u8,
    pub player_ambition: f32,
    pub rejection_reason: Option<NegotiationRejectionReason>,

    /// Staff member responsible for negotiating this transfer
    pub negotiator_staff_id: Option<u32>,
    /// Reason for signing (from scout report / transfer request)
    pub reason: String,
    /// Source country ID (None = same country as buying club)
    pub selling_country_id: Option<u32>,
    /// Selling country's continent_id (for geographic preference checks)
    pub selling_continent_id: Option<u32>,
    /// Selling country's code (for geographic preference checks)
    pub selling_country_code: String,
    /// If the buying club previously sold this player: (club_id, fee).
    /// Used in personal terms — player may resist returning to club that sold them.
    pub player_sold_from: Option<(u32, f64)>,
    /// Cached player name (resolved at negotiation start)
    pub player_name: String,
    /// Cached selling club name (resolved at negotiation start)
    pub selling_club_name: String,
    /// Foreign moves only: captured at creation from the full cross-border
    /// assessment — the player would refuse this move on willingness grounds
    /// (a clear sporting step down with no availability signal). The
    /// personal-terms resolver reads it as the foreign hard floor, since the
    /// buyer's country no longer holds the seller-side data to recompute it.
    /// `false` for domestic negotiations (their floor recomputes live).
    pub foreign_terms_floor_blocked: bool,
    /// Foreign moves only: the seller-side player importance captured at
    /// creation (the buyer's country can't recompute it once the deal is in
    /// flight). The club-fee resolver reads this instead of a flat constant
    /// so forcing a deal through abroad is no easier/cheaper than the
    /// equivalent domestic move. `None` for domestic moves, whose importance
    /// recomputes live.
    pub foreign_seller_importance: Option<f32>,
    /// Loan-in target's `(position group, ability)` captured at creation.
    /// The borrower depth cap folds pending loans by resolving each
    /// negotiation's player in-country — impossible for FOREIGN targets,
    /// which made in-flight cross-border loans invisible to the cap and
    /// re-opened the loan over-accumulation hole. `None` for permanent
    /// deals and for legacy rows (the fold then falls back to the lookup).
    pub loan_target_profile: Option<(PlayerFieldPositionGroup, u8)>,
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
        // Overall deadline: 45 days
        let expiry_date = created_date
            .checked_add_signed(Duration::days(45))
            .unwrap_or(created_date);

        // Initial approach phase: 1 day
        let phase_duration = 1i64;
        let phase_expiry = created_date
            .checked_add_signed(Duration::days(phase_duration))
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
            has_option_to_buy: false,
            is_unsolicited: false,
            buying_league_reputation: 0,
            phase: NegotiationPhase::InitialApproach {
                started: created_date,
            },
            phase_expiry,
            offered_salary: None,
            staged_reservation_wage: None,
            selling_club_reputation,
            buying_club_reputation,
            player_age,
            player_ambition,
            rejection_reason: None,
            negotiator_staff_id: None,
            reason: String::new(),
            selling_country_id: None,
            selling_continent_id: None,
            selling_country_code: String::new(),
            player_sold_from: None,
            player_name: String::new(),
            selling_club_name: String::new(),
            foreign_terms_floor_blocked: false,
            foreign_seller_importance: None,
            loan_target_profile: None,
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
        let duration = IntegerUtils::random(2, 4) as i64;
        self.phase = NegotiationPhase::ClubNegotiation {
            started: current_date,
            round: 1,
        };
        self.phase_expiry = current_date
            .checked_add_signed(Duration::days(duration))
            .unwrap_or(current_date);
    }

    pub fn advance_club_negotiation_round(&mut self, current_date: NaiveDate) {
        if let NegotiationPhase::ClubNegotiation { round, .. } = self.phase {
            let duration = IntegerUtils::random(2, 4) as i64;
            self.phase = NegotiationPhase::ClubNegotiation {
                started: current_date,
                round: round + 1,
            };
            self.phase_expiry = current_date
                .checked_add_signed(Duration::days(duration))
                .unwrap_or(current_date);
        }
    }

    pub fn advance_to_personal_terms(&mut self, current_date: NaiveDate) {
        let duration = IntegerUtils::random(1, 3) as i64;
        self.phase = NegotiationPhase::PersonalTerms {
            started: current_date,
            round: 0,
        };
        self.phase_expiry = current_date
            .checked_add_signed(Duration::days(duration))
            .unwrap_or(current_date);
    }

    /// Re-enter the personal-terms phase for another wage round after the
    /// buyer has improved its offer. Bumps the round counter (the caller caps
    /// the number of rounds) and resets the short phase timer so the player
    /// evaluates the new offer on the next tick.
    pub fn advance_personal_terms_round(&mut self, current_date: NaiveDate) {
        let round = match self.phase {
            NegotiationPhase::PersonalTerms { round, .. } => round.saturating_add(1),
            _ => 1,
        };
        let duration = IntegerUtils::random(1, 2) as i64;
        self.phase = NegotiationPhase::PersonalTerms {
            started: current_date,
            round,
        };
        self.phase_expiry = current_date
            .checked_add_signed(Duration::days(duration))
            .unwrap_or(current_date);
    }

    /// Raise the wage the buyer is putting on the table during personal-terms
    /// negotiation. Updates both the loose `offered_salary` (which the
    /// acceptance roll reads) and the structured personal-terms package's
    /// annual wage (the authoritative figure installed on completion), so the
    /// wage the player accepts is exactly the wage he is then paid.
    pub fn raise_offered_salary(&mut self, new_wage: u32) {
        self.offered_salary = Some(new_wage);
        if let Some(terms) = self.current_offer.personal_terms.as_mut() {
            terms.annual_wage = Some(new_wage);
        }
    }

    pub fn advance_to_medical(&mut self, current_date: NaiveDate) {
        let duration = 1i64;
        self.phase = NegotiationPhase::MedicalAndFinalization {
            started: current_date,
        };
        self.phase_expiry = current_date
            .checked_add_signed(Duration::days(duration))
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

#[cfg(test)]
mod personal_terms_round_tests {
    use super::{NegotiationPhase, TransferNegotiation};
    use crate::shared::{Currency, CurrencyValue};
    use crate::transfers::offer::TransferOffer;
    use chrono::NaiveDate;

    fn negotiation(date: NaiveDate) -> TransferNegotiation {
        let offer = TransferOffer::new(CurrencyValue::new(1_000_000.0, Currency::Usd), 2, date);
        TransferNegotiation::new(1, 100, 0, 10, 2, offer, date, 0.5, 0.5, 25, 0.5)
    }

    #[test]
    fn personal_terms_starts_at_round_zero_then_advances() {
        let date = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        let mut n = negotiation(date);

        n.advance_to_personal_terms(date);
        match n.phase {
            NegotiationPhase::PersonalTerms { round, .. } => assert_eq!(round, 0),
            _ => panic!("expected PersonalTerms phase"),
        }

        n.advance_personal_terms_round(date);
        n.advance_personal_terms_round(date);
        match n.phase {
            NegotiationPhase::PersonalTerms { round, .. } => assert_eq!(round, 2),
            _ => panic!("expected PersonalTerms phase after two wage rounds"),
        }
    }

    #[test]
    fn raise_offered_salary_updates_the_loose_wage() {
        let date = NaiveDate::from_ymd_opt(2026, 7, 1).unwrap();
        let mut n = negotiation(date);
        assert!(n.offered_salary.is_none());

        n.raise_offered_salary(80_000);
        assert_eq!(n.offered_salary, Some(80_000));
        // A bare offer carries no structured personal-terms package, so only
        // the loose wage updates — and the call must not panic.
        assert!(n.current_offer.personal_terms.is_none());
    }
}

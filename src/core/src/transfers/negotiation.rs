use crate::PlayerFieldPositionGroup;
use crate::transfers::offer::TransferOffer;
use crate::transfers::pipeline::appraisal::{PlayerStance, TermsRefusalCause};
use crate::transfers::pipeline::planning::BriefTier;
use crate::transfers::reason::TransferReason;
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
    /// Selling club's league reputation at negotiation start (0–10000).
    /// Paired with the buyer's so personal terms can read the move as a
    /// step up, sideways, or down IN STAGE — which is what a player
    /// chasing a bigger league is actually weighing, and which the club
    /// reputations alone cannot express (a mid-table Spanish side is a
    /// smaller club than Zenit and a far bigger stage).
    pub selling_league_reputation: u16,
    /// The player's big-stage pull (0..1) captured at creation. Foreign
    /// moves cannot re-read the player at resolution time — he lives in
    /// another country's borrow — so his appetite for a bigger stage is
    /// staged here the same way seller importance already is. Domestic
    /// deals recompute it live and ignore this.
    pub player_stage_inclination: f32,

    // Phased negotiation fields
    pub phase: NegotiationPhase,
    pub phase_expiry: NaiveDate,
    /// Annual wage the buying club has staged for the player. None until the
    /// pipeline fills it at negotiation start.
    pub offered_salary: Option<u32>,
    /// The wage the buyer OPENED at, before any personal-terms round
    /// raised it. `None` until the first raise, which is when it is
    /// latched. See [`Self::raise_offered_salary`].
    pub opening_salary: Option<u32>,
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
    pub reason: TransferReason,
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
    /// Foreign moves only: the selling club's annualised income, wage bill
    /// and wage budget, captured at creation. The seller lives in another
    /// country's borrow at resolution time, so the one number that decides
    /// these deals in real life — what the fee is worth against the
    /// club's own year — has to travel with the negotiation. `None` for
    /// domestic moves, which read the seller's books live.
    /// Order: `(annual income, annual wage bill, wage budget)`.
    pub foreign_seller_finances: Option<(i64, i64, i64)>,
    /// Loan-in target's `(position group, ability)` captured at creation.
    /// The borrower depth cap folds pending loans by resolving each
    /// negotiation's player in-country — impossible for FOREIGN targets,
    /// which made in-flight cross-border loans invisible to the cap and
    /// re-opened the loan over-accumulation hole. `None` for permanent
    /// deals and for legacy rows (the fold then falls back to the lookup).
    pub loan_target_profile: Option<(PlayerFieldPositionGroup, u8)>,
    /// The highest fee at which this deal is still worth doing to THIS
    /// buyer — the fee where its
    /// [`crate::transfers::pipeline::UpgradeMath`] value reaches zero,
    /// captured at creation.
    ///
    /// This is what a bidding war resolves on: two clubs keep raising until
    /// one of them reaches its own number and stops. Without it the
    /// escalation had only the seller's asking price to aim at, so every
    /// buyer converged on the same figure and nobody ever outbid anybody.
    /// `None` for negotiations opened by paths that do not value a squad
    /// improvement (loans, free agents, legacy rows); the escalation then
    /// falls back to its budget cap exactly as before.
    pub buyer_ceiling_fee: Option<f64>,
    /// How transformative the buying club means this signing to be — the
    /// [`crate::transfers::pipeline::planning::BriefTier`] of the request it
    /// answers, captured at creation. Drives the opening ratio and the
    /// deadline premium. `None` for negotiations with no brief behind them.
    pub brief_tier: Option<BriefTier>,
    /// The player's own side of the appraisal, staged at creation.
    ///
    /// Personal terms are resolved by the BUYING country's pass, so on a
    /// cross-border deal the player, his club, his depth chart and his mind
    /// are all inside another country's borrow by the time he is asked.
    /// Every player-side fact the appraisal reads is therefore captured
    /// when the seller's country IS in scope, exactly as
    /// `foreign_seller_importance` and `foreign_seller_finances` already
    /// are. `None` for domestic deals, which rebuild it live each round.
    pub staged_stance: Option<PlayerStance>,
    /// Sporting distance of the move, staged alongside the stance — it
    /// needs both clubs, and only creation has both.
    pub staged_sporting_drop: Option<f32>,
    /// Which axis the player refused on, once he has. Written by the
    /// personal-terms resolver for the story, the census and the trace.
    pub terms_refusal_cause: Option<TermsRefusalCause>,
    /// What he said he would go for, in the round he refused. Paired with
    /// the cause so a diagnostic can print "he would go for $31M a year;
    /// we can pay $12M" rather than a bare probability.
    pub terms_reservation_wage: Option<u32>,
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
            selling_league_reputation: 0,
            player_stage_inclination: 0.0,
            phase: NegotiationPhase::InitialApproach {
                started: created_date,
            },
            phase_expiry,
            offered_salary: None,
            opening_salary: None,
            staged_reservation_wage: None,
            selling_club_reputation,
            buying_club_reputation,
            player_age,
            player_ambition,
            rejection_reason: None,
            negotiator_staff_id: None,
            reason: TransferReason::default(),
            selling_country_id: None,
            selling_continent_id: None,
            selling_country_code: String::new(),
            player_sold_from: None,
            player_name: String::new(),
            selling_club_name: String::new(),
            foreign_terms_floor_blocked: false,
            foreign_seller_importance: None,
            foreign_seller_finances: None,
            loan_target_profile: None,
            buyer_ceiling_fee: None,
            brief_tier: None,
            staged_stance: None,
            staged_sporting_drop: None,
            terms_refusal_cause: None,
            terms_reservation_wage: None,
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

    /// Table the buyer's OPENING wage — the figure its own level implies.
    ///
    /// Sets the anchor at the same moment as the offer, because the two
    /// are the same number at creation and the anchor is what every later
    /// round's wage power is a stretch on. Leaving `opening_salary` for
    /// `raise_offered_salary` to latch meant a path that never staged a
    /// salary latched `None`, and round two then anchored on the raise —
    /// 1.3 compounding into 1.69.
    pub fn open_salary_at(&mut self, wage: u32) {
        self.offered_salary = Some(wage);
        if self.opening_salary.is_none() {
            self.opening_salary = Some(wage);
        }
        if let Some(terms) = self.current_offer.personal_terms.as_mut() {
            terms.annual_wage = Some(wage);
        }
    }

    /// Raise the wage the buyer is putting on the table during personal-terms
    /// negotiation. Updates both the loose `offered_salary` (which the
    /// acceptance roll reads) and the structured personal-terms package's
    /// annual wage (the authoritative figure installed on completion), so the
    /// wage the player accepts is exactly the wage he is then paid.
    pub fn raise_offered_salary(&mut self, new_wage: u32) {
        // Remember where the buyer opened. Its wage POWER is a stretch on
        // its own level wage, and if that anchor moved with each raise the
        // ceiling would chase the offer up its own ladder — two rounds of
        // 1.3× compounding into a club paying nearly 1.7× what its level
        // says it can. The opening figure is the level; the raises are the
        // stretch. Normally already stamped by `open_salary_at`.
        if self.opening_salary.is_none() {
            self.opening_salary = self.offered_salary;
        }
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

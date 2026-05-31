use crate::shared::Currency;
use crate::shared::CurrencyValue;
use chrono::NaiveDate;

/// An offer placed in the transfer market. The base fee + optional
/// clauses model the *club-to-club* side of the deal. The optional
/// [`PersonalTermsOffer`] holds the *club-to-player* package that gets
/// installed on the player's contract once the move closes — keeping
/// the two sides separate avoids the wage/length numbers leaking into
/// the seller-side acceptance maths.
#[derive(Debug, Clone)]
pub struct TransferOffer {
    pub base_fee: CurrencyValue,
    pub clauses: Vec<TransferClause>,
    pub salary_contribution: Option<CurrencyValue>, // For loans
    /// Permanent contract length the buyer is offering the **player**
    /// (in **years**). `None` lets the execution layer fall back to its
    /// age-banded default. Read by `complete_transfer` only for
    /// permanent moves; for loans, see [`loan_duration_months`].
    pub contract_length_years: Option<u8>,
    /// Loan length in **months** the buyer is requesting. `None` lets
    /// the execution layer compute the loan-end from the parent
    /// league's season calendar (the usual case). Kept separate from
    /// [`contract_length_years`] because callers historically passed a
    /// single `contract_length` field whose units were ambiguous —
    /// `complete_transfer` treated it as months for loans while
    /// `with_contract_length` documented it as years.
    pub loan_duration_months: Option<u8>,
    /// Personal-terms package the buyer is willing to commit to the
    /// player. Populated when the offer flows through the structured
    /// personal-terms phase; falls back to compute-on-execute defaults
    /// when None.
    pub personal_terms: Option<PersonalTermsOffer>,
    pub offering_club_id: u32,
    pub offered_date: NaiveDate,
}

/// Structured personal-terms package the buyer is committing to. Lives
/// next to [`TransferOffer`] so the offer carries everything the
/// execution layer needs to install the resulting contract — no
/// compute-on-execute defaults silently overriding what the negotiation
/// settled.
///
/// Fields are intentionally `Option` where they're "use the calculator
/// default if absent" — the execution layer fills only what is staged.
#[derive(Debug, Clone, Default)]
pub struct PersonalTermsOffer {
    /// Annual salary the buyer commits to.
    pub annual_wage: Option<u32>,
    /// One-off signing bonus paid on contract installation. Captured as
    /// a contract bonus during install.
    pub signing_bonus: Option<u32>,
    /// One-off agent fee paid at signing — flows through the buying
    /// club's finance as a cash outflow at install time.
    pub agent_fee: Option<u32>,
    /// Permanent contract length in **years**. Overrides the
    /// `TransferOffer::contract_length_years` field when both are set —
    /// negotiation-side data is the authoritative source.
    pub contract_years: Option<u8>,
    /// Promised squad role at the buying club. Drives the player's
    /// shock / role-fit events on arrival.
    pub squad_status_promise: Option<PromisedSquadStatus>,
    /// Release-clause fee written into the new contract on install.
    /// Always `None` unless the negotiation explicitly agreed on one.
    pub release_clause_fee: Option<u32>,
}

/// Squad role the buyer promises the player as part of personal terms.
/// A subset of [`PlayerSquadStatus`] (only the roles that come up as
/// realistic public promises) so the negotiation can't accidentally
/// commit to internal states like `NotYetSet` or `Invalid`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromisedSquadStatus {
    KeyPlayer,
    FirstTeamRegular,
    FirstTeamSquadRotation,
    HotProspectForTheFuture,
}

#[derive(Debug, Clone)]
pub enum TransferClause {
    AppearanceFee(CurrencyValue, u32),  // Money after X appearances
    GoalBonus(CurrencyValue, u32),      // Money after X goals
    SellOnClause(f32),                  // Percentage of future transfer
    PromotionBonus(CurrencyValue),      // Money if buying club gets promoted
    Installments(CurrencyValue, u8),    // Money paid over N years
    LoanOptionToBuy(CurrencyValue),     // Optional future permanent fee
    LoanObligationToBuy(CurrencyValue), // Mandatory future permanent fee
}

impl Default for TransferOffer {
    fn default() -> Self {
        TransferOffer {
            base_fee: CurrencyValue {
                amount: 0.0,
                currency: Currency::Usd,
            },
            clauses: Vec::new(),
            salary_contribution: None,
            contract_length_years: None,
            loan_duration_months: None,
            personal_terms: None,
            offering_club_id: 0,
            offered_date: NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
        }
    }
}

impl TransferOffer {
    pub fn new(base_fee: CurrencyValue, offering_club_id: u32, offered_date: NaiveDate) -> Self {
        TransferOffer {
            base_fee,
            clauses: Vec::new(),
            salary_contribution: None,
            contract_length_years: None,
            loan_duration_months: None,
            personal_terms: None,
            offering_club_id,
            offered_date,
        }
    }

    pub fn with_clause(mut self, clause: TransferClause) -> Self {
        self.clauses.push(clause);
        self
    }

    pub fn with_salary_contribution(mut self, contribution: CurrencyValue) -> Self {
        self.salary_contribution = Some(contribution);
        self
    }

    /// Set the permanent-deal contract length (years). Has no effect on
    /// loan duration — see [`with_loan_duration_months`] for that.
    pub fn with_contract_length(mut self, years: u8) -> Self {
        self.contract_length_years = Some(years);
        self
    }

    /// Set the loan duration (months). Independent of
    /// [`contract_length_years`] so callers can't accidentally encode
    /// "1" and have it mean a 1-month loan AND a 1-year permanent
    /// contract at the same time.
    pub fn with_loan_duration_months(mut self, months: u8) -> Self {
        self.loan_duration_months = Some(months);
        self
    }

    /// Attach the agreed personal terms. Overrides legacy fallbacks for
    /// wage / contract length on the execution side.
    pub fn with_personal_terms(mut self, terms: PersonalTermsOffer) -> Self {
        self.personal_terms = Some(terms);
        self
    }

    /// Resolved permanent contract length (years), preferring the
    /// personal-terms package when present.
    pub fn resolved_contract_years(&self) -> Option<u8> {
        self.personal_terms
            .as_ref()
            .and_then(|t| t.contract_years)
            .or(self.contract_length_years)
    }

    /// Resolved annual wage commitment, if any was staged.
    pub fn resolved_annual_wage(&self) -> Option<u32> {
        self.personal_terms.as_ref().and_then(|t| t.annual_wage)
    }

    /// Resolved release-clause fee, if the personal terms include one.
    pub fn resolved_release_clause_fee(&self) -> Option<u32> {
        self.personal_terms
            .as_ref()
            .and_then(|t| t.release_clause_fee)
    }

    /// Resolved signing-bonus commitment, if any.
    pub fn resolved_signing_bonus(&self) -> Option<u32> {
        self.personal_terms.as_ref().and_then(|t| t.signing_bonus)
    }

    /// Resolved agent fee — paid by the buying club at signing.
    pub fn resolved_agent_fee(&self) -> Option<u32> {
        self.personal_terms.as_ref().and_then(|t| t.agent_fee)
    }

    /// Resolved promised squad status, if the personal terms set one.
    pub fn resolved_squad_status_promise(&self) -> Option<PromisedSquadStatus> {
        self.personal_terms
            .as_ref()
            .and_then(|t| t.squad_status_promise)
    }

    pub fn total_potential_value(&self) -> f64 {
        let mut total = self.base_fee.amount;

        for clause in &self.clauses {
            match clause {
                TransferClause::AppearanceFee(fee, _) => total += fee.amount * 0.7, // Assume 70% chance of meeting appearances
                TransferClause::GoalBonus(fee, _) => total += fee.amount * 0.5, // Assume 50% chance of meeting goal bonus
                TransferClause::SellOnClause(percentage) => {
                    total += total * (*percentage as f64) * 0.3
                } // Assume 30% chance of future sale
                TransferClause::PromotionBonus(fee) => total += fee.amount * 0.2, // Assume 20% chance of promotion
                // `Installments` is a payment STRUCTURE: it splits the
                // *existing* base fee across N years rather than adding
                // a new cash component on top. So it shouldn't lift
                // `total` (counted twice would double the headline);
                // it only affects timing, which the bid valuation
                // captures via a separate up-front discount factor.
                TransferClause::Installments(_, _) => {}
                TransferClause::LoanOptionToBuy(fee) => total += fee.amount * 0.25,
                TransferClause::LoanObligationToBuy(fee) => total += fee.amount,
            }
        }

        total
    }

    pub fn loan_future_fee(&self) -> Option<(CurrencyValue, bool)> {
        self.clauses.iter().find_map(|clause| match clause {
            TransferClause::LoanOptionToBuy(fee) => Some((fee.clone(), false)),
            TransferClause::LoanObligationToBuy(fee) => Some((fee.clone(), true)),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::{Currency, CurrencyValue};

    fn money(amount: f64) -> CurrencyValue {
        CurrencyValue {
            amount,
            currency: Currency::Usd,
        }
    }

    #[test]
    fn extracts_loan_option_to_buy_as_future_fee() {
        let offer = TransferOffer::default()
            .with_clause(TransferClause::LoanOptionToBuy(money(5_000_000.0)));

        let future_fee = offer.loan_future_fee();

        assert_eq!(
            future_fee.map(|(fee, obligation)| (fee.amount, obligation)),
            Some((5_000_000.0, false))
        );
    }

    #[test]
    fn extracts_loan_obligation_to_buy_as_mandatory_future_fee() {
        let offer = TransferOffer::default()
            .with_clause(TransferClause::LoanObligationToBuy(money(7_000_000.0)));

        let future_fee = offer.loan_future_fee();

        assert_eq!(
            future_fee.map(|(fee, obligation)| (fee.amount, obligation)),
            Some((7_000_000.0, true))
        );
    }

    #[test]
    fn personal_terms_override_legacy_contract_length() {
        // Setting both: the personal-terms field wins. This matches the
        // realism rule that the negotiated wage/length package is the
        // authoritative source, not a fallback default.
        let mut terms = PersonalTermsOffer::default();
        terms.contract_years = Some(5);
        terms.annual_wage = Some(120_000);
        let offer = TransferOffer::default()
            .with_contract_length(2)
            .with_personal_terms(terms);
        assert_eq!(offer.resolved_contract_years(), Some(5));
        assert_eq!(offer.resolved_annual_wage(), Some(120_000));
    }

    #[test]
    fn loan_duration_is_independent_of_permanent_contract_years() {
        // The legacy bug: `contract_length = Some(1)` meant "1 year"
        // for permanent deals but was interpreted as "1 month" for
        // loans. The new split fields make this unambiguous: setting
        // years to 4 leaves loan duration None.
        let offer = TransferOffer::default()
            .with_contract_length(4)
            .with_loan_duration_months(6);
        assert_eq!(offer.contract_length_years, Some(4));
        assert_eq!(offer.loan_duration_months, Some(6));
    }
}

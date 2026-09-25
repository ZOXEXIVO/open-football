//! What the board is willing to do with money.
//!
//! Three questions, one actor. What will we pay for this purpose
//! ([`ClubBoard::fee_envelope`])? Will we sign this cheque
//! ([`ClubBoard::hear`], through the transfer hearing)? And what will we
//! take back for the same man later ([`ClubBoard::reservation_for`])?
//!
//! Every term is a function of what the board already holds — its vision's
//! financial stance, the chairman's temperament, the ownership model's risk
//! appetite and exit pressure, its own regulatory heat, and the mandates it
//! has already closed. There are no club archetypes here and no per-reason
//! branches: the purpose enters only through the minutes it promised.

use crate::club::CareerRunway;
use crate::club::board::ClubBoard;
use crate::club::board::chairman::ChairmanAmbition;
use crate::club::board::mandate::{MandateAuthor, MandateOutcome, SigningMandate};
use crate::club::board::vision::FinancialStance;
use crate::transfers::pipeline::TransferNeedPriority;
use crate::transfers::squad::ledger::{AssetRow, LedgerPressure};
use crate::transfers::squad::plan::MoneySlack;
use crate::transfers::value::upgrade::{DealInputs, UpgradeMath};

pub use crate::transfers::value::upgrade::TargetBelief;

/// The two numbers a board hands its recruitment desk: where to open, and
/// where to stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FeeEnvelope {
    /// The most the board will put on the table first.
    pub open: f64,
    /// The fee at which the deal stops being worth doing to this club, for
    /// this purpose. The buyer's own number, and the end of the auction.
    pub walk_away: f64,
}

impl FeeEnvelope {
    /// Nothing is worth paying — the deal has no value at any fee.
    pub const NONE: FeeEnvelope = FeeEnvelope {
        open: 0.0,
        walk_away: 0.0,
    };

    /// The fee the board will not go past, after the rope its temperament
    /// and its recent record earn the deal.
    pub fn ceiling(&self, stretch: f64) -> f64 {
        self.walk_away * stretch.max(0.0)
    }
}

/// How far past its own valuation a board will go before it says no.
///
/// Not a tolerance stack: one multiple over the walk-away, built from the
/// owner's appetite for risk, the money he puts in himself, the chairman's
/// temperament, how badly the shirt is needed, and how much of its recent
/// spending the club has already written off.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MandateStretch(f64);

impl MandateStretch {
    /// Most a board ever pays over its own number, whatever the terms add
    /// up to — past this it is not stretching, it is not valuing.
    pub const MAX: f64 = 2.0;
    pub const MIN: f64 = 1.0;

    pub fn value(self) -> f64 {
        self.0
    }
}

/// What the board will take for a man it owns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reservation {
    /// The number it advertises.
    pub ask: f64,
    /// The number below which it would rather keep him — what is left of
    /// the fee on its own books, less what it is willing to write off.
    pub floor: f64,
    /// Share of the book the club is prepared to book as a loss today.
    pub write_off: f32,
    pub verdict: ReservationVerdict,
}

/// What a club does with a man the market will not pay his book for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservationVerdict {
    /// The market clears the book, or the board will write the rest off.
    Sell,
    /// Somebody else pays him to play while the book runs down. The
    /// natural home of an unrealised mandate.
    Loan,
    /// Neither — the club keeps him and waits.
    Hold,
}

/// What a club believes about a target, and how sure it is.
///
/// Two of these fields are not about the player at all. `confidence` is how
/// well the club knows him, and `league_gap` is how far the league it is
/// watching sits below its own — together they say how wrong the headline
/// number could be, which is the difference between a board that pays for a
/// scout's report and one that discounts it.
pub struct BeliefRisk;

impl BeliefRisk {
    /// Spread on a target nobody has watched, in ability points.
    const SIGMA_BASE: f32 = 10.0;
    /// Extra spread a full league of distance adds — the step-up question
    /// no amount of watching answers.
    const SIGMA_LEAGUE_STEP: f32 = 8.0;

    /// How wrong the club could be about him.
    pub fn sigma(confidence: f32, league_gap: f32) -> f32 {
        Self::SIGMA_BASE * (1.0 - confidence.clamp(0.0, 1.0))
            + Self::SIGMA_LEAGUE_STEP * league_gap.clamp(0.0, 1.0)
    }
}

impl ClubBoard {
    /// What a wasted fee costs the man who asked for it, on the two facets
    /// a recruitment decision is actually about.
    const WASTED_RESULTS: i32 = -6;
    const WASTED_COMMUNICATION: i32 = -4;

    /// A mandate has closed. The board writes down what its money bought,
    /// and the man who asked for it answers for the ones that bought
    /// nothing.
    ///
    /// The politics is the point: a fee that delivered no football costs
    /// the manager standing on results and on the room, so the director of
    /// football's override threshold bites sooner. A board trusts its
    /// recruitment desk more than its coach after a wasted window, and
    /// nothing in the model could say so.
    pub fn on_mandate_closed(&mut self, outcome: MandateOutcome) {
        if outcome.is_unrealised() && matches!(outcome.author, MandateAuthor::Manager) {
            self.relationship.adjust_results(Self::WASTED_RESULTS);
            self.relationship
                .adjust_communication(Self::WASTED_COMMUNICATION);
        }
        self.mandate_ledger.push(outcome);
    }

    /// Whose mandate this is.
    ///
    /// The manager's, unless the board has lost enough faith in him for
    /// the recruitment desk to be the one signing off — the same threshold
    /// that governs every other override of his judgement.
    pub fn mandate_author(&self) -> MandateAuthor {
        if self.confidence.level < self.vision.manager_autonomy.dof_override_threshold() {
            MandateAuthor::DirectorOfFootball
        } else {
            MandateAuthor::Manager
        }
    }

    /// Points of believed ability a board discounts per point of spread.
    ///
    /// A conservative board reads a scout's headline number as the top of a
    /// range; a reckless one reads it as the number. Everything between is
    /// the owner's appetite for risk and the vision's stance on money.
    pub fn risk_aversion(&self) -> f64 {
        let stance = match self.vision.financial_stance {
            FinancialStance::Austerity => 0.30,
            FinancialStance::Conservative => 0.15,
            FinancialStance::Balanced => 0.0,
            FinancialStance::Ambitious => -0.15,
        };
        let chairman = match self.chairman.ambition {
            ChairmanAmbition::Reckless => -0.20,
            _ => 0.0,
        };
        (0.9 - 0.8 * self.ownership.risk_tolerance as f64 / 100.0 + stance + chairman).max(0.0)
    }

    /// What the club thinks it is buying, once it has discounted what it
    /// does not know.
    pub fn believed_level_eff(&self, belief: &TargetBelief) -> f32 {
        let sigma = BeliefRisk::sigma(belief.confidence, belief.league_gap);
        (belief.believed_level - self.risk_aversion() as f32 * sigma).max(0.0)
    }

    /// Price a purpose.
    ///
    /// The sporting benefit is the minutes the mandate promised, times the
    /// points he brings over the man whose minutes those are, times what a
    /// point is worth at this club — solved for the fee at which the deal
    /// breaks even. A cover signing and a starter signing at the same
    /// ability differ here by exactly one thing: how much of the season
    /// the club said he would play.
    /// Price a purpose.
    ///
    /// `allocation` is what the club's own brief set aside for this shirt.
    /// It is the floor of the walk-away, and it is what makes a bench
    /// purpose priceable at all: a body for the bench improves nobody, so
    /// the sporting arithmetic answers zero — and a board that has set
    /// aside money for a body spends that and not a penny more. Before
    /// this the same answer read as "no number", and the seller's ask was
    /// the only figure in the room.
    pub fn fee_envelope(
        &self,
        mandate: &SigningMandate,
        belief: &TargetBelief,
        money: &MoneySlack,
        allocation: f64,
        days_left_frac: f32,
    ) -> FeeEnvelope {
        let deal = UpgradeMath::evaluate(&DealInputs {
            group: belief.group,
            tier: belief.tier,
            believed_level: self.believed_level_eff(belief),
            incumbent_level: belief.incumbent_level,
            replacement_level: belief.replacement_level,
            believed_ceiling: belief.believed_ceiling,
            league_gap: belief.league_gap,
            minutes: mandate.minutes,
            age: belief.age,
            annual_wage: belief.annual_wage,
            annual_income: money.annual_income,
            idle_cash: money.idle_cash,
        });
        let walk_away = deal.ceiling_fee.max(allocation.max(0.0));
        FeeEnvelope {
            open: walk_away * UpgradeMath::open_ratio(belief.tier, days_left_frac),
            walk_away,
        }
    }

    /// How far past its own number this board will go for this signing.
    pub fn stretch(&self, priority: &TransferNeedPriority) -> MandateStretch {
        let chairman = match self.chairman.ambition {
            ChairmanAmbition::Reckless => 0.25,
            ChairmanAmbition::Ambitious => 0.10,
            ChairmanAmbition::Balanced => 0.0,
            ChairmanAmbition::Conservative => -0.10,
        };
        let urgency = match priority {
            TransferNeedPriority::Critical => 0.35,
            TransferNeedPriority::Important => 0.15,
            TransferNeedPriority::Optional => 0.0,
        };
        let value = 1.0
            + 0.25 * self.ownership.risk_tolerance as f64 / 100.0
            + 0.25 * self.ownership.benefactor.clamp(0.0, 1.0) as f64
            + chairman
            + urgency
            - self.mandate_ledger.discipline();
        MandateStretch(value.clamp(MandateStretch::MIN, MandateStretch::MAX))
    }

    /// What the club would take for a man it owns, and what it does when
    /// the market will not pay it.
    ///
    /// The floor is the club's own money, not the player's mood: what is
    /// left of the fee on the books, less the share this board is prepared
    /// to write off today. A transfer request, a `NotNeeded` label or a
    /// durable grievance still ease the MARKET fraction everywhere else —
    /// they have never been a reason to forget what a man cost.
    /// Share of what a man is still on the books for that this board is
    /// prepared to book as a loss today.
    ///
    /// Everything in it is money the board is already answering for: the
    /// stance it runs the club on, how short of cash it is, how hard the
    /// wage bill is pressing, its regulatory standing, an owner who wants
    /// out — and, pulling the other way, the losses it has already booked,
    /// because a board that has written off a third of its recent spending
    /// is less willing to write off any more.
    pub fn write_off_share(&self, pressure: &LedgerPressure) -> f64 {
        let austerity = match self.vision.financial_stance {
            FinancialStance::Austerity => 1.0,
            FinancialStance::Conservative => 0.4,
            _ => 0.0,
        };
        (0.15 * austerity
            + 0.35 * pressure.cash_need
            + 0.25 * pressure.wage_pressure
            + 0.25 * self.regulatory_strain()
            + 0.10 * (self.ownership.exit_pressure as f64 / 100.0)
            + 0.10 * (self.ownership.risk_tolerance as f64 - 50.0) / 50.0
            - 0.10 * self.mandate_ledger.burn())
        .clamp(0.0, 1.0)
    }

    /// The price the club's own books put under a sale.
    pub fn book_floor(&self, book: f64, pressure: &LedgerPressure) -> f64 {
        book.max(0.0) * (1.0 - self.write_off_share(pressure))
    }

    pub fn reservation_for(
        &self,
        row: &AssetRow,
        market_ask: f64,
        book: f64,
        pressure: &LedgerPressure,
    ) -> Reservation {
        let write_off = self.write_off_share(pressure);
        let floor = self.book_floor(book, pressure);
        Reservation {
            ask: market_ask.max(floor),
            floor,
            write_off: write_off as f32,
            verdict: self.exit_route(
                book,
                market_ask,
                Self::has_loan_runway(row.contract_months_remaining, row.age, row.loans_used),
                pressure,
            ),
        }
    }

    /// What the club does with a man it is finished with.
    ///
    /// A fee that clears what he is still on the books for is a sale. One
    /// that does not is a loss the board has to choose to book — and while
    /// he has a season elsewhere left in him it would rather somebody else
    /// paid him to play than write it off today. That is what makes the
    /// loan market the natural home of an unrealised mandate.
    pub fn exit_route(
        &self,
        book: f64,
        market_ask: f64,
        has_loan_runway: bool,
        pressure: &LedgerPressure,
    ) -> ReservationVerdict {
        if market_ask >= self.book_floor(book, pressure) {
            ReservationVerdict::Sell
        } else if has_loan_runway {
            ReservationVerdict::Loan
        } else {
            ReservationVerdict::Hold
        }
    }

    /// Regulatory heat as a 0..1 share — the board's own gauge, which the
    /// FFP standing is what writes.
    fn regulatory_strain(&self) -> f64 {
        (self.pressure.regulatory_pressure as f64 / 70.0).clamp(0.0, 1.0)
    }

    /// Is there a season elsewhere left in him? A man with contract, career
    /// and spells still in hand is lent out rather than written off.
    pub fn has_loan_runway(
        contract_months_remaining: Option<i32>,
        age: u8,
        loans_used: u8,
    ) -> bool {
        const MIN_CONTRACT_MONTHS: i32 = 12;
        contract_months_remaining.is_some_and(|m| m >= MIN_CONTRACT_MONTHS)
            && CareerRunway::at(age) >= 0.25
            && loans_used < AssetRow::MAX_LOANS
    }
}

#[cfg(test)]
mod doctrine_tests {
    use super::*;
    use crate::club::board::mandate::{MandateAuthor, MandatePurpose};
    use crate::club::board::vision::FinancialStance;
    use crate::club::team::squad::SquadAssetClass;
    use crate::transfers::squad::plan::BriefTier;
    use crate::{PathwayStage, PlayerFieldPositionGroup};
    use chrono::NaiveDate;

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2028, 7, 8).unwrap()
        }

        fn money() -> MoneySlack {
            MoneySlack {
                ratio: 0.5,
                idle_cash: 100_000_000.0,
                annual_income: 400_000_000.0,
            }
        }

        /// A Championship keeper an elite club is looking at: better than
        /// the man it has, but not by much, and watched from a league two
        /// levels down.
        fn belief() -> TargetBelief {
            TargetBelief {
                group: PlayerFieldPositionGroup::Goalkeeper,
                tier: BriefTier::B,
                believed_level: 175.0,
                incumbent_level: 145.0,
                replacement_level: 105.0,
                believed_ceiling: 182.0,
                league_gap: 0.6,
                confidence: 0.5,
                age: 24,
                annual_wage: 4_000_000.0,
            }
        }

        fn mandate(purpose: MandatePurpose) -> SigningMandate {
            SigningMandate::new(
                purpose,
                PlayerFieldPositionGroup::Goalkeeper,
                24,
                Self::date(),
                MandateAuthor::Board,
            )
        }

        fn heir() -> SigningMandate {
            Self::mandate(MandatePurpose::Heir {
                incumbent_id: 7,
                handover: NaiveDate::from_ymd_opt(2032, 7, 8).unwrap(),
            })
        }

        fn row(months: i32, age: u8) -> AssetRow {
            AssetRow {
                player_id: 1,
                group: PlayerFieldPositionGroup::Goalkeeper,
                age,
                contract_months_remaining: Some(months),
                estimated_value: 13_000_000.0,
                annual_wage: 4_800_000.0,
                asset_class: SquadAssetClass::TrueSurplus,
                observable_level: 130,
                squad_average_level: 145,
                believed_ceiling: 150,
                group_rank: 2,
                is_transfer_requested: true,
                stage_pull: 0.5,
                renewal_blocked: false,
                signing_protected: false,
                stage: PathwayStage::Rotation,
                verdict_multiple: None,
                unsellable: false,
                loans_used: 0,
                book_value: 31_000_000.0,
            }
        }
    }

    #[test]
    fn an_heir_is_worth_a_fraction_of_the_same_man_as_a_starter() {
        // The Vítek shape. Same club, same player, same wage — the only
        // difference is what the board said he was for.
        let board = ClubBoard::new();
        let heir = board.fee_envelope(&Fx::heir(), &Fx::belief(), &Fx::money(), 0.0, 0.5);
        let starter = board.fee_envelope(
            &Fx::mandate(MandatePurpose::Starter),
            &Fx::belief(),
            &Fx::money(),
            0.0,
            0.5,
        );
        assert!(starter.walk_away > 0.0);
        assert!(
            heir.walk_away <= starter.walk_away * 0.5,
            "heir {} vs starter {}",
            heir.walk_away,
            starter.walk_away
        );
    }

    #[test]
    fn a_non_upgrade_still_has_a_price() {
        // The whole reason the buyer had no number of its own: a cover, an
        // heir and a prospect are bought precisely because they are not
        // better than the man in the shirt, and the old model answered
        // "then price it at nothing" — which left the seller's ask as the
        // only number in the room.
        let board = ClubBoard::new();
        // Not one point better than the man in the shirt, and the club
        // still put four million aside for the shirt behind him.
        let mut cover = Fx::belief();
        cover.believed_level = 120.0;
        let envelope = board.fee_envelope(
            &Fx::mandate(MandatePurpose::Cover),
            &cover,
            &Fx::money(),
            4_000_000.0,
            0.5,
        );
        assert!(envelope.walk_away >= 4_000_000.0, "{}", envelope.walk_away);
        assert!(envelope.open < envelope.walk_away);
    }

    #[test]
    fn a_cautious_board_discounts_an_unproven_step_up_further() {
        let mut bold = ClubBoard::new();
        bold.chairman.ambition = ChairmanAmbition::Reckless;
        bold.ownership.risk_tolerance = 90;
        let mut cautious = ClubBoard::new();
        cautious.vision.financial_stance = FinancialStance::Austerity;
        cautious.ownership.risk_tolerance = 15;

        let belief = Fx::belief();
        assert!(bold.believed_level_eff(&belief) > cautious.believed_level_eff(&belief));
        let bold_fee = bold.fee_envelope(&Fx::heir(), &belief, &Fx::money(), 0.0, 0.5);
        let cautious_fee = cautious.fee_envelope(&Fx::heir(), &belief, &Fx::money(), 0.0, 0.5);
        assert!(bold_fee.walk_away > cautious_fee.walk_away);
    }

    #[test]
    fn a_solvent_board_keeps_most_of_the_book_under_a_pushing_player() {
        // A transfer request is the player's business. It has never been a
        // reason for the club to forget what he cost.
        let board = ClubBoard::new();
        let quiet = LedgerPressure {
            cash_need: 0.0,
            wage_pressure: 0.0,
        };
        let reservation =
            board.reservation_for(&Fx::row(36, 26), 8_100_000.0, 31_000_000.0, &quiet);
        assert!(
            reservation.floor >= 31_000_000.0 * 0.6,
            "{}",
            reservation.floor
        );
        assert_eq!(reservation.verdict, ReservationVerdict::Loan);
        assert!(reservation.ask >= reservation.floor);
    }

    #[test]
    fn a_club_that_needs_the_money_books_the_loss() {
        let mut board = ClubBoard::new();
        board.vision.financial_stance = FinancialStance::Austerity;
        board.pressure.regulatory_pressure = 70;
        let squeezed = LedgerPressure {
            cash_need: 1.0,
            wage_pressure: 1.0,
        };
        let reservation =
            board.reservation_for(&Fx::row(36, 26), 8_100_000.0, 31_000_000.0, &squeezed);
        assert!(reservation.write_off > 0.6, "{}", reservation.write_off);
        assert_eq!(reservation.verdict, ReservationVerdict::Sell);
    }

    #[test]
    fn a_man_with_no_contract_runway_is_held_rather_than_lent() {
        let board = ClubBoard::new();
        let quiet = LedgerPressure {
            cash_need: 0.0,
            wage_pressure: 0.0,
        };
        let reservation = board.reservation_for(&Fx::row(6, 34), 1_000_000.0, 20_000_000.0, &quiet);
        assert_eq!(reservation.verdict, ReservationVerdict::Hold);
    }

    #[test]
    fn a_wasted_fee_lands_on_the_man_who_asked_for_it() {
        use crate::club::board::mandate::{MandateExit, MandateOutcome};

        let wasted = SigningMandate::new(
            MandatePurpose::Heir {
                incumbent_id: 7,
                handover: NaiveDate::from_ymd_opt(2032, 7, 8).unwrap(),
            },
            PlayerFieldPositionGroup::Goalkeeper,
            24,
            Fx::date(),
            MandateAuthor::Manager,
        )
        .with_money(41_600_000.0, 4_800_000.0);

        let mut board = ClubBoard::new();
        let before = board.relationship.clone();
        board.on_mandate_closed(MandateOutcome::close(
            1,
            &wasted,
            0.0,
            MandateExit::Sold(8_100_000.0),
            NaiveDate::from_ymd_opt(2030, 7, 5).unwrap(),
        ));
        assert!(
            board.relationship.trust_results < before.trust_results,
            "a fee that delivered no football is the coach's to answer for"
        );
        assert!(board.mandate_ledger.burn() > 0.0);

        // The same fee, asked for by the recruitment desk: the board wrote
        // it, and the manager does not carry it.
        let desk = SigningMandate {
            author: MandateAuthor::DirectorOfFootball,
            ..wasted
        };
        let mut board = ClubBoard::new();
        let before = board.relationship.clone();
        board.on_mandate_closed(MandateOutcome::close(
            1,
            &desk,
            0.0,
            MandateExit::Sold(8_100_000.0),
            NaiveDate::from_ymd_opt(2030, 7, 5).unwrap(),
        ));
        assert_eq!(board.relationship.trust_results, before.trust_results);
    }

    #[test]
    fn the_stretch_stays_inside_its_band() {
        let mut board = ClubBoard::new();
        board.ownership.risk_tolerance = 100;
        board.ownership.benefactor = 1.0;
        board.chairman.ambition = ChairmanAmbition::Reckless;
        let wide = board.stretch(&TransferNeedPriority::Critical);
        assert!(wide.value() <= MandateStretch::MAX);

        let mut tight = ClubBoard::new();
        tight.ownership.risk_tolerance = 0;
        tight.chairman.ambition = ChairmanAmbition::Conservative;
        assert!(tight.stretch(&TransferNeedPriority::Optional).value() >= MandateStretch::MIN);
    }
}

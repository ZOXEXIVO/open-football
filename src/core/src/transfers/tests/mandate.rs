//! The Vítek shape, both halves of it.
//!
//! A Championship goalkeeper bought by an elite club for a starter's fee
//! on a bench purpose, then written off at a fifth of it two years later.
//! Neither half was a bug in one number: nothing in the model owned the
//! decision "what did we buy him for, what is he worth to us, and what
//! will we take for him". Both tests fail on HEAD — the first because the
//! buyer had no number of its own for a non-upgrade, the second because
//! the seller's floor was zeroed by the player's own transfer request and
//! absent entirely on a cross-border sale.

use chrono::NaiveDate;

use crate::club::board::ClubBoard;
use crate::club::board::mandate::{
    MandateAuthor, MandatePurpose, MinutesCurve, ReservationVerdict, SigningMandate, TargetBelief,
};
use crate::club::board::vision::FinancialStance;
use crate::club::team::squad::SquadAssetClass;
use crate::transfers::pipeline::TransferNeedPriority;
use crate::transfers::squad::ledger::{AssetRow, LedgerPressure};
use crate::transfers::squad::plan::{BriefTier, MoneySlack};
use crate::transfers::tests::kit::{TestClub, TestDate, TestPlayer};
use crate::{PathwayStage, PlayerFieldPositionGroup, PlayerPlan, PlayerPositionType};

/// Milan and Middlesbrough, in the shape the doctrine has to price.
struct VitekFixtures;

impl VitekFixtures {
    /// The fee the move actually completed at.
    const FEE_IN: f64 = 41_600_000.0;
    /// …and what he was sold on for two years later.
    const FEE_OUT: f64 = 8_100_000.0;

    const SIGNED: (i32, u32, u32) = (2028, 7, 8);
    const SOLD: (i32, u32, u32) = (2030, 7, 5);

    fn signed() -> NaiveDate {
        TestDate::on(Self::SIGNED.0, Self::SIGNED.1, Self::SIGNED.2)
    }

    fn sold() -> NaiveDate {
        TestDate::on(Self::SOLD.0, Self::SOLD.1, Self::SOLD.2)
    }

    /// An elite club with a year's income in the bank — the buyer whose
    /// marginal dollar is cheap and whose budget therefore bounded
    /// nothing.
    fn buyer() -> MoneySlack {
        MoneySlack {
            ratio: 0.6,
            idle_cash: 300_000_000.0,
            annual_income: 400_000_000.0,
        }
    }

    /// What the buyer's scouts believe: a Championship keeper of 130,
    /// behind a 145 incumbent, watched from six thousand points of league
    /// reputation below.
    fn belief() -> TargetBelief {
        TargetBelief {
            group: PlayerFieldPositionGroup::Goalkeeper,
            tier: BriefTier::B,
            believed_level: 130.0,
            incumbent_level: 145.0,
            replacement_level: 108.0,
            believed_ceiling: 150.0,
            league_gap: 0.6,
            confidence: 0.5,
            age: 24,
            annual_wage: 4_800_000.0,
        }
    }

    fn mandate(purpose: MandatePurpose) -> SigningMandate {
        SigningMandate::new(
            purpose,
            PlayerFieldPositionGroup::Goalkeeper,
            24,
            Self::signed(),
            MandateAuthor::Manager,
        )
    }

    /// The succession purpose the club actually raised: an heir behind a
    /// 33-year-old whose first-choice career has four seasons left.
    fn heir() -> SigningMandate {
        Self::mandate(MandatePurpose::Heir {
            incumbent_id: 7,
            handover: TestDate::on(2032, 7, 8),
        })
    }

    /// The allocation a `Watch`-urgency succession search carries: a
    /// fraction of the discretionary unit, not the whole pot.
    const SUCCESSION_ALLOCATION: f64 = 6_000_000.0;
}

/// Half one: the purchase.
///
/// The board has to have a number of its own, and that number has to be
/// about the PURPOSE. Bought to sit behind the man in the shirt for four
/// seasons, he is worth a fraction of what he would be worth bought to
/// take it — and the escalation may not leave the room the doctrine gives
/// it, whatever the seller is asking.
#[test]
fn an_heir_is_priced_as_an_heir_and_not_as_a_starter() {
    let board = ClubBoard::new();
    let belief = VitekFixtures::belief();
    let money = VitekFixtures::buyer();

    let heir = board.fee_envelope(
        &VitekFixtures::heir(),
        &belief,
        &money,
        VitekFixtures::SUCCESSION_ALLOCATION,
        0.5,
    );
    // The same club, the same player, the same wage — bought to take the
    // shirt rather than to wait for it.
    let starter = board.fee_envelope(
        &VitekFixtures::mandate(MandatePurpose::Starter),
        &TargetBelief {
            // A man bought as the starter is one the club believes will
            // improve on its starter; the belief is otherwise identical.
            believed_level: 160.0,
            ..belief
        },
        &money,
        VitekFixtures::SUCCESSION_ALLOCATION,
        0.5,
    );

    assert!(
        heir.walk_away <= starter.walk_away * 0.45,
        "an heir must not be priced like a starter: {} vs {}",
        heir.walk_away,
        starter.walk_away
    );

    // And the whole escalation — the board's number plus every scrap of
    // rope its temperament earns the deal — stays an order of magnitude
    // under what the move actually completed at.
    let ceiling = heir.ceiling(board.stretch(&TransferNeedPriority::Optional).value());
    assert!(
        ceiling < VitekFixtures::FEE_IN * 0.5,
        "the escalation may not leave the doctrine: {ceiling} against {}",
        VitekFixtures::FEE_IN
    );
}

/// Half two: the sale.
///
/// Two years on he has played three times. The club's own books still
/// carry half of what it paid, and it is solvent — so the number under
/// the sale is that book, and the answer to a market that will not pay it
/// is a loan, not a write-off. His transfer request is his business and
/// changes neither.
#[test]
fn a_solvent_club_loans_an_unrealised_mandate_rather_than_writing_it_off() {
    let signed = VitekFixtures::signed();
    let sold = VitekFixtures::sold();
    let mandate = VitekFixtures::heir().with_money(VitekFixtures::FEE_IN, 4_800_000.0);
    let book = mandate.book_value(sold);

    let board = ClubBoard::new();
    let solvent = LedgerPressure {
        cash_need: 0.0,
        wage_pressure: 0.0,
    };
    let row = AssetRow {
        player_id: 1,
        group: PlayerFieldPositionGroup::Goalkeeper,
        age: 26,
        contract_months_remaining: Some(36),
        estimated_value: 13_000_000.0,
        annual_wage: 4_800_000.0,
        asset_class: SquadAssetClass::TrueSurplus,
        observable_level: 130,
        squad_average_level: 150,
        believed_ceiling: 150,
        group_rank: 2,
        // He has asked to go. That is his business.
        is_transfer_requested: true,
        stage_pull: 0.6,
        renewal_blocked: false,
        signing_protected: false,
        stage: PathwayStage::Rotation,
        verdict_multiple: None,
        unsellable: false,
        loans_used: 0,
        book_value: book,
    };

    let reservation = board.reservation_for(&row, VitekFixtures::FEE_OUT, book, &solvent);
    assert!(
        reservation.floor >= VitekFixtures::FEE_IN * 0.6 * 0.5,
        "727 days into a four-year amortisation the book is still half the \
         fee, and a solvent board writes off almost none of it: {}",
        reservation.floor
    );
    assert!(
        reservation.floor > VitekFixtures::FEE_OUT,
        "the sale price has to be refused: floor {} vs fee {}",
        reservation.floor,
        VitekFixtures::FEE_OUT
    );
    assert_eq!(
        reservation.verdict,
        ReservationVerdict::Loan,
        "a man with three years of contract and a career left is lent out"
    );

    // …and a club that genuinely needs the money still gets to sell him.
    // The doctrine is a decision, not a lock.
    let mut distressed = ClubBoard::new();
    distressed.vision.financial_stance = FinancialStance::Austerity;
    distressed.pressure.regulatory_pressure = 70;
    let squeezed = LedgerPressure {
        cash_need: 1.0,
        wage_pressure: 1.0,
    };
    assert_eq!(
        distressed
            .reservation_for(&row, VitekFixtures::FEE_OUT, book, &squeezed)
            .verdict,
        ReservationVerdict::Sell
    );

    // The plan he is on says the same thing from the other side: two
    // seasons into a five-season promise, having played nothing, he is
    // behind schedule rather than evaluated.
    let plan = PlayerPlan::from_mandate(mandate, signed);
    assert!(!plan.on_schedule(sold, 0.03, MinutesCurve::MAX_HORIZON * 4));
    assert!(
        !plan.is_expired(sold),
        "the mandate still has seasons to run"
    );
}

/// The buyer that could not stop: on a cross-border deal the whole
/// transfer budget was the only cap, and the seller's floor did not exist
/// at all. Both halves are now the clubs' own numbers.
#[test]
fn a_cross_border_seller_still_remembers_what_it_paid() {
    let signed = VitekFixtures::signed();
    let sold = VitekFixtures::sold();
    let keeper = TestPlayer::new(1)
        .ability(130)
        .age(26)
        .position(PlayerPositionType::Goalkeeper)
        .on(signed)
        .build();
    let mut club = TestClub::new(10)
        .reputation(8_500)
        .players(vec![keeper])
        .build();

    let mandate = VitekFixtures::heir().with_money(VitekFixtures::FEE_IN, 4_800_000.0);
    if let Some(player) = club.teams.teams[0].players.find_mut(1) {
        player.plan = Some(PlayerPlan::from_mandate(mandate, signed));
    }

    let book = club.teams.teams[0]
        .players
        .find(1)
        .map(|p| p.book_value(sold))
        .unwrap_or(0.0);
    assert!(book > VitekFixtures::FEE_OUT, "{book}");

    let floor = club.board.book_floor(
        book,
        &LedgerPressure {
            cash_need: 0.0,
            wage_pressure: 0.0,
        },
    );
    assert!(
        floor > VitekFixtures::FEE_OUT,
        "the floor a foreign buyer has to clear: {floor}"
    );
}

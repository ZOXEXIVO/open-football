//! What a board does with a man it listed and could not sell.
//!
//! A window shuts with him still on the list. Nothing forces him out: the
//! board looks at what the market has told it, what he costs to keep, and
//! what he would take to go, and picks whichever exit costs the club
//! least — keep selling at a lower price, lend him out while paying part
//! of his wage, or pay him to leave. The longer the market has refused
//! him and the heavier his wage, the further down that list it is willing
//! to go. His age, his contract type and why he was listed never enter.

use crate::club::board::ClubBoard;
use crate::club::player::transfer::SettlementAsk;
use crate::transfers::market::TransferListing;
use crate::transfers::squad::ledger::LedgerPressure;
use log::debug;

/// Everything the board is shown about one listing that got through a
/// window unsold.
#[derive(Debug, Clone, Copy)]
pub struct StrandedCase {
    /// Open-window days the market has had to look at him.
    pub exposure_days: u16,
    pub annual_wage: f64,
    /// What the club's wage mandate allows the average man — the ruler a
    /// "heavy" wage is read against.
    pub average_wage: f64,
    pub annual_wage_bill: u32,
    pub contract_days_left: i64,
    pub current_ask: f64,
    pub best_rejected_bid: Option<f64>,
    /// What is left of his fee on the club's books.
    pub book_value: f64,
    pub has_loan_runway: bool,
    /// His own answer to being asked to leave.
    pub settlement: SettlementAsk,
}

impl StrandedCase {
    /// The wages he is owed until his deal runs out.
    fn carry(&self) -> f64 {
        self.annual_wage * self.contract_days_left.max(0) as f64 / 365.0
    }

    /// How hard he is pushing to go, 0..1: the share of what he is owed he
    /// would give up to be let go. A man who would walk for nothing is at
    /// the manager's door every week; one who wants every penny is not.
    fn push(&self) -> f32 {
        if self.settlement.carry <= 0.0 {
            return 0.0;
        }
        (1.0 - self.settlement.ask / self.settlement.carry).clamp(0.0, 1.0) as f32
    }
}

/// The board's appetite to end the stalemate, 0..1. Zero before the market
/// has seen him; rising with every open-window day he goes unbought, and
/// faster the more his wage weighs on the club and the harder he pushes to
/// leave.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct StrandedResolve(f32);

impl StrandedResolve {
    /// Open-window days that make one unit of exposure: roughly a summer
    /// and a winter window of market.
    const EXPOSURE_SCALE: f32 = 110.0;
    /// How hard a board presses a man on an ordinary wage at an ordinary
    /// club, per unit of exposure …
    const BASE: f32 = 0.35;
    /// … and what a heavy wage, a wage bill over its mandate, a short cash
    /// position and a player pushing to go add to it.
    const W_WAGE: f32 = 0.5;
    const W_PRESSURE: f32 = 0.6;
    const W_CASH: f32 = 0.4;
    const W_PUSH: f32 = 1.0;

    pub fn read(case: &StrandedCase, pressure: &LedgerPressure) -> Self {
        let exposure = case.exposure_days as f32 / Self::EXPOSURE_SCALE;
        let wage_ratio = if case.average_wage > 0.0 {
            (case.annual_wage / case.average_wage) as f32
        } else {
            1.0
        };
        let weight = Self::BASE
            + Self::W_WAGE * wage_ratio.max(0.0).ln_1p()
            + Self::W_PRESSURE * pressure.wage_pressure as f32
            + Self::W_CASH * pressure.cash_need as f32
            + Self::W_PUSH * case.push();
        StrandedResolve((1.0 - (-exposure * weight).exp()).clamp(0.0, 1.0))
    }

    pub fn value(self) -> f32 {
        self.0
    }
}

/// Which door the board holds open for him.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StrandedRoute {
    /// Stay on the list at the new price.
    KeepSelling,
    /// Somebody else plays him, and the club keeps paying `subsidy` of his
    /// wage to make that happen.
    LoanOut { subsidy: f32 },
    /// Tear the contract up for `amount`.
    Settle { amount: f64 },
}

/// The board's answer at one review.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrandedVerdict {
    pub resolve: StrandedResolve,
    /// The asking price the listing restarts from.
    pub anchor: f64,
    /// The least the board will now take — no bid at or above it is
    /// refused for price.
    pub board_floor: f64,
    pub route: StrandedRoute,
}

impl ClubBoard {
    /// A club won't absorb a one-off above this share of a year's wage
    /// bill however keen it is to see a man gone …
    const LUMP_SUM_WAGE_BILL_DIVISOR: u32 = 12;
    /// … and can always find this much.
    const LUMP_SUM_FLOOR: u32 = 10_000;

    /// The largest pay-off the club can hand over in one go without it
    /// being a cash-flow shock.
    pub fn lump_sum_cap(annual_wage_bill: u32) -> u32 {
        (annual_wage_bill / Self::LUMP_SUM_WAGE_BILL_DIVISOR).max(Self::LUMP_SUM_FLOOR)
    }

    /// Review a listing that went through a window unsold.
    pub fn review_stranded(
        &self,
        case: &StrandedCase,
        pressure: &LedgerPressure,
    ) -> StrandedVerdict {
        let resolve = StrandedResolve::read(case, pressure);
        let r = resolve.value() as f64;
        let write_off = self.write_off_share(pressure).max(r);
        let unwritten_book = case.book_value.max(0.0) * (1.0 - write_off);

        // The price the market has named: the best bid refused, or — after a
        // window in which nobody bid at all — nothing at the price asked.
        let target = case.best_rejected_bid.unwrap_or(0.0);
        let anchor = (case.current_ask - (case.current_ask - target).max(0.0) * r)
            .max(unwritten_book)
            .min(case.current_ask);
        let board_floor = (anchor * TransferListing::DECAY_FLOOR)
            .max(unwritten_book)
            .min(anchor);

        let carry = case.carry();
        // What keeping him on the market is still worth: the sale the board
        // believes in brings a fee AND takes the rest of his wages off the
        // books, so a pay-off only wins once that belief has worn down.
        let hold_value =
            (1.0 - r) * (case.best_rejected_bid.unwrap_or(0.0).max(case.current_ask) + carry);
        let worth_paying = carry - unwritten_book - hold_value;
        let lump_cap = Self::lump_sum_cap(case.annual_wage_bill) as f64;
        let board_max = worth_paying.min(lump_cap);
        let player_min = case.settlement.ask;

        let loan_value = if case.has_loan_runway {
            (1.0 - r) * case.annual_wage * (case.contract_days_left as f64 / 365.0).min(1.0)
        } else {
            0.0
        };

        // They meet in the middle of the gap, when there is one.
        let settlement = (board_max >= player_min)
            .then(|| player_min + (board_max - player_min) / 2.0)
            .filter(|amount| carry - amount - unwritten_book - hold_value > loan_value);
        let route = match settlement {
            Some(amount) => StrandedRoute::Settle { amount },
            None if case.has_loan_runway => StrandedRoute::LoanOut {
                subsidy: resolve.value(),
            },
            None => StrandedRoute::KeepSelling,
        };
        if !matches!(route, StrandedRoute::Settle { .. }) {
            debug!(
                "stranded hold: {:?} resolve {:.2} carry {:.0} asked {:.0} worth paying {:.0} lump cap {:.0} loan value {:.0}",
                route, r, carry, player_min, worth_paying, lump_cap, loan_value,
            );
        }

        StrandedVerdict {
            resolve,
            anchor,
            board_floor,
            route,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fx;

    impl Fx {
        const YEAR: i64 = 365;

        fn calm() -> LedgerPressure {
            LedgerPressure {
                cash_need: 0.0,
                wage_pressure: 0.0,
            }
        }

        /// An ordinary earner at an ordinary club, listed at 5M, nothing on
        /// the books, and a man who would only leave for every penny he is
        /// owed.
        fn case(exposure_days: u16, annual_wage: f64, contract_days_left: i64) -> StrandedCase {
            let carry = annual_wage * contract_days_left as f64 / 365.0;
            StrandedCase {
                exposure_days,
                annual_wage,
                average_wage: 1_000_000.0,
                annual_wage_bill: 30_000_000,
                contract_days_left,
                current_ask: 5_000_000.0,
                best_rejected_bid: None,
                book_value: 0.0,
                has_loan_runway: true,
                settlement: SettlementAsk { ask: carry, carry },
            }
        }

        fn asking(mut case: StrandedCase, share_of_carry: f64) -> StrandedCase {
            let carry = case.settlement.carry;
            case.settlement = SettlementAsk {
                ask: carry * share_of_carry,
                carry,
            };
            case
        }

        fn review(case: &StrandedCase) -> StrandedVerdict {
            ClubBoard::new().review_stranded(case, &Self::calm())
        }
    }

    #[test]
    fn more_exposure_never_lowers_resolve() {
        let mut last = 0.0;
        for days in [0u16, 10, 40, 80, 110, 220, 440] {
            let resolve =
                StrandedResolve::read(&Fx::case(days, 1_000_000.0, 3 * Fx::YEAR), &Fx::calm());
            assert!(
                resolve.value() >= last,
                "{days}d read {} after {last}",
                resolve.value()
            );
            last = resolve.value();
        }
        let unseen = StrandedResolve::read(&Fx::case(0, 1_000_000.0, 3 * Fx::YEAR), &Fx::calm());
        assert_eq!(unseen.value(), 0.0);
    }

    #[test]
    fn a_player_who_would_walk_for_nothing_wears_the_board_down_sooner() {
        let pushing = Fx::asking(Fx::case(80, 1_000_000.0, 2 * Fx::YEAR), 0.0);
        let content = Fx::case(80, 1_000_000.0, 2 * Fx::YEAR);
        let pushing = StrandedResolve::read(&pushing, &Fx::calm());
        let content = StrandedResolve::read(&content, &Fx::calm());
        assert!(
            pushing > content,
            "pushing {pushing:?} vs content {content:?}"
        );
    }

    #[test]
    fn a_heavy_earner_presses_harder_than_a_cheap_one() {
        let heavy = StrandedResolve::read(&Fx::case(80, 4_000_000.0, 2 * Fx::YEAR), &Fx::calm());
        let cheap = StrandedResolve::read(&Fx::case(80, 500_000.0, 2 * Fx::YEAR), &Fx::calm());
        assert!(heavy > cheap, "heavy {:?} vs cheap {:?}", heavy, cheap);
    }

    #[test]
    fn a_listing_made_days_before_the_close_barely_registers() {
        let verdict = Fx::review(&Fx::case(3, 1_000_000.0, 3 * Fx::YEAR));
        assert!(verdict.resolve.value() < 0.05, "{:?}", verdict.resolve);
        assert!(
            verdict.anchor > 5_000_000.0 * 0.95,
            "anchor {}",
            verdict.anchor
        );
        assert!(!matches!(verdict.route, StrandedRoute::Settle { .. }));
    }

    #[test]
    fn a_rejected_bid_names_the_price() {
        let mut case = Fx::case(440, 1_000_000.0, 3 * Fx::YEAR);
        case.best_rejected_bid = Some(2_000_000.0);
        let verdict = Fx::review(&case);
        assert!(
            (verdict.anchor - 2_000_000.0).abs() < 400_000.0,
            "anchor {} should sit near the 2M bid",
            verdict.anchor
        );
        assert!(verdict.board_floor <= verdict.anchor);
    }

    /// A window in which nobody bid at all is the market's answer to the
    /// price; the longer it has been saying it, the further the board comes
    /// down.
    #[test]
    fn a_window_without_a_bid_cuts_the_price() {
        let early = Fx::review(&Fx::case(80, 1_000_000.0, 3 * Fx::YEAR));
        let late = Fx::review(&Fx::case(440, 1_000_000.0, 3 * Fx::YEAR));
        assert!(early.anchor < 5_000_000.0);
        assert!(
            late.anchor < early.anchor * 0.5,
            "{} then {}",
            early.anchor,
            late.anchor
        );
    }

    #[test]
    fn the_anchor_never_falls_below_the_book_the_board_keeps() {
        let mut case = Fx::case(440, 1_000_000.0, 3 * Fx::YEAR);
        case.best_rejected_bid = Some(500_000.0);
        case.book_value = 10_000_000.0;
        let pressure = Fx::calm();
        let board = ClubBoard::new();
        let verdict = board.review_stranded(&case, &pressure);
        let kept = case.book_value
            * (1.0
                - board
                    .write_off_share(&pressure)
                    .max(verdict.resolve.value() as f64));
        assert!(verdict.anchor >= kept.min(case.current_ask) - 1.0);
        assert!(verdict.board_floor >= kept.min(verdict.anchor) - 1.0);
    }

    #[test]
    fn a_young_stranded_player_goes_out_on_loan() {
        let case = Fx::asking(Fx::case(80, 1_000_000.0, 3 * Fx::YEAR), 0.1);
        assert!(ClubBoard::has_loan_runway(Some(36), 24, 0));
        match Fx::review(&case).route {
            StrandedRoute::LoanOut { subsidy } => assert!(subsidy > 0.0),
            other => panic!("expected a loan, got {other:?}"),
        }
    }

    /// A sale takes his wages off the books as well as bringing a fee, so
    /// a board that still believes in one does not pay him to leave after
    /// a single window — even when he would walk for nothing.
    #[test]
    fn one_window_is_not_enough_to_pay_off_a_man_who_could_still_be_sold() {
        let mut case = Fx::asking(Fx::case(40, 1_000_000.0, 3 * Fx::YEAR), 0.5);
        case.current_ask = 400_000.0;
        match Fx::review(&case).route {
            StrandedRoute::LoanOut { .. } => {}
            other => panic!("expected a loan, got {other:?}"),
        }
    }

    #[test]
    fn a_near_expiry_veteran_has_no_loan_route() {
        assert!(!ClubBoard::has_loan_runway(Some(8), 34, 0));
        let mut case = Fx::case(220, 2_000_000.0, 240);
        case.has_loan_runway = false;
        let route = Fx::review(&case).route;
        assert!(!matches!(route, StrandedRoute::LoanOut { .. }), "{route:?}");
    }

    #[test]
    fn a_frozen_out_player_with_a_market_takes_a_pay_off() {
        let mut case = Fx::asking(Fx::case(330, 3_000_000.0, 2 * Fx::YEAR), 0.2);
        case.current_ask = 2_000_000.0;
        case.annual_wage_bill = 60_000_000;
        let carry = case.settlement.carry;
        match Fx::review(&case).route {
            StrandedRoute::Settle { amount } => {
                assert!(amount >= case.settlement.ask, "{amount} under his ask");
                assert!(amount < carry, "{amount} is not below the {carry} owed");
            }
            other => panic!("expected a settlement, got {other:?}"),
        }
    }

    #[test]
    fn a_veteran_with_no_market_keeps_his_contract() {
        let mut case = Fx::asking(Fx::case(330, 3_000_000.0, 2 * Fx::YEAR), 0.95);
        case.has_loan_runway = false;
        assert!(case.settlement.ask > ClubBoard::lump_sum_cap(case.annual_wage_bill) as f64);
        assert_eq!(Fx::review(&case).route, StrandedRoute::KeepSelling);
    }

    #[test]
    fn a_recent_big_fee_signing_is_not_paid_off() {
        let mut case = Fx::asking(Fx::case(20, 3_000_000.0, 3 * Fx::YEAR), 0.0);
        case.book_value = 20_000_000.0;
        let route = Fx::review(&case).route;
        assert!(!matches!(route, StrandedRoute::Settle { .. }), "{route:?}");
    }
}

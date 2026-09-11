//! What the board hands the manager for the season, and the arithmetic
//! that arrives at it.
//!
//! [`SeasonTargets`] is the mandate: a transfer budget, a wage budget, the
//! slice of both the owner rather than the revenue is paying for, squad
//! limits, and where the club is expected to finish. [`SeasonBudget`] is
//! how the board gets there — free cash, ambition, regulatory standing and
//! the owner's cheque, floored and capped so neither a break-even giant nor
//! a cash-poor minnow ends up frozen out of the market.

use crate::club::board::chairman::{ChairmanAmbition, ChairmanProfile};
use crate::club::board::context::{BoardContext, FfpStatus};
use crate::club::board::ownership::{ClubBenefactor, OwnershipModel};
use crate::club::board::vision::{ClubVision, LongTermGoal};
use crate::transfers::value::wage::OwnerEnvelopes;

#[derive(Debug, Clone, Default)]
pub struct SeasonTargets {
    pub transfer_budget: i32,
    pub wage_budget: i32,
    /// The slice of [`Self::wage_budget`] the OWNER is funding rather than
    /// the club's revenue. Held separately so the negotiation's wage power
    /// can take it back out of the mandate before adding the tier envelope
    /// — otherwise the same cheque is counted on both arms.
    pub owner_subsidy: i64,
    /// That cheque, split per brief tier and drawn down as shirts are
    /// signed. See [`OwnerEnvelopes`].
    pub owner_envelopes: OwnerEnvelopes,
    pub max_squad_size: u8,
    pub min_squad_size: u8,
    /// Expected league finish position (1-based). Board judges performance against this.
    pub expected_position: u8,
    /// Minimum acceptable position before board becomes unhappy
    pub min_acceptable_position: u8,
    /// Signed money the board has added to or taken off the transfer
    /// mandate since the season opened.
    ///
    /// Kept on the mandate rather than applied to the live budget, because
    /// the live budget is rebuilt from this mandate every month: a cut
    /// written only to the live figure survived until the next monthly
    /// recompute and then vanished, which is why an unhappy board re-issued
    /// the same 25% cut — and the same news story — every month it stayed
    /// unhappy. Reset with the rest of the mandate at the season turn.
    pub mandate_adjustment: i64,
    /// Signed money the board has added to or taken off the wage mandate,
    /// on the same terms.
    pub wage_mandate_adjustment: i64,
}

impl SeasonTargets {
    /// The transfer mandate as it stands after everything the board has
    /// moved this season.
    pub fn adjusted_transfer_budget(&self) -> i64 {
        (self.transfer_budget as i64 + self.mandate_adjustment).max(0)
    }

    /// The wage mandate as it stands after everything the board has moved
    /// this season.
    pub fn adjusted_wage_budget(&self) -> i64 {
        (self.wage_budget as i64 + self.wage_mandate_adjustment).max(0)
    }
}

/// Board confidence in the current management (0-100).
/// Drops when results are poor, recovers when exceeding expectations.
/// At 0 — or after sustained Poor mood — the manager is sacked.
#[derive(Debug, Clone)]
pub struct BoardConfidence {
    pub level: i32,
}

impl Default for BoardConfidence {
    fn default() -> Self {
        BoardConfidence { level: 65 }
    }
}

/// The season mandate, derived from what the club earns, what its owner
/// will fund, and how much ambition the boardroom is carrying.
pub struct SeasonBudget;

impl SeasonBudget {
    /// Size this season's transfer chest, wage mandate, squad limits and
    /// league expectation from the club's standing.
    pub fn derive(
        board_ctx: &BoardContext,
        vision: &ClubVision,
        chairman: &ChairmanProfile,
        ownership: &OwnershipModel,
    ) -> SeasonTargets {
        let rep = board_ctx.reputation_score;

        // Revenue-based budgets: a club's transfer war chest comes from
        // the slack between projected income and projected expenses, not
        // from the cash balance. Clubs that spent the offseason hauling
        // in TV money get a meaningful budget; clubs running at a deficit
        // get nothing — even if the bank account looks healthy from a
        // recent owner injection.
        // A year's income the club can be judged on TODAY: the trailing
        // sum once a month has closed, its own revenue model's projection
        // before that. The trailing sum alone is 0 for every club in a
        // freshly created world, and every branch below that tests
        // `projected_income < 1.0` then took its cold-start arm — which
        // set `owner_subsidy = 0.0` and held the mandate at the existing
        // bill, for the whole of season one.
        let projected_income = board_ctx.projected_annual_income.max(0) as f64;
        // The expense side has to be projected with it, or the first
        // computation reads a club with a $110M wage bill as having no
        // costs at all and hands it a year's revenue as free cash. The
        // wage bill is the honest floor: it is most of what a club spends
        // and it is the one figure the context always carries.
        let projected_expenses = board_ctx
            .trailing_annual_outcome
            .max(board_ctx.total_annual_wages as i64)
            .max(0) as f64;
        let projected_free_cash = (projected_income - projected_expenses).max(0.0);

        let ambition_mult = vision.budget_multiplier();
        let chair_mult = chairman.budget_multiplier() as f64;
        let ffp_mult = match board_ctx.ffp_status {
            FfpStatus::Clean => 1.00,
            FfpStatus::Watchlist => 0.70,
            FfpStatus::Breach => 0.35,
        };

        // No-revenue fallback: a club whose own revenue model cannot even
        // project a year — it has no main team — would have free_cash == 0
        // and never get a budget. Seed the calculation with a
        // reputation-scaled allowance so it can still make signings.
        //
        // Since `projected_income` reads the PROJECTION, this is no longer
        // the whole world's first season; it is the handful of clubs with
        // nothing to project from.
        let seed_budget = if projected_income < 1.0 {
            let cash = board_ctx.balance.max(0) as f64;
            let seed_pct = if rep >= 0.8 {
                0.30
            } else if rep >= 0.6 {
                0.25
            } else if rep >= 0.4 {
                0.20
            } else {
                0.15
            };
            cash * seed_pct
        } else {
            0.0
        };

        // Ownership wealth/risk multiplier. Neutral owners resolve to 1.0
        // so the legacy budget tests are unchanged; a deep-pocketed
        // risk-taker can inflate the war chest, a cautious one throttles it.
        let owner_mult = ownership.budget_multiplier();
        let revenue_budget =
            projected_free_cash * ambition_mult * chair_mult * ffp_mult * owner_mult;

        // Reserve floor: a solvent club that merely BREAKS EVEN on the P&L
        // (revenue_budget ≈ 0) can still spend a bounded slice of its cash
        // reserves on transfers — directly, or via installments against future
        // revenue. Without this every established break-even club got a zero
        // transfer budget and was frozen out of the market for the whole
        // season, the single biggest cause of a stuck market. Gated on real,
        // positive cash (never an owner-loan overdraft) and on FFP so a
        // constrained club can't tap it freely; the division-tier ceiling
        // below still caps the result. It's a FLOOR, not mandated spend —
        // a rich club's revenue budget already exceeds it, so nothing changes
        // for clubs that were never frozen.
        const RESERVE_FLOOR_PCT: f64 = 0.10;
        let reserve_floor = if board_ctx.balance > 0 {
            board_ctx.balance as f64 * RESERVE_FLOOR_PCT * ffp_mult
        } else {
            0.0
        };
        // Income floor: what a going concern finances against its
        // TURNOVER. The revenue budget above is the P&L surplus, and for
        // every established club the P&L is roughly flat — wages, the
        // amortization of past fees and the ground eat the income — so the
        // surplus is zero and the reserve floor was the whole budget. Ten
        // per cent of the cash pile is a war chest for a club with no
        // revenue; for a giant earning $800M it froze the top of the
        // market: the 2032 census read every solvent giant at 52–95M
        // against strikers valued 83–307M, while the same clubs banked
        // half a billion. A fee is not paid out of last year's profit — it
        // is financed over the contract against next year's turnover,
        // which is why real clubs of that size gross-spend 15–25 % of
        // revenue in an ordinary year. The floor is that share, scaled by
        // how much of a wage-cover cushion the club actually holds (six
        // months of wages in the bank = full cover, nothing = none), so a
        // club living hand-to-mouth gets nothing from it and an indebted
        // one nothing at all. Gated on FFP like the reserve floor; the
        // ceiling below still caps it.
        const INCOME_FLOOR_SHARE: f64 = 0.15;
        const CASH_COVER_WAGE_MONTHS: f64 = 6.0;
        let income_floor = {
            let cover_bar = board_ctx.total_annual_wages as f64 * CASH_COVER_WAGE_MONTHS / 12.0;
            let cash_cover = if board_ctx.balance > 0 && cover_bar > 0.0 {
                (board_ctx.balance as f64 / cover_bar).clamp(0.0, 1.0)
            } else {
                0.0
            };
            board_ctx.trailing_annual_income.max(0) as f64
                * INCOME_FLOOR_SHARE
                * cash_cover
                * ffp_mult
        };
        // An owner-funded club spends its owner's cash on fees as well as
        // on wages: the 8 % idle-cash share below is what a club prudently
        // reinvests, and it is nothing like what a benefactor will write a
        // cheque for. Same `benefactor × idle × 0.35` envelope as the wage
        // subsidy, so one ratio governs both halves of what a club pays.
        let idle_cash =
            ClubBenefactor::idle_cash(board_ctx.balance, board_ctx.total_annual_wages as i64);
        let owner_fee_headroom =
            ClubBenefactor::subsidy_per_year(ownership.benefactor, 1.0, idle_cash);
        // …and the cheque is SPENDABLE, not merely permitted. Lifting only
        // the ceiling left it nowhere to land: a low-income cash-rich club
        // sits on the reserve floor, far under its ceiling, so a $53M ask
        // was priced affordable by `spend_power` and then never bid,
        // because bids are sized against `season_targets.transfer_budget`.
        let raw_budget = (revenue_budget + seed_budget)
            .max(reserve_floor)
            .max(income_floor)
            .max(owner_fee_headroom)
            .max(0.0);

        let eco = board_ctx.country_economic_factor as f64;
        let price = board_ctx.country_price_level as f64;
        let price_ceiling = price * price * 80_000_000.0;
        let eco_ceiling = eco * eco * 300_000_000.0;
        // Division tier caps the war chest: lower leagues simply don't move
        // the same money. Top flight (tier 1) is unconstrained here.
        let tier_factor = match board_ctx.league_tier {
            0 | 1 => 1.0,
            2 => 0.6,
            3 => 0.35,
            _ => 0.2,
        };
        // The country ceiling above is a property of the LEAGUE, not of the
        // club, so every top-flight side in a country shared one number.
        // For the biggest clubs it was the binding constraint and it never
        // moved: a club could win everything, treble its revenue and pile
        // up two billion in cash, and its transfer budget stayed pinned to
        // the same ~$115M as its mid-table neighbour. Money that is never
        // spendable is a finance leak — it accumulates on the balance sheet
        // for ever and never reaches the market.
        //
        // So the ceiling grows with the club as well as with the country:
        // a share of what it EARNS, plus a share of the cash genuinely idle
        // behind its wage commitments. A rep-6000 club with modest revenue
        // and no reserves barely moves; a giant reaches the gross-spend band
        // real clubs of that size actually operate in.
        const INCOME_SHARE: f64 = 0.25;
        const IDLE_CASH_SHARE: f64 = 0.08;
        let annual_income = board_ctx.trailing_annual_income.max(0) as f64;
        let club_headroom =
            INCOME_SHARE * annual_income + IDLE_CASH_SHARE * idle_cash + owner_fee_headroom;
        let budget_ceiling = (price_ceiling.min(eco_ceiling) + club_headroom) * tier_factor;
        let mut transfer_budget = raw_budget.min(budget_ceiling) as i32;

        // A club trading in emergency measures or administration cannot buy
        // players for a fee, whatever its revenue says.
        if board_ctx.debt_standing.blocks_transfer_spending() {
            transfer_budget = 0;
        }

        // Wage budget: target wage/revenue ratio. Healthy clubs run
        // 55–65% wages on revenue; distressed clubs squeeze that down to
        // 45–50%; reckless elite owners are allowed to push to 70%. The
        // debt standing supplies a hard ceiling on top.
        let target_ratio =
            (Self::wage_revenue_target(board_ctx.ffp_status, chairman.ambition, rep)
                + ownership.wage_ratio_bonus())
            .clamp(0.30, 0.80)
            .min(board_ctx.debt_standing.wage_ratio_ceiling());

        let current_wages = board_ctx.total_annual_wages as f64;
        // What the owner is putting into the wage bill this year. Held on
        // the targets as well as folded into the mandate, because the
        // negotiation's wage power has to take it back OUT before adding a
        // tier envelope — the same cheque on both arms was the "$33M shirt
        // is $55M" defect.
        let owner_subsidy = if projected_income < 1.0 {
            // Nothing to project from at all: the mandate below is the
            // existing bill and carries no owner money, so neither does
            // the ledger.
            0.0
        } else {
            ownership.owner_subsidy_per_year(idle_cash)
        };
        let wage_budget = if projected_income < 1.0 {
            // Nothing to project from (a club with no main team): hold the
            // mandate at the existing bill rather than inventing a cut.
            current_wages
        } else {
            // Size the mandate off *revenue*, not off the wage bill.
            //
            // The old line was:
            //
            //     let revenue_floor = projected_income.max(total_annual_wages);
            //     (revenue_floor * target_ratio).max(total_annual_wages * 0.95)
            //
            // Both halves ratcheted upward. Taking `max` with the wage bill
            // meant that once wages passed revenue the budget was computed
            // from the wages themselves, so overspending justified itself;
            // and the closing floor of 95% of the current bill made it
            // arithmetically impossible for the board to ever mandate a
            // reduction. A club whose income had collapsed kept authorising
            // the wages that were bankrupting it.
            //
            // The mandate can now fall — but by at most 15% per recompute,
            // because squads unwind through expiries and sales, not
            // overnight.
            //
            // …plus what the owner is funding. A club whose cash its
            // revenue cannot explain can hold a wage its revenue cannot
            // explain either — that is the entire mechanism behind the
            // Saudi, Chinese, Russian and MLS windows, and it needs no
            // league list to express (memory
            // `feedback_balance_system_not_cases`).
            (projected_income * target_ratio).max(current_wages * 0.85) + owner_subsidy
        } as i32;

        // Squad size limits based on reputation
        let (min_squad, max_squad) = if rep >= 0.8 {
            (25u8, 50u8)
        } else if rep >= 0.6 {
            (23, 45)
        } else if rep >= 0.4 {
            (20, 38)
        } else if rep >= 0.2 {
            (18, 30)
        } else {
            (16, 25)
        };

        // Expected league position. Reputation sets the baseline, then the
        // owner's ambition and the long-term goal pull it up or down, and a
        // lower division nudges a reputable club towards the promotion mix.
        //
        // TODO: previous-season finish and a league-relative wage / squad-
        // ability rank would sharpen this. Neither is threaded into
        // `BoardContext` yet, so we fall back to the reputation baseline
        // rather than invent a rank.
        let (expected, min_acceptable) = if board_ctx.league_size > 0 {
            let league_sz = board_ctx.league_size as f32;

            // Baseline: 0.0 = champions, 1.0 = bottom of the table.
            let mut frac = 1.0 - rep;

            // Owner ambition shifts the bar; reckless owners demand more.
            frac += match chairman.ambition {
                ChairmanAmbition::Reckless => -0.10,
                ChairmanAmbition::Ambitious => -0.05,
                ChairmanAmbition::Balanced => 0.0,
                ChairmanAmbition::Conservative => 0.06,
            };

            // The long-term goal anchors a ceiling (or floor) on the brief.
            if let Some(goal) = vision.long_term_goal {
                match goal {
                    LongTermGoal::WinLeague
                    | LongTermGoal::WinContinental
                    | LongTermGoal::PromotionToTopFlight => frac = frac.min(0.10),
                    LongTermGoal::EstablishTopHalf => frac = frac.min(0.45),
                    LongTermGoal::Survive => frac = frac.max(0.72),
                    LongTermGoal::WinDomesticCup => {}
                }
            }

            // A reputable club in a lower division is expected to push up.
            if board_ctx.league_tier >= 2 {
                frac -= 0.05;
            }

            frac = frac.clamp(0.02, 0.97);
            let expected = ((frac * league_sz).round() as u8).clamp(1, board_ctx.league_size);
            // Acceptable floor sits a quarter-table below the target.
            let buffer = (league_sz * 0.25).max(2.0) as u8;
            let min_acceptable = expected.saturating_add(buffer).min(board_ctx.league_size);
            (expected, min_acceptable)
        } else {
            (1, 1)
        };

        SeasonTargets {
            transfer_budget,
            wage_budget,
            owner_subsidy: owner_subsidy as i64,
            owner_envelopes: OwnerEnvelopes::split(owner_subsidy),
            max_squad_size: max_squad,
            min_squad_size: min_squad,
            expected_position: expected,
            min_acceptable_position: min_acceptable,
            // A fresh mandate carries no history: whatever the board moved
            // last season was a judgement on last season.
            mandate_adjustment: 0,
            wage_mandate_adjustment: 0,
        }
    }

    /// Target wage-to-revenue ratio. Healthy clubs target 55-65%;
    /// distressed clubs squeeze it to 45-50%; reckless owners at the elite
    /// tier are allowed up to 70%.
    fn wage_revenue_target(
        ffp: FfpStatus,
        ambition: ChairmanAmbition,
        reputation_score: f32,
    ) -> f64 {
        let base: f64 = match ffp {
            FfpStatus::Clean => 0.62,
            FfpStatus::Watchlist => 0.55,
            FfpStatus::Breach => 0.48,
        };
        if matches!(ambition, ChairmanAmbition::Reckless) && reputation_score >= 0.75 {
            return 0.70;
        }
        if matches!(ambition, ChairmanAmbition::Conservative) {
            return (base - 0.05_f64).max(0.35);
        }
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::board::ClubBoard;
    use crate::club::board::scoring::SeasonPhase;

    fn make_ctx(income: i64, outcome: i64, ffp: FfpStatus) -> BoardContext {
        let mut c = BoardContext::new();
        c.balance = 10_000_000;
        c.total_annual_wages = 50_000_000;
        c.reputation_score = 0.6;
        c.country_economic_factor = 1.0;
        c.country_price_level = 1.0;
        c.trailing_annual_income = income;
        c.trailing_annual_outcome = outcome;
        // What `Club::simulate` writes: the trailing sum once it exists,
        // a revenue projection before that. The budgets read THIS.
        c.projected_annual_income = income;
        c.ffp_status = ffp;
        c
    }

    fn calc(ctx: &BoardContext) -> SeasonTargets {
        let mut board = ClubBoard::new();
        board.calculate_season_targets(ctx);
        board.season_targets.expect("should produce targets")
    }

    #[test]
    fn budget_shrinks_under_ffp_breach() {
        let clean = make_ctx(120_000_000, 90_000_000, FfpStatus::Clean);
        let breach = make_ctx(120_000_000, 90_000_000, FfpStatus::Breach);
        let watchlist = make_ctx(120_000_000, 90_000_000, FfpStatus::Watchlist);

        let t_clean = calc(&clean);
        let t_breach = calc(&breach);
        let t_watch = calc(&watchlist);

        assert!(
            t_breach.transfer_budget < t_clean.transfer_budget,
            "breach must cut transfer budget vs clean: {} vs {}",
            t_breach.transfer_budget,
            t_clean.transfer_budget
        );
        assert!(
            t_watch.transfer_budget < t_clean.transfer_budget,
            "watchlist must cut transfer budget vs clean"
        );
        assert!(
            t_breach.transfer_budget <= t_watch.transfer_budget,
            "breach must cut harder than watchlist"
        );
    }

    #[test]
    fn budget_zero_when_outflows_exceed_inflows_and_no_seed_cash() {
        let mut ctx = make_ctx(80_000_000, 95_000_000, FfpStatus::Clean);
        ctx.balance = 0; // no seed cash
        let t = calc(&ctx);
        assert_eq!(t.transfer_budget, 0);
    }

    #[test]
    fn break_even_but_cash_rich_club_still_gets_a_reserve_budget() {
        // Established club (has trailing history, so no cold-start seed),
        // P&L exactly break-even (revenue budget ≈ 0), but sitting on real
        // cash: the reserve floor keeps it active in the market instead of
        // freezing it at a zero budget for the whole season.
        let mut ctx = make_ctx(80_000_000, 80_000_000, FfpStatus::Clean);
        ctx.balance = 20_000_000;
        let t = calc(&ctx);
        assert!(
            t.transfer_budget > 0,
            "a solvent break-even club must still have a transfer budget: {}",
            t.transfer_budget
        );
    }

    /// The 2032 census: every solvent giant sat on the 10 %-of-cash reserve
    /// floor because its P&L was flat, and 52–95M bought nothing in a
    /// market where a starting striker is valued at 83–307M. A club that
    /// turns over 800M with six months of wages in the bank finances fees
    /// against its turnover, not out of last year's profit.
    #[test]
    fn a_break_even_giant_spends_against_its_turnover() {
        let mut ctx = make_ctx(800_000_000, 800_000_000, FfpStatus::Clean);
        ctx.balance = 500_000_000;
        ctx.total_annual_wages = 480_000_000;
        ctx.reputation_score = 0.9;
        let t = calc(&ctx);
        assert!(
            t.transfer_budget >= 110_000_000,
            "a cash-covered break-even giant must reach the income floor: {}",
            t.transfer_budget
        );
        assert!(
            t.transfer_budget <= 130_000_000,
            "…and no further than its turnover share allows: {}",
            t.transfer_budget
        );
    }

    #[test]
    fn the_income_floor_needs_cash_cover_and_a_positive_balance() {
        // Same turnover, but living hand-to-mouth: a tenth of the wage
        // cover means a tenth of the floor.
        let mut thin = make_ctx(800_000_000, 800_000_000, FfpStatus::Clean);
        thin.balance = 24_000_000;
        thin.total_annual_wages = 480_000_000;
        let t_thin = calc(&thin);
        assert!(
            t_thin.transfer_budget < 30_000_000,
            "a cash-poor club gets only the covered slice: {}",
            t_thin.transfer_budget
        );

        // In the red there is no floor at all, whatever the turnover.
        let mut indebted = make_ctx(800_000_000, 900_000_000, FfpStatus::Clean);
        indebted.balance = -100_000_000;
        indebted.total_annual_wages = 480_000_000;
        assert_eq!(calc(&indebted).transfer_budget, 0);
    }

    #[test]
    fn cold_start_with_zero_history_falls_back_to_cash_seed() {
        let mut ctx = make_ctx(0, 0, FfpStatus::Clean);
        ctx.balance = 50_000_000;
        ctx.reputation_score = 0.85; // 0.30 seed pct
        let t = calc(&ctx);
        assert!(t.transfer_budget > 0);
    }

    #[test]
    fn wage_budget_distress_ratio_lower_than_clean() {
        let clean = make_ctx(100_000_000, 60_000_000, FfpStatus::Clean);
        let distress = make_ctx(100_000_000, 60_000_000, FfpStatus::Breach);
        let t_clean = calc(&clean);
        let t_distress = calc(&distress);
        assert!(
            t_distress.wage_budget <= t_clean.wage_budget,
            "distressed wage budget should not exceed clean"
        );
    }

    #[test]
    fn ambitious_owner_with_title_goal_raises_expected_position() {
        let mut ctx = make_ctx(120_000_000, 90_000_000, FfpStatus::Clean);
        ctx.league_size = 20;
        ctx.reputation_score = 0.55;

        let mut ambitious = ClubBoard::new();
        ambitious.chairman.ambition = ChairmanAmbition::Reckless;
        ambitious.vision.long_term_goal = Some(LongTermGoal::WinLeague);
        ambitious.calculate_season_targets(&ctx);
        let amb = ambitious.season_targets.unwrap().expected_position;

        let mut modest = ClubBoard::new();
        modest.calculate_season_targets(&ctx);
        let mid = modest.season_targets.unwrap().expected_position;

        assert!(
            amb < mid,
            "title-chasing owner expects higher: {amb} vs {mid}"
        );
        assert!(
            amb <= 3,
            "a reckless title goal targets the very top: {amb}"
        );
    }

    #[test]
    fn low_rep_survival_side_is_not_expected_to_finish_mid_table() {
        let mut ctx = make_ctx(40_000_000, 38_000_000, FfpStatus::Clean);
        ctx.league_size = 20;
        ctx.reputation_score = 0.18;

        let mut board = ClubBoard::new();
        board.vision.long_term_goal = Some(LongTermGoal::Survive);
        board.chairman.ambition = ChairmanAmbition::Conservative;
        board.calculate_season_targets(&ctx);
        let t = board.season_targets.unwrap();

        assert!(
            t.expected_position >= 14,
            "a survival side expects the lower reaches, not mid-table: {}",
            t.expected_position
        );
        assert!(t.min_acceptable_position >= t.expected_position);
    }

    #[test]
    fn conservative_small_club_is_not_handed_an_impossible_finish() {
        let mut ctx = make_ctx(30_000_000, 29_000_000, FfpStatus::Clean);
        ctx.league_size = 20;
        ctx.reputation_score = 0.30;

        let mut board = ClubBoard::new();
        board.chairman.ambition = ChairmanAmbition::Conservative;
        board.calculate_season_targets(&ctx);
        let t = board.season_targets.unwrap();

        assert!(
            t.expected_position >= 10,
            "a modest club shouldn't be told to finish near the top: {}",
            t.expected_position
        );
    }

    #[test]
    fn lower_division_reputable_club_expects_to_push_for_promotion() {
        let mut top_flight = make_ctx(60_000_000, 55_000_000, FfpStatus::Clean);
        top_flight.league_size = 20;
        top_flight.reputation_score = 0.5;
        top_flight.league_tier = 1;
        let mut second_tier = top_flight.clone();
        second_tier.league_tier = 2;

        let mut a = ClubBoard::new();
        a.calculate_season_targets(&top_flight);
        let mut b = ClubBoard::new();
        b.calculate_season_targets(&second_tier);

        assert!(
            b.season_targets.unwrap().expected_position
                <= a.season_targets.unwrap().expected_position,
            "the same club expects a higher finish in a weaker division"
        );
    }

    #[test]
    fn season_phase_delays_table_judgment_until_enough_matches() {
        assert_eq!(SeasonPhase::classify(4, 38), SeasonPhase::TooEarly);
        assert_eq!(SeasonPhase::classify(8, 38), SeasonPhase::Early);
        assert!(!SeasonPhase::classify(8, 38).can_sack_manager());
        assert!(SeasonPhase::classify(16, 38).can_sack_manager());
        assert_eq!(SeasonPhase::classify(32, 38), SeasonPhase::RunIn);
    }
}

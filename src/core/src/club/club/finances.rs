use super::Club;
use crate::ContractBonusType;
use crate::club::DistressLevel;
use crate::club::classify_distress;
use crate::club::finance::{
    AdministrationState, DebtProfile, DebtStanding, RevenueInputs, RevenueModel,
};
use crate::club::news::affairs::ClubAffair;
use crate::context::GlobalContext;
use chrono::Datelike;
use chrono::NaiveDate;
use log::debug;

/// Country price level: scales ticket prices and commercial income by the
/// local economy. England 1.5, Colombia 0.4, default 1.0.
fn get_price_level(ctx: &GlobalContext<'_>) -> f64 {
    ctx.country
        .as_ref()
        .map(|c| c.price_level as f64)
        .unwrap_or(1.0)
}

impl Club {
    pub(super) fn process_monthly_finances(&mut self, ctx: GlobalContext<'_>) {
        let club_name = ctx.club.as_ref().expect("no club found").name;
        let date = ctx.simulation.date.date();

        // Economic factors from country
        let tv_multiplier = ctx
            .country
            .as_ref()
            .map(|c| c.tv_revenue_multiplier)
            .unwrap_or(1.0);
        let attendance_factor = ctx
            .country
            .as_ref()
            .map(|c| c.stadium_attendance_factor)
            .unwrap_or(1.0);
        let sponsorship_strength = ctx
            .country
            .as_ref()
            .map(|c| c.sponsorship_market_strength)
            .unwrap_or(1.0);

        // 1. Player wages: annual salary / 12. `Team::get_annual_salary`
        // returns *only* player wages (loan-aware: borrowers bill the
        // loan contract, not the parent contract).
        for team in self.teams.iter() {
            let annual_salary = team.get_annual_salary();
            let monthly_salary = annual_salary / 12;
            self.finance.push_salary(club_name, monthly_salary as i64);
        }

        // 1b. Lump-sum bonuses owed this month: signing bonus on freshly
        // signed contracts, loyalty bonus on each contract anniversary
        // year. Mutates the contract's `signing_bonus_paid` / per-year
        // memos so a re-run of this pass cannot double-charge.
        let bonus_payout = settle_lump_sum_bonuses(self, date);
        if bonus_payout > 0 {
            self.finance.balance.push_expense_player_wages(bonus_payout);
        }

        // 2. Staff wages: coaching, medical, scouting staff
        for team in self.teams.iter() {
            let staff_monthly = team.staffs.get_annual_salary() / 12;
            if staff_monthly > 0 {
                self.finance
                    .balance
                    .push_expense_staff_wages(staff_monthly as i64);
            }
        }

        // 3. Sponsorship income
        let sponsorship_income: i64 = self
            .finance
            .sponsorship
            .get_sponsorship_incomes(date)
            .iter()
            .map(|c| (c.wage / 12) as i64)
            .sum();
        if sponsorship_income > 0 {
            self.finance
                .balance
                .push_income_sponsorship(sponsorship_income);
        }

        // 4. Broadcast, matchday and commercial income.
        //
        // Every line is continuous in the club's own inputs — see
        // `finance::revenue`. The old model looked each one up from a
        // six-bucket reputation ladder, so a single tier slip cut broadcast
        // income 60% and commercial income 70% overnight while the wage
        // bill, fixed by contracts already signed, didn't move at all.
        let reputation_score = self
            .teams
            .main()
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.0);
        let has_main_team = self.teams.main().is_some();

        if has_main_team {
            let (recent_wins_ratio, league_pos, total_teams) =
                self.compute_team_form_and_position(&ctx);
            let league_tier = ctx
                .club
                .as_ref()
                .map(|c| c.main_league_tier.max(1))
                .unwrap_or(1);

            let inputs = RevenueInputs {
                reputation_score,
                league_tier,
                league_position: league_pos,
                league_size: total_teams,
                tv_market: tv_multiplier,
                sponsorship_market: sponsorship_strength,
                attendance_factor,
                price_level: get_price_level(&ctx) as f32,
                stadium_capacity: self.facilities.capacity_or_estimate(reputation_score),
                recent_wins_ratio,
                home_matches: self.finance.take_home_match_count(),
                parachute: self.finance.parachute,
            };

            // Supporters turning up is the only part of the gate that moves
            // with results; the ground itself does not shrink.
            let form_mult = self.facilities.dynamic_attendance_multiplier(
                recent_wins_ratio,
                league_pos,
                total_teams,
            );
            let revenue = RevenueModel::monthly(&inputs, form_mult);

            if revenue.broadcast_base > 0 {
                self.finance.balance.push_income_tv(revenue.broadcast_base);
            }
            if revenue.broadcast_merit > 0 {
                self.finance
                    .balance
                    .push_income_tv_placement(revenue.broadcast_merit);
            }
            if revenue.parachute > 0 {
                self.finance
                    .balance
                    .push_income_parachute(revenue.parachute);
            }
            if revenue.matchday > 0 {
                self.finance.balance.push_income_matchday(revenue.matchday);
                // Keep the club's observed average gate honest — it feeds
                // the board's expansion case and the UI.
                self.facilities.average_attendance = revenue.attendance;
            }
            if revenue.commercial > 0 {
                self.finance
                    .balance
                    .push_income_merchandising(revenue.commercial);
            }
        }

        // 5. Amortization: each outstanding transfer purchase contributes
        // its monthly slice as a P&L expense. Cash already left the
        // balance at the upfront purchase, so this only hits `outcome` and
        // the categorised `expense_amortization` bucket.
        self.finance.tick_amortization();

        // 6. Facility maintenance costs
        let facility_cost: i64 = (self.facilities.training.to_rating() as i64
            + self.facilities.youth.to_rating() as i64
            + self.facilities.academy.to_rating() as i64)
            * 5_000;
        self.finance.balance.push_expense_facilities(facility_cost);

        // 7. Operating overhead: administration, taxes, community,
        // marketing, infrastructure. A share of revenue (the way real
        // clubs' SG&A actually scales) plus a fixed institutional floor
        // that doesn't shrink in a lean year. The floor is continuous in
        // reputation rather than a tier lookup, so a club sliding down the
        // table doesn't get an overnight cost cut it hasn't earned.
        let monthly_income = self.finance.balance.income;
        let overhead = if has_main_team {
            RevenueModel::operating_overhead(monthly_income, reputation_score)
        } else {
            0
        };
        if overhead > 0 {
            self.finance.balance.push_expense_facilities(overhead);
        }

        // 8. Debt service and resolution.
        //
        // Interest is charged only on borrowing inside the club's agreed
        // facility, at a single-digit annual rate. The old model charged
        // 0.6-1.5% *per month* on the entire negative balance and added it
        // straight back onto that balance — an uncapped compounding term
        // that drove clubs to nine-figure and then ten-figure debts with no
        // mechanism anywhere to stop or resolve it.
        let funded_months = self.finance.monthly_history_depth(date);
        // Sizing the club's borrowing facility off a trailing year it hasn't
        // lived yet would read every DB-seeded debt as a bankruptcy on the
        // first tick of a new world. The estimate annualises whatever months
        // exist; the month just billed backstops the very first tick, when
        // history is still empty.
        let trailing_income = self
            .finance
            .estimated_annual_income(date)
            .max(monthly_income.max(0).saturating_mul(12));
        let avg_wages = self.finance.trailing_avg_monthly_wages(date);
        let distress = classify_distress(self.finance.balance.balance, avg_wages);
        self.finance.distress_level = distress;

        let league_tier = ctx
            .club
            .as_ref()
            .map(|c| c.main_league_tier.max(1))
            .unwrap_or(1);
        self.resolve_debt(
            club_name,
            trailing_income,
            avg_wages,
            distress,
            league_tier,
            funded_months,
            date,
        );

        // 9. Excess-cash deployment. Nothing else in the sim scales with
        // accumulated wealth — budgets come from trailing free cash flow,
        // wages from ability/reputation — so without this the balance is a
        // monotone accumulator and a decade of small operating surpluses
        // plus a few player sales parks nine-figure cash at a Regional
        // club. Real boards don't hoard: cash beyond a working reserve
        // goes into stadium/training programmes and owner distributions.
        // Booked as a pure cash outflow (like the upfront leg of a
        // transfer purchase): capital deployment is not an operating
        // expense, so P&L, FFP maths and budget projections stay clean.
        let funded_months = self.finance.monthly_history_depth(date);
        let deployment = ExcessCashDeployment::amount(
            self.finance.balance.balance,
            trailing_income,
            funded_months,
        );
        if deployment > 0 {
            self.finance.balance.push_cash_outflow(deployment);
            debug!(
                "club: {}, finance: excess-cash deployment of {} (balance {}, trailing income {})",
                club_name, deployment, self.finance.balance.balance, trailing_income
            );
        }

        let _ = club_name;
    }

    /// Service and, where necessary, resolve the club's debt.
    ///
    /// Walks the escalation ladder a real club meets: ordinary borrowing
    /// inside a facility, then owner funding, then emergency trading, then
    /// administration. The last rung is what guarantees termination — it
    /// writes the unpayable balance down to something the club can service
    /// in exchange for a points deduction and a year of embargo. Without a
    /// terminal state the balance is a pure divergent series.
    ///
    /// Three rungs of the ladder are also *dated events* rather than
    /// states, so each is filed in the club's diary at the line that
    /// performs it — see [`crate::club::news::affairs`]. The standing it
    /// leaves behind (a debt, an embargo) the press can read off the club
    /// whenever it likes; the day it changed, it cannot.
    #[allow(clippy::too_many_arguments)]
    fn resolve_debt(
        &mut self,
        club_name: &str,
        trailing_income: i64,
        avg_monthly_wages: i64,
        distress: DistressLevel,
        league_tier: u8,
        funded_months: usize,
        date: NaiveDate,
    ) {
        let administration = self.finance.debt.administration;
        let balance = self.finance.balance.balance;
        let standing = DebtProfile::classify(balance, trailing_income, distress, administration);
        self.finance.debt.standing = standing;

        // Interest on serviced borrowing only.
        let interest = self.finance.debt.monthly_interest(balance, trailing_income);
        if interest > 0 {
            self.finance.balance.push_expense_debt_interest(interest);
        }

        // The owner covers what he's willing to of the shortfall past the
        // facility. Booked as funding, not revenue — it must not flatter
        // the P&L or inflate next season's revenue-derived budgets.
        let appetite = self.board.ownership.injection_appetite();
        let injection = DebtProfile::owner_injection(
            self.finance.balance.balance,
            trailing_income,
            appetite,
            standing,
        );
        if injection > 0 {
            self.finance.balance.push_owner_investment(injection);
            self.affairs
                .record(ClubAffair::OwnerBailout { amount: injection }, date);
            debug!(
                "club: {}, finance: owner injected {} to cover a shortfall",
                club_name, injection
            );
        }

        // Tick down an administration already in force, or enter one.
        if let Some(mut state) = self.finance.debt.administration {
            if state.tick() {
                self.finance.debt.administration = None;
                self.finance.debt.standing = DebtProfile::classify(
                    self.finance.balance.balance,
                    trailing_income,
                    distress,
                    None,
                );
                self.affairs.record(ClubAffair::AdministrationExited, date);
                debug!("club: {}, finance: exited administration", club_name);
            } else {
                self.finance.debt.administration = Some(state);
            }
            return;
        }

        // Administration is a judgement about a club's trading record, so
        // it needs a real trading record. A world that has just been
        // generated must never put its indebted clubs straight into
        // administration on the strength of a seeded opening balance.
        const MIN_MONTHS_BEFORE_ADMINISTRATION: usize = 12;
        if funded_months < MIN_MONTHS_BEFORE_ADMINISTRATION {
            return;
        }

        let trailing_wages = avg_monthly_wages.saturating_mul(12);
        if DebtProfile::should_enter_administration(
            self.finance.balance.balance,
            trailing_income,
            trailing_wages,
            standing,
        ) {
            let written = DebtProfile::administration_write_down(
                self.finance.balance.balance,
                trailing_income,
            );
            if written > 0 {
                self.finance.balance.push_debt_write_down(written);
            }
            let state = AdministrationState::enter(league_tier);
            debug!(
                "club: {}, finance: entered administration — {} points deducted, {} written down",
                club_name, state.points_deduction, written
            );
            self.affairs.record(
                ClubAffair::AdministrationEntered {
                    points_deduction: state.points_deduction,
                },
                date,
            );
            self.finance.debt.administration = Some(state);
            self.finance.debt.standing = DebtStanding::Administration;
        }
    }

    /// Returns (recent_wins_ratio, league_position, total_teams) for the
    /// club's main team. Form comes from the last ~5 matches in the team's
    /// `match_history`; league position rides through `ClubContext` —
    /// which the country simulation populates from the live table.
    fn compute_team_form_and_position(&self, ctx: &GlobalContext<'_>) -> (f32, u16, u16) {
        let wins_ratio = self
            .teams
            .main()
            .map(|team| team.match_history.recent_wins_ratio(5))
            .unwrap_or(0.5);

        let (position, total) = ctx
            .club
            .as_ref()
            .map(|c| (c.league_position as u16, c.league_size as u16))
            .map(|(p, t)| if p == 0 || t == 0 { (10, 20) } else { (p, t) })
            .unwrap_or((10, 20));

        (wins_ratio, position, total)
    }
}

/// Board cash-deployment policy. A club keeps a working reserve of
/// [`Self::RESERVE_REVENUE_MULTIPLE`] × trailing annual revenue; cash
/// beyond that steadily leaves the club as infrastructure programmes and
/// owner distributions. This is the sim's only wealth-proportional flow —
/// every other expense scales with income or squad ability, so without it
/// balances only ever go up.
pub struct ExcessCashDeployment;

impl ExcessCashDeployment {
    /// Working reserve kept as a multiple of trailing annual revenue.
    /// 1.5× ≈ a season and a half of income — a healthy war chest, far
    /// below the 20×+ multiples hoarding produced.
    const RESERVE_REVENUE_MULTIPLE: f64 = 1.5;
    /// Absolute reserve floor so a tiny club is never drained toward zero.
    const RESERVE_FLOOR: i64 = 5_000_000;
    /// Fraction of the excess deployed per month (~46%/year): large piles
    /// unwind over a few seasons rather than vanishing overnight.
    const MONTHLY_RATE: f64 = 0.05;
    /// A full trailing year of completed-month snapshots is required
    /// before the policy runs, so a freshly generated world doesn't sweep
    /// DB-seeded balances before the club has any revenue evidence.
    const MIN_FUNDED_MONTHS: usize = 12;

    /// Cash to deploy this month. Zero when history is too shallow, the
    /// balance sits within the reserve, or the club is in the red.
    pub fn amount(balance: i64, trailing_annual_income: i64, funded_months: usize) -> i64 {
        if funded_months < Self::MIN_FUNDED_MONTHS {
            return 0;
        }
        let reserve = ((trailing_annual_income.max(0) as f64 * Self::RESERVE_REVENUE_MULTIPLE)
            as i64)
            .max(Self::RESERVE_FLOOR);
        let excess = balance - reserve;
        if excess <= 0 {
            return 0;
        }
        (excess as f64 * Self::MONTHLY_RATE) as i64
    }
}

/// True when `today` falls on or past `started`'s month/day in the
/// current calendar year. Used to gate annual loyalty payouts so a
/// Dec-31 contract doesn't accidentally pay a Jan-1 loyalty in the
/// following year — the contract hasn't reached its anniversary yet.
fn has_reached_anniversary(today: NaiveDate, started: NaiveDate) -> bool {
    if today.month() > started.month() {
        return true;
    }
    if today.month() < started.month() {
        return false;
    }
    today.day() >= started.day()
}

#[cfg(test)]
mod anniversary_tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn dec_31_signing_does_not_pay_jan_1_next_year() {
        // The classic edge case: a Dec-31 signing must NOT pay a
        // loyalty bonus on Jan 1 — that's not the anniversary.
        let signed = d(2026, 12, 31);
        assert!(!has_reached_anniversary(d(2027, 1, 1), signed));
        assert!(!has_reached_anniversary(d(2027, 6, 1), signed));
        assert!(!has_reached_anniversary(d(2027, 12, 30), signed));
        // Pays on the anniversary itself.
        assert!(has_reached_anniversary(d(2027, 12, 31), signed));
    }

    #[test]
    fn mid_year_signing_pays_after_anniversary_in_following_year() {
        let signed = d(2026, 7, 1);
        // Same month, before the day.
        assert!(!has_reached_anniversary(d(2027, 6, 30), signed));
        // Anniversary day.
        assert!(has_reached_anniversary(d(2027, 7, 1), signed));
        // Later in the year.
        assert!(has_reached_anniversary(d(2027, 11, 1), signed));
    }

    #[test]
    fn signing_year_does_not_pay_anniversary() {
        // Even though the date passes the month/day check WITHIN the
        // signing year, callers must additionally gate on year >
        // started.year() — the loyalty bonus pays from the FIRST
        // anniversary onward, not from "the day after signing".
        let signed = d(2026, 7, 1);
        // The helper itself just checks month/day — the year guard is
        // upstream in settle_lump_sum_bonuses.
        assert!(has_reached_anniversary(d(2026, 12, 31), signed));
    }
}

/// Pay every lump-sum bonus owed to a player on this monthly tick. Walks
/// the club's player contracts and, for each one:
///   - SigningBonus pays once on the first finance pass after acceptance.
///     Mutates `signing_bonus_paid = true` so subsequent passes skip it.
///   - LoyaltyBonus pays once per calendar year — the contract's
///     `last_loyalty_paid_year` memo prevents same-year double pay.
///   - InternationalCapFee pays per cap gained since the last pass.
///     Tracked via `last_intl_caps_paid` so the difference is the new caps.
///
/// Returns the total expense to charge to the club this month.
fn settle_lump_sum_bonuses(club: &mut Club, date: NaiveDate) -> i64 {
    let year = date.year();
    let mut total: i64 = 0;
    for team in club.teams.teams.iter_mut() {
        for player in team.players.players.iter_mut() {
            // Cap-tracking baseline lives on the player; caps cumulative
            // count is `player.player_attributes.international_apps`.
            let current_caps = player.player_attributes.international_apps;
            let baseline_caps = player.last_intl_caps_paid;

            if let Some(contract) = player.contract.as_mut() {
                for bonus in &contract.bonuses {
                    if bonus.value <= 0 {
                        continue;
                    }
                    match bonus.bonus_type {
                        ContractBonusType::SigningBonus => {
                            if !contract.signing_bonus_paid {
                                total += bonus.value as i64;
                            }
                        }
                        ContractBonusType::LoyaltyBonus => {
                            // Pay only when the calendar date is on or
                            // past the contract's month/day anniversary
                            // for the current year. A Dec-31 signing
                            // therefore pays nothing on Jan 1 of the
                            // following year — payout falls due on the
                            // next Dec 31. Pay at most once per
                            // calendar year (last_loyalty_paid_year
                            // memo). Year of signing pays nothing —
                            // that's the signing bonus.
                            if let Some(started) = contract.started {
                                if year <= started.year() {
                                    // Signing year — no loyalty payout.
                                } else if contract.last_loyalty_paid_year == Some(year) {
                                    // Already paid this calendar year.
                                } else if has_reached_anniversary(date, started) {
                                    total += bonus.value as i64;
                                }
                            }
                        }
                        ContractBonusType::InternationalCapFee => {
                            let new_caps = current_caps.saturating_sub(baseline_caps) as i64;
                            if new_caps > 0 {
                                total += bonus.value as i64 * new_caps;
                            }
                        }
                        _ => {}
                    }
                }
                // Memo updates AFTER the bonus value scan so a re-entrant
                // call within the same month is a no-op.
                if !contract.signing_bonus_paid
                    && contract
                        .bonuses
                        .iter()
                        .any(|b| matches!(b.bonus_type, ContractBonusType::SigningBonus))
                {
                    contract.signing_bonus_paid = true;
                }
                if let Some(started) = contract.started {
                    if year > started.year()
                        && contract.last_loyalty_paid_year != Some(year)
                        && has_reached_anniversary(date, started)
                        && contract
                            .bonuses
                            .iter()
                            .any(|b| matches!(b.bonus_type, ContractBonusType::LoyaltyBonus))
                    {
                        contract.last_loyalty_paid_year = Some(year);
                    }
                }
            }
            // Update international-caps baseline on the player so the next
            // pass only counts further caps. Done outside the contract
            // borrow.
            if current_caps > baseline_caps {
                player.last_intl_caps_paid = current_caps;
            }
        }
    }
    total
}

#[cfg(test)]
mod excess_cash_tests {
    use super::ExcessCashDeployment;

    #[test]
    fn no_deployment_without_a_full_year_of_history() {
        // A freshly generated club with a DB-seeded war chest must not be
        // swept before it has a year of revenue evidence.
        assert_eq!(ExcessCashDeployment::amount(450_000_000, 0, 0), 0);
        assert_eq!(ExcessCashDeployment::amount(450_000_000, 5_000_000, 11), 0);
    }

    #[test]
    fn balance_within_reserve_is_untouched() {
        // Reserve = 1.5 × 20M = 30M; a 25M balance sits inside it.
        assert_eq!(ExcessCashDeployment::amount(25_000_000, 20_000_000, 12), 0);
        // Negative balances obviously deploy nothing.
        assert_eq!(ExcessCashDeployment::amount(-5_000_000, 20_000_000, 12), 0);
    }

    #[test]
    fn reserve_floor_protects_tiny_clubs() {
        // Trailing income 1M → 1.5× would be 1.5M, but the 5M floor wins:
        // a 4M balance is fully protected.
        assert_eq!(ExcessCashDeployment::amount(4_000_000, 1_000_000, 12), 0);
        // 6M balance → 1M excess over the floor → 5% deployed.
        assert_eq!(
            ExcessCashDeployment::amount(6_000_000, 1_000_000, 12),
            50_000
        );
    }

    #[test]
    fn hoarded_small_club_pile_unwinds() {
        // The reported case: a Regional club sitting on 450M with ~5M/yr
        // revenue. Reserve = 7.5M, excess = 442.5M → ~22.1M leaves this
        // month; the pile converges to the reserve over a few seasons.
        let monthly = ExcessCashDeployment::amount(450_000_000, 5_000_000, 12);
        assert_eq!(monthly, 22_125_000);
    }

    #[test]
    fn healthy_elite_war_chest_is_kept() {
        // An Elite club with 250M revenue keeps up to 375M cash — only
        // genuine excess above that unwinds.
        assert_eq!(
            ExcessCashDeployment::amount(300_000_000, 250_000_000, 12),
            0
        );
        let monthly = ExcessCashDeployment::amount(500_000_000, 250_000_000, 12);
        assert_eq!(monthly, (125_000_000f64 * 0.05) as i64);
    }
}

#[cfg(test)]
mod helpers_tests {
    use crate::club::ClubFacilities;

    #[test]
    fn capacity_is_grossed_up_from_the_recorded_gate() {
        // The database records a typical attendance, not a capacity, so a
        // ground that averages 60,000 must model as bigger than that.
        let capacity = ClubFacilities::seed_capacity(60_000, 0.85);
        assert!(
            capacity > 60_000 && capacity < 80_000,
            "implausible capacity {capacity} from a 60,000 gate"
        );
    }

    #[test]
    fn capacity_falls_back_to_reputation_without_attendance_data() {
        let big = ClubFacilities::seed_capacity(0, 0.85);
        let small = ClubFacilities::seed_capacity(0, 0.20);
        assert!(big > small);
        // Never zero — a zero capacity silently erases matchday income.
        assert!(small > 0);
    }

    #[test]
    fn capacity_does_not_track_current_reputation() {
        // Anfield does not shrink when the team gets relegated. Once a club
        // carries a real capacity, the reputation argument is ignored.
        let facilities = ClubFacilities {
            stadium_capacity: 61_000,
            ..ClubFacilities::default()
        };
        assert_eq!(facilities.capacity_or_estimate(0.85), 61_000);
        assert_eq!(facilities.capacity_or_estimate(0.30), 61_000);
    }
}

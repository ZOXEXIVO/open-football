use super::Club;
use crate::club::{DistressLevel, classify_distress};
use crate::context::GlobalContext;
use crate::{ContractBonusType, ReputationLevel};
use chrono::Datelike;
use chrono::NaiveDate;
use log::debug;

/// Country price level: scales ticket prices, merchandising etc. by local economy.
/// England 1.5, Colombia 0.4, default 1.0.
fn get_price_level(ctx: &GlobalContext<'_>) -> f64 {
    ctx.country
        .as_ref()
        .map(|c| c.price_level as f64)
        .unwrap_or(1.0)
}

/// Tier-1 monthly TV base by reputation tier — sized to look right at a
/// market multiplier of 1.0 (a top-five league country) before the league
/// tier and league-position fan-out runs on top of it.
fn tv_revenue_base(rep: ReputationLevel) -> i64 {
    match rep {
        ReputationLevel::Elite => 8_000_000,
        ReputationLevel::Continental => 3_200_000,
        ReputationLevel::National => 900_000,
        ReputationLevel::Regional => 180_000,
        ReputationLevel::Local => 35_000,
        ReputationLevel::Amateur => 5_000,
    }
}

fn ticket_base_price(rep: ReputationLevel) -> f64 {
    match rep {
        ReputationLevel::Elite => 55.0,
        ReputationLevel::Continental => 40.0,
        ReputationLevel::National => 28.0,
        ReputationLevel::Regional => 15.0,
        ReputationLevel::Local => 8.0,
        ReputationLevel::Amateur => 4.0,
    }
}

/// Stadium-capacity ceiling derived from reputation tier. Used to cap
/// dynamic attendance so an in-form National-tier club isn't projected to
/// pull Premier League gates. Replace with per-club capacity once the
/// `ClubFacilities` ground-capacity field is plumbed in.
fn stadium_capacity_for(rep: ReputationLevel) -> u32 {
    match rep {
        ReputationLevel::Elite => 55_000,
        ReputationLevel::Continental => 38_000,
        ReputationLevel::National => 22_000,
        ReputationLevel::Regional => 9_000,
        ReputationLevel::Local => 3_500,
        ReputationLevel::Amateur => 1_000,
    }
}

fn league_tier_of(ctx: &GlobalContext<'_>, _league_id: Option<u32>) -> u8 {
    ctx.club
        .as_ref()
        .map(|c| c.main_league_tier.max(1))
        .unwrap_or(1)
}

fn league_tier_multiplier(tier: u8) -> f64 {
    // Smoothed so a promotion isn't a 3.5x TV windfall overnight. The
    // old 0.28 step from tier-2 to tier-1 turned a Championship-style
    // promotion into an instant revenue shock that compounded with the
    // reputation tier-hop into runaway balance growth. Real broadcast
    // ladders gap by ~2x between tiers, not 3-5x.
    match tier {
        1 => 1.00,
        2 => 0.45,
        3 => 0.20,
        _ => 0.08,
    }
}

/// Position-based TV bonus. Champion: 1.20; top-4: 1.10; top half: 1.00;
/// bottom half: 0.90; relegation zone (bottom three or bottom 15%): 0.80.
/// Mid-table is the neutral baseline.
fn placement_multiplier(position: u16, total_teams: u16) -> f64 {
    if position == 0 || total_teams == 0 {
        return 1.0;
    }
    if position == 1 {
        return 1.20;
    }
    let n = total_teams as f64;
    let p = position as f64;
    let rel = p / n; // 0 (top) → 1 (bottom)
    if p <= 4.0 {
        1.10
    } else if rel <= 0.5 {
        1.00
    } else if p > n - 3.5 || rel >= 0.85 {
        0.80
    } else {
        0.90
    }
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

        // 4. TV, matchday, merchandising, facility costs — from main team reputation
        let main_team = self.teams.main();
        if let Some(team) = main_team {
            let rep = team.reputation.level();
            let league_id = team.league_id;
            let (recent_wins_ratio, league_pos, total_teams) =
                self.compute_team_form_and_position(&ctx);

            // TV: reputation base × country market × league tier × placement.
            // The reputation base is what a tier-1 club earns in a "world-
            // average" market with mid-table placement; tier and placement
            // multipliers fan that out to give relegation strugglers and
            // tier-2 clubs realistic-looking numbers.
            let tv_base = tv_revenue_base(rep);
            let league_tier = league_tier_of(&ctx, league_id);
            let tier_mult = league_tier_multiplier(league_tier);
            let placement_mult = placement_multiplier(league_pos, total_teams);
            let tv_revenue =
                (tv_base as f64 * tv_multiplier as f64 * tier_mult * placement_mult) as i64;
            self.finance.balance.push_income_tv(tv_revenue);
            // Decompose so the UI can show base vs placement separately.
            // The placement bonus is the slice that wouldn't have been paid
            // at neutral mid-table — clamped to non-negative to keep the
            // base/placement story coherent for relegation-band clubs.
            let placement_premium = ((placement_mult - 1.0).max(0.0)
                * tv_base as f64
                * tv_multiplier as f64
                * tier_mult) as i64;
            if placement_premium > 0 {
                self.finance
                    .balance
                    .push_income_tv_placement(placement_premium);
                // Avoid double-counting against `income_tv` total.
                self.finance.balance.income_tv -= placement_premium;
            }

            // Matchday: actual home matches this month × per-match gate.
            let price_level = get_price_level(&ctx);
            let base_attendance = self.facilities.average_attendance as f64;
            let form_mult = self.facilities.dynamic_attendance_multiplier(
                recent_wins_ratio,
                league_pos,
                total_teams,
            ) as f64;
            let stadium_capacity = stadium_capacity_for(rep) as f64;
            let raw_attendance = base_attendance * attendance_factor as f64 * form_mult;
            let attendance = raw_attendance.min(stadium_capacity).max(0.0) as i64;
            let ticket_price = (ticket_base_price(rep) * price_level) as i64;
            let home_matches = self.finance.take_home_match_count() as i64;
            let matchday_revenue = attendance * ticket_price * home_matches;
            if matchday_revenue > 0 {
                self.finance.balance.push_income_matchday(matchday_revenue);
            }

            // Merchandising scales with rep, country market, and price level.
            let merch_base: f64 = match rep {
                ReputationLevel::Elite => 500_000.0,
                ReputationLevel::Continental => 150_000.0,
                ReputationLevel::National => 50_000.0,
                ReputationLevel::Regional => 10_000.0,
                ReputationLevel::Local => 2_000.0,
                ReputationLevel::Amateur => 500.0,
            };
            let merch_revenue = (merch_base * sponsorship_strength as f64 * price_level) as i64;
            self.finance
                .balance
                .push_income_merchandising(merch_revenue);
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
        // marketing, infrastructure. Modelled as a fraction of monthly
        // revenue (the way real-world football clubs' SG&A actually
        // scales) plus a reputation-tier fixed-cost floor for the
        // institutional infrastructure that doesn't shrink in a lean
        // year. The earlier `balance × 0.3%` tax was a wealth check
        // that barely bit at $20M+ balances ($60K/mo on a $3M/mo
        // revenue stream), so income outpaced expenses and balances
        // compounded into rocket-shape growth across the board.
        let monthly_income = self.finance.balance.income;
        let revenue_overhead = ((monthly_income.max(0) as f64) * 0.15) as i64;
        let tier_overhead: i64 = if let Some(team) = main_team {
            match team.reputation.level() {
                ReputationLevel::Elite => 800_000,
                ReputationLevel::Continental => 350_000,
                ReputationLevel::National => 140_000,
                ReputationLevel::Regional => 50_000,
                ReputationLevel::Local => 15_000,
                ReputationLevel::Amateur => 5_000,
            }
        } else {
            0
        };
        let overhead = revenue_overhead + tier_overhead;
        if overhead > 0 {
            self.finance.balance.push_expense_facilities(overhead);
        }

        // 8. Debt interest: only when the club is genuinely in the red.
        // Scales with distress severity — a club drowning in long-term
        // debt pays more than one with a short cash-flow dip.
        let post_balance = self.finance.balance.balance;
        if post_balance < 0 {
            let avg_wages = self.finance.trailing_avg_monthly_wages(date);
            let level = classify_distress(post_balance, avg_wages);
            let rate = match level {
                DistressLevel::None => 0.006,
                DistressLevel::Distress => 0.006,
                DistressLevel::Severe => 0.010,
                DistressLevel::Insolvency => 0.015,
            };
            let interest = ((-post_balance) as f64 * rate) as i64;
            if interest > 0 {
                self.finance.balance.push_expense_debt_interest(interest);
            }
        }

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
        let trailing_income = self.finance.trailing_annual_income(date);
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
    use super::{
        league_tier_multiplier, placement_multiplier, stadium_capacity_for, tv_revenue_base,
    };
    use crate::ReputationLevel;

    #[test]
    fn tv_revenue_base_scales_with_reputation() {
        assert!(
            tv_revenue_base(ReputationLevel::Elite) > tv_revenue_base(ReputationLevel::Continental)
        );
        assert!(
            tv_revenue_base(ReputationLevel::Continental)
                > tv_revenue_base(ReputationLevel::National)
        );
        assert!(tv_revenue_base(ReputationLevel::Amateur) > 0);
    }

    #[test]
    fn league_tier_mult_decays_below_top_flight() {
        assert_eq!(league_tier_multiplier(1), 1.00);
        assert!((league_tier_multiplier(2) - 0.45).abs() < 1e-6);
        assert!((league_tier_multiplier(3) - 0.20).abs() < 1e-6);
        assert!((league_tier_multiplier(4) - 0.08).abs() < 1e-6);
        // Each step down is roughly 2x, not 3-5x — promotion shouldn't
        // be a TV windfall on its own; the reputation tier-hop alone
        // does most of the lifting.
        assert!(league_tier_multiplier(1) > league_tier_multiplier(2));
        assert!(league_tier_multiplier(2) > league_tier_multiplier(3));
        assert!(league_tier_multiplier(3) > league_tier_multiplier(4));
    }

    #[test]
    fn placement_multiplier_handles_table_extremes() {
        assert_eq!(placement_multiplier(0, 0), 1.00); // unknown
        assert_eq!(placement_multiplier(1, 20), 1.20); // champion
        assert_eq!(placement_multiplier(3, 20), 1.10); // top-4
        assert_eq!(placement_multiplier(10, 20), 1.00); // top half
        assert_eq!(placement_multiplier(13, 20), 0.90); // bottom half
        assert_eq!(placement_multiplier(19, 20), 0.80); // relegation zone
    }

    #[test]
    fn stadium_capacity_grows_with_reputation() {
        assert!(
            stadium_capacity_for(ReputationLevel::Elite)
                > stadium_capacity_for(ReputationLevel::Regional)
        );
        assert!(stadium_capacity_for(ReputationLevel::Amateur) >= 100);
    }
}

use crate::club::{classify_distress, DistressLevel};
use crate::context::GlobalContext;
use crate::{ContractBonusType, ReputationLevel};
use super::Club;
use chrono::Datelike;

/// Country price level: scales ticket prices, merchandising etc. by local economy.
/// England 1.5, Colombia 0.4, default 1.0.
fn get_price_level(ctx: &GlobalContext<'_>) -> f64 {
    ctx.country.as_ref()
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
    match tier {
        1 => 1.00,
        2 => 0.28,
        3 => 0.10,
        _ => 0.04,
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
        let tv_multiplier = ctx.country.as_ref()
            .map(|c| c.tv_revenue_multiplier)
            .unwrap_or(1.0);
        let attendance_factor = ctx.country.as_ref()
            .map(|c| c.stadium_attendance_factor)
            .unwrap_or(1.0);
        let sponsorship_strength = ctx.country.as_ref()
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
            self.finance
                .balance
                .push_expense_player_wages(bonus_payout);
        }

        // 2. Staff wages: coaching, medical, scouting staff
        for team in self.teams.iter() {
            let staff_monthly = team.staffs.get_annual_salary() / 12;
            if staff_monthly > 0 {
                self.finance.balance.push_expense_staff_wages(staff_monthly as i64);
            }
        }

        // 3. Sponsorship income
        let sponsorship_income: i64 = self.finance.sponsorship
            .get_sponsorship_incomes(date)
            .iter()
            .map(|c| (c.wage / 12) as i64)
            .sum();
        if sponsorship_income > 0 {
            self.finance.balance.push_income_sponsorship(sponsorship_income);
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
            self.finance.balance.push_income_merchandising(merch_revenue);
        }

        // 5. Amortization: each outstanding transfer purchase contributes
        // its monthly slice as a P&L expense. Cash already left the
        // balance at the upfront purchase, so this only hits `outcome` and
        // the categorised `expense_amortization` bucket.
        self.finance.tick_amortization();

        // 6. Facility maintenance costs
        let facility_cost: i64 = (
            self.facilities.training.to_rating() as i64 +
            self.facilities.youth.to_rating() as i64 +
            self.facilities.academy.to_rating() as i64
        ) * 5_000;
        self.finance.balance.push_expense_facilities(facility_cost);

        // 7. Operating overhead: administration, taxes, community, infrastructure
        let balance = self.finance.balance.balance;
        if balance > 1_000_000 {
            let balance_overhead = (balance as f64 * 0.003) as i64;
            let tier_overhead: i64 = if let Some(team) = main_team {
                match team.reputation.level() {
                    ReputationLevel::Elite => 500_000,
                    ReputationLevel::Continental => 200_000,
                    ReputationLevel::National => 80_000,
                    ReputationLevel::Regional => 30_000,
                    ReputationLevel::Local => 10_000,
                    ReputationLevel::Amateur => 3_000,
                }
            } else {
                0
            };
            self.finance.balance.push_expense_facilities(balance_overhead + tier_overhead);
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

        let _ = club_name;
    }

    /// Returns (recent_wins_ratio, league_position, total_teams) for the
    /// club's main team. Form comes from the last ~5 matches in the team's
    /// `match_history`; league position rides through `ClubContext` —
    /// which the country simulation populates from the live table.
    fn compute_team_form_and_position(
        &self,
        ctx: &GlobalContext<'_>,
    ) -> (f32, u16, u16) {
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

/// True when `today` falls on or past `started`'s month/day in the
/// current calendar year. Used to gate annual loyalty payouts so a
/// Dec-31 contract doesn't accidentally pay a Jan-1 loyalty in the
/// following year — the contract hasn't reached its anniversary yet.
fn has_reached_anniversary(today: chrono::NaiveDate, started: chrono::NaiveDate) -> bool {
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
fn settle_lump_sum_bonuses(club: &mut Club, date: chrono::NaiveDate) -> i64 {
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
                            let new_caps =
                                current_caps.saturating_sub(baseline_caps) as i64;
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
mod helpers_tests {
    use super::{
        league_tier_multiplier, placement_multiplier, stadium_capacity_for, tv_revenue_base,
    };
    use crate::ReputationLevel;

    #[test]
    fn tv_revenue_base_scales_with_reputation() {
        assert!(tv_revenue_base(ReputationLevel::Elite) > tv_revenue_base(ReputationLevel::Continental));
        assert!(
            tv_revenue_base(ReputationLevel::Continental)
                > tv_revenue_base(ReputationLevel::National)
        );
        assert!(tv_revenue_base(ReputationLevel::Amateur) > 0);
    }

    #[test]
    fn league_tier_mult_decays_below_top_flight() {
        assert_eq!(league_tier_multiplier(1), 1.00);
        assert!((league_tier_multiplier(2) - 0.28).abs() < 1e-6);
        assert!((league_tier_multiplier(3) - 0.10).abs() < 1e-6);
        assert!((league_tier_multiplier(4) - 0.04).abs() < 1e-6);
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

//! The club's daily tick.
//!
//! Thin orchestration on purpose: it works out what kind of day this is —
//! ordinary, week-beginning, month-beginning, season-opening — and dispatches
//! to the pass that owns each job. Nothing here decides anything about a
//! player, a budget or a keeper; it states that the day has arrived and lets
//! the owner react.

use crate::Club;
use crate::club::ClubResult;
use crate::club::board::FfpStatus;
use crate::club::news::ClubAffair;
use crate::context::GlobalContext;
use crate::shared::{Currency, CurrencyValue};

use super::boardroom::LeagueStanding;

impl Club {
    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> ClubResult {
        let date = ctx.simulation.date.date();

        let country_economic_factor = ctx
            .country
            .as_ref()
            .map(|c| c.tv_revenue_multiplier)
            .unwrap_or(1.0);
        let country_price_level = ctx.country.as_ref().map(|c| c.price_level).unwrap_or(1.0);
        // League position from country-level context. `league_played` is
        // the count for THIS season — the club's own match history is never
        // truncated at a season turn, so it is the only honest source for
        // how far into a campaign the board is judging.
        let (league_pos, league_sz, league_played, total_matches, league_tier) = ctx
            .club
            .as_ref()
            .map(|c| {
                (
                    c.league_position,
                    c.league_size,
                    c.league_matches_played,
                    c.total_league_matches,
                    c.main_league_tier,
                )
            })
            .unwrap_or((0, 0, 0, 0, 1));

        let mut board_ctx = self.build_board_context(
            country_economic_factor,
            country_price_level,
            league_played,
            date,
        );
        board_ctx.league_position = league_pos;
        board_ctx.league_size = league_sz;
        board_ctx.total_matches = total_matches;
        board_ctx.league_tier = league_tier.max(1);
        // Annualised by funded months: in a world's first year the raw
        // trailing sums cover only the months lived so far, and budgets or
        // debt ratios sized off them read every young club as broke.
        board_ctx.trailing_annual_income = self.finance.estimated_annual_income(date);
        board_ctx.trailing_annual_outcome = self.finance.estimated_annual_outcome(date);
        // …and a figure that exists even before the first month closes, so
        // the ratios whose denominator is a year of revenue fail closed
        // instead of dividing by nothing.
        board_ctx.projected_annual_income =
            self.projected_annual_income(&ctx, board_ctx.league_tier, date);
        board_ctx.ffp_status = if self.finance.is_ffp_breach(date) {
            FfpStatus::Breach
        } else if self.finance.is_ffp_watchlist(date) {
            FfpStatus::Watchlist
        } else {
            FfpStatus::Clean
        };

        // Derived finance signals for the board's component scoring.
        board_ctx.profit_loss_12m =
            board_ctx.trailing_annual_income - board_ctx.trailing_annual_outcome;
        let debt = (-board_ctx.balance).max(0) as f64;
        let revenue = board_ctx.trailing_annual_income.max(1) as f64;
        board_ctx.debt_ratio = (debt / revenue) as f32;

        // League-position-relative distances (top-tier conventions: bottom
        // 3 relegate, top ~5 reach Europe / a playoff spot).
        if league_sz > 0 && league_pos > 0 {
            let relegation_edge = league_sz.saturating_sub(3);
            board_ctx.distance_to_relegation = relegation_edge as i16 - league_pos as i16 + 1;
            let europe_edge: u8 = 5.min(league_sz);
            board_ctx.distance_to_europe_or_playoff = league_pos as i16 - europe_edge as i16;
        }

        // Attendance demand + supporter mood from recent form and standing.
        let win_ratio = self
            .teams
            .main()
            .map(|t| t.match_history.recent_wins_ratio(5))
            .unwrap_or(0.5);
        board_ctx.attendance_ratio = self.facilities.dynamic_attendance_multiplier(
            win_ratio,
            league_pos as u16,
            league_sz as u16,
        );
        let standing = if league_sz > 0 && league_pos > 0 {
            1.0 - (league_pos as f32 / league_sz as f32)
        } else {
            0.5
        };
        board_ctx.supporter_mood = (win_ratio * 0.55 + standing * 0.45).clamp(0.0, 1.0);

        // Build club context with facility data for training/academy + best
        // staff attribute scores so per-player systems can consult them
        // without walking the whole staff list each call.
        let staff_q = self.compute_staff_qualities();

        // Preserve any reputation/league info already injected by the
        // country-level orchestrator (`Country::simulate_clubs`) — without
        // this, a fresh `with_club` here would wipe main-team / league /
        // country reputation before the academy pipeline reads them.
        let preserved = ctx.club.as_ref().cloned();
        let club_ctx = ctx.with_club(self.id, &self.name);
        let club_ctx = {
            let mut c = club_ctx;
            if let Some(ref mut cc) = c.club {
                let mut next = cc
                    .clone()
                    .with_facilities(
                        self.facilities.training.multiplier(),
                        self.facilities.youth.multiplier(),
                        self.facilities.academy.multiplier(),
                        self.facilities.recruitment.multiplier(),
                    )
                    .with_staff_quality(staff_q.medical, staff_q.sports_science, staff_q.youth)
                    .with_coach_scores(
                        staff_q.coach_technical,
                        staff_q.coach_mental,
                        staff_q.coach_fitness,
                        staff_q.coach_goalkeeping,
                    )
                    .with_pathway_reputation(self.academy.pathway_reputation);

                if let Some(prev) = preserved {
                    next = next
                        .with_league_position(
                            prev.league_position,
                            prev.league_size,
                            prev.total_league_matches,
                            prev.league_matches_played,
                        )
                        .with_main_league_tier(prev.main_league_tier)
                        .with_reputations(
                            prev.main_team_reputation,
                            prev.main_team_world_reputation,
                            prev.league_reputation,
                            prev.country_reputation,
                        );
                }

                *cc = next;
            }
            c
        };

        // The four table numbers the manager's situation needs, lifted
        // out of the club context so the situated think below does not
        // hold a borrow of the club's own name for its duration.
        let table = club_ctx
            .club
            .as_ref()
            .map(LeagueStanding::from_context)
            .unwrap_or_default();

        let mut result = ClubResult::new(
            self.id,
            self.finance.simulate(ctx.with_finance()),
            self.teams.simulate(club_ctx.clone()),
            self.board.simulate(ctx.with_board_data(board_ctx)),
            self.academy.simulate(club_ctx.clone()),
        );

        // Intake day. The academy takes boys in on one morning a year
        // and then the only evidence is a longer squad list, so the
        // club writes the day down while it still has a number.
        if result.academy.intake > 0 {
            self.record_affair(
                ClubAffair::AcademyIntake {
                    count: result.academy.intake,
                    golden: result.academy.golden_intake,
                },
                date,
            );
        }

        if ctx.simulation.is_week_beginning() {
            if self.teams.ensure_coach_state(date) {
                self.open_manager_review_window(date);
            }
            self.teams.update_all_impressions(date);

            // The manager's situated think. Runs after the coach state
            // is refreshed so the dressing-room reading it takes is
            // this week's rather than last week's.
            self.run_manager_mind(date, table);

            // Weekly: move loan returnees from main to reserve
            self.move_loan_returns_to_reserve(date);

            // Weekly: rebalance players across all teams
            self.rebalance_squads(date);

            // Weekly: a youth squad that cannot field eleven players is
            // an emergency, not something to leave until the season
            // turns over. Runs after the rebalance so it only counts a
            // hole the club's own promotions could not close, and is
            // bounded to one rescue a month inside the academy.
            let emergency_callups = self.process_youth_emergency_callups(
                date,
                ctx.country.as_ref().map(|c| c.code.as_str()).unwrap_or(""),
            );
            if !emergency_callups.is_empty() {
                result.academy_transfers.extend(emergency_callups);
            }

            // Weekly: hand pro contracts to youth players who've earned
            // them on form (also makes them visible to the loan market).
            self.review_youth_contracts(date);
        } else {
            self.teams.manage_critical_squad_moves(date);
        }

        if ctx.simulation.is_month_beginning() {
            if self.teams.ensure_coach_state(date) {
                self.open_manager_review_window(date);
            }
            // Offer proactive contract renewals. Pass the chairman's wage
            // cap and league prestige so the renewal pass sizes its offers
            // correctly.
            let wage_budget = self
                .finance
                .wage_budget
                .as_ref()
                .map(|b| b.amount.max(0.0) as u32);
            // Use the team's world reputation as a proxy for league prestige
            // — `CountryContext` doesn't carry the league table here, and the
            // two correlate strongly (top-rep teams play in top-rep leagues).
            let league_rep = self
                .teams
                .main()
                .map(|t| t.reputation.world)
                .unwrap_or(5_000);
            self.teams
                .run_contract_renewals_with_budget(date, wage_budget, league_rep);

            // Monthly: process wages (annual salary / 12) and income
            self.process_monthly_finances(ctx.clone());

            // Monthly: re-derive the live budgets from the board's mandate
            // and the club's current standing. Must run after the finance
            // pass so it sees this month's distress and debt classification.
            self.recompute_budgets();

            // Monthly: audit squad utilization and list underused players
            self.audit_squad_utilization(date);

            // Monthly: the goalkeeping department reviews the whole keeper
            // room — first team, reserves and academy together, because
            // there is only one shirt and the queue for it runs across every
            // squad the club owns. Runs after the utilization audit so the
            // pecking order it declares is the last word on a keeper's
            // standing that month.
            self.review_goalkeeping_department(date);
        }

        // Season start: reset player states and graduate academy players
        let season = ctx
            .country
            .as_ref()
            .map(|c| c.season_dates)
            .unwrap_or_default();
        if ctx.simulation.is_season_start(&season) {
            // Sync budgets from board targets to finance system
            if let Some(targets) = &self.board.season_targets {
                self.finance.transfer_budget = Some(CurrencyValue {
                    amount: targets.adjusted_transfer_budget() as f64,
                    currency: Currency::Usd,
                });
                self.finance.wage_budget = Some(CurrencyValue {
                    amount: targets.adjusted_wage_budget() as f64,
                    currency: Currency::Usd,
                });
            }
            // A new campaign's trading starts from nothing, and so do the
            // board's one-shot guards on the money it may move.
            self.finance.season_fees.reset();
            self.board.budget_moves.on_new_season();

            self.reset_for_new_season();
            let country_code = ctx.country.as_ref().map(|c| c.code.as_str()).unwrap_or("");
            let (academy_transfers, released_players) =
                self.process_academy_graduations(date, country_code);
            // Graduation day as one morning rather than as a handful of
            // separate free arrivals. The market desk already reports
            // each boy individually; this is the piece about the year
            // group, which is what a local readership turns up for.
            if !academy_transfers.is_empty() {
                self.record_affair(
                    ClubAffair::AcademyGraduationBatch {
                        count: academy_transfers.len() as u16,
                    },
                    date,
                );
            }
            result.academy_transfers = academy_transfers;
            result.academy_released_players = released_players;
            self.trim_positional_surplus(date);
        }

        result
    }

    /// A new season opens. Every player reports to pre-season; what that does
    /// to him is his own business — see [`crate::Player::on_pre_season`].
    fn reset_for_new_season(&mut self) {
        for team in self.teams.teams.iter_mut() {
            for player in team.players.players.iter_mut() {
                player.on_pre_season();
            }
        }
    }
}

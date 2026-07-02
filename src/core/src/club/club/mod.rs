mod finances;
mod graduation;
mod squad;
mod utilization;

use graduation::graduation_salary;

use crate::club::academy::ClubAcademy;
use crate::club::board::{BoardContext, ClubBoard, FfpStatus};
use crate::club::facilities::ClubFacilities;
use crate::club::status::ClubStatus;
use crate::club::{ClubFinances, ClubResult, StaffPosition};
use crate::context::GlobalContext;
use crate::shared::{Currency, CurrencyValue, Location};
use crate::transfers::pipeline::ClubTransferPlan;
use crate::utils::DateUtils;
use crate::{ReputationLevel, TeamCollection, TeamType};
use chrono::NaiveDate;

#[derive(Debug, Clone, PartialEq)]
pub enum ClubPhilosophy {
    /// Develop youth and sell for profit (Ajax, Benfica, Dortmund)
    DevelopAndSell,
    /// Sign established players, compete now (PSG, Chelsea, Man City)
    SignToCompete,
    /// Loan-heavy strategy, minimal spending (smaller clubs)
    LoanFocused,
    /// Balanced approach (most clubs)
    Balanced,
}

#[derive(Debug, Clone)]
pub struct ClubColors {
    pub background: String,
    pub foreground: String,
}

impl Default for ClubColors {
    fn default() -> Self {
        ClubColors {
            background: "#1e272d".to_string(),
            foreground: "#ffffff".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Club {
    pub id: u32,
    pub name: String,

    pub location: Location,

    pub board: ClubBoard,

    pub finance: ClubFinances,

    pub status: ClubStatus,

    pub academy: ClubAcademy,

    pub colors: ClubColors,

    pub teams: TeamCollection,

    pub transfer_plan: ClubTransferPlan,

    pub philosophy: ClubPhilosophy,

    pub facilities: ClubFacilities,

    pub rivals: Vec<u32>,
}

/// Aggregated best staff attribute scores across all teams at the club.
/// Precomputed once per club-tick so per-player systems can read via
/// ClubContext without walking the staff list.
pub(crate) struct StaffQualitySnapshot {
    pub medical: f32,
    pub sports_science: f32,
    pub youth: f32,
    pub coach_technical: u8,
    pub coach_mental: u8,
    pub coach_fitness: u8,
    pub coach_goalkeeping: u8,
}

impl Club {
    pub fn is_rival(&self, other_club_id: u32) -> bool {
        self.rivals.contains(&other_club_id)
    }

    pub fn new(
        id: u32,
        name: String,
        location: Location,
        finance: ClubFinances,
        academy: ClubAcademy,
        status: ClubStatus,
        colors: ClubColors,
        teams: TeamCollection,
        facilities: ClubFacilities,
    ) -> Self {
        let philosophy = Self::determine_philosophy(&teams);

        Club {
            id,
            name,
            location,
            finance,
            status,
            academy,
            colors,
            board: ClubBoard::new(),
            teams,
            transfer_plan: ClubTransferPlan::new(),
            philosophy,
            facilities,
            rivals: Vec::new(),
        }
    }

    fn compute_staff_qualities(&self) -> StaffQualitySnapshot {
        let mut best_physio: u8 = 0;
        let mut best_sports_science: u8 = 0;
        let mut best_wwy: u8 = 0;
        let mut best_technical: u8 = 0;
        let mut best_mental: u8 = 0;
        let mut best_fitness: u8 = 0;
        let mut best_goalkeeping: u8 = 0;

        for team in self.teams.iter() {
            for staff in team.staffs.iter() {
                let medical = &staff.staff_attributes.medical;
                if medical.physiotherapy > best_physio {
                    best_physio = medical.physiotherapy;
                }
                if medical.sports_science > best_sports_science {
                    best_sports_science = medical.sports_science;
                }
                let coaching = &staff.staff_attributes.coaching;
                if coaching.working_with_youngsters > best_wwy {
                    best_wwy = coaching.working_with_youngsters;
                }
                if coaching.technical > best_technical {
                    best_technical = coaching.technical;
                }
                if coaching.mental > best_mental {
                    best_mental = coaching.mental;
                }
                if coaching.fitness > best_fitness {
                    best_fitness = coaching.fitness;
                }
                let gk = &staff.staff_attributes.goalkeeping;
                // Average the 3 GK coaching attributes as a single coach score
                let gk_avg =
                    ((gk.shot_stopping as u16 + gk.handling as u16 + gk.distribution as u16) / 3)
                        as u8;
                if gk_avg > best_goalkeeping {
                    best_goalkeeping = gk_avg;
                }
            }
        }

        StaffQualitySnapshot {
            medical: (best_physio as f32 / 20.0).clamp(0.0, 1.0),
            sports_science: (best_sports_science as f32 / 20.0).clamp(0.0, 1.0),
            youth: (best_wwy as f32 / 20.0).clamp(0.0, 1.0),
            coach_technical: best_technical,
            coach_mental: best_mental,
            coach_fitness: best_fitness,
            coach_goalkeeping: best_goalkeeping,
        }
    }

    fn determine_philosophy(teams: &TeamCollection) -> ClubPhilosophy {
        let rep_level = teams
            .main()
            .map(|t| t.reputation.level())
            .unwrap_or(ReputationLevel::Amateur);

        match rep_level {
            ReputationLevel::Elite => ClubPhilosophy::SignToCompete,
            ReputationLevel::Continental => ClubPhilosophy::Balanced,
            ReputationLevel::National => ClubPhilosophy::Balanced,
            _ => ClubPhilosophy::LoanFocused,
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> ClubResult {
        let date = ctx.simulation.date.date();

        let country_economic_factor = ctx
            .country
            .as_ref()
            .map(|c| c.tv_revenue_multiplier)
            .unwrap_or(1.0);
        let country_price_level = ctx.country.as_ref().map(|c| c.price_level).unwrap_or(1.0);
        // League position from country-level context
        let (league_pos, league_sz, total_matches, league_tier) = ctx
            .club
            .as_ref()
            .map(|c| {
                (
                    c.league_position,
                    c.league_size,
                    c.total_league_matches,
                    c.main_league_tier,
                )
            })
            .unwrap_or((0, 0, 0, 1));

        let mut board_ctx =
            self.build_board_context(country_economic_factor, country_price_level, date);
        board_ctx.league_position = league_pos;
        board_ctx.league_size = league_sz;
        board_ctx.total_matches = total_matches;
        board_ctx.league_tier = league_tier.max(1);
        board_ctx.trailing_annual_income = self.finance.trailing_annual_income(date);
        board_ctx.trailing_annual_outcome = self.finance.trailing_annual_outcome(date);
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

        let mut result = ClubResult::new(
            self.id,
            self.finance.simulate(ctx.with_finance()),
            self.teams.simulate(club_ctx.clone()),
            self.board.simulate(ctx.with_board_data(board_ctx)),
            self.academy.simulate(club_ctx.clone()),
        );

        if ctx.simulation.is_week_beginning() {
            self.teams.ensure_coach_state(date);
            self.teams.update_all_impressions(date);

            // Weekly: move loan returnees from main to reserve
            self.move_loan_returns_to_reserve(date);

            // Weekly: rebalance players across all teams
            self.rebalance_squads(date);

            // Weekly: hand pro contracts to youth players who've earned
            // them on form (also makes them visible to the loan market).
            self.review_youth_contracts(date);
        } else {
            self.teams.manage_critical_squad_moves(date);
        }

        if ctx.simulation.is_month_beginning() {
            self.teams.ensure_coach_state(date);
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

            // Monthly: audit squad utilization and list underused players
            self.audit_squad_utilization(date);
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
                    amount: targets.transfer_budget as f64,
                    currency: Currency::Usd,
                });
                self.finance.wage_budget = Some(CurrencyValue {
                    amount: targets.wage_budget as f64,
                    currency: Currency::Usd,
                });
            }

            self.process_pre_season_reset();
            let country_code = ctx.country.as_ref().map(|c| c.code.as_str()).unwrap_or("");
            let (academy_transfers, released_players) =
                self.process_academy_graduations(date, country_code);
            result.academy_transfers = academy_transfers;
            result.academy_released_players = released_players;
            self.trim_positional_surplus(date);
        }

        result
    }

    fn build_board_context(
        &self,
        country_economic_factor: f32,
        country_price_level: f32,
        date: NaiveDate,
    ) -> BoardContext {
        let main_team = self.teams.main();

        let main_squad_size = main_team.map(|t| t.players.len()).unwrap_or(0);

        let reserve_squad_size: usize = self
            .teams
            .iter()
            .filter(|t| t.team_type != TeamType::Main)
            .map(|t| t.players.len())
            .sum();

        let total_annual_wages: u32 = self.teams.iter().map(|t| t.get_annual_salary()).sum();

        let reputation_score = main_team
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.0);

        // Recent form from match history (last 5 matches)
        let (recent_wins, _draws, recent_losses) = main_team
            .map(|t| t.match_history.recent_results(5))
            .unwrap_or((0, 0, 0));
        let recent_goal_difference = main_team
            .map(|t| {
                t.match_history
                    .items()
                    .iter()
                    .rev()
                    .take(5)
                    .map(|m| m.score.0.get() as i16 - m.score.1.get() as i16)
                    .sum()
            })
            .unwrap_or(0);

        let matches_played = main_team
            .map(|t| t.match_history.items().len().min(255) as u8)
            .unwrap_or(0);

        // Average squad ability
        let avg_squad_ability = main_team
            .map(|t| t.players.current_ability_avg())
            .unwrap_or(0);

        let main_tactic = main_team
            .and_then(|t| t.tactics.as_ref())
            .map(|tac| tac.tactic_type);
        let wage_budget_usage = self
            .finance
            .wage_budget
            .as_ref()
            .map(|b| {
                if b.amount <= 0.0 {
                    0.0
                } else {
                    total_annual_wages as f32 / b.amount as f32
                }
            })
            .unwrap_or(0.0);

        // Full-season points-per-match and goal difference from the match
        // history (score.0 = us, score.1 = them).
        let (points_per_match, goal_difference) = main_team
            .map(|t| {
                let items = t.match_history.items();
                if items.is_empty() {
                    return (0.0f32, 0i16);
                }
                let mut points = 0u32;
                let mut gd = 0i16;
                for m in items {
                    let us = m.score.0.get() as i16;
                    let them = m.score.1.get() as i16;
                    gd += us - them;
                    if us > them {
                        points += 3;
                    } else if us == them {
                        points += 1;
                    }
                }
                (points as f32 / items.len() as f32, gd)
            })
            .unwrap_or((0.0, 0));

        // Squad age profile, youth share, injury crisis, and key-player
        // unrest from the main squad. `u21_minutes_share` is approximated
        // by the U21 headcount share (a true minutes figure isn't tracked
        // at this layer yet).
        let (squad_avg_age, u21_minutes_share, injury_crisis_score, key_player_unrest_count) =
            main_team
                .map(|t| {
                    let players = t.players.players();
                    let n = players.len();
                    if n == 0 {
                        return (0u8, 0.0f32, 0.0f32, 0u8);
                    }
                    let mut age_sum = 0u32;
                    let mut u21 = 0u32;
                    let mut injured = 0u32;
                    let mut unrest = 0u32;
                    for p in &players {
                        let age = DateUtils::age(p.birth_date, date);
                        age_sum += age as u32;
                        if age <= 21 {
                            u21 += 1;
                        }
                        if p.player_attributes.is_injured {
                            injured += 1;
                        }
                        if p.happiness().morale < 35.0 {
                            unrest += 1;
                        }
                    }
                    (
                        (age_sum / n as u32) as u8,
                        u21 as f32 / n as f32,
                        injured as f32 / n as f32,
                        unrest.min(u8::MAX as u32) as u8,
                    )
                })
                .unwrap_or((0, 0.0, 0.0, 0));

        let manager_contract_months_left = main_team
            .and_then(|t| t.staffs.find_by_position(StaffPosition::Manager))
            .and_then(|s| s.contract.as_ref())
            .map(|c| ((c.expired - date).num_days() / 30).max(0) as i32)
            .unwrap_or(0);

        BoardContext {
            balance: self.finance.balance.balance,
            total_annual_wages,
            reputation_score,
            main_squad_size,
            reserve_squad_size,
            country_economic_factor,
            country_price_level,
            trailing_annual_income: 0,
            trailing_annual_outcome: 0,
            ffp_status: FfpStatus::Clean,
            league_position: 0,
            league_size: 0,
            recent_wins,
            recent_losses,
            recent_goal_difference,
            matches_played,
            total_matches: 0,
            avg_squad_ability,
            squad_avg_age,
            wage_budget_usage,
            main_tactic,
            league_tier: 1,
            points_per_match,
            goal_difference,
            distance_to_relegation: 0,
            distance_to_europe_or_playoff: 0,
            attendance_ratio: 1.0,
            supporter_mood: 0.5,
            transfer_budget_usage: 0.0,
            debt_ratio: 0.0,
            profit_loss_12m: 0,
            academy_graduates_this_season: 0,
            u21_minutes_share,
            injury_crisis_score,
            manager_contract_months_left,
            key_player_unrest_count,
            facility_training: self.facilities.training.clone(),
            facility_youth: self.facilities.youth.clone(),
            facility_academy: self.facilities.academy.clone(),
            facility_recruitment: self.facilities.recruitment.clone(),
        }
    }
}

//! Assembling what the board is told each tick.
//!
//! The board reasons about finance, results, squad profile and facilities at
//! once, and the club is the only place that holds all four. Everything here
//! is a read: [`BoardContext`] is a snapshot handed to
//! [`crate::club::board::ClubBoard::simulate`], never a channel back into the
//! club.

use chrono::NaiveDate;

use crate::club::board::{BoardContext, FfpStatus};
use crate::utils::DateUtils;
use crate::{Club, StaffPosition, TeamType};

/// Aggregated best staff attribute scores across all teams at the club.
/// Precomputed once per club-tick so per-player systems can read via
/// ClubContext without walking the staff list.
pub(in crate::club::core) struct StaffQualitySnapshot {
    pub medical: f32,
    pub sports_science: f32,
    pub youth: f32,
    pub coach_technical: u8,
    pub coach_mental: u8,
    pub coach_fitness: u8,
    pub coach_goalkeeping: u8,
}

impl Club {
    pub(in crate::club::core) fn compute_staff_qualities(&self) -> StaffQualitySnapshot {
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

    /// Matches of form the board reads as "recent".
    const FORM_WINDOW: usize = 5;

    /// Assemble what the board is told today.
    ///
    /// `league_matches_played` comes from the country-level orchestrator and
    /// is the count for THIS season — the one figure the club cannot derive,
    /// because its own match history is never truncated at a season turn.
    /// Every sporting number below is scoped to it.
    pub(in crate::club::core) fn build_board_context(
        &self,
        country_economic_factor: f32,
        country_price_level: f32,
        league_matches_played: u8,
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

        // Form, points and goals — all of them this season's, off one walk
        // of the slice the league's own count delimits.
        let season = main_team
            .map(|t| {
                t.match_history
                    .season_record(league_matches_played as usize, Self::FORM_WINDOW)
            })
            .unwrap_or_default();

        // Average squad ability
        let avg_squad_ability = main_team
            .map(|t| t.players.current_ability_avg())
            .unwrap_or(0);

        let main_tactic = main_team
            .and_then(|t| t.tactics.as_ref())
            .map(|tac| tac.tactic_type);
        // Both usage ratios are measured against the board's own MANDATE,
        // not against the live budget.
        //
        // The live wage budget is itself derived from the mandate and then
        // throttled by distress, so dividing the bill by it made the ratio
        // partly a measurement of the board's own last decision: a club put
        // under a distress throttle read as overspending purely because the
        // denominator had shrunk.
        let mandate = self.board.season_targets.as_ref();
        let wage_budget_usage = mandate
            .map(|t| t.adjusted_wage_budget())
            .filter(|m| *m > 0)
            .map(|m| total_annual_wages as f32 / m as f32)
            .unwrap_or(0.0);
        // …and what the manager has actually spent of his chest. Hard-coded
        // to zero until now, so the financial component score could never
        // see transfer spending at all.
        let transfer_budget_usage = mandate
            .map(|t| {
                self.finance
                    .season_fees
                    .usage_against(t.adjusted_transfer_budget().min(i32::MAX as i64) as i32)
            })
            .unwrap_or(0.0);

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

        let manager_contract = main_team
            .and_then(|t| t.staffs.find_by_position(StaffPosition::Manager))
            .and_then(|s| s.contract.as_ref());
        let manager_contract_months_left = manager_contract
            .map(|c| ((c.expired - date).num_days() / 30).max(0) as i32)
            .unwrap_or(0);
        let manager_annual_salary = manager_contract.map(|c| c.salary).unwrap_or(0);

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
            projected_annual_income: 0,
            ffp_status: FfpStatus::Clean,
            debt_standing: self.finance.debt.standing,
            league_position: 0,
            league_size: 0,
            recent_wins: season.recent_wins,
            recent_losses: season.recent_losses,
            recent_goal_difference: season.recent_goal_difference,
            matches_played: season.played,
            total_matches: 0,
            avg_squad_ability,
            squad_avg_age,
            wage_budget_usage,
            main_tactic,
            league_tier: 1,
            points_per_match: season.points_per_match,
            goal_difference: season.goal_difference,
            distance_to_relegation: 0,
            distance_to_europe_or_playoff: 0,
            attendance_ratio: 1.0,
            supporter_mood: 0.5,
            transfer_budget_usage,
            debt_ratio: 0.0,
            profit_loss_12m: 0,
            academy_graduates_this_season: 0,
            u21_minutes_share,
            injury_crisis_score,
            manager_contract_months_left,
            manager_annual_salary,
            fees_received_this_season: self.finance.season_fees.received,
            key_player_unrest_count,
            facility_training: self.facilities.training.clone(),
            facility_youth: self.facilities.youth.clone(),
            facility_academy: self.facilities.academy.clone(),
            facility_recruitment: self.facilities.recruitment.clone(),
        }
    }
}

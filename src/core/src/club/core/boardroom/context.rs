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

    pub(in crate::club::core) fn build_board_context(
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
            projected_annual_income: 0,
            ffp_status: FfpStatus::Clean,
            debt_standing: self.finance.debt.standing,
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

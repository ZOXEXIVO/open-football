use crate::club::player::player::Player;
use crate::club::staff::staff::Staff;
use crate::club::team::coach_perception::{CoachDecisionState, RecentMoveType};
use crate::context::GlobalContext;
use crate::{PlayerStatusType, Team};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::{execute_moves, record_player_decisions, record_moves};

// ─── AI response types ─────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct AiDemotionAdvice {
    demotions: Vec<AiDemotionDecision>,
}

#[derive(Deserialize, Debug)]
struct AiDemotionDecision {
    player_id: u32,
    reason: String,
}

// ─── AI prompt data types ──────────────────────────────────────────

#[derive(Serialize)]
struct DemotionQueryLlm {
    #[serde(rename = "s")]
    staff: serde_json::Value,
    #[serde(rename = "sl")]
    staff_legend: serde_json::Value,
    #[serde(rename = "pl")]
    player_legend: serde_json::Value,
    #[serde(rename = "t")]
    teams: Vec<TeamPlayersLlm>,
}

#[derive(Serialize)]
struct TeamPlayersLlm {
    #[serde(rename = "l")]
    label: String,
    #[serde(rename = "ti")]
    team_index: usize,
    #[serde(rename = "cnt")]
    player_count: usize,
    #[serde(rename = "p")]
    players: Vec<serde_json::Value>,
}

// ─── Public API ────────────────────────────────────────────────────

pub struct SquadDemotion;

impl SquadDemotion {
    /// AI-driven demotion decisions. Skips if AI is unavailable.
    pub fn manage(
        ctx: &GlobalContext<'_>,
        teams: &mut [Team],
        coach_state: &mut Option<CoachDecisionState>,
        main_idx: usize,
        reserve_idx: Option<usize>,
        date: NaiveDate,
    ) {
        if !ctx.ai_enabled() {
            return;
        }

        let reserve_idx = match reserve_idx {
            Some(idx) => idx,
            None => return,
        };

        if teams[main_idx].players.players.is_empty() {
            return;
        }

        let team_indices = Self::collect_team_indices(main_idx, reserve_idx);
        let query = Self::build_prompt(teams, main_idx, &team_indices);
        let format = Self::response_format();

        let advice: AiDemotionAdvice = match ctx.ai(query, format) {
            Some(a) => a,
            None => return,
        };

        Self::execute_advice(teams, coach_state, main_idx, reserve_idx, &team_indices, &advice, date);
    }

    /// Mandatory administrative demotions: Lst and Loa status players.
    /// These are deterministic and bypass AI — not coaching decisions.
    pub fn identify_administrative_demotions(main_team: &Team) -> Vec<u32> {
        main_team
            .players
            .players
            .iter()
            .filter_map(|player| {
                let statuses = player.statuses.get();
                if statuses.contains(&PlayerStatusType::Lst)
                    || statuses.contains(&PlayerStatusType::Loa)
                {
                    Some(player.id)
                } else {
                    None
                }
            })
            .collect()
    }

    // ─── Prompt building ──────────────────────────────────────────

    fn collect_team_indices(main_idx: usize, reserve_idx: usize) -> Vec<(usize, &'static str)> {
        vec![(main_idx, "Main Team"), (reserve_idx, "Reserve Team")]
    }

    fn build_prompt(
        teams: &[Team],
        main_idx: usize,
        team_indices: &[(usize, &str)],
    ) -> String {
        let staff_data = teams[main_idx].staffs.head_coach().as_llm();
        let data_json = Self::build_data_json(teams, team_indices, &staff_data);
        let teams_section = Self::build_teams_section(teams, team_indices);
        let previous_decisions_section = Self::build_previous_decisions(teams, team_indices);

        format!(
            include_str!("prompt.md"),
            staff_legend = Staff::llm_legend(),
            staff_data = staff_data,
            teams_section = teams_section,
            previous_decisions_section = previous_decisions_section,
            data_json = data_json,
        )
    }

    fn build_data_json(
        teams: &[Team],
        team_indices: &[(usize, &str)],
        staff_data: &str,
    ) -> String {
        let staff_json: serde_json::Value = serde_json::from_str(staff_data).unwrap();
        let staff_legend_json: serde_json::Value =
            serde_json::from_str(Staff::llm_legend()).unwrap();
        let player_legend_json: serde_json::Value =
            serde_json::from_str(Player::llm_legend()).unwrap();

        let squad_teams: Vec<TeamPlayersLlm> = team_indices
            .iter()
            .map(|&(idx, label)| {
                let team = &teams[idx];
                let head_coach = team.staffs.head_coach();
                let players: Vec<serde_json::Value> = team
                    .players
                    .players
                    .iter()
                    .map(|p| serde_json::from_str(&p.as_llm(head_coach)).unwrap())
                    .collect();
                TeamPlayersLlm {
                    label: label.to_string(),
                    team_index: idx,
                    player_count: players.len(),
                    players,
                }
            })
            .collect();

        let query_data = DemotionQueryLlm {
            staff: staff_json,
            staff_legend: staff_legend_json,
            player_legend: player_legend_json,
            teams: squad_teams,
        };

        if cfg!(debug_assertions) {
            serde_json::to_string_pretty(&query_data).unwrap()
        } else {
            serde_json::to_string(&query_data).unwrap()
        }
    }

    fn build_teams_section(teams: &[Team], team_indices: &[(usize, &str)]) -> String {
        team_indices
            .iter()
            .map(|&(idx, label)| {
                format!(
                    "team_index={}, type={}, players={}",
                    idx, label, teams[idx].players.players.len()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn build_previous_decisions(teams: &[Team], team_indices: &[(usize, &str)]) -> String {
        let mut decisions = String::new();
        for &(idx, _) in team_indices {
            for player in &teams[idx].players.players {
                for d in &player.decision_history.items {
                    decisions.push_str(&format!(
                        "id={},action={},reason={},date={}\n",
                        player.id, d.decision, d.movement, d.date.format("%Y-%m-%d")
                    ));
                }
            }
        }

        if decisions.is_empty() {
            String::new()
        } else {
            format!("\n[PREVIOUS DECISIONS]\n{}", decisions)
        }
    }

    fn response_format() -> String {
        r#"Respond ONLY with JSON: {"demotions":[{"player_id":123,"reason":"..."}]}"#.to_string()
    }

    // ─── Advice execution ─────────────────────────────────────────

    fn execute_advice(
        teams: &mut [Team],
        coach_state: &mut Option<CoachDecisionState>,
        main_idx: usize,
        reserve_idx: usize,
        team_indices: &[(usize, &str)],
        advice: &AiDemotionAdvice,
        date: NaiveDate,
    ) {
        let coach_name = teams[main_idx].staffs.head_coach().full_name.to_string();
        let max_age = teams[reserve_idx].team_type.max_age();
        let mut any_move = false;

        for decision in &advice.demotions {
            if !Self::is_valid_demotion(teams, main_idx, team_indices, decision.player_id, max_age, date) {
                continue;
            }

            execute_moves(teams, main_idx, reserve_idx, &[decision.player_id]);
            record_player_decisions(
                teams, main_idx, reserve_idx,
                &[decision.player_id], date, &coach_name, &decision.reason,
            );
            record_moves(coach_state, &[decision.player_id], RecentMoveType::DemotedToReserves, date);
            any_move = true;
        }

        if any_move {
            if let Some(state) = coach_state {
                state.weeks_since_last_change = 0;
            }
        }
    }

    fn is_valid_demotion(
        teams: &[Team],
        main_idx: usize,
        _team_indices: &[(usize, &str)],
        player_id: u32,
        max_age: Option<u8>,
        date: NaiveDate,
    ) -> bool {
        let player = match teams[main_idx].players.players.iter().find(|p| p.id == player_id) {
            Some(p) => p,
            None => return false,
        };

        // Respect reserve team age limit
        if let Some(max) = max_age {
            if crate::utils::DateUtils::age(player.birth_date, date) > max {
                return false;
            }
        }

        true
    }
}

use crate::club::player::player::Player;
use crate::club::staff::staff::Staff;
use crate::club::team::coach_perception::{CoachDecisionState, RecentMoveType};
use crate::context::GlobalContext;
use crate::Team;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::{execute_moves, record_player_decisions, record_moves};

// ─── AI prompt data types ──────────────────────────────────────────

#[derive(Serialize)]
struct SquadQueryLlm {
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
    #[serde(rename = "p")]
    players: Vec<serde_json::Value>,
}

// ─── AI response types ─────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct AiSquadAdvice {
    moves: Vec<AiSquadMove>,
}

#[derive(Deserialize, Debug)]
struct AiSquadMove {
    player_id: u32,
    from_team_index: usize,
    to_team_index: usize,
    reason: String,
}

// ─── Public API ────────────────────────────────────────────────────

pub struct SquadComposition;

impl SquadComposition {
    /// Weekly AI-driven squad review (demotions, recalls, youth promotions).
    pub fn manage(
        ctx: &GlobalContext<'_>,
        teams: &mut [Team],
        coach_state: &mut Option<CoachDecisionState>,
        main_idx: usize,
        reserve_idx: Option<usize>,
        youth_idx: Option<usize>,
        date: NaiveDate,
    ) {
        if !ctx.ai_enabled() {
            return;
        }

        let team_indices: Vec<(usize, &str)> = {
            let mut v = vec![(main_idx, "Main Team")];
            if let Some(idx) = reserve_idx { v.push((idx, "Reserve Team")); }
            if let Some(idx) = youth_idx { v.push((idx, "Youth Team")); }
            v
        };

        let main_coach = teams[main_idx].staffs.head_coach();
        let staff_data = main_coach.as_llm();
        let staff_json: serde_json::Value = serde_json::from_str(&staff_data).unwrap();
        let staff_legend_json: serde_json::Value = serde_json::from_str(Staff::llm_legend()).unwrap();
        let player_legend_json: serde_json::Value = serde_json::from_str(Player::llm_legend()).unwrap();

        let squad_teams: Vec<TeamPlayersLlm> = team_indices.iter().map(|&(idx, label)| {
            let team = &teams[idx];
            let head_coach = team.staffs.head_coach();
            let players: Vec<serde_json::Value> = team.players.players.iter()
                .map(|p| serde_json::from_str(&p.as_llm(head_coach)).unwrap())
                .collect();
            TeamPlayersLlm {
                label: label.to_string(),
                team_index: idx,
                players,
            }
        }).collect();

        let squad_data = SquadQueryLlm {
            staff: staff_json,
            staff_legend: staff_legend_json,
            player_legend: player_legend_json,
            teams: squad_teams,
        };

        let data_json = if cfg!(debug_assertions) {
            serde_json::to_string_pretty(&squad_data).unwrap()
        } else {
            serde_json::to_string(&squad_data).unwrap()
        };

        // Collect previous moves from decision history
        let mut previous_moves = String::new();
        for &(idx, _) in &team_indices {
            for player in &teams[idx].players.players {
                for d in &player.decision_history.items {
                    previous_moves.push_str(&format!(
                        "id={},from_team={},reason={},date={}\n",
                        player.id, idx, d.decision, d.date.format("%Y-%m-%d")
                    ));
                }
            }
        }

        let previous_moves_section = if previous_moves.is_empty() {
            String::new()
        } else {
            format!("\n[PREVIOUS MOVES]\n{}", previous_moves)
        };

        // Build teams description
        let teams_section: String = team_indices.iter().map(|&(idx, _)| {
            let team = &teams[idx];
            let type_name = match team.team_type {
                crate::TeamType::Main => "Main",
                crate::TeamType::B => "Reserve",
                crate::TeamType::U18 => "Under 18s",
                crate::TeamType::U19 => "Under 19s",
                crate::TeamType::U20 => "Under 20s",
                crate::TeamType::U21 => "Under 21s",
                crate::TeamType::U23 => "Under 23s",
            };
            format!("team_index={}, type={}", idx, type_name)
        }).collect::<Vec<_>>().join("\n");

        let query = format!(
            include_str!("prompt.md"),
            staff_legend = Staff::llm_legend(),
            staff_data = staff_data,
            teams_section = teams_section,
            previous_moves_section = previous_moves_section,
            data_json = data_json,
        );

        let format = String::from(r#"Respond ONLY with JSON: {"moves":[{"player_id":123,"from_team_index":0,"to_team_index":1, reason: "Describe move reason"}]}"#);

        let advice: AiSquadAdvice = match ctx.ai(query, format) {
            Some(a) => a,
            None => return,
        };

        let valid_indices: Vec<usize> = team_indices.iter().map(|(idx, _)| *idx).collect();
        let coach_name = teams[main_idx].staffs.head_coach().full_name.to_string();
        let mut any_move = false;

        for m in &advice.moves {
            if !valid_indices.contains(&m.from_team_index) || !valid_indices.contains(&m.to_team_index) {
                continue;
            }
            if m.from_team_index == m.to_team_index {
                continue;
            }
            let exists = teams[m.from_team_index].players.players.iter().any(|p| p.id == m.player_id);
            if !exists {
                continue;
            }

            execute_moves(teams, m.from_team_index, m.to_team_index, &[m.player_id]);
            record_player_decisions(teams, m.from_team_index, m.to_team_index, &[m.player_id], date, &coach_name, &m.reason);

            let move_type = if m.to_team_index == main_idx {
                RecentMoveType::RecalledFromReserves
            } else {
                RecentMoveType::DemotedToReserves
            };
            record_moves(coach_state, &[m.player_id], move_type, date);
            any_move = true;
        }

        if any_move {
            if let Some(state) = coach_state {
                state.weeks_since_last_change = 0;
            }
        }
    }
}

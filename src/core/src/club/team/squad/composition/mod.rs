use crate::club::staff::perception::{CoachDecisionState, RecentMoveType};
use crate::{Team, TeamType};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::{MIN_FIRST_TEAM_SQUAD, execute_moves, record_moves, record_player_decisions};

// ─── AI prompt data types ──────────────────────────────────────────

#[derive(Serialize)]
struct SquadQueryLlm {
    staff: serde_json::Value,
    teams: Vec<TeamPlayersLlm>,
}

#[derive(Serialize)]
struct TeamPlayersLlm {
    label: String,
    team_index: usize,
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
    /// Build AI prompt without calling AI (read-only teams).
    pub fn prepare_request(
        teams: &[Team],
        main_idx: usize,
        reserve_idx: Option<usize>,
        youth_idx: Option<usize>,
        sim_date: NaiveDate,
    ) -> (String, String) {
        let team_indices = Self::collect_team_indices(main_idx, reserve_idx, youth_idx);
        let query = Self::build_prompt(teams, main_idx, &team_indices, sim_date);
        let format = Self::response_format();
        (query, format)
    }

    /// Apply raw AI response string to mutable teams.
    pub fn execute_response(
        response: &str,
        teams: &mut [Team],
        coach_state: &mut Option<CoachDecisionState>,
        main_idx: usize,
        reserve_idx: Option<usize>,
        youth_idx: Option<usize>,
        date: NaiveDate,
    ) {
        let advice: AiSquadAdvice = match serde_json::from_str(response) {
            Ok(a) => a,
            Err(_) => return,
        };
        let team_indices = Self::collect_team_indices(main_idx, reserve_idx, youth_idx);
        Self::execute_advice(teams, coach_state, main_idx, &team_indices, &advice, date);
    }

    // ─── Prompt building ──────────────────────────────────────────

    fn collect_team_indices(
        main_idx: usize,
        reserve_idx: Option<usize>,
        youth_idx: Option<usize>,
    ) -> Vec<(usize, &'static str)> {
        let mut v = vec![(main_idx, "Main Team")];
        if let Some(idx) = reserve_idx {
            v.push((idx, "Reserve Team"));
        }
        if let Some(idx) = youth_idx {
            v.push((idx, "Youth Team"));
        }
        v
    }

    fn build_prompt(
        teams: &[Team],
        main_idx: usize,
        team_indices: &[(usize, &str)],
        sim_date: NaiveDate,
    ) -> String {
        let staff_data = teams[main_idx].staffs.head_coach().as_llm();
        let data_json = Self::build_data_json(teams, team_indices, &staff_data, sim_date);
        let teams_section = Self::build_teams_section(teams, team_indices);
        let previous_moves_section = Self::build_previous_moves(teams, team_indices);

        format!(
            include_str!("prompt.md"),
            staff_data = staff_data,
            teams_section = teams_section,
            previous_moves_section = previous_moves_section,
            data_json = data_json,
        )
    }

    fn build_data_json(
        teams: &[Team],
        team_indices: &[(usize, &str)],
        staff_data: &str,
        sim_date: NaiveDate,
    ) -> String {
        let staff_json: serde_json::Value = serde_json::from_str(staff_data).unwrap();

        let squad_teams: Vec<TeamPlayersLlm> = team_indices
            .iter()
            .map(|&(idx, label)| {
                let team = &teams[idx];
                let head_coach = team.staffs.head_coach();
                let players: Vec<serde_json::Value> = team
                    .players
                    .players
                    .iter()
                    .map(|p| serde_json::from_str(&p.as_llm(head_coach, sim_date)).unwrap())
                    .collect();
                TeamPlayersLlm {
                    label: label.to_string(),
                    team_index: idx,
                    players,
                }
            })
            .collect();

        let squad_data = SquadQueryLlm {
            staff: staff_json,
            teams: squad_teams,
        };

        if cfg!(debug_assertions) {
            serde_json::to_string_pretty(&squad_data).unwrap()
        } else {
            serde_json::to_string(&squad_data).unwrap()
        }
    }

    fn build_teams_section(teams: &[Team], team_indices: &[(usize, &str)]) -> String {
        team_indices
            .iter()
            .map(|&(idx, _)| {
                let type_name = match teams[idx].team_type {
                    TeamType::Main => "Main",
                    TeamType::B => "B Team",
                    TeamType::Second => "Second Team",
                    TeamType::Reserve => "U20",
                    TeamType::U18 => "Under 18s",
                    TeamType::U19 => "Under 19s",
                    TeamType::U20 => "Under 20s",
                    TeamType::U21 => "Under 21s",
                    TeamType::U23 => "Under 23s",
                };
                format!("team_index={}, type={}", idx, type_name)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn build_previous_moves(teams: &[Team], team_indices: &[(usize, &str)]) -> String {
        let mut moves = String::new();
        for &(idx, _) in team_indices {
            for player in &teams[idx].players.players {
                for d in &player.decision_history.items {
                    moves.push_str(&format!(
                        "id={},from_team={},reason={},date={}\n",
                        player.id,
                        idx,
                        d.decision,
                        d.date.format("%Y-%m-%d")
                    ));
                }
            }
        }

        if moves.is_empty() {
            String::new()
        } else {
            format!("\n[PREVIOUS MOVES]\n{}", moves)
        }
    }

    fn response_format() -> String {
        r#"Respond ONLY with JSON: {"moves":[{"player_id":123,"from_team_index":0,"to_team_index":1, reason: "Describe move reason"}]}"#.to_string()
    }

    // ─── Advice execution ─────────────────────────────────────────

    fn execute_advice(
        teams: &mut [Team],
        coach_state: &mut Option<CoachDecisionState>,
        main_idx: usize,
        team_indices: &[(usize, &str)],
        advice: &AiSquadAdvice,
        date: NaiveDate,
    ) {
        let valid_indices: Vec<usize> = team_indices.iter().map(|(idx, _)| *idx).collect();
        let coach_name = teams[main_idx].staffs.head_coach().full_name.to_string();
        let mut any_move = false;

        for m in &advice.moves {
            if !Self::is_valid_move(teams, &valid_indices, m) {
                continue;
            }

            // Guard: block demotions from main team if squad is at minimum
            if m.from_team_index == main_idx && m.to_team_index != main_idx {
                if teams[main_idx].players.players.len() <= MIN_FIRST_TEAM_SQUAD {
                    continue;
                }
            }

            execute_moves(teams, m.from_team_index, m.to_team_index, &[m.player_id]);
            record_player_decisions(
                teams,
                m.from_team_index,
                m.to_team_index,
                &[m.player_id],
                date,
                &coach_name,
                &m.reason,
            );

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

    fn is_valid_move(teams: &[Team], valid_indices: &[usize], m: &AiSquadMove) -> bool {
        if !valid_indices.contains(&m.from_team_index) || !valid_indices.contains(&m.to_team_index)
        {
            return false;
        }
        if m.from_team_index == m.to_team_index {
            return false;
        }
        teams[m.from_team_index].players.contains(m.player_id)
    }
}

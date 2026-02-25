use crate::club::player::player::Player;
use crate::club::staff::staff::Staff;
use crate::club::team::coach_perception::CoachDecisionState;
use crate::context::GlobalContext;
use crate::shared::{Currency, CurrencyValue};
use crate::{PlayerStatusType, Team, TransferItem};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

// ─── AI response types ─────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct AiTransferListAdvice {
    transfer_list: Vec<AiListingDecision>,
    loan_list: Vec<AiListingDecision>,
    delist: Vec<AiListingDecision>,
}

#[derive(Deserialize, Debug)]
struct AiListingDecision {
    player_id: u32,
    reason: String,
}

// ─── AI prompt data types ──────────────────────────────────────────

#[derive(Serialize)]
struct TransferListQueryLlm {
    #[serde(rename = "s")]
    staff: serde_json::Value,
    #[serde(rename = "sl")]
    staff_legend: serde_json::Value,
    #[serde(rename = "pl")]
    player_legend: serde_json::Value,
    #[serde(rename = "tl")]
    current_transfer_list: Vec<TransferListEntryLlm>,
    #[serde(rename = "t")]
    teams: Vec<TeamPlayersLlm>,
}

#[derive(Serialize)]
struct TransferListEntryLlm {
    #[serde(rename = "id")]
    player_id: u32,
    #[serde(rename = "ask")]
    asking_price: f64,
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

pub struct TransferListManager;

impl TransferListManager {
    /// AI-only transfer list management. Skips entirely if AI is unavailable.
    pub fn manage(
        ctx: &GlobalContext<'_>,
        teams: &mut [Team],
        _coach_state: &Option<CoachDecisionState>,
        main_idx: usize,
        date: NaiveDate,
    ) {
        if !ctx.ai_enabled() {
            return;
        }

        let team_indices = Self::collect_team_indices(teams);
        let query = Self::build_prompt(teams, main_idx, &team_indices, date);
        let format = Self::response_format();

        let advice: AiTransferListAdvice = match ctx.ai(query, format) {
            Some(a) => a,
            None => return,
        };

        Self::execute_advice(teams, main_idx, &team_indices, &advice, date);
    }

    // ─── Prompt building ──────────────────────────────────────────

    fn collect_team_indices(teams: &[Team]) -> Vec<(usize, &'static str)> {
        teams
            .iter()
            .enumerate()
            .map(|(idx, t)| {
                let label = match t.team_type {
                    crate::TeamType::Main => "Main Team",
                    crate::TeamType::B => "Reserve Team",
                    crate::TeamType::U18 => "Under 18s",
                    crate::TeamType::U19 => "Under 19s",
                    crate::TeamType::U20 => "Under 20s",
                    crate::TeamType::U21 => "Under 21s",
                    crate::TeamType::U23 => "Under 23s",
                };
                (idx, label)
            })
            .collect()
    }

    fn build_prompt(
        teams: &[Team],
        main_idx: usize,
        team_indices: &[(usize, &str)],
        _date: NaiveDate,
    ) -> String {
        let staff_data = teams[main_idx].staffs.head_coach().as_llm();
        let data_json = Self::build_data_json(teams, main_idx, team_indices, &staff_data);
        let teams_section = Self::build_teams_section(teams, team_indices);
        let previous_decisions_section = Self::build_previous_decisions(teams, team_indices);
        let current_tl = Self::build_current_transfer_list_section(teams, main_idx, &data_json);
        let current_loans = Self::build_current_loans_section(teams, main_idx);

        format!(
            include_str!("prompt.md"),
            staff_legend = Staff::llm_legend(),
            staff_data = staff_data,
            teams_section = teams_section,
            current_tl = current_tl,
            current_loans = current_loans,
            previous_decisions_section = previous_decisions_section,
            data_json = data_json,
        )
    }

    fn build_data_json(
        teams: &[Team],
        main_idx: usize,
        team_indices: &[(usize, &str)],
        staff_data: &str,
    ) -> String {
        let staff_json: serde_json::Value = serde_json::from_str(staff_data).unwrap();
        let staff_legend_json: serde_json::Value =
            serde_json::from_str(Staff::llm_legend()).unwrap();
        let player_legend_json: serde_json::Value =
            serde_json::from_str(Player::llm_legend()).unwrap();

        let current_transfer_list: Vec<TransferListEntryLlm> = teams[main_idx]
            .transfer_list
            .items()
            .iter()
            .map(|item| TransferListEntryLlm {
                player_id: item.player_id,
                asking_price: item.amount.amount,
            })
            .collect();

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

        let query_data = TransferListQueryLlm {
            staff: staff_json,
            staff_legend: staff_legend_json,
            player_legend: player_legend_json,
            current_transfer_list,
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
                        player.id,
                        d.decision,
                        d.movement,
                        d.date.format("%Y-%m-%d")
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

    fn build_current_transfer_list_section(
        teams: &[Team],
        main_idx: usize,
        data_json: &str,
    ) -> String {
        if teams[main_idx].transfer_list.items().is_empty() {
            "None".to_string()
        } else {
            data_json.to_string()
        }
    }

    fn build_current_loans_section(teams: &[Team], main_idx: usize) -> String {
        let loan_ids: Vec<u32> = teams[main_idx]
            .players
            .players
            .iter()
            .filter(|p| p.statuses.get().contains(&PlayerStatusType::Loa))
            .map(|p| p.id)
            .collect();

        if loan_ids.is_empty() {
            "None".to_string()
        } else {
            loan_ids
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    fn response_format() -> String {
        r#"Respond ONLY with JSON: {"transfer_list":[{"player_id":123,"reason":"..."}],"loan_list":[{"player_id":456,"reason":"..."}],"delist":[{"player_id":789,"reason":"..."}]}"#.to_string()
    }

    // ─── Advice execution ─────────────────────────────────────────

    fn execute_advice(
        teams: &mut [Team],
        main_idx: usize,
        team_indices: &[(usize, &str)],
        advice: &AiTransferListAdvice,
        date: NaiveDate,
    ) {
        let coach_name = teams[main_idx].staffs.head_coach().full_name.to_string();

        let transfer_ids: Vec<u32> = advice.transfer_list.iter().map(|d| d.player_id).collect();
        let loan_ids: Vec<u32> = advice.loan_list.iter().map(|d| d.player_id).collect();

        // Collect all IDs being listed this tick to prevent contradictory delist
        let mut just_listed: Vec<u32> = Vec::new();

        Self::execute_transfer_listings(teams, main_idx, team_indices, &advice.transfer_list, &loan_ids, &coach_name, date, &mut just_listed);
        Self::execute_loan_listings(teams, team_indices, &advice.loan_list, &transfer_ids, &coach_name, date, &mut just_listed);
        Self::execute_delistings(teams, main_idx, team_indices, &advice.delist, &just_listed, &coach_name, date);
    }

    fn execute_transfer_listings(
        teams: &mut [Team],
        main_idx: usize,
        team_indices: &[(usize, &str)],
        decisions: &[AiListingDecision],
        loan_ids: &[u32],
        coach_name: &str,
        date: NaiveDate,
        just_listed: &mut Vec<u32>,
    ) {
        for decision in decisions {
            if loan_ids.contains(&decision.player_id) {
                continue;
            }
            if !player_exists_in_teams(teams, team_indices, decision.player_id) {
                continue;
            }
            if teams[main_idx].transfer_list.contains(decision.player_id) {
                continue;
            }
            let asking_price = find_player_in_teams(teams, team_indices, decision.player_id)
                .map(|p| p.value(date))
                .unwrap_or(0.0);

            teams[main_idx]
                .transfer_list
                .add(TransferItem::new(
                    decision.player_id,
                    CurrencyValue::new(asking_price, Currency::Usd),
                ));

            set_player_status(teams, team_indices, decision.player_id, PlayerStatusType::Lst, date);
            record_listing_decision(teams, team_indices, decision.player_id, date, coach_name,
                &format!("Transfer listed: {}", decision.reason));
            just_listed.push(decision.player_id);
        }
    }

    fn execute_loan_listings(
        teams: &mut [Team],
        team_indices: &[(usize, &str)],
        decisions: &[AiListingDecision],
        transfer_ids: &[u32],
        coach_name: &str,
        date: NaiveDate,
        just_listed: &mut Vec<u32>,
    ) {
        for decision in decisions {
            if transfer_ids.contains(&decision.player_id) {
                continue;
            }
            if !player_exists_in_teams(teams, team_indices, decision.player_id) {
                continue;
            }
            if has_status(teams, team_indices, decision.player_id, PlayerStatusType::Loa) {
                continue;
            }

            set_player_status(teams, team_indices, decision.player_id, PlayerStatusType::Loa, date);
            record_listing_decision(teams, team_indices, decision.player_id, date, coach_name,
                &format!("Loan listed: {}", decision.reason));
            just_listed.push(decision.player_id);
        }
    }

    fn execute_delistings(
        teams: &mut [Team],
        main_idx: usize,
        team_indices: &[(usize, &str)],
        decisions: &[AiListingDecision],
        just_listed: &[u32],
        coach_name: &str,
        date: NaiveDate,
    ) {
        for decision in decisions {
            if just_listed.contains(&decision.player_id) {
                continue;
            }
            if !player_exists_in_teams(teams, team_indices, decision.player_id) {
                continue;
            }

            let was_transfer_listed = teams[main_idx].transfer_list.contains(decision.player_id);
            let was_loan_listed = has_status(teams, team_indices, decision.player_id, PlayerStatusType::Loa);

            if !was_transfer_listed && !was_loan_listed {
                continue;
            }

            if was_transfer_listed {
                teams[main_idx].transfer_list.remove(decision.player_id);
                remove_player_status(teams, team_indices, decision.player_id, PlayerStatusType::Lst);
            }

            if was_loan_listed {
                remove_player_status(teams, team_indices, decision.player_id, PlayerStatusType::Loa);
            }

            record_listing_decision(teams, team_indices, decision.player_id, date, coach_name,
                &format!("Delisted: {}", decision.reason));
        }
    }
}

// ─── Helper functions ──────────────────────────────────────────────

fn player_exists_in_teams(teams: &[Team], indices: &[(usize, &str)], player_id: u32) -> bool {
    indices
        .iter()
        .any(|&(idx, _)| teams[idx].players.players.iter().any(|p| p.id == player_id))
}

fn find_player_in_teams<'a>(
    teams: &'a [Team],
    indices: &[(usize, &str)],
    player_id: u32,
) -> Option<&'a Player> {
    for &(idx, _) in indices {
        if let Some(p) = teams[idx].players.players.iter().find(|p| p.id == player_id) {
            return Some(p);
        }
    }
    None
}

fn has_status(
    teams: &[Team],
    indices: &[(usize, &str)],
    player_id: u32,
    status: PlayerStatusType,
) -> bool {
    for &(idx, _) in indices {
        if let Some(p) = teams[idx].players.players.iter().find(|p| p.id == player_id) {
            return p.statuses.get().contains(&status);
        }
    }
    false
}

fn set_player_status(
    teams: &mut [Team],
    indices: &[(usize, &str)],
    player_id: u32,
    status: PlayerStatusType,
    date: NaiveDate,
) {
    for &(idx, _) in indices {
        if let Some(p) = teams[idx].players.players.iter_mut().find(|p| p.id == player_id) {
            p.statuses.add(date, status);
            return;
        }
    }
}

fn remove_player_status(
    teams: &mut [Team],
    indices: &[(usize, &str)],
    player_id: u32,
    status: PlayerStatusType,
) {
    for &(idx, _) in indices {
        if let Some(p) = teams[idx].players.players.iter_mut().find(|p| p.id == player_id) {
            p.statuses.remove(status);
            return;
        }
    }
}

fn record_listing_decision(
    teams: &mut [Team],
    indices: &[(usize, &str)],
    player_id: u32,
    date: NaiveDate,
    decided_by: &str,
    reason: &str,
) {
    for &(idx, _) in indices {
        if let Some(p) = teams[idx].players.players.iter_mut().find(|p| p.id == player_id) {
            p.decision_history.add(
                date,
                "Transfer list".to_string(),
                reason.to_string(),
                decided_by.to_string(),
            );
            return;
        }
    }
}

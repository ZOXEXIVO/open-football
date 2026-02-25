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

        // Collect team indices
        let team_indices: Vec<(usize, &str)> = teams
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
            .collect();

        let main_coach = teams[main_idx].staffs.head_coach();
        let staff_data = main_coach.as_llm();
        let staff_json: serde_json::Value = serde_json::from_str(&staff_data).unwrap();
        let staff_legend_json: serde_json::Value =
            serde_json::from_str(Staff::llm_legend()).unwrap();
        let player_legend_json: serde_json::Value =
            serde_json::from_str(Player::llm_legend()).unwrap();

        // Current transfer list entries
        let current_transfer_list: Vec<TransferListEntryLlm> = teams[main_idx]
            .transfer_list
            .items()
            .iter()
            .map(|item| TransferListEntryLlm {
                player_id: item.player_id,
                asking_price: item.amount.amount,
            })
            .collect();

        // Build squad data per team
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

        let data_json = if cfg!(debug_assertions) {
            serde_json::to_string_pretty(&query_data).unwrap()
        } else {
            serde_json::to_string(&query_data).unwrap()
        };

        // Collect previous decisions
        let mut previous_decisions = String::new();
        for &(idx, _) in &team_indices {
            for player in &teams[idx].players.players {
                for d in &player.decision_history.items {
                    previous_decisions.push_str(&format!(
                        "id={},action={},reason={},date={}\n",
                        player.id,
                        d.decision,
                        d.movement,
                        d.date.format("%Y-%m-%d")
                    ));
                }
            }
        }

        let previous_decisions_section = if previous_decisions.is_empty() {
            String::new()
        } else {
            format!("\n[PREVIOUS DECISIONS]\n{}", previous_decisions)
        };

        // Build teams description
        let teams_section: String = team_indices
            .iter()
            .map(|&(idx, label)| {
                let team = &teams[idx];
                format!(
                    "team_index={}, type={}, players={}",
                    idx, label, team.players.players.len()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Loan-listed player IDs
        let loan_listed: Vec<u32> = teams[main_idx]
            .players
            .players
            .iter()
            .filter(|p| p.statuses.get().contains(&PlayerStatusType::Loa))
            .map(|p| p.id)
            .collect();

        let current_loans_section = if loan_listed.is_empty() {
            "None".to_string()
        } else {
            loan_listed
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };

        let current_tl = if teams[main_idx].transfer_list.items().is_empty() {
            "None".to_string()
        } else {
            data_json.clone()
        };

        let query = format!(
            include_str!("prompt.md"),
            staff_legend = Staff::llm_legend(),
            staff_data = staff_data,
            teams_section = teams_section,
            current_tl = current_tl,
            current_loans = current_loans_section,
            previous_decisions_section = previous_decisions_section,
            data_json = data_json,
        );

        let format = String::from(
            r#"Respond ONLY with JSON: {"transfer_list":[{"player_id":123,"reason":"..."}],"loan_list":[{"player_id":456,"reason":"..."}],"delist":[{"player_id":789,"reason":"..."}]}"#,
        );

        let advice: AiTransferListAdvice = match ctx.ai(query, format) {
            Some(a) => a,
            None => return,
        };

        let coach_name = teams[main_idx].staffs.head_coach().full_name.to_string();

        // Collect all player IDs being listed to detect double-listing
        let transfer_ids: Vec<u32> = advice.transfer_list.iter().map(|d| d.player_id).collect();
        let loan_ids: Vec<u32> = advice.loan_list.iter().map(|d| d.player_id).collect();

        // Execute transfer list additions
        for decision in &advice.transfer_list {
            if loan_ids.contains(&decision.player_id) {
                continue;
            }
            if !player_exists_in_teams(teams, &team_indices, decision.player_id) {
                continue;
            }
            if teams[main_idx].transfer_list.contains(decision.player_id) {
                continue;
            }

            let asking_price = find_player_in_teams(teams, &team_indices, decision.player_id)
                .map(|p| p.value(date))
                .unwrap_or(0.0);

            teams[main_idx]
                .transfer_list
                .add(TransferItem::new(
                    decision.player_id,
                    CurrencyValue::new(asking_price, Currency::Usd),
                ));

            set_player_status(teams, &team_indices, decision.player_id, PlayerStatusType::Lst, date);

            record_listing_decision(
                teams, &team_indices, decision.player_id, date, &coach_name,
                &format!("Transfer listed: {}", decision.reason),
            );
        }

        // Execute loan list additions
        for decision in &advice.loan_list {
            if transfer_ids.contains(&decision.player_id) {
                continue;
            }
            if !player_exists_in_teams(teams, &team_indices, decision.player_id) {
                continue;
            }
            if has_status(teams, &team_indices, decision.player_id, PlayerStatusType::Loa) {
                continue;
            }

            set_player_status(teams, &team_indices, decision.player_id, PlayerStatusType::Loa, date);

            record_listing_decision(
                teams, &team_indices, decision.player_id, date, &coach_name,
                &format!("Loan listed: {}", decision.reason),
            );
        }

        // Execute delistings
        for decision in &advice.delist {
            if !player_exists_in_teams(teams, &team_indices, decision.player_id) {
                continue;
            }

            let was_transfer_listed = teams[main_idx].transfer_list.contains(decision.player_id);
            let was_loan_listed =
                has_status(teams, &team_indices, decision.player_id, PlayerStatusType::Loa);

            if !was_transfer_listed && !was_loan_listed {
                continue;
            }

            if was_transfer_listed {
                teams[main_idx].transfer_list.remove(decision.player_id);
                remove_player_status(teams, &team_indices, decision.player_id, PlayerStatusType::Lst);
            }

            if was_loan_listed {
                remove_player_status(teams, &team_indices, decision.player_id, PlayerStatusType::Loa);
            }

            record_listing_decision(
                teams, &team_indices, decision.player_id, date, &coach_name,
                &format!("Delisted: {}", decision.reason),
            );
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

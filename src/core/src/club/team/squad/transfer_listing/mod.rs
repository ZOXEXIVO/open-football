use crate::club::player::player::Player;
use crate::club::team::squad::MIN_FIRST_TEAM_SQUAD;
use crate::shared::{Currency, CurrencyValue};
use crate::{PlayerSquadStatus, PlayerStatusType, Team, TeamType, TransferItem};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    staff: Value,
    current_transfer_list: Vec<TransferListEntryLlm>,
    teams: Vec<TeamPlayersLlm>,
}

#[derive(Serialize)]
struct TransferListEntryLlm {
    player_id: u32,
    asking_price: f64,
}

#[derive(Serialize)]
struct TeamPlayersLlm {
    label: String,
    team_index: usize,
    player_count: usize,
    players: Vec<Value>,
}

// ─── Public API ────────────────────────────────────────────────────

pub struct TransferListManager;

impl TransferListManager {
    /// Build AI prompt without calling AI (read-only teams).
    ///
    /// `wage_budget_headroom` is the club's remaining annual wage capacity
    /// (board wage budget minus current wage bill). When supplied, the
    /// LLM payload labels each player's `contract_stalemate.pending_ask.affordable`
    /// against it; without it (callers that don't have a board context),
    /// affordability surfaces as `null` and the prompt treats it as unknown.
    pub fn prepare_request(
        teams: &[Team],
        main_idx: usize,
        date: NaiveDate,
        wage_budget_headroom: Option<u32>,
    ) -> (String, String) {
        let team_indices = Self::collect_team_indices(teams);
        let query = Self::build_prompt(teams, main_idx, &team_indices, date, wage_budget_headroom);
        let format = Self::response_format();
        (query, format)
    }

    /// Apply raw AI response string to mutable teams.
    pub fn execute_response(response: &str, teams: &mut [Team], main_idx: usize, date: NaiveDate) {
        let advice: AiTransferListAdvice = match serde_json::from_str(response) {
            Ok(a) => a,
            Err(_) => return,
        };
        let team_indices = Self::collect_team_indices(teams);
        Self::execute_advice(teams, main_idx, &team_indices, &advice, date);
    }

    // ─── Prompt building ──────────────────────────────────────────

    fn collect_team_indices(teams: &[Team]) -> Vec<(usize, &'static str)> {
        teams
            .iter()
            .enumerate()
            .map(|(idx, t)| {
                let label = match t.team_type {
                    TeamType::Main => "Main Team",
                    TeamType::B => "B Team",
                    TeamType::Second => "Second Team",
                    TeamType::Reserve => "Reserve Team",
                    TeamType::U18 => "Under 18s",
                    TeamType::U19 => "Under 19s",
                    TeamType::U20 => "Under 20s",
                    TeamType::U21 => "Under 21s",
                    TeamType::U23 => "Under 23s",
                };
                (idx, label)
            })
            .collect()
    }

    fn build_prompt(
        teams: &[Team],
        main_idx: usize,
        team_indices: &[(usize, &str)],
        sim_date: NaiveDate,
        wage_budget_headroom: Option<u32>,
    ) -> String {
        let staff_data = teams[main_idx].staffs.head_coach().as_llm();
        let data_json =
            Self::build_data_json(teams, team_indices, &staff_data, sim_date, wage_budget_headroom);
        let teams_section = Self::build_teams_section(teams, team_indices);
        let previous_decisions_section = Self::build_previous_decisions(teams, team_indices);
        let current_tl = Self::build_current_transfer_list_section(teams, team_indices, &data_json);
        let current_loans = Self::build_current_loans_section(teams, main_idx);

        format!(
            include_str!("prompt.md"),
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
        team_indices: &[(usize, &str)],
        staff_data: &str,
        sim_date: NaiveDate,
        wage_budget_headroom: Option<u32>,
    ) -> String {
        let staff_json: Value = serde_json::from_str(staff_data).unwrap();

        // Aggregate across every team in the club, not just Main — a player
        // listed while sitting in the reserves must still show in the prompt.
        let current_transfer_list: Vec<TransferListEntryLlm> =
            Self::all_listed_entries(teams, team_indices);

        // Wage budget headroom for the club — supplied by the club-level
        // caller. Used to label `contract_stalemate.pending_ask.affordable`
        // in the AI payload so the prompt can distinguish "player wants
        // more, we can pay" from "player wants more than we can afford".
        // None when the caller has no board context (tests, fixtures).
        let headroom = wage_budget_headroom;
        let squad_teams: Vec<TeamPlayersLlm> = team_indices
            .iter()
            .map(|&(idx, label)| {
                let team = &teams[idx];
                let head_coach = team.staffs.head_coach();
                let players: Vec<Value> = team
                    .players
                    .players
                    .iter()
                    .map(|p| {
                        serde_json::from_str(
                            &p.as_llm_with_affordability(head_coach, sim_date, headroom),
                        )
                        .unwrap()
                    })
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
                    idx,
                    label,
                    teams[idx].players.players.len()
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
        team_indices: &[(usize, &str)],
        data_json: &str,
    ) -> String {
        if Self::any_team_has_listing(teams, team_indices) {
            data_json.to_string()
        } else {
            "None".to_string()
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

    // ─── Club-wide transfer-list access ────────────────────────────
    //
    // A transfer-list entry lives on the team the player currently sits in — an
    // internal Main↔Reserve/B move migrates the entry with him. Every read or
    // write of the club's transfer list must therefore scan ALL of the club's
    // teams, not just the Main team; otherwise a player listed while in the
    // reserves becomes invisible to the current-list prompt data and cannot be
    // delisted.

    /// Aggregate every team's transfer-list entries across the club.
    fn all_listed_entries(
        teams: &[Team],
        team_indices: &[(usize, &str)],
    ) -> Vec<TransferListEntryLlm> {
        let mut out = Vec::new();
        for &(idx, _) in team_indices {
            for item in teams[idx].transfer_list.items() {
                out.push(TransferListEntryLlm {
                    player_id: item.player_id,
                    asking_price: item.amount.amount,
                });
            }
        }
        out
    }

    /// Any of the club's teams currently holds a transfer-list entry.
    fn any_team_has_listing(teams: &[Team], team_indices: &[(usize, &str)]) -> bool {
        team_indices
            .iter()
            .any(|&(idx, _)| !teams[idx].transfer_list.items().is_empty())
    }

    /// The player is transfer-listed on ANY of the club's teams.
    fn is_listed_anywhere(teams: &[Team], team_indices: &[(usize, &str)], player_id: u32) -> bool {
        team_indices
            .iter()
            .any(|&(idx, _)| teams[idx].transfer_list.contains(player_id))
    }

    /// Remove the player's transfer-list entry from whichever team holds it.
    /// Returns true if an entry was removed.
    fn remove_listing_anywhere(
        teams: &mut [Team],
        team_indices: &[(usize, &str)],
        player_id: u32,
    ) -> bool {
        let mut removed = false;
        for &(idx, _) in team_indices {
            if teams[idx].transfer_list.contains(player_id) {
                teams[idx].transfer_list.remove(player_id);
                removed = true;
            }
        }
        removed
    }

    /// Add the asking-price entry to the team the player currently sits in, so
    /// the listing is co-located with the player (consistent with the
    /// internal-move migration). Falls back to the Main team if he can't be
    /// located on any squad.
    fn add_listing_to_player_team(
        teams: &mut [Team],
        team_indices: &[(usize, &str)],
        main_idx: usize,
        player_id: u32,
        item: TransferItem,
    ) {
        for &(idx, _) in team_indices {
            if teams[idx].players.contains(player_id) {
                teams[idx].transfer_list.add(item);
                return;
            }
        }
        teams[main_idx].transfer_list.add(item);
    }

    // ─── Advice execution ─────────────────────────────────────────

    fn execute_advice(
        teams: &mut [Team],
        main_idx: usize,
        team_indices: &[(usize, &str)],
        advice: &AiTransferListAdvice,
        date: NaiveDate,
    ) {
        let coach_name = teams[main_idx].staffs.head_coach_name();

        let transfer_ids: Vec<u32> = advice.transfer_list.iter().map(|d| d.player_id).collect();
        let loan_ids: Vec<u32> = advice.loan_list.iter().map(|d| d.player_id).collect();

        // Count non-listed first team players to enforce minimum squad size
        let available_main = teams[main_idx]
            .players
            .iter()
            .filter(|p| {
                let s = p.statuses.get();
                !s.contains(&PlayerStatusType::Lst) && !s.contains(&PlayerStatusType::Loa)
            })
            .count();
        let mut listing_budget = available_main.saturating_sub(MIN_FIRST_TEAM_SQUAD);

        // Collect all IDs being listed this tick to prevent contradictory delist
        let mut just_listed: Vec<u32> = Vec::new();

        Self::execute_transfer_listings(
            teams,
            main_idx,
            team_indices,
            &advice.transfer_list,
            &loan_ids,
            &coach_name,
            date,
            &mut just_listed,
            &mut listing_budget,
        );
        Self::execute_loan_listings(
            teams,
            main_idx,
            team_indices,
            &advice.loan_list,
            &transfer_ids,
            &coach_name,
            date,
            &mut just_listed,
            &mut listing_budget,
        );
        Self::execute_delistings(
            teams,
            team_indices,
            &advice.delist,
            &just_listed,
            &coach_name,
            date,
        );
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
        listing_budget: &mut usize,
    ) {
        for decision in decisions {
            if loan_ids.contains(&decision.player_id) {
                continue;
            }
            if !player_exists_in_teams(teams, team_indices, decision.player_id) {
                continue;
            }
            // Loaned-in players belong to another club — cannot be listed
            if is_on_loan(teams, team_indices, decision.player_id) {
                continue;
            }
            // Already listed on any of the club's teams (entries follow the
            // player across internal moves).
            if Self::is_listed_anywhere(teams, team_indices, decision.player_id) {
                continue;
            }

            // Guard: valuable players cannot be listed unless they've
            // actively signalled they want out (REQ) or are unhappy (UNH).
            // Protects against LLM hallucinations like "contract expiring" on
            // a key striker with years left on his deal.
            if is_protected_from_listing(teams, team_indices, decision.player_id) {
                continue;
            }

            // Guard: skip listing main team players when budget exhausted
            let is_main_team_player = teams[main_idx].players.contains(decision.player_id);
            if is_main_team_player && *listing_budget == 0 {
                continue;
            }

            // Asking price uses the selling club's actual blended
            // reputation so the listing fee reflects market context, not
            // a flat 0/0 baseline. The main team carries the canonical
            // market score for the club.
            let club_rep = teams[main_idx].reputation.market_value_score();
            let asking_price = find_player_in_teams(teams, team_indices, decision.player_id)
                .map(|p| p.value(date, club_rep, club_rep))
                .unwrap_or(0.0);

            // Co-locate the asking-price entry with the player's current team so
            // it stays consistent across internal squad moves.
            Self::add_listing_to_player_team(
                teams,
                team_indices,
                main_idx,
                decision.player_id,
                TransferItem::new(
                    decision.player_id,
                    CurrencyValue::new(asking_price, Currency::Usd),
                ),
            );

            set_player_status(
                teams,
                team_indices,
                decision.player_id,
                PlayerStatusType::Lst,
                date,
            );
            record_listing_decision(
                teams,
                team_indices,
                decision.player_id,
                date,
                coach_name,
                &format!("Transfer listed: {}", decision.reason),
            );
            just_listed.push(decision.player_id);

            if is_main_team_player {
                *listing_budget = listing_budget.saturating_sub(1);
            }
        }
    }

    fn execute_loan_listings(
        teams: &mut [Team],
        main_idx: usize,
        team_indices: &[(usize, &str)],
        decisions: &[AiListingDecision],
        transfer_ids: &[u32],
        coach_name: &str,
        date: NaiveDate,
        just_listed: &mut Vec<u32>,
        listing_budget: &mut usize,
    ) {
        for decision in decisions {
            if transfer_ids.contains(&decision.player_id) {
                continue;
            }
            if !player_exists_in_teams(teams, team_indices, decision.player_id) {
                continue;
            }
            // Loaned-in players belong to another club — cannot be re-loaned
            if is_on_loan(teams, team_indices, decision.player_id) {
                continue;
            }
            if has_status(
                teams,
                team_indices,
                decision.player_id,
                PlayerStatusType::Loa,
            ) {
                continue;
            }

            // Same protection used by the transfer-listing pass — covers
            // KeyPlayer/FirstTeamRegular/HotProspect and force-selected.
            if is_protected_from_listing(teams, team_indices, decision.player_id) {
                continue;
            }

            // Guard: skip listing main team players when budget exhausted
            let is_main_team_player = teams[main_idx].players.contains(decision.player_id);
            if is_main_team_player && *listing_budget == 0 {
                continue;
            }

            set_player_status(
                teams,
                team_indices,
                decision.player_id,
                PlayerStatusType::Loa,
                date,
            );
            record_listing_decision(
                teams,
                team_indices,
                decision.player_id,
                date,
                coach_name,
                &format!("Loan listed: {}", decision.reason),
            );
            just_listed.push(decision.player_id);

            if is_main_team_player {
                *listing_budget = listing_budget.saturating_sub(1);
            }
        }
    }

    fn execute_delistings(
        teams: &mut [Team],
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

            // A listed player may sit on any of the club's teams (an internal
            // move migrates his entry), so look across the whole club.
            let was_transfer_listed =
                Self::is_listed_anywhere(teams, team_indices, decision.player_id);
            let was_loan_listed = has_status(
                teams,
                team_indices,
                decision.player_id,
                PlayerStatusType::Loa,
            );

            if !was_transfer_listed && !was_loan_listed {
                continue;
            }

            if was_transfer_listed {
                Self::remove_listing_anywhere(teams, team_indices, decision.player_id);
                remove_player_status(
                    teams,
                    team_indices,
                    decision.player_id,
                    PlayerStatusType::Lst,
                );
            }

            if was_loan_listed {
                remove_player_status(
                    teams,
                    team_indices,
                    decision.player_id,
                    PlayerStatusType::Loa,
                );
            }

            record_listing_decision(
                teams,
                team_indices,
                decision.player_id,
                date,
                coach_name,
                &format!("Delisted: {}", decision.reason),
            );
        }
    }
}

// ─── Helper functions ──────────────────────────────────────────────

fn player_exists_in_teams(teams: &[Team], indices: &[(usize, &str)], player_id: u32) -> bool {
    indices
        .iter()
        .any(|&(idx, _)| teams[idx].players.contains(player_id))
}

fn find_player_in_teams<'a>(
    teams: &'a [Team],
    indices: &[(usize, &str)],
    player_id: u32,
) -> Option<&'a Player> {
    for &(idx, _) in indices {
        if let Some(p) = teams[idx].players.find(player_id) {
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
        if let Some(p) = teams[idx].players.find(player_id) {
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
        if let Some(p) = teams[idx].players.find_mut(player_id) {
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
        if let Some(p) = teams[idx].players.find_mut(player_id) {
            p.statuses.remove(status);
            return;
        }
    }
}

fn is_on_loan(teams: &[Team], indices: &[(usize, &str)], player_id: u32) -> bool {
    for &(idx, _) in indices {
        if let Some(p) = teams[idx].players.find(player_id) {
            return p.is_on_loan();
        }
    }
    false
}

fn is_protected_from_listing(teams: &[Team], indices: &[(usize, &str)], player_id: u32) -> bool {
    for &(idx, _) in indices {
        if let Some(p) = teams[idx].players.find(player_id) {
            // Manager-pinned players are absolutely protected — even an
            // explicit player request to leave is overridden, since the
            // flag's whole purpose is "do not move this player". The pin
            // is meaningful only while the player is on a contract; a
            // free agent is no longer at the club to be pinned.
            if p.is_force_match_selection && p.contract.is_some() {
                return true;
            }
            let wants_out = p
                .statuses
                .get()
                .iter()
                .any(|s| matches!(s, PlayerStatusType::Req | PlayerStatusType::Unh));
            if wants_out {
                return false;
            }
            let squad_status = p.contract.as_ref().map(|c| c.squad_status.clone());
            return matches!(
                squad_status,
                Some(PlayerSquadStatus::KeyPlayer)
                    | Some(PlayerSquadStatus::FirstTeamRegular)
                    | Some(PlayerSquadStatus::HotProspectForTheFuture)
            );
        }
    }
    false
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
        if let Some(p) = teams[idx]
            .players
            .players
            .iter_mut()
            .find(|p| p.id == player_id)
        {
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

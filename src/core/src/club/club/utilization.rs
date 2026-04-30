use super::Club;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::pipeline::{LoanOutCandidate, LoanOutReason, LoanOutStatus};
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{ContractType, Person, PlayerStatusType, ReputationLevel, TransferItem};
use chrono::NaiveDate;
use log::debug;

impl Club {
    /// Monthly audit: identify underutilized players in non-main teams and list them for loan/transfer.
    pub(super) fn audit_squad_utilization(&mut self, date: NaiveDate) {
        let main_idx = match self.teams.main_index() {
            Some(idx) => idx,
            None => return,
        };

        let rep_level = self.teams.teams[main_idx].reputation.level();

        // Wealthy clubs are more patient with underutilized players
        let (idle_threshold, games_threshold) = match rep_level {
            ReputationLevel::Elite => (120u16, 5u16),
            ReputationLevel::Continental => (90, 4),
            ReputationLevel::National => (60, 3),
            ReputationLevel::Regional => (45, 2),
            _ => (30, 1),
        };

        // Wealthy clubs within squad targets don't need to aggressively list
        let total_squad: usize = self.teams.iter().map(|t| t.players.len()).sum();
        let max_squad = self
            .board
            .season_targets
            .as_ref()
            .map(|t| t.max_squad_size as usize)
            .unwrap_or(50);
        let wealthy_within_limits = matches!(
            rep_level,
            ReputationLevel::Elite | ReputationLevel::Continental
        ) && total_squad < max_squad;

        // Collect underutilized player decisions
        let mut loan_players: Vec<(usize, u32, String)> = Vec::new();
        let mut transfer_players: Vec<(usize, u32, String)> = Vec::new();

        for (ti, team) in self.teams.iter().enumerate() {
            if ti == main_idx {
                continue;
            }

            for player in team.players.iter() {
                // Skip youth contracts
                if player
                    .contract
                    .as_ref()
                    .map(|c| c.contract_type == ContractType::Youth)
                    .unwrap_or(false)
                {
                    continue;
                }

                // Skip loan players
                if player.is_on_loan() {
                    continue;
                }

                // Skip already listed/loaned
                let statuses = player.statuses.get();
                if statuses.contains(&PlayerStatusType::Lst)
                    || statuses.contains(&PlayerStatusType::Loa)
                {
                    continue;
                }

                // Manager-pinned players: never auto-list, transfer or loan.
                if player.is_force_match_selection {
                    continue;
                }

                let days_idle = player.player_attributes.days_since_last_match;
                let total_games = player.statistics.total_games();

                // Reputation-scaled underutilization threshold
                if days_idle < idle_threshold || total_games >= games_threshold {
                    continue;
                }

                let age = player.age(date);
                let ca = player.player_attributes.current_ability;
                let pa = player.player_attributes.potential_ability;

                // Compare player CA to the main team average —
                // don't list players who are still competitive with the first team
                let main_avg_ca = self.teams.teams[main_idx].players.current_ability_avg();

                // Wealthy clubs within squad limits: only list truly unwanted players
                if wealthy_within_limits && ca >= 50 {
                    continue;
                }

                // Protect quality players who are competitive with the main team,
                // regardless of age — don't list a CA 120 player just because they're 31
                if ca >= main_avg_ca.saturating_sub(10) && age < 35 {
                    continue;
                }

                // Decision: choose Lst vs Loa based on player profile and club context
                if age <= 23 && pa > ca.saturating_add(5) {
                    loan_players.push((ti, player.id, "dec_reason_young_develop".to_string()));
                } else if ca < 60 && pa < 70 {
                    transfer_players.push((
                        ti,
                        player.id,
                        "dec_reason_low_ability_surplus".to_string(),
                    ));
                } else if age >= 34 && ca < main_avg_ca.saturating_sub(20) {
                    transfer_players.push((ti, player.id, "dec_reason_aging_surplus".to_string()));
                } else if matches!(
                    rep_level,
                    ReputationLevel::Elite | ReputationLevel::Continental
                ) && age <= 29
                {
                    loan_players.push((
                        ti,
                        player.id,
                        "dec_reason_underutilized_top_club".to_string(),
                    ));
                } else {
                    transfer_players.push((ti, player.id, "dec_reason_underutilized".to_string()));
                }
            }
        }

        self.process_underutilized_players(date, main_idx, &loan_players, &transfer_players);
    }

    fn process_underutilized_players(
        &mut self,
        date: NaiveDate,
        main_idx: usize,
        loan_players: &[(usize, u32, String)],
        transfer_players: &[(usize, u32, String)],
    ) {
        // Reputation-based loan fee multiplier
        let rep_multiplier = match self.teams.teams[main_idx].reputation.level() {
            ReputationLevel::Elite => 0.15,
            ReputationLevel::Continental => 0.10,
            ReputationLevel::National => 0.05,
            ReputationLevel::Regional => 0.02,
            _ => 0.0, // Local/Amateur: free loan
        };

        // Use the seller's actual blended reputation (not 0/0) so the
        // board's loan/transfer estimates track the player's true market
        // price. Country isn't visible here, so the helper approximates
        // league rep from the club's reputation score.
        let (seller_league_rep, seller_club_rep) =
            PlayerValuationCalculator::seller_context_from_club(self);

        // Process loan recommendations
        for (team_idx, player_id, reason) in loan_players {
            let team_idx = *team_idx;
            let player_id = *player_id;
            let team_name = self.teams.teams[team_idx].name.clone();

            let loan_fee = if rep_multiplier > 0.0 {
                let player_value = self.teams.teams[team_idx]
                    .players
                    .find(player_id)
                    .map(|p| p.value(date, seller_league_rep, seller_club_rep))
                    .unwrap_or(0.0);
                FormattingUtils::round_fee(player_value * rep_multiplier)
            } else {
                0.0
            };

            let player = match self.teams.teams[team_idx].players.find_mut(player_id) {
                Some(p) => p,
                None => continue,
            };

            player.statuses.add(date, PlayerStatusType::Loa);
            player.decision_history.add(
                date,
                "dec_board_loan_listed".to_string(),
                reason.clone(),
                "dec_decided_board".to_string(),
            );

            debug!(
                "Board loan-listed: {} (age {}, CA={}) from {}, loan fee: {}",
                player.full_name,
                player.age(date),
                player.player_attributes.current_ability,
                team_name,
                loan_fee
            );

            self.transfer_plan
                .loan_out_candidates
                .push(LoanOutCandidate {
                    player_id,
                    reason: LoanOutReason::LackOfPlayingTime,
                    status: LoanOutStatus::Listed,
                    loan_fee,
                });
        }

        // Process transfer recommendations
        for (team_idx, player_id, reason) in transfer_players {
            let team_idx = *team_idx;
            let player_id = *player_id;
            let team_name = self.teams.teams[team_idx].name.clone();

            let asking_price = {
                let player = match self.teams.teams[team_idx].players.find(player_id) {
                    Some(p) => p,
                    None => continue,
                };
                player.value(date, seller_league_rep, seller_club_rep) * 0.5
            };

            let player = match self.teams.teams[team_idx].players.find_mut(player_id) {
                Some(p) => p,
                None => continue,
            };

            player.statuses.add(date, PlayerStatusType::Lst);
            player.decision_history.add(
                date,
                "dec_board_transfer_listed".to_string(),
                reason.clone(),
                "dec_decided_board".to_string(),
            );

            debug!(
                "Board transfer-listed: {} (age {}, CA={}) from {}, asking {}",
                player.full_name,
                player.age(date),
                player.player_attributes.current_ability,
                team_name,
                asking_price
            );

            self.teams.teams[main_idx]
                .transfer_list
                .add(TransferItem::new(
                    player_id,
                    CurrencyValue::new(asking_price, Currency::Usd),
                ));
        }
    }
}

//! What the club decided about a player's squad, and how a decision becomes
//! a badge, a market listing or a loan candidate.

use chrono::NaiveDate;

use crate::shared::{Currency, CurrencyValue};
use crate::transfers::pipeline::TransferTrace;
use crate::transfers::pipeline::{
    LoanDestinationPreference, LoanOutCandidate, LoanOutReason, LoanOutStatus,
};
use crate::transfers::value::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{Club, Person, PlayerStatusType, ReputationLevel, TransferItem};
use log::debug;

/// One squad decision the club has reached this tick: put this player on the
/// market, or offer him out on loan.
///
/// `reason` is the i18n key the player's decision history, the press and the
/// transfer funnel trace all record, so the sweeps that collect decisions and
/// the pass that acts on them agree on why it happened.
pub(in crate::club::core) struct SquadDecision {
    pub team_idx: usize,
    pub player_id: u32,
    pub reason: &'static str,
}

impl SquadDecision {
    pub(in crate::club::core) const YOUNG_DEVELOP: &'static str = "dec_reason_young_develop";
    pub(in crate::club::core) const LOW_ABILITY_SURPLUS: &'static str =
        "dec_reason_low_ability_surplus";
    pub(in crate::club::core) const AGING_SURPLUS: &'static str = "dec_reason_aging_surplus";
    pub(in crate::club::core) const UNDERUTILIZED: &'static str = "dec_reason_underutilized";
    pub(in crate::club::core) const UNDERUTILIZED_TOP_CLUB: &'static str =
        "dec_reason_underutilized_top_club";
    pub(in crate::club::core) const WAGE_RELIEF: &'static str = "dec_reason_wage_relief";
    pub(in crate::club::core) const NEEDS_FIRST_TEAM_MINUTES: &'static str =
        "dec_reason_needs_first_team_minutes";
    pub(in crate::club::core) const LACK_PLAYING_TIME: &'static str =
        "dec_reason_lack_playing_time";

    pub(in crate::club::core) fn new(
        team_idx: usize,
        player_id: u32,
        reason: &'static str,
    ) -> Self {
        SquadDecision {
            team_idx,
            player_id,
            reason,
        }
    }
}

impl Club {
    pub(in crate::club::core) fn process_underutilized_players(
        &mut self,
        date: NaiveDate,
        main_idx: usize,
        loan_players: &[SquadDecision],
        transfer_players: &[SquadDecision],
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
        for decision in loan_players {
            let (team_idx, player_id) = (decision.team_idx, decision.player_id);
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
                decision.reason.to_string(),
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
                    preferred_destination: LoanDestinationPreference::Any,
                });
        }

        // Process transfer recommendations
        for decision in transfer_players {
            let (team_idx, player_id) = (decision.team_idx, decision.player_id);
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
            // The badge is the visible half of the decision; the contract
            // flag is the durable half. The flag survives the listing
            // pass's badge reconciliation, blocks renewal offers, and tells
            // the pass this is a club decision to materialise without a
            // second history row. Without it the badge was stripped the
            // first time the pass ran, the renewal manager saw a clean
            // player and re-signed him, and this audit listed him again
            // every window.
            if let Some(contract) = player.contract.as_mut() {
                contract.is_transfer_listed = true;
            }
            player.decision_history.add(
                date,
                "dec_board_transfer_listed".to_string(),
                decision.reason.to_string(),
                "dec_decided_board".to_string(),
            );
            // The board's own listing pass — including the wage-relief sale,
            // which arrives here tagged `dec_reason_wage_relief`. Every other
            // listing entry point reports itself to the funnel trace; without
            // this one, a marquee signing listed for money looked to the
            // trace like a player nobody had listed at all.
            TransferTrace::list(player, date, "board_utilization", decision.reason);

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

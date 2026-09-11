//! Prime-age professionals parked in a senior reserve squad.
//!
//! Every other sweep in the utilization audit reads MINUTES, and a B-team
//! regular has plenty — thirty league starts a season, none of them for the
//! team he was signed to play for. So this is the one pass that asks a
//! different question.

use std::collections::HashMap;

use chrono::NaiveDate;

use crate::club::player::statistics::StuckCareerScan;
use crate::club::staff::perception::AbilityEstimator;
use crate::club::team::squad::{SquadAssetClass, SquadAssetContext};
use crate::{Club, ContractType, Person, PlayerFieldPositionGroup, PlayerStatusType, Team};

use super::decision::SquadDecision;
use super::depth::{PromotionBar, YouthDevelopmentLoanPolicy};

impl Club {
    /// Resolve prime-age professionals parked in a senior reserve squad
    /// (B / Second / Reserve).
    ///
    /// Every other sweep in the audit reads MINUTES, and a B-team regular
    /// has plenty — thirty league starts a season, none of them for the
    /// team he was signed to play for. He therefore tripped nothing: not
    /// the idle-days audit (he is never idle), not the release gate (his
    /// own squad's "key player" label protected him), not the renewal
    /// cutoff (the ledger said he was playing). He simply stayed, at his
    /// peak, for as long as his contract kept renewing.
    ///
    /// The question this pass asks is the one none of the others could:
    /// is this adult, past the age where a reserve squad is development,
    /// ever going to play for the first team? If he is close to the
    /// promotion bar he is genuine cover and stays. If he is not, he
    /// leaves — on loan while he is young enough for that to be a career
    /// step, on the transfer list once he is not.
    pub(in crate::club::core) fn collect_parked_prime_resolutions(
        team: &Team,
        team_idx: usize,
        bar: &PromotionBar,
        asset_ctx: &SquadAssetContext,
        date: NaiveDate,
        loan_players: &mut Vec<SquadDecision>,
        transfer_players: &mut Vec<SquadDecision>,
    ) {
        /// From this age a senior reserve squad has stopped being a
        /// development pathway and started being a waiting room. Matches
        /// the reserve-ambition audit's own prime threshold.
        const PARKED_PRIME_AGE: u8 = 24;
        /// Past this age a loan is no longer a career step — the answer is
        /// a permanent move to a club that will pick him.
        const LOAN_VIABLE_MAX_AGE: u8 = 27;
        /// Observable level within this much of the first team's promotion
        /// bar means he is genuine cover, not a forgotten man.
        const PROMOTION_REACH: u8 = 6;
        /// He must have had a full season down there before this fires —
        /// a player just demoted may yet win his place back.
        const SETTLED_DAYS: i64 = 300;

        // The reserve side still has to field a team on Saturday. Track
        // how many uncommitted players each group has left so the pass
        // stops at a usable XI instead of emptying the squad — the same
        // fielding footprint the youth development-loan pass respects.
        let mut group_remaining: HashMap<PlayerFieldPositionGroup, usize> = HashMap::new();
        for player in team.players.iter() {
            if player.is_on_loan() || player.contract.is_none() {
                continue;
            }
            if player.statuses.has(PlayerStatusType::Lst)
                || player.statuses.has(PlayerStatusType::Loa)
            {
                continue;
            }
            *group_remaining
                .entry(player.position().position_group())
                .or_default() += 1;
        }

        for player in team.players.iter() {
            let age = player.age(date);
            if age < PARKED_PRIME_AGE {
                continue;
            }
            if player.is_on_loan() || player.is_force_match_selection {
                continue;
            }
            if player.contract.is_none()
                || player
                    .contract
                    .as_ref()
                    .is_some_and(|c| c.contract_type == ContractType::Youth || c.is_transfer_listed)
            {
                continue;
            }
            if player.statuses.has(PlayerStatusType::Lst)
                || player.statuses.has(PlayerStatusType::Loa)
                || player.statuses.has(PlayerStatusType::Frt)
            {
                continue;
            }
            // A recent arrival — or a recent demotion — has not yet had the
            // season that would make this a verdict rather than a guess.
            if StuckCareerScan::club_tenure_days(player, date)
                .is_some_and(|days| days < SETTLED_DAYS)
            {
                continue;
            }
            if player.signing_protection_active(date) {
                continue;
            }

            // Knocking on the first team's door: real cover, and the weekly
            // rebalance will promote him the moment he clears the bar.
            let group = player.position().position_group();
            let level = AbilityEstimator::observable_level(player);
            if level + PROMOTION_REACH >= bar.floor(group) {
                continue;
            }

            // Never strip the reserve side below a fieldable XI.
            let remaining = group_remaining.get(&group).copied().unwrap_or(0);
            if remaining <= YouthDevelopmentLoanPolicy::min_field(group) {
                continue;
            }

            // The club's own view of him still gates the exit — a player it
            // genuinely rates as first-team material is not shipped out on
            // an age heuristic. Below the first team the labels no longer
            // shortcut this, so the classification is the inference: how he
            // actually compares with the senior squad.
            match asset_ctx.classify_in_squad(player, date, team.team_type) {
                SquadAssetClass::CorePlayer | SquadAssetClass::FirstTeamUseful => continue,
                SquadAssetClass::ProspectDevelopment => {
                    loan_players.push(SquadDecision::new(
                        team_idx,
                        player.id,
                        SquadDecision::NEEDS_FIRST_TEAM_MINUTES,
                    ));
                }
                SquadAssetClass::RotationUseful
                | SquadAssetClass::UnknownNeedsEvaluation
                | SquadAssetClass::TrueSurplus => {
                    if age <= LOAN_VIABLE_MAX_AGE {
                        loan_players.push(SquadDecision::new(
                            team_idx,
                            player.id,
                            SquadDecision::NEEDS_FIRST_TEAM_MINUTES,
                        ));
                    } else {
                        transfer_players.push(SquadDecision::new(
                            team_idx,
                            player.id,
                            SquadDecision::LACK_PLAYING_TIME,
                        ));
                    }
                }
            }
            if let Some(count) = group_remaining.get_mut(&group) {
                *count = count.saturating_sub(1);
            }
        }
    }
}

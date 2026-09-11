//! Who the club sends out on loan, and why.
//!
//! Three routes, all of them about a player who is not going to play here
//! this season: the positional surplus of a squad that only fields one side a
//! week, the boy who is ready for men's football but blocked, and the keeper
//! the goalkeeping department has asked to be given a season elsewhere. The
//! monthly utilization audit dispatches all three and owns what happens to
//! the decisions afterwards.

use chrono::NaiveDate;

use crate::club::staff::perception::AbilityEstimator;
use crate::transfers::loan::guard::LoanAssetGuard;
use crate::{Club, Person, PlayerFieldPositionGroup, PlayerStatusType, Team};

use super::decision::SquadDecision;
use super::depth::{
    KeeperLoanView, MainPromotionFloor, YouthDevelopmentLoanPolicy, YouthSquadDepth,
};

/// The club's loan-out sweep, squad by squad.
pub(in crate::club::core) struct LoanSweep;

impl LoanSweep {
    /// Collect development loan-outs for one non-competing squad (a youth
    /// side, or any non-main team without a league) by positional surplus.
    /// Such a side fields and rotates roughly one match a week, so it needs
    /// only so many per position; the rest are blocked depth that develops
    /// better playing senior football on loan. Keeps the best `keep` by the
    /// coach-observable level (visible skill + training — youth football
    /// produces no official ratings) and loans the remainder — a
    /// manager-pinned player in the surplus simply stays. Players already on
    /// loan / listed, or without a contract, are left alone. Contract type is
    /// deliberately not checked: this is the one path that loans both
    /// full-time and youth-contract prospects out.
    pub(in crate::club::core) fn positional_surplus(
        club: &Club,
        team: &Team,
        team_idx: usize,
        date: NaiveDate,
        keepers: &KeeperLoanView,
        out: &mut Vec<SquadDecision>,
    ) {
        for group in PlayerFieldPositionGroup::ALL {
            let keep = YouthSquadDepth::keep_for(group);
            let mut active: Vec<(u32, u8, bool)> = team
                .players
                .iter()
                .filter(|p| {
                    p.position().position_group() == group
                        && !p.is_on_loan()
                        && p.contract.is_some()
                        && !p.statuses.has(PlayerStatusType::Lst)
                        && !p.statuses.has(PlayerStatusType::Loa)
                })
                .map(|p| {
                    (
                        p.id,
                        AbilityEstimator::observable_level(p),
                        // A first-team-calibre player registered on a youth
                        // or league-less side is not "surplus depth" there
                        // — he is the club's own starter, filed in the
                        // wrong squad, and the weekly rebalance is about to
                        // promote him. He still COUNTS toward the squad's
                        // depth (he is on this roster today); he is simply
                        // never the body that leaves.
                        p.is_force_match_selection
                            || LoanAssetGuard::parent_holds_for(club, p, date),
                    )
                })
                .collect();
            if active.len() <= keep {
                continue;
            }
            // Keep the best `keep` by observable level; the rest are surplus.
            active.sort_by(|a, b| b.1.cmp(&a.1));
            for (player_id, _, pinned) in active.into_iter().skip(keep) {
                // A keeper the goalkeeping department is building around
                // is not surplus, however many keepers sit on this roster.
                if !pinned && !keepers.protects(player_id) {
                    out.push(SquadDecision::new(
                        team_idx,
                        player_id,
                        SquadDecision::YOUNG_DEVELOP,
                    ));
                }
            }
        }
    }

    /// Keepers the goalkeeping department has asked to be sent out for
    /// minutes.
    ///
    /// The depth-based passes cannot reach these men. A twenty-one-year-old
    /// third choice on the reserve side is not positional surplus (the group
    /// is not over-depth), is not below the promotion floor by enough to be
    /// a blocked youth-team prospect, and plays league football so the idle
    /// sweep never sees him — and he is nonetheless a keeper with three men
    /// in front of him and a career going nowhere. That is the standard
    /// route in real football and it was the one route the club could not
    /// choose deliberately.
    pub(in crate::club::core) fn keeper_department(
        team: &Team,
        team_idx: usize,
        keepers: &KeeperLoanView,
        out: &mut Vec<SquadDecision>,
    ) {
        for player in team.players.iter() {
            if !keepers.wants_out(player.id) {
                continue;
            }
            if player.is_on_loan()
                || player.contract.is_none()
                || player.is_force_match_selection
                || player.statuses.has(PlayerStatusType::Lst)
                || player.statuses.has(PlayerStatusType::Loa)
                || out.iter().any(|d| d.player_id == player.id)
            {
                continue;
            }
            out.push(SquadDecision::new(
                team_idx,
                player.id,
                SquadDecision::YOUNG_DEVELOP,
            ));
        }
    }

    /// Age-based development loans for ONE youth squad: a youngster old enough
    /// for senior football (>= [`YouthDevelopmentLoanPolicy::SENIOR_LOAN_AGE`])
    /// who won't make the first team (current ability below the main-team
    /// promotion floor at his position) should go out on loan for minutes
    /// rather than stagnate in the youth side. Complements
    /// [`Self::positional_surplus`]: that one loans positional *surplus*
    /// (deep groups); this one loans *blocked but ready* youngsters even when
    /// the group is not over-depth. Never strips a group below the minimum it
    /// needs to field a match, never re-flags a player the surplus pass already
    /// took, and never touches a promotion-bound prospect (the rebalance
    /// promotes him) or an on-loan / listed / pinned / contract-less player.
    pub(in crate::club::core) fn youth_development(
        club: &Club,
        team: &Team,
        team_idx: usize,
        main_floor: &MainPromotionFloor,
        keepers: &KeeperLoanView,
        date: NaiveDate,
        out: &mut Vec<SquadDecision>,
    ) {
        for group in PlayerFieldPositionGroup::ALL {
            let floor = main_floor.get(group);
            let min_field = YouthDevelopmentLoanPolicy::min_field(group);

            // Stay-eligible players in this group, excluding anyone the
            // surplus pass already flagged. (id, age, current ability).
            let active: Vec<(u32, u8, u8, bool)> = team
                .players
                .iter()
                .filter(|p| {
                    p.position().position_group() == group
                        && !p.is_on_loan()
                        && p.contract.is_some()
                        && !p.is_force_match_selection
                        && !p.statuses.has(PlayerStatusType::Lst)
                        && !p.statuses.has(PlayerStatusType::Loa)
                        && !out.iter().any(|d| d.player_id == p.id)
                        // The keeper the first team has started travelling
                        // with is not stagnating in the youth side — he is
                        // exactly where the club decided to put him.
                        && !keepers.protects(p.id)
                })
                .map(|p| {
                    (
                        p.id,
                        p.age(date),
                        p.player_attributes.current_ability,
                        // The boy who is already good enough to start for
                        // the first team is a promotion, not a development
                        // loan. He still counts toward the youth side's
                        // fielding minimum — he is on this roster today.
                        LoanAssetGuard::parent_holds_for(club, p, date),
                    )
                })
                .collect();

            let mut remaining = active.len();
            if remaining <= min_field {
                continue;
            }

            // Senior-ready youngsters below the first-team bar, oldest first
            // (most ready for senior football, least served by another year of
            // youth rotation). Loan them down to the fielding minimum.
            let mut candidates: Vec<(u32, u8)> = active
                .iter()
                .filter(|(_, age, ca, parent_holds)| {
                    !parent_holds
                        && *age >= YouthDevelopmentLoanPolicy::SENIOR_LOAN_AGE
                        && *ca < floor
                })
                .map(|(id, age, _, _)| (*id, *age))
                .collect();
            candidates.sort_by(|a, b| b.1.cmp(&a.1));

            for (player_id, _age) in candidates {
                if remaining <= min_field {
                    break;
                }
                out.push(SquadDecision::new(
                    team_idx,
                    player_id,
                    SquadDecision::YOUNG_DEVELOP,
                ));
                remaining -= 1;
            }
        }
    }
}

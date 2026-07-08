use super::Club;
use crate::club::staff::perception::PotentialEstimator;
use crate::club::team::squad::{SquadAssetClass, SquadAssetContext, SquadEvidenceContext};
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::pipeline::{LoanOutCandidate, LoanOutReason, LoanOutStatus};
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{
    ContractType, Person, PlayerFieldPositionGroup, PlayerStatusType, ReputationLevel, Team,
    TransferItem,
};
use chrono::NaiveDate;
use log::debug;
use std::collections::HashSet;

/// Days after a permanent / loan move during which a player's idle days are
/// not yet read as underutilization — he hasn't had a fair chance to break
/// into the squad. Mirrors the post-transfer settling window the happiness
/// model uses for playing-time grievances.
const RECENT_TRANSFER_GRACE_DAYS: i64 = 30;

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

        // Central squad-asset classifier, built once against the senior
        // squad's level. Every non-main player is measured against it so a
        // reserve player who is really first-team-useful (or a development
        // prospect) is routed correctly instead of being transfer-listed
        // off idle days alone.
        let asset_ctx = SquadAssetContext::build(self, date);

        // Collect underutilized player decisions
        let mut loan_players: Vec<(usize, u32, String)> = Vec::new();
        let mut transfer_players: Vec<(usize, u32, String)> = Vec::new();

        // Per-group main-team promotion floor — the current ability at/above
        // which a non-main player is promoted to the first team by the weekly
        // `rebalance_squads`. The youth development-loan pass only fires BELOW
        // this bar, so a promotion-bound prospect is left for the rebalance to
        // promote rather than loaned away.
        let main_floor = MainPromotionFloor::snapshot(&self.teams.teams[main_idx]);

        for (ti, team) in self.teams.iter().enumerate() {
            if ti == main_idx {
                continue;
            }

            // Squads that don't play official/league football — youth sides
            // (U18..U23) AND any non-main team without a league — never
            // accumulate the official appearances the idle-days signal below
            // relies on (friendlies don't count), so the early-season gate
            // would skip them every single tick (a youth side could carry
            // three keepers forever; a league-less reserve side never sheds
            // anyone). Assess them on positional SURPLUS instead — generic
            // across positions and contract types (the one path that loans
            // full-time *and* youth-contract prospects out). Youth sides
            // additionally get age-based development loans: a senior-ready
            // youngster blocked from the first team should go out for minutes,
            // not stagnate in the youth squad.
            let plays_league_football = team.league_id.is_some() && !team.team_type.is_youth();
            if !plays_league_football {
                Self::collect_surplus_loans(team, ti, &mut loan_players);
                if team.team_type.is_youth() {
                    Self::collect_youth_development_loans(
                        team,
                        ti,
                        &main_floor,
                        date,
                        &mut loan_players,
                    );
                }
                continue;
            }

            // Thin-sample protection: until this league squad has played a
            // meaningful number of official matches, a player's idle days
            // don't yet distinguish "surplus" from "hasn't had his chance".
            if SquadEvidenceContext::from_squad(&team.players).is_early_season() {
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

                // Missing minutes that aren't a squad-management signal: a
                // player out injured, recovering, suspended, or short of
                // match fitness hasn't been benched by choice, so his idle
                // days say nothing about whether the club wants him.
                // Likewise a player still inside his post-transfer grace
                // period hasn't had a fair chance to break in yet.
                if !player.is_ready_for_match() {
                    continue;
                }
                if player
                    .days_since_transfer(date)
                    .map(|d| d < RECENT_TRANSFER_GRACE_DAYS)
                    .unwrap_or(false)
                {
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
                // Board decisions read the observable ceiling — the
                // hidden biological PA is not visible to clubs.
                let pa = PotentialEstimator::observable_ceiling(player, date);

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

                // Central squad-asset policy gates the disposal. Useful
                // seniors (core / first-team / credible rotation) and
                // players the club hasn't been able to evaluate yet are
                // never shipped out on idle-days evidence alone; a blocked
                // young prospect goes out on loan for minutes, not the
                // transfer list; only genuine surplus falls through to the
                // existing transfer/loan profile below.
                match asset_ctx.classify(player, date) {
                    SquadAssetClass::CorePlayer
                    | SquadAssetClass::FirstTeamUseful
                    | SquadAssetClass::RotationUseful
                    | SquadAssetClass::UnknownNeedsEvaluation => continue,
                    SquadAssetClass::ProspectDevelopment => {
                        loan_players.push((ti, player.id, "dec_reason_young_develop".to_string()));
                        continue;
                    }
                    SquadAssetClass::TrueSurplus => {}
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

        // In-season size discipline. The board flags `squad_excess` once the
        // total squad exceeds its ceiling by more than five, but nothing
        // consumed it — so an over-limit squad that tripped no per-position
        // glut just bloated all season. Give that determination a consumer:
        // when over the ceiling, list the worst genuine surplus across ALL
        // teams (including the main squad, which the idle sweep above skips),
        // worst-first, until the excess is cleared.
        const SIZE_TRIM_MARGIN: usize = 5;
        if total_squad > max_squad + SIZE_TRIM_MARGIN {
            let already: HashSet<u32> = loan_players
                .iter()
                .chain(transfer_players.iter())
                .map(|(_, id, _)| *id)
                .collect();
            let excess = total_squad - max_squad;
            // (team_idx, id, ca, age) — rank low CA first, then older first.
            let mut surplus: Vec<(usize, u32, u8, u8)> = Vec::new();
            for (ti, team) in self.teams.iter().enumerate() {
                for player in team.players.iter() {
                    if already.contains(&player.id)
                        || player.is_on_loan()
                        || player.is_force_match_selection
                        || player
                            .contract
                            .as_ref()
                            .map(|c| c.is_transfer_listed)
                            .unwrap_or(true)
                    {
                        continue;
                    }
                    if !matches!(
                        asset_ctx.classify(player, date),
                        SquadAssetClass::TrueSurplus
                    ) {
                        continue;
                    }
                    surplus.push((
                        ti,
                        player.id,
                        player.player_attributes.current_ability,
                        player.age(date),
                    ));
                }
            }
            surplus.sort_by(|a, b| a.2.cmp(&b.2).then(b.3.cmp(&a.3)));
            for (ti, id, _, _) in surplus.into_iter().take(excess) {
                transfer_players.push((ti, id, "dec_reason_underutilized".to_string()));
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

    /// Collect development loan-outs for one non-competing squad (a youth
    /// side, or any non-main team without a league) by positional surplus.
    /// Such a side fields and rotates roughly one match a week, so it needs
    /// only so many per position; the rest are blocked depth that develops
    /// better playing senior football on loan. Keeps the best `keep` by
    /// current ability and loans the remainder — a manager-pinned player in
    /// the surplus simply stays. Players already on loan / listed, or without
    /// a contract, are left alone. Contract type is deliberately not checked:
    /// this is the one path that loans both full-time and youth-contract
    /// prospects out.
    fn collect_surplus_loans(
        team: &Team,
        team_idx: usize,
        loan_players: &mut Vec<(usize, u32, String)>,
    ) {
        for group in [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ] {
            let keep = YouthSquadDepth::keep_for(group);
            let mut active: Vec<(u32, u8, bool)> = team
                .players
                .iter()
                .filter(|p| {
                    p.position().position_group() == group
                        && !p.is_on_loan()
                        && p.contract.is_some()
                        && {
                            let s = p.statuses.get();
                            !s.contains(&PlayerStatusType::Lst)
                                && !s.contains(&PlayerStatusType::Loa)
                        }
                })
                .map(|p| {
                    (
                        p.id,
                        p.player_attributes.current_ability,
                        p.is_force_match_selection,
                    )
                })
                .collect();
            if active.len() <= keep {
                continue;
            }
            // Keep the best `keep` by current ability; the rest are surplus.
            active.sort_by(|a, b| b.1.cmp(&a.1));
            for (player_id, _, pinned) in active.into_iter().skip(keep) {
                if !pinned {
                    loan_players.push((
                        team_idx,
                        player_id,
                        "dec_reason_young_develop".to_string(),
                    ));
                }
            }
        }
    }

    /// Age-based development loans for ONE youth squad: a youngster old enough
    /// for senior football (>= `YouthDevelopmentLoanPolicy::SENIOR_LOAN_AGE`)
    /// who won't make the first team (current ability below the main-team
    /// promotion floor at his position) should go out on loan for minutes
    /// rather than stagnate in the youth side. Complements
    /// [`Self::collect_surplus_loans`]: that one loans positional *surplus*
    /// (deep groups); this one loans *blocked but ready* youngsters even when
    /// the group is not over-depth. Never strips a group below the minimum it
    /// needs to field a match, never re-flags a player the surplus pass already
    /// took, and never touches a promotion-bound prospect (the rebalance
    /// promotes him) or an on-loan / listed / pinned / contract-less player.
    fn collect_youth_development_loans(
        team: &Team,
        team_idx: usize,
        main_floor: &MainPromotionFloor,
        date: NaiveDate,
        loan_players: &mut Vec<(usize, u32, String)>,
    ) {
        for group in [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ] {
            let floor = main_floor.get(group);
            let min_field = YouthDevelopmentLoanPolicy::min_field(group);

            // Stay-eligible players in this group, excluding anyone the
            // surplus pass already flagged. (id, age, current ability).
            let active: Vec<(u32, u8, u8)> = team
                .players
                .iter()
                .filter(|p| {
                    p.position().position_group() == group
                        && !p.is_on_loan()
                        && p.contract.is_some()
                        && !p.is_force_match_selection
                        && {
                            let s = p.statuses.get();
                            !s.contains(&PlayerStatusType::Lst)
                                && !s.contains(&PlayerStatusType::Loa)
                        }
                        && !loan_players.iter().any(|(_, id, _)| *id == p.id)
                })
                .map(|p| (p.id, p.age(date), p.player_attributes.current_ability))
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
                .filter(|(_, age, ca)| {
                    *age >= YouthDevelopmentLoanPolicy::SENIOR_LOAN_AGE && *ca < floor
                })
                .map(|(id, age, _)| (*id, *age))
                .collect();
            candidates.sort_by(|a, b| b.1.cmp(&a.1));

            for (player_id, _age) in candidates {
                if remaining <= min_field {
                    break;
                }
                loan_players.push((team_idx, player_id, "dec_reason_young_develop".to_string()));
                remaining -= 1;
            }
        }
    }
}

/// Per-group main-team promotion floor: the current ability at/above which a
/// non-main player is promoted to the first team by the weekly
/// `rebalance_squads`. The youth development-loan pass reads it so a
/// promotion-bound prospect (at/above the bar) is left for the rebalance to
/// promote rather than loaned away. Depths mirror `MIN_MAIN_DEPTH` in
/// squad.rs — keep them in sync.
struct MainPromotionFloor {
    floors: [(PlayerFieldPositionGroup, u8); 4],
}

impl MainPromotionFloor {
    /// CA an understrength group falls back to (squad.rs `DEPTH_GAP_FLOOR`):
    /// when the main is short at a position any decent youth fills the gap, so
    /// there is no "below the first team" band to loan from.
    const DEPTH_GAP_FLOOR: u8 = 60;

    /// Below this many players at a group the main team is short there.
    /// Mirrors squad.rs `MIN_MAIN_DEPTH`.
    fn min_depth(group: PlayerFieldPositionGroup) -> usize {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 2,
            PlayerFieldPositionGroup::Defender => 6,
            PlayerFieldPositionGroup::Midfielder => 6,
            PlayerFieldPositionGroup::Forward => 4,
        }
    }

    fn snapshot(main: &Team) -> Self {
        MainPromotionFloor {
            floors: [
                (
                    PlayerFieldPositionGroup::Goalkeeper,
                    Self::for_group(main, PlayerFieldPositionGroup::Goalkeeper),
                ),
                (
                    PlayerFieldPositionGroup::Defender,
                    Self::for_group(main, PlayerFieldPositionGroup::Defender),
                ),
                (
                    PlayerFieldPositionGroup::Midfielder,
                    Self::for_group(main, PlayerFieldPositionGroup::Midfielder),
                ),
                (
                    PlayerFieldPositionGroup::Forward,
                    Self::for_group(main, PlayerFieldPositionGroup::Forward),
                ),
            ],
        }
    }

    fn for_group(main: &Team, group: PlayerFieldPositionGroup) -> u8 {
        let (count, worst) = main
            .players
            .iter()
            .filter(|p| p.position().position_group() == group)
            .map(|p| p.player_attributes.current_ability)
            .fold((0usize, u8::MAX), |(c, w), a| (c + 1, w.min(a)));
        if count < Self::min_depth(group) {
            Self::DEPTH_GAP_FLOOR
        } else {
            worst.saturating_add(1)
        }
    }

    fn get(&self, group: PlayerFieldPositionGroup) -> u8 {
        self.floors
            .iter()
            .find(|(g, _)| *g == group)
            .map(|(_, f)| *f)
            .unwrap_or(u8::MAX)
    }
}

/// Per-position depth a single non-competing squad keeps before the remainder
/// are loaned out for development. Smaller than a senior squad's depth: such a
/// side plays roughly once a week, so a third keeper or a deep outfield
/// reserve never sees minutes and develops better on loan.
struct YouthSquadDepth;

impl YouthSquadDepth {
    fn keep_for(group: PlayerFieldPositionGroup) -> usize {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 2,
            PlayerFieldPositionGroup::Defender => 7,
            PlayerFieldPositionGroup::Midfielder => 7,
            PlayerFieldPositionGroup::Forward => 5,
        }
    }
}

/// Policy for the age-based youth development-loan pass
/// ([`Club::collect_youth_development_loans`]).
struct YouthDevelopmentLoanPolicy;

impl YouthDevelopmentLoanPolicy {
    /// Age at/above which a youth player is treated as ready for senior loan
    /// football. Below it he keeps developing in the youth side rather than
    /// being shipped to a senior club too early.
    const SENIOR_LOAN_AGE: u8 = 18;

    /// Players a youth squad must retain per group so it can still field a
    /// match — the development-loan pass never strips a group below this. A
    /// 4-4-2 fielding-XI footprint; deeper squads loan the senior-ready fringe
    /// above it. Deliberately below [`YouthSquadDepth::keep_for`]: the surplus
    /// pass trims to a comfortable rotation depth, this pass then loans the
    /// blocked-but-ready players down toward a usable XI.
    fn min_field(group: PlayerFieldPositionGroup) -> usize {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 1,
            PlayerFieldPositionGroup::Defender => 4,
            PlayerFieldPositionGroup::Midfielder => 4,
            PlayerFieldPositionGroup::Forward => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
        PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, PlayerSquadStatus, StaffCollection, Team, TeamBuilder,
        TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::{NaiveDate, NaiveTime};

    /// Fixtures for the underutilization audit: a CA-130 first team plus a
    /// reserve squad. The reserve always carries one "busy" regular so the
    /// thin-sample gate is cleared and the audit actually runs.
    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()
        }

        /// `played` seeds official appearances (early-season sample);
        /// `idle` drives the underutilization trigger; condition is set
        /// full so the new match-readiness guard doesn't short-circuit.
        fn player(
            id: u32,
            ca: u8,
            pa: u8,
            age: u8,
            status: PlayerSquadStatus,
            played: u16,
            idle: u16,
        ) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ca;
            attrs.potential_ability = pa;
            attrs.condition = 10_000; // fully fit -> is_ready_for_match()
            attrs.days_since_last_match = idle;
            let mut contract =
                PlayerClubContract::new(20_000, NaiveDate::from_ymd_opt(2030, 6, 30).unwrap());
            contract.squad_status = status;
            let mut p = PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("U".into(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(2026 - age as i32, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap();
            p.statistics.played = played;
            p
        }

        /// A full-time youth goalkeeper (the case the user hit: U19 keepers
        /// on full contracts, so the youth-contract skip doesn't apply).
        fn gk(id: u32, ca: u8, age: u8) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ca;
            attrs.potential_ability = ca.saturating_add(30);
            attrs.condition = 10_000;
            let mut contract =
                PlayerClubContract::new(20_000, NaiveDate::from_ymd_opt(2030, 6, 30).unwrap());
            contract.squad_status = PlayerSquadStatus::NotYetSet;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("G".into(), format!("K{id}")))
                .birth_date(NaiveDate::from_ymd_opt(2026 - age as i32, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::Goalkeeper,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap()
        }

        fn team(id: u32, tt: TeamType, players: Vec<Player>) -> Team {
            TeamBuilder::new()
                .id(id)
                .league_id(Some(1))
                .club_id(100)
                .name(format!("t{id}"))
                .slug(format!("t{id}"))
                .team_type(tt)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        /// Like [`Self::team`] but with NO league — a friendly-only squad,
        /// the case the early-season idle gate used to skip forever.
        fn team_no_league(id: u32, tt: TeamType, players: Vec<Player>) -> Team {
            TeamBuilder::new()
                .id(id)
                .league_id(None)
                .club_id(100)
                .name(format!("t{id}"))
                .slug(format!("t{id}"))
                .team_type(tt)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn club(reserve_extra: Vec<Player>) -> Club {
            let main: Vec<Player> = (1..=5)
                .map(|i| Fx::player(i, 130, 130, 27, PlayerSquadStatus::FirstTeamRegular, 20, 0))
                .collect();
            // id 90: a "busy" reserve regular so the reserve squad clears
            // the early-season sample gate (and is itself skipped, having
            // enough games).
            let mut reserve = vec![Fx::player(
                90,
                100,
                100,
                24,
                PlayerSquadStatus::FirstTeamSquadRotation,
                12,
                0,
            )];
            reserve.extend(reserve_extra);
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(1_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![
                    Fx::team(10, TeamType::Main, main),
                    Fx::team(11, TeamType::Reserve, reserve),
                ]),
                ClubFacilities::default(),
            )
        }

        fn has(club: &Club, id: u32, s: PlayerStatusType) -> bool {
            club.teams.teams.iter().any(|t| {
                t.players
                    .players
                    .iter()
                    .any(|p| p.id == id && p.statuses.get().contains(&s))
            })
        }

        fn listed(club: &Club, id: u32) -> bool {
            Fx::has(club, id, PlayerStatusType::Lst) || Fx::has(club, id, PlayerStatusType::Loa)
        }
    }

    /// A blocked young prospect with a real squad sample and no minutes is
    /// loan-listed for development, never transfer-listed.
    #[test]
    fn blocked_prospect_is_loan_listed_not_transfer_listed() {
        let mut club = Fx::club(vec![Fx::player(
            200,
            100,
            130,
            19,
            PlayerSquadStatus::NotYetSet,
            0,
            150,
        )]);
        club.audit_squad_utilization(Fx::date());
        assert!(
            Fx::has(&club, 200, PlayerStatusType::Loa),
            "a blocked prospect should be loan-listed"
        );
        assert!(
            !Fx::has(&club, 200, PlayerStatusType::Lst),
            "a blocked prospect must NOT be transfer-listed"
        );
    }

    /// A reserve player competitive with the first team is not listed just
    /// because he's been idle.
    #[test]
    fn competitive_reserve_player_is_not_listed() {
        let mut club = Fx::club(vec![Fx::player(
            201,
            125,
            125,
            30,
            PlayerSquadStatus::NotYetSet,
            0,
            150,
        )]);
        club.audit_squad_utilization(Fx::date());
        assert!(
            !Fx::listed(&club, 201),
            "a reserve player competitive with the first team must not be listed off idle days"
        );
    }

    /// A genuinely surplus, low-ability older reserve player can still be
    /// transfer-listed.
    #[test]
    fn low_ability_older_surplus_is_still_listed() {
        let mut club = Fx::club(vec![Fx::player(
            202,
            55,
            60,
            35,
            PlayerSquadStatus::NotYetSet,
            0,
            150,
        )]);
        club.audit_squad_utilization(Fx::date());
        assert!(
            Fx::has(&club, 202, PlayerStatusType::Lst),
            "a clearly-surplus low-ability veteran should still be transfer-listed"
        );
    }

    /// An injured player's idle days are not a squad-management signal — he
    /// is left alone.
    #[test]
    fn injured_player_is_not_listed() {
        let mut injured = Fx::player(203, 55, 60, 33, PlayerSquadStatus::NotYetSet, 0, 200);
        injured.player_attributes.is_injured = true;
        let mut club = Fx::club(vec![injured]);
        club.audit_squad_utilization(Fx::date());
        assert!(
            !Fx::listed(&club, 203),
            "an injured player must not be transfer-listed off idle days"
        );
    }

    /// The reported case: a U19 squad carries THREE keepers on full-time
    /// contracts. The youth-contract skip doesn't apply (they're full-time)
    /// and the old early-season gate skipped the squad forever (youth
    /// leagues are friendlies → no official minutes). The positional-surplus
    /// path keeps the best two and loans the third out for development.
    #[test]
    fn youth_team_surplus_keeper_is_loan_listed_for_development() {
        let main: Vec<Player> = (1..=5)
            .map(|i| Fx::player(i, 130, 130, 27, PlayerSquadStatus::FirstTeamRegular, 20, 0))
            .collect();
        let u19 = vec![
            Fx::gk(401, 90, 18),
            Fx::gk(402, 85, 18),
            Fx::gk(403, 80, 18),
        ];
        let mut club = Club::new(
            100,
            "Club".to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![
                Fx::team(10, TeamType::Main, main),
                Fx::team(11, TeamType::U19, u19),
            ]),
            ClubFacilities::default(),
        );

        club.audit_squad_utilization(Fx::date());

        assert!(
            Fx::has(&club, 403, PlayerStatusType::Loa),
            "the surplus third keeper must be loan-listed for development"
        );
        assert!(
            !Fx::has(&club, 401, PlayerStatusType::Loa)
                && !Fx::has(&club, 402, PlayerStatusType::Loa),
            "the two best keepers stay at the club"
        );
    }

    /// B: a non-main team WITHOUT a league plays only friendlies, so the
    /// official-appearance idle gate used to skip it forever. It is now
    /// assessed on positional surplus like a youth side, so depth beyond the
    /// rotation need is loan-listed.
    #[test]
    fn league_less_reserve_surplus_is_loan_listed() {
        let main: Vec<Player> = (1..=5)
            .map(|i| Fx::player(i, 130, 130, 27, PlayerSquadStatus::FirstTeamRegular, 20, 0))
            .collect();
        // Nine midfielders on a league-less Second team — keep_for(MID)=7, so
        // the two weakest are surplus and must be loan-listed.
        let reserve: Vec<Player> = (300..309)
            .map(|i| Fx::player(i, 90, 90, 24, PlayerSquadStatus::NotYetSet, 0, 0))
            .collect();
        let mut club = Club::new(
            100,
            "Club".to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![
                Fx::team(10, TeamType::Main, main),
                Fx::team_no_league(11, TeamType::Second, reserve),
            ]),
            ClubFacilities::default(),
        );

        club.audit_squad_utilization(Fx::date());

        let loaned = (300..309)
            .filter(|id| Fx::has(&club, *id, PlayerStatusType::Loa))
            .count();
        assert_eq!(
            loaned, 2,
            "surplus beyond keep_for(MID)=7 on a league-less side must be loan-listed"
        );
    }

    /// C: two U19 keepers blocked by a three-deep main GK line. They are NOT a
    /// positional surplus (<= keep_for(GK)=2) but they ARE senior-ready and
    /// can't break in, so the older one is sent out on a development loan; the
    /// youth side keeps one to field a match.
    #[test]
    fn senior_ready_youth_blocked_from_first_team_is_loaned() {
        let main = vec![Fx::gk(1, 150, 28), Fx::gk(2, 150, 28), Fx::gk(3, 150, 28)];
        let u19 = vec![Fx::gk(401, 80, 19), Fx::gk(402, 80, 18)];
        let mut club = Club::new(
            100,
            "Club".to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![
                Fx::team(10, TeamType::Main, main),
                Fx::team(11, TeamType::U19, u19),
            ]),
            ClubFacilities::default(),
        );

        club.audit_squad_utilization(Fx::date());

        assert!(
            Fx::has(&club, 401, PlayerStatusType::Loa),
            "the older blocked U19 keeper should get a development loan"
        );
        assert!(
            !Fx::has(&club, 402, PlayerStatusType::Loa),
            "the youth side must keep one keeper to field a match"
        );
    }

    /// C never strips a youth group below its fielding minimum — a side's ONLY
    /// keeper is kept even when blocked and senior-ready.
    #[test]
    fn last_youth_keeper_is_never_loaned() {
        let main = vec![Fx::gk(1, 150, 28), Fx::gk(2, 150, 28), Fx::gk(3, 150, 28)];
        let u19 = vec![Fx::gk(401, 80, 19)];
        let mut club = Club::new(
            100,
            "Club".to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![
                Fx::team(10, TeamType::Main, main),
                Fx::team(11, TeamType::U19, u19),
            ]),
            ClubFacilities::default(),
        );

        club.audit_squad_utilization(Fx::date());

        assert!(
            !Fx::has(&club, 401, PlayerStatusType::Loa),
            "a youth side's only keeper must never be loaned out"
        );
    }

    /// C respects the promotion bar: a youngster above the main-team promotion
    /// floor is left for the rebalance to promote, not loaned; only the
    /// below-floor (genuinely blocked) keeper goes out.
    #[test]
    fn promotion_ready_youth_is_left_for_promotion() {
        // Main two keepers at CA 90 → promotion floor = 91.
        let main = vec![Fx::gk(1, 90, 28), Fx::gk(2, 90, 28)];
        // 401 clears the floor (promotion-bound); 402 is below it (blocked).
        let u19 = vec![Fx::gk(401, 120, 19), Fx::gk(402, 70, 19)];
        let mut club = Club::new(
            100,
            "Club".to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![
                Fx::team(10, TeamType::Main, main),
                Fx::team(11, TeamType::U19, u19),
            ]),
            ClubFacilities::default(),
        );

        club.audit_squad_utilization(Fx::date());

        assert!(
            !Fx::has(&club, 401, PlayerStatusType::Loa),
            "a promotion-ready youth keeper must be kept for promotion, not loaned"
        );
        assert!(
            Fx::has(&club, 402, PlayerStatusType::Loa),
            "the blocked below-floor keeper should still get a development loan"
        );
    }
}

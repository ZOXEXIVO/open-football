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

        for (ti, team) in self.teams.iter().enumerate() {
            if ti == main_idx {
                continue;
            }

            // Youth squads (U18..U23) play friendly youth leagues, so they
            // never accumulate the official appearances the idle-days signal
            // below relies on — the early-season gate would skip them every
            // single tick, which is why a youth side can carry three keepers
            // forever. A youth team also plays only once a week, so depth
            // beyond what one match needs never gets minutes and is better
            // off on a development loan. Assess youth squads on POSITIONAL
            // SURPLUS instead — generic across positions and contract types
            // (this is the one path that loans full-time *and* youth-contract
            // prospects out).
            if team.team_type.is_youth() {
                Self::collect_youth_surplus_loans(team, ti, &mut loan_players);
                continue;
            }

            // Thin-sample protection: until this squad has played a
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

    /// Collect development loan-outs for one youth squad by positional
    /// surplus. A youth side fields and rotates one match a week, so it
    /// needs only so many per position; the rest are blocked depth that
    /// develops better playing senior football on loan. Keeps the best
    /// `keep` by current ability and loans the remainder — a manager-pinned
    /// player in the surplus simply stays. Players already on loan / listed,
    /// or without a contract, are left alone. Contract type is deliberately
    /// not checked: this is the one path that loans both full-time and
    /// youth-contract prospects out.
    fn collect_youth_surplus_loans(
        team: &Team,
        team_idx: usize,
        loan_players: &mut Vec<(usize, u32, String)>,
    ) {
        use PlayerFieldPositionGroup as G;
        for group in [G::Goalkeeper, G::Defender, G::Midfielder, G::Forward] {
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
}

/// Per-position depth a single youth squad keeps before the remainder are
/// loaned out for development. Smaller than a senior squad's depth: a youth
/// side plays once a week, so a third keeper or a deep outfield reserve
/// never sees minutes and develops better on loan.
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
        let u19 = vec![Fx::gk(401, 90, 18), Fx::gk(402, 85, 18), Fx::gk(403, 80, 18)];
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
}

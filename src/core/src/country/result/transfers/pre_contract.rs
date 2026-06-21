//! Pre-contract (Bosman) staging for the country transfer market.
//!
//! Real clubs don't wait for a useful player to become an unattached free
//! agent — once he's inside the final months of an expiring deal and his
//! club clearly won't keep him, a rival agrees a *pre-contract*: a free
//! transfer signed now, effective the day the current contract lapses.
//! The player is NOT moved early; he plays out his deal, then the expiry
//! pass in `handle_free_agents` routes the free move to the agreed club
//! instead of dropping him into the open pool.
//!
//! This module owns the *staging* half — deciding who gets pre-signed by
//! whom and on what terms — and stamps a [`PreContractAgreement`] onto the
//! player. Execution lives in `free_agents.rs`
//! (`collect_pre_contract_signings`), which reuses the ordinary in-country
//! free-agent signing path.
//!
//! Domestic-only for now: the buyer is always a club in the same country,
//! so the move executes through `execute_transfer_within_country`. A
//! cross-border Bosman would need the deferred cross-country queue and is
//! left for later.

use super::config::TransferConfig;
use super::free_agent_market_calc::{BuyerRoleFit, FreeAgentMarketCalculator};
use super::types::can_club_accept_player;
use crate::club::player::calculators::WageCalculator;
use crate::club::player::contract::RENEWAL_REJECTED_LABEL;
use crate::club::player::transfer::{MarketStage, PreContractAgreement};
use crate::transfers::pipeline::TransferRequestStatus;
use crate::utils::IntegerUtils;
use crate::{Country, Person, PlayerFieldPositionGroup, PlayerSquadStatus, PlayerStatusType};
use chrono::NaiveDate;
use log::debug;

/// A staged pre-contract decision produced by the read phase, applied to
/// the player in the write phase. Split so the matcher can scan clubs
/// immutably while it decides, then mutate one player at a time.
struct StagedPreContract {
    player_id: u32,
    agreement: PreContractAgreement,
    to_club_id: u32,
}

/// One expiring player worth pre-signing, lifted out of the roster scan so
/// the buyer search doesn't hold a borrow on the whole club list.
struct LeavingPlayer {
    player_id: u32,
    current_club_id: u32,
    group: PlayerFieldPositionGroup,
    ability: u8,
    current_reputation: i16,
    age: u8,
    days_to_expiry: i64,
}

pub(super) struct PreContractManager;

impl PreContractManager {
    /// Stage pre-contracts for the country. Runs year-round (a Bosman is
    /// window-independent) and is capped hard so most expiring players
    /// still run their deal down and reach the open market.
    pub(super) fn stage(country: &mut Country, date: NaiveDate, config: &TransferConfig) {
        let cap = config.max_pre_contracts_per_country_per_day;
        if cap == 0 {
            return;
        }

        // Phase A (read): find leaving players and a domestic buyer for
        // each. The chosen agreements are collected; nothing mutates yet.
        let leaving = Self::collect_leaving_players(country, date, config);
        if leaving.is_empty() {
            return;
        }

        let mut staged: Vec<StagedPreContract> = Vec::new();
        for player in &leaving {
            if staged.len() >= cap {
                break;
            }
            // A pre-contract becomes more likely as expiry nears: a deal
            // six months out is rare, one a few weeks out is common.
            let progress = ((config.pre_contract_window_days - player.days_to_expiry).max(0)
                as f32)
                / config.pre_contract_window_days.max(1) as f32;
            let chance = 3.0 + progress * 9.0; // 3% at the window edge → 12% near expiry
            let roll = IntegerUtils::random(1, 1000) as f32 / 10.0;
            if roll > chance {
                continue;
            }
            if let Some(decision) = Self::choose_buyer(country, player, date) {
                staged.push(decision);
            }
        }

        if staged.is_empty() {
            return;
        }

        // Phase B (write): stamp the agreement onto each chosen player.
        for decision in staged {
            let Some(player) = country
                .clubs
                .iter_mut()
                .flat_map(|c| c.teams.teams.iter_mut())
                .flat_map(|t| t.players.players.iter_mut())
                .find(|p| p.id == decision.player_id)
            else {
                continue;
            };
            // Re-check the player didn't change hands or pick up an
            // agreement between the read and write phases.
            if player.pending_pre_contract().is_some() || player.contract.is_none() {
                continue;
            }
            debug!(
                "Pre-contract staged: player {} (id {}) → club {} effective on expiry \
                 (${}/y, {}y)",
                player.full_name,
                player.id,
                decision.to_club_id,
                decision.agreement.annual_wage,
                decision.agreement.contract_years,
            );
            player.stage_pre_contract(decision.agreement);
        }
    }

    /// Scan every club's roster for players in the final months of an
    /// expiring deal who are clearly heading for the exit and worth
    /// pre-signing.
    fn collect_leaving_players(
        country: &Country,
        date: NaiveDate,
        config: &TransferConfig,
    ) -> Vec<LeavingPlayer> {
        let mut out: Vec<LeavingPlayer> = Vec::new();
        for club in &country.clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    if player.is_on_loan()
                        || player.is_retired()
                        || player.is_force_match_selection
                        || player.pending_pre_contract().is_some()
                    {
                        continue;
                    }
                    let Some(contract) = player.contract.as_ref() else {
                        continue;
                    };
                    let days_to_expiry = (contract.expiration - date).num_days();
                    if days_to_expiry < 1 || days_to_expiry > config.pre_contract_window_days {
                        continue;
                    }
                    let ability = player.player_attributes.current_ability;
                    if ability < config.pre_contract_min_ability {
                        continue;
                    }
                    if !Self::is_leaving(player, contract, date) {
                        continue;
                    }
                    out.push(LeavingPlayer {
                        player_id: player.id,
                        current_club_id: club.id,
                        group: player.position().position_group(),
                        ability,
                        current_reputation: player.player_attributes.current_reputation,
                        age: player.age(date),
                        days_to_expiry,
                    });
                }
            }
        }
        out
    }

    /// A player is "leaving" — and so a valid pre-contract target — when
    /// his club has signalled it won't keep him: he is listed / released,
    /// surplus to the squad plan, has just had renewal talks collapse, or
    /// is being actively chased by rival clubs while running his deal down.
    fn is_leaving(
        player: &crate::Player,
        contract: &crate::PlayerClubContract,
        date: NaiveDate,
    ) -> bool {
        let statuses = player.statuses.get();
        if statuses.contains(&PlayerStatusType::Lst) || statuses.contains(&PlayerStatusType::Frt) {
            return true;
        }
        if contract.is_transfer_listed
            || matches!(contract.squad_status, PlayerSquadStatus::NotNeeded)
        {
            return true;
        }
        // Renewal talks collapsed in the last ~5 months — the club tried
        // and failed, so a rival can step in.
        let renewal_failed = player.decision_history.items.iter().any(|d| {
            d.decision == RENEWAL_REJECTED_LABEL && (date - d.date).num_days() <= 150
        });
        if renewal_failed {
            return true;
        }
        // Wanted by rivals while inside the Bosman window.
        statuses.iter().any(|s| {
            matches!(
                s,
                PlayerStatusType::Wnt | PlayerStatusType::Enq | PlayerStatusType::Bid
            )
        })
    }

    /// Pick the best domestic buyer for a leaving player: a club (other
    /// than his current one) with an open transfer request for his
    /// position group, roster room, and a tier whose quality band fits
    /// him. Among candidates the strongest fitting club wins — the
    /// realistic destination a player negotiates toward. Returns the
    /// staged agreement with priced terms, or `None` when no club fits.
    fn choose_buyer(
        country: &Country,
        player: &LeavingPlayer,
        date: NaiveDate,
    ) -> Option<StagedPreContract> {
        let mut best: Option<(u32, f32, u32, u8, Option<PlayerSquadStatus>)> = None;
        for club in &country.clubs {
            if club.id == player.current_club_id || club.teams.teams.is_empty() {
                continue;
            }
            if !can_club_accept_player(club) {
                continue;
            }
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }
            let has_request = plan.transfer_requests.iter().any(|r| {
                r.status != TransferRequestStatus::Fulfilled
                    && r.status != TransferRequestStatus::Abandoned
                    && r.position.position_group() == player.group
            });
            if !has_request {
                continue;
            }

            let Some(main_team) = club.teams.main().or_else(|| club.teams.teams.first()) else {
                continue;
            };
            let club_score = (main_team.reputation.world as f32 / 10_000.0).clamp(0.0, 1.0);
            // Low pressure: the player is employed and weighing a
            // considered move, not a desperate one — the quality band
            // stays tight so a club only pre-signs a player who fits its
            // level.
            let cp = 0.1;
            let min_ca = FreeAgentMarketCalculator::min_acceptable_ca(club_score, player.group, cp);
            let max_ca = FreeAgentMarketCalculator::max_acceptable_ca(club_score, player.group, cp);
            if player.ability < min_ca || player.ability > max_ca {
                continue;
            }

            // Prefer the strongest fitting club — a leaving player moves
            // up where he can.
            if best
                .as_ref()
                .map(|(_, score, _, _, _)| club_score > *score)
                .unwrap_or(true)
            {
                let league_reputation = main_team
                    .league_id
                    .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                    .map(|l| l.reputation)
                    .unwrap_or(0);
                let negotiator_skill = main_team
                    .staffs
                    .find_negotiator()
                    .map(|s| (s.staff_attributes.mental.man_management as u32 * 5).min(100) as u8)
                    .unwrap_or(50);

                let (annual_wage, contract_years, promised_status) = Self::price_terms(
                    player,
                    club_score,
                    league_reputation,
                    negotiator_skill,
                    country.reputation,
                );
                best = Some((
                    club.id,
                    club_score,
                    annual_wage,
                    contract_years,
                    promised_status,
                ));
            }
        }

        let (to_club_id, _score, annual_wage, contract_years, promised_status) = best?;
        Some(StagedPreContract {
            player_id: player.player_id,
            to_club_id,
            agreement: PreContractAgreement {
                to_club_id,
                to_country_id: country.id,
                annual_wage,
                contract_years,
                promised_status,
                agreed_on: date,
            },
        })
    }

    /// Price the pre-contract through the shared free-agent wage chain so
    /// the deal sits on the same scale as the rest of the market. A
    /// pre-contract player is treated as `Fresh` (still employed, not
    /// desperate) so he lands a proper multi-year deal, not a trial.
    fn price_terms(
        player: &LeavingPlayer,
        club_score: f32,
        league_reputation: u16,
        negotiator_skill: u8,
        country_reputation: u16,
    ) -> (u32, u8, Option<PlayerSquadStatus>) {
        let market_wage = WageCalculator::expected_annual_wage_raw(
            player.ability,
            player.current_reputation,
            player.group == PlayerFieldPositionGroup::Forward,
            player.group == PlayerFieldPositionGroup::Goalkeeper,
            player.age,
            club_score,
            league_reputation,
        );
        let role = FreeAgentMarketCalculator::infer_buyer_role(player.ability, club_score, player.group);
        let annual_wage = FreeAgentMarketCalculator::offer_wage(
            market_wage,
            role,
            negotiator_skill,
            country_reputation,
            // Reservation ≈ market wage for an employed player weighing a
            // move; he isn't decaying his demand on the open market.
            market_wage,
            0.0,
        );
        let contract_years =
            FreeAgentMarketCalculator::stage_contract_years(MarketStage::Fresh, player.age, player.ability);
        let promised_status = match role {
            BuyerRoleFit::KeyPlayer => Some(PlayerSquadStatus::KeyPlayer),
            BuyerRoleFit::Starter => Some(PlayerSquadStatus::FirstTeamRegular),
            BuyerRoleFit::Rotation => Some(PlayerSquadStatus::FirstTeamSquadRotation),
            BuyerRoleFit::Backup | BuyerRoleFit::Emergency => None,
        };
        (annual_wage, contract_years, promised_status)
    }
}

#[cfg(test)]
mod tests {
    //! Spec test #4: the pre-contract flow must STAGE a future free
    //! transfer without moving the player before his contract expires, and
    //! then route the move to the agreed club once it does lapse.

    use super::*;
    use crate::club::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::country::result::CountryResult;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::transfers::pipeline::{TransferNeedPriority, TransferNeedReason, TransferRequest};
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
        PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, StaffCollection, Team, TeamCollection, TeamReputation,
        TeamType, TrainingSchedule,
    };
    use chrono::NaiveTime;

    struct PreContractFixtures;

    impl PreContractFixtures {
        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        /// A useful, transfer-listed midfielder whose deal expires in
        /// `days_to_expiry` days — a textbook leaving player.
        fn leaving_player(id: u32, today: NaiveDate, days_to_expiry: i64) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = 80;
            attrs.potential_ability = 85;
            attrs.current_reputation = 2400;
            let mut p = PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Lea".to_string(), format!("Ving{id}")))
                .birth_date(today - chrono::Duration::days(27 * 365))
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 16,
                    }],
                })
                .player_attributes(attrs)
                .build()
                .unwrap();
            let mut contract =
                PlayerClubContract::new(60_000, today + chrono::Duration::days(days_to_expiry));
            contract.is_transfer_listed = true;
            p.contract = Some(contract);
            p
        }

        fn team(id: u32, club_id: u32, players: Vec<Player>) -> Team {
            Team::builder()
                .id(id)
                .league_id(Some(1))
                .club_id(club_id)
                .name(format!("Team{id}"))
                .slug(format!("team-{id}"))
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(2000, 2000, 4000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn club(id: u32, main: Team) -> Club {
            Club::new(
                id,
                format!("Club{id}"),
                Location::new(1),
                ClubFinances::new(1_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![main]),
                ClubFacilities::default(),
            )
        }

        /// Buyer club with an open midfielder request and roster room.
        fn buyer_club(id: u32) -> Club {
            let mut club = Self::club(id, Self::team(id * 10, id, Vec::new()));
            club.transfer_plan.initialized = true;
            club.transfer_plan
                .transfer_requests
                .push(TransferRequest::new(
                    1,
                    PlayerPositionType::MidfielderCenter,
                    TransferNeedPriority::Critical,
                    TransferNeedReason::SquadPadding,
                    50,
                    90,
                    0.0,
                ));
            club
        }

        fn country(clubs: Vec<Club>) -> Country {
            Country::builder()
                .id(1)
                .code("en".to_string())
                .slug("england".to_string())
                .name("England".to_string())
                .continent_id(1)
                .reputation(5000)
                .leagues(LeagueCollection::new(vec![League::new(
                    1,
                    "L".to_string(),
                    "english".to_string(),
                    1,
                    5000,
                    LeagueSettings {
                        season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                        season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                        tier: 1,
                        promotion_spots: 0,
                        relegation_spots: 0,
                        league_group: None,
                    },
                    false,
                )]))
                .clubs(clubs)
                .build()
                .unwrap()
        }

        fn find_player(country: &Country, player_id: u32) -> Option<(u32, &Player)> {
            country.clubs.iter().find_map(|c| {
                c.teams
                    .teams
                    .iter()
                    .flat_map(|t| t.players.players.iter())
                    .find(|p| p.id == player_id)
                    .map(|p| (c.id, p))
            })
        }
    }

    #[test]
    fn pre_contract_is_staged_without_moving_the_player_before_expiry() {
        let today = PreContractFixtures::d(2026, 3, 1);
        let config = TransferConfig::default();
        // Current club (B = 100) holds the leaving midfielder, 120 days
        // from expiry. A domestic rival (A = 200) wants a midfielder.
        let leaver = PreContractFixtures::leaving_player(1, today, 120);
        let club_b = PreContractFixtures::club(100, PreContractFixtures::team(10, 100, vec![leaver]));
        let club_a = PreContractFixtures::buyer_club(200);
        let mut country = PreContractFixtures::country(vec![club_b, club_a]);

        // Staging is probabilistic per tick; once it lands it sticks.
        let mut staged = false;
        for _ in 0..3000 {
            PreContractManager::stage(&mut country, today, &config);
            if PreContractFixtures::find_player(&country, 1)
                .map(|(_, p)| p.pending_pre_contract().is_some())
                .unwrap_or(false)
            {
                staged = true;
                break;
            }
        }
        assert!(staged, "a leaving useful player must eventually be pre-signed");

        let (club_id, player) =
            PreContractFixtures::find_player(&country, 1).expect("player still present");
        // Crucially: NOT moved. He is still on his current club's roster
        // under his existing, unexpired contract.
        assert_eq!(club_id, 100, "the player must not move before expiry");
        let contract = player
            .contract
            .as_ref()
            .expect("the running contract must be untouched");
        assert!(
            contract.expiration > today,
            "the contract must still be live — no early move"
        );
        // The staged agreement points at the domestic buyer.
        let agreement = player.pending_pre_contract().expect("agreement present");
        assert_eq!(agreement.to_club_id, 200);
        assert_eq!(agreement.to_country_id, 1);
        assert!(agreement.annual_wage > 0);
        assert!(agreement.contract_years >= 1);
    }

    #[test]
    fn staged_pre_contract_executes_the_free_move_on_expiry() {
        let today = PreContractFixtures::d(2026, 6, 30);
        // The leaving player's contract lapses TODAY, with a pre-contract
        // already agreed with the domestic buyer (200).
        let mut leaver = PreContractFixtures::leaving_player(1, today, 0);
        leaver.stage_pre_contract(PreContractAgreement {
            to_club_id: 200,
            to_country_id: 1,
            annual_wage: 70_000,
            contract_years: 2,
            promised_status: Some(PlayerSquadStatus::FirstTeamRegular),
            agreed_on: PreContractFixtures::d(2026, 3, 1),
        });
        let club_b = PreContractFixtures::club(100, PreContractFixtures::team(10, 100, vec![leaver]));
        let club_a = PreContractFixtures::buyer_club(200);
        let mut country = PreContractFixtures::country(vec![club_b, club_a]);

        let mut summary = crate::country::result::transfers::types::TransferActivitySummary::new();
        let config = TransferConfig::default();
        let mut domestic = Vec::new();
        let mut offered = Vec::new();
        let mut rejected = Vec::new();
        let mut blocked = Vec::new();
        let _ = CountryResult::handle_free_agents(
            &mut country,
            today,
            &mut summary,
            &[],
            &config,
            &mut domestic,
            &mut offered,
            &mut rejected,
            &mut blocked,
        );

        let (club_id, player) = PreContractFixtures::find_player(&country, 1)
            .expect("player must still exist somewhere in the country");
        assert_eq!(
            club_id, 200,
            "the expiring pre-contract must move the player to the agreed club"
        );
        assert!(
            player.contract.as_ref().is_some_and(|c| c.expiration > today),
            "the buyer must install a fresh contract on arrival"
        );
        assert!(
            player.pending_pre_contract().is_none(),
            "the consumed agreement must be cleared after the move"
        );
        // The free move was recorded as a completed transfer.
        assert!(
            country
                .transfer_market
                .transfer_history
                .iter()
                .any(|t| t.player_id == 1 && t.to_club_id == 200),
            "the pre-contract free transfer must be in the history"
        );
    }
}

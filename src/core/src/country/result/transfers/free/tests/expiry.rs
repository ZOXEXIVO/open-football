//! Expiry-day last-chance renewal: a player whose contract lapses must
//! get one synchronous offer from his club before the release sweep
//! clears the contract. Acceptance keeps him under a fresh deal and out
//! of the same-day free-agent flow; rejection falls through to the
//! existing release path.
//!
//! Worlds are built by [`crate::transfers::tests::kit`]; only what this
//! module is actually about — the personality that decides the answer and
//! the decision-history it leaves — is spelled out here.

use super::super::*;
use crate::club::player::contract::RENEWAL_REJECTED_LABEL;
use crate::country::result::transfers::free::FreeAgentPass;
use crate::transfers::tests::kit::{TestClub, TestCountry, TestDate, TestPlayer, TestTeam};
use crate::{
    Club, PersonAttributes, Player, PlayerClubContract, PlayerPositionType, PlayerSquadStatus, Team,
};

struct ExpiryRenewalFixtures;

impl ExpiryRenewalFixtures {
    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        TestDate::on(y, m, day)
    }

    /// A steady professional with the ambition / loyalty pair the test is
    /// about; every other trait sits at the middle of the scale.
    fn attrs(ambition: f32, loyalty: f32) -> PersonAttributes {
        PersonAttributes {
            adaptability: 12.0,
            ambition,
            controversy: 5.0,
            loyalty,
            pressure: 12.0,
            professionalism: 12.0,
            sportsmanship: 12.0,
            temperament: 12.0,
            consistency: 12.0,
            important_matches: 12.0,
            dirtiness: 5.0,
        }
    }

    fn player(
        id: u32,
        position: PlayerPositionType,
        attrs: PersonAttributes,
        salary: u32,
        squad_status: PlayerSquadStatus,
        expiration: NaiveDate,
    ) -> Player {
        TestPlayer::new(id)
            .position(position)
            .person(attrs)
            .ability(100)
            .potential(110)
            .age(28)
            .on(Self::d(2026, 6, 10))
            .contract_until(salary, expiration)
            .squad_status(squad_status)
            .build()
    }

    fn team(id: u32, club_id: u32, players: Vec<Player>) -> Team {
        TestTeam::new(id)
            .club_id(club_id)
            .name(&format!("Team{id}"))
            .players(players)
            .build()
    }

    fn club(id: u32, main: Team) -> Club {
        TestClub::new(id)
            .name(&format!("Club{id}"))
            .teams(vec![main])
            .build()
    }

    fn country(clubs: Vec<Club>) -> Country {
        TestCountry::new(1).clubs(clubs).build()
    }

    fn run(country: &mut Country, date: NaiveDate) -> Vec<GlobalFreeAgentSigning> {
        let mut summary = TransferActivitySummary::new();
        let config = TransferConfig::default();
        let mut domestic_signed_ids = Vec::new();
        let mut global_offered_ids = Vec::new();
        let mut global_rejected_ids = Vec::new();
        let mut global_blocked = Vec::new();
        FreeAgentPass::handle_free_agents(
            country,
            date,
            &FreeAgentWorld {
                global_pool: &[],
                market_map: &MarketMap::default(),
                config: &config,
            },
            &mut FreeAgentLedger {
                summary: &mut summary,
                domestic_signed_ids: &mut domestic_signed_ids,
                global_offered_ids: &mut global_offered_ids,
                global_rejected_ids: &mut global_rejected_ids,
                global_blocked: &mut global_blocked,
            },
        )
    }

    fn find_player(country: &Country, club_id: u32, player_id: u32) -> &Player {
        country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .expect("club exists")
            .teams
            .teams
            .iter()
            .flat_map(|t| t.players.players.iter())
            .find(|p| p.id == player_id)
            .expect("player still in roster")
    }

    fn history_count(player: &Player, label: &str) -> usize {
        player
            .decision_history
            .items
            .iter()
            .filter(|d| d.decision == label)
            .count()
    }
}

#[test]
fn accepted_expiry_offer_renews_contract_and_keeps_player() {
    let date = ExpiryRenewalFixtures::d(2026, 6, 10);
    // Low current salary against a 700k top earner: the wage-structure
    // cap leaves the offer well above the player's own market
    // valuation (his absolute walk-away floor), so the acceptance
    // handler takes the big raise deterministically.
    let renewer = ExpiryRenewalFixtures::player(
        1,
        PlayerPositionType::MidfielderCenter,
        ExpiryRenewalFixtures::attrs(8.0, 12.0),
        10_000,
        PlayerSquadStatus::FirstTeamRegular,
        date,
    );
    let anchor = ExpiryRenewalFixtures::player(
        2,
        PlayerPositionType::Striker,
        ExpiryRenewalFixtures::attrs(8.0, 12.0),
        700_000,
        PlayerSquadStatus::KeyPlayer,
        ExpiryRenewalFixtures::d(2028, 6, 30),
    );
    let main = ExpiryRenewalFixtures::team(10, 100, vec![renewer, anchor]);
    let club = ExpiryRenewalFixtures::club(100, main);
    let mut country = ExpiryRenewalFixtures::country(vec![club]);

    let global_signings = ExpiryRenewalFixtures::run(&mut country, date);

    assert!(global_signings.is_empty());
    let p = ExpiryRenewalFixtures::find_player(&country, 100, 1);
    let contract = p
        .contract
        .as_ref()
        .expect("accepted expiry offer must install a fresh contract");
    assert!(
        contract.expiration > date,
        "renewed contract must run past today, got {}",
        contract.expiration
    );
    assert_eq!(
        ExpiryRenewalFixtures::history_count(p, RENEWAL_OFFERED_LABEL),
        1,
        "expiry-day offer must be recorded in decision history"
    );
    assert_eq!(
        ExpiryRenewalFixtures::history_count(p, RENEWAL_REJECTED_LABEL),
        0
    );
}

#[test]
fn rejected_expiry_offer_falls_through_to_release() {
    let date = ExpiryRenewalFixtures::d(2026, 6, 10);
    // The player is his own top earner, so the wage-structure cap
    // turns the final offer into a pay cut; loyalty 5 rejects every
    // pay-cut branch deterministically.
    let leaver = ExpiryRenewalFixtures::player(
        1,
        PlayerPositionType::Striker,
        ExpiryRenewalFixtures::attrs(8.0, 5.0),
        100_000,
        PlayerSquadStatus::FirstTeamRegular,
        date,
    );
    let main = ExpiryRenewalFixtures::team(10, 100, vec![leaver]);
    let club = ExpiryRenewalFixtures::club(100, main);
    let mut country = ExpiryRenewalFixtures::country(vec![club]);

    ExpiryRenewalFixtures::run(&mut country, date);

    let p = ExpiryRenewalFixtures::find_player(&country, 100, 1);
    assert!(
        p.contract.is_none(),
        "rejected expiry offer must still end in release"
    );
    assert_eq!(
        ExpiryRenewalFixtures::history_count(p, RENEWAL_OFFERED_LABEL),
        1,
        "the final offer must be on record even when it fails"
    );
    assert_eq!(
        ExpiryRenewalFixtures::history_count(p, RENEWAL_REJECTED_LABEL),
        1,
        "rejection must use the existing rejection label"
    );
}

#[test]
fn loaned_in_expired_parent_contract_is_not_renewed_by_borrower() {
    let date = ExpiryRenewalFixtures::d(2026, 6, 10);
    let mut loanee = ExpiryRenewalFixtures::player(
        1,
        PlayerPositionType::Striker,
        ExpiryRenewalFixtures::attrs(8.0, 12.0),
        50_000,
        PlayerSquadStatus::FirstTeamRegular,
        date,
    );
    // Parent club 99 owns the (expired) permanent contract; the
    // borrower (club 100) only holds the loan agreement.
    loanee.contract_loan = Some(PlayerClubContract::new_loan(
        20_000,
        ExpiryRenewalFixtures::d(2026, 12, 31),
        99,
        1,
        100,
    ));
    let main = ExpiryRenewalFixtures::team(10, 100, vec![loanee]);
    let club = ExpiryRenewalFixtures::club(100, main);
    let mut country = ExpiryRenewalFixtures::country(vec![club]);

    ExpiryRenewalFixtures::run(&mut country, date);

    let p = ExpiryRenewalFixtures::find_player(&country, 100, 1);
    assert_eq!(
        ExpiryRenewalFixtures::history_count(p, RENEWAL_OFFERED_LABEL),
        0,
        "the borrower must not make an expiry-day offer on a loanee"
    );
    let parent_contract = p
        .contract
        .as_ref()
        .expect("parent contract is not the borrower's to clear");
    assert_eq!(
        parent_contract.expiration, date,
        "parent contract must be left exactly as it was"
    );
}

#[test]
fn renewed_player_is_excluded_from_same_day_free_agent_flow() {
    let date = ExpiryRenewalFixtures::d(2026, 6, 10);
    let renewer = ExpiryRenewalFixtures::player(
        1,
        PlayerPositionType::Goalkeeper,
        ExpiryRenewalFixtures::attrs(8.0, 12.0),
        10_000,
        PlayerSquadStatus::FirstTeamRegular,
        date,
    );
    // High anchor for the same reason as the acceptance test above:
    // the capped offer must clear the keeper's walk-away floor.
    let anchor = ExpiryRenewalFixtures::player(
        2,
        PlayerPositionType::Striker,
        ExpiryRenewalFixtures::attrs(8.0, 12.0),
        700_000,
        PlayerSquadStatus::KeyPlayer,
        ExpiryRenewalFixtures::d(2028, 6, 30),
    );
    let club_a = ExpiryRenewalFixtures::club(
        100,
        ExpiryRenewalFixtures::team(10, 100, vec![renewer, anchor]),
    );
    // Club B has an empty main squad — the hungriest possible buyer:
    // the emergency pass would grab any available free-agent keeper.
    let club_b = ExpiryRenewalFixtures::club(200, ExpiryRenewalFixtures::team(20, 200, Vec::new()));
    let mut country = ExpiryRenewalFixtures::country(vec![club_a, club_b]);

    let global_signings = ExpiryRenewalFixtures::run(&mut country, date);

    assert!(global_signings.is_empty());
    let p = ExpiryRenewalFixtures::find_player(&country, 100, 1);
    assert!(
        p.contract.as_ref().is_some_and(|c| c.expiration > date),
        "player must have renewed at his own club"
    );
    let club_b_roster: usize = country
        .clubs
        .iter()
        .find(|c| c.id == 200)
        .unwrap()
        .teams
        .teams
        .iter()
        .map(|t| t.players.players.len())
        .sum();
    assert_eq!(
        club_b_roster, 0,
        "a renewed player must not be signable as a same-day free agent"
    );
    assert!(
        country.transfer_market.transfer_history.is_empty(),
        "no free transfer may be recorded for a renewed player"
    );
}

/// Spec test #3: a player who has already rejected a season's worth of
/// renewal offers and is still asking for a wage the club won't fund
/// must NOT receive yet another identical expiry-day proposal. The
/// final offer is suppressed (the club lets him walk) instead of
/// spamming the same losing deal — no new RENEWAL_OFFERED row appears.
#[test]
fn repeated_rejected_renewal_does_not_spam_expiry_day_offer() {
    use crate::club::player::contract::RENEWAL_OFFERED_LABEL;
    use crate::club::player::mailbox::{PlayerContractAsk, RejectionReason};

    let date = ExpiryRenewalFixtures::d(2026, 6, 10);
    let mut player = ExpiryRenewalFixtures::player(
        1,
        PlayerPositionType::MidfielderCenter,
        ExpiryRenewalFixtures::attrs(8.0, 8.0),
        100_000,
        PlayerSquadStatus::FirstTeamRegular,
        date,
    );
    // Three renewal offers already made (and turned down) this rolling
    // year — the season's worth of attempts is spent.
    for offer_date in [
        ExpiryRenewalFixtures::d(2026, 1, 10),
        ExpiryRenewalFixtures::d(2026, 3, 10),
        ExpiryRenewalFixtures::d(2026, 5, 10),
    ] {
        player.decision_history.add(
            offer_date,
            "3y · $110,000/y".to_string(),
            RENEWAL_OFFERED_LABEL.to_string(),
            "Coach".to_string(),
        );
    }
    // His standing ask is a wage well above anything the club's
    // valuation produces for a CA-100 player — the only sticking point
    // is money, with no clause / role / length demand to grant.
    player.pending_contract_ask = Some(PlayerContractAsk {
        desired_salary: 800_000,
        desired_years: 3,
        recorded_on: ExpiryRenewalFixtures::d(2026, 5, 10),
        demanded_status: None,
        demanded_release_clause: None,
        demanded_signing_bonus: None,
        rejection_reason: Some(RejectionReason::LowSalary),
    });

    let offers_before = ExpiryRenewalFixtures::history_count(&player, RENEWAL_OFFERED_LABEL);
    assert_eq!(
        offers_before, 3,
        "fixture must start with three prior offers"
    );

    let main = ExpiryRenewalFixtures::team(10, 100, vec![player]);
    let club = ExpiryRenewalFixtures::club(100, main);
    let mut country = ExpiryRenewalFixtures::country(vec![club]);

    ExpiryRenewalFixtures::run(&mut country, date);

    let p = ExpiryRenewalFixtures::find_player(&country, 100, 1);
    assert_eq!(
        ExpiryRenewalFixtures::history_count(p, RENEWAL_OFFERED_LABEL),
        3,
        "no fourth identical offer may be made — the expiry offer is suppressed"
    );
    assert!(
        p.contract.is_none(),
        "with no improved offer the player walks for free on expiry"
    );
}

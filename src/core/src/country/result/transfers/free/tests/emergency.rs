//! Moved verbatim out of `free_agents.rs` — see that file's `mod emergency_fill_tests`.

use super::super::*;
use crate::club::academy::ClubAcademy;
use crate::club::player::builder::PlayerBuilder;
use crate::competitions::global::GlobalCompetitions;
use crate::continent::Continent;
use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
use crate::shared::Location;
use crate::shared::fullname::FullName;
use crate::transfers::deal::negotiation::NegotiationRejectionReason;
use crate::transfers::market::TransferListingStatus;
use crate::transfers::market::map::{CorridorWeight, CountryTransferProfile, MarketCountryFacts};
use crate::transfers::pipeline::{ShortlistCandidateStatus, TransferNeedPriority};
use crate::transfers::squad::needs::EmergencyContractTermsPolicy;
use crate::utils::random::engine::RandomEngine;
use crate::{
    Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes, Player,
    PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
    PlayerSkills, StaffCollection, Team, TeamCollection, TeamReputation, TeamType,
    TrainingSchedule,
};
use chrono::NaiveTime;

/// Test fixtures grouped on a unit struct so the test module
/// stays free of loose helper fns (project convention — see
/// `feedback_use_directives`).
struct EmergencyFillFixtures;

impl EmergencyFillFixtures {
    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn player(id: u32, position: PlayerPositionType) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Test".to_string(), format!("P{id}")))
            .birth_date(Self::d(1998, 1, 1))
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 16,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn team(id: u32, name: &str, slug: &str, players: Vec<Player>) -> Team {
        Team::builder()
            .id(id)
            .league_id(Some(1))
            .club_id(100)
            .name(name.to_string())
            .slug(slug.to_string())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(4000, 4000, 4000))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    fn club(id: u32, name: &str, main: Team) -> Club {
        Club::new(
            id,
            name.to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![main]),
            ClubFacilities::default(),
        )
    }

    fn country(clubs: Vec<Club>) -> Country {
        Country::builder()
            .id(1)
            .code("en".to_string())
            .slug("england".to_string())
            .name("England".to_string())
            .continent_id(1)
            // Buyer-country reputation drives the chasm gate in
            // EmergencySquadFillStrategy. 5000 sits comfortably
            // above the test candidates' reference reputation
            // (3000-4000), so the gate passes — the failing
            // alternative is unintentionally testing the
            // rep-chasm rejection path.
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
                    split_season: false,
                },
                false,
            )]))
            .clubs(clubs)
            .build()
            .unwrap()
    }

    /// Build a free-agent candidate sourced from the global pool
    /// (club_id = 0) — the path emergency fill exercises most
    /// commonly because expired-contract candidates are normally
    /// rare on any given tick.
    fn candidate(
        player_id: u32,
        ability: u8,
        age: u8,
        position_group: PlayerFieldPositionGroup,
        same_country: bool,
    ) -> FreeAgentCandidate {
        let code = if same_country { "en" } else { "ar" };
        FreeAgentCandidate {
            player_id,
            player_name: format!("Cand{player_id}"),
            club_id: 0,
            club_name: "Free Agent".to_string(),
            ability,
            potential: ability.saturating_add(5),
            age,
            position_group,
            days_to_expiry: 0,
            nationality_country_reputation: if same_country { 5000 } else { 3000 },
            nationality_region: ScoutingRegion::from_country(1, code),
            nationality_country_code: code.to_string(),
            nationality_continent_id: 1,
            career_pressure: 0.6,
            days_free: 0,
            reference_reputation: if same_country { 4000 } else { 3000 },
            last_salary: 50_000,
            last_country_reputation: 5000,
            last_league_reputation: 4500,
            world_reputation: 1500,
            current_reputation: 1500,
            professionalism_norm: 0.5,
            failed_approach_streak: 0,
            is_global_pool: true,
            nationality_country_id: 0,
            last_country_id: 0,
        }
    }

    /// Variant of [`Self::candidate`] with explicit career pressure
    /// override — needed for the acceptance tests where a low-
    /// pressure superstar should reject a tiny club's emergency
    /// offer, and a high-pressure veteran should accept.
    fn candidate_with(
        player_id: u32,
        ability: u8,
        age: u8,
        position_group: PlayerFieldPositionGroup,
        same_country: bool,
        career_pressure: f32,
        reference_reputation: u16,
    ) -> FreeAgentCandidate {
        let mut c = Self::candidate(player_id, ability, age, position_group, same_country);
        c.career_pressure = career_pressure;
        c.reference_reputation = reference_reputation;
        c
    }

    /// Build a country with a configurable reputation so the
    /// realism tests can exercise low-rep / high-rep buyers
    /// without rewriting the whole fixture.
    fn country_with_reputation(clubs: Vec<Club>, reputation: u16) -> Country {
        Country::builder()
            .id(1)
            .code("en".to_string())
            .slug("england".to_string())
            .name("England".to_string())
            .continent_id(1)
            .reputation(reputation)
            .leagues(LeagueCollection::new(vec![League::new(
                1,
                "L".to_string(),
                "english".to_string(),
                1,
                reputation,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                    split_season: false,
                },
                false,
            )]))
            .clubs(clubs)
            .build()
            .unwrap()
    }

    /// Run the emergency pass with both side-channel vecs
    /// allocated locally — most tests don't care about offered /
    /// rejected tracking, so funneling those into a helper keeps
    /// the test bodies tight.
    fn run_emergency(
        country: &Country,
        candidates: &[FreeAgentCandidate],
        config: &TransferConfig,
        signings: &mut Vec<FreeAgentSigning>,
    ) -> (Vec<u32>, Vec<u32>) {
        let mut offered = Vec::new();
        let mut rejected = Vec::new();
        CountryResult::handle_free_agents_emergency_pass(
            country,
            candidates,
            config,
            &FreeAgentMarketVisibility::build(0, &MarketMap::default(), &[]),
            signings,
            &mut offered,
            &mut rejected,
            &mut BlockReasonRecorder::new(),
        );
        (offered, rejected)
    }
}

#[test]
fn empty_main_team_generates_emergency_signings_for_each_group() {
    // Test-isolation: seed the shared RandomEngine so the per-slot
    // weighted cluster pick + acceptance roll sequence is independent
    // of how many RNG draws preceding tests consumed (mirrors the
    // seeded sibling tests in this block).
    RandomEngine::set_seed(0xE11E_C7AB_u64);
    // Empty squad → urgent flag. Emergency pass should produce at
    // least one signing per missing group (GK/DEF/MID/FWD) up to
    // the per-club cap, before the normal request-driven matcher
    // has any state to work from.
    let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);

    let mut candidates = Vec::new();
    for i in 0..3 {
        candidates.push(EmergencyFillFixtures::candidate(
            100 + i,
            70,
            26,
            PlayerFieldPositionGroup::Goalkeeper,
            true,
        ));
    }
    for i in 0..8 {
        candidates.push(EmergencyFillFixtures::candidate(
            200 + i,
            75,
            26,
            PlayerFieldPositionGroup::Defender,
            true,
        ));
    }
    for i in 0..8 {
        candidates.push(EmergencyFillFixtures::candidate(
            300 + i,
            75,
            26,
            PlayerFieldPositionGroup::Midfielder,
            true,
        ));
    }
    for i in 0..5 {
        candidates.push(EmergencyFillFixtures::candidate(
            400 + i,
            80,
            26,
            PlayerFieldPositionGroup::Forward,
            true,
        ));
    }

    let mut signings = Vec::new();
    let config = TransferConfig::default();
    EmergencyFillFixtures::run_emergency(&country, &candidates, &config, &mut signings);

    // Empty squad triggers the adaptive cap: per-club cap is
    // lifted to the playable-size floor so the club can reach 11
    // in one tick when the pool allows it. Country cap is still
    // the ceiling.
    assert!(
        !signings.is_empty(),
        "empty squad must generate emergency signings, got 0"
    );
    assert!(
        signings.len() <= config.emergency_max_signings_per_country_per_day,
        "exceeded country emergency cap"
    );
}

#[test]
fn club_short_one_gk_signs_a_gk_first() {
    // Test-isolation: seed the global RandomEngine so the probabilistic
    // acceptance roll is deterministic regardless of how many RNG draws
    // preceding tests consumed (mirrors the seeded sibling tests above).
    // Without this the outcome is execution-order dependent.
    RandomEngine::set_seed(0xE11E_C7AB_u64);
    // Squad has 0 GK and a handful of outfield bodies — emergency
    // pass must reach for the goalkeeper before anything else.
    let players: Vec<Player> = (0..8)
        .map(|i| EmergencyFillFixtures::player(i, PlayerPositionType::DefenderCenter))
        .chain(
            (0..6).map(|i| {
                EmergencyFillFixtures::player(20 + i, PlayerPositionType::MidfielderCenter)
            }),
        )
        .chain((0..4).map(|i| EmergencyFillFixtures::player(40 + i, PlayerPositionType::Striker)))
        .collect();

    let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);

    // Candidate pool: GKs only (everything else already filled).
    // Full career pressure pins the acceptance roll near-certain —
    // the assertion is about slot ordering, not willingness.
    let candidates: Vec<FreeAgentCandidate> = (0..3)
        .map(|i| {
            EmergencyFillFixtures::candidate_with(
                500 + i,
                70,
                26,
                PlayerFieldPositionGroup::Goalkeeper,
                true,
                1.0,
                3500,
            )
        })
        .collect();

    let mut signings = Vec::new();
    EmergencyFillFixtures::run_emergency(
        &country,
        &candidates,
        &TransferConfig::default(),
        &mut signings,
    );

    assert!(
        !signings.is_empty(),
        "GK-deficient squad must sign at least one goalkeeper"
    );
    assert_eq!(
        signings[0].reason.key, "emergency_squad_fill_gk",
        "first emergency signing for a GK-deficient squad must be tagged GK"
    );
}

#[test]
fn full_squad_does_not_emergency_sign() {
    // 25-player squad split sensibly across groups → no emergency
    // need at all. signings should stay empty regardless of the
    // candidates available.
    let mut players: Vec<Player> = Vec::new();
    for i in 0..2 {
        players.push(EmergencyFillFixtures::player(
            i,
            PlayerPositionType::Goalkeeper,
        ));
    }
    for i in 0..8 {
        players.push(EmergencyFillFixtures::player(
            10 + i,
            PlayerPositionType::DefenderCenter,
        ));
    }
    for i in 0..9 {
        players.push(EmergencyFillFixtures::player(
            20 + i,
            PlayerPositionType::MidfielderCenter,
        ));
    }
    for i in 0..6 {
        players.push(EmergencyFillFixtures::player(
            40 + i,
            PlayerPositionType::Striker,
        ));
    }

    let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);

    let candidates: Vec<FreeAgentCandidate> = (0..10)
        .map(|i| {
            EmergencyFillFixtures::candidate(
                600 + i,
                80,
                27,
                PlayerFieldPositionGroup::Midfielder,
                true,
            )
        })
        .collect();

    let mut signings = Vec::new();
    EmergencyFillFixtures::run_emergency(
        &country,
        &candidates,
        &TransferConfig::default(),
        &mut signings,
    );
    assert!(signings.is_empty(), "full squad should not emergency-sign");
}

#[test]
fn underfilled_club_signs_multiple_despite_normal_daily_cap() {
    // Test-isolation: seed the shared RandomEngine so the per-slot
    // weighted cluster pick + acceptance roll sequence stays
    // deterministic regardless of suite position (the cluster pick
    // now draws RNG for multi-candidate slots).
    RandomEngine::set_seed(0xE11E_C7AB_u64);
    // Squad of 9 (urgent < 11). Normal max_free_agent_signings_per_day
    // is 2; the emergency pass uses a separate per-club cap (5
    // by default) so the underfilled club must be able to sign
    // more than 2 in a single tick.
    let players: Vec<Player> = (0..9)
        .map(|i| EmergencyFillFixtures::player(i, PlayerPositionType::MidfielderCenter))
        .collect();

    let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);

    // Full career pressure keeps the per-candidate acceptance roll
    // near-certain — the assertion is about cap behaviour, not
    // player willingness, and must not flake on the shared stream.
    let mut candidates: Vec<FreeAgentCandidate> = Vec::new();
    for i in 0..2 {
        candidates.push(EmergencyFillFixtures::candidate_with(
            700 + i,
            70,
            26,
            PlayerFieldPositionGroup::Goalkeeper,
            true,
            1.0,
            3500,
        ));
    }
    for i in 0..8 {
        candidates.push(EmergencyFillFixtures::candidate_with(
            710 + i,
            75,
            26,
            PlayerFieldPositionGroup::Defender,
            true,
            1.0,
            3500,
        ));
    }
    for i in 0..5 {
        candidates.push(EmergencyFillFixtures::candidate_with(
            720 + i,
            80,
            26,
            PlayerFieldPositionGroup::Forward,
            true,
            1.0,
            3500,
        ));
    }

    let mut signings = Vec::new();
    let config = TransferConfig::default();
    EmergencyFillFixtures::run_emergency(&country, &candidates, &config, &mut signings);
    assert!(
        signings.len() > config.max_free_agent_signings_per_day,
        "emergency pass should exceed the normal {} daily cap (urgent club, got {} signings)",
        config.max_free_agent_signings_per_day,
        signings.len()
    );
}

#[test]
fn zero_transfer_budget_does_not_block_emergency_fill() {
    // Seed + full career pressure: what's under test is the budget
    // independence, not the acceptance roll — with cp 0.6 each
    // candidate accepted only ~40% of the time and all five could
    // decline on an unlucky stream.
    RandomEngine::set_seed(0x0B0D_6E70);
    // Construct a club whose finance balance is zero / negative —
    // emergency fill should still proceed because free-agent fee
    // is 0 and the emergency pass doesn't gate on transfer budget.
    let players: Vec<Player> = (0..8)
        .map(|i| EmergencyFillFixtures::player(i, PlayerPositionType::MidfielderCenter))
        .collect();
    let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
    let mut club = EmergencyFillFixtures::club(100, "FC", main);
    // Zero out the finance balance — the emergency path must not
    // care, because no fee is paid.
    club.finance = ClubFinances::new(0, Vec::new());
    let country = EmergencyFillFixtures::country(vec![club]);

    let candidates: Vec<FreeAgentCandidate> = (0..5)
        .map(|i| {
            EmergencyFillFixtures::candidate_with(
                800 + i,
                70,
                27,
                PlayerFieldPositionGroup::Defender,
                true,
                1.0,
                3500,
            )
        })
        .collect();

    let mut signings = Vec::new();
    EmergencyFillFixtures::run_emergency(
        &country,
        &candidates,
        &TransferConfig::default(),
        &mut signings,
    );
    assert!(
        !signings.is_empty(),
        "zero-budget club must still emergency-sign free agents"
    );
}

#[test]
fn emergency_pass_skips_player_already_signed_in_same_tick() {
    // Pre-populate signings with one of the candidates — the
    // pass must not re-pick the same player. This is the
    // multi-club path: two underfilled clubs in the same country
    // shouldn't both grab the same free agent.
    let players: Vec<Player> = (0..8)
        .map(|i| EmergencyFillFixtures::player(i, PlayerPositionType::MidfielderCenter))
        .collect();
    let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);

    let candidate =
        EmergencyFillFixtures::candidate(900, 70, 26, PlayerFieldPositionGroup::Defender, true);
    let already =
        EmergencyFillFixtures::candidate(901, 70, 26, PlayerFieldPositionGroup::Defender, true);
    let candidates = vec![candidate, already];

    // Mark player 900 as already signed in this tick.
    let mut signings = vec![FreeAgentSigning {
        player_id: 900,
        player_name: "Cand900".to_string(),
        from_club_id: 0,
        from_club_name: "Free Agent".to_string(),
        to_club_id: 200,
        reason: TransferReason::key("emergency_squad_fill_def"),
        terms: None,
        fills_group: Some(PlayerFieldPositionGroup::Defender),
    }];

    EmergencyFillFixtures::run_emergency(
        &country,
        &candidates,
        &TransferConfig::default(),
        &mut signings,
    );
    assert!(
        !signings.iter().skip(1).any(|s| s.player_id == 900),
        "emergency pass must not re-pick a player already signed this tick"
    );
}

#[test]
fn emergency_pass_respects_per_club_cap() {
    // Squad of 0 → every group missing. With per-club cap of 5
    // the pass must sign at most 5 even when 20 candidates are
    // available in the right groups.
    let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);

    let mut candidates: Vec<FreeAgentCandidate> = Vec::new();
    // Plenty of every group.
    for grp in &[
        PlayerFieldPositionGroup::Goalkeeper,
        PlayerFieldPositionGroup::Defender,
        PlayerFieldPositionGroup::Midfielder,
        PlayerFieldPositionGroup::Forward,
    ] {
        for i in 0..5 {
            let pid = match grp {
                PlayerFieldPositionGroup::Goalkeeper => 1000 + i,
                PlayerFieldPositionGroup::Defender => 1100 + i,
                PlayerFieldPositionGroup::Midfielder => 1200 + i,
                PlayerFieldPositionGroup::Forward => 1300 + i,
            };
            candidates.push(EmergencyFillFixtures::candidate(pid, 75, 26, *grp, true));
        }
    }

    let mut signings = Vec::new();
    let config = TransferConfig::default();
    EmergencyFillFixtures::run_emergency(&country, &candidates, &config, &mut signings);
    // Per-club cap is lifted to the playable-size floor when the
    // squad is empty, so use the urgent floor (or country cap when
    // smaller) as the realistic upper bound.
    let expected_max = config
        .emergency_urgent_per_club_cap_floor
        .max(config.emergency_max_signings_per_club_per_day)
        .min(config.emergency_max_signings_per_country_per_day);
    assert!(
        signings.len() <= expected_max,
        "per-club cap exceeded: got {} signings, cap {}",
        signings.len(),
        expected_max
    );
}

#[test]
fn emergency_pass_picks_domestic_over_foreign_at_equal_quality() {
    // Empty squad. Two equally-strong defender candidates available,
    // one domestic, one foreign — the domestic preference should
    // surface the domestic player first. Both candidates use full
    // career pressure so the new acceptance roll lands reliably
    // for whichever is offered, isolating the scoring preference.
    let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);

    let domestic = EmergencyFillFixtures::candidate_with(
        2000,
        75,
        27,
        PlayerFieldPositionGroup::Defender,
        true,
        1.0,
        3500,
    );
    let foreign = EmergencyFillFixtures::candidate_with(
        2001,
        75,
        27,
        PlayerFieldPositionGroup::Defender,
        false,
        1.0,
        3500,
    );
    // Order foreign first to prove ordering isn't the reason —
    // scoring is.
    let candidates = vec![foreign, domestic];

    let mut signings = Vec::new();
    EmergencyFillFixtures::run_emergency(
        &country,
        &candidates,
        &TransferConfig::default(),
        &mut signings,
    );
    // The first DEF-tagged signing should be the domestic one.
    let first_def = signings
        .iter()
        .find(|s| s.reason.key == "emergency_squad_fill_def");
    assert_eq!(
        first_def.map(|s| s.player_id),
        Some(2000),
        "domestic candidate should win the defender slot, signings={:?}",
        signings
            .iter()
            .map(|s| (s.player_id, &s.reason))
            .collect::<Vec<_>>()
    );
}

#[test]
fn urgent_club_reaches_eleven_in_one_tick_with_plausible_pool() {
    // Pin the shared sim RNG so the per-candidate acceptance roll
    // sequence is deterministic — otherwise this test is at the
    // mercy of whatever earlier tests in the suite drained from
    // the thread-local stream, and one or two unlucky rejects
    // tip the signings count under the 11 floor.
    RandomEngine::set_seed(0xE11E_C7AB_u64);

    // Empty squad + plenty of plausible candidates → adaptive cap
    // lifts to the playable-size floor. The signing budget is
    // capped by the country-wide cap, but with 20 of room and a
    // pool of 20+ realistic candidates, a single tick must land
    // at least 11 signings so the club becomes playable.
    let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);

    // Full career pressure pins each acceptance roll near-certain;
    // the assertion measures the adaptive cap, not willingness.
    let mut candidates = Vec::new();
    for i in 0..3 {
        candidates.push(EmergencyFillFixtures::candidate_with(
            3000 + i,
            70,
            28,
            PlayerFieldPositionGroup::Goalkeeper,
            true,
            1.0,
            3500,
        ));
    }
    for i in 0..8 {
        candidates.push(EmergencyFillFixtures::candidate_with(
            3100 + i,
            75,
            28,
            PlayerFieldPositionGroup::Defender,
            true,
            1.0,
            3500,
        ));
    }
    for i in 0..8 {
        candidates.push(EmergencyFillFixtures::candidate_with(
            3200 + i,
            75,
            28,
            PlayerFieldPositionGroup::Midfielder,
            true,
            1.0,
            3500,
        ));
    }
    for i in 0..5 {
        candidates.push(EmergencyFillFixtures::candidate_with(
            3300 + i,
            75,
            28,
            PlayerFieldPositionGroup::Forward,
            true,
            1.0,
            3500,
        ));
    }

    let mut signings = Vec::new();
    let config = TransferConfig::default();
    EmergencyFillFixtures::run_emergency(&country, &candidates, &config, &mut signings);
    assert!(
        signings.len() >= config.emergency_min_playable_size,
        "urgent club should reach at least {} signings in one tick (got {})",
        config.emergency_min_playable_size,
        signings.len()
    );
}

#[test]
fn urgency_turns_off_at_eleven_signings() {
    // 10 players + plenty of plausible candidates → the 11th
    // signing flips the urgent flag off. Subsequent slots run
    // under non-urgent rules, which means a low-rep buyer should
    // start rejecting candidates the urgent path would have
    // accepted. We assert urgency by counting how many signings
    // tagged the depth slot vs. the urgent-group slots.
    let players: Vec<Player> = (0..10)
        .map(|i| EmergencyFillFixtures::player(i, PlayerPositionType::MidfielderCenter))
        .collect();
    let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);

    // Candidate pool: 1 keeper to flip projected count to 11, then
    // a few extras of each non-keeper group so the projection
    // continues filling but the urgency check has fired. Full
    // career pressure removes RNG flakiness from the GK signing
    // — what we're testing is the slot ordering, not acceptance.
    let mut candidates = Vec::new();
    candidates.push(EmergencyFillFixtures::candidate_with(
        4000,
        70,
        28,
        PlayerFieldPositionGroup::Goalkeeper,
        true,
        1.0,
        3500,
    ));
    for i in 0..3 {
        candidates.push(EmergencyFillFixtures::candidate_with(
            4100 + i,
            75,
            28,
            PlayerFieldPositionGroup::Defender,
            true,
            1.0,
            3500,
        ));
    }

    let mut signings = Vec::new();
    EmergencyFillFixtures::run_emergency(
        &country,
        &candidates,
        &TransferConfig::default(),
        &mut signings,
    );
    // Buyer projected 10 → first signing (GK) makes it 11; urgent
    // flag turns off afterwards. The pass must still be able to
    // sign defenders (group floor 7 > current 0) but uses non-
    // urgent rules. We can't assert "urgency was off" directly,
    // but we can assert the first signing was the keeper, since
    // GK gets explicit priority.
    assert!(
        signings
            .iter()
            .any(|s| s.reason.key == "emergency_squad_fill_gk"),
        "GK shortfall must be filled first when projected starts urgent"
    );
}

#[test]
fn elite_low_pressure_player_does_not_sign_for_low_rep_emergency_club() {
    // 800-rep amateur side, urgent (0 players). A CA-180 megastar
    // with low career pressure should not be signed even on the
    // urgent path — the scoring chasm gate (now relaxed for urgent
    // but still bounded) and the soft CA cap both block.
    let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country_with_reputation(vec![club], 800);

    let mega = EmergencyFillFixtures::candidate_with(
        5000,
        180,
        27,
        PlayerFieldPositionGroup::Midfielder,
        false,
        0.1,
        7500,
    );
    let candidates = vec![mega];

    let mut signings = Vec::new();
    EmergencyFillFixtures::run_emergency(
        &country,
        &candidates,
        &TransferConfig::default(),
        &mut signings,
    );
    assert!(
        !signings.iter().any(|s| s.player_id == 5000),
        "elite low-pressure player must not sign for low-rep urgent club"
    );
}

#[test]
fn accepted_emergency_signing_stages_wage_and_terms() {
    // After a successful emergency signing the staged terms must
    // travel with the signing so execution installs the agreed
    // wage and role promise rather than the calculator default.
    let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);

    let candidate = EmergencyFillFixtures::candidate_with(
        5100,
        80,
        29,
        PlayerFieldPositionGroup::Midfielder,
        true,
        0.7,
        3500,
    );
    let candidates = vec![candidate];

    let mut signings = Vec::new();
    EmergencyFillFixtures::run_emergency(
        &country,
        &candidates,
        &TransferConfig::default(),
        &mut signings,
    );
    let staged = signings
        .iter()
        .find(|s| s.player_id == 5100)
        .expect("acceptance should land the signing");
    let terms = staged.terms.expect("emergency pass must stage terms");
    assert!(terms.annual_wage > 0, "annual wage must be staged");
    assert!(
        terms.contract_years <= EmergencyContractTermsPolicy::YOUNG_USEFUL_YEARS,
        "emergency contract length must stay short"
    );
    assert_eq!(
        staged.fills_group,
        Some(PlayerFieldPositionGroup::Midfielder)
    );
}

#[test]
fn rejected_emergency_offer_updates_offered_and_rejected_ids() {
    // High-pressure offer that the buyer can't credibly match —
    // we set up a low-rep buyer + high-rep + low-pressure
    // candidate. Expected outcome: offer is made (offered_ids
    // populated) AND rejected (rejected_ids populated).
    // Determinism is tricky because the acceptance roll is RNG,
    // but the prob will be near zero when the buyer is tiny and
    // the player has no pressure.
    let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country_with_reputation(vec![club], 1200);

    // CA 170, low pressure, very high reference rep. Even on the
    // urgent path the chasm gate is now `2500 + 0.1*4500 = 2950`,
    // so 1200+2950=4150 vs 7800 ref rep — the gate fails and the
    // candidate is skipped before scoring (no offered/rejected).
    // For this test we want the offer to be MADE but rejected, so
    // pick a borderline candidate that passes the score gate but
    // fails the acceptance roll. CA 150, mid pressure, ref rep
    // 4500 — chasm: 2500+0.4*4500=4300, 1200+4300=5500 > 4500 ✓.
    let borderline = EmergencyFillFixtures::candidate_with(
        5200,
        150,
        27,
        PlayerFieldPositionGroup::Midfielder,
        false,
        0.4,
        4500,
    );
    let candidates = vec![borderline];

    let mut signings = Vec::new();
    let mut offered = Vec::new();
    let mut rejected = Vec::new();
    CountryResult::handle_free_agents_emergency_pass(
        &country,
        &candidates,
        &TransferConfig::default(),
        &FreeAgentMarketVisibility::build(0, &MarketMap::default(), &[]),
        &mut signings,
        &mut offered,
        &mut rejected,
        &mut BlockReasonRecorder::new(),
    );
    // If the offer was made at all, it must have been tracked. The
    // candidate is global-pool (club_id=0). Whether it accepted
    // depends on the RNG, but `offered_ids` is populated
    // regardless of outcome.
    if !signings.iter().any(|s| s.player_id == 5200) {
        // Rejected branch: offered AND rejected populated. The
        // score gate may filter the candidate before offering, in
        // which case neither is populated — that's also acceptable
        // behaviour (no offer made).
        if offered.contains(&5200) {
            assert!(
                rejected.contains(&5200),
                "an offer made and not signed must be tracked as rejected"
            );
        }
    }
}

#[test]
fn emergency_signing_marks_matching_transfer_request_fulfilled() {
    // Stage a transfer request for a defender on the club's
    // transfer plan; emergency signing should mark it fulfilled.
    let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
    let mut club = EmergencyFillFixtures::club(100, "FC", main);
    club.transfer_plan.initialized = true;
    club.transfer_plan
        .transfer_requests
        .push(TransferRequest::new(
            1,
            PlayerPositionType::DefenderCenter,
            TransferNeedPriority::Critical,
            TransferNeedReason::SquadPadding,
            50,
            80,
            0.0,
        ));
    let mut country = EmergencyFillFixtures::country(vec![club]);

    let candidate = EmergencyFillFixtures::candidate_with(
        5300,
        75,
        29,
        PlayerFieldPositionGroup::Defender,
        true,
        0.6,
        3500,
    );
    let candidates = vec![candidate];

    // Drive the full handle_free_agents path so the post-signing
    // sync runs. This requires the same context handle_free_agents
    // builds — we approximate by running the pass and then the
    // sync helper directly.
    let mut signings = Vec::new();
    let mut offered = Vec::new();
    let mut rejected = Vec::new();
    CountryResult::handle_free_agents_emergency_pass(
        &country,
        &candidates,
        &TransferConfig::default(),
        &FreeAgentMarketVisibility::build(0, &MarketMap::default(), &[]),
        &mut signings,
        &mut offered,
        &mut rejected,
        &mut BlockReasonRecorder::new(),
    );
    if let Some(signing) = signings.iter().find(|s| s.player_id == 5300) {
        if let Some(group) = signing.fills_group {
            TransferPlanSync::mark_group_fulfilled(&mut country, signing.to_club_id, group);
        }
    }
    let club = &country.clubs[0];
    // Either: the request was marked fulfilled by the sync helper,
    // or the candidate wasn't accepted (RNG dependent) and the
    // request stays pending.
    let request = club
        .transfer_plan
        .transfer_requests
        .iter()
        .find(|r| r.id == 1)
        .expect("staged request must survive the pass");
    if signings.iter().any(|s| s.player_id == 5300) {
        assert_eq!(
            request.status,
            TransferRequestStatus::Fulfilled,
            "matching request must be fulfilled after a successful signing"
        );
    }
}

#[test]
fn depth_fill_rotates_into_thinnest_group_not_always_midfield() {
    // 25 players (no group shortage) wouldn't trigger emergency.
    // Construct a club at exactly the group minimums (GK 2 / DEF 7
    // / FWD 4 / MID 7 = 20). Total 20 sits under the threshold of
    // 18 — wait, 20 > 18 so the pass exits early. Lower DEF count
    // by 2 so the depth slot rotates into DEF as the thinnest
    // group (1 short vs MID/FWD/GK at minimums).
    //
    // We test the thinnest_group helper directly because the
    // full-pass scoring randomness makes integration testing
    // flaky.
    let needs = FirstTeamSquadNeeds {
        main_team_size: 18,
        total_missing: 7,
        gk_missing: 0,
        def_missing: 2,
        mid_missing: 0,
        fwd_missing: 0,
        gk_count: 2,
        def_count: 5,
        mid_count: 7,
        fwd_count: 4,
        urgent: false,
    };
    let projected = EmergencyProjectedSquad::from_needs(&needs);
    assert_eq!(
        projected.thinnest_group(),
        PlayerFieldPositionGroup::Defender,
        "depth fill must rotate into the currently thinnest group"
    );
}

#[test]
fn country_cap_still_limits_one_country_pool() {
    // Two unplayable clubs in the same country with a massive
    // candidate pool — country cap (default 20) must bound the
    // total even though each club individually would otherwise
    // sign the full playable-size lift.
    let main_a = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
    let club_a = EmergencyFillFixtures::club(100, "FCA", main_a);

    let main_b = EmergencyFillFixtures::team(20, "ZZ", "zz", Vec::new());
    // Use a different club id to avoid the same-club skip.
    let club_b = Club::new(
        200,
        "FCB".to_string(),
        Location::new(1),
        ClubFinances::new(1_000_000, Vec::new()),
        ClubAcademy::new(3),
        ClubStatus::Professional,
        ClubColors::default(),
        TeamCollection::new(vec![main_b]),
        ClubFacilities::default(),
    );
    let country = EmergencyFillFixtures::country(vec![club_a, club_b]);

    let mut candidates = Vec::new();
    for i in 0..40 {
        candidates.push(EmergencyFillFixtures::candidate(
            6000 + i,
            70,
            28,
            if i % 4 == 0 {
                PlayerFieldPositionGroup::Goalkeeper
            } else if i % 4 == 1 {
                PlayerFieldPositionGroup::Defender
            } else if i % 4 == 2 {
                PlayerFieldPositionGroup::Midfielder
            } else {
                PlayerFieldPositionGroup::Forward
            },
            true,
        ));
    }

    let mut signings = Vec::new();
    let config = TransferConfig::default();
    EmergencyFillFixtures::run_emergency(&country, &candidates, &config, &mut signings);
    assert!(
        signings.len() <= config.emergency_max_signings_per_country_per_day,
        "country cap exceeded: {} signings, cap {}",
        signings.len(),
        config.emergency_max_signings_per_country_per_day
    );
}

/// Test fixtures for the realism-gate / cross-region tests added
/// alongside the strictness rework. Kept on a dedicated struct so
/// the original [`EmergencyFillFixtures`] helpers stay focused on
/// the existing pipeline tests and the new cases can dial in
/// continent / code / region without rewriting the shared
/// helpers.
struct CrossRegionFixtures;

impl CrossRegionFixtures {
    /// Buyer context for the picker / gate tests. Builds an
    /// Algerian-style low-rep North-African buyer when `algerian`
    /// is true, an English-style mid-rep European buyer otherwise.
    /// Strictness is exposed so a single fixture works for the
    /// depth (Strict) and GK (Flexible) variants.
    fn buyer(
        algerian: bool,
        strictness: EmergencyStrictness,
        urgent: bool,
    ) -> EmergencyBuyerContext {
        let (rep, code, continent, region_prestige, club_score, league_rep) = if algerian {
            (
                1500,
                "dz".to_string(),
                0u32,
                ScoutingRegion::from_country(0, "dz").league_prestige(),
                0.18,
                1400u16,
            )
        } else {
            (
                5000,
                "en".to_string(),
                1u32,
                ScoutingRegion::from_country(1, "en").league_prestige(),
                0.55,
                4800u16,
            )
        };
        EmergencyBuyerContext {
            country_reputation: rep,
            country_code: code,
            continent_id: continent,
            region_prestige,
            club_reputation_score: club_score,
            league_reputation: league_rep,
            negotiator_skill: 50,
            urgent,
            strictness,
            import_capacity: 1.0,
            country_id: continent,
            foreign_slots_free: None,
        }
    }

    /// Build a free-agent candidate in the global pool with an
    /// explicit nationality (continent + code). Lets a single
    /// helper cover Russian (`ru`, continent 1), Algerian (`dz`,
    /// continent 0), and any other cross-region setup the gate
    /// tests need.
    fn candidate(
        player_id: u32,
        ability: u8,
        age: u8,
        group: PlayerFieldPositionGroup,
        code: &str,
        continent_id: u32,
        nationality_country_reputation: u16,
        reference_reputation: u16,
        career_pressure: f32,
    ) -> FreeAgentCandidate {
        FreeAgentCandidate {
            player_id,
            player_name: format!("Cand{player_id}"),
            club_id: 0,
            club_name: "Free Agent".to_string(),
            ability,
            potential: ability.saturating_add(5),
            age,
            position_group: group,
            days_to_expiry: 0,
            nationality_country_reputation,
            nationality_region: ScoutingRegion::from_country(continent_id, code),
            nationality_country_code: code.to_string(),
            nationality_continent_id: continent_id,
            career_pressure,
            days_free: 0,
            reference_reputation,
            last_salary: 40_000,
            last_country_reputation: nationality_country_reputation,
            last_league_reputation: ((nationality_country_reputation as f32) * 0.85) as u16,
            world_reputation: 1200,
            current_reputation: 1200,
            professionalism_norm: 0.5,
            failed_approach_streak: 0,
            is_global_pool: true,
            nationality_country_id: 0,
            last_country_id: 0,
        }
    }
}

#[test]
fn realism_region_gate_blocks_russian_to_algerian_depth_at_low_pressure() {
    // Russian player + Algerian club + Strict (depth) slot must
    // block before scoring even runs. Pressure 0.5 is comfortably
    // below the `Strict + cross-continent` cutoff of 0.85.
    let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
    let russian = CrossRegionFixtures::candidate(
        1,
        75,
        27,
        PlayerFieldPositionGroup::Defender,
        "ru",
        1,
        3000,
        3500,
        0.5,
    );
    assert!(
        !EmergencyRealismGates::passes_region(&russian, &buyer),
        "Strict + cross-continent + pressure 0.5 must fail the region gate"
    );
    assert!(
        EmergencyRealismGates::evaluate(
            &russian,
            &buyer,
            PlayerFieldPositionGroup::Defender,
            &FreeAgentMarketVisibility::build(0, &MarketMap::default(), &[]),
        )
        .is_err(),
        "the composite gate must reject the same case"
    );
}

/// A world with the corridors § B4 and § B5 argue about: Russia →
/// Turkey is a real corridor both cards name, Russia → Brazil and
/// Russia → Cameroon are named by nobody.
struct EmergencyMarketFixtures;

impl EmergencyMarketFixtures {
    const RU: u32 = 1;
    const TR: u32 = 2;
    const BR: u32 = 3;
    const CM: u32 = 4;

    fn facts(id: u32, code: &str, continent: u32, top: u16, wage: u32) -> MarketCountryFacts {
        MarketCountryFacts {
            id,
            code: code.to_string(),
            continent_id: continent,
            region: ScoutingRegion::from_country(continent, code),
            reputation: top,
            top_flight_reputation: top,
            median_top_flight_wage: wage,
        }
    }

    fn weight(country_id: u32, weight: f32) -> CorridorWeight {
        CorridorWeight {
            country_id,
            weight,
            money: false,
        }
    }

    fn world() -> MarketMap {
        let mut facts = HashMap::new();
        for f in [
            Self::facts(Self::RU, "ru", 1, 6500, 900_000),
            Self::facts(Self::TR, "tr", 1, 7000, 900_000),
            Self::facts(Self::BR, "br", 3, 7800, 500_000),
            Self::facts(Self::CM, "cm", 0, 2000, 20_000),
        ] {
            facts.insert(f.id, f);
        }
        let mut profiles = HashMap::new();
        profiles.insert(
            Self::TR,
            CountryTransferProfile {
                import: vec![Self::weight(Self::BR, 1.0), Self::weight(Self::RU, 0.6)],
                foreign_share: 0.58,
                ..Default::default()
            },
        );
        profiles.insert(
            Self::RU,
            CountryTransferProfile {
                export: vec![Self::weight(Self::TR, 1.0)],
                foreign_share: 0.35,
                ..Default::default()
            },
        );
        profiles.insert(
            Self::BR,
            CountryTransferProfile {
                export: vec![Self::weight(Self::TR, 0.3)],
                foreign_share: 0.07,
                ..Default::default()
            },
        );
        profiles.insert(
            Self::CM,
            CountryTransferProfile {
                foreign_share: 0.06,
                ..Default::default()
            },
        );
        MarketMap::new(profiles, facts)
    }

    /// A Russian journeyman with a given time on the market.
    fn russian(
        player_id: u32,
        group: PlayerFieldPositionGroup,
        days_free: i64,
        career_pressure: f32,
    ) -> FreeAgentCandidate {
        let mut candidate = CrossRegionFixtures::candidate(
            player_id,
            72,
            29,
            group,
            "ru",
            1,
            6500,
            3500,
            career_pressure,
        );
        candidate.nationality_country_id = Self::RU;
        candidate.last_country_id = Self::RU;
        candidate.days_free = days_free;
        candidate
    }

    fn buyer(
        country_id: u32,
        code: &str,
        continent_id: u32,
        strictness: EmergencyStrictness,
        import_capacity: f32,
    ) -> EmergencyBuyerContext {
        EmergencyBuyerContext {
            country_reputation: 6000,
            country_code: code.to_string(),
            continent_id,
            region_prestige: ScoutingRegion::from_country(continent_id, code).league_prestige(),
            club_reputation_score: 0.45,
            league_reputation: 5500,
            negotiator_skill: 50,
            urgent: true,
            strictness,
            import_capacity,
            country_id,
            foreign_slots_free: None,
        }
    }
}

/// § B4 — the emergency fill is the literal "random team that urgently
/// needs a player" of the original complaint, and it was the ONE door
/// with no visibility gate at all: the corridor entered as six points of
/// score, which a thin candidate pool erases.
#[test]
fn an_emergency_depth_slot_cannot_see_a_market_nobody_here_works() {
    let map = EmergencyMarketFixtures::world();
    let brazil = EmergencyMarketFixtures::buyer(
        EmergencyMarketFixtures::BR,
        "br",
        3,
        EmergencyStrictness::Strict,
        0.10,
    );
    for (days_free, pressure) in [(0i64, 0.2f32), (120, 0.6), (400, 0.95)] {
        let russian = EmergencyMarketFixtures::russian(
            900,
            PlayerFieldPositionGroup::Defender,
            days_free,
            pressure,
        );
        let visibility = FreeAgentMarketVisibility::build(
            EmergencyMarketFixtures::BR,
            &map,
            std::slice::from_ref(&russian),
        );
        assert!(
            !EmergencyRealismGates::passes_market(&russian, &brazil, &visibility),
            "a Russian journeyman must stay invisible to a Brazilian depth slot \
             (days_free={days_free}, pressure={pressure})"
        );
    }
}

/// The other half of the same gate: it must not close a corridor the
/// cards actually name. Turkey imports Russians and Russia's card names
/// Turkey back, so a released Russian is a Turkish target on day one.
#[test]
fn an_emergency_depth_slot_sees_a_market_its_league_actually_works() {
    let map = EmergencyMarketFixtures::world();
    let turkey = EmergencyMarketFixtures::buyer(
        EmergencyMarketFixtures::TR,
        "tr",
        1,
        EmergencyStrictness::Strict,
        0.58,
    );
    let russian = EmergencyMarketFixtures::russian(901, PlayerFieldPositionGroup::Defender, 0, 0.2);
    let visibility = FreeAgentMarketVisibility::build(
        EmergencyMarketFixtures::TR,
        &map,
        std::slice::from_ref(&russian),
    );
    assert!(
        EmergencyRealismGates::passes_market(&russian, &turkey, &visibility),
        "Russia → Turkey is the corridor the cards name; day one must be visible"
    );
}

/// The strictness ladder loosens the bar and never removes it. A
/// no-keeper Cameroonian club looks harder than a Brazilian depth slot
/// — two stages looser rather than one — and a Russian is still not
/// somebody Yaoundé has heard of, however long he has been available.
#[test]
fn the_no_keeper_carve_out_loosens_the_market_gate_without_opening_it() {
    let map = EmergencyMarketFixtures::world();
    let cameroon = EmergencyMarketFixtures::buyer(
        EmergencyMarketFixtures::CM,
        "cm",
        0,
        EmergencyStrictness::Flexible,
        0.06,
    );
    let keeper =
        EmergencyMarketFixtures::russian(902, PlayerFieldPositionGroup::Goalkeeper, 400, 0.95);
    let visibility = FreeAgentMarketVisibility::build(
        EmergencyMarketFixtures::CM,
        &map,
        std::slice::from_ref(&keeper),
    );
    assert!(
        !EmergencyRealismGates::passes_market(&keeper, &cameroon, &visibility),
        "the Flexible relief is two stages of the ladder, not the removal of it"
    );
    // And the relief is real where there IS a corridor: the same man,
    // the same day, read by a market that works Russia.
    let turkey = EmergencyMarketFixtures::buyer(
        EmergencyMarketFixtures::TR,
        "tr",
        1,
        EmergencyStrictness::Flexible,
        0.58,
    );
    let turkish_view = FreeAgentMarketVisibility::build(
        EmergencyMarketFixtures::TR,
        &map,
        std::slice::from_ref(&keeper),
    );
    assert!(EmergencyRealismGates::passes_market(
        &keeper,
        &turkey,
        &turkish_view
    ));
}

/// § B8 — the emergency pass is not an exemption from registration.
#[test]
fn an_emergency_slot_will_not_sign_a_foreigner_it_cannot_register() {
    let map = EmergencyMarketFixtures::world();
    let mut turkey = EmergencyMarketFixtures::buyer(
        EmergencyMarketFixtures::TR,
        "tr",
        1,
        EmergencyStrictness::Flexible,
        0.58,
    );
    let russian = EmergencyMarketFixtures::russian(903, PlayerFieldPositionGroup::Defender, 0, 0.9);
    let visibility = FreeAgentMarketVisibility::build(
        EmergencyMarketFixtures::TR,
        &map,
        std::slice::from_ref(&russian),
    );
    turkey.foreign_slots_free = Some(1);
    assert!(
        EmergencyRealismGates::evaluate(
            &russian,
            &turkey,
            PlayerFieldPositionGroup::Defender,
            &visibility,
        )
        .is_ok(),
        "a club with a slot left may spend it"
    );
    turkey.foreign_slots_free = Some(0);
    assert_eq!(
        EmergencyRealismGates::evaluate(
            &russian,
            &turkey,
            PlayerFieldPositionGroup::Defender,
            &visibility,
        ),
        Err(FreeAgentBlockReason::NoRegistrationSlot),
    );
    // A domestic candidate never consumes a slot, so the same full
    // quota does not touch him.
    let mut turk = russian.clone();
    turk.nationality_country_id = EmergencyMarketFixtures::TR;
    turk.nationality_country_code = "tr".to_string();
    assert!(!turkey.would_block_registration(turk.nationality_country_id));
}

#[test]
fn realism_region_gate_passes_russian_to_algerian_at_very_high_pressure() {
    // Same cross-continent move but with the player on the very
    // verge of retiring (pressure 0.92) — Strict region gate now
    // lets it through. The rep / quality gates do their own
    // checks; the test isolates the region behaviour.
    let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
    let russian = CrossRegionFixtures::candidate(
        2,
        70,
        33,
        PlayerFieldPositionGroup::Defender,
        "ru",
        1,
        1800,
        1700,
        0.92,
    );
    assert!(
        EmergencyRealismGates::passes_region(&russian, &buyer),
        "Strict + cross-continent at high pressure must clear the region gate"
    );
}

#[test]
fn realism_region_gate_lets_gk_flexible_pass_where_depth_strict_blocks() {
    // Same candidate, same buyer — only the slot strictness
    // changes. Flexible (no-keeper GK fill) now requires a 0.65
    // pressure floor, so the test runs at 0.70: well past Flexible
    // but below Strict's 0.85 floor. Tests the strictness dial
    // directly without leaning on the old "any pressure" carve-out.
    let cross = CrossRegionFixtures::candidate(
        3,
        72,
        30,
        PlayerFieldPositionGroup::Goalkeeper,
        "ru",
        1,
        2200,
        2400,
        0.70,
    );
    let gk_buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Flexible, true);
    let depth_buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
    assert!(
        EmergencyRealismGates::passes_region(&cross, &gk_buyer),
        "Flexible GK fill should accept a cross-region keeper past its 0.65 floor"
    );
    assert!(
        !EmergencyRealismGates::passes_region(&cross, &depth_buyer),
        "Strict depth fill must reject the same candidate"
    );
}

#[test]
fn realism_region_gate_blocks_russian_to_african_gk_at_routine_pressure() {
    // Regression: a Russian free-agent keeper was signing for a
    // Cameroonian club via `emergency_squad_fill_gk` (Flexible
    // strictness) at routine career pressure. The Flexible floor
    // of 0.65 must block the move; only a player well past peak
    // is allowed to cross continents into a markedly weaker region
    // even for a no-keeper slot.
    let cameroonian_buyer = EmergencyBuyerContext {
        country_reputation: 1100,
        country_code: "cm".to_string(),
        continent_id: 0,
        region_prestige: ScoutingRegion::from_country(0, "cm").league_prestige(),
        club_reputation_score: 0.14,
        league_reputation: 1000,
        negotiator_skill: 50,
        urgent: true,
        strictness: EmergencyStrictness::Flexible,
        import_capacity: 1.0,
        country_id: 0,
        foreign_slots_free: None,
    };
    let russian_gk = CrossRegionFixtures::candidate(
        60,
        70,
        28,
        PlayerFieldPositionGroup::Goalkeeper,
        "ru",
        1,
        2200,
        2200,
        0.45,
    );
    assert!(
        !EmergencyRealismGates::passes_region(&russian_gk, &cameroonian_buyer),
        "Flexible GK fill + Russian → Cameroon at routine pressure must remain blocked"
    );
}

#[test]
fn realism_region_gate_passes_russian_to_african_gk_at_high_pressure() {
    // Same Russian → Cameroonian GK case as the blocking test
    // above, but at 0.78 — comfortably above the Flexible floor
    // of 0.65 and the Standard floor of 0.75. A near-retirement
    // veteran landing a desperation no-keeper slot is the
    // realistic carve-out the dial is meant to allow.
    let cameroonian_buyer = EmergencyBuyerContext {
        country_reputation: 1100,
        country_code: "cm".to_string(),
        continent_id: 0,
        region_prestige: ScoutingRegion::from_country(0, "cm").league_prestige(),
        club_reputation_score: 0.14,
        league_reputation: 1000,
        negotiator_skill: 50,
        urgent: true,
        strictness: EmergencyStrictness::Flexible,
        import_capacity: 1.0,
        country_id: 0,
        foreign_slots_free: None,
    };
    let russian_gk = CrossRegionFixtures::candidate(
        61,
        68,
        34,
        PlayerFieldPositionGroup::Goalkeeper,
        "ru",
        1,
        1600,
        1500,
        0.78,
    );
    assert!(
        EmergencyRealismGates::passes_region(&russian_gk, &cameroonian_buyer),
        "Flexible GK fill at high pressure must clear the region gate"
    );
}

#[test]
fn picker_prefers_domestic_depth_over_higher_ca_foreign() {
    // Two candidates for a Strict (depth) defender slot at an
    // Algerian buyer: a domestic Algerian at CA 65 and a Russian
    // at CA 75 on full pressure (so the Russian could in principle
    // clear the region gate). Locality ordering must still pick
    // the Algerian — depth is not about raw ability.
    let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
    let algerian = CrossRegionFixtures::candidate(
        10,
        65,
        29,
        PlayerFieldPositionGroup::Defender,
        "dz",
        0,
        1500,
        1500,
        0.6,
    );
    let russian = CrossRegionFixtures::candidate(
        11,
        75,
        33,
        PlayerFieldPositionGroup::Defender,
        "ru",
        1,
        2200,
        2000,
        0.95,
    );
    let candidates = vec![russian, algerian];
    let signings: Vec<FreeAgentSigning> = Vec::new();
    let rejected: HashSet<u32> = HashSet::new();
    let slot = EmergencyGroupSlot {
        group: PlayerFieldPositionGroup::Defender,
        missing: 1,
        reason: "emergency_squad_fill_depth",
    };
    let pick = EmergencyCandidatePicker::pick(
        &candidates,
        &signings,
        &rejected,
        slot,
        &buyer,
        999,
        &FreeAgentMarketVisibility::build(0, &MarketMap::default(), &[]),
        &mut BlockReasonRecorder::new(),
    );
    let picked = pick.expect("at least one candidate must clear all gates");
    assert_eq!(
        picked.player_id, 10,
        "Strict depth at Algeria must prefer the domestic CA-65 Algerian over the foreign CA-75 Russian"
    );
}

#[test]
fn picker_skips_only_unrealistic_candidates_for_depth() {
    // Only candidate available is a low-pressure Russian against
    // an Algerian Strict (depth) slot. With no domestic / closer
    // alternative the picker should return None rather than fall
    // back to the unrealistic cross-region option.
    let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
    let russian = CrossRegionFixtures::candidate(
        20,
        80,
        27,
        PlayerFieldPositionGroup::Midfielder,
        "ru",
        1,
        3000,
        3500,
        0.4,
    );
    let candidates = vec![russian];
    let signings: Vec<FreeAgentSigning> = Vec::new();
    let rejected: HashSet<u32> = HashSet::new();
    let slot = EmergencyGroupSlot {
        group: PlayerFieldPositionGroup::Midfielder,
        missing: 1,
        reason: "emergency_squad_fill_depth",
    };
    let pick = EmergencyCandidatePicker::pick(
        &candidates,
        &signings,
        &rejected,
        slot,
        &buyer,
        999,
        &FreeAgentMarketVisibility::build(0, &MarketMap::default(), &[]),
        &mut BlockReasonRecorder::new(),
    );
    assert!(
        pick.is_none(),
        "depth slot must skip rather than fall back to an unrealistic cross-region pick"
    );
}

#[test]
fn pressure_threshold_separates_blocked_from_passing_step_down() {
    // Same Russian candidate against the same Algerian Strict
    // depth slot, only career pressure changes. Low pressure
    // must fail every gate; very high pressure must clear the
    // region gate. This proves pressure is the dial that
    // unlocks realistic step-downs.
    let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
    let low_pressure = CrossRegionFixtures::candidate(
        30,
        70,
        33,
        PlayerFieldPositionGroup::Defender,
        "ru",
        1,
        1800,
        1700,
        0.2,
    );
    let high_pressure = CrossRegionFixtures::candidate(
        31,
        70,
        33,
        PlayerFieldPositionGroup::Defender,
        "ru",
        1,
        1800,
        1700,
        0.95,
    );
    assert!(
        !EmergencyRealismGates::passes_region(&low_pressure, &buyer),
        "low-pressure cross-continent depth must remain blocked"
    );
    assert!(
        EmergencyRealismGates::passes_region(&high_pressure, &buyer),
        "very high pressure must unlock the region gate"
    );
}

#[test]
fn depth_strictness_does_not_get_urgent_rep_bonus() {
    // High-rep Russian candidate, low-rep buyer. The 400-point
    // Standard rep bonus / 800-point Flexible rep bonus must NOT
    // apply for Strict depth — otherwise the urgent uplift
    // creeps into the depth path. Demonstrates the difference
    // between strictness levels at the rep gate.
    let candidate = CrossRegionFixtures::candidate(
        40,
        85,
        30,
        PlayerFieldPositionGroup::Midfielder,
        "ru",
        1,
        3500,
        3500,
        0.4,
    );
    let strict_buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
    let flex_buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Flexible, true);
    let strict_pass = EmergencyRealismGates::passes_reputation(&candidate, &strict_buyer);
    let flex_pass = EmergencyRealismGates::passes_reputation(&candidate, &flex_buyer);
    assert!(
        flex_pass || !strict_pass,
        "Flexible rep gate must be at least as permissive as Strict — \
         strict_pass={strict_pass} flex_pass={flex_pass}"
    );
}

#[test]
fn realism_region_gate_blocks_russian_to_algerian_standard_at_low_pressure() {
    // Standard slot (urgent sub-11 outfield fill) now also fires
    // the hard cross-continent guard. Same Russian → Algerian
    // case as the Strict test, but at the Standard pressure
    // floor (0.75) instead of 0.85.
    let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Standard, true);
    let russian = CrossRegionFixtures::candidate(
        50,
        75,
        27,
        PlayerFieldPositionGroup::Defender,
        "ru",
        1,
        3000,
        3500,
        0.5,
    );
    assert!(
        !EmergencyRealismGates::passes_region(&russian, &buyer),
        "Standard urgent fill + cross-continent + pressure 0.5 must still fail the region gate"
    );
}

#[test]
fn realism_region_gate_passes_russian_to_algerian_standard_at_high_pressure() {
    // The Standard floor is 0.75 — at 0.80 the Russian veteran
    // can land in Algeria for an urgent group fill, mirroring
    // the Strict path's "verge of retiring" carve-out.
    let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Standard, true);
    let russian = CrossRegionFixtures::candidate(
        51,
        70,
        33,
        PlayerFieldPositionGroup::Defender,
        "ru",
        1,
        1800,
        1700,
        0.80,
    );
    assert!(
        EmergencyRealismGates::passes_region(&russian, &buyer),
        "Standard slot at very high pressure must clear the region gate"
    );
}

#[test]
fn slot_strictness_maps_correctly_from_reason() {
    // Sanity check that the policy struct routes each emergency
    // reason to the strictness the spec calls for.
    assert_eq!(
        EmergencySlotStrictness::from_reason("emergency_squad_fill_gk", true),
        EmergencyStrictness::Flexible
    );
    assert_eq!(
        EmergencySlotStrictness::from_reason("emergency_squad_fill_def", true),
        EmergencyStrictness::Standard
    );
    assert_eq!(
        EmergencySlotStrictness::from_reason("emergency_squad_fill_def", false),
        EmergencyStrictness::Strict
    );
    assert_eq!(
        EmergencySlotStrictness::from_reason("emergency_squad_fill_depth", true),
        EmergencyStrictness::Strict
    );
    assert_eq!(
        EmergencySlotStrictness::from_reason("emergency_squad_fill_depth", false),
        EmergencyStrictness::Strict
    );
}

/// Fixtures for the depth-through-pipeline tests. Separate struct
/// so the squad / pool-snapshot builders the new flow needs don't
/// bloat the shared [`EmergencyFillFixtures`] helpers.
struct DepthPipelineFixtures;

impl DepthPipelineFixtures {
    /// Balanced 20-man squad (2 GK / 7 DEF / 7 MID / 4 FWD): every
    /// group minimum met and total above the emergency threshold,
    /// so the emergency pass skips the club entirely and only the
    /// staged DepthCover request drives market activity.
    fn balanced_squad() -> Vec<Player> {
        let mut players: Vec<Player> = Vec::new();
        for i in 0..2 {
            players.push(EmergencyFillFixtures::player(
                i,
                PlayerPositionType::Goalkeeper,
            ));
        }
        for i in 0..7 {
            players.push(EmergencyFillFixtures::player(
                10 + i,
                PlayerPositionType::DefenderCenter,
            ));
        }
        for i in 0..7 {
            players.push(EmergencyFillFixtures::player(
                20 + i,
                PlayerPositionType::MidfielderCenter,
            ));
        }
        for i in 0..4 {
            players.push(EmergencyFillFixtures::player(
                30 + i,
                PlayerPositionType::Striker,
            ));
        }
        players
    }

    /// Snapshot row for the global free-agent pool input of
    /// `handle_free_agents`.
    fn pool_summary(
        player_id: u32,
        ability: u8,
        age: u8,
        group: PlayerFieldPositionGroup,
        same_country: bool,
        career_pressure: f32,
        reference_reputation: u16,
    ) -> GlobalFreeAgentSummary {
        let code = if same_country { "en" } else { "ar" };
        GlobalFreeAgentSummary {
            player_id,
            player_name: format!("Pool{player_id}"),
            ability,
            potential: ability.saturating_add(5),
            age,
            position_group: group,
            nationality_country_reputation: reference_reputation,
            nationality_continent_id: if same_country { 1 } else { 3 },
            nationality_country_code: code.to_string(),
            nationality_country_id: if same_country { 1 } else { 2 },
            career_pressure,
            days_free: 0,
            reference_reputation,
            last_salary: 50_000,
            last_country_reputation: reference_reputation,
            last_league_reputation: ((reference_reputation as f32) * 0.85) as u16,
            world_reputation: 1500,
            current_reputation: 1500,
            professionalism_norm: 0.5,
            failed_approach_streak: 0,
            last_country_id: if same_country { 1 } else { 2 },
        }
    }

    /// Drive `handle_free_agents` until the staged-negotiation
    /// matcher fires (the daily-chance roll is probabilistic) or
    /// `max_ticks` pass. Returns the pool signings the calls
    /// produced plus the offered / rejected side-channels.
    fn run_until_negotiation(
        country: &mut Country,
        pool: &[GlobalFreeAgentSummary],
        max_ticks: usize,
    ) -> (Vec<GlobalFreeAgentSigning>, Vec<u32>, Vec<u32>) {
        let date = EmergencyFillFixtures::d(2026, 6, 10);
        let config = TransferConfig::default();
        let mut all_signings = Vec::new();
        let mut offered = Vec::new();
        let mut rejected = Vec::new();
        for _ in 0..max_ticks {
            let mut summary = TransferActivitySummary::new();
            let mut domestic = Vec::new();
            let mut blocked = Vec::new();
            let signings = CountryResult::handle_free_agents(
                country,
                date,
                &mut summary,
                pool,
                &MarketMap::default(),
                &config,
                &mut domestic,
                &mut offered,
                &mut rejected,
                &mut blocked,
            );
            all_signings.extend(signings);
            if !country.transfer_market.negotiations.is_empty() {
                break;
            }
        }
        (all_signings, offered, rejected)
    }

    /// Balanced club + staged midfield DepthCover request + one
    /// domestic pool journeyman — the canonical "depth fill should
    /// negotiate" setup the flow tests share.
    fn staged_depth_country() -> (Country, Vec<GlobalFreeAgentSummary>) {
        let main =
            EmergencyFillFixtures::team(10, "FC", "fc", DepthPipelineFixtures::balanced_squad());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let mut country = EmergencyFillFixtures::country(vec![club]);
        country.clubs[0].transfer_plan.initialized = true;
        EmergencyDepthRequestPlanner::stage_requests(
            &mut country,
            &[EmergencyDepthRequestIntent {
                club_id: 100,
                group: PlayerFieldPositionGroup::Midfielder,
            }],
        );
        let pool = vec![DepthPipelineFixtures::pool_summary(
            9000,
            80,
            28,
            PlayerFieldPositionGroup::Midfielder,
            true,
            0.6,
            4000,
        )];
        (country, pool)
    }
}

#[test]
fn depth_slot_stages_pipeline_request_instead_of_direct_signing() {
    // Squad of 14: MID six short, every other group at its minimum.
    // The candidate pool carries no midfielders, so the MID slot is
    // dead this tick and the planner falls to the depth tail —
    // which previously direct-signed a defender under
    // `emergency_squad_fill_depth`. Now it must return an intent
    // and sign nothing.
    let mut players: Vec<Player> = Vec::new();
    for i in 0..2 {
        players.push(EmergencyFillFixtures::player(
            i,
            PlayerPositionType::Goalkeeper,
        ));
    }
    for i in 0..7 {
        players.push(EmergencyFillFixtures::player(
            10 + i,
            PlayerPositionType::DefenderCenter,
        ));
    }
    players.push(EmergencyFillFixtures::player(
        20,
        PlayerPositionType::MidfielderCenter,
    ));
    for i in 0..4 {
        players.push(EmergencyFillFixtures::player(
            30 + i,
            PlayerPositionType::Striker,
        ));
    }

    let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let mut country = EmergencyFillFixtures::country(vec![club]);

    let candidates: Vec<FreeAgentCandidate> = (0..5)
        .map(|i| {
            EmergencyFillFixtures::candidate(
                7000 + i,
                75,
                28,
                PlayerFieldPositionGroup::Defender,
                true,
            )
        })
        .collect();

    let mut signings = Vec::new();
    let mut offered = Vec::new();
    let mut rejected = Vec::new();
    let intents = CountryResult::handle_free_agents_emergency_pass(
        &country,
        &candidates,
        &TransferConfig::default(),
        &FreeAgentMarketVisibility::build(0, &MarketMap::default(), &[]),
        &mut signings,
        &mut offered,
        &mut rejected,
        &mut BlockReasonRecorder::new(),
    );

    assert!(
        signings.is_empty(),
        "depth tail must not direct-sign, got {:?}",
        signings
            .iter()
            .map(|s| (s.player_id, &s.reason))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        intents.len(),
        1,
        "depth shortfall must surface as an intent"
    );
    assert_eq!(intents[0].group, PlayerFieldPositionGroup::Defender);

    EmergencyDepthRequestPlanner::stage_requests(&mut country, &intents);
    {
        let plan = &country.clubs[0].transfer_plan;
        let request = plan
            .transfer_requests
            .iter()
            .find(|r| r.reason == TransferNeedReason::DepthCover)
            .expect("depth intent must stage a DepthCover request");
        assert_eq!(request.priority, TransferNeedPriority::Optional);
        assert_eq!(request.position, PlayerPositionType::DefenderCenter);
        assert_eq!(request.status, TransferRequestStatus::Pending);
    }

    // Re-staging while the request is open must not duplicate it.
    EmergencyDepthRequestPlanner::stage_requests(&mut country, &intents);
    let depth_count = country.clubs[0]
        .transfer_plan
        .transfer_requests
        .iter()
        .filter(|r| r.reason == TransferNeedReason::DepthCover)
        .count();
    assert_eq!(depth_count, 1, "open depth request must dedup re-staging");
}

#[test]
fn depth_request_creates_pending_personal_terms_negotiation() {
    RandomEngine::set_seed(0xDE91_07F1);
    let (mut country, pool) = DepthPipelineFixtures::staged_depth_country();

    let (signings, offered, _rejected) =
        DepthPipelineFixtures::run_until_negotiation(&mut country, &pool, 400);

    assert!(
        signings.is_empty(),
        "depth request must not produce an instant pool signing"
    );
    let negotiation = country
        .transfer_market
        .negotiations
        .values()
        .next()
        .expect("plausible domestic candidate must enter negotiation within 400 ticks");
    assert_eq!(negotiation.status, NegotiationStatus::Pending);
    assert!(
        matches!(negotiation.phase, NegotiationPhase::PersonalTerms { .. }),
        "staged depth negotiation must enter PersonalTerms, got {:?}",
        negotiation.phase
    );
    assert_eq!(negotiation.player_id, 9000);
    assert_eq!(
        negotiation.selling_club_id, 0,
        "pool free agents negotiate from the synthetic club-0 seller"
    );
    assert!(negotiation.offered_salary.unwrap_or(0) > 0);
    assert!(negotiation.current_offer.personal_terms.is_some());
    assert_eq!(
        negotiation.reason.key,
        TransferNeedReason::DepthCover.as_signing_reason_key(),
        "history reason must carry the depth motive, not a raw emergency tag"
    );
    assert!(
        country
            .transfer_market
            .negotiations
            .values()
            .all(|n| n.status != NegotiationStatus::Accepted),
        "depth path must never insert a pre-accepted negotiation"
    );
    assert!(country.transfer_market.transfer_history.is_empty());
    assert!(
        offered.contains(&9000),
        "negotiated offer must bump the offered counter"
    );

    let plan = &country.clubs[0].transfer_plan;
    let request = plan
        .transfer_requests
        .iter()
        .find(|r| r.reason == TransferNeedReason::DepthCover)
        .unwrap();
    assert_eq!(request.status, TransferRequestStatus::Negotiating);
    let shortlist = plan
        .shortlists
        .iter()
        .find(|s| s.transfer_request_id == request.id)
        .expect("staging must wire a shortlist for the request");
    assert!(shortlist.candidates.iter().any(
        |c| c.player_id == 9000 && c.status == ShortlistCandidateStatus::CurrentlyPursuing
    ));
}

#[test]
fn low_rep_club_cannot_depth_sign_high_rep_foreign_free_agent() {
    RandomEngine::set_seed(0x10F_FA11);
    // 800-rep country, routine pressure, CA-165 foreign star with a
    // 7500 reference reputation: the strict depth gates (tier CA
    // ceiling without overreach + pressure-scaled rep drop) must
    // filter the candidate before any offer or negotiation exists.
    let main = EmergencyFillFixtures::team(10, "FC", "fc", DepthPipelineFixtures::balanced_squad());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let mut country = EmergencyFillFixtures::country_with_reputation(vec![club], 800);
    country.clubs[0].transfer_plan.initialized = true;
    EmergencyDepthRequestPlanner::stage_requests(
        &mut country,
        &[EmergencyDepthRequestIntent {
            club_id: 100,
            group: PlayerFieldPositionGroup::Midfielder,
        }],
    );

    let pool = vec![DepthPipelineFixtures::pool_summary(
        9100,
        165,
        27,
        PlayerFieldPositionGroup::Midfielder,
        false,
        0.3,
        7500,
    )];
    let (signings, offered, _rejected) =
        DepthPipelineFixtures::run_until_negotiation(&mut country, &pool, 300);

    assert!(signings.is_empty(), "no pool signing may be staged");
    assert!(
        country.transfer_market.negotiations.is_empty(),
        "implausible star must never enter a depth negotiation at a low-rep club"
    );
    assert!(country.transfer_market.transfer_history.is_empty());
    assert!(
        !offered.contains(&9100),
        "filtered candidates must not be counted as offered"
    );
}

#[test]
fn depth_personal_terms_rejection_updates_request_and_shortlist() {
    RandomEngine::set_seed(0xBAD_7E55);
    let (mut country, pool) = DepthPipelineFixtures::staged_depth_country();
    DepthPipelineFixtures::run_until_negotiation(&mut country, &pool, 400);
    assert!(!country.transfer_market.negotiations.is_empty());

    // Mirror what `resolve_personal_terms` does on a declined offer
    // — the staged shortlist wiring must respond like any pipeline
    // pursuit: candidate marked failed, Optional request abandoned,
    // negotiation slot released.
    PipelineProcessor::on_negotiation_resolved(&mut country, 100, 9000, false);

    let plan = &country.clubs[0].transfer_plan;
    let request = plan
        .transfer_requests
        .iter()
        .find(|r| r.reason == TransferNeedReason::DepthCover)
        .unwrap();
    assert_eq!(
        request.status,
        TransferRequestStatus::Abandoned,
        "Optional depth request with an exhausted shortlist must be abandoned"
    );
    let shortlist = plan
        .shortlists
        .iter()
        .find(|s| s.transfer_request_id == request.id)
        .unwrap();
    assert!(shortlist.candidates.iter().any(
        |c| c.player_id == 9000 && c.status == ShortlistCandidateStatus::NegotiationFailed
    ));
    assert_eq!(plan.active_negotiation_count, 0);
}

#[test]
fn pool_depth_medical_completion_defers_global_signing_without_direct_history() {
    // The medical phase keeps an unconditional 1% collapse roll even
    // for healthy players, and the seeded RNG stream is mixed with a
    // per-test-thread id — so any single seed can land in the 1%
    // band depending on suite composition. Retry across a few seeds:
    // a genuine completion-path regression fails every attempt,
    // while the 1% artifact cannot survive eight (P ≈ 1e-16).
    for attempt in 0..8u64 {
        RandomEngine::set_seed(0xD0C7_0001 + attempt);
        let (mut country, pool) = DepthPipelineFixtures::staged_depth_country();
        DepthPipelineFixtures::run_until_negotiation(&mut country, &pool, 400);
        let neg_id = *country
            .transfer_market
            .negotiations
            .keys()
            .next()
            .expect("staging must have created the negotiation");

        // Fast-forward the negotiation to a mature medical phase so a
        // single resolver tick completes it.
        let date = EmergencyFillFixtures::d(2026, 6, 10);
        if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
            negotiation.phase = NegotiationPhase::MedicalAndFinalization { started: date };
            negotiation.phase_expiry = date;
        }

        // A second club has a competing pool bid in flight (not yet
        // phase-ready, so the resolver doesn't touch it directly) —
        // completion must sweep it like `complete_transfer` would.
        let competing_id = country.transfer_market.next_negotiation_id;
        country.transfer_market.next_negotiation_id += 1;
        country.transfer_market.negotiations.insert(
            competing_id,
            TransferNegotiation::new(
                competing_id,
                9000,
                0,
                0,
                200,
                TransferOffer::new(CurrencyValue::new(0.0, Currency::Usd), 200, date),
                date,
                0.4,
                0.3,
                28,
                0.5,
            ),
        );

        RandomEngine::set_seed(42 + attempt);
        let mut summary = TransferActivitySummary::new();
        let outcomes = CountryResult::resolve_pending_negotiations(
            &mut country,
            date,
            &MarketMap::default(),
            &mut summary,
        );

        // 1% medical collapse — the RNG artifact, not the behaviour
        // under test. Re-roll the scenario with the next seed.
        if country.transfer_market.negotiations[&neg_id].rejection_reason
            == Some(NegotiationRejectionReason::MedicalFailed)
        {
            continue;
        }

        assert!(
            outcomes.deferred.is_empty(),
            "pool free agents must not enter the club-to-club execution queue"
        );
        assert_eq!(
            outcomes.free_agent_signings.len(),
            1,
            "cleared medical must defer exactly one global pool signing"
        );
        let signing = &outcomes.free_agent_signings[0];
        assert_eq!(signing.player_id, 9000);
        assert_eq!(signing.buying_club_id, 100);
        assert_eq!(
            signing.reason.key,
            TransferNeedReason::DepthCover.as_signing_reason_key()
        );
        assert!(
            signing.terms.is_some(),
            "negotiated wage / length / role must travel to execution"
        );
        assert!(
            country.transfer_market.transfer_history.is_empty(),
            "the resolver must not write history — the deferred executor owns that row"
        );
        assert_eq!(
            country.transfer_market.negotiations[&neg_id].status,
            NegotiationStatus::Accepted
        );
        assert_eq!(
            country.transfer_market.negotiations[&competing_id].status,
            NegotiationStatus::Rejected,
            "competing pool negotiations must be cancelled on completion"
        );
        assert!(
            country
                .transfer_market
                .listings
                .iter()
                .filter(|l| l.player_id == 9000)
                .all(|l| l.status == TransferListingStatus::Completed),
            "pool player's listings must be marked completed on medical success"
        );
        let request = country.clubs[0]
            .transfer_plan
            .transfer_requests
            .iter()
            .find(|r| r.reason == TransferNeedReason::DepthCover)
            .unwrap();
        assert_eq!(
            request.status,
            TransferRequestStatus::Fulfilled,
            "request is fulfilled only once the negotiation actually completes"
        );
        return;
    }
    panic!("medical collapsed on every seed — the completion path is broken, not unlucky");
}

#[test]
fn unmarked_depth_cover_request_keeps_legacy_instant_signing() {
    RandomEngine::set_seed(0x1E6A_C001);
    // A DepthCover request from the weekly evaluation (no
    // EmergencyFreeAgentDepth marker) must keep the legacy instant
    // free-agent path — the staged-negotiation flow is reserved
    // for emergency-planner depth requests.
    let main = EmergencyFillFixtures::team(10, "FC", "fc", DepthPipelineFixtures::balanced_squad());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let mut country = EmergencyFillFixtures::country(vec![club]);
    country.clubs[0].transfer_plan.initialized = true;
    country.clubs[0]
        .transfer_plan
        .transfer_requests
        .push(TransferRequest::new(
            1,
            PlayerPositionType::MidfielderCenter,
            TransferNeedPriority::Optional,
            TransferNeedReason::DepthCover,
            60,
            80,
            0.0,
        ));

    let pool = vec![DepthPipelineFixtures::pool_summary(
        9300,
        80,
        28,
        PlayerFieldPositionGroup::Midfielder,
        true,
        1.0,
        3500,
    )];

    let date = EmergencyFillFixtures::d(2026, 6, 10);
    let config = TransferConfig::default();
    let mut signings = Vec::new();
    for _ in 0..400 {
        let mut summary = TransferActivitySummary::new();
        let mut domestic = Vec::new();
        let mut offered = Vec::new();
        let mut rejected = Vec::new();
        let mut blocked = Vec::new();
        signings = CountryResult::handle_free_agents(
            &mut country,
            date,
            &mut summary,
            &pool,
            &MarketMap::default(),
            &config,
            &mut domestic,
            &mut offered,
            &mut rejected,
            &mut blocked,
        );
        if !signings.is_empty() {
            break;
        }
    }

    assert!(
        !signings.is_empty(),
        "unmarked DepthCover must still instant-sign within 400 ticks"
    );
    assert_eq!(signings[0].player_id, 9300);
    assert!(
        country.transfer_market.negotiations.is_empty(),
        "normal evaluated DepthCover requests must not enter the staged-negotiation flow"
    );
}

#[test]
fn pool_executor_writes_single_history_row_on_success() {
    let date = EmergencyFillFixtures::d(2026, 6, 10);
    let mut pool_player = EmergencyFillFixtures::player(9400, PlayerPositionType::MidfielderCenter);
    pool_player.ensure_free_agent_state(date, 4000);
    let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);
    let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
    let mut data = SimulatorData::new(
        date.and_hms_opt(12, 0, 0).unwrap(),
        vec![continent],
        GlobalCompetitions::new(Vec::new()),
    );
    data.free_agents.push(pool_player);

    let signing = GlobalFreeAgentSigning {
        player_id: 9400,
        player_name: "Pool P9400".to_string(),
        buying_country_id: 1,
        buying_club_id: 100,
        reason: TransferReason::key(TransferNeedReason::DepthCover.as_signing_reason_key()),
        terms: None,
    };
    let executed =
        GlobalFreeAgentPool::execute_signing(&mut data, &signing, date, &TransferConfig::default());

    assert!(executed, "unclaimed pool player must be signable");
    assert!(data.free_agents.is_empty(), "player leaves the pool");
    let country = data.country(1).unwrap();
    let rows: Vec<_> = country
        .transfer_market
        .transfer_history
        .iter()
        .filter(|t| t.player_id == 9400)
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "the executor is the single writer of the history row"
    );
    assert_eq!(
        rows[0].reason.key,
        TransferNeedReason::DepthCover.as_signing_reason_key()
    );
    assert!(
        country.clubs[0]
            .teams
            .teams
            .iter()
            .any(|t| t.players.players.iter().any(|p| p.id == 9400)),
        "player must land on the buying club's roster"
    );
}

#[test]
fn pool_executor_writes_no_history_when_player_already_claimed() {
    let date = EmergencyFillFixtures::d(2026, 6, 10);
    let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);
    let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
    let mut data = SimulatorData::new(
        date.and_hms_opt(12, 0, 0).unwrap(),
        vec![continent],
        GlobalCompetitions::new(Vec::new()),
    );
    // Pool is empty — another country claimed the player earlier in
    // the same tick. The executor must fail silently.
    let signing = GlobalFreeAgentSigning {
        player_id: 9400,
        player_name: "Pool P9400".to_string(),
        buying_country_id: 1,
        buying_club_id: 100,
        reason: TransferReason::key(TransferNeedReason::DepthCover.as_signing_reason_key()),
        terms: None,
    };
    let executed =
        GlobalFreeAgentPool::execute_signing(&mut data, &signing, date, &TransferConfig::default());

    assert!(!executed, "claimed player cannot be signed twice");
    assert!(
        data.country(1)
            .unwrap()
            .transfer_market
            .transfer_history
            .is_empty(),
        "no phantom history row may be written for a claimed player"
    );
}

/// Fixtures for the fallback-matcher and market-clearing tests.
/// Wrapped on a unit struct per the project convention.
struct MarketClearingFixtures;

impl MarketClearingFixtures {
    /// Buyer country: one balanced-squad club (no emergency need),
    /// transfer plan initialized with a single open SquadPadding
    /// request for a defender.
    fn country_with_defender_request() -> Country {
        let main =
            EmergencyFillFixtures::team(10, "FC", "fc", DepthPipelineFixtures::balanced_squad());
        let mut club = EmergencyFillFixtures::club(100, "FC", main);
        club.transfer_plan.initialized = true;
        club.transfer_plan
            .transfer_requests
            .push(TransferRequest::new(
                1,
                PlayerPositionType::DefenderCenter,
                TransferNeedPriority::Critical,
                TransferNeedReason::SquadPadding,
                50,
                80,
                0.0,
            ));
        EmergencyFillFixtures::country(vec![club])
    }

    /// Long-tail global-pool candidate for the clearing pass:
    /// domestic journeyman deep in the decay curve.
    fn long_term_candidate(player_id: u32) -> FreeAgentCandidate {
        let mut c = EmergencyFillFixtures::candidate(
            player_id,
            70,
            29,
            PlayerFieldPositionGroup::Defender,
            true,
        );
        c.career_pressure = 0.95;
        c.days_free = 400;
        c
    }

    fn run_clearing(
        country: &Country,
        candidates: &[FreeAgentCandidate],
        config: &TransferConfig,
    ) -> Vec<FreeAgentSigning> {
        let mut signings = Vec::new();
        let mut offered = Vec::new();
        let mut rejected = Vec::new();
        let mut recorder = BlockReasonRecorder::new();
        // Non-peak date (March) so the club-scaled / peak-window cap
        // adjustments stay at the base values these tests assert on.
        let date = EmergencyFillFixtures::d(2026, 3, 10);
        CountryResult::handle_free_agents_market_clearing_pass(
            country,
            candidates,
            config,
            date,
            &FreeAgentMarketVisibility::build(0, &MarketMap::default(), &[]),
            &HashSet::new(),
            &mut signings,
            &mut offered,
            &mut rejected,
            &mut recorder,
            &MarketMap::default(),
        );
        signings
    }

    /// Buyer country with one club deliberately thin in defenders (a
    /// single CB) and NO transfer plan / open requests. The high
    /// position-depth need makes the soft tier's opportunistic fit
    /// gate reliably pass for a domestic defender, so the soft-tier
    /// tests exercise the early domestic clearing layer rather than
    /// the hard backstop.
    fn country_thin_in_defenders() -> Country {
        let mut players: Vec<Player> = Vec::new();
        for i in 0..2 {
            players.push(EmergencyFillFixtures::player(
                i,
                PlayerPositionType::Goalkeeper,
            ));
        }
        players.push(EmergencyFillFixtures::player(
            10,
            PlayerPositionType::DefenderCenter,
        ));
        for i in 0..7 {
            players.push(EmergencyFillFixtures::player(
                20 + i,
                PlayerPositionType::MidfielderCenter,
            ));
        }
        for i in 0..4 {
            players.push(EmergencyFillFixtures::player(
                30 + i,
                PlayerPositionType::Striker,
            ));
        }
        let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
        let club = EmergencyFillFixtures::club(100, "FC", main);
        EmergencyFillFixtures::country(vec![club])
    }
}

#[test]
fn request_matcher_tries_fallback_candidates_past_rejecting_top_quality() {
    // Two pool defenders against one open request. Candidate A is
    // the raw-quality pick (CA 95) but practically unsignable: no
    // career pressure and a reservation wage anchored to a 50M
    // previous salary, so nearly every offer is declined. B is a
    // pressured domestic journeyman who accepts realistic terms.
    //
    // The legacy matcher (single best by raw quality) would offer
    // ONLY A, tick after tick, and B would never sign. The
    // fallback matcher must (a) eventually sign B and (b) offer
    // BOTH candidates across trials — proof that a failed roll
    // moves on to the next-ranked candidate instead of abandoning
    // the request.
    let date = EmergencyFillFixtures::d(2026, 6, 10);
    let config = TransferConfig::default();

    let mut star = DepthPipelineFixtures::pool_summary(
        8100,
        95,
        27,
        PlayerFieldPositionGroup::Defender,
        false,
        0.0,
        6200,
    );
    star.last_salary = 50_000_000;
    let journeyman = DepthPipelineFixtures::pool_summary(
        8101,
        75,
        28,
        PlayerFieldPositionGroup::Defender,
        true,
        0.7,
        3500,
    );
    let pool = vec![star, journeyman];

    let mut star_offered = false;
    let mut journeyman_offered = false;
    let mut journeyman_signed = false;
    for _ in 0..400 {
        // Fresh country per trial — a successful signing fulfills
        // the request and would otherwise stop the matcher.
        let mut country = MarketClearingFixtures::country_with_defender_request();
        let mut summary = TransferActivitySummary::new();
        let mut domestic = Vec::new();
        let mut offered = Vec::new();
        let mut rejected = Vec::new();
        let mut blocked = Vec::new();
        let signings = CountryResult::handle_free_agents(
            &mut country,
            date,
            &mut summary,
            &pool,
            &MarketMap::default(),
            &config,
            &mut domestic,
            &mut offered,
            &mut rejected,
            &mut blocked,
        );
        star_offered |= offered.contains(&8100);
        journeyman_offered |= offered.contains(&8101);
        journeyman_signed |= signings.iter().any(|s| s.player_id == 8101);
        if star_offered && journeyman_offered && journeyman_signed {
            break;
        }
    }

    assert!(
        journeyman_signed,
        "fallback matcher must eventually sign the realistic journeyman"
    );
    assert!(
        star_offered && journeyman_offered,
        "both candidates must field offers across trials (star={star_offered}, \
         journeyman={journeyman_offered}) — single-candidate matching starves the pool"
    );
}

#[test]
fn market_clearing_signs_long_term_domestic_free_agent_without_request() {
    // No transfer plan, no requests, no emergency need — the only
    // path that can sign this 400-days-free domestic journeyman is
    // the market-clearing pass.
    let main = EmergencyFillFixtures::team(10, "FC", "fc", DepthPipelineFixtures::balanced_squad());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);
    let config = TransferConfig::default();

    let mut signed = None;
    for _ in 0..400 {
        let candidates = vec![MarketClearingFixtures::long_term_candidate(8200)];
        let signings = MarketClearingFixtures::run_clearing(&country, &candidates, &config);
        if let Some(s) = signings.into_iter().next() {
            signed = Some(s);
            break;
        }
    }

    let signing = signed
        .expect("market clearing must pick up a long-term domestic free agent within 400 ticks");
    assert_eq!(signing.player_id, 8200);
    assert_eq!(signing.to_club_id, 100);
    assert_eq!(signing.reason.key, "free_agent_market_clearing");
    assert!(
        signing.fills_group.is_none(),
        "clearing services no request — request bookkeeping must stay untouched"
    );
    let terms = signing
        .terms
        .expect("clearing must stage explicit short-deal terms");
    assert!(terms.annual_wage > 0);
    assert!(
        matches!(terms.role, BuyerRoleFit::Backup | BuyerRoleFit::Emergency),
        "clearing offers are squad-role deals, got {:?}",
        terms.role
    );
}

#[test]
fn market_clearing_skips_players_below_both_thresholds() {
    // cp 0.3 and 60 days free: under BOTH soft (0.40 / 75d) and hard
    // (0.75 / 365d) eligibility floors — the pass must never touch
    // them, no matter how many ticks. (A 100-day / 0.5-cp player is
    // now deliberately soft-eligible — see the soft-clearing tests.)
    let main = EmergencyFillFixtures::team(10, "FC", "fc", DepthPipelineFixtures::balanced_squad());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);
    let config = TransferConfig::default();

    for _ in 0..50 {
        let mut c = EmergencyFillFixtures::candidate(
            8300,
            70,
            29,
            PlayerFieldPositionGroup::Defender,
            true,
        );
        c.career_pressure = 0.3;
        c.days_free = 60;
        let signings = MarketClearingFixtures::run_clearing(&country, &[c], &config);
        assert!(
            signings.is_empty(),
            "players below both soft and hard floors are not market-clearing eligible"
        );
    }
}

#[test]
fn market_clearing_per_day_cap_prevents_mass_pool_draining() {
    // Thirty fully-desperate candidates against one open club:
    // every tick must stay at or under the COMBINED per-country
    // clearing cap (soft + hard), even though far more would pass
    // the gates. This is the realism backstop against draining the
    // pool in a single week.
    let main = EmergencyFillFixtures::team(10, "FC", "fc", DepthPipelineFixtures::balanced_squad());
    let club = EmergencyFillFixtures::club(100, "FC", main);
    let country = EmergencyFillFixtures::country(vec![club]);
    let config = TransferConfig::default();
    // Both tiers can fire in one tick for a fully-desperate domestic
    // cohort (soft 1 + hard 2 = 3).
    let combined_cap = config.soft_market_clearing_max_signings_per_country_per_day
        + config.market_clearing_max_signings_per_country_per_day;

    let mut any_signed = false;
    for _ in 0..100 {
        let candidates: Vec<FreeAgentCandidate> = (0..30)
            .map(|i| {
                let mut c = MarketClearingFixtures::long_term_candidate(8400 + i);
                c.career_pressure = 1.0;
                c
            })
            .collect();
        let signings = MarketClearingFixtures::run_clearing(&country, &candidates, &config);
        assert!(
            signings.len() <= combined_cap,
            "clearing must respect the combined soft+hard per-day cap ({combined_cap}), \
             got {} signings",
            signings.len()
        );
        any_signed |= !signings.is_empty();
    }
    assert!(
        any_signed,
        "with 30 desperate candidates over 100 ticks, clearing must sign someone"
    );
}

#[test]
fn soft_market_clearing_signs_100_day_domestic_backup() {
    // A ~100-day-free domestic backup-level defender (cp 0.55) is
    // BELOW both hard thresholds (0.75 pressure / 365 days), so any
    // clearing signing here can only come from the new SOFT tier —
    // the early, domestic, opportunistic layer. Acceptance criterion
    // #1: a fringe domestic free agent resolves in months, not years.
    let country = MarketClearingFixtures::country_thin_in_defenders();
    let config = TransferConfig::default();

    let mut signed = None;
    for _ in 0..400 {
        let mut c = EmergencyFillFixtures::candidate(
            8500,
            78,
            29,
            PlayerFieldPositionGroup::Defender,
            true,
        );
        c.career_pressure = 0.55;
        c.days_free = 100;
        let signings = MarketClearingFixtures::run_clearing(&country, &[c], &config);
        if let Some(s) = signings.into_iter().next() {
            signed = Some(s);
            break;
        }
    }

    let signing =
        signed.expect("soft clearing must sign a 100-day domestic backup within 400 ticks");
    assert_eq!(signing.player_id, 8500);
    assert_eq!(signing.reason.key, "free_agent_market_clearing");
    let terms = signing
        .terms
        .expect("soft clearing stages short-deal terms");
    // Stage-aware contract: a Flexible-stage player gets a short
    // 1-2 year deal, never a long commitment.
    assert!(
        terms.contract_years <= 2,
        "soft-stage clearing deals stay short, got {}y",
        terms.contract_years
    );
    assert!(matches!(
        terms.role,
        BuyerRoleFit::Backup | BuyerRoleFit::Emergency
    ));
}

#[test]
fn opportunistic_clearing_signs_free_agent_with_no_matching_request() {
    // The club has NO open transfer request for a defender (empty
    // transfer plan), yet a domestic soft-eligible journeyman is
    // signed anyway through the opportunistic depth logic. Acceptance
    // criterion #6 / spec test #6: NoMatchingRequest free agents are
    // still reachable.
    let country = MarketClearingFixtures::country_thin_in_defenders();
    assert!(
        country.clubs[0].transfer_plan.transfer_requests.is_empty(),
        "fixture must have no open requests for this test to be meaningful"
    );
    let config = TransferConfig::default();

    let mut signed = false;
    for _ in 0..400 {
        let mut c = EmergencyFillFixtures::candidate(
            8550,
            78,
            30,
            PlayerFieldPositionGroup::Defender,
            true,
        );
        // Soft-eligible by pressure, still below the hard floor.
        c.career_pressure = 0.6;
        c.days_free = 120;
        let signings = MarketClearingFixtures::run_clearing(&country, &[c], &config);
        if signings.iter().any(|s| s.player_id == 8550) {
            signed = true;
            break;
        }
    }
    assert!(
        signed,
        "opportunistic soft clearing must sign a useful domestic FA with no open request"
    );
}

/// Spec test #2: a USEFUL domestic player whose contract recently
/// expired (≈11-12 weeks free) is signed through opportunistic
/// clearing WITHOUT any club holding an explicit transfer request for
/// him. At 80 days free he sits below the legacy 90-day soft floor —
/// the lowered floor (75d / 0.40cp) is exactly what lets a local club
/// take a punt on him a few weeks sooner.
#[test]
fn useful_domestic_expired_player_cleared_opportunistically_under_lowered_floor() {
    let country = MarketClearingFixtures::country_thin_in_defenders();
    assert!(
        country.clubs[0].transfer_plan.transfer_requests.is_empty(),
        "fixture must have no open requests — this is the opportunistic, no-request path"
    );
    // 80 days free is below the legacy 90-day soft-clearing floor; it
    // is only reachable because the floor was lowered to 75 days.
    assert!(
        80 >= TransferConfig::default().soft_market_clearing_min_days_free,
        "80 days must clear the (lowered) soft days-free floor"
    );
    let config = TransferConfig::default();

    let mut signing = None;
    for _ in 0..800 {
        let mut c = EmergencyFillFixtures::candidate(
            8560,
            80,
            29,
            PlayerFieldPositionGroup::Defender,
            true,
        );
        // Domestic, useful, recently expired: eligible via the days
        // floor; the modest pressure keeps the opportunistic fit over
        // its Open-stage threshold for a thin-in-defenders club.
        c.career_pressure = 0.5;
        c.days_free = 80;
        let signings = MarketClearingFixtures::run_clearing(&country, &[c], &config);
        if let Some(s) = signings.into_iter().find(|s| s.player_id == 8560) {
            signing = Some(s);
            break;
        }
    }

    let signing = signing
        .expect("a useful domestic expired player must clear opportunistically within 800 ticks");
    assert_eq!(signing.reason.key, "free_agent_market_clearing");
    // No request was serviced — the opportunistic path leaves request
    // bookkeeping untouched.
    assert!(signing.fills_group.is_none());
    let terms = signing.terms.expect("clearing stages short-deal terms");
    assert!(matches!(
        terms.role,
        BuyerRoleFit::Backup | BuyerRoleFit::Emergency
    ));
}

#[test]
fn soft_clearing_ignores_cross_continent_candidates() {
    // The soft tier is the LOCAL market outlet: a cross-continent
    // foreigner at the same soft-eligible stage must NOT be swept by
    // it (he is the hard tier's job, and only once far more
    // pressured). Below the hard floor he stays unsigned.
    let country = MarketClearingFixtures::country_thin_in_defenders();
    let config = TransferConfig::default();

    for _ in 0..200 {
        let mut c = EmergencyFillFixtures::candidate(
            8560,
            78,
            29,
            PlayerFieldPositionGroup::Defender,
            false,
        );
        // Foreign AND cross-continent (different continent id).
        c.nationality_continent_id = 7;
        c.nationality_region = ScoutingRegion::from_country(7, "br");
        c.career_pressure = 0.6;
        c.days_free = 120;
        let signings = MarketClearingFixtures::run_clearing(&country, &[c], &config);
        assert!(
            signings.is_empty(),
            "soft clearing must not reach a cross-continent foreigner below the hard floor"
        );
    }
}

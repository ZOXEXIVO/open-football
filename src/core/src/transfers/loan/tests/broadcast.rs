//! Moved verbatim out of `loan_market.rs` — see that file's `mod transfer_broadcast_tests`.

//! The permanent-transfer mirror of the loan push: a transfer-listed
//! player unsold past the broadcast threshold asks the club to find
//! him a new team, and the scouts offer him around — a same-tier club
//! with room and budget responds by opening a normal (non-loan)
//! purchase negotiation.
use super::super::*;
use crate::academy::ClubAcademy;
use crate::club::player::core::builder::PlayerBuilder;
use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
use crate::shared::Location;
use crate::shared::fullname::FullName;
use crate::transfers::loan::LoanPipeline;
use crate::transfers::market::TransferListing;
use crate::transfers::squad::bands::TierBands;
use crate::{
    Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, PlayerAttributes,
    PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
    PlayerSkills, PlayerSquadStatus, StaffCollection, TeamBuilder, TeamCollection, TeamReputation,
    TeamType, TrainingSchedule,
};
use chrono::{Duration, NaiveTime};

struct Fx;

impl Fx {
    fn monday() -> NaiveDate {
        let d = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        assert_eq!(d.weekday(), Weekday::Mon, "fixture date must be a Monday");
        d
    }

    fn schedule() -> TrainingSchedule {
        TrainingSchedule::new(
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        )
    }

    /// CA of an at-tier midfield starter for a `world`-rep club — the
    /// listed player is built exactly at the buyer's baseline so the
    /// tier-window gate is satisfied by construction.
    fn at_tier_ca(world: u16) -> u8 {
        let score = TeamReputation::new(world, world, world).overall_score();
        TierBands::tier_starter_ca_score(score, PlayerFieldPositionGroup::Midfielder)
    }

    fn player(id: u32, position: PlayerPositionType, ca: u8, age: u8) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ca;
        attrs.potential_ability = ca;
        attrs.condition = 10_000;
        let mut contract =
            PlayerClubContract::new(50_000, NaiveDate::from_ymd_opt(2030, 6, 30).unwrap());
        contract.squad_status = PlayerSquadStatus::MainBackupPlayer;
        contract.is_transfer_listed = true;
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("T".into(), format!("P{id}")))
            .birth_date(NaiveDate::from_ymd_opt(2026 - age as i32, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .contract(Some(contract))
            .build()
            .unwrap()
    }

    fn team(id: u32, club_id: u32, world: u16, players: Vec<Player>) -> Team {
        TeamBuilder::new()
            .id(id)
            .league_id(Some(1))
            .club_id(club_id)
            .name(format!("t{id}"))
            .slug(format!("t{id}"))
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(world, world, world))
            .training_schedule(Self::schedule())
            .build()
            .unwrap()
    }

    fn club(id: u32, teams: Vec<Team>, budget: f64) -> Club {
        let mut club = Club::new(
            id,
            format!("Club{id}"),
            Location::new(1),
            ClubFinances::new(10_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(teams),
            ClubFacilities::default(),
        );
        club.transfer_plan.initialized = true;
        club.transfer_plan.total_budget = budget;
        club
    }

    fn country(clubs: Vec<Club>) -> Country {
        let league = League::new(
            1,
            "L".into(),
            "l".into(),
            1,
            500,
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
        );
        Country::builder()
            .id(1)
            .code("EN".into())
            .slug("en".into())
            .name("England".into())
            .continent_id(1)
            .leagues(LeagueCollection::new(vec![league]))
            .clubs(clubs)
            .build()
            .unwrap()
    }

    /// One country: seller club 1 holding listed midfielder 400 (listed
    /// `listed_days_ago` days back), buyer club 2 at the same tier with
    /// an empty midfield and a funded plan.
    fn market(listed_days_ago: i64, origin: TransferListingOrigin) -> Country {
        const WORLD: u16 = 5_000;
        let ca = Self::at_tier_ca(WORLD);
        let seller_main = Fx::team(
            10,
            1,
            WORLD,
            vec![Fx::player(
                400,
                PlayerPositionType::MidfielderCenter,
                ca,
                26,
            )],
        );
        let seller = Fx::club(1, vec![seller_main], 1_000_000.0);

        // Buyer roster is keepers-only, so the midfield line has room.
        let buyer_main = Fx::team(
            20,
            2,
            WORLD,
            vec![Fx::player(401, PlayerPositionType::Goalkeeper, ca, 27)],
        );
        let buyer = Fx::club(2, vec![buyer_main], 5_000_000.0);

        let mut country = Fx::country(vec![seller, buyer]);
        country
            .transfer_market
            .add_listing(TransferListing::new_with_origin(
                400,
                1,
                10,
                CurrencyValue {
                    amount: 1_000_000.0,
                    currency: Currency::Usd,
                },
                Fx::monday() - Duration::days(listed_days_ago),
                TransferListingType::Transfer,
                origin,
            ));
        country
    }

    fn listed_player(country: &Country) -> &Player {
        country.clubs[0].teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 400)
            .unwrap()
    }
}

#[test]
fn stale_listing_is_broadcast_and_a_buyer_responds() {
    let date = Fx::monday();
    let mut country = Fx::market(100, TransferListingOrigin::SellerListed);

    LoanPipeline::broadcast_listed_transfers(&mut country, date);

    // The push opened a normal purchase negotiation at the buyer.
    assert!(
        country.transfer_market.has_active_negotiation_for(400, 2),
        "a same-tier club with room and budget must respond to the push"
    );
    let negotiation = country
        .transfer_market
        .negotiations
        .values()
        .find(|n| n.player_id == 400)
        .unwrap();
    assert!(!negotiation.is_loan, "the push opens a PERMANENT deal");

    // The cascade state lives on the seller's plan.
    assert!(
        country.clubs[0]
            .transfer_plan
            .transfer_broadcasts
            .contains_key(&400),
        "the seller must be running a transfer broadcast for the player"
    );

    // The player's ask is on his feed.
    assert!(
        Fx::listed_player(&country)
            .happiness
            .recent_events
            .iter()
            .any(|e| e.event_type == HappinessEventType::AskedClubToArrangeTransfer),
        "three months unsold — the player asks the club to find him a new team"
    );
}

#[test]
fn broadcast_anchor_derives_from_listing_age() {
    let date = Fx::monday();
    let mut country = Fx::market(100, TransferListingOrigin::SellerListed);

    LoanPipeline::broadcast_listed_transfers(&mut country, date);

    // The cascade clock starts when the LISTING cleared its grace, not
    // when the push first visited it — a save loaded with an old unsold
    // listing resumes at the depth its age has earned.
    let broadcast = country.clubs[0]
        .transfer_plan
        .transfer_broadcasts
        .get(&400)
        .expect("broadcast state must exist");
    assert_eq!(
        broadcast.since,
        date - Duration::days(100 - 21),
        "anchor = listed_date + grace, independent of when the push first ran"
    );
}

#[test]
fn stale_listing_reaches_lower_tier_buyer_with_relaxed_ceiling() {
    let date = Fx::monday();
    const SELLER_WORLD: u16 = 5_000;
    const BUYER_WORLD: u16 = 2_000;
    let ca = Fx::at_tier_ca(SELLER_WORLD);

    // Premise: the seller-tier starter really is above the lower-tier
    // buyer's normal target ceiling (otherwise this exercises nothing),
    // and within it once the full staleness relaxation (+35) applies.
    let buyer_score = TeamReputation::new(BUYER_WORLD, BUYER_WORLD, BUYER_WORLD).overall_score();
    let buyer_ceiling =
        TierBands::tier_target_ceiling_score(buyer_score, PlayerFieldPositionGroup::Midfielder);
    assert!(
        ca > buyer_ceiling,
        "fixture premise: at-tier CA {ca} must exceed the lower-tier ceiling {buyer_ceiling}"
    );
    assert!(
        ca <= buyer_ceiling.saturating_add(35),
        "fixture premise: the relaxed ceiling must reach the player ({ca} vs {buyer_ceiling}+35)"
    );

    let build = |listed_days_ago: i64| {
        let seller_main = Fx::team(
            10,
            1,
            SELLER_WORLD,
            vec![Fx::player(
                400,
                PlayerPositionType::MidfielderCenter,
                ca,
                26,
            )],
        );
        let seller = Fx::club(1, vec![seller_main], 1_000_000.0);
        let buyer_main = Fx::team(
            20,
            2,
            BUYER_WORLD,
            vec![Fx::player(401, PlayerPositionType::Goalkeeper, ca, 27)],
        );
        let buyer = Fx::club(2, vec![buyer_main], 5_000_000.0);
        let mut country = Fx::country(vec![seller, buyer]);
        country
            .transfer_market
            .add_listing(TransferListing::new_with_origin(
                400,
                1,
                10,
                CurrencyValue {
                    amount: 1_000_000.0,
                    currency: Currency::Usd,
                },
                Fx::monday() - Duration::days(listed_days_ago),
                TransferListingType::Transfer,
                TransferListingOrigin::SellerListed,
            ));
        country
    };

    // A month in: the cascade still sits near the seller's own tier and
    // the ceiling has barely relaxed — the lower-tier club is not yet a
    // taker for a player this good.
    let mut early = build(30);
    LoanPipeline::broadcast_listed_transfers(&mut early, date);
    assert!(
        !early.transfer_market.has_active_negotiation_for(400, 2),
        "a month-old listing must not yet reach a much lower tier"
    );

    // Half a season unsold: the cumulative tier band has walked down to
    // (and past) the buyer's level and the staleness-relaxed ceiling
    // admits the player — the bargain-above-your-level signing.
    let mut stale = build(250);
    LoanPipeline::broadcast_listed_transfers(&mut stale, date);
    assert!(
        stale.transfer_market.has_active_negotiation_for(400, 2),
        "a half-season-stale listing must reach the lower-tier buyer"
    );
}

#[test]
fn fresh_listing_is_not_broadcast() {
    let date = Fx::monday();
    // Within the grace window — the pull-side market still owns it.
    let mut country = Fx::market(14, TransferListingOrigin::SellerListed);

    LoanPipeline::broadcast_listed_transfers(&mut country, date);

    assert!(
        !country.transfer_market.has_active_negotiation_for(400, 2),
        "a fresh listing is not pushed"
    );
    assert!(
        country.clubs[0]
            .transfer_plan
            .transfer_broadcasts
            .is_empty(),
        "no broadcast state for a fresh listing"
    );
    assert!(
        !Fx::listed_player(&country)
            .happiness
            .recent_events
            .iter()
            .any(|e| e.event_type == HappinessEventType::AskedClubToArrangeTransfer),
        "the player has nothing to complain about yet"
    );
}

#[test]
fn synthetic_listing_is_never_broadcast() {
    let date = Fx::monday();
    // A stale SYNTHETIC listing backs someone's unsolicited approach —
    // it does not represent a willingness to sell and must never be
    // pushed around the market.
    let mut country = Fx::market(100, TransferListingOrigin::SyntheticUnsolicited);

    LoanPipeline::broadcast_listed_transfers(&mut country, date);

    assert!(
        !country.transfer_market.has_active_negotiation_for(400, 2),
        "synthetic listings never enter the seller push"
    );
    assert!(
        country.clubs[0]
            .transfer_plan
            .transfer_broadcasts
            .is_empty()
    );
}

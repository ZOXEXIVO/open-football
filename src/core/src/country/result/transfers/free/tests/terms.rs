//! A free-agent signing answers the request it fills, never lands surplus,
//! and is offered the role he would actually hold.

use super::super::*;
use crate::club::academy::ClubAcademy;
use crate::club::player::builder::PlayerBuilder;
use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
use crate::shared::Location;
use crate::shared::fullname::FullName;
use crate::transfers::pipeline::{TransferNeedPriority, TransferNeedReason};
use crate::{
    Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
    PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
    PlayerSkills, StaffCollection, Team, TeamCollection, TeamReputation, TeamType,
    TrainingSchedule,
};
use chrono::NaiveTime;

struct TermsFixtures;

impl TermsFixtures {
    const GK: PlayerFieldPositionGroup = PlayerFieldPositionGroup::Goalkeeper;
    const SLACK: u8 = 5;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2031, 7, 1).unwrap()
    }

    fn keeper(id: u32, ca: u8) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("Keeper".to_string(), format!("K{id}")))
            .birth_date(NaiveDate::from_ymd_opt(1998, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Goalkeeper,
                    level: 18,
                }],
            })
            .player_attributes(PlayerAttributes {
                current_ability: ca,
                potential_ability: ca,
                ..Default::default()
            })
            .build()
            .unwrap()
    }

    /// A club whose main squad holds keepers of these abilities.
    fn club(id: u32, keepers: &[u8]) -> Club {
        let players = keepers
            .iter()
            .enumerate()
            .map(|(i, &ca)| Self::keeper(id * 100 + i as u32, ca))
            .collect();
        let main = Team::builder()
            .id(id)
            .league_id(Some(1))
            .club_id(id)
            .name(format!("Club{id}"))
            .slug(format!("club-{id}"))
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(6000, 6000, 6000))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap();
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
                    split_season: false,
                },
                false,
            )]))
            .clubs(clubs)
            .build()
            .unwrap()
    }

    /// A long-unemployed keeper — pressure high enough that the old
    /// pressure-widened band would have admitted him almost anywhere.
    fn candidate(player_id: u32, ability: u8, age: u8) -> FreeAgentCandidate {
        FreeAgentCandidate {
            player_id,
            player_name: format!("Free{player_id}"),
            club_id: 0,
            club_name: "Free Agent".to_string(),
            ability,
            potential: ability,
            age,
            position_group: Self::GK,
            days_to_expiry: 0,
            nationality_country_reputation: 5000,
            nationality_region: ScoutingRegion::from_country(1, "en"),
            nationality_country_code: "en".to_string(),
            nationality_continent_id: 1,
            career_pressure: 0.9,
            days_free: 300,
            reference_reputation: 4000,
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

    fn request(reason: TransferNeedReason, min_ability: u8, min_gain: i16) -> TransferRequest {
        let mut r = TransferRequest::new(
            1,
            PlayerPositionType::Goalkeeper,
            TransferNeedPriority::Important,
            reason,
            min_ability,
            min_ability.saturating_add(5),
            0.0,
        );
        r.min_gain = min_gain;
        r
    }

    /// Run the request matcher's gate for one club, one request and one
    /// candidate.
    fn evaluate(
        club: &Club,
        request: &TransferRequest,
        candidate: &FreeAgentCandidate,
    ) -> Result<BuyerRoleFit, FreeAgentBlockReason> {
        let visibility = FreeAgentMarketVisibility::build(0, &MarketMap::default(), &[]);
        let registration = SquadRegistrationLimits::default();
        let main = club.teams.main().unwrap();
        let ladder = main.squad_ladder();
        let ctx = RequestBuyerContext {
            club_score: main.reputation.overall_score(),
            league_reputation: 5000,
            negotiator_skill: 50,
            country_reputation: 5000,
            continent_id: 1,
            region_prestige: ScoutingRegion::from_country(1, "en").league_prestige(),
            visibility: &visibility,
            foreign_slots: registration.count(club),
            benefactor: 0.0,
            ladder: &ladder,
            fit: SquadFitSnapshot::build(club, Self::GK, Self::today(), registration),
        };
        RequestCandidateGates::evaluate(candidate, &ctx, request, Self::GK, false, Self::SLACK)
    }

    fn promise_of(
        role: BuyerRoleFit,
        candidate: &FreeAgentCandidate,
    ) -> Option<PromisedSquadStatus> {
        FreeAgentOfferPricing::compute(candidate, Self::GK, role, 0.6, 5000, 50, 5000)
            .signed_terms(candidate)
            .to_personal_terms()
            .squad_status_promise
    }
}

#[test]
fn a_desperate_keeper_below_the_incumbent_cannot_fill_an_upgrade_request() {
    let club = TermsFixtures::club(1, &[120, 100]);
    let upgrade = TermsFixtures::request(TransferNeedReason::QualityUpgrade, 124, 4);
    let desperate = TermsFixtures::candidate(9, 118, 27);

    assert_eq!(
        TermsFixtures::evaluate(&club, &upgrade, &desperate),
        Err(FreeAgentBlockReason::BelowMinimumAbility),
        "a free man who does not improve on the incumbent is not an upgrade"
    );
}

#[test]
fn a_request_age_band_binds() {
    let club = TermsFixtures::club(1, &[120, 100]);
    let heir = TermsFixtures::request(TransferNeedReason::SuccessionPlanning, 95, 0);
    let too_old = TermsFixtures::candidate(9, 115, heir.preferred_age_max + 3);

    assert_eq!(
        TermsFixtures::evaluate(&club, &heir, &too_old),
        Err(FreeAgentBlockReason::OutsideAgeBand)
    );
}

#[test]
fn a_fourth_keeper_below_the_third_is_refused() {
    let club = TermsFixtures::club(1, &[140, 130, 120]);
    let cover = TermsFixtures::request(TransferNeedReason::DepthCover, 90, 0);

    assert_eq!(
        TermsFixtures::evaluate(&club, &cover, &TermsFixtures::candidate(9, 110, 27)),
        Err(FreeAgentBlockReason::SurplusOnArrival)
    );
}

#[test]
fn a_keeper_who_outranks_the_third_is_not_blocked_by_fit() {
    let club = TermsFixtures::club(1, &[140, 130, 120]);
    let cover = TermsFixtures::request(TransferNeedReason::DepthCover, 90, 0);

    assert_eq!(
        TermsFixtures::evaluate(&club, &cover, &TermsFixtures::candidate(9, 125, 27)),
        Ok(BuyerRoleFit::Backup),
        "displacing the weakest keeper is ordinary squad upgrading"
    );
}

#[test]
fn a_third_keeper_is_offered_a_backup_role_with_no_promise() {
    let club = TermsFixtures::club(1, &[140, 130]);
    let cover = TermsFixtures::request(TransferNeedReason::DepthCover, 90, 0);
    let third = TermsFixtures::candidate(9, 125, 27);

    let role = TermsFixtures::evaluate(&club, &cover, &third).unwrap();
    assert_eq!(role, BuyerRoleFit::Backup);
    assert_eq!(TermsFixtures::promise_of(role, &third), None);
}

#[test]
fn a_real_upgrade_is_offered_and_promised_a_key_role() {
    let club = TermsFixtures::club(1, &[120, 110]);
    let upgrade = TermsFixtures::request(TransferNeedReason::QualityUpgrade, 124, 4);
    let better = TermsFixtures::candidate(9, 135, 27);

    let role = TermsFixtures::evaluate(&club, &upgrade, &better).unwrap();
    assert_eq!(role, BuyerRoleFit::KeyPlayer);
    assert_eq!(
        TermsFixtures::promise_of(role, &better),
        Some(PromisedSquadStatus::KeyPlayer)
    );
}

#[test]
fn market_clearing_never_lands_a_keeper_behind_three_better_ones() {
    let country = TermsFixtures::country(vec![
        TermsFixtures::club(1, &[140, 130, 120]),
        TermsFixtures::club(2, &[140]),
    ]);
    let buyers = MarketClearingBuyer::rows_for_country(&country, TermsFixtures::today());
    let journeyman = TermsFixtures::candidate(9, 110, 31);

    let stocked = buyers.iter().find(|b| b.club_id == 1).unwrap();
    assert_eq!(stocked.clearing_role(&journeyman), None);

    let short = buyers.iter().find(|b| b.club_id == 2).unwrap();
    assert_eq!(
        short.clearing_role(&journeyman),
        Some(BuyerRoleFit::Backup),
        "a club with one keeper has a backup's role for him, and clearing pitches no more"
    );
}

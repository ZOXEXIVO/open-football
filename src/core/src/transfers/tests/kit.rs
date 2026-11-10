//! Shared fixtures for transfer-system tests.
//!
//! Thirty-nine separate `struct Fx` blocks across the market's test modules
//! were building the same five things — a date, a player, a team, a club, a
//! country with one league — and each rebuilt roughly a hundred and fifty
//! lines of boilerplate to get there. The cost is not the duplication; it is
//! that writing a test for a new gate starts with reconstructing a world,
//! so gates get shipped without one.
//!
//! Everything here is a builder with a working default, so a test states
//! only what it is actually about:
//!
//! ```ignore
//! let barca = TestClub::new(10)
//!     .name("Barcelona")
//!     .reputation(9_000)
//!     .players(vec![TestPlayer::new(1).ability(176).age(19).build()])
//!     .build();
//! let spain = TestCountry::new(1).code("es").league_reputation(9_200)
//!     .clubs(vec![barca])
//!     .build();
//! ```
//!
//! The defaults are deliberately unremarkable — a 25-year-old of ability
//! 100 at a 4 000-reputation club in a 5 000-reputation league — so a test
//! that does not set a dial is not silently testing an extreme. Where a
//! gate turns on a value, the test says that value out loud.
//!
//! Two modules are ported onto it so far — `expiry_renewal_tests` and
//! `scan_loan_market_tests`; the remaining thirty-seven fixtures convert as
//! later steps touch their files.

// A fixture library carries the dials the world HAS, not only the ones
// today's callers happen to set. An unused builder here is a dial waiting
// for its first test, not dead code.
#![allow(dead_code)]

use crate::PathwayStage;
use crate::club::academy::ClubAcademy;
use crate::club::player::builder::PlayerBuilder;
use crate::club::player::language::LanguageProfile;
use crate::club::player::mind::{AnchorView, CareerPlanView};
use crate::club::player::transfer::LevelFloor;
use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
use crate::shared::Location;
use crate::shared::fullname::FullName;
use crate::transfers::ScoutingRegion;
use crate::transfers::gate::{AppraisalConfig, PlayerOfferAppraisal, PlayerStanceBuilder};
use crate::transfers::pipeline::{PlayerSummary, SellerPlausibilityContext};
use crate::transfers::squad::standing::CareerRecordSnapshot;
use crate::transfers::view::player::PlayerSummaryBuilder;
use crate::{
    Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes, Player,
    PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerFieldPositionGroup,
    PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills, PlayerSquadStatus,
    PositionCoverage, StaffCollection, Team, TeamCollection, TeamReputation, TeamType,
    TrainingSchedule,
};
use chrono::{Datelike, NaiveDate, NaiveTime};

/// The calendar the fixtures run on.
pub struct TestDate;

impl TestDate {
    /// A fixed "today" for tests that only need a coherent calendar. Sits
    /// inside the European summer window, which is where most of the
    /// market's gates are open — call [`Self::on`] with a different date
    /// when the window itself is what the test is about.
    const TODAY: (i32, u32, u32) = (2026, 7, 1);

    /// `NaiveDate` without the `unwrap` at every call site.
    pub fn on(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("test fixture date is valid")
    }

    /// The 1 January `age` years before `today` — the birth date a player
    /// of that age would carry.
    pub fn birth_on(today: NaiveDate, age: u8) -> NaiveDate {
        Self::on(today.year() - age as i32, 1, 1)
    }

    /// [`Self::TODAY`] as a date.
    pub fn today() -> NaiveDate {
        Self::on(Self::TODAY.0, Self::TODAY.1, Self::TODAY.2)
    }
}

/// A player, described by the handful of facts the market actually reads.
pub struct TestPlayer {
    id: u32,
    ability: u8,
    potential: u8,
    age: u8,
    position: PlayerPositionType,
    position_level: u8,
    country_id: u32,
    person: PersonAttributes,
    skills: Option<PlayerSkills>,
    condition: Option<i16>,
    contract: Option<PlayerClubContract>,
    squad_status: Option<PlayerSquadStatus>,
    today: NaiveDate,
}

impl TestPlayer {
    pub fn new(id: u32) -> Self {
        TestPlayer {
            id,
            ability: 100,
            potential: 120,
            age: 25,
            position: PlayerPositionType::Striker,
            position_level: 16,
            country_id: 1,
            person: PersonAttributes::default(),
            skills: None,
            condition: None,
            contract: None,
            squad_status: None,
            today: TestDate::today(),
        }
    }

    /// Technical / mental / physical skills. Left at `default()` unless
    /// asked, because most market gates read `current_ability` and not the
    /// skills behind it — call [`Self::skills_match_ability`] where the
    /// code under test actually looks at them (selection, role fit).
    pub fn skills(mut self, skills: PlayerSkills) -> Self {
        self.skills = Some(skills);
        self
    }

    /// Give him the flat skill set his ability implies, so a CA-176 player
    /// does not read as a collection of average attributes.
    pub fn skills_match_ability(mut self) -> Self {
        self.skills = Some(PlayerSkills::flat_for_ability(self.ability));
        self
    }

    /// Match fitness, 0..10000. Anything that asks "is he available" reads
    /// it; a player left at the default is not necessarily fresh.
    pub fn condition(mut self, condition: i16) -> Self {
        self.condition = Some(condition);
        self
    }

    /// Fully fit — the usual intent when a test is not about fitness.
    pub fn fit(self) -> Self {
        self.condition(10_000)
    }

    /// Personality. Ambition and loyalty drive most of the player's own
    /// side of the market — whether he asks to leave, and what he will
    /// listen to.
    pub fn person(mut self, person: PersonAttributes) -> Self {
        self.person = person;
        self
    }

    /// Ambition and loyalty on the 1..20 scale, the two dials the market
    /// reads most; everything else stays at the default.
    pub fn drive(mut self, ambition: f32, loyalty: f32) -> Self {
        self.person.ambition = ambition;
        self.person.loyalty = loyalty;
        self
    }

    /// What the club has told him he is. Written onto the contract, so it
    /// only takes effect alongside one.
    pub fn squad_status(mut self, status: PlayerSquadStatus) -> Self {
        self.squad_status = Some(status);
        self
    }

    /// Current ability, 1..200.
    pub fn ability(mut self, ability: u8) -> Self {
        self.ability = ability;
        if self.potential < ability {
            self.potential = ability;
        }
        self
    }

    /// Hidden potential ability. Never read directly by the market — the
    /// coach's estimate is — but the generators and estimators need it set.
    pub fn potential(mut self, potential: u8) -> Self {
        self.potential = potential;
        self
    }

    /// Age in whole years at [`Self::on`] (default [`TestDate::today`]).
    pub fn age(mut self, age: u8) -> Self {
        self.age = age;
        self
    }

    pub fn position(mut self, position: PlayerPositionType) -> Self {
        self.position = position;
        self
    }

    /// How well he plays the position, 1..20. Drives every "can he cover
    /// this shirt" read.
    pub fn position_level(mut self, level: u8) -> Self {
        self.position_level = level;
        self
    }

    /// Passport. The loan-home pathway, the corridor map and the foreigner
    /// quota all read it — `is_abroad` is a passport test, not a language
    /// one.
    pub fn country_id(mut self, country_id: u32) -> Self {
        self.country_id = country_id;
        self
    }

    pub fn contract(mut self, contract: PlayerClubContract) -> Self {
        self.contract = Some(contract);
        self
    }

    /// A deal on `salary` running to `expiry`.
    pub fn contract_until(self, salary: u32, expiry: NaiveDate) -> Self {
        self.contract(PlayerClubContract::new(salary, expiry))
    }

    /// A scholarship deal — what an academy graduate is on, and what the
    /// development-loan pathway checks for.
    pub fn youth_contract_until(self, salary: u32, expiry: NaiveDate) -> Self {
        self.contract(PlayerClubContract::new_youth(salary, expiry))
    }

    /// The date his age is measured against. Set it when the test's own
    /// "today" is not [`TODAY`], or the player will be the wrong age.
    pub fn on(mut self, today: NaiveDate) -> Self {
        self.today = today;
        self
    }

    pub fn build(self) -> Player {
        let birth = TestDate::birth_on(self.today, self.age);
        let mut attributes = PlayerAttributes {
            current_ability: self.ability,
            potential_ability: self.potential,
            ..Default::default()
        };
        if let Some(condition) = self.condition {
            attributes.condition = condition;
        }

        let contract = self.contract.map(|mut c| {
            if let Some(status) = self.squad_status {
                c.squad_status = status;
            }
            c
        });

        PlayerBuilder::new()
            .id(self.id)
            .full_name(FullName::new("Test".to_string(), format!("P{}", self.id)))
            .birth_date(birth)
            .country_id(self.country_id)
            .attributes(self.person)
            .skills(self.skills.unwrap_or_default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: self.position,
                    level: self.position_level,
                }],
            })
            .player_attributes(attributes)
            .contract(contract)
            .build()
            .expect("test player builds")
    }
}

/// A squad. Defaults to the club's main team in league 1.
pub struct TestTeam {
    id: u32,
    club_id: u32,
    league_id: Option<u32>,
    name: String,
    slug: String,
    team_type: TeamType,
    reputation: u16,
    players: Vec<Player>,
}

impl TestTeam {
    pub fn new(id: u32) -> Self {
        TestTeam {
            id,
            club_id: 100,
            league_id: Some(1),
            name: format!("Team {id}"),
            slug: format!("team-{id}"),
            team_type: TeamType::Main,
            reputation: 4_000,
            players: Vec::new(),
        }
    }

    pub fn club_id(mut self, club_id: u32) -> Self {
        self.club_id = club_id;
        self
    }

    pub fn league_id(mut self, league_id: Option<u32>) -> Self {
        self.league_id = league_id;
        self
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self.slug = name.to_lowercase().replace(' ', "-");
        self
    }

    pub fn team_type(mut self, team_type: TeamType) -> Self {
        self.team_type = team_type;
        self
    }

    /// Sets home / national / world reputation to the same figure — the
    /// shape almost every gate reads. Set the three apart by hand when a
    /// test is about the gap between them.
    pub fn reputation(mut self, reputation: u16) -> Self {
        self.reputation = reputation;
        self
    }

    pub fn players(mut self, players: Vec<Player>) -> Self {
        self.players = players;
        self
    }

    pub fn build(self) -> Team {
        Team::builder()
            .id(self.id)
            .league_id(self.league_id)
            .club_id(self.club_id)
            .name(self.name)
            .slug(self.slug)
            .team_type(self.team_type)
            .players(PlayerCollection::new(self.players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(
                self.reputation,
                self.reputation,
                self.reputation,
            ))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).expect("09:00 is a time"),
                NaiveTime::from_hms_opt(15, 0, 0).expect("15:00 is a time"),
            ))
            .build()
            .expect("test team builds")
    }
}

/// A club with one main team. Give it players and it builds the squad for
/// you; hand it whole teams when the test is about more than the first XI.
pub struct TestClub {
    id: u32,
    name: String,
    balance: i64,
    reputation: u16,
    league_id: Option<u32>,
    location_country_id: u32,
    status: ClubStatus,
    players: Vec<Player>,
    teams: Option<Vec<Team>>,
}

impl TestClub {
    pub fn new(id: u32) -> Self {
        TestClub {
            id,
            name: format!("Club {id}"),
            balance: 1_000_000,
            reputation: 4_000,
            league_id: Some(1),
            location_country_id: 1,
            status: ClubStatus::Professional,
            players: Vec::new(),
            teams: None,
        }
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    /// Cash in the bank. Seller distress, the fee gates and the emergency
    /// pass all read it.
    pub fn balance(mut self, balance: i64) -> Self {
        self.balance = balance;
        self
    }

    /// Main-team reputation. Ignored when [`Self::teams`] supplies the
    /// squads directly.
    pub fn reputation(mut self, reputation: u16) -> Self {
        self.reputation = reputation;
        self
    }

    pub fn league_id(mut self, league_id: Option<u32>) -> Self {
        self.league_id = league_id;
        self
    }

    pub fn location_country_id(mut self, country_id: u32) -> Self {
        self.location_country_id = country_id;
        self
    }

    pub fn status(mut self, status: ClubStatus) -> Self {
        self.status = status;
        self
    }

    /// Players for the generated main team.
    pub fn players(mut self, players: Vec<Player>) -> Self {
        self.players = players;
        self
    }

    /// Full squad list, replacing the generated main team. Use when the
    /// test needs a B side, a youth team, or an unusual `TeamType` order —
    /// the main team is resolved by type, never by position.
    pub fn teams(mut self, teams: Vec<Team>) -> Self {
        self.teams = Some(teams);
        self
    }

    pub fn build(self) -> Club {
        let teams = self.teams.unwrap_or_else(|| {
            vec![
                TestTeam::new(self.id * 10)
                    .club_id(self.id)
                    .league_id(self.league_id)
                    .name(&self.name)
                    .reputation(self.reputation)
                    .players(self.players)
                    .build(),
            ]
        });

        Club::new(
            self.id,
            self.name,
            Location::new(self.location_country_id),
            ClubFinances::new(self.balance, Vec::new()),
            ClubAcademy::new(3),
            self.status,
            ClubColors::default(),
            TeamCollection::new(teams),
            ClubFacilities::default(),
        )
    }
}

/// A country with one top-flight league.
pub struct TestCountry {
    id: u32,
    code: String,
    name: String,
    continent_id: u32,
    reputation: u16,
    league_reputation: u16,
    clubs: Vec<Club>,
}

impl TestCountry {
    pub fn new(id: u32) -> Self {
        TestCountry {
            id,
            code: "en".to_string(),
            name: "England".to_string(),
            continent_id: 1,
            reputation: 5_000,
            league_reputation: 5_000,
            clubs: Vec::new(),
        }
    }

    /// Two-letter code. The transfer calendar, the corridor map and the
    /// language model all key off it, so a test about any of those must set
    /// it to the country it means.
    pub fn code(mut self, code: &str) -> Self {
        self.code = code.to_string();
        self
    }

    pub fn name(mut self, name: &str) -> Self {
        self.name = name.to_string();
        self
    }

    pub fn continent_id(mut self, continent_id: u32) -> Self {
        self.continent_id = continent_id;
        self
    }

    /// Country reputation, 0..10000. Distinct from the league's: the
    /// free-agent realism gates read the country, the plausibility model
    /// reads the league.
    pub fn reputation(mut self, reputation: u16) -> Self {
        self.reputation = reputation;
        self
    }

    /// Reputation of the single league this country carries (id 1).
    pub fn league_reputation(mut self, reputation: u16) -> Self {
        self.league_reputation = reputation;
        self
    }

    pub fn clubs(mut self, clubs: Vec<Club>) -> Self {
        self.clubs = clubs;
        self
    }

    pub fn build(self) -> Country {
        let slug = self.name.to_lowercase().replace(' ', "-");
        Country::builder()
            .id(self.id)
            .code(self.code)
            .slug(slug.clone())
            .name(self.name)
            .continent_id(self.continent_id)
            .reputation(self.reputation)
            .leagues(LeagueCollection::new(vec![League::new(
                1,
                "League".to_string(),
                slug,
                self.id,
                self.league_reputation,
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
            .clubs(self.clubs)
            .build()
            .expect("test country builds")
    }
}

/// A player as the market reads him across a border. The stored readings —
/// his floor, his ask and his refusal price — are derived the way the pool
/// builder derives them unless a test sets them outright.
pub struct TestSummary {
    player_id: u32,
    club_id: u32,
    country_id: u32,
    country_code: String,
    nationality_country_id: u32,
    age: u8,
    ability: u8,
    reputation: i16,
    salary: u32,
    estimated_value: f64,
    contract_months: i16,
    squad_status: PlayerSquadStatus,
    league_reputation: u16,
    club_reputation_score: f32,
    position_group_rank: u8,
    appearances: u16,
    club_matches: u16,
    starter_share: f32,
    is_listed: bool,
    is_loan_listed: bool,
    is_transfer_requested: bool,
    market_resignation: f32,
    level_floor: Option<LevelFloor>,
    asking_wage: Option<f64>,
    refusal_price: Option<f64>,
}

impl TestSummary {
    pub fn new(player_id: u32) -> Self {
        TestSummary {
            player_id,
            club_id: 900,
            country_id: 1,
            country_code: "en".to_string(),
            nationality_country_id: 1,
            age: 25,
            ability: 100,
            reputation: 3_000,
            salary: 500_000,
            estimated_value: 2_000_000.0,
            contract_months: 36,
            squad_status: PlayerSquadStatus::FirstTeamRegular,
            league_reputation: 5_000,
            club_reputation_score: 0.4,
            position_group_rank: 1,
            appearances: 20,
            club_matches: 20,
            starter_share: 0.5,
            is_listed: false,
            is_loan_listed: false,
            is_transfer_requested: false,
            market_resignation: 0.0,
            level_floor: None,
            asking_wage: None,
            refusal_price: None,
        }
    }

    pub fn club_id(mut self, club_id: u32) -> Self {
        self.club_id = club_id;
        self
    }

    /// Where he plays. His passport follows unless set apart.
    pub fn country(mut self, country_id: u32, code: &str) -> Self {
        self.country_id = country_id;
        self.country_code = code.to_string();
        self.nationality_country_id = country_id;
        self
    }

    pub fn nationality(mut self, country_id: u32) -> Self {
        self.nationality_country_id = country_id;
        self
    }

    pub fn age(mut self, age: u8) -> Self {
        self.age = age;
        self
    }

    pub fn ability(mut self, ability: u8) -> Self {
        self.ability = ability;
        self
    }

    pub fn reputation(mut self, reputation: i16) -> Self {
        self.reputation = reputation;
        self
    }

    pub fn salary(mut self, salary: u32) -> Self {
        self.salary = salary;
        self
    }

    pub fn value(mut self, estimated_value: f64) -> Self {
        self.estimated_value = estimated_value;
        self
    }

    pub fn contract_months(mut self, months: i16) -> Self {
        self.contract_months = months;
        self
    }

    pub fn squad_status(mut self, status: PlayerSquadStatus) -> Self {
        self.squad_status = status;
        self
    }

    /// The division he plays in, and how big his club is in it.
    pub fn league(mut self, league_reputation: u16, club_reputation_score: f32) -> Self {
        self.league_reputation = league_reputation;
        self.club_reputation_score = club_reputation_score;
        self
    }

    pub fn rank(mut self, rank: u8) -> Self {
        self.position_group_rank = rank;
        self
    }

    pub fn appearances(mut self, appearances: u16, club_matches: u16) -> Self {
        self.appearances = appearances;
        self.club_matches = club_matches;
        self
    }

    pub fn listed(mut self) -> Self {
        self.is_listed = true;
        self
    }

    pub fn loan_listed(mut self) -> Self {
        self.is_loan_listed = true;
        self
    }

    pub fn requested(mut self) -> Self {
        self.is_transfer_requested = true;
        self
    }

    pub fn resignation(mut self, resignation: f32) -> Self {
        self.market_resignation = resignation;
        self
    }

    pub fn level_floor(mut self, floor: LevelFloor) -> Self {
        self.level_floor = Some(floor);
        self
    }

    pub fn asking_wage(mut self, ask: f64) -> Self {
        self.asking_wage = Some(ask);
        self
    }

    pub fn refusal_price(mut self, price: f64) -> Self {
        self.refusal_price = Some(price);
        self
    }

    pub fn build(self) -> PlayerSummary {
        let region = ScoutingRegion::from_country(1, &self.country_code);
        let mut summary = PlayerSummary {
            player_id: self.player_id,
            club_id: self.club_id,
            country_id: self.country_id,
            continent_id: 1,
            region,
            country_code: self.country_code.clone(),
            nationality_country_id: self.nationality_country_id,
            nationality_continent_id: Some(1),
            nationality_region: Some(region),
            starter_share: self.starter_share,
            tenure_days: 700,
            return_home_desire: 0.0,
            home_return_wanted: false,
            ambition: 12,
            loyalty: 10,
            adaptability: 12,
            leave_pressure: 0.0,
            stay_pressure: 0.0,
            parent_loan: None,
            career_plan: CareerPlanView::none(),
            anchors: AnchorView::default(),
            asks_for_a_bigger_stage: false,
            parent_subsidy: 0.0,
            player_name: format!("Player {}", self.player_id),
            club_name: format!("Club {}", self.club_id),
            position: PlayerPositionType::MidfielderCenter,
            position_group: PlayerFieldPositionGroup::Midfielder,
            coverage: PositionCoverage::single(PlayerPositionType::MidfielderCenter),
            age: self.age,
            estimated_value: self.estimated_value,
            is_listed: self.is_listed,
            is_loan_listed: self.is_loan_listed,
            skill_ability: self.ability,
            observable_level: self.ability,
            pathway_stage: PathwayStage::Rotation,
            average_rating: 6.8,
            goals: 2,
            assists: 2,
            appearances: self.appearances,
            determination: 12.0,
            work_rate: 12.0,
            composure: 12.0,
            anticipation: 12.0,
            technical_avg: 12.0,
            mental_avg: 12.0,
            physical_avg: 12.0,
            current_reputation: self.reputation,
            home_reputation: self.reputation,
            world_reputation: self.reputation,
            country_reputation: 5_000,
            club_world_reputation: (self.club_reputation_score * 10_000.0) as i16,
            club_best_in_group: self.ability,
            is_injured: false,
            contract_months_remaining: self.contract_months,
            salary: self.salary,
            language_profile: LanguageProfile::default(),
            international_apps: 0,
            career_record: CareerRecordSnapshot::default(),
            seller_ctx: SellerPlausibilityContext {
                club_reputation_score: self.club_reputation_score,
                league_reputation: self.league_reputation,
                league_id: Some(1),
                position_group_rank: self.position_group_rank,
                squad_status: self.squad_status,
                is_transfer_requested: self.is_transfer_requested,
                is_unhappy: false,
                in_debt: false,
                days_on_market: 0,
                market_resignation: self.market_resignation,
                club_matches_played: self.club_matches,
                big_stage_inclination: 0.0,
                is_marketed: false,
            },
            level_floor: LevelFloor::none(),
            asking_wage: 0.0,
            refusal_price: 0.0,
        };
        summary.level_floor = self
            .level_floor
            .unwrap_or_else(|| PlayerSummaryBuilder::level_floor(&summary));
        let stance = PlayerStanceBuilder::from_summary(&summary, 0.0, region);
        let cfg = AppraisalConfig::default();
        summary.asking_wage = self
            .asking_wage
            .unwrap_or_else(|| PlayerOfferAppraisal::asking_wage(&stance, &cfg));
        summary.refusal_price = self
            .refusal_price
            .unwrap_or_else(|| PlayerOfferAppraisal::refusal_price(&stance, &cfg));
        summary
    }
}

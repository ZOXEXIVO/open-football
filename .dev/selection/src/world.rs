//! Where every first team and competition lives, and what each tracked club's
//! players were on the morning of a matchday.

use chrono::{Datelike, NaiveDate};
use core::club::staff::perception::AbilityEstimator;
use core::club::staff::{PlannedRole, PlayerPlanEntry};
use core::r#match::MatchResult;
use core::{
    Club, Person, Player, PlayerPositionType, PlayerSquadStatus, PlayerStatusType, SimulatorData,
    TeamType,
};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    League,
    Cup,
    Continental,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::League, Kind::Cup, Kind::Continental];

    pub fn key(self) -> &'static str {
        match self {
            Kind::League => "league",
            Kind::Cup => "cup",
            Kind::Continental => "continental",
        }
    }

    pub fn slot(self) -> usize {
        self as usize
    }
}

/// The five lines of an outfield, as a bench covers them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PitchLine {
    CentralDefence,
    WideDefence,
    CentralMidfield,
    WideAttack,
    Forward,
}

impl PitchLine {
    pub const ALL: [PitchLine; 5] = [
        PitchLine::CentralDefence,
        PitchLine::WideDefence,
        PitchLine::CentralMidfield,
        PitchLine::WideAttack,
        PitchLine::Forward,
    ];

    /// A substitute counts as cover for a line he plays at this level or
    /// better, on the 1..20 familiarity scale.
    const COVER_LEVEL: u8 = 14;

    pub fn key(self) -> &'static str {
        match self {
            PitchLine::CentralDefence => "central_defence",
            PitchLine::WideDefence => "wide_defence",
            PitchLine::CentralMidfield => "central_midfield",
            PitchLine::WideAttack => "wide_attack",
            PitchLine::Forward => "forward",
        }
    }

    pub fn bit(self) -> u8 {
        1 << self as u8
    }

    pub fn of(position: PlayerPositionType) -> Option<PitchLine> {
        use PlayerPositionType::*;
        match position {
            Goalkeeper => None,
            Sweeper | DefenderCenterLeft | DefenderCenter | DefenderCenterRight => {
                Some(PitchLine::CentralDefence)
            }
            DefenderLeft | DefenderRight | WingbackLeft | WingbackRight => {
                Some(PitchLine::WideDefence)
            }
            DefensiveMidfielder | MidfielderCenterLeft | MidfielderCenter
            | MidfielderCenterRight => Some(PitchLine::CentralMidfield),
            MidfielderLeft
            | MidfielderRight
            | AttackingMidfielderLeft
            | AttackingMidfielderCenter
            | AttackingMidfielderRight => Some(PitchLine::WideAttack),
            Striker | ForwardLeft | ForwardCenter | ForwardRight => Some(PitchLine::Forward),
        }
    }

    pub fn covered_by(player: &Player) -> u8 {
        player
            .positions
            .positions
            .iter()
            .filter(|p| p.level >= Self::COVER_LEVEL)
            .filter_map(|p| Self::of(p.position))
            .fold(0, |mask, line| mask | line.bit())
    }
}

#[derive(Clone, Copy)]
pub struct TeamRef {
    pub continent: usize,
    pub country: usize,
    pub club: usize,
    pub tier: u8,
}

/// Rebuilt monthly: promotions and relegations move teams between tiers.
pub struct WorldIndex {
    pub main_teams: HashMap<u32, TeamRef>,
    leagues: HashMap<u32, (Kind, u8)>,
    league_rounds: HashMap<u32, u32>,
    /// Leagues whose season runs inside one calendar year.
    calendar_year: HashMap<u32, bool>,
    team_club: HashMap<u32, u32>,
    /// Each domestic cup's country, as (continent, country).
    cups: HashMap<u32, (usize, usize)>,
}

impl WorldIndex {
    const CONTINENTAL_IDS: std::ops::RangeInclusive<u32> = 900_000_001..=900_000_099;

    pub fn build(data: &SimulatorData) -> Self {
        let mut main_teams = HashMap::new();
        let mut leagues = HashMap::new();
        let mut league_rounds = HashMap::new();
        let mut calendar_year = HashMap::new();
        let mut team_club = HashMap::new();
        let mut cups = HashMap::new();
        for (ci, continent) in data.continents.iter().enumerate() {
            for (ki, country) in continent.countries.iter().enumerate() {
                for league in &country.leagues.leagues {
                    if !league.friendly {
                        leagues.insert(league.id, (Kind::League, league.settings.tier));
                        let teams = league.table.rows.len() as u32;
                        league_rounds.insert(league.id, teams.saturating_sub(1) * 2);
                        calendar_year.insert(
                            league.id,
                            league.settings.season_starting_half.from_month <= 6,
                        );
                    }
                }
                if let Some(cup) = country.domestic_cup.as_ref() {
                    leagues.insert(cup.league.id, (Kind::Cup, 0));
                    cups.insert(cup.league.id, (ci, ki));
                }
                for (bi, club) in country.clubs.iter().enumerate() {
                    for team in &club.teams.teams {
                        team_club.insert(team.id, club.id);
                        if team.team_type != TeamType::Main {
                            continue;
                        }
                        let tier = team
                            .league_id
                            .and_then(|id| leagues.get(&id))
                            .map(|(_, t)| *t)
                            .unwrap_or(9);
                        main_teams.insert(
                            team.id,
                            TeamRef {
                                continent: ci,
                                country: ki,
                                club: bi,
                                tier,
                            },
                        );
                    }
                }
            }
        }
        WorldIndex {
            main_teams,
            leagues,
            league_rounds,
            calendar_year,
            team_club,
            cups,
        }
    }

    /// The year a cup's running edition was drawn in.
    pub fn cup_edition(&self, data: &SimulatorData, league_id: u32) -> Option<i32> {
        let (continent, country) = self.cups.get(&league_id)?;
        data.continents
            .get(*continent)?
            .countries
            .get(*country)?
            .domestic_cup
            .as_ref()
            .map(|cup| cup.season_start_year)
    }

    pub fn club_of_team(&self, team_id: u32) -> Option<u32> {
        self.team_club.get(&team_id).copied()
    }

    /// The year a league's running season started in.
    pub fn season_year(&self, league_id: u32, date: NaiveDate) -> i32 {
        let calendar = self.calendar_year.get(&league_id).copied().unwrap_or(false);
        if calendar || date.month() >= 7 {
            date.year()
        } else {
            date.year() - 1
        }
    }

    pub fn club<'a>(&self, data: &'a SimulatorData, team: &TeamRef) -> Option<&'a Club> {
        data.continents
            .get(team.continent)?
            .countries
            .get(team.country)?
            .clubs
            .get(team.club)
    }

    /// The top two divisions are the census's population.
    pub fn tracked(&self) -> impl Iterator<Item = (&u32, &TeamRef)> {
        self.main_teams.iter().filter(|(_, r)| r.tier <= 2)
    }

    pub fn kind(&self, m: &MatchResult) -> Option<Kind> {
        if m.friendly {
            return None;
        }
        if let Some((kind, _)) = self.leagues.get(&m.league_id) {
            return Some(*kind);
        }
        Self::CONTINENTAL_IDS
            .contains(&m.league_id)
            .then_some(Kind::Continental)
    }

    pub fn league_rounds(&self, league_id: u32) -> Option<u32> {
        self.league_rounds.get(&league_id).copied().filter(|n| *n > 0)
    }
}

/// One player as he stood before the day's matches.
#[derive(Clone, Copy)]
pub struct PlayerFacts {
    pub team_id: u32,
    pub first_team: bool,
    pub age: u8,
    pub born: i32,
    pub keeper: bool,
    /// Not injured, not away with his country, not suspended.
    pub fit: bool,
    pub status: usize,
    pub listed: bool,
    pub borrowed: bool,
    /// Days since his loan here began, for a borrowed player.
    pub borrowed_days: Option<i64>,
    /// The level the staff see in him.
    pub level: u8,
    pub parent_club: Option<u32>,
    pub role: Option<PlannedRole>,
    /// The share of his side's football the plan gives him.
    pub share: f32,
    /// The plan's first choice for a shirt of the formation.
    pub first_choice: bool,
    pub lines: u8,
}

impl PlayerFacts {
    pub const STATUS: [&'static str; 9] = [
        "key",
        "regular",
        "rotation",
        "backup",
        "hot_prospect",
        "youngster",
        "not_needed",
        "not_set",
        "none",
    ];

    pub const ROLES: [PlannedRole; 9] = PlannedRole::LADDER;

    pub fn role_key(role: Option<PlannedRole>) -> &'static str {
        match role {
            Some(PlannedRole::Cornerstone) => "cornerstone",
            Some(PlannedRole::Starter) => "starter",
            Some(PlannedRole::Rotation) => "rotation",
            Some(PlannedRole::CupKeeper) => "cup_keeper",
            Some(PlannedRole::SuccessionHeir) => "heir",
            Some(PlannedRole::DevelopmentPathway) => "development",
            Some(PlannedRole::Cover) => "cover",
            Some(PlannedRole::ShopWindow) => "shop_window",
            Some(PlannedRole::NotInPlans) => "not_in_plans",
            None => "unplanned",
        }
    }

    /// Roles in ladder order, with the unplanned last.
    pub fn role_slot(role: Option<PlannedRole>) -> usize {
        role.map(|r| r.as_u8() as usize).unwrap_or(9)
    }

    pub fn role_of_slot(slot: usize) -> Option<PlannedRole> {
        Self::ROLES.get(slot).copied()
    }

    fn status_slot(player: &Player) -> usize {
        match player.contract.as_ref().map(|c| &c.squad_status) {
            Some(PlayerSquadStatus::KeyPlayer) => 0,
            Some(PlayerSquadStatus::FirstTeamRegular) => 1,
            Some(PlayerSquadStatus::FirstTeamSquadRotation) => 2,
            Some(PlayerSquadStatus::MainBackupPlayer) => 3,
            Some(PlayerSquadStatus::HotProspectForTheFuture) => 4,
            Some(PlayerSquadStatus::DecentYoungster) => 5,
            Some(PlayerSquadStatus::NotNeeded) => 6,
            Some(_) => 7,
            None => 8,
        }
    }

    fn read(
        player: &Player,
        team_id: u32,
        first_team: bool,
        plan: Option<&PlayerPlanEntry>,
        date: NaiveDate,
    ) -> Self {
        let attributes = &player.player_attributes;
        PlayerFacts {
            team_id,
            first_team,
            age: player.age(date),
            born: player.birth_date.year(),
            keeper: player.positions.is_goalkeeper(),
            fit: !attributes.is_injured
                && !attributes.is_banned
                && !player.statuses.is_on_international_duty(),
            status: Self::status_slot(player),
            listed: player.statuses.has(PlayerStatusType::Lst),
            borrowed: player.is_on_loan(),
            borrowed_days: player
                .contract_loan
                .as_ref()
                .and_then(|loan| loan.started)
                .map(|started| (date - started).num_days()),
            level: AbilityEstimator::observable_level(player),
            parent_club: player
                .contract_loan
                .as_ref()
                .and_then(|loan| loan.loan_from_club_id),
            role: plan.map(|entry| entry.role),
            share: plan.map(|entry| entry.share).unwrap_or(0.0),
            first_choice: plan.is_some_and(|entry| entry.place == 0 && entry.shirt.is_some()),
            lines: PitchLine::covered_by(player),
        }
    }
}

pub struct ClubFacts {
    pub club_id: u32,
    pub club_name: String,
    pub tier: u8,
    pub players: HashMap<u32, PlayerFacts>,
    pub declared_number_one: Option<u32>,
    pub declared_deputy: Option<u32>,
    /// Training reads two or more competitive fixtures in the seven days
    /// around this morning.
    pub double_match_week: bool,
}

impl ClubFacts {
    fn read(club: &Club, main_team_id: u32, tier: u8, date: NaiveDate) -> Self {
        let mut players = HashMap::new();
        for team in &club.teams.teams {
            let plan = &team.staffs.head_coach().squad_plan;
            let first_team = team.id == main_team_id;
            for player in team.players.iter() {
                players.insert(
                    player.id,
                    PlayerFacts::read(player, team.id, first_team, plan.entry(player.id), date),
                );
            }
        }
        ClubFacts {
            club_id: club.id,
            club_name: club.name.clone(),
            tier,
            players,
            declared_number_one: club.keeper_plan().and_then(|plan| plan.number_one()),
            declared_deputy: club.keeper_plan().and_then(|plan| plan.deputy()),
            double_match_week: club
                .teams
                .teams
                .iter()
                .find(|t| t.id == main_team_id)
                .is_some_and(|t| t.fixture_window.fixtures_this_week(date) >= 2),
        }
    }

    /// Fit keepers in the club's other squads, and how many of them were
    /// free: their own side idle that day, or left with a fit keeper of its
    /// own.
    pub fn keepers_elsewhere(&self, team_id: u32, playing: &dyn Fn(u32) -> bool) -> (usize, usize) {
        let elsewhere: Vec<&PlayerFacts> = self
            .players
            .values()
            .filter(|p| p.keeper && p.fit && p.team_id != team_id)
            .collect();
        let free = elsewhere
            .iter()
            .filter(|p| {
                !playing(p.team_id)
                    || elsewhere.iter().filter(|q| q.team_id == p.team_id).count() >= 2
            })
            .count();
        (elsewhere.len(), free)
    }
}

/// Every tracked club on one morning, by first-team id.
pub struct DayFacts {
    pub date: NaiveDate,
    pub clubs: HashMap<u32, ClubFacts>,
    /// First-team players aged 21 or under, to their first team.
    pub young_first_team: HashMap<u32, u32>,
    /// Every tracked club's team, to its first team.
    pub team_owner: HashMap<u32, u32>,
}

impl DayFacts {
    pub const YOUNG_AGE: u8 = 21;

    pub fn read(data: &SimulatorData, index: &WorldIndex) -> Self {
        let date = data.date.date();
        let mut clubs = HashMap::new();
        let mut young_first_team = HashMap::new();
        let mut team_owner = HashMap::new();
        for (&team_id, team_ref) in index.tracked() {
            let Some(club) = index.club(data, team_ref) else {
                continue;
            };
            let facts = ClubFacts::read(club, team_id, team_ref.tier, date);
            for (&player_id, player) in &facts.players {
                if player.first_team && player.age <= Self::YOUNG_AGE {
                    young_first_team.insert(player_id, team_id);
                }
            }
            for team in &club.teams.teams {
                team_owner.insert(team.id, team_id);
            }
            clubs.insert(team_id, facts);
        }
        DayFacts {
            date,
            clubs,
            young_first_team,
            team_owner,
        }
    }
}

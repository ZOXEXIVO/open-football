//! One man on one day, as the census reads him, and how big his club is.

use chrono::NaiveDate;
use core::club::player::mind::{CareerArc, GoalKind};
use core::club::player::player::TransferRequestReason;
use core::{Person, Player, PlayerSquadStatus, PlayerStatusType, SimulatorData};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Tier {
    Elite,
    Top,
    Mid,
    Low,
    Pool,
}

impl Tier {
    pub const CLUBS: [Tier; 4] = [Tier::Elite, Tier::Top, Tier::Mid, Tier::Low];

    pub fn of_rank(rank: u32) -> Tier {
        match rank {
            0..=49 => Tier::Elite,
            50..=249 => Tier::Top,
            250..=999 => Tier::Mid,
            u32::MAX => Tier::Pool,
            _ => Tier::Low,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tier::Elite => "elite(top50)",
            Tier::Top => "top(51-250)",
            Tier::Mid => "mid(251-1000)",
            Tier::Low => "low",
            Tier::Pool => "pool",
        }
    }
}

pub struct AgeBand;

impl AgeBand {
    pub const COUNT: u8 = 5;

    pub fn of(age: u8) -> u8 {
        match age {
            0..=20 => 0,
            21..=24 => 1,
            25..=29 => 2,
            30..=32 => 3,
            _ => 4,
        }
    }

    pub fn label(band: u8) -> &'static str {
        match band {
            0 => "<=20",
            1 => "21-24",
            2 => "25-29",
            3 => "30-32",
            _ => "33+",
        }
    }
}

/// Clubs by the reputation of their main team, re-read once a year: a club
/// that rises over a decade should read as big by the end of it.
#[derive(Default)]
pub struct ClubRanks {
    ranks: HashMap<u32, u32>,
}

impl ClubRanks {
    pub fn read(data: &SimulatorData) -> Self {
        let mut clubs: Vec<(u32, f32)> = Vec::new();
        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    if let Some(main) = club.teams.main() {
                        clubs.push((club.id, main.reputation.overall_score()));
                    }
                }
            }
        }
        clubs.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        ClubRanks {
            ranks: clubs
                .iter()
                .enumerate()
                .map(|(rank, (id, _))| (*id, rank as u32))
                .collect(),
        }
    }

    pub fn of(&self, club_id: u32) -> u32 {
        self.ranks.get(&club_id).copied().unwrap_or(u32::MAX)
    }
}

/// Where he is, shared by everyone in one club's team.
#[derive(Clone, Copy)]
pub struct Place<'a> {
    pub club_id: u32,
    pub club_name: &'a str,
    pub league_rep: u16,
    pub club_rep: f32,
    pub rank: u32,
    pub main_team: bool,
}

impl Place<'_> {
    pub const NOWHERE: Place<'static> = Place {
        club_id: 0,
        club_name: "(free agent)",
        league_rep: 0,
        club_rep: 0.0,
        rank: u32::MAX,
        main_team: false,
    };
}

/// Yesterday's reading is held for the whole world, so nothing here allocates.
#[derive(Clone, Copy)]
pub struct Snap {
    pub club_id: u32,
    pub league_rep: u16,
    pub club_rep: f32,
    pub rank: u32,
    pub age: u8,
    pub ca: u8,
    pub rep: i16,
    pub status: &'static str,
    pub regular: bool,
    pub starting: bool,
    pub starter: f32,
    pub on_loan: bool,
    pub days_left: i32,
    pub salary: u32,
    pub listed: bool,
    pub req: bool,
    pub req_reason: Option<TransferRequestReason>,
    pub asks_only_bigger: bool,
    pub top_goal: GoalKind,
    pub top_pressure: f32,
    pub arc: Option<CareerArc>,
}

impl Snap {
    pub fn read(p: &Player, date: NaiveDate, place: &Place) -> Snap {
        let contract = p.contract.as_ref();
        let status = contract.map_or("-", |c| Self::status_label(&c.squad_status));
        let seen = p.happiness.appearances_tracked >= 6;
        let starter = if seen { p.happiness.starter_ratio } else { 0.0 };
        let starting = place.main_team && seen && p.happiness.starter_ratio >= 0.5;
        let (top_goal, top_pressure) = p
            .mind
            .goals()
            .strongest()
            .map_or((GoalKind::None, 0.0), |g| (g.kind, g.pressure()));
        Snap {
            club_id: place.club_id,
            league_rep: place.league_rep,
            club_rep: place.club_rep,
            rank: place.rank,
            age: p.age(date),
            ca: p.player_attributes.current_ability,
            rep: p.player_attributes.current_reputation,
            status,
            regular: starting || (place.main_team && matches!(status, "Key" | "Regular")),
            starting,
            starter,
            on_loan: p.is_on_loan(),
            days_left: contract.map_or(0, |c| (c.expiration - date).num_days() as i32),
            salary: contract.map_or(0, |c| c.salary),
            listed: contract.is_some_and(|c| c.is_transfer_listed)
                || p.statuses.has(PlayerStatusType::Lst),
            req: p.statuses.has(PlayerStatusType::Req),
            req_reason: p.transfer_request_reasons.first().copied(),
            asks_only_bigger: p.asks_only_for_a_bigger_stage(),
            top_goal,
            top_pressure,
            arc: p.mind.career.plan.map(|plan| plan.arc),
        }
    }

    pub fn tier(&self) -> Tier {
        Tier::of_rank(self.rank)
    }

    pub fn status_label(status: &PlayerSquadStatus) -> &'static str {
        match status {
            PlayerSquadStatus::KeyPlayer => "Key",
            PlayerSquadStatus::FirstTeamRegular => "Regular",
            PlayerSquadStatus::FirstTeamSquadRotation => "Rotation",
            PlayerSquadStatus::MainBackupPlayer => "Backup",
            PlayerSquadStatus::HotProspectForTheFuture => "HotProspect",
            PlayerSquadStatus::DecentYoungster => "Youngster",
            PlayerSquadStatus::NotNeeded => "NotNeeded",
            PlayerSquadStatus::NotYetSet => "NotSet",
            _ => "?",
        }
    }

    /// Higher is a bigger role, so a promotion can be read as a rise.
    pub fn role_rank(status: &str) -> u8 {
        match status {
            "Key" => 5,
            "Regular" => 4,
            "Rotation" => 3,
            "Backup" => 2,
            "HotProspect" | "Youngster" => 1,
            _ => 0,
        }
    }
}

/// What the market wrote about a man today.
#[derive(Clone)]
pub struct MoveRow {
    pub fee: f64,
    pub kind: MoveKind,
    pub reason: String,
    pub to_club_id: u32,
    pub player_name: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveKind {
    Permanent,
    Loan,
    Free,
}

/// Visits every player under contract and in the pool.
pub struct WorldWalk;

impl WorldWalk {
    pub fn players<'a>(
        data: &'a SimulatorData,
        ranks: &ClubRanks,
        mut visit: impl FnMut(&'a Player, &Place<'a>),
    ) {
        for continent in &data.continents {
            for country in &continent.countries {
                let reps: HashMap<u32, u16> = country
                    .leagues
                    .leagues
                    .iter()
                    .map(|l| (l.id, l.reputation))
                    .collect();
                for club in &country.clubs {
                    let main = club.teams.main();
                    let club_rep = main.map_or(0.0, |t| t.reputation.overall_score());
                    let league_rep = main
                        .and_then(|t| t.league_id)
                        .and_then(|l| reps.get(&l).copied())
                        .unwrap_or(0);
                    let rank = ranks.of(club.id);
                    for team in &club.teams.teams {
                        let place = Place {
                            club_id: club.id,
                            club_name: &club.name,
                            league_rep,
                            club_rep,
                            rank,
                            main_team: team.team_type == core::TeamType::Main,
                        };
                        for p in &team.players.players {
                            visit(p, &place);
                        }
                    }
                }
            }
        }
        for p in &data.free_agents {
            visit(p, &Place::NOWHERE);
        }
    }
}

/// The men who retired since the last call, by a cursor per country.
#[derive(Default)]
pub struct RetirementWatch {
    cursors: HashMap<u32, usize>,
}

impl RetirementWatch {
    pub fn new_since<'a>(&mut self, data: &'a SimulatorData) -> Vec<&'a Player> {
        let mut retired = Vec::new();
        for continent in &data.continents {
            for country in &continent.countries {
                let players = &country.retired_players.players;
                let cursor = self.cursors.entry(country.id).or_insert(0);
                if *cursor > players.len() {
                    *cursor = 0;
                }
                retired.extend(players[*cursor..].iter());
                *cursor = players.len();
            }
        }
        retired
    }
}

/// Today's market rows by player, by a cursor per country.
#[derive(Default)]
pub struct MarketWatch {
    cursors: HashMap<u32, usize>,
}

impl MarketWatch {
    pub fn new_rows(&mut self, data: &SimulatorData) -> HashMap<u32, Vec<MoveRow>> {
        let mut rows: HashMap<u32, Vec<MoveRow>> = HashMap::new();
        for continent in &data.continents {
            for country in &continent.countries {
                let history = &country.transfer_market.transfer_history;
                let cursor = self.cursors.entry(country.id).or_insert(0);
                if *cursor > history.len() {
                    *cursor = 0;
                }
                for row in &history[*cursor..] {
                    let kind = match row.transfer_type {
                        core::transfers::TransferType::Permanent => MoveKind::Permanent,
                        core::transfers::TransferType::Loan(_) => MoveKind::Loan,
                        core::transfers::TransferType::Free => MoveKind::Free,
                    };
                    rows.entry(row.player_id).or_default().push(MoveRow {
                        fee: row.fee.amount,
                        kind,
                        reason: row.reason.key.clone(),
                        to_club_id: row.to_club_id,
                        player_name: row.player_name.clone(),
                    });
                }
                *cursor = history.len();
            }
        }
        rows
    }
}

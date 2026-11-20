//! Who a player is to the census and where he is: his position kind and
//! group, his place in a club, and the club's rank in the world.

use core::{Player, PlayerPositionType, SimulatorData, TeamType};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Kind {
    GK,
    DC,
    FB,
    DM,
    MC,
    WM,
    AMC,
    W,
    ST,
}

impl Kind {
    pub const ALL: [Kind; 9] = [
        Kind::GK,
        Kind::DC,
        Kind::FB,
        Kind::DM,
        Kind::MC,
        Kind::WM,
        Kind::AMC,
        Kind::W,
        Kind::ST,
    ];

    pub fn of(position: PlayerPositionType) -> Kind {
        use PlayerPositionType::*;
        match position {
            Goalkeeper => Kind::GK,
            Sweeper | DefenderCenterLeft | DefenderCenter | DefenderCenterRight => Kind::DC,
            DefenderLeft | DefenderRight | WingbackLeft | WingbackRight => Kind::FB,
            DefensiveMidfielder => Kind::DM,
            MidfielderCenterLeft | MidfielderCenter | MidfielderCenterRight => Kind::MC,
            MidfielderLeft | MidfielderRight => Kind::WM,
            AttackingMidfielderCenter => Kind::AMC,
            AttackingMidfielderLeft | AttackingMidfielderRight | ForwardLeft | ForwardRight => {
                Kind::W
            }
            ForwardCenter | Striker => Kind::ST,
        }
    }

    pub fn group(self) -> usize {
        match self {
            Kind::GK => 0,
            Kind::DC | Kind::FB => 1,
            Kind::DM | Kind::MC | Kind::WM => 2,
            Kind::AMC | Kind::W | Kind::ST => 3,
        }
    }
}

pub const GROUPS: [&str; 4] = ["GK", "DEF", "MID", "FWD"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    Main = 0,
    Reserve = 1,
    Youth = 2,
    Academy = 3,
    Free = 4,
}

impl Place {
    pub const ALL: [Place; 5] = [
        Place::Main,
        Place::Reserve,
        Place::Youth,
        Place::Academy,
        Place::Free,
    ];
    pub const NAMES: [&str; 5] = ["main", "reserve", "youth", "academy", "free"];

    pub fn of(team: &TeamType) -> Place {
        match team {
            TeamType::Main => Place::Main,
            TeamType::B | TeamType::Reserve | TeamType::Second => Place::Reserve,
            _ => Place::Youth,
        }
    }

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    /// In a club squad: not an academy boy, not a free agent.
    pub fn in_squad(self) -> bool {
        !matches!(self, Place::Academy | Place::Free)
    }
}

/// One player as the walk meets him.
pub struct Seen<'a> {
    pub player: &'a Player,
    pub place: Place,
    pub rank: u32,
    pub league_rep: u16,
}

/// Every player of the world: first teams, reserve and youth sides,
/// academies and the free-agent pool.
pub struct WorldWalk;

impl WorldWalk {
    pub fn visit<'a>(data: &'a SimulatorData, mut visit: impl FnMut(Seen<'a>)) {
        let ranks = Self::club_ranks(data);
        for continent in &data.continents {
            for country in &continent.countries {
                let reps: HashMap<u32, u16> = country
                    .leagues
                    .leagues
                    .iter()
                    .map(|l| (l.id, l.reputation))
                    .collect();
                for club in &country.clubs {
                    let rank = ranks.get(&club.id).copied().unwrap_or(u32::MAX);
                    for team in &club.teams.teams {
                        let league_rep = team
                            .league_id
                            .and_then(|l| reps.get(&l).copied())
                            .unwrap_or(0);
                        let place = Place::of(&team.team_type);
                        for player in &team.players.players {
                            visit(Seen {
                                player,
                                place,
                                rank,
                                league_rep,
                            });
                        }
                    }
                    for player in &club.academy.players.players {
                        visit(Seen {
                            player,
                            place: Place::Academy,
                            rank,
                            league_rep: 0,
                        });
                    }
                }
            }
        }
        for player in &data.free_agents {
            visit(Seen {
                player,
                place: Place::Free,
                rank: u32::MAX,
                league_rep: 0,
            });
        }
    }

    /// Clubs by the reputation of their first team, best first.
    fn club_ranks(data: &SimulatorData) -> HashMap<u32, u32> {
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
        clubs
            .iter()
            .enumerate()
            .map(|(rank, (id, _))| (*id, rank as u32))
            .collect()
    }
}

//! Who the coach would play in every shirt of his formation, and who
//! stands behind each of them.

use crate::club::staff::coach::memory::CoachMemoryStore;
use crate::club::staff::coach::standing::StandingRead;
use crate::club::staff::goalkeeping::KeeperRoomPlan;
use crate::club::staff::perception::{AbilityEstimator, PotentialEstimator};
use crate::utils::SlotAssignment;
use crate::{Player, PlayerPositionType, RoleFamiliarity, Tactics};
use chrono::NaiveDate;
use rustc_hash::FxHashMap;

/// One man's place in the chart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChartPlace {
    /// Index of his shirt in the formation.
    pub shirt: usize,
    pub slot: PlayerPositionType,
    /// Men planned ahead of him for the shirt: 0 is the first choice.
    pub place: usize,
    /// What the coach makes of him in that shirt: the level he can see,
    /// scaled by how well the man plays the position and moved by the trust
    /// he has earned.
    pub standing: f32,
}

/// The men planned for one shirt, first choice first.
#[derive(Debug, Clone)]
pub struct ShirtDepth {
    pub slot: PlayerPositionType,
    pub men: Vec<(u32, f32)>,
}

/// Built from what the coach can observe, never from hidden ability. Every
/// player is planned in one shirt: the one where the squad needs him most.
#[derive(Debug, Clone, Default)]
pub struct SquadDepthChart {
    shirts: Vec<ShirtDepth>,
    places: FxHashMap<u32, ChartPlace>,
}

impl SquadDepthChart {
    /// What a shirt is worth to a man who cannot play it: far enough below
    /// any standing that the assignment gives it to anyone else first.
    const CANNOT_PLAY: f32 = -1.0e4;

    /// Levels a man the staff wholly believe in gains on the chart of a
    /// side that exists only to develop players.
    const BELIEF_REACH: f32 = 20.0;

    /// `development` is how far the side exists to develop players
    /// ([`crate::TeamType::development_weight`]): there the staff's belief
    /// in a man counts toward his standing as well as what he is today.
    pub fn build(
        players: &[&Player],
        tactics: &Tactics,
        keepers: Option<&KeeperRoomPlan>,
        memory: Option<&CoachMemoryStore>,
        development: f32,
        today: NaiveDate,
    ) -> Self {
        let slots = tactics.positions();
        let mut shirts: Vec<ShirtDepth> = slots
            .iter()
            .map(|&slot| ShirtDepth {
                slot,
                men: Vec::new(),
            })
            .collect();

        let standing = |player: &Player, slot: PlayerPositionType| -> f32 {
            let level = f32::from(AbilityEstimator::observable_level(player));
            let familiarity =
                RoleFamiliarity::credit_at(RoleFamiliarity::level_for_slot(&player.positions, slot));
            let belief = if development > 0.0 {
                development
                    * Self::BELIEF_REACH
                    * PotentialEstimator::believed_growth(player, today)
            } else {
                0.0
            };
            (level + belief) * familiarity
                + StandingRead::chart_margin(memory.and_then(|m| m.standing_of(player.id)))
        };

        // The gloves follow the goalkeeping department's declared order
        // where it has one, and the coach's own eye behind it.
        if let Some(goal) = slots
            .iter()
            .position(|slot| *slot == PlayerPositionType::Goalkeeper)
        {
            let declared = |id: u32| -> usize {
                let order = keepers.map(|plan| [plan.number_one(), plan.deputy(), plan.third()]);
                order
                    .and_then(|order| order.iter().position(|k| *k == Some(id)))
                    .unwrap_or(usize::MAX)
            };
            let mut room: Vec<(u32, f32)> = players
                .iter()
                .filter(|p| p.positions.is_goalkeeper())
                .map(|p| (p.id, standing(p, PlayerPositionType::Goalkeeper)))
                .collect();
            room.sort_by(|a, b| {
                declared(a.0)
                    .cmp(&declared(b.0))
                    .then(b.1.total_cmp(&a.1))
                    .then(a.0.cmp(&b.0))
            });
            shirts[goal].men = room;
        }

        // Outfield shirts are filled a string at a time: one assignment
        // names the first choices, the next names the men behind them from
        // whoever is left, until everyone is planned somewhere. Nobody is
        // planned for a shirt he cannot play: a shirt the squad has no one
        // for is left to the day's team sheet.
        let outfield: Vec<usize> = (0..slots.len())
            .filter(|i| slots[*i] != PlayerPositionType::Goalkeeper)
            .collect();
        let familiarity = |player: &Player, slot: PlayerPositionType| -> f32 {
            RoleFamiliarity::level_for_slot(&player.positions, slot)
        };
        let mut pool: Vec<&Player> = players
            .iter()
            .copied()
            .filter(|p| !p.positions.is_goalkeeper())
            .collect();
        pool.sort_by_key(|p| p.id);
        while !pool.is_empty() && !outfield.is_empty() {
            let scores: Vec<f32> = pool
                .iter()
                .flat_map(|p| {
                    outfield.iter().map(|&shirt| {
                        if RoleFamiliarity::credit_at(familiarity(p, slots[shirt])) > 0.0 {
                            standing(p, slots[shirt])
                        } else {
                            Self::CANNOT_PLAY
                        }
                    })
                })
                .collect();
            let string = SlotAssignment::solve(&scores, outfield.len(), &vec![false; pool.len()]);
            let mut placed: Vec<usize> = Vec::with_capacity(outfield.len());
            for (i, candidate) in string.into_iter().enumerate() {
                let Some(candidate) = candidate else {
                    continue;
                };
                let score = scores[candidate * outfield.len() + i];
                if score <= Self::CANNOT_PLAY {
                    continue;
                }
                shirts[outfield[i]].men.push((pool[candidate].id, score));
                placed.push(candidate);
            }
            if placed.is_empty() {
                break;
            }
            placed.sort_unstable_by(|a, b| b.cmp(a));
            for candidate in placed {
                pool.swap_remove(candidate);
            }
            pool.sort_by_key(|p| p.id);
        }
        // A man who can play no shirt of this shape stands last behind the
        // one he is least unfamiliar with, among those somebody holds.
        for player in pool {
            let shirt = outfield
                .iter()
                .copied()
                .max_by(|a, b| {
                    let key = |shirt: usize| {
                        (
                            !shirts[shirt].men.is_empty(),
                            familiarity(player, slots[shirt]),
                        )
                    };
                    let (held_a, level_a) = key(*a);
                    let (held_b, level_b) = key(*b);
                    held_a.cmp(&held_b).then(level_a.total_cmp(&level_b))
                });
            if let Some(shirt) = shirt {
                let standing = standing(player, slots[shirt]);
                shirts[shirt].men.push((player.id, standing));
            }
        }

        let places = shirts
            .iter()
            .enumerate()
            .flat_map(|(shirt, depth)| {
                depth.men.iter().enumerate().map(move |(place, (id, standing))| {
                    (
                        *id,
                        ChartPlace {
                            shirt,
                            slot: depth.slot,
                            place,
                            standing: *standing,
                        },
                    )
                })
            })
            .collect();

        SquadDepthChart { shirts, places }
    }

    pub fn shirts(&self) -> &[ShirtDepth] {
        &self.shirts
    }

    pub fn place_of(&self, player_id: u32) -> Option<&ChartPlace> {
        self.places.get(&player_id)
    }

    /// The first choice for the shirt a player is planned in, and his
    /// standing there.
    pub fn first_choice_ahead_of(&self, player_id: u32) -> Option<(u32, f32)> {
        let place = self.places.get(&player_id)?;
        self.shirts[place.shirt]
            .men
            .first()
            .copied()
            .filter(|(id, _)| *id != player_id)
    }

    /// The best standing any first choice holds.
    pub fn top_standing(&self) -> f32 {
        self.shirts
            .iter()
            .filter_map(|shirt| shirt.men.first())
            .map(|(_, standing)| *standing)
            .fold(0.0, f32::max)
    }

    /// The mean standing of everyone in the chart: what the squad is.
    pub fn squad_standing(&self) -> f32 {
        if self.places.is_empty() {
            return 0.0;
        }
        self.places.values().map(|place| place.standing).sum::<f32>() / self.places.len() as f32
    }
}

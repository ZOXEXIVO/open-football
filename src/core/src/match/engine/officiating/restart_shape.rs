//! **Where the others stand while a free kick, a penalty or a throw-in is
//! set up** — the same job [`CornerShape`](super::corner_shape::CornerShape)
//! does for a corner.
//!
//! A free kick within shooting range gets a wall on the line between ball
//! and goal at the restraining distance, and every other opponent inside
//! that distance steps back to it; within delivery range the side taking
//! it sends its best men in the air into the area. A penalty empties the area and the arc
//! of everybody but the taker and the keeper. A throw-in gets two
//! team-mates showing short. Like the corner it places people and decides
//! nothing: no RNG, no clock, the same players in the same places always
//! produce the same plan. Holding them there belongs to `SetPieceHold`.

use crate::r#match::engine::set_pieces::{FreeKickBand, wall_size_for};
use crate::r#match::{MatchPlayer, PassOriginRestart, PlayerSide};
use nalgebra::Vector3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RestartStation {
    pub player_id: u32,
    pub position: Vector3<f32>,
}

pub struct RestartShape;

impl RestartShape {
    /// 9.15 m, the distance the laws keep opponents from a dead ball.
    pub const RETREAT: f32 = 73.2;
    /// Standing just beyond a line rather than on it.
    const MARGIN: f32 = 4.0;
    /// Where a free-kick wall stands, out from the ball: the restraining
    /// distance and a step.
    pub const WALL_DISTANCE: f32 = Self::RETREAT + Self::MARGIN;
    /// Close enough to his station to count as on it: `Arrive` settles a
    /// man within three units of the point.
    const ON_STATION: f32 = 8.0;
    /// Shoulder to shoulder in a wall.
    const WALL_SPACING: f32 = 5.0;
    /// 16.5 m and 40.32 m: the penalty area's depth and width.
    const AREA_DEPTH: f32 = 132.0;
    const AREA_WIDTH: f32 = 322.6;
    /// Within this of goal a free kick that is not struck at goal is
    /// delivered into the area, and the side taking it loads the area for
    /// it (~40 m).
    pub const DELIVERY_RANGE: f32 = 320.0;
    /// Men sent into the area for a delivery, and how many of the side's
    /// deepest stay behind the ball.
    const DELIVERY_RUNNERS: usize = 4;
    const HELD_BACK: usize = 2;
    /// Where they stand across the area, off the goal's centre line:
    /// either post and just outside each.
    const RUNNER_SPREAD: [f32; 4] = [-96.0, -32.0, 32.0, 96.0];
    /// A stride on his own side of the defensive line, so he is onside
    /// when it is struck.
    const ONSIDE_STEP: f32 = 8.0;
    /// The six-yard line: nobody lines up for a delivery inside it.
    const SIX_YARD_DEPTH: f32 = 44.0;
    /// The two men showing for a throw: one up the line, one infield.
    const THROW_UP_THE_LINE: f32 = 72.0;
    const THROW_INFIELD: f32 = 56.0;

    /// Whether a restart of this kind is set up with stations and held.
    pub fn is_shaped(origin: PassOriginRestart) -> bool {
        matches!(
            origin,
            PassOriginRestart::Corner
                | PassOriginRestart::DirectFreeKick
                | PassOriginRestart::IndirectFreeKick
                | PassOriginRestart::Penalty
                | PassOriginRestart::ThrowIn
        )
    }

    /// The opponents have stood off a free kick or a penalty as the laws
    /// require and its wall is up, so it may be taken.
    pub fn formed(
        origin: PassOriginRestart,
        players: &[MatchPlayer],
        taker: &MatchPlayer,
        spot: Vector3<f32>,
        goal_x: f32,
        field_height: f32,
    ) -> bool {
        if origin == PassOriginRestart::Penalty {
            return Self::area_clear(players, taker, spot, goal_x, field_height);
        }
        Self::opponents(players, taker).all(|p| {
            p.tactical_position.current_position.is_goalkeeper()
                || ((p.position - spot).xy().norm() >= Self::RETREAT
                    && p
                        .set_piece_station
                        .is_none_or(|s| (p.position - s).xy().norm() <= Self::ON_STATION))
        })
    }

    /// Who is standing where the laws do not let him while a free kick or
    /// a penalty waits, and where he goes: anybody in the area or the arc
    /// at a penalty, an opponent inside 9.15 m at a free kick. The plan
    /// made at the award only knew who was there at that moment.
    pub fn restraint(
        origin: PassOriginRestart,
        players: &[MatchPlayer],
        taker_id: u32,
        spot: Vector3<f32>,
        field_width: f32,
        field_height: f32,
    ) -> Vec<RestartStation> {
        if origin == PassOriginRestart::Penalty {
            return Self::plan(origin, players, taker_id, spot, field_width, field_height);
        }
        let Some(taker) = players.iter().find(|p| p.id == taker_id) else {
            return Vec::new();
        };
        Self::opponents(players, taker)
            .filter(|p| !p.tactical_position.current_position.is_goalkeeper())
            .filter_map(|p| {
                Self::outside_retreat(p.position, spot).map(|position| RestartStation {
                    player_id: p.id,
                    position,
                })
            })
            .collect()
    }

    /// Nobody but the taker and the keeper he faces is in the area or
    /// within 9.15 m of the spot.
    fn area_clear(
        players: &[MatchPlayer],
        taker: &MatchPlayer,
        spot: Vector3<f32>,
        goal_x: f32,
        field_height: f32,
    ) -> bool {
        let (top, bottom) = (
            (field_height - Self::AREA_WIDTH) * 0.5,
            (field_height + Self::AREA_WIDTH) * 0.5,
        );
        players.iter().all(|p| {
            let in_area = (p.position.x - goal_x).abs() < Self::AREA_DEPTH
                && p.position.y > top
                && p.position.y < bottom;
            let on_arc = (p.position - spot).xy().norm() < Self::RETREAT;
            p.id == taker.id
                || p.off_pitch
                || (p.team_id != taker.team_id
                    && p.tactical_position.current_position.is_goalkeeper())
                || !(in_area || on_arc)
        })
    }

    /// Stations for every player who has somewhere to be.
    pub fn plan(
        origin: PassOriginRestart,
        players: &[MatchPlayer],
        taker_id: u32,
        spot: Vector3<f32>,
        field_width: f32,
        field_height: f32,
    ) -> Vec<RestartStation> {
        let Some(taker) = players.iter().find(|p| p.id == taker_id) else {
            return Vec::new();
        };
        let Some(attacking) = taker.side else {
            return Vec::new();
        };
        let goal_x = match attacking {
            PlayerSide::Left => field_width,
            PlayerSide::Right => 0.0,
        };
        let goal = Vector3::new(goal_x, field_height * 0.5, 0.0);
        match origin {
            PassOriginRestart::DirectFreeKick | PassOriginRestart::IndirectFreeKick => {
                Self::free_kick(players, taker, spot, goal, field_height)
            }
            PassOriginRestart::Penalty => Self::penalty(players, taker, spot, goal, field_height),
            PassOriginRestart::ThrowIn => {
                Self::throw_in(players, taker, spot, goal, field_width, field_height)
            }
            _ => Vec::new(),
        }
    }

    fn opponents<'a>(
        players: &'a [MatchPlayer],
        taker: &'a MatchPlayer,
    ) -> impl Iterator<Item = &'a MatchPlayer> + 'a {
        players
            .iter()
            .filter(move |p| p.team_id != taker.team_id && !p.off_pitch)
    }

    fn free_kick(
        players: &[MatchPlayer],
        taker: &MatchPlayer,
        spot: Vector3<f32>,
        goal: Vector3<f32>,
        field_height: f32,
    ) -> Vec<RestartStation> {
        let to_goal = Vector3::new(goal.x - spot.x, goal.y - spot.y, 0.0);
        let distance = to_goal.norm();
        let band = FreeKickBand::from_distance(distance);
        let wide = (spot.y - field_height * 0.5).abs() > field_height * 0.18;
        let wall_size = if band == FreeKickBand::Far || distance <= 0.0 {
            0
        } else {
            wall_size_for(band, wide) as usize
        };
        let dir = if distance > 0.0 {
            to_goal / distance
        } else {
            to_goal
        };
        let across = Vector3::new(-dir.y, dir.x, 0.0);
        let wall_centre = spot + dir * Self::WALL_DISTANCE;

        let mut outfield: Vec<&MatchPlayer> = Self::opponents(players, taker)
            .filter(|p| !p.tactical_position.current_position.is_goalkeeper())
            .collect();
        outfield.sort_by(|a, b| {
            let da = (a.position - wall_centre).norm_squared();
            let db = (b.position - wall_centre).norm_squared();
            da.total_cmp(&db)
        });

        let mut stations = Vec::new();
        let wall_n = wall_size.min(outfield.len());
        for (i, man) in outfield.iter().take(wall_n).enumerate() {
            let offset = (i as f32 - (wall_n as f32 - 1.0) * 0.5) * Self::WALL_SPACING;
            stations.push(RestartStation {
                player_id: man.id,
                position: wall_centre + across * offset,
            });
        }
        for man in outfield.iter().skip(wall_n) {
            if let Some(back) = Self::outside_retreat(man.position, spot) {
                stations.push(RestartStation {
                    player_id: man.id,
                    position: back,
                });
            }
        }
        if distance <= Self::DELIVERY_RANGE {
            stations.extend(Self::delivery_runners(players, taker, goal));
        }
        stations
    }

    /// The side taking a free kick within range of the area loads it: its
    /// best men in the air line up across the area level with the
    /// defence, and its two deepest stay behind the ball. Without them a
    /// free kick out wide was played short, because nobody had gone up
    /// for it.
    fn delivery_runners(
        players: &[MatchPlayer],
        taker: &MatchPlayer,
        goal: Vector3<f32>,
    ) -> Vec<RestartStation> {
        let depth = |p: &MatchPlayer| (p.position.x - goal.x).abs();
        let mut line: Vec<f32> = Self::opponents(players, taker).map(depth).collect();
        line.sort_by(f32::total_cmp);
        let line_depth = line
            .get(1)
            .copied()
            .unwrap_or(Self::AREA_DEPTH)
            .clamp(Self::SIX_YARD_DEPTH, Self::AREA_DEPTH)
            + Self::ONSIDE_STEP;

        let mut mates: Vec<&MatchPlayer> = players
            .iter()
            .filter(|p| {
                p.team_id == taker.team_id
                    && p.id != taker.id
                    && !p.off_pitch
                    && !p.tactical_position.current_position.is_goalkeeper()
            })
            .collect();
        mates.sort_by(|a, b| depth(b).total_cmp(&depth(a)));
        let aerial = |p: &MatchPlayer| {
            p.skills.technical.heading * 0.5
                + p.skills.physical.jumping * 0.3
                + p.skills.physical.strength * 0.2
        };
        let mut runners: Vec<&MatchPlayer> = mates.into_iter().skip(Self::HELD_BACK).collect();
        runners.sort_by(|a, b| aerial(b).total_cmp(&aerial(a)).then(a.id.cmp(&b.id)));

        let toward_play = if goal.x > 0.0 { -1.0 } else { 1.0 };
        runners
            .into_iter()
            .take(Self::DELIVERY_RUNNERS)
            .zip(Self::RUNNER_SPREAD)
            .map(|(man, across)| RestartStation {
                player_id: man.id,
                position: Vector3::new(goal.x + toward_play * line_depth, goal.y + across, 0.0),
            })
            .collect()
    }

    fn penalty(
        players: &[MatchPlayer],
        taker: &MatchPlayer,
        spot: Vector3<f32>,
        goal: Vector3<f32>,
        field_height: f32,
    ) -> Vec<RestartStation> {
        let outward = if goal.x > spot.x { -1.0 } else { 1.0 };
        let edge_x = goal.x + outward * (Self::AREA_DEPTH + Self::MARGIN);
        let (top, bottom) = (
            (field_height - Self::AREA_WIDTH) * 0.5,
            (field_height + Self::AREA_WIDTH) * 0.5,
        );
        let in_area = |at: Vector3<f32>| {
            (at.x - goal.x).abs() < Self::AREA_DEPTH + Self::MARGIN && at.y > top && at.y < bottom
        };
        let defending_keeper = |p: &MatchPlayer| {
            p.team_id != taker.team_id && p.tactical_position.current_position.is_goalkeeper()
        };
        players
            .iter()
            .filter(|p| p.id != taker.id && !p.off_pitch && !defending_keeper(p))
            .filter(|p| in_area(p.position) || (p.position - spot).norm() < Self::RETREAT)
            .map(|p| {
                let mut at = Vector3::new(edge_x, p.position.y.clamp(top, bottom), 0.0);
                // The arc: still inside 9.15 m of the spot, step further out.
                let gap = (at - spot).norm();
                if gap < Self::RETREAT + Self::MARGIN {
                    let dy = at.y - spot.y;
                    let need = ((Self::RETREAT + Self::MARGIN).powi(2) - dy * dy)
                        .max(0.0)
                        .sqrt();
                    at.x = spot.x + outward * need;
                }
                RestartStation {
                    player_id: p.id,
                    position: at,
                }
            })
            .collect()
    }

    fn throw_in(
        players: &[MatchPlayer],
        taker: &MatchPlayer,
        spot: Vector3<f32>,
        goal: Vector3<f32>,
        field_width: f32,
        field_height: f32,
    ) -> Vec<RestartStation> {
        let infield = if spot.y < field_height * 0.5 {
            1.0
        } else {
            -1.0
        };
        let forward = if goal.x > spot.x { 1.0 } else { -1.0 };
        let targets = [
            Vector3::new(
                spot.x + forward * Self::THROW_UP_THE_LINE,
                spot.y + infield * 16.0,
                0.0,
            ),
            Vector3::new(
                spot.x - forward * 12.0,
                spot.y + infield * Self::THROW_INFIELD,
                0.0,
            ),
        ];
        let mut mates: Vec<&MatchPlayer> = players
            .iter()
            .filter(|p| {
                p.team_id == taker.team_id
                    && p.id != taker.id
                    && !p.off_pitch
                    && !p.tactical_position.current_position.is_goalkeeper()
            })
            .collect();
        let mut stations = Vec::new();
        for target in targets {
            let target = Vector3::new(
                target.x.clamp(8.0, field_width - 8.0),
                target.y.clamp(8.0, field_height - 8.0),
                0.0,
            );
            let Some((idx, _)) = mates.iter().enumerate().min_by(|(_, a), (_, b)| {
                (a.position - target)
                    .norm_squared()
                    .total_cmp(&(b.position - target).norm_squared())
            }) else {
                break;
            };
            let man = mates.remove(idx);
            stations.push(RestartStation {
                player_id: man.id,
                position: target,
            });
        }
        stations
    }

    /// Where a man inside the restraining distance steps back to, or
    /// `None` when he is already outside it.
    fn outside_retreat(at: Vector3<f32>, spot: Vector3<f32>) -> Option<Vector3<f32>> {
        let from = Vector3::new(at.x - spot.x, at.y - spot.y, 0.0);
        let gap = from.norm();
        if gap >= Self::RETREAT {
            return None;
        }
        let dir = if gap > 0.0 {
            from / gap
        } else {
            Vector3::new(1.0, 0.0, 0.0)
        };
        Some(spot + dir * (Self::RETREAT + Self::MARGIN))
    }
}

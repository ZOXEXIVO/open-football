//! **Where the twenty-two stand while a kick-off is taken.**
//!
//! Law 8 puts the ball on the centre mark and the opponents outside the
//! centre circle; universal practice, rather than the law, puts a second
//! man of the kicking side alongside the taker, because a kick-off is a
//! pass and a pass needs somebody to receive it.
//!
//! The engine had none of that. `assign_kickoff` teleported one man onto
//! the ball and left the other twenty-one on the formation dots that
//! `reset_players_positions` had just written. Measured on a 4-2-3-1 at
//! the first whistle, the man that puts on the ball is the lone striker:
//!
//! ```text
//!   nearest team-mate     60 u  (7.5 m)  — and BEHIND the ball
//!   nearest opponent      15 u  (1.9 m)  — the opposing striker, stood
//!                                          well inside the centre circle
//! ```
//!
//! So every kick-off was taken by an isolated forward with a defender on
//! his shoulder and nobody to give it to. `ForwardPassingState` scanned
//! for a pass, found nothing it would play, timed out after thirty ticks
//! and dropped him into `Running` — and he set off up the pitch with it
//! alone. That is the reported *"he plays the ball to himself"*, and it
//! is the same defect [`ThrowInDelivery`] was written for at the other
//! restart.
//!
//! This module places people; it decides nothing. No RNG, no clock — the
//! same twenty-two always produce the same set-up, which is what keeps
//! replays reproducible and leaves the post-goal path's single RNG draw
//! the only one it takes.
//!
//! [`ThrowInDelivery`]: crate::r#match::common_states::ThrowInDelivery

use crate::PlayerFieldPositionGroup;
use crate::r#match::{MatchPlayer, PlayerSide};
use nalgebra::Vector3;

/// A player and the spot the set-up puts him on.
#[derive(Debug, Clone, Copy)]
pub struct KickoffStation {
    pub player_id: u32,
    pub position: Vector3<f32>,
}

/// The kick-off set-up: who stands over the ball with the taker, and
/// everybody the referee has to move to make room for it.
#[derive(Debug, Clone)]
pub struct KickoffPlan {
    /// The team-mate the kick-off is played to. `None` only when the
    /// kicking side has nobody left but its goalkeeper.
    pub partner: Option<u32>,
    pub stations: Vec<KickoffStation>,
}

pub struct KickoffShape;

impl KickoffShape {
    /// Law 8's ten yards — the distance the opponents keep from the ball
    /// until it is in play. 73 u = 9.15 m, which is the centre circle's
    /// radius and the same retreat `CornerShape` gives at the flag.
    pub const CIRCLE: f32 = 73.0;

    /// How far to the side of the ball the partner stands (3.25 m)…
    const PARTNER_SQUARE: f32 = 26.0;

    /// …and how far off it into his own half (1.5 m). Beside the ball
    /// rather than on it: the engine reads a ball within `Ball::AT_FEET`
    /// of a man as riding on him, and two men cannot both have it.
    const PARTNER_DROP: f32 = 12.0;

    /// Plan the set-up for a kick-off `side` is about to take from `spot`.
    ///
    /// `taker_id` is already on the ball and is excluded from every pool.
    pub fn plan(
        players: &[MatchPlayer],
        side: PlayerSide,
        taker_id: u32,
        spot: Vector3<f32>,
    ) -> KickoffPlan {
        let mut stations = Vec::with_capacity(6);
        let partner = Self::partner(players, side, taker_id, spot).map(|partner| {
            stations.push(KickoffStation {
                player_id: partner.id,
                position: Self::partner_station(partner, side, spot),
            });
            partner.id
        });
        Self::retreat(players, side.opposite(), spot, &mut stations);
        KickoffPlan { partner, stations }
    }

    /// The man who walks up and stands over the ball with the taker: the
    /// nearest outfielder to the centre mark who is not taking it.
    ///
    /// That is the same question `assign_kickoff` asks to pick the taker,
    /// asked once more — a kick-off is two men out of one scan, and the
    /// formation dots put the pair a real kick-off uses at the top of it
    /// whatever the shape.
    fn partner<'a>(
        players: &'a [MatchPlayer],
        side: PlayerSide,
        taker_id: u32,
        spot: Vector3<f32>,
    ) -> Option<&'a MatchPlayer> {
        players
            .iter()
            .filter(|p| {
                p.side == Some(side)
                    && p.id != taker_id
                    && !p.is_sent_off
                    && p.tactical_position.current_position.position_group()
                        != PlayerFieldPositionGroup::Goalkeeper
            })
            .min_by(|a, b| {
                (a.position - spot)
                    .norm_squared()
                    .total_cmp(&(b.position - spot).norm_squared())
            })
    }

    /// Beside the ball, on the side of the pitch he already stands on and
    /// a stride into his own half.
    fn partner_station(
        partner: &MatchPlayer,
        side: PlayerSide,
        spot: Vector3<f32>,
    ) -> Vector3<f32> {
        let lateral = if partner.position.y < spot.y {
            -1.0
        } else {
            1.0
        };
        Vector3::new(
            spot.x - side.forward_dir_x() * Self::PARTNER_DROP,
            spot.y + lateral * Self::PARTNER_SQUARE,
            0.0,
        )
    }

    /// Law 8's other half: every opponent inside the centre circle steps
    /// back out of it.
    ///
    /// Radially, so it is the shortest retreat that satisfies the law and
    /// a defender keeps the side of the pitch his shape gave him.
    fn retreat(
        players: &[MatchPlayer],
        defending: PlayerSide,
        spot: Vector3<f32>,
        stations: &mut Vec<KickoffStation>,
    ) {
        let own_half = Vector3::new(-defending.forward_dir_x(), 0.0, 0.0);
        for player in players
            .iter()
            .filter(|p| p.side == Some(defending) && !p.is_sent_off)
        {
            let away = player.position - spot;
            if away.norm_squared() >= Self::CIRCLE * Self::CIRCLE {
                continue;
            }
            let out = away.try_normalize(0.01).unwrap_or(own_half);
            stations.push(KickoffStation {
                player_id: player.id,
                position: Vector3::new(
                    spot.x + out.x * Self::CIRCLE,
                    spot.y + out.y * Self::CIRCLE,
                    0.0,
                ),
            });
        }
    }
}

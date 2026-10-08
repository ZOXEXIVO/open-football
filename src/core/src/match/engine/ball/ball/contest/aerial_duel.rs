//! **The long ball contested in the air** — a lofted pass dropping on its
//! man with an opponent close enough to go up with him.
//!
//! The open-play cross and the corner were each a private transaction
//! between the passer and one named receiver until their aerial contests
//! existed, and every other lofted pass still was: the in-flight window
//! reserves the delivery for its receiver, the interception roll fades out
//! above head height, and the receiver claims anything under 2.8 m. So a
//! 40 m ball dropping on a striker with a centre-half at his back reached
//! him four times in five — real long balls are won about as often as they
//! are lost, and the ones the defence wins are headed away as second
//! balls, a share of them into touch.

use crate::r#match::engine::ball::ball::{AerialReach, Ball, PlayerReach};
use crate::r#match::events::EventCollection;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{MatchContext, MatchPlayer, PassOriginRestart};
use nalgebra::Vector3;

impl Ball {
    /// The defender's share of an even duel: he is facing the ball, the
    /// man it was played to has it dropping over his shoulder.
    const DUEL_PARITY: f32 = 0.55;
    /// How much an aerial edge is worth, either way.
    const DUEL_SPREAD: f32 = 1.2;
    /// A header is met on the way back the way it came, give or take this
    /// many radians.
    const HEADED_SPREAD: f32 = 0.9;
    /// Pace of the header across the grass, u/tick: 0.8 = 10 m/s.
    const HEADED_SPEED: f32 = 0.8;
    const HEADED_SPEED_SPREAD: f32 = 0.25;
    /// How high it loops off his head, metres.
    const HEADED_APEX: f32 = 4.0;
    const HEADED_APEX_SPREAD: f32 = 1.5;
    /// Ticks the header is nobody's, so it leaves the two men before
    /// either can claim it.
    const HEADED_SEPARATION: usize = 12;

    /// One duel per lofted delivery, on the tick the ball comes down into
    /// the reach of an opponent near its man. The receiver winning it is
    /// the reception that was going to happen; the opponent winning it
    /// heads the ball away.
    pub fn try_aerial_duel(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        if self.aerial_duel_resolved
            || self.current_owner.is_some()
            || self.held_in_hands
            || self.flags.in_flight_state == 0
            || self.cached_shot_target.is_some()
            || self.aerial_delivery.is_some()
            || self.position.z <= AerialReach::VOLLEY
            || self.velocity.z >= 0.0
        {
            return;
        }
        let Some(receiver) = self
            .pass_target_player_id
            .and_then(|id| players.iter().find(|p| p.id == id))
        else {
            return;
        };
        let Some(challenger) = players
            .iter()
            .filter(|p| p.team_id != receiver.team_id && !p.off_pitch)
            .filter(|p| !p.tactical_position.current_position.is_goalkeeper())
            .filter(|p| PlayerReach::can_strike(self, p, true))
            .min_by(|a, b| {
                let gap = |p: &MatchPlayer| (p.position - self.position).xy().norm();
                gap(a).total_cmp(&gap(b))
            })
        else {
            return;
        };
        self.aerial_duel_resolved = true;

        let minute = sc::minute_from_ticks(self.current_tick_cached);
        let theirs = sc::aerial_outfield_defender(challenger, minute)
            * AerialReach::reach_difficulty(self.position.z, challenger.skills.physical.jumping);
        let ours = if PlayerReach::can_strike(self, receiver, true) {
            sc::aerial_outfield_attacker(receiver, minute)
                * AerialReach::reach_difficulty(self.position.z, receiver.skills.physical.jumping)
        } else {
            0.0
        };
        let headed_away = (Self::DUEL_PARITY + (theirs - ours) * Self::DUEL_SPREAD).clamp(0.1, 0.9);
        if context.rng.unit_f32() >= headed_away {
            return;
        }

        let leap =
            AerialReach::header_leap_for(self.position.z, challenger.skills.physical.jumping);
        if leap > 0.0 {
            events.add_player_event(PlayerEvent::Leap(challenger.id, leap));
        }
        let back = Vector3::new(-self.velocity.x, -self.velocity.y, 0.0)
            .try_normalize(1.0e-4)
            .unwrap_or_else(|| Vector3::new(1.0, 0.0, 0.0));
        let (sin, cos) = context.rng.jitter(0.0, Self::HEADED_SPREAD).sin_cos();
        let headed = Vector3::new(
            back.x * cos - back.y * sin,
            back.x * sin + back.y * cos,
            0.0,
        ) * context
            .rng
            .jitter(Self::HEADED_SPEED, Self::HEADED_SPEED_SPREAD);
        let apex = context
            .rng
            .jitter(Self::HEADED_APEX, Self::HEADED_APEX_SPREAD);
        self.velocity = Vector3::new(headed.x, headed.y, Ball::launch_speed_for_apex(apex));
        self.previous_owner = Some(receiver.id);
        self.pass_target_player_id = None;
        self.flags.in_flight_state = Self::HEADED_SEPARATION;
        self.claim_cooldown = 0;
        let tick = self.current_tick_cached;
        self.record_touch(challenger.id, challenger.team_id, tick, false);
        self.offside_snapshot = None;
        self.pass_origin_restart = PassOriginRestart::OpenPlay;
    }
}

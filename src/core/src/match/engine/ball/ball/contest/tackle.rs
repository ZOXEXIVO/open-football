//! **How the ball comes off a won tackle.**
//!
//! Winning a tackle used to mean owning the ball: every won challenge
//! handed the tackler a ball at his feet, whether it was a planted block
//! front-on or a leg thrown at it from behind at full stretch. A real
//! challenge as often knocks the ball away as keeps it — the stretch
//! nearly always does — and the ball it knocks away is a second ball for
//! whoever gets there, or a throw-in when the challenge is on the line.

use crate::r#match::common_states::TackleEngagement;
use crate::r#match::engine::ball::ball::{Ball, MAX_OWNER_TRACK_DISTANCE};
use crate::r#match::engine::flow::rng::MatchRng;
use crate::r#match::{MatchPlayer, PassOriginRestart};
use nalgebra::Vector3;

impl Ball {
    /// Share of won challenges a tackler keeps when he makes them at
    /// contact range — the block tackle, his foot planted through the
    /// ball.
    const KEPT_AT_CONTACT: f32 = 0.70;
    /// …and at the furthest he can reach for it: the poke and the slide,
    /// which take the ball off the man rather than giving it to anybody.
    const KEPT_AT_STRETCH: f32 = 0.20;
    /// Pace of the knock, u/tick: 0.40 = 5 m/s, give or take 2.
    const KNOCK_SPEED: f32 = 0.40;
    const KNOCK_SPEED_SPREAD: f32 = 0.15;
    /// How far off the line of the challenge the ball squirts, radians.
    const KNOCK_SPREAD: f32 = 0.6;
    /// Share of the carrier's own run the ball keeps through the contact.
    const CARRIED: f32 = 0.5;
    /// Ticks the knocked ball is nobody's, so it separates from the two
    /// men before either can claim it — the failed first touch's window.
    const KNOCK_SEPARATION: usize = 12;

    /// A won tackle. True when the tackler comes away with the ball;
    /// false when his challenge knocked it loose, driven on the line of
    /// it and carrying some of the carrier's run. `control` is his
    /// ball-winning against the standard of the match, 0.5 an ordinary
    /// one.
    pub fn on_tackle_won(
        &mut self,
        tackler: &MatchPlayer,
        control: f32,
        carrier_velocity: Vector3<f32>,
        rng: &MatchRng,
    ) -> bool {
        let challenge = Vector3::new(
            self.position.x - tackler.position.x,
            self.position.y - tackler.position.y,
            0.0,
        );
        let stretch = ((challenge.norm() - TackleEngagement::CONTACT)
            / (MAX_OWNER_TRACK_DISTANCE - TackleEngagement::CONTACT))
            .clamp(0.0, 1.0);
        let kept = (Self::KEPT_AT_CONTACT
            + (Self::KEPT_AT_STRETCH - Self::KEPT_AT_CONTACT) * stretch)
            * (0.5 + control.clamp(0.0, 1.0));
        if rng.unit_f32() < kept {
            return true;
        }

        let carried = Vector3::new(carrier_velocity.x, carrier_velocity.y, 0.0);
        let line = challenge
            .try_normalize(1.0e-4)
            .or_else(|| carried.try_normalize(1.0e-4))
            .unwrap_or_else(|| Vector3::new(1.0, 0.0, 0.0));
        let (sin, cos) = rng.jitter(0.0, Self::KNOCK_SPREAD).sin_cos();
        let knocked = Vector3::new(
            line.x * cos - line.y * sin,
            line.x * sin + line.y * cos,
            0.0,
        );
        self.velocity = knocked * rng.jitter(Self::KNOCK_SPEED, Self::KNOCK_SPEED_SPREAD)
            + carried * Self::CARRIED;
        self.previous_owner = self.current_owner;
        self.current_owner = None;
        self.pass_target_player_id = None;
        self.flags.in_flight_state = Self::KNOCK_SEPARATION;
        self.claim_cooldown = 0;
        let tick = self.current_tick_cached;
        self.record_touch(tackler.id, tackler.team_id, tick, false);
        self.offside_snapshot = None;
        self.pass_origin_restart = PassOriginRestart::OpenPlay;
        false
    }
}

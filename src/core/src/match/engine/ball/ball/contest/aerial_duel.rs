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

#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::diagnostics::delivery_diag::TouchKind;
use crate::r#match::engine::ball::ball::{
    AerialDelivery, AerialOutcome, AerialReach, Ball, PlayerReach,
};
use crate::r#match::events::EventCollection;
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::defender_census::{
    DefenderCensus, HeaderBooking, HeaderRule,
};
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::forward_census::FwdHeaderRule;
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
    pub(in crate::r#match::engine::ball::ball) const HEADED_SEPARATION: usize = 12;
    /// How long the ball waits for a man who has won it to be up for it
    /// (0.25 s) before it is a loose ball again.
    const HEADED_DEADLINE: u64 = 25;

    /// One duel per dropping ball, decided once in its flight: a lofted
    /// pass against an opponent near its man, or a ball played to nobody
    /// between the nearest man of each side under it.
    pub fn try_aerial_duel(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        if self.aerial_decided
            || self.current_owner.is_some()
            || self.held_in_hands
            || self.cached_shot_target.is_some()
            || self.aerial_delivery.is_some()
            || self.position.z <= AerialReach::VOLLEY
            || self.velocity.z >= 0.0
        {
            return;
        }
        match self.pass_target_player_id {
            Some(target) if self.flags.in_flight_state > 0 => {
                if let Some(receiver) = players.iter().find(|p| p.id == target) {
                    self.contest_pass(context, players, receiver, events);
                }
            }
            None if !MatchContext::second_ball_duel_off() => {
                self.contest_second_ball(context, players);
            }
            _ => {}
        }
    }

    /// The receiver winning it is the reception that was going to happen;
    /// the opponent winning it heads the ball away.
    fn contest_pass(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        receiver: &MatchPlayer,
        events: &mut EventCollection,
    ) {
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
        self.aerial_decided = true;
        #[cfg(feature = "match-logs")]
        context
            .defender_census
            .borrow_mut()
            .note_flight_decided(self.last_release_tick, true);

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

        #[cfg(feature = "match-logs")]
        context
            .defender_census
            .borrow_mut()
            .note_award(challenger.id, self.current_tick_cached);
        let back = Vector3::new(-self.velocity.x, -self.velocity.y, 0.0)
            .try_normalize(1.0e-4)
            .unwrap_or_else(|| Vector3::new(1.0, 0.0, 0.0));
        let velocity = Self::headed_along(context, back);

        // A man who plays his own header goes up for it in his heading
        // state, and the ball is played off his head once he is there.
        if challenger.plays_his_own_header() {
            self.aerial_delivery = Some(AerialDelivery {
                winner_id: challenger.id,
                target: self.position,
                outcome: AerialOutcome::HeadedAway {
                    velocity,
                    receiver: receiver.id,
                },
                arrival_height: self.position.z,
                deadline_tick: self.current_tick_cached + Self::HEADED_DEADLINE,
                force_heading: false,
            });
            self.pending_aerial_strike = Some(challenger.id);
            return;
        }

        let leap =
            AerialReach::header_leap_for(self.position.z, challenger.skills.physical.jumping);
        if leap > 0.0 {
            events.add_player_event(PlayerEvent::Leap(challenger.id, leap));
        }
        #[cfg(feature = "match-logs")]
        DefenderCensus::note_header(
            HeaderRule::AerialDuel,
            challenger,
            leap > 0.0,
            HeaderBooking::Nothing,
        );
        self.velocity = velocity;
        self.previous_owner = Some(receiver.id);
        self.pass_target_player_id = None;
        self.flags.in_flight_state = Self::HEADED_SEPARATION;
        self.claim_cooldown = 0;
        let tick = self.current_tick_cached;
        #[cfg(feature = "match-logs")]
        {
            self.touch_kind_hint = Some(TouchKind::Head);
        }
        self.record_touch(challenger.id, challenger.team_id, tick, false);
        self.offside_snapshot = None;
        self.pass_origin_restart = PassOriginRestart::OpenPlay;
    }

    /// The nearest man of each side under it goes up for it: the side that
    /// played it as the attacker, the other facing it as the defender, on
    /// the pass duel's composites. The winner is staged and plays it; a
    /// ball only one side can reach is theirs, and nothing rolls for it.
    fn contest_second_ball(&mut self, context: &MatchContext, players: &[MatchPlayer]) {
        let Some(played_by) = self.last_touch_team_id else {
            return;
        };
        let under = |theirs: bool| {
            players
                .iter()
                .filter(|p| (p.team_id == played_by) != theirs && !p.off_pitch)
                .filter(|p| !p.tactical_position.current_position.is_goalkeeper())
                .filter(|p| PlayerReach::can_strike(self, p, true))
                .min_by(|a, b| {
                    let gap = |p: &MatchPlayer| (p.position - self.position).xy().norm();
                    gap(a).total_cmp(&gap(b))
                })
        };
        let (Some(attacker), Some(defender)) = (under(false), under(true)) else {
            return;
        };
        self.aerial_decided = true;
        #[cfg(feature = "match-logs")]
        context
            .defender_census
            .borrow_mut()
            .note_flight_decided(self.last_release_tick, true);

        let minute = sc::minute_from_ticks(self.current_tick_cached);
        let theirs = sc::aerial_outfield_defender(defender, minute)
            * AerialReach::reach_difficulty(self.position.z, defender.skills.physical.jumping);
        let ours = sc::aerial_outfield_attacker(attacker, minute)
            * AerialReach::reach_difficulty(self.position.z, attacker.skills.physical.jumping);
        let defended = (Self::DUEL_PARITY + (theirs - ours) * Self::DUEL_SPREAD).clamp(0.1, 0.9);
        let (winner, loser) = if context.rng.unit_f32() < defended {
            (defender, attacker)
        } else {
            (attacker, defender)
        };
        #[cfg(feature = "match-logs")]
        {
            context
                .defender_census
                .borrow_mut()
                .note_award(winner.id, self.current_tick_cached);
            context.forward_census.borrow_mut().note_award(
                winner.id,
                FwdHeaderRule::AerialDuel,
                self.current_tick_cached,
            );
        }

        // A defender's heading state plays the outcome the ball carries, away
        // from his own goal; anyone else's chooses his own header, so the
        // ball is held in the band for him.
        let forward = Vector3::new(winner.side.map_or(1.0, |s| s.forward_dir_x()), 0.0, 0.0);
        let outcome = if winner.tactical_position.current_position.is_defender() {
            AerialOutcome::HeadedAway {
                velocity: Self::headed_along(context, forward),
                receiver: loser.id,
            }
        } else {
            self.aerial_contest_winner = Some(winner.id);
            AerialOutcome::Header {
                drift: Vector3::new(forward.x * 0.12, 0.0, -0.02),
            }
        };
        self.aerial_delivery = Some(AerialDelivery {
            winner_id: winner.id,
            target: self.position,
            outcome,
            arrival_height: self.position.z,
            deadline_tick: self.current_tick_cached + Self::HEADED_DEADLINE,
            force_heading: false,
        });
        self.pending_aerial_strike = Some(winner.id);
    }

    /// A header met along `along`, give or take the duel's spread, at its
    /// pace and loop.
    fn headed_along(context: &MatchContext, along: Vector3<f32>) -> Vector3<f32> {
        let (sin, cos) = context.rng.jitter(0.0, Self::HEADED_SPREAD).sin_cos();
        let headed = Vector3::new(
            along.x * cos - along.y * sin,
            along.x * sin + along.y * cos,
            0.0,
        ) * context
            .rng
            .jitter(Self::HEADED_SPEED, Self::HEADED_SPEED_SPREAD);
        let apex = context
            .rng
            .jitter(Self::HEADED_APEX, Self::HEADED_APEX_SPREAD);
        Vector3::new(headed.x, headed.y, Ball::launch_speed_for_apex(apex))
    }
}

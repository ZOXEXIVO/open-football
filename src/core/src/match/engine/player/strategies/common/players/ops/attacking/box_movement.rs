//! **Attacking a patch of the penalty area** — what an occupant of a
//! [`BoxSlot`] actually does with it, tick by tick.
//!
//! # The problem this exists to solve
//!
//! [`AttackPlan`](crate::r#match::AttackPlan) hands out four places in
//! and around the box, and it hands them out early on purpose:
//! `wants_bodies_forward` is true from `Progression` onward, because "the
//! runs that arrive in the box have to start before the ball reaches the
//! final third or they arrive late". That intent is right. What the
//! engine did with it was not — every off-ball state took the slot
//! coordinate and steered straight at it, so the run did not *start*
//! early, it *finished* early. Measured (`dev_match fwdpath 45 14`), a
//! forward spent **39% of the match within 16.5 m of the opposing goal**,
//! against a real striker's 5-10%, and 3.6 of the 4 slots were filled
//! whenever the plan was live — four men standing in a penalty area while
//! the ball was still being worked out of defence.
//!
//! Two things follow from standing on the spot you mean to finish from,
//! and a defender exploits both immediately:
//!
//! * he is in his marker's field of view for the whole possession, so
//!   there is never a moment the defender has to find him again;
//! * he is stationary when the ball arrives, and the defender is not, so
//!   the defender gets there first.
//!
//! A centre-forward is coached to do the opposite of both. He arrives.
//!
//! # The model
//!
//! One scalar, read from the BALL and therefore identical for all four
//! occupants — which is the point, because what breaks a defensive line
//! is four men going at the same moment along four different lines.
//! [`BoxMovement::stage`] is that scalar, and it stages the occupant
//! through three points:
//!
//! | stage | where he is | what he is doing |
//! |---|---|---|
//! | 0.0 | his anchor in the team block | at the front of the shape, not in the box |
//! | 0.5 | [`BoxSlot::wait_target`] | in the defender's back, working his zone |
//! | 1.0 | [`BoxSlot::target`] | attacking the delivery |
//!
//! On top of the staged point sit two movements that are the difference
//! between occupying a zone and standing in one:
//!
//! * **The work.** A slow ellipse traced around the waiting point, keyed
//!   to the match clock so it is a real cadence rather than a function of
//!   whatever state he happens to be in. Amplitude is his `off_the_ball`
//!   priced against the standard of the match: everybody moves, an elite
//!   mover never stops. It fades out as the delivery comes, because at
//!   that point he is running, not shuffling.
//! * **The marker.** [`MarkerEvasion`] — blind side, seam and the
//!   check-and-spin, scaled by his timing against his marker's
//!   reading of it. That is the contest; the work above is the habit, and
//!   it is what a forward does when nobody is close enough to contest.
//!
//! # Why this owns the steering as well as the target
//!
//! Both `ForwardRunningState` and `ForwardCreatingSpaceState` had their
//! own copy of "steer at the slot, and freeze within 6u of it", with
//! different slowing distances. Two copies of a movement rule is how the
//! engine's off-ball play drifted apart in the first place, so the whole
//! behaviour — target, cadence, evasion and the steering that serves it —
//! lives here and the states ask for a velocity.

use crate::r#match::player::strategies::common::players::ops::marker_evasion::MarkerEvasion;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{BoxSlot, MatchStandard, StateProcessingContext, SteeringBehavior, TeamShape};
use nalgebra::Vector3;

pub struct BoxMovement;

impl BoxMovement {
    /// Attacking progress at which an occupant starts leaving the block
    /// and heading for his patch: the ball is crossing halfway.
    const APPROACH: f32 = 0.48;
    /// …and where the staging is complete — the ball has reached the
    /// final third, so he is in the position he means to finish from.
    /// [`Self::in_flight`] short-circuits to it whenever the ball is
    /// actually travelling.
    ///
    /// # Why this window and not a wider one
    ///
    /// As a coaching instruction the pair reads "hold your line until we
    /// are over halfway, be in the box by the time the ball is in the
    /// final third", which is what it should read. It is also the pair
    /// the **shot mix** wants, and that is the binding constraint: how
    /// much of an attack an occupant spends outside the box decides
    /// where his side's shots come from, so these are titrated against
    /// the distance bands rather than chosen (`OF_BOX_APPROACH` /
    /// `OF_BOX_STRIKE`, `dev_match stats 40 14 14`).
    ///
    /// The first cut was (0.40, 0.85) — start in our own half, arrive at
    /// the byline — and it is too slow at both ends. The occupant is
    /// still walking in when the ball arrives, and the shots move out
    /// with him:
    ///
    /// | window | on-target | 11-16.5 m | 22-30 m | goals |
    /// |---|---|---|---|---|
    /// | unstaged (as it was) | 30.7% | 29.0% | 16.0% | 3.08 |
    /// | (0.40, 0.85) | 27.0% | 25.9% | 20.9% | 2.70 |
    /// | (0.45, 0.72) | 26.7% | 26.5% | 18.0% | 2.72 |
    /// | **(0.48, 0.65)** | **31.4%** | **29.0%** | **16.3%** | **3.11** |
    ///
    /// The engine is already starved close in (2-4% of shots inside 6 m
    /// against a real ~15%) and over-supplied at 16-22 m, so a staging
    /// that pushes occupancy further out is pushing the wrong way. This
    /// window keeps every band where it was and still cuts the camping
    /// in half — which is the whole point: the fix has to be free.
    const STRIKE: f32 = 0.65;

    /// Both ends are `OF_BOX_APPROACH` / `OF_BOX_STRIKE` overridable —
    /// the pair decides how much of an attack an occupant spends outside
    /// the box, and that is a **shot-mix** quantity, not a movement one,
    /// so it has to be titrated against the distance bands rather than
    /// picked. Read once per process.
    #[inline]
    fn approach() -> f32 {
        use std::sync::OnceLock;
        static R: OnceLock<f32> = OnceLock::new();
        *R.get_or_init(|| Self::env_f32("OF_BOX_APPROACH", Self::APPROACH))
    }

    #[inline]
    fn strike() -> f32 {
        use std::sync::OnceLock;
        static R: OnceLock<f32> = OnceLock::new();
        *R.get_or_init(|| Self::env_f32("OF_BOX_STRIKE", Self::STRIKE))
    }

    fn env_f32(key: &str, fallback: f32) -> f32 {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(fallback)
    }

    /// Period of the zone-working cadence, in milliseconds of match
    /// time. ~4 s is a stride out and a stride back — the rhythm a
    /// centre-forward circles his marker at, slow enough that it reads
    /// as movement rather than as jitter.
    ///
    /// **Keyed on the match clock, not on `in_state_time`.** The one
    /// existing cadence in the engine — `MarkerEvasion`'s check-and-spin
    /// — is keyed on time-in-state with a 220-tick period, and a forward
    /// in `CreatingSpace` bounces through `Assisting` roughly every 16
    /// ticks, so its phase never reaches 0.08 of a cycle. The check is
    /// permanent and the spin never happens. A rhythm has to run off a
    /// clock that state churn cannot reset.
    ///
    /// That cadence has a corrected clock of its own now, but it is
    /// gated off pending a chance-economy re-fit — see
    /// `MarkerEvasion::live_cadence`. This one is live, and is why an
    /// occupant keeps moving whether or not anybody is marking him.
    const WORK_PERIOD_MS: u64 = 4000;

    /// How far he works his zone, in game units. Everybody moves — the
    /// base is the floor a professional footballer never goes below —
    /// and the bonus is what separates a striker who is impossible to
    /// mark from one who is easy.
    ///
    /// Sized off what it costs rather than off what looks busiest. The
    /// target traces an ellipse, so its own speed is its perimeter over
    /// `WORK_PERIOD_MS`: at the ceiling below that is ~2 m/s, a jog, and
    /// the occupant only ever chases it from inside `SETTLE_HOLD`, so
    /// `Arrive` serves it at a fraction of his top speed. Twice these
    /// numbers would have him sprinting circles in his own zone, and the
    /// engine already runs its outfielders further than a real match
    /// does.
    const WORK_BASE: f32 = 5.0; // ~0.6 m
    const WORK_SKILL: f32 = 9.0; // …up to ~1.75 m for an elite mover

    /// Slowing distance for the two ends of the stage. Holding a zone is
    /// a settle; attacking a delivery is not, so the arrival gets
    /// sharper as the ball comes.
    const SETTLE_HOLD: f32 = 26.0;
    const SETTLE_ATTACK: f32 = 8.0;

    /// How near the goal the ball has to be, and how fast it has to be
    /// travelling, to count as a delivery already on its way (~26 m,
    /// and any real strike).
    const FLIGHT_RANGE: f32 = 210.0;
    const FLIGHT_SPEED: f32 = 0.35;

    /// **How far along this attack is**, 0..1, read from the ball alone.
    ///
    /// Deliberately carries no term for the player asking, so every
    /// occupant of the box reads the same number on the same tick and
    /// the four runs are simultaneous. A box full of men each breaking
    /// on his own private trigger is a box full of men a defender picks
    /// off one at a time.
    pub fn stage(ctx: &StateProcessingContext) -> f32 {
        if Self::staging_off() {
            return 1.0;
        }
        if Self::in_flight(ctx) {
            return 1.0;
        }
        let Some(side) = ctx.player.side else {
            return 0.0;
        };
        let field_width = ctx.context.field_size.width as f32;
        let ball_x = ctx.tick_context.positions.ball.position.x;
        let progress = side.attacking_progress_x(ball_x, field_width);
        let (approach, strike) = (Self::approach(), Self::strike());
        ((progress - approach) / (strike - approach).max(0.01)).clamp(0.0, 1.0)
    }

    /// A/B control for the staging. With `OF_BOX_STAGE_OFF` set, every
    /// occupant is pinned at stage 1 — he goes straight to the patch he
    /// finishes from and stays there, which is the behaviour this module
    /// replaced, and the cadence fades to nothing with it.
    ///
    /// Same pattern and same purpose as `MatchContext::shape_off`: the
    /// effect reaches every attacking tick of every possession, so "what
    /// did the staging cost or buy?" cannot be answered from a diff, and
    /// the harness's noise floor (±0.15 goals/match over 5+ runs) means
    /// the two arms have to be the same binary. Debug infrastructure —
    /// do not remove.
    fn staging_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_BOX_STAGE_OFF").is_ok())
    }

    /// Is the ball already on its way into the area? Nobody owns a ball
    /// in flight, so this is the window between the cross leaving a boot
    /// and somebody meeting it — the moment every occupant goes.
    fn in_flight(ctx: &StateProcessingContext) -> bool {
        if ctx.ball().is_owned() {
            return false;
        }
        let ball_pos = ctx.tick_context.positions.ball.position;
        let ball_vel = ctx.tick_context.positions.ball.velocity;
        if ball_vel.norm() < Self::FLIGHT_SPEED {
            return false;
        }
        let goal = ctx.player().opponent_goal_position();
        let to_goal = goal - ball_pos;
        if to_goal.magnitude() > Self::FLIGHT_RANGE {
            return false;
        }
        // …and travelling toward that goal rather than away from it.
        to_goal
            .try_normalize(0.01)
            .zip(ball_vel.try_normalize(0.01))
            .is_some_and(|(a, b)| a.dot(&b) > 0.0)
    }

    /// **Where the plan wants this occupant right now** — the staged
    /// point alone, with neither the cadence nor the evasion on top.
    ///
    /// This is the anchor, in the sense
    /// [`my_anchor`](crate::r#match::TeamOperationsImpl::my_anchor) means
    /// it, and it is deliberately smooth: `ShapeDiscipline` tethers a
    /// player to it, so an anchor carrying the shuffle would make the
    /// recall — and the shape census — oscillate with it. The shuffle is
    /// how he occupies the point; the point is where he is meant to be.
    ///
    /// It is also what makes the staging survive at all. The tether reads
    /// `my_anchor` and pulls up to 85% of a player's velocity toward it
    /// from 6 m out, so while that returned the slot coordinate itself
    /// there was no such thing as holding off from the box: any state
    /// that tried was dragged in by the layer above it.
    pub fn hold(ctx: &StateProcessingContext, slot: BoxSlot) -> Vector3<f32> {
        Self::staged_point(ctx, slot, Self::stage(ctx))
    }

    /// Where this occupant should actually be moving to this tick — the
    /// staged point, plus the habit, plus the contest.
    pub fn target(ctx: &StateProcessingContext, slot: BoxSlot) -> Vector3<f32> {
        let stage = Self::stage(ctx);
        let base = Self::staged_point(ctx, slot, stage);
        // `MarkerEvasion` is bounded, so it adjusts the angle he attacks
        // his patch from without ever letting him evade his way out of
        // the assignment.
        MarkerEvasion::evade(ctx, base + Self::work(ctx, stage))
    }

    /// The velocity that serves it. Both off-ball forward states call
    /// this so there is exactly one box-movement behaviour in the
    /// engine.
    ///
    /// **Never returns a hard zero.** The predecessor did, within 6u of
    /// the slot, and a forward standing still in a penalty area is the
    /// one thing this module exists to stop. `Arrive` decelerates into
    /// its target on its own, so a man who is where he wants to be
    /// already has a near-zero velocity — and because the target below
    /// is never stationary, "where he wants to be" keeps moving.
    pub fn steer(ctx: &StateProcessingContext, slot: BoxSlot) -> Vector3<f32> {
        let stage = Self::stage(ctx);
        let target = Self::target(ctx, slot);
        let slowing = Self::SETTLE_HOLD + (Self::SETTLE_ATTACK - Self::SETTLE_HOLD) * stage;
        let out = SteeringBehavior::Arrive {
            target,
            slowing_distance: slowing,
        }
        .calculate(ctx.player)
        .velocity
            * MarkerEvasion::burst(ctx);
        Self::note(ctx, slot, out);
        out
    }

    /// The box-occupancy census (`match-logs` only). Sampled here rather
    /// than at the two call sites so both states are measured by the same
    /// ruler — see `BoxSlotDiag`.
    #[cfg(feature = "match-logs")]
    fn note(ctx: &StateProcessingContext, slot: BoxSlot, out: Vector3<f32>) {
        let goal = ctx.player().opponent_goal_position();
        let field_height = ctx.context.field_size.height as f32;
        let field_width = ctx.context.field_size.width as f32;
        let forward_dir = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
        let ball = ctx.tick_context.positions.ball.position;
        let finish = slot.target(goal, ball.y, field_height, forward_dir);
        crate::mid_run_diag::BoxSlotDiag::note(
            out.norm() < 1e-4,
            out.norm(),
            (finish - ctx.player.position).magnitude(),
            MarkerEvasion::read(ctx).is_some(),
            ctx.players()
                .opponents()
                .all()
                .map(|o| (o.position - ctx.player.position).magnitude())
                .fold(f32::MAX, f32::min),
            ctx.player
                .side
                .map(|s| s.attacking_progress_x(ball.x, field_width))
                .unwrap_or(0.0),
            (goal - ctx.player.position).magnitude(),
            ctx.in_state_time,
        );
    }

    #[cfg(not(feature = "match-logs"))]
    #[inline(always)]
    fn note(_ctx: &StateProcessingContext, _slot: BoxSlot, _out: Vector3<f32>) {}

    /// The three-point staging: block anchor → waiting point → the patch
    /// itself.
    fn staged_point(ctx: &StateProcessingContext, slot: BoxSlot, stage: f32) -> Vector3<f32> {
        let goal = ctx.player().opponent_goal_position();
        let field_height = ctx.context.field_size.height as f32;
        let forward_dir = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
        let ball_y = ctx.tick_context.positions.ball.position.y;

        let wait = slot.wait_target(goal, ball_y, field_height, forward_dir);
        if stage >= 0.5 {
            let finish = slot.target(goal, ball_y, field_height, forward_dir);
            return wait + (finish - wait) * ((stage - 0.5) * 2.0);
        }
        // The far half. His place while the ball is still a long way
        // back is simply the one he would have had without a box slot at
        // all: his touchline if the plan gave him one, otherwise his
        // anchor in the team block, which already accounts for the ball,
        // the phase, the press and the line height.
        //
        // Resolved here rather than by calling `my_anchor` because
        // `my_anchor` resolves the box slot FIRST — asking it would
        // return the very point this staging exists to hold him away
        // from — and the order below is the rest of that same list.
        let hold = Self::far_anchor(ctx).unwrap_or(wait);
        hold + (wait - hold) * (stage * 2.0)
    }

    fn far_anchor(ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let team = ctx.team();
        if let Some(wide) = team.my_width_anchor() {
            return Some(wide);
        }
        let shape: &TeamShape = team.shape();
        shape.anchor_of(ctx.player.id)
    }

    /// The zone-working cadence — a slow ellipse around wherever he is
    /// waiting, fading out as the delivery arrives.
    ///
    /// Two axes a quarter-cycle apart rather than one, so it reads as a
    /// forward circling in his zone instead of sliding along a rail. The
    /// wide axis is lateral, across the marker's face, because that is
    /// the direction that actually costs a defender his picture of the
    /// ball.
    fn work(ctx: &StateProcessingContext, stage: f32) -> Vector3<f32> {
        // He works his zone when he is IN it. A long way behind the ball
        // he is holding a line with the rest of the block and there is
        // nothing to lose a marker for yet; once the ball is on its way
        // he is running, not shuffling. So the cadence ramps in as he
        // arrives and back out as the delivery comes.
        const EDGE: f32 = 0.30;
        let fade = (stage / EDGE).min((1.0 - stage) / EDGE).clamp(0.0, 1.0);
        if fade <= 0.01 {
            return Vector3::zeros();
        }

        // Priced against the standard of football in this match, so a
        // fourth-tier striker works his box like a fourth-tier striker
        // rather than like a statue. See `MatchStandard`.
        let shift = MatchStandard::shift(ctx.context);
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let movement = (sc::n(sc::eff(
            ctx.player,
            sc::EffActionContext::mental(minute),
            |p| p.skills.mental.off_the_ball,
        )) - shift)
            .clamp(0.0, 1.0);
        let amplitude = (Self::WORK_BASE + Self::WORK_SKILL * movement) * fade;

        // Each occupant gets his own phase, or the four of them shuffle
        // in lockstep and the movement carries no information at all.
        // The clock is reduced modulo the period BEFORE it reaches an
        // `f32` — by the 90th minute it is 5.4 million, where a single
        // 10 ms tick is worth about fifteen ULPs of the quotient.
        let offset = (ctx.player.id % 16) as f32 / 16.0;
        let cycle = (ctx.context.total_match_time % Self::WORK_PERIOD_MS) as f32
            / Self::WORK_PERIOD_MS as f32;
        let phase = (cycle + offset).fract() * std::f32::consts::TAU;

        // Lateral is `y` for both sides — the pitch runs along `x`, so
        // across the goal is across the pitch whichever end he attacks.
        Vector3::new(phase.cos() * amplitude * 0.45, phase.sin() * amplitude, 0.0)
    }
}

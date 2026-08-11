use crate::r#match::DefensiveDuty;
use crate::r#match::StateProcessingContext;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor, FIELD_PLAYER_JADEDNESS_INTERVAL,
    JADEDNESS_INCREMENT, LOW_CONDITION_THRESHOLD,
};
use crate::r#match::player::strategies::common::states::TackleEngagement;
use crate::r#match::player::strategies::players::DefensiveRole;
use nalgebra::Vector3;

/// The one rule every defender obeys regardless of which state he is in:
/// **do not let the ball sit behind you.**
///
/// # Why this is a cross-state constraint and not a state
///
/// Line depth used to live in exactly one place — the zonal target in
/// `DefenderHoldingLineState::velocity` — and that state hands off to
/// `Pressing` / `Running` / `Covering` / `Marking` on every
/// `DefensiveRole` branch the moment `opponents().with_ball()` is `Some`.
/// So the only code in the engine that knew about defensive depth was
/// dead in precisely the situation it existed for: opposition
/// possession. The team-shared `defensive_line_x` did not cover for it
/// either — that value is read only as a deviation guard, never as a
/// steering target, and it is computed from game phase alone without
/// ever consulting where the ball is.
///
/// The measured result (`dev_match stats`, `block_diag`): shots struck
/// from 11.4m against a back four sitting at **34.6m** from its own goal,
/// with **0.18** opposition outfielders goal-side of the ball against a
/// real 2-4, and 0.0% of 187k shot-ticks ever finding a defender in the
/// lane. `blocks` read 0.01 per defender per match against a real ~0.9,
/// and every shot in the game was effectively a one-on-one with the
/// keeper — which is also why keeper quality was so far and away the
/// most load-bearing attribute in a squad.
///
/// # The rule
///
/// If the ball is goal-side of me and I am not the man engaging it, my
/// movement must carry me back toward my own goal. The `Primary` role is
/// exempt: somebody has to go to the ball, and that is his job. Everyone
/// else recovers. Lateral movement is untouched, so marking, covering
/// and shuttling all keep working — this only overrides the depth
/// component, which no defender state was managing at all.
///
/// Naturally inert whenever the ball is upfield of the defender, which
/// is every possession spent in the opposition half.
pub struct DefensiveRecovery;

impl DefensiveRecovery {
    /// Base recovery speed, before the pace and recovery multipliers.
    /// See the note at the call site for why this is the line-holding
    /// base rather than the player's nominal max speed.
    const RECOVERY_SPEED: f32 = 3.0;

    /// How far goal-side of the ball a defender is trying to get, in
    /// game units (24u = 3m).
    ///
    /// The rule needs a margin, not just "level with the ball". Without
    /// one it switches off the instant a defender reaches the ball's own
    /// depth, his state's velocity resumes and pulls him back upfield,
    /// and the equilibrium settles ABOVE the ball — measured 19.8m
    /// against a ball at 11.7m, with only 0.27 defenders actually
    /// goal-side. Recovering to a cover point instead puts the
    /// equilibrium where defending happens: between the ball and the
    /// goal, close enough to get a body in the way.
    /// Tuned, not guessed: 14u was measured against 24u and cost
    /// defensive quality for nothing — goal-side presence fell 1.04 →
    /// 0.72 per shot and blocks 0.07 → 0.05, while goals/match sat at
    /// 2.22 either way. The population goals level is set by the rule
    /// EXISTING (defenders being goal-side at all), not by how far
    /// goal-side they get, so there is no trade to make here and the
    /// margin should be the one that defends best.
    const COVER_MARGIN: f32 = 24.0;

    /// The depth (x) velocity this defender must be running at and how
    /// strongly the rule applies, or `None` when it is inert and his
    /// state keeps its own depth.
    ///
    /// Returns only the depth component on purpose: lateral movement is
    /// the state's business — marking, covering and shuttling all keep
    /// working — and depth is the one axis no defender state was
    /// managing at all.
    ///
    /// # Why there is a weight and not just a velocity
    ///
    /// The recovery speed is ~2-4 u/tick against state velocities of
    /// ~0.2-0.4 and a top speed of ~0.28-0.63 — an order of magnitude
    /// larger, deliberately (see the note on `RECOVERY_SPEED`). That is
    /// fine while the rule is ON: the engine-wide clamp scales the result
    /// back to a sprint. What is not fine is that it used to switch on
    /// and off at a hard boundary. Because the override swamps the
    /// state's own vector, the velocity did not adjust at that boundary,
    /// it INVERTED: full sprint toward the ball on one tick, full sprint
    /// toward his own goal on the next.
    ///
    /// And the boundary is exactly where defenders live. `cover_x` sits
    /// 24u goal-side of the ball; a defender recovering to it crosses it,
    /// the rule switches off, his state pulls him back upfield, he
    /// crosses back, and it switches on again. Dumping the raw per-tick
    /// velocities showed precisely this: a chaser 36u from the ball
    /// alternating between `(0.166, 0.226)` — cos +1.0 to the ball — and
    /// `(-0.280, -0.002)` — pure goal-ward, cos -0.64 — at identical
    /// saturated speed, without closing any distance (`dev_match trace`
    /// reversal dumps). Every defender state sat at the top of the
    /// fast-reversal table for this one reason, which is why five
    /// hypotheses aimed INSIDE `DefenderTakeBallState` all measured null:
    /// the velocity was being overwritten after that state returned.
    ///
    /// The weight ramps the rule in across the cover zone instead. A
    /// defender deep out of position still recovers at exactly the speed
    /// he did before — the calibrated end of the behaviour is untouched —
    /// while one hovering near the cover point keeps his own steering and
    /// merely leans goal-ward. There is no longer any position at which
    /// his velocity can jump.
    pub fn depth_override(ctx: &StateProcessingContext) -> Option<(f32, f32)> {
        // The man on the ball is not defending a line.
        if ctx.player.has_ball(ctx) {
            return None;
        }
        let own_goal_x = ctx.ball().direction_to_own_goal().x;
        let me_x = ctx.player.position.x;
        // Unit direction from me toward my own goal, so the test below
        // reads the same on both sides of the pitch.
        let to_goal = (own_goal_x - me_x).signum();

        // WHAT THIS RULE POINTS AT.
        //
        // It used to point at one number: `ball_x ± COVER_MARGIN`, the
        // same for every defender on the side, driven at the same base
        // speed. That is a correct rule about DEPTH and it fixed a real
        // problem — goal-side presence went from 0.18 to 3.28 opposition
        // outfielders per shot — but because the target was shared, four
        // defenders of similar pace converged on one x and then slid
        // together as the ball moved. It is the mechanism behind a back
        // line that moves as one body.
        //
        // The rule now points at the defender's OWN assignment: the
        // goal-side shoulder of the man he is marking, the cover point
        // behind the carrier, or — when he has nobody — the shared ball
        // reference, which is the right answer for a zone-holder and is
        // exactly the previous behaviour. Same ramp, same speeds, same
        // exemptions; only the target differs, so what was measured about
        // this rule still holds while the line stops being rigid.
        // A defender with an assignment recovers to THAT; one without
        // keeps the ball-relative reference, which is what a zone-holder
        // should do and is precisely the old behaviour. The anchor
        // already includes its own goal-side offset (a marker's shoulder,
        // a cover point), so it needs no further margin — adding one on
        // top would stand a marker 4.5 m off his man.
        let ball_x = ctx.tick_context.positions.ball.position.x;
        let duty_anchor_x = ctx.team().my_duty_anchor().map(|a| a.x);
        // Clamped to the pitch. With the ball inside `COVER_MARGIN` of a
        // goal line — every corner, every goalmouth scramble — the raw
        // cover point lands BEHIND the goal line, i.e. off the field. A
        // defender can never reach it, so the rule never goes inert: he
        // is driven into the touchline and held there, pushing outward
        // every tick while the boundary clamp shoves him back.
        //
        // The dump caught exactly that — a defender oscillating across
        // x = 1.6 -> 0.7 -> -0.2 -> 0.2 with the ball 40u away, velocity
        // alternating (-0.457, -0.06) and (0.175, -0.065) (`dev_match
        // trace` reversal dumps). Keeping the cover point on the pitch
        // means arriving at it is possible, which is what lets the rule
        // switch off.
        let field_w = ctx.context.field_size.width as f32;
        let zone_cover_x = ball_x + to_goal * Self::COVER_MARGIN;
        let cover_x = match duty_anchor_x {
            // An assignment must never leave a defender LESS goal-side
            // than the shared rule would have: marking a man who has
            // drifted high cannot be allowed to undo the goal-side
            // guarantee this rule exists to provide.
            Some(x) if (x - zone_cover_x) * to_goal > 0.0 => x,
            _ => zone_cover_x,
        }
        .clamp(0.0, field_w);
        // How far up-field of the cover point I still am. Positive means
        // there is recovering to do; zero or less and the rule is inert.
        let behind = (cover_x - me_x) * to_goal;
        if behind <= 0.0 {
            return None;
        }
        // Ramp the rule in across the depth of the cover zone itself, so
        // no new constant is invented: a defender at the cover point is
        // barely affected, one a full zone up-field is fully committed to
        // the recovery run. Smoothstep so the weight has no corner at
        // either end of the band.
        let t = (behind / Self::COVER_MARGIN).clamp(0.0, 1.0);
        let weight = t * t * (3.0 - 2.0 * t);
        // Somebody has to go to the ball. The plan's designated presser
        // is that man; the legacy geometric `Primary` stays as the
        // fallback for the ticks where no plan is live (a loose ball with
        // no carrier, the first refresh of a possession).
        if matches!(ctx.team().my_duty(), DefensiveDuty::Press)
            || matches!(
                ctx.player().defensive().defensive_role_for_ball_carrier(),
                DefensiveRole::Primary
            )
        {
            return None;
        }
        // Deliberately the same base speed `DefenderHoldingLineState`
        // steers at, NOT `max_speed_with_condition_cached`. The states
        // do not agree on a speed scale — holding the line steers at
        // 3.0, covering at `pace * 0.9` (~12), and both exceed the
        // player's nominal max speed, which only the `SteeringBehavior`
        // paths clamp against. Recovering at the nominal max therefore
        // makes a defender uniquely slower than every attacker he is
        // chasing: measured, it dropped goal-side presence from 1.06 to
        // 0.90 per shot. Matching the line-holding base keeps him on the
        // same scale as the players around him. `recovery_run_mult` and
        // the pace term still make a quick defender genuinely quicker.
        let profile = DefenderSkillProfile::from_ctx(ctx);
        let pace = (ctx.player.skills.physical.pace / 20.0).clamp(0.6, 1.2);
        Some((
            to_goal * Self::RECOVERY_SPEED * pace * profile.recovery_run_mult,
            weight,
        ))
    }
}

/// The back line, as one shared piece of geometry.
///
/// # Why this exists
///
/// `HoldingLine` and `Running` both answer the question "is this defender
/// in the line?", and they used to answer it with **different
/// quantities**. `DefenderHoldingLineState` left for `Running` when the
/// defender was more than 35u from the computed line x;
/// `DefenderRunningState::phase_dispatch` sent him straight back to
/// `HoldingLine` whenever `position_to_distance() != Big`, i.e. based on
/// distance from his *start position*. A defender can trivially satisfy
/// both at once — near his start, far from where the line has drifted —
/// and every such defender then ran the two-cycle
/// `HoldingLine -> Running -> HoldingLine` at AI-tick cadence, forever.
///
/// It was the single largest source of state churn in the engine:
/// 152,000 transitions each way in ONE match, with 98.4% of `HoldingLine`
/// visits lasting a single tick (`dev_match trace`). The back four never
/// held a line and never committed to a recovery run; they alternated
/// between the two ~9 times a second.
///
/// # The rule
///
/// One quantity — distance from the line — with hysteresis. A defender
/// must actually arrive (`ENTER_BAND`) before he counts as part of the
/// line, and once he is part of it he keeps holding until he genuinely
/// drifts out (`EXIT_BAND`). The gap between the two bands is the
/// commitment: inside it, whatever the defender is already doing is the
/// right answer, which is exactly how a real back four behaves — you
/// don't join or abandon a line over a metre.
pub struct DefensiveLine;

impl DefensiveLine {
    /// Drift that pulls a settled defender back out of the line.
    /// Unchanged from `MAX_DEFENSIVE_LINE_DEVIATION`, so a defender
    /// leaves the line exactly when he did before.
    pub const EXIT_BAND: f32 = 35.0;
    /// How close a defender must get before he counts as having joined
    /// the line. Comfortably inside `EXIT_BAND` — the difference is the
    /// hysteresis that makes the pair of states agree.
    pub const ENTER_BAND: f32 = 20.0;

    /// The x-coordinate of the line the back four is currently holding.
    ///
    /// Blends the live back-line average with the team-shared, phase-aware
    /// `defensive_line_x` rather than overwriting it, so one recovering
    /// defender can't teleport the whole line's reference point.
    pub fn position_x(ctx: &StateProcessingContext) -> f32 {
        let (sum_x, count) = ctx
            .players()
            .teammates()
            .defenders()
            .map(|p| p.position.x)
            .fold((0.0f32, 0u32), |(s, c), x| (s + x, c + 1));

        let avg_x = if count > 0 {
            sum_x / count as f32
        } else {
            ctx.player.position.x
        };

        let target_line_x = ctx
            .context
            .tactical_for_team(ctx.player.team_id)
            .defensive_line_x;
        avg_x * 0.6 + target_line_x * 0.4
    }

    /// How far this defender is off the line, along the goal-to-goal axis.
    pub fn deviation(ctx: &StateProcessingContext) -> f32 {
        (ctx.player.position.x - Self::position_x(ctx)).abs()
    }

    /// Is this defender part of the line right now?
    ///
    /// Hysteretic on the defender's CURRENT state, which is what makes
    /// this safe to call from both sides of the hand-off: a defender
    /// already holding gets the generous `EXIT_BAND`, one still running
    /// back has to reach `ENTER_BAND` first. Both states therefore read
    /// the same predicate and can never contradict each other.
    pub fn is_in_line(ctx: &StateProcessingContext) -> bool {
        let holding = matches!(
            ctx.player.state,
            crate::r#match::player::state::PlayerState::Defender(DefenderState::HoldingLine)
        );
        let band = if holding {
            Self::EXIT_BAND
        } else {
            Self::ENTER_BAND
        };
        Self::deviation(ctx) <= band
    }
}

/// Defender-specific activity intensity configuration
pub struct DefenderConfig;

impl ActivityIntensityConfig for DefenderConfig {
    fn very_high_fatigue() -> f32 {
        8.0 // Explosive actions tire quickly
    }

    fn high_fatigue() -> f32 {
        5.0 // Base from running state
    }

    fn moderate_fatigue() -> f32 {
        3.0
    }

    fn low_fatigue() -> f32 {
        1.0
    }

    fn recovery_rate() -> f32 {
        -3.0
    }

    fn sprint_multiplier() -> f32 {
        1.5 // Sprinting
    }

    fn jogging_multiplier() -> f32 {
        0.6
    }

    fn walking_multiplier() -> f32 {
        0.3
    }

    fn low_condition_threshold() -> i16 {
        LOW_CONDITION_THRESHOLD
    }

    fn jadedness_interval() -> u64 {
        FIELD_PLAYER_JADEDNESS_INTERVAL
    }

    fn jadedness_increment() -> i16 {
        JADEDNESS_INCREMENT
    }
}

/// Defender condition processor (type alias for clarity)
pub type DefenderCondition = ConditionProcessor<DefenderConfig>;

// Re-export for convenience
pub use crate::r#match::engine::player::strategies::common::ActivityIntensity;

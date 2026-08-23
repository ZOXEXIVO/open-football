use crate::r#match::DefensiveDuty;
use crate::r#match::StateProcessingContext;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor, FIELD_PLAYER_JADEDNESS_INTERVAL,
    JADEDNESS_INCREMENT, LOW_CONDITION_THRESHOLD,
};
use crate::r#match::player::strategies::RestartHold;
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

/// Holding a position that is itself moving.
///
/// # Why this exists
///
/// `DefenderCoveringState` and `DefenderGuardingState` both end their
/// steering with a special case for "I have arrived", and both wrote it
/// as `opponent_velocity * 0.4` / `* 0.5` — travel the way my man is
/// travelling, at half his speed.
///
/// That is a defender being left behind, by construction. Whatever gap he
/// had at the moment he arrived grows at half the carrier's speed for as
/// long as he holds the point, and no amount of arriving correctly
/// survives it. It is also, exactly, the thing reported from the viewer:
/// *"they run parallel to the movement and don't try to take the ball"* —
/// written into two states as their terminal behaviour.
///
/// A man holding station on a moving one matches his velocity and
/// corrects whatever error is left. The correction is a gain on the error
/// itself, so it vanishes as he settles and there is no distance at which
/// his velocity changes character; the engine-wide speed clamp is the
/// only ceiling it needs.
pub struct StationKeeping;

impl StationKeeping {
    /// How hard the residual error is corrected, per tick.
    ///
    /// The error here is bounded by the states' own 2u arrival radius, so
    /// this contributes at most 0.3 u/tick — the same order as a
    /// defender's top speed (0.28-0.63) and therefore able to close the
    /// last stride, while being negligible once he is on the point.
    const GAIN: f32 = 0.15;

    /// The velocity that keeps a defender on a point moving with `man`.
    /// `error` is the vector from the defender to where he wants to be.
    pub fn hold(man_velocity: Vector3<f32>, error: Vector3<f32>) -> Vector3<f32> {
        man_velocity + error * Self::GAIN
    }
}

pub struct DefensiveRecovery;

/// Is there a ball here that could actually be intercepted?
///
/// `DefenderInterceptingState` splits a ball nobody is carrying into a
/// claim (`TakeBall`) or a chase, and hands anything else straight to
/// `Tackling` or `Pressing`. Eleven separate entry points each drew
/// their own condition for going in, most of them without asking whether
/// anybody had the ball at all — so the state was routinely entered for
/// one it would refuse on the next line, and spent its life as a
/// one-tick pass-through: 11,328 exits across three matches at a mean
/// dwell of 1.4 AI ticks, 95.5% of visits lasting a single tick.
///
/// One predicate mirroring the state's own contract is the only version
/// that cannot drift out of step with it. Same fix as the midfielder and
/// forward trees.
pub struct Interception;

impl Interception {
    pub fn is_available(ctx: &StateProcessingContext) -> bool {
        !ctx.player.has_ball(ctx) && !ctx.ball().is_owned()
    }
}

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
        //
        // ⚠ …AND UNTIL NOW THAT SENTENCE WAS ONLY TRUE OF A BALL SOMEBODY
        // WAS CARRYING.
        //
        // Both tests above are silent on a LOOSE ball.
        // `compute_defensive_role_for_ball_carrier` opens with
        // `let Some(ball_carrier) = opponents().with_ball().next() else {
        // return DefensiveRole::Hold }`, and `DutyAssigner` nominates a
        // `Press` against a live point of attack. So for every delivery
        // in flight, every rebound and every ball dropping into our own
        // area — the half of a match nobody owns the ball — NOBODY was
        // exempt, including the one man the loose-ball election had just
        // sent to win it. His state steers him at the meeting point, this
        // rule then overwrote the DEPTH half of that with a sprint at his
        // own goal, and what survived was the lateral half: he tracked
        // across to the ball's line and slid past it toward the goal.
        //
        // Reported from the viewer exactly that way — *"a pass is played
        // into the penalty area where there are only defenders; they
        // don't try to get the ball, they simply run towards the goal,
        // and the opponent takes it and scores"* — and measured with the
        // BOX-DELIVERY CENSUS (`mid_run_diag::BOXBALL_SAMPLES`), 120
        // fixtures at L14: with a loose ball in our own area the nearest
        // defender is 3.4 m off the drop and is pointed more at his own
        // goal than at the ball on **41% of ticks**, rising to **47%** on
        // the samples where he is the nearest man to it and no attacker
        // is closer. **71% of those samples are a defender in `TakeBall`**
        // — the state whose entire job is to go and get it — running
        // goalward on 45% of them.
        //
        // Note what the rule points at: `cover_x` is 24u GOAL-SIDE of the
        // ball. For a man who is going to win it, "three metres past it
        // toward my own goal" is not a cover point, it is the wrong side
        // of the ball.
        if matches!(ctx.team().my_duty(), DefensiveDuty::Press)
            || matches!(
                ctx.player().defensive().defensive_role_for_ball_carrier(),
                DefensiveRole::Primary
            )
            || Self::is_going_for_a_loose_ball(ctx)
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

    /// Am I the man my side has sent after a ball nobody has?
    ///
    /// The same lexicographic election `should_force_takeball` uses to
    /// redirect exactly one player per side into `TakeBall`, asked
    /// through the one shared predicate ([`LooseBallChase::is_designated`])
    /// so the two cannot drift apart. Measured against the ball's LANDING
    /// position for the same reason that election is: a delivery is
    /// contested at the drop, not at its apex.
    ///
    /// Deliberately the ELECTION and not the `TakeBall` state. Exempting
    /// the state outright is recorded as tried and reverted — it is
    /// long-lived, so the goal-side rule switched off for most of a
    /// defender's match (goal-side presence per shot 1.05 → 0.82,
    /// clearances 1.64 → 8.14 against a real ~3.5). The election is one
    /// man, only while the ball is genuinely loose, and it lapses on the
    /// tick somebody claims it.
    ///
    /// The restart guard mirrors `should_force_takeball`'s: a ball out of
    /// play may only be touched by its taker, so nobody else is "going
    /// for it" however near he is standing.
    ///
    /// ⚠ AND IT HAS TO BE **THEIR** BALL.
    ///
    /// Nothing in this rule asks whose possession it is — it fires
    /// whenever the ball is goal-side of a defender, our own build-up
    /// included. That is pre-existing, and the whole population is
    /// calibrated on top of it, so widening the exemption to a pass
    /// between two of our own defenders changes something this report is
    /// not about. Measured over 120 fixtures: exempting the receiver of
    /// our own pass took **pass accuracy 85.5% → 95.8%** and passes
    /// 920 → 1 195 a team, because a centre-half receiving a square ball
    /// had been running away from it. That is a real defect and a
    /// separate piece of work; it is not this one.
    ///
    /// The report is about winning a ball BACK, so the exemption is
    /// scoped to a ball whose last touch was not one of ours.
    fn is_going_for_a_loose_ball(ctx: &StateProcessingContext) -> bool {
        if Self::loose_recovery_legacy() {
            return false;
        }
        if ctx.tick_context.ball.is_owned {
            return false;
        }
        let theirs = ctx.tick_context.ball.last_owner.is_none_or(|id| {
            ctx.context
                .players
                .by_id(id)
                .is_none_or(|p| p.team_id != ctx.player.team_id)
        });
        if !theirs {
            return false;
        }
        let Some(side) = ctx.player.side else {
            return false;
        };
        if let Some(taker) = RestartHold::taker(ctx.tick_context) {
            return taker == ctx.player.id;
        }
        let drop = ctx.tick_context.positions.ball.landing_position;
        let my_dist_sq = (drop - ctx.player.position).norm_squared();
        if !ctx
            .tick_context
            .chase
            .is_designated(side, ctx.player.id, my_dist_sq)
        {
            return false;
        }
        // …AND IT HAS TO BE A BALL HE CAN ACTUALLY WIN.
        //
        // Being my side's nearest man is not the same as being the
        // nearest man. If an opponent is going to get there first I am
        // not going for the ball at all, I am defending the space behind
        // it — which is precisely what this rule is for, so it must keep
        // running.
        //
        // Without this the exemption fires on every pass in flight
        // anywhere on the pitch, because somebody on the defending side
        // is always the nearest of his eleven. Measured over 3×300
        // fixtures against `OF_LOOSE_RECOVERY_LEGACY`, that unscoped
        // version reads as a faster, looser match on every axis — goals
        // 2.52 → 2.82, shots 13.5 → 15.2 a team, pass accuracy 85.5% →
        // 89.2% — with the defensive geometry itself barely moving
        // (goal-side at the strike 2.82 → 2.71, back line 123u → 123u).
        // It was not costing compactness; it was taking a defender out
        // of shape 60 m from a ball he was never going to reach.
        //
        // The striker gamble in [`LooseBallChase::chase_dist_sq`] is on
        // both sides of this comparison, which is right: a forward really
        // does read a rebound earlier, and a defender who knows the
        // striker will beat him to it should be dropping off.
        let theirs_closer = ctx
            .tick_context
            .chase
            .best(side.opposite())
            .is_some_and(|best| best.dist_sq < my_dist_sq);
        !theirs_closer
    }

    /// Diagnostic switch: with `OF_LOOSE_RECOVERY_LEGACY` set, the
    /// goal-side rule reverts to overriding the depth of the man his side
    /// has sent after a loose ball — the behaviour the BOX-DELIVERY
    /// CENSUS measured at 41% goalward.
    ///
    /// This is the A/B control for that work. It reaches every loose ball
    /// on every tick, so the effect cannot be read off a diff, and it must
    /// not be answered by checking out an older revision either — the
    /// working tree moves underneath you. Same pattern and purpose as
    /// `LooseBallChase::tail_chase` and `MovementEffort::chase_legacy`;
    /// read once per process. Debug infrastructure — do not remove.
    pub fn loose_recovery_legacy() -> bool {
        use std::sync::OnceLock;
        static LEGACY: OnceLock<bool> = OnceLock::new();
        *LEGACY.get_or_init(|| std::env::var("OF_LOOSE_RECOVERY_LEGACY").is_ok())
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
        // Average only the defenders who are actually IN the line.
        //
        // This averaged every defender on the team, and a centre-half who
        // has gone up for a corner is at the other end of the pitch — one
        // of them at x≈800 against three at x≈100 drags the average to
        // 275 and hands the whole back line a reference two-thirds of the
        // way up the field. (Attacking corners are not rare here: the
        // back line spends thousands of seconds a match in that state.)
        // The same applies to a full-back who has pushed on.
        //
        // A positional filter rather than a state one, because the states
        // that take a defender out of the line are many and the property
        // that matters is the same for all of them: he is nowhere near
        // it. 0.60 of the pitch is past halfway, so a genuinely high line
        // still counts in full; only men who have gone up are excluded.
        let field_len = ctx.context.field_size.width as f32;
        let forward = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
        let upfield = |x: f32| if forward > 0.0 { x } else { field_len - x };
        let line_limit = field_len * 0.60;
        // ⚠ THE BACK LINE IS THE BACK FOUR, and `defenders()` is not.
        //
        // `is_defender()` is the POSITION GROUP, and that group holds
        // `DefensiveMidfielder` — a man whose job is to sit ten to fifteen
        // metres IN FRONT of the back four. Averaging him into the line
        // reference does two things at once, both wrong: it drags the
        // reference upfield so the back four is clamped to a line that is
        // not theirs, and it then clamps the DM himself to within 2 m of
        // that same line, so the screen in front of the back four is
        // vacated by construction — which is precisely the band (11-22 m)
        // the shot mix says the chances come from.
        //
        // The harness's own back-line spread sampler already excludes DMs
        // for exactly this reason (`tick.rs`); this is the same exclusion
        // on the side that does the constraining, so the measurement and
        // the mechanism finally describe the same four players.
        let (sum_x, count) = ctx
            .players()
            .teammates()
            .defenders()
            .filter(|p| !p.tactical_positions.is_defensive_midfielder())
            .filter(|p| upfield(p.position.x) <= line_limit)
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
        // 0.6/0.4 → 0.85/0.15, because this is a DESCRIPTION, not an
        // instruction.
        //
        // `position_x` answers "where is the line", and every shape
        // constraint bounds defenders relative to it. `defensive_line_x`
        // answers something else — "how high does the phase want to
        // play" — and it is a constant per phase (mid block 280u, attack
        // 462u) that never reads the ball. Defenders sit ~215u from it,
        // so blending it at 0.4 put the shape reference 86u from the men
        // it describes and left it there: measured depth lag was 85-102u
        // all match while the width axis, which has no such aspirational
        // term, sat at 18u. 0.4 × 215u = 86u — the lag WAS this blend.
        //
        // The reference is also partly the thing being corrected, so the
        // fixed point of the old weighting required the whole line to
        // travel the entire distance to the tactical line before the lag
        // closed. It never did, because the ball kept it where it was.
        //
        // A small pull is kept: the aspiration should still bias the
        // line, and the live average is what stops one recovering
        // defender teleporting the reference (helped now that men who
        // have gone up are excluded from it). Pushing UP to the phase
        // line is a movement decision that belongs to `PushingUp`, not
        // something to be smuggled into the shape reference.
        let blended = avg_x * 0.85 + target_line_x * 0.15;

        // ── A line cannot hold station in front of the ball ───────────
        //
        // `defensive_line_x` is a constant per PHASE — a mid block is
        // 280u (35 m from our own goal) whether the ball is on the
        // halfway line or in our six-yard box. Blended at 0.4 it dragged
        // the whole shape reference upfield of wherever the danger
        // actually was, and every defender was then bounded relative to
        // that: measured, defenders sat 98u (12.3 m) from their shape
        // target on the DEPTH axis while the width axis was only 19u out.
        // They were not failing to steer — they were steering, all match,
        // at a line that had been placed in front of them.
        //
        // No back four defends a ball in its own third from thirty-five
        // metres. Phase sets how high the line WANTS to be; the ball sets
        // how high it may actually be, and the ball wins in our own half.
        // A couple of metres of licence keeps the offside line playable
        // rather than pinning the line exactly on the ball.
        if ctx.ball().on_own_side() {
            let forward = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
            let ball_x = ctx.tick_context.positions.ball.position.x;
            let ahead_of_ball = (blended - ball_x) * forward;
            if ahead_of_ball > Self::LINE_AHEAD_OF_BALL {
                return ball_x + Self::LINE_AHEAD_OF_BALL * forward;
            }
        }
        blended
    }

    /// How far this defender is off the line, along the goal-to-goal axis.
    pub fn deviation(ctx: &StateProcessingContext) -> f32 {
        (ctx.player.position.x - Self::position_x(ctx)).abs()
    }

    /// How far a defender may stray LATERALLY from his compact slot while
    /// doing an individual job. 60u = 7.5 m — enough to track a man
    /// through his own zone and into the edge of his neighbour's, not
    /// enough to follow him across the pitch and take the line with him.
    pub const SHAPE_LEASH: f32 = 60.0;
    /// How far a defender holding shape may drop BEHIND the line to
    /// cover. 48u = 6 m — the covering centre-half sitting off the man
    /// engaging, which is what makes a back four a diagonal.
    /// Tightened 48u → 34u. The band `DEPTH_DROP + DEPTH_STEP_UP` is the
    /// spread the line is ALLOWED, and the defenders' residual lag adds
    /// to it on top — at 64u of band the measured spread could not have
    /// come under ~130u however well anyone steered. 34u (4.25 m) is the
    /// real distance a covering centre-half sits off the man engaging.
    pub const DEPTH_DROP: f32 = 34.0;
    /// …and how far he may step UP through it. 16u = 2 m: essentially
    /// nothing. A defender in front of his own line plays the attack
    /// onside behind him.
    pub const DEPTH_STEP_UP: f32 = 16.0;
    /// How far upfield of the ball the line may sit while the ball is in
    /// our own half. 16u = 2 m — enough to keep an offside line playable,
    /// not enough to leave the back four in front of the danger.
    pub const LINE_AHEAD_OF_BALL: f32 = 16.0;

    /// This defender's lateral anchor in the unit's CURRENT shape.
    ///
    /// The line has always had a shared reference for its depth
    /// (`position_x`) and never one for its width, and that asymmetry is
    /// the whole of the defensive-shape problem. Depth is a team quantity
    /// that every state respects; width was left to each defender's
    /// `start_position` — his KICKOFF slot — so the back four defended
    /// its own six-yard box at the width it lines up in for kick-off.
    ///
    /// A defending back four squeezes toward its own centre as the ball
    /// comes at it: the far-side full-back tucks in, and a unit spanning
    /// 50 m at kick-off defends its box across barely 30. That is what
    /// this returns — the kickoff slot pulled toward the middle by how
    /// deep the danger is and how compact the side wants to be.
    pub fn compact_slot_y(ctx: &StateProcessingContext) -> f32 {
        let centre_y = ctx.context.field_size.height as f32 / 2.0;
        let field_len = ctx.context.field_size.width as f32;
        let ball_x = ctx.tick_context.positions.ball.position.x;
        let own_goal_x = ctx.ball().direction_to_own_goal().x;
        let danger = (1.0 - (ball_x - own_goal_x).abs() / field_len.max(1.0)).clamp(0.0, 1.0);
        let compactness = ctx.team().compactness_target();
        // 0.20/0.25 left the widest adjacent gap at 15.2 m against a real
        // 3-8 m — the direction was right and the magnitude was half what
        // it needed to be. At full danger and compactness a back four that
        // lines up across 50 m now defends its own box across ~27, which
        // is what a deep block actually looks like.
        let squeeze = 1.0 - (0.30 + compactness * 0.35) * danger;
        centre_y + (ctx.player.start_position.y - centre_y) * squeeze
    }

    /// Is `target` inside our own penalty area? Real dimensions at
    /// 0.125 m/unit: 16.5 m deep (132u) and 20.16 m either side of the
    /// centre (161u).
    fn target_in_own_box(ctx: &StateProcessingContext, target: Vector3<f32>) -> bool {
        const BOX_DEPTH: f32 = 132.0;
        const BOX_HALF_WIDTH: f32 = 161.0;
        let own_goal = ctx.ball().direction_to_own_goal();
        let centre_y = ctx.context.field_size.height as f32 / 2.0;
        (target.x - own_goal.x).abs() <= BOX_DEPTH && (target.y - centre_y).abs() <= BOX_HALF_WIDTH
    }

    /// Constrain a target a state chose for its own reasons so that the
    /// UNIT keeps its shape.
    ///
    /// Individual duties are why the shape breaks: `Marking` goes to a
    /// man and `Running` goes to the ball, and between them they are 62%
    /// of everything the back line does. Neither refers to the line at
    /// all, so the compactness logic — which lives in `HoldingLine`,
    /// where the back four spends 6% of its time — governs almost
    /// nothing. Measured: 18-19 m between adjacent defenders against a
    /// real 3-8 m, and 96% of the defenders inside the block window when
    /// a shot is struck are wider than the corridor.
    ///
    /// This does not replace the state's decision — the defender still
    /// goes to his man — it bounds how far that job may drag him off his
    /// slot. A man who runs further than the leash belongs to the next
    /// defender along, which is what a zone is.
    ///
    /// The presser is deliberately exempt: somebody has to leave the line
    /// to engage the ball, and he is the one doing it.
    pub fn hold_shape(ctx: &StateProcessingContext, target: Vector3<f32>) -> Vector3<f32> {
        Self::hold_shape_inner(ctx, target, None)
    }

    /// The same constraint, for a defender who has been given a MAN.
    ///
    /// # Why marking needs its own bound
    ///
    /// The zonal leash is right about runners and wrong about assignments,
    /// and the measurement is unambiguous: sampled in `Marking::velocity`
    /// over 60 matches, the marker's OWN target sits **0.77 m** from his
    /// man — exactly the `ideal_marking_distance` the profile asks for —
    /// and `hold_shape` then pushes it out to **3.71 m**. Against a
    /// midfielder it pushes it to **6.78 m**. The whole of the reported
    /// marking gap is this displacement; nothing about the marking model
    /// itself was wrong.
    ///
    /// The binding half is DEPTH, not width. `SHAPE_LEASH` allows 7.5 m
    /// either side, which is enough to follow a man across; `DEPTH_STEP_UP`
    /// allows **2 m** in front of the line, so a midfielder loitering eight
    /// metres ahead of the back four cannot be marked at all — by
    /// construction, and precisely in the band the shot mix says the
    /// chances come from.
    ///
    /// The step-up bound exists because a defender ahead of his line plays
    /// everyone behind him onside. That argument does not apply to a man
    /// going to his own marker's position: the marking offset is
    /// GOAL-SIDE of the man, so he can never get further forward than the
    /// man he is marking, and the man is already setting the line. So the
    /// allowance becomes "as far up as your man, no further" — which is
    /// what a defender given an assignment actually does, and is offside-
    /// safe by construction.
    pub fn hold_shape_on_man(
        ctx: &StateProcessingContext,
        target: Vector3<f32>,
        man: Vector3<f32>,
    ) -> Vector3<f32> {
        Self::hold_shape_inner(ctx, target, Some(man))
    }

    fn hold_shape_inner(
        ctx: &StateProcessingContext,
        target: Vector3<f32>,
        man: Option<Vector3<f32>>,
    ) -> Vector3<f32> {
        if ctx.player.has_ball(ctx) {
            return target;
        }
        if matches!(ctx.team().my_duty(), DefensiveDuty::Press) {
            return target;
        }
        // ── …and inside our own box, the MAN beats the shape ──────────
        //
        // Everything above this line is a zonal argument, and a zonal
        // argument stops being the right one at the edge of your own
        // penalty area: that is where every defence in football goes
        // man-to-man, because the cost of losing your man is a goal
        // rather than a gap.
        //
        // Left in, the leash is what produced the point-blank supply.
        // A runner arriving from deep drags his marker to the edge of a
        // 60u lateral / 34u depth box drawn around a line that sits ~8 m
        // off the goal — and there the marker STOPS, while the man he is
        // marking carries on into the six-yard box and receives free.
        // Measured: markers sat 5.2 m off their man against an
        // `ideal_marking_distance` of 0.9-1.75 m, midfield runners 9.7 m
        // off, 47% of attackers in our third with nobody within 3 m, and
        // 34% of all shots struck from inside 6 m against a real 15%.
        //
        // Keyed to where the TARGET is, not to where the defender is, so
        // it releases him to follow a man in rather than releasing anyone
        // who happens to be standing in his own area.
        if Self::target_in_own_box(ctx, target) {
            return target;
        }
        let slot = Self::compact_slot_y(ctx);
        let leashed = target
            .y
            .clamp(slot - Self::SHAPE_LEASH, slot + Self::SHAPE_LEASH);

        // ── DEPTH, and why it is ASYMMETRIC ──────────────────────────
        //
        // Width was the missing team quantity; depth had one all along
        // (`position_x`) that the individual-duty states simply ignored,
        // and the result is the same shape of failure — 18 m of depth
        // spread across a back four against a real 3-8 m.
        //
        // The two directions are not equivalent, which is why an
        // even clamp would be wrong. Dropping OFF the line is ordinary
        // cover: the spare man sits behind whoever is engaging, and a
        // back four is a diagonal precisely because of it. Pushing UP
        // through the line is not — a defender ahead of his line plays
        // everyone onside behind him and is the one thing a back four
        // never does by accident. So he may drop a long way and step up
        // barely at all, and the two together bound the spread at
        // ~8 m, which is the real number.
        //
        // Applied here rather than per-state deliberately: a previous
        // attempt pulled only `Running` toward the line average while
        // `Marking` was still following its man's depth, and the spread
        // got WORSE (137u → 174u) because the two groups were being
        // pulled to different references. Depth only converges if
        // everybody holding shape reads the same one.
        let forward = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
        let line_x = Self::position_x(ctx);
        let ahead_of_line = (target.x - line_x) * forward;
        // "As far up as your man, no further" — see `hold_shape_on_man`.
        // Never TIGHTER than the ordinary allowance: a man behind the line
        // does not drag his marker backwards out of it.
        // …but bounded. Unbounded, "go to your man" lets a centre-half
        // follow a midfielder as far as the assignment reaches, and the
        // back-four depth spread measured 13.2 m → 15.0 m when it was
        // first tried. A defender goes and gets a man in front of him; he
        // does not leave the last line to do it. 72u = 9 m is the screen
        // in front of the back four, which is as far up as a member of it
        // has any business being.
        const MARK_STEP_UP_CAP: f32 = 72.0;
        let step_up = man.map_or(Self::DEPTH_STEP_UP, |m| {
            Self::DEPTH_STEP_UP
                .max((m.x - line_x) * forward)
                .min(MARK_STEP_UP_CAP)
        });
        let bounded = ahead_of_line.clamp(-Self::DEPTH_DROP, step_up);
        let shaped = Vector3::new(line_x + bounded * forward, leashed, target.z);
        #[cfg(feature = "match-logs")]
        {
            let d = shaped - ctx.player.position;
            crate::r#match::player::strategies::players::ops::forward_shot_decision::mid_run_diag::DefenceDiag::note_shape_lag(
                d.magnitude(),
                ctx.in_state_time,
                d.x.abs(),
                d.y.abs(),
            );
        }
        shaped
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

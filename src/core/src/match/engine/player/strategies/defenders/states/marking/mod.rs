use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::shared::{
    ActivityIntensity, DefenderCondition, DefenderRead, DefensiveLine, LineStep, ReadLevel,
};
use crate::r#match::events::Event;
use crate::r#match::player::events::{FoulCommitted, FoulSource, PlayerEvent};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::shared::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::shared::players::ops::marker_evasion::MarkerEvasion;
use crate::r#match::player::strategies::shared::states::{
    ContactFoul, MarkEngagement, StationKeeping,
};
use crate::r#match::player::strategies::shared::team::KeeperVoice;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct DefenderMarkingState {}

impl StateProcessingHandler for DefenderMarkingState {
    /// Marking his man: the one the plan gave him or, with none, the most
    /// dangerous man in his zone the plan has given nobody (see
    /// `DefenderRead::man`). He gives him up beyond `MarkEngagement::RELEASE`
    /// and to every door above the mark.
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if let Some(change) = DefenderRead::change_above(ctx, ReadLevel::Mark) {
            return Some(change);
        }
        let Some(man) = DefenderRead::man(ctx) else {
            return Some(DefenderRead::change(ctx));
        };
        let gap = man.distance(ctx);
        if gap > MarkEngagement::RELEASE {
            return Some(DefenderRead::change(ctx));
        }
        // The shirt pull. A marker who is being pulled away from grabs,
        // leans or blocks — the commonest foul in football. See
        // `ContactFoul`.
        let losing_him = man.velocity(ctx).norm() > ctx.player.velocity.norm() + 0.08;
        if ContactFoul::commits_now(ctx, gap, losing_him) {
            return Some(StateChangeResult::with_defender_state_and_event(
                DefenderState::HoldingLine,
                Event::PlayerEvent(PlayerEvent::CommitFoul(FoulCommitted {
                    fouler: ctx.player.id,
                    fouled: man.id,
                    severity: ContactFoul::severity(ctx, losing_him),
                    source: FoulSource::Holding,
                    challenge: None,
                })),
            ));
        }
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Profile-driven marking: ideal distance and goal-side weight
        // both scale with marking_profile / positioning curve, so an
        // elite defender stands closer and more goal-side, while a
        // poor marker stays at arm's length and gets shrugged off runs.
        if let Some(opponent_to_mark) = DefenderRead::man(ctx) {
            let def_profile = DefenderSkillProfile::from_ctx(ctx);
            // Switched off for a moment as his man goes: he carries on as
            // he was rather than following the run.
            if def_profile.late_to_the_run(ctx, &opponent_to_mark) {
                return Some(ctx.player.velocity);
            }
            let own_goal = ctx.ball().direction_to_own_goal();
            let opponent_velocity = opponent_to_mark.velocity(ctx);

            // READING THE RUN — signed, and the defender's own property.
            //
            // This was a flat `0.3`: every marker in the game predicted
            // his man's movement forward, perfectly, whoever he was. A
            // pursuer who re-aims at where you are GOING every tick
            // cannot be beaten by movement, which is why the attacking
            // side's evasion work moved attacker separation not at all —
            // 5.0 m and 56% unmarked before and after, at every
            // amplitude tried.
            //
            // Real marking is a contest of reading. A defender who reads
            // it arrives before you do; one who doesn't is a step behind
            // and gets spun, and a check-and-spin beats him precisely
            // BECAUSE he extrapolated the first movement. Signing this
            // quantity is what makes that true: lead for the good
            // reader, trail for the poor one.
            //
            // ── THE READ IS A DUEL, NOT A PROPERTY ────────────────────
            //
            // It was `(reading − 0.5)` on a blend that did not include
            // `marking` — so the tracker's read was priced against an
            // absolute constant, and the MAN being tracked contributed
            // nothing: an 18-off_the_ball mover degraded his marker's
            // extrapolation exactly as much as a 6, which is why every
            // evasion-amplitude sweep measured zero separation and the
            // 2026-08-31 definitive sweep still had off_the_ball leaning
            // wrong-side (+0.24 pooled) and marking at −0.16.
            //
            // Both halves now come from `MarkerEvasion`'s shared blends —
            // ONE contest, one source: `holder_quality` (marking-led,
            // the same blend the evasion edge reads) against
            // `mover_quality` (off_the_ball-led). A linear extrapolation
            // is exactly what a check-and-spin exploits, and how far it
            // is exploited is now the mover's trickiness against the
            // tracker's reading. Centred at zero for an even contest at
            // ANY level — both blends ride the same population, so the
            // duel is divisionally flat by construction.
            let minute = sc::minute_from_ms(ctx.context.total_match_time);
            let mover_q = ctx
                .context
                .players
                .by_id(opponent_to_mark.id)
                .map(|m| MarkerEvasion::mover_quality(m, minute))
                .unwrap_or(0.5);
            let read_edge = (MarkerEvasion::holder_quality(ctx.player) - mover_q).clamp(-0.5, 0.5);
            // −span (a step behind) … +span (a step ahead).
            // `OF_READ_SPAN` overrides for titration.
            let max_read_lag = MarkerEvasion::read_span();
            // ── …CENTRED ON THE TIME IT TAKES HIM TO GET THERE ────────
            //
            // Signing the read term around ZERO means the AVERAGE marker
            // aims at where his man is standing right now. A pursuer who
            // does that can never close: by the time he arrives the man
            // has moved on, and he settles at a standing lag of roughly
            // (the man's speed × his own travel time). That is the whole
            // of the residual gap — measured, the marker's TARGET now sits
            // 1.87 m from his man (`hold_shape_on_man` fixed the leash)
            // while the marker himself sits 4.2 m away. The target is
            // right and he never reaches it.
            //
            // Marking is not a chase, it is an interception: you run to
            // where he will be. So the lead is centred on the marker's own
            // closing time, and the read term swings either side of it —
            // a good reader is there before the man, a poor one is still
            // a step behind, and the average one arrives with him.
            let closing_ticks = {
                let gap = (opponent_to_mark.position - ctx.player.position).magnitude();
                let speed = ctx.player.max_speed_with_condition_cached().max(0.05);
                (gap / speed).min(max_read_lag * 2.0)
            };
            let prediction_time = closing_ticks + read_edge * 2.0 * max_read_lag;
            let opponent_future_position =
                opponent_to_mark.position + opponent_velocity * prediction_time;

            let mark_dist = Self::stand_off(
                own_goal,
                opponent_to_mark.position,
                def_profile.ideal_marking_distance,
            );
            let goal_side_w = def_profile.goal_side_weight;

            let to_goal = (own_goal - opponent_future_position)
                .try_normalize(0.01)
                .unwrap_or_default();
            let goal_side_offset = to_goal * mark_dist * goal_side_w;

            let ball_position = ctx.tick_context.positions.ball.position;
            let to_ball = (ball_position - opponent_future_position)
                .try_normalize(0.01)
                .unwrap_or_default();
            let ball_side_offset = to_ball * mark_dist * (1.0 - goal_side_w);

            // **"GET IN FRONT OF HIM!"**
            //
            // A marker who is purely goal-side of his man is BEHIND him
            // when the ball arrives. All of box defending is being across
            // him, on the side the ball is coming from, so the pass has
            // to beat you before it beats the goal — and the man who can
            // see whether that is happening is the one standing behind
            // the whole picture. See [`KeeperVoice::front_foot`]: inside
            // the zone his voice reaches, a commanding keeper's defenders
            // stand up to a metre further across the lane and a quiet
            // keeper's a step further behind it. Centred, so a median
            // keeper leaves the marking geometry exactly where it was
            // fitted, and applied as an absolute offset rather than as a
            // shift on `goal_side_weight` — that blend is scaled by
            // `ideal_marking_distance` and the same lean expressed
            // through it was worth 7 cm, which measured as no channel at
            // all.
            let front_foot = to_ball * KeeperVoice::front_foot(ctx, own_goal);
            let desired_position =
                opponent_future_position + goal_side_offset + ball_side_offset + front_foot;
            // …but not at the cost of the unit's shape. Marking is 31% of
            // everything the back line does and referred to nothing but
            // its man, so a defender followed a runner clean across the
            // pitch and took an 18-metre hole in the line with him. The
            // leash is his zone: a man who runs further than that is the
            // next defender's. See [`DefensiveLine::hold_shape`].
            //
            // The unleashed target, kept only for the leash census below.
            #[cfg(feature = "match-logs")]
            let want = desired_position;
            // The assignment bound, not the zonal one — see
            // [`DefensiveLine::hold_shape_on_man`]. He may not be dragged
            // across the pitch, but he may go as far up as the man he was
            // given, and inside his own area the man comes first.
            let desired_position =
                DefensiveLine::hold_shape_on_man(ctx, desired_position, opponent_to_mark.position);
            // …unless the line is stepping up past a runner, which takes
            // his man with it.
            let desired_position = match LineStep::depth(ctx, desired_position.x) {
                Some(x) => Vector3::new(x, desired_position.y, desired_position.z),
                None => desired_position,
            };
            // What did the leash actually cost this duel? See
            // `DefenceDiag::note_mark_leash`.
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::DefenceDiag::note_mark_leash(
                (want - opponent_to_mark.position).magnitude(),
                (desired_position - opponent_to_mark.position).magnitude(),
                (desired_position - want).magnitude(),
                opponent_to_mark.tactical_positions.is_midfielder(),
            );

            let to_desired = desired_position - ctx.player.position;
            let distance = to_desired.magnitude();

            // In position he moves with his man: at his man's pace, faster
            // only by what closing the gap needs.
            if distance < 1.0 {
                return Some(
                    StationKeeping::hold(opponent_velocity, to_desired)
                        + ctx.player().separation_velocity() * 0.2,
                );
            }

            // Urgency relative to the profile-driven mark distance.
            let urgency = (distance / mark_dist.max(4.0)).clamp(0.6, 2.0);

            let threat_boost = if opponent_to_mark.has_ball(ctx) && distance < 20.0 {
                1.3
            } else {
                1.0
            };

            // Steer onto the marking position rather than assigning the
            // velocity outright.
            //
            // `direction * (pace * urgency * mult)` produced an absolute
            // velocity of up to ~40 u/tick against a ~0.63 u/tick top
            // speed, with no reference to the velocity the defender
            // already had — so the engine clamp kept whatever heading
            // `desired_position` implied on that tick. Since the marking
            // TARGET is a discrete choice (`find_best_marking_target`
            // picks a man), every time the chosen man changed the
            // defender's heading inverted at full speed rather than
            // curving across. The per-tick dumps show exactly that shape:
            // four or five ticks of smooth, steadily rotating movement at
            // a constant 0.407, then an abrupt snap to the opposite
            // heading AT A DIFFERENT SPEED — a changed target, not an
            // overshoot (`dev_match trace` reversal dumps). `Defender:
            // Marking` was the top fast-reversal state once the goal-side
            // override was fixed.
            //
            // `Arrive` integrates from the current velocity under a force
            // limit and decelerates into the mark, which is also what
            // marking looks like: you settle alongside your man rather
            // than arriving at a sprint. Urgency and the threat boost
            // still scale the result, and the recovery multiplier still
            // reaches the speed through the profile.
            let base = SteeringBehavior::Arrive {
                target: desired_position,
                slowing_distance: mark_dist.max(4.0),
            }
            .calculate(ctx.player)
            .velocity;
            Some(
                base * urgency * threat_boost * def_profile.recovery_run_mult
                    + ctx.player().separation_velocity() * 0.2,
            )
        } else {
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Marking a runner means matching their movement with the ball
        // live — high intensity. Was Moderate, which capped the defender
        // to a jog and let the marked attacker pull away.
        // ⚠ A MARKER MUST BE ALLOWED TO RUN AS FAST AS THE MAN HE MARKS.
        //
        // This is a speed CAP (`MovementEffort::speed_fraction`), not a
        // label. It read `High` — 0.78 of top speed — while
        // `ForwardRunningInBehindState` declares `VeryHigh`, 0.95. So the
        // striker making the run was allowed 22% more speed than the
        // defender tracking it, every time, and no amount of marking-
        // distance tuning could close a gap the movement layer was opening:
        // over a 20 m run the cap alone puts 3.6 m between them.
        //
        // Measured against it: markers sat **5.7 m** off their man on the
        // ticks where they were actually in a marking state, against an
        // `ideal_marking_distance` of 0.9-1.75 m, and the attacker got away
        // on 59% of duels.
        //
        // Raising it does NOT make defenders sprint all match — the tier is
        // a ceiling and `MovementEffort` leaves any velocity already below
        // it untouched, so a marker shadowing a walking attacker still
        // walks. It only stops the movement layer from forbidding him to
        // keep up. Fatigue is 75% velocity-driven for the same reason.
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl DefenderMarkingState {
    /// Within this distance of his goal (32.5 m) a man is marked as tight as
    /// the defender marks.
    const TIGHT_WITHIN: f32 = 260.0;
    /// From this distance (58.5 m) he is given a couple of yards (2.5 m).
    const LOOSE_FROM: f32 = 468.0;
    const LOOSE: f32 = 20.0;

    /// How far off his man a marker stands: his own marking distance near
    /// goal, widening continuously to a couple of yards in midfield.
    pub(crate) fn stand_off(own_goal: Vector3<f32>, man: Vector3<f32>, tight: f32) -> f32 {
        let from_goal = (man - own_goal).xy().norm();
        let t = ((from_goal - Self::TIGHT_WITHIN) / (Self::LOOSE_FROM - Self::TIGHT_WITHIN))
            .clamp(0.0, 1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        tight + (Self::LOOSE.max(tight) - tight) * eased
    }
}

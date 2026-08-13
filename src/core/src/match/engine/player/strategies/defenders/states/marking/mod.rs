use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{
    ActivityIntensity, DefenderCondition, DefensiveLine,
};
use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::common::states::ContactFoul;
use crate::r#match::player::strategies::players::DefensiveRole;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const TACKLING_DISTANCE_THRESHOLD: f32 = 10.0; // Aggressive tackle when marking — don't let attacker turn
const STAMINA_THRESHOLD: f32 = 20.0; // Minimum stamina to continue marking
const BALL_PROXIMITY_THRESHOLD: f32 = 15.0; // Increased from 10.0 - react earlier to ball
const HEADING_HEIGHT: f32 = 1.5;
const HEADING_DISTANCE: f32 = 5.0;

#[derive(Default, Clone)]
pub struct DefenderMarkingState {}

impl StateProcessingHandler for DefenderMarkingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // A MAN WITH THE BALL IS NOT MARKING ANYBODY.
        //
        // Nothing in this state ever asked whether the defender himself
        // had won it. Every `has_ball` here is a question about the
        // OPPONENT — is my man carrying it — which reads as coverage
        // until you look for the other one. So a defender who intercepted
        // or was simply the nearest body when the ball arrived carried on
        // marking: `process` fell through to `None` (keep marking) and
        // `velocity` steered him onto his marking position, where
        // `distance < 1.0` cuts the velocity to almost nothing. Settled
        // on his mark with the ball at his feet, re-deciding the same
        // thing every tick — a fixed point, broken only by the stall
        // detector 20 s later.
        //
        // Measured with the per-state dwell split: `Defender: Marking`
        // was **99% in the 250-plus dwell bucket** — after
        // `Forward: Running` was fixed, the last genuine freeze in the
        // table rather than a claim-tick label.
        //
        // `Running` is where a defender's on-ball decisions live, and
        // this is the same hand-off `DefenderStandingState` has always
        // made.
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // The shirt pull. A marker who is being pulled away from grabs,
        // leans or blocks — the commonest foul in football and one this
        // engine could not produce, because tackling was its only foul
        // source. See `ContactFoul`.
        if ContactFoul::is_decision_tick(ctx) {
            if let Some(man) = self.find_best_marking_target(ctx) {
                let gap = (man.position - ctx.player.position).magnitude();
                // He is going past me if he is moving and I am not with
                // him — the moment a beaten defender reaches out.
                let losing_him = man.velocity(ctx).norm() > ctx.player.velocity.norm() + 0.08;
                let p = ContactFoul::probability(ctx, gap, losing_him);
                if ctx.context.rng.bernoulli(p) {
                    return Some(StateChangeResult::with_defender_state_and_event(
                        DefenderState::Standing,
                        Event::PlayerEvent(PlayerEvent::CommitFoul(
                            ctx.player.id,
                            ContactFoul::severity(ctx, losing_him),
                        )),
                    ));
                }
            }
        }

        // BOX EMERGENCY — stop marking an off-ball runner if the
        // carrier is INSIDE our penalty area and we're one of the two
        // closest defenders. A shot is imminent; engage the carrier
        // regardless of marking duties.
        if ctx.player().defensive().is_box_emergency_for_me() {
            if let Some(carrier) = ctx.players().opponents().with_ball().next() {
                let d = carrier.distance(ctx);
                if d < 25.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Tackling,
                    ));
                }
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }
        }

        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
        }

        // 1. Check if the defender has enough stamina to continue marking
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina < STAMINA_THRESHOLD {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Resting,
            ));
        }

        // Check if ball is aerial and at heading height
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_distance = ctx.ball().distance();

        if ball_position.z > HEADING_HEIGHT
            && ball_distance < HEADING_DISTANCE
            && ctx.ball().is_towards_player_with_angle(0.6)
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Heading,
            ));
        }

        // 2. Find best opponent to mark using coordination system
        // First try to find an unmarked opponent if current target is being engaged by another defender
        let opponent_to_mark = self.find_best_marking_target(ctx);

        if let Some(opponent) = opponent_to_mark {
            let distance_to_opponent = opponent.distance(ctx);

            // Priority: If opponent with ball is close, press/tackle immediately
            if opponent.has_ball(ctx) {
                if distance_to_opponent < TACKLING_DISTANCE_THRESHOLD {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Tackling,
                    ));
                }
                // Press the ball carrier aggressively — any marking defender should engage
                if distance_to_opponent < 30.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Pressing,
                    ));
                }
            }

            // Has he lost him?
            //
            // ⚠ THIS DISAGREES WITH `DefenderRunningState` BY AN ORDER OF
            // MAGNITUDE, AND THE DISAGREEMENT IS LOAD-BEARING. It
            // releases at `ideal_marking_distance * 2` (14-28u, i.e.
            // 1.75-3.5 m) while `Running` picks a man up at
            // `MARK_BREAK_DISTANCE` (150u, 18.75 m) — so every man
            // between the two figures satisfies both conditions at once
            // and the pair runs as a two-tick oscillator. Measured with
            // `dev_match trace`: ~191,000 `Running <-> Marking` loops
            // across three matches, an order of magnitude above anything
            // else in the engine.
            //
            // Replacing it with one hysteretic pair (engage 150u, release
            // 200u — the `ShapeStation` idiom) DOES remove the loop:
            // flips/min 164.6 -> 140.4, pong 36.2% -> 23.0%, and the loop
            // leaves the table entirely. It also makes the match WORSE on
            // every axis measured: goals 4.4 -> 5.2, passes/team 890 ->
            // 1050, ball-stuck 45 -> 114 s/match, FWD goal share 58% ->
            // 46%.
            //
            // The flicker is doing real work. Alternating between this
            // state's MAN-oriented steering and `Running`'s SHAPE-oriented
            // steering blends the two; pinning a defender in `Marking`
            // lets him follow his man out of the line, and
            // `DefensiveLine::hold_shape` is not a strong enough leash to
            // hold him there for seconds at a time.
            //
            // So the loop stays until `Marking`'s own steering is
            // shape-safe. Do not "fix" it in isolation.
            let def_profile = DefenderSkillProfile::from_ctx(ctx);
            if distance_to_opponent > def_profile.ideal_marking_distance * 2.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Running,
                ));
            }

            // If ball is close and unmarked, consider intercepting
            if ctx.ball().distance() < BALL_PROXIMITY_THRESHOLD && !opponent.has_ball(ctx) {
                if ctx.ball().is_towards_player_with_angle(0.7) {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Intercepting,
                    ));
                }
            }

            // Role check: if a ball carrier exists and our role has
            // flipped away from Help, route back through Standing so the
            // role block can reassign us (Primary if we're now closest,
            // Cover if we're goal-side second, Hold otherwise).
            if ctx.players().opponents().with_ball().next().is_some() {
                let role = ctx.player().defensive().defensive_role_for_ball_carrier();
                if role != DefensiveRole::Help {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Standing,
                    ));
                }
            }

            // Continue marking
            None
        } else {
            // No opponent to mark — drop to Standing so the role block
            // there can re-evaluate (HoldingLine would route Help straight
            // back to Marking, causing a state ping-pong).
            Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ))
        }
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Profile-driven marking: ideal distance and goal-side weight
        // both scale with marking_profile / positioning curve, so an
        // elite defender stands closer and more goal-side, while a
        // poor marker stays at arm's length and gets shrugged off runs.
        if let Some(opponent_to_mark) = self.find_best_marking_target(ctx) {
            let def_profile = DefenderSkillProfile::from_ctx(ctx);
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
            // Scored on the same three attributes `MarkerEvasion` reads
            // him on, so the two halves of the duel are one contest.
            let reading = {
                let s = &ctx.player.skills.mental;
                ((s.positioning / 20.0) * 0.40
                    + (s.anticipation / 20.0) * 0.35
                    + (s.concentration / 20.0) * 0.25)
                    .clamp(0.0, 1.0)
            };
            // −MAX_READ_LAG (a step behind) … +MAX_READ_LAG (a step ahead).
            const MAX_READ_LAG: f32 = 14.0;
            let prediction_time = (reading - 0.5) * 2.0 * MAX_READ_LAG;
            let opponent_future_position =
                opponent_to_mark.position + opponent_velocity * prediction_time;

            let mark_dist = def_profile.ideal_marking_distance;
            let goal_side_w = def_profile.goal_side_weight;

            let to_goal = (own_goal - opponent_future_position).normalize();
            let goal_side_offset = to_goal * mark_dist * goal_side_w;

            let ball_position = ctx.tick_context.positions.ball.position;
            let to_ball = (ball_position - opponent_future_position).normalize();
            let ball_side_offset = to_ball * mark_dist * (1.0 - goal_side_w);

            let desired_position = opponent_future_position + goal_side_offset + ball_side_offset;
            // …but not at the cost of the unit's shape. Marking is 31% of
            // everything the back line does and referred to nothing but
            // its man, so a defender followed a runner clean across the
            // pitch and took an 18-metre hole in the line with him. The
            // leash is his zone: a man who runs further than that is the
            // next defender's. See [`DefensiveLine::hold_shape`].
            let desired_position = DefensiveLine::hold_shape(ctx, desired_position);

            let to_desired = desired_position - ctx.player.position;
            let distance = to_desired.magnitude();

            if distance < 1.0 {
                return Some(to_desired * 0.5);
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
        DefenderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl DefenderMarkingState {
    /// Find the best marking target using the role system.
    /// In the Help role (ball carrier active), pick the most dangerous
    /// non-carrier unmarked opponent — this cuts pass lanes around the
    /// primary presser. Otherwise (no live ball-carrier scenario) fall
    /// back to the generic "find most dangerous opponent" scan.
    fn find_best_marking_target(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        if ctx.players().opponents().with_ball().next().is_some() {
            if let Some(help_target) = ctx.player().defensive().find_help_target() {
                return Some(help_target);
            }
        }

        // No live carrier: mark the most dangerous unmarked opponent.
        ctx.player()
            .defensive()
            .find_unmarked_opponent(100.0)
            .or_else(|| self.find_most_dangerous_opponent(ctx))
    }

    /// Find the most dangerous opponent to mark based on multiple factors
    fn find_most_dangerous_opponent(
        &self,
        ctx: &StateProcessingContext,
    ) -> Option<MatchPlayerLite> {
        let player_ops = ctx.player();
        let own_goal_position = ctx.ball().direction_to_own_goal();
        let ball_position = ctx.tick_context.positions.ball.position;

        let mut best_opponent = None;
        let mut best_score = f32::MIN;

        // Direct iteration — no .collect() needed
        for opponent in ctx.players().opponents().nearby(150.0) {
            let mut danger_score = 0.0;

            if opponent.has_ball(ctx) {
                danger_score += 100.0;
            }

            let distance_to_own_goal = (opponent.position - own_goal_position).magnitude();
            danger_score += (500.0 - distance_to_own_goal) / 10.0;

            let distance_to_defender = opponent.distance(ctx);
            danger_score += (100.0 - distance_to_defender.min(100.0)) / 5.0;

            let opponent_velocity = opponent.velocity(ctx);
            let speed_sq = opponent_velocity.norm_squared();

            if speed_sq > 0.01 {
                let speed = speed_sq.sqrt();
                let to_our_goal = (own_goal_position - opponent.position).normalize();
                let velocity_dir = opponent_velocity * (1.0 / speed);
                let alignment = velocity_dir.dot(&to_our_goal);

                if alignment > 0.0 {
                    danger_score += alignment * 30.0;
                    if speed > 3.0 && alignment > 0.7 {
                        danger_score += 25.0;
                    }
                }
            }

            if !opponent.has_ball(ctx) {
                let distance_to_ball = (opponent.position - ball_position).magnitude();
                danger_score += (50.0 - distance_to_ball.min(50.0)) / 5.0;
            }

            // Receiver threat — weights off_the_ball, finishing,
            // anticipation, pace + composure heavily, replacing the
            // simple (pace+dribbling+finishing)/3 average. A pacy poacher
            // (high finishing + off_ball) is now scored materially above
            // a midfield carrier with the same average skill total.
            let opponent_skills = player_ops.skills(opponent.id);
            let receiver_threat = ((opponent_skills.mental.off_the_ball / 20.0).powf(1.45) * 0.22
                + (opponent_skills.physical.pace / 20.0).powf(1.25) * 0.14
                + (opponent_skills.physical.acceleration / 20.0).powf(1.25) * 0.12
                + (opponent_skills.technical.finishing / 20.0).powf(1.45) * 0.16
                + (opponent_skills.mental.anticipation / 20.0).powf(1.30) * 0.14
                + (opponent_skills.mental.composure / 20.0).powf(1.20) * 0.08)
                .clamp(0.0, 1.0);
            danger_score += receiver_threat * 30.0;

            // COMMITMENT: keep the man you are already going to.
            //
            // Everything above scores the SITUATION, and several of the
            // terms sit within a few points of each other for two
            // attackers in the same area. The argmax therefore swapped
            // between them on tiny frame-to-frame changes, and because
            // the marking velocity is `direction * (pace * urgency)` —
            // saturated by the engine clamp — a swap does not nudge the
            // defender, it inverts him at full speed. The per-tick dumps
            // show exactly that: four or five ticks of smooth, steadily
            // rotating movement at a constant 0.407, then an abrupt snap
            // to the opposite heading at a different speed, which is a
            // changed target rather than an overshoot (`dev_match trace`
            // reversal dumps). `Defender: Marking` was left as the top
            // fast-reversal state once the goal-side override was fixed.
            //
            // The defender's own velocity is the memory of who he chose —
            // he has been running at that man — so aligning with it
            // rewards continuing. A genuinely more dangerous opponent
            // still wins: this is worth ~15 points against the 100 a ball
            // carrier scores and the 30 a goalward run does, so it breaks
            // ties without overriding a real threat. Continuous in the
            // geometry, so it cannot itself introduce a step.
            let my_velocity = ctx.player.velocity;
            let my_speed_sq = my_velocity.norm_squared();
            if my_speed_sq > 0.0025 {
                let to_opponent = opponent.position - ctx.player.position;
                let to_opp_dist = to_opponent.magnitude();
                if to_opp_dist > 0.01 {
                    let heading = my_velocity * (1.0 / my_speed_sq.sqrt());
                    let alignment = heading.dot(&(to_opponent * (1.0 / to_opp_dist)));
                    danger_score += alignment.max(0.0) * 15.0;
                }
            }

            if danger_score > best_score {
                best_score = danger_score;
                best_opponent = Some(opponent);
            }
        }

        best_opponent
    }
}

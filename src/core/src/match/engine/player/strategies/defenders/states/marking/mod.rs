use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::players::DefensiveRole;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
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

            // If opponent is too far, switch to running to catch up.
            // The "too far" threshold is the marking_profile-driven
            // ideal_marking_distance scaled 2x, so a poor marker has a
            // wider tolerance (he stands further off anyway) and an
            // elite marker breaks contact earlier.
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

            let prediction_time = 0.3;
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

            let to_desired = desired_position - ctx.player.position;
            let distance = to_desired.magnitude();

            if distance < 1.0 {
                return Some(to_desired * 0.5);
            }

            let direction = to_desired.normalize();
            // Urgency relative to the profile-driven mark distance.
            let urgency = (distance / mark_dist.max(4.0)).clamp(0.6, 2.0);
            // Recovery-run mult bakes pace/stamina/condition into a
            // 0.58..1.05 multiplier so tired markers can't keep up
            // with fast attackers.
            let speed = ctx.player.skills.physical.pace * urgency * def_profile.recovery_run_mult;

            let threat_boost = if opponent_to_mark.has_ball(ctx) && distance < 20.0 {
                1.3
            } else {
                1.0
            };

            Some(direction * speed * threat_boost + ctx.player().separation_velocity() * 0.2)
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

            if danger_score > best_score {
                best_score = danger_score;
                best_opponent = Some(opponent);
            }
        }

        best_opponent
    }
}

use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::{
    ShotDecision, evaluate_forward_shot_decision,
};
use crate::r#match::player::strategies::common::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PassEvaluator, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct MidfielderDribblingState {}

impl StateProcessingHandler for MidfielderDribblingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        let mid_profile = MidfielderSkillProfile::from_ctx(ctx);
        let shot_profile = ctx.player().shooting().shot_profile();
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let has_clear_shot = ctx.player().has_clear_shot();

        // AM carve-out: forward helper picks the trigger so a low-skill
        // #10 can still finish their own dribble. The standard gate
        // below requires mid_shot_selection >= 0.42 which a 10-skill
        // AM never reaches.
        if ctx
            .player
            .tactical_position
            .current_position
            .is_attacking_midfielder()
        {
            if let ShotDecision::Shoot { reason } =
                evaluate_forward_shot_decision(ctx, "AM_DRIB_FWD")
            {
                return Some(
                    StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                        .with_shot_reason(reason),
                );
            }
        }

        // Shooting from dribble — gated on midfielder shot selection
        // and the unified shot profile, not raw distance bands. A 5/20
        // midfielder shouldn't pivot to a shot just because they ended
        // up in their personal "in_shooting_range" band.
        if has_clear_shot
            && distance_to_goal <= 32.0
            && shot_profile.expected_xg(distance_to_goal, true) >= 0.12
            && (mid_profile.mid_shot_selection >= 0.34 || shot_profile.execution_skill >= 0.50)
        {
            return Some(
                StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                    .with_shot_reason("MID_DRIB_IN_RANGE"),
            );
        }

        // Point-blank — inside the box, but with a skill-graded
        // willingness so 5/20 midfielders can still pass instead of
        // miscueing the easy chance. Real point-blank shots succeed for
        // composed finishers; panicked low-skill players hit the keeper.
        if distance_to_goal < 22.0 {
            let point_blank_willingness = (0.10
                + shot_profile.selection_skill * 0.30
                + mid_profile.mid_shot_selection * 0.20)
                .clamp(0.12, 0.65);
            if rand::random::<f32>() < point_blank_willingness {
                return Some(
                    StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                        .with_shot_reason("MID_DRIB_POINT_BLANK"),
                );
            }
            // Roll failed — try cutback / pass before forcing the shot.
            if PassEvaluator::find_best_pass_option(ctx, 60.0).is_some() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
        }

        // Carry budget scaled by carry_selection (skill-blended) instead
        // of raw dribbling — a poor dribbler with high decisions still
        // gets some carry tolerance via composure / decisions weighting.
        let max_dribble_ticks = (25.0 + mid_profile.carry_selection * 55.0) as u64;

        // Under heavy pressure — defer to press_resistance: high
        // resistance lets us shield/pass cleanly, low resistance forces
        // a hurried release.
        let close_opponents = ctx.players().opponents().nearby(15.0).count();
        if close_opponents >= 2 {
            if distance_to_goal < 32.0 && has_clear_shot && mid_profile.mid_shot_selection >= 0.42 {
                return Some(
                    StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                        .with_shot_reason("MID_DRIB_PRESSURED_SHOOT"),
                );
            }
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Passing,
            ));
        }

        // Timeout — force a decision
        if ctx.in_state_time > max_dribble_ticks {
            if PassEvaluator::find_best_pass_option(ctx, 200.0).is_some() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // If carry quality is low and a pass opens up mid-dribble, take
        // it. Replaces the raw `dribbling_skill < 0.7` heuristic.
        if ctx.in_state_time > 15 && !mid_profile.allows_take_on_one() {
            if PassEvaluator::find_best_pass_option(ctx, 200.0).is_some() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if !ctx.player.has_ball(ctx) {
            let ball_pos = ctx.tick_context.positions.ball.position;
            let direction = (ball_pos - ctx.player.position).normalize();
            return Some(direction * ctx.player.skills.physical.pace * 0.3);
        }

        let goal_pos = ctx.player().opponent_goal_position();
        let player_pos = ctx.player.position;
        let to_goal = (goal_pos - player_pos).normalize();

        let dribble_skill = ctx.player.skills.technical.dribbling / 20.0;
        let pace = ctx.player.skills.physical.pace / 20.0;
        let agility = ctx.player.skills.physical.agility / 20.0;

        // Base dribble speed — faster than walking, slower than sprinting
        let base_speed = 3.5 * (0.5 * dribble_skill + 0.3 * pace + 0.2 * agility);

        // Find nearest opponent to dribble around
        let nearest_opponent = ctx.players().opponents().nearby(30.0).min_by(|a, b| {
            let da = (a.position - player_pos).magnitude();
            let db = (b.position - player_pos).magnitude();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });

        let direction = if let Some(opponent) = nearest_opponent {
            let opp_dist = (opponent.position - player_pos).magnitude();

            if opp_dist < 20.0 {
                // Opponent is close — use skill-based evasion
                let to_opp = (opponent.position - player_pos).normalize();
                // Perpendicular direction (dodge sideways, biased toward goal)
                let perp = Vector3::new(-to_opp.y, to_opp.x, 0.0);
                // Choose the perpendicular that points more toward goal
                let dodge_dir = if perp.dot(&to_goal) > (-perp).dot(&to_goal) {
                    perp
                } else {
                    -perp
                };
                // Blend dodge direction with goal direction (skilled players stay on course)
                (to_goal * dribble_skill + dodge_dir * (1.0 - dribble_skill * 0.5)).normalize()
            } else {
                // Opponent nearby but not immediate — curve run to avoid
                let to_opp = (opponent.position - player_pos).normalize();
                let avoidance = to_goal - to_opp * 0.3;
                avoidance.normalize()
            }
        } else {
            // Open space — run straight toward goal
            to_goal
        };

        Some(direction * base_speed + ctx.player().separation_velocity())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Dribbling is moderate intensity
        MidfielderCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}

impl MidfielderDribblingState {
    #[allow(dead_code)]
    fn find_open_teammate<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        // Use the proper pass evaluator instead of a random nearby pick
        // — random fallbacks fed the ball into covered teammates.
        PassEvaluator::find_best_pass_option(ctx, 200.0).map(|(t, _)| t)
    }

    #[allow(dead_code)]
    fn is_in_shooting_position(&self, ctx: &StateProcessingContext) -> bool {
        let shooting_range = 25.0;
        let player_position = ctx.player.position;
        let goal_position = ctx.player().opponent_goal_position();

        let distance_to_goal = (player_position - goal_position).magnitude();

        distance_to_goal <= shooting_range
    }

    #[allow(dead_code)]
    fn should_return_to_position(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the player is far from their starting position and the team is not in possession
        let distance_from_start = ctx.player().distance_from_start_position();
        let team_in_possession = ctx.team().is_control_ball();

        distance_from_start > 20.0 && !team_in_possession
    }

    #[allow(dead_code)]
    fn should_press(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the player should press the opponent with the ball
        let ball_distance = ctx.ball().distance();
        let pressing_distance = 150.0; // Adjust the threshold as needed

        !ctx.team().is_control_ball() && ball_distance < pressing_distance
    }
}

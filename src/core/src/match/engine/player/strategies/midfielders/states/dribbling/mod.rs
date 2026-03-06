use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use rand::prelude::IteratorRandom;

#[derive(Default, Clone)]
pub struct MidfielderDribblingState {}

impl StateProcessingHandler for MidfielderDribblingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Shooting takes priority — use proper shooting range check
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        if ctx.player().shooting().in_shooting_range() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Shooting,
            ));
        }

        // Point-blank — always shoot regardless of skill
        if distance_to_goal < 50.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Shooting,
            ));
        }

        let dribbling_skill = ctx.player.skills.technical.dribbling / 20.0;

        // Skilled dribblers can carry longer (40-80 ticks), less skilled exit earlier (25-40)
        let max_dribble_ticks = (25.0 + dribbling_skill * 55.0) as u64;

        // Check if heavily pressured — multiple opponents closing in
        let close_opponents = ctx.players().opponents().nearby(15.0).count();
        if close_opponents >= 2 {
            // Under heavy pressure near goal: shoot
            if distance_to_goal < 120.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Passing,
            ));
        }

        // Timeout — force a decision
        if ctx.in_state_time > max_dribble_ticks {
            // Look for pass first
            if self.find_open_teammate(ctx).is_some() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
            // Otherwise go back to running (will re-evaluate)
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // If a great pass opens up mid-dribble, take it
        if ctx.in_state_time > 15 && dribbling_skill < 0.7 {
            if self.find_open_teammate(ctx).is_some() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
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
        let nearest_opponent = ctx.players().opponents().nearby(30.0)
            .min_by(|a, b| {
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
    fn find_open_teammate<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<u32> {
        // Find an open teammate to pass to
        let teammates = ctx.players().teammates().nearby_ids(150.0);

        if let Some((teammate_id, _)) = teammates.choose(&mut rand::rng()) {
            return Some(teammate_id);
        }

        None
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

use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PassingEventModel, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;
use std::sync::LazyLock;
use rand::prelude::IteratorRandom;

static MIDFIELDER_LONG_PASSING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_passing_data.json")));

// Constants used in passing calculations
const MAX_PASS_DISTANCE: f32 = 300.0; // Maximum distance for a short pass
const MIN_PASS_SPEED: f32 = 10.0; // Minimum speed of the pass
const MAX_PASS_SPEED: f32 = 15.0; // Maximum speed of the pass
const STAMINA_COST_PASS: f32 = 2.0; // Stamina cost of making a pass
const OPPONENT_COLLISION_RADIUS: f32 = 0.5; // Radius representing opponent's collision area

#[derive(Default)]
pub struct MidfielderPassingState {}

impl StateProcessingHandler for MidfielderPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the midfielder still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Pressing
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        // Determine the best teammate to pass to
        if let Some(target_teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventModel::build()
                        .with_player_id(ctx.player.id)
                        .with_target(target_teammate.position)
                        .with_force(ctx.player().pass_teammate_power(target_teammate.id))
                        .build()
                )),
            ));
        }

        if ctx.ball().distance_to_opponent_goal() < 200.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Shooting,
            ))
        }
        
        if ctx.in_state_time > 100 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Distributing,
            ))
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if ctx.in_state_time % 10 == 0 {
            if let Some(nearest_teammate) = ctx.players().teammates().nearby_to_opponent_goal() {
                return Some(
                    SteeringBehavior::Arrive {
                        target: nearest_teammate.position,
                        slowing_distance: 30.0,
                    }
                    .calculate(ctx.player)
                    .velocity,
                );
            }
        }

        None
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderPassingState {
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;

        let attacking_third_start = if ctx.player.side == Some(PlayerSide::Left) {
            field_width * (2.0 / 3.0)
        } else {
            field_width / 3.0
        };

        if player_position.x >= attacking_third_start {
            // Player is in the attacking third, prioritize teammates near the opponent's goal
            self.find_best_pass_option_attacking_third(ctx)
        } else if player_position.x >= field_width / 3.0
            && player_position.x <= field_width * (2.0 / 3.0)
        {
            // Player is in the middle third, prioritize teammates in advanced positions
            self.find_best_pass_option_middle_third(ctx)
        } else {
            // Player is in the defensive third, prioritize safe passes to nearby teammates
            self.find_best_pass_option_defensive_third(ctx)
        }
    }

    fn find_best_pass_option_attacking_third(
        &self,
        ctx: &StateProcessingContext<'_>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        let nearest_to_goal = teammates
            .all()
            .filter(|teammate| {
                // Check if the teammate is in a dangerous position near the opponent's goal
                let goal_distance_threshold = ctx.context.field_size.width as f32 * 0.2;
                (teammate.position - ctx.ball().direction_to_opponent_goal()).magnitude()
                    < goal_distance_threshold
            })
            .min_by(|a, b| {
                let dist_a = (a.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                let dist_b = (b.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });

        nearest_to_goal
    }

    fn find_best_pass_option_defensive_third<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        let nearest_teammate = teammates
            .nearby(200.0)
            .min_by(|a, b| {
                let dist_a = (a.position - ctx.player.position).magnitude();
                let dist_b = (b.position - ctx.player.position).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });

        nearest_teammate
    }

    fn find_best_pass_option_middle_third(
        &self,
        ctx: &StateProcessingContext<'_>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        let nearest_to_goal = teammates
            .all()
            .filter(|teammate| {
                // Check if the teammate is in a dangerous position near the opponent's goal
                let goal_distance_threshold = ctx.context.field_size.width as f32 * 0.2;
                (teammate.position - ctx.ball().direction_to_opponent_goal()).magnitude()
                    < goal_distance_threshold
            })
            .min_by(|a, b| {
                let dist_a = (a.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                let dist_b = (b.position - ctx.ball().direction_to_opponent_goal()).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });

        nearest_to_goal
    }

    /// Checks if the pass to the target teammate is feasible using ray tracing.
    fn is_pass_feasible_ray_tracing(
        &self,
        ctx: &StateProcessingContext,
        target_teammate: &MatchPlayerLite,
    ) -> bool {
        true
    }

    /// Checks if a ray intersects with a sphere (opponent).
    fn ray_intersects_sphere(
        &self,
        ray_origin: Vector3<f32>,
        ray_direction: Vector3<f32>,
        sphere_center: Vector3<f32>,
        sphere_radius: f32,
        max_distance: f32,
    ) -> bool {
        let m = ray_origin - sphere_center;
        let b = m.dot(&ray_direction);
        let c = m.dot(&m) - sphere_radius * sphere_radius;

        // Exit if the ray's origin is outside the sphere (c > 0) and pointing away from the sphere (b > 0)
        if c > 0.0 && b > 0.0 {
            return false;
        }

        let discriminant = b * b - c;

        // A negative discriminant indicates no intersection
        if discriminant < 0.0 {
            return false;
        }

        // Compute the distance to the intersection point
        let t = -b - discriminant.sqrt();

        // If t is negative, the intersection is behind the ray's origin
        if t < 0.0 {
            return false;
        }

        // Check if the intersection point is within the maximum distance
        t <= max_distance
    }
}

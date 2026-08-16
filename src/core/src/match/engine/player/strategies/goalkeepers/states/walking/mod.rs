use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperRestPosition,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::player::strategies::processor::StateChangeResult;
use crate::r#match::player::strategies::processor::{
    StateProcessingContext, StateProcessingHandler,
};
use crate::r#match::{ConditionContext, PlayerSide, SteeringBehavior, VectorExtensions};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperWalkingState {}

impl StateProcessingHandler for GoalkeeperWalkingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Shot in flight at our goal — break off walking and commit
        // to the save.
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            if Some(target.defending_side) == ctx.player.side {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            }
        }

        // Direct catch for very close SLOW balls.
        //
        // ⚠ Both speed bars in this state were above the engine's own
        // `MAX_SHOT_VELOCITY` (3.2 u/tick), so neither excluded anything:
        // "slow ball" meant every ball, and a keeper mid-stroll reached
        // out and collected shots. 2.0 u/tick (25 m/s) is a driven ball —
        // past that he is making a save, not picking it up.
        if ctx.ball().distance() < 5.0
            && !ctx.ball().is_owned()
            && ctx.ball().on_own_side()
            && ctx.tick_context.positions.ball.velocity.norm() < 2.0
        {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Catching,
            ));
        }

        // If goalkeeper has the ball, immediately transition to passing
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Distributing,
            ));
        }

        // Notification system: if ball system notified us to take the ball, act immediately
        if ctx.ball().should_take_ball_immediately() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::TakeBall,
            ));
        }

        // Loose ball nearby — go claim it directly
        if !ctx.ball().is_owned() && ctx.ball().distance() < 30.0 && ctx.ball().on_own_side() {
            let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
            if ball_speed < 2.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Catching,
                ));
            }
        }

        // Check ball proximity and threat level
        let ball_distance = ctx.ball().distance();
        let ball_on_own_side = ctx.ball().on_own_side();

        // Improved threat assessment using goalkeeper skills
        let threat_level = self.assess_threat_level(ctx);

        // Ball on own side while walking — switch to Standing so the
        // threat logic can drive a PreparingForSave transition. Attentive
        // was a redundant mid-state between Walking and PreparingForSave.
        if ball_on_own_side {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // Check if ball is coming directly at goalkeeper
        if ctx.ball().is_towards_player_with_angle(0.85) && ball_distance < 200.0 {
            // Use anticipation skill to determine response timing
            let anticipation_factor = ctx.player.skills.mental.anticipation / 20.0;
            let reaction_distance = 250.0 + (anticipation_factor * 100.0); // Better anticipation = earlier reaction

            if ball_distance < reaction_distance {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            }
        }

        // Use decision-making skill for coming out
        if self.should_come_out_advanced(ctx) && ball_distance < 60.0 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ComingOut,
            ));
        }

        // Check positioning using goalkeeper-specific skills
        if self.is_significantly_out_of_position(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // High threat while walking — go straight to PreparingForSave.
        // UnderPressure was a pass-through that looked for Catching
        // /Distributing next tick anyway.
        if threat_level > 0.7 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::PreparingForSave,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Same resting position as every other keeper state — see
        // `KeeperRestPosition`. This state used to own a SECOND copy of
        // the positioning model with different constants, so the same
        // keeper wanted to be in two different places depending on which
        // state he happened to be in.
        //
        // And within 10u of it he `Wander`ed, on a 6.25 m radius, forever:
        // measured **0% still** in this state, a keeper pacing aimlessly
        // around his box. Same pattern removed from the outfield walking
        // states. He stands set instead.
        let optimal_position = self.calculate_intelligent_position(ctx);
        if KeeperRestPosition::is_set_with(
            ctx.player.position,
            optimal_position,
            GoalkeeperSkillProfile::from_ctx(ctx).concentration,
        ) {
            return Some(Vector3::zeros());
        }

        let pace =
            KeeperRestPosition::pace(ctx.ball().distance(), ctx.context.field_size.width as f32);
        Some(
            SteeringBehavior::Arrive {
                target: optimal_position,
                slowing_distance: 8.0,
            }
            .calculate(ctx.player)
            .velocity
                * pace,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Walking state has low intensity but more activity than standing
        GoalkeeperCondition::with_velocity(ActivityIntensity::Low).process(ctx);
    }
}

impl GoalkeeperWalkingState {
    /// Assess threat level using goalkeeper mental skills
    fn assess_threat_level(&self, ctx: &StateProcessingContext) -> f32 {
        let mut threat = 0.0;

        // Use reflexes to assess multiple threats
        let concentration_factor = ctx.player.skills.goalkeeping.reflexes / 20.0;

        // Check for opponents with ball
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = opponent_with_ball
                .position
                .distance_to(&ctx.player.position);

            // Better concentration means better threat assessment
            if distance_to_opponent < 50.0 {
                threat += 0.8 * concentration_factor;
            } else if distance_to_opponent < 100.0 {
                threat += 0.5 * concentration_factor;
            }
        }

        // Check ball velocity and trajectory
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // Use anticipation to predict threats. `> 10.0` u/tick is three
        // times the engine's shot cap, so this term never once fired and
        // a ball travelling at the keeper contributed nothing to his read
        // of the danger.
        let anticipation_factor = ctx.player.skills.mental.anticipation / 20.0;
        if ball_speed > 1.5 && ctx.ball().is_towards_player_with_angle(0.6) {
            threat += 0.4 * anticipation_factor;
        }

        threat.min(1.0)
    }

    /// Advanced decision for coming out using goalkeeper skills
    fn should_come_out_advanced(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        let goalkeeper_skills = &ctx.player.skills;

        // Key skills for coming out decisions
        let decision_skill = goalkeeper_skills.mental.decisions / 20.0;
        let rushing_out = goalkeeper_skills.goalkeeping.rushing_out / 20.0;
        let command_of_area = goalkeeper_skills.goalkeeping.command_of_area / 20.0;
        let anticipation = goalkeeper_skills.mental.anticipation / 20.0;

        // Combined skill factor for coming out
        let coming_out_ability =
            (decision_skill + rushing_out + command_of_area + anticipation) / 4.0;

        // Base threshold adjusted by skills
        let base_threshold = 100.0;
        let skill_adjusted_threshold = base_threshold * (0.6 + coming_out_ability * 0.8); // Range: 60-140

        // Check if ball is loose and in dangerous area
        let ball_loose = !ctx.ball().is_owned();
        let ball_in_danger_zone = ball_distance < skill_adjusted_threshold;

        // Check if goalkeeper can reach ball first
        if ball_loose && ball_in_danger_zone {
            // Use acceleration and agility for reach calculation
            let reach_ability = (goalkeeper_skills.physical.acceleration
                + goalkeeper_skills.physical.agility)
                / 40.0;

            // Check if any opponent is closer
            for opponent in ctx.players().opponents().nearby(150.0) {
                let opp_distance_to_ball =
                    (opponent.position - ctx.tick_context.positions.ball.position).magnitude();
                let keeper_distance_to_ball = ball_distance;

                // Factor in goalkeeper's reach ability
                if opp_distance_to_ball < keeper_distance_to_ball * (1.0 - reach_ability * 0.3) {
                    return false; // Opponent will reach first
                }
            }

            return true;
        }

        false
    }

    /// Check if significantly out of position using positioning skill
    fn is_significantly_out_of_position(&self, ctx: &StateProcessingContext) -> bool {
        let optimal_position = self.calculate_intelligent_position(ctx);
        let current_distance = ctx.player.position.distance_to(&optimal_position);

        // Use positioning skill to determine tolerance
        let positioning_skill = ctx.player.skills.mental.positioning / 20.0;
        let tolerance = 120.0 - (positioning_skill * 40.0); // Better positioning = tighter tolerance (80-120)

        current_distance > tolerance
    }

    /// Calculate intelligent position using multiple goalkeeper skills
    /// Delegates to the ONE shared keeper positioning model.
    ///
    /// This used to be a second, divergent copy of it — same idea,
    /// different constants — so the same keeper wanted to be in two
    /// different places depending on which state he was in. Both copies
    /// were wrong the same way: the whole depth range came to a couple of
    /// metres, so he never left his line. See `KeeperRestPosition`.
    fn calculate_intelligent_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        KeeperRestPosition::point(
            ctx.ball().direction_to_own_goal(),
            ctx.tick_context.positions.ball.position,
            ctx.tick_context.positions.ball.velocity,
            ctx.player.side.unwrap_or(PlayerSide::Left),
            ctx.team().tactical().defensive_line_x,
            ctx.context.field_size.width as f32,
            ctx.player.skills.goalkeeping.command_of_area / 20.0,
            GoalkeeperSkillProfile::from_ctx(ctx).positioning,
        )
    }

    // NB the old `limit_to_penalty_area` helper is gone. It clamped the
    // keeper into his own box (plus a token 10% for a commanding one),
    // which is the constraint `KeeperRestPosition` exists to remove: a
    // keeper cannot come and meet a ball played in behind if he is not
    // allowed out of his area, and the Laws only stop him HANDLING it
    // there. `GoalkeeperStandingState::clamp_sweep_range` is the bound
    // that replaced it, and this copy had been dead since.
}

use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperAerialClaim,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// Backstop only — the real length of the state is the leap itself, and
/// he is on the ground again when `MatchPlayer::height` returns to zero:
/// 55-75 ENGINE ticks for the apexes `PlayerMatchState::leap_apex` asks
/// for, which is ~28-38 of the AI ticks `in_state_time` counts.
///
/// This was 25 AI ticks and was the ONLY exit, so a keeper left the state
/// before he had come down and spent the rest of his own jump airborne
/// while the state machine had him standing.
const MAX_JUMP_TICKS: u64 = 110;
const MIN_DIVING_DISTANCE: f32 = 1.0; // Minimum distance to dive
const MAX_DIVING_DISTANCE: f32 = 8.0; // Maximum distance to dive (extended reach)

/// A keeper attacking a high ball.
///
/// # Why this state was dead
///
/// Until the aerial claim was wired ([`KeeperAerialClaim`]) this had
/// exactly ONE inbound transition in the engine — from `Punching`, on the
/// branch that fires when the ball is out of punching range — so it only
/// ever ran on a keeper who had already made his contact, and it made him
/// jump vertically on the spot afterwards. A keeper never left the ground
/// to take a cross, a corner or a chip, and `aerial_reach`,
/// `command_of_area`, `punching` and `jumping` were decorative.
#[derive(Default, Clone)]
pub struct GoalkeeperJumpingState {}

impl StateProcessingHandler for GoalkeeperJumpingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Got it in the air — come down with it.
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::HoldingBall,
            ));
        }

        // Back on the floor with nothing: the claim is over.
        if !ctx.player.is_airborne() && ctx.in_state_time > 4 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }
        if ctx.in_state_time >= MAX_JUMP_TICKS {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // At the top of the leap, with the ball there: catch it or put a
        // fist through it. Which one is the classic goalkeeping judgement
        // — a keeper with the hands and the space takes it cleanly, one
        // being jostled in a crowd or meeting a driven ball punches it as
        // far away as he can. Before this the state could only ever catch.
        if self.can_reach_ball(ctx) {
            let prof = GoalkeeperSkillProfile::from_ctx(ctx);
            let crowd = (ctx.players().opponents().nearby(24.0).count() as f32 / 3.0).clamp(0.0, 1.0);
            let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
            let power = ((ball_speed - 1.2) / 2.0).clamp(0.0, 1.0);
            let catch_prob = self.catch_probability(ctx, &prof);

            if prof.should_punch(catch_prob, crowd, power) {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Punching,
                ));
            }
            if ctx.context.rng.unit_f32() < catch_prob {
                let mut result =
                    StateChangeResult::with_goalkeeper_state(GoalkeeperState::Catching);
                result
                    .events
                    .add_player_event(PlayerEvent::RequestBallReceive(ctx.player.id));
                return Some(result);
            }
            // Got a hand to it and no more — a keeper who misses his punch
            // is not left hanging in the air doing nothing.
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Punching,
            ));
        }

        None
    }

    /// The GROUND part of the jump — where he travels while he is up.
    ///
    /// The rise itself is not here and must not be: this vector is put
    /// through an acceleration limiter and a `max_speed` clamp that both take
    /// its 3-D norm, and a vertical term in metres inside a horizontal vector
    /// in grid units is measured against the wrong limit and then dropped by
    /// `MatchPlayer::move_to`, which integrates x and y only. The leap is a
    /// take-off, requested once on entering this state — see
    /// `PlayerMatchState::leap_apex`.
    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Calculate base jump vector
        let jump_vector = self.calculate_jump_vector(ctx);

        // Add diving motion if needed
        let diving_vector = if self.should_dive(ctx) {
            self.calculate_diving_vector(ctx)
        } else {
            Vector3::zeros()
        };

        // Combine all motion components, flat: whatever vertical lean the
        // bearing to a high ball put into them belongs to the take-off.
        let mut combined_velocity = jump_vector + diving_vector;
        combined_velocity.z = 0.0;

        // Explosive scaling — gated by the unified explosive multiplier
        // and aerial command so weak keepers can't cover the full goal.
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let attribute_scaling =
            (0.55 + prof.aerial_command * 0.45 + prof.dive_reach * 0.25) * prof.explosive_mult;

        Some(combined_velocity * attribute_scaling)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Jumping is a very high intensity activity requiring significant energy expenditure
        GoalkeeperCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl GoalkeeperJumpingState {
    /// Envelope the keeper can get a glove on at the top of this leap.
    /// Kept separate from the odds so the state can ask "is it there?"
    /// once and then decide between catching and punching, rather than
    /// rolling a catch and treating a failed roll as out of reach.
    ///
    /// The vertical half is [`KeeperAerialClaim::leap_ceiling`] and MUST
    /// stay so: that is what the decision to come for the ball was made
    /// against, and a local formula here disagreed with it badly — an
    /// ordinary keeper's own ceiling read **1.95 m** against the 3.0 m the
    /// claim had already committed him to, so he ran out, left the ground,
    /// and could not reach a ball he had correctly judged was his.
    ///
    /// `keeper.position.z` is 0 for every player by design (the real leap
    /// lives in `MatchPlayer::height`, out of reach of every distance
    /// helper), so the ball's own `z` is the height to measure and the
    /// jump is counted exactly once.
    fn reach_envelope(&self, ctx: &StateProcessingContext, prof: &GoalkeeperSkillProfile) -> (f32, f32) {
        let vertical = KeeperAerialClaim::leap_ceiling(ctx.player.skills.physical.jumping);
        let horizontal = MAX_DIVING_DISTANCE + prof.dive_reach * 6.0 + prof.aerial_command * 2.0;
        (vertical, horizontal)
    }

    fn can_reach_ball(&self, ctx: &StateProcessingContext) -> bool {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let (vertical, horizontal) = self.reach_envelope(ctx, &prof);
        let ball_pos = ctx.tick_context.positions.ball.position;
        (ball_pos - ctx.player.position).magnitude() <= horizontal && ball_pos.z <= vertical
    }

    /// Odds he holds it cleanly, given how far he has had to stretch, how
    /// hard the ball was hit and how high it is.
    fn catch_probability(
        &self,
        ctx: &StateProcessingContext,
        prof: &GoalkeeperSkillProfile,
    ) -> f32 {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let distance = (ball_pos - ctx.player.position).magnitude();
        let (vertical, horizontal) = self.reach_envelope(ctx, prof);

        let stretch = (distance / horizontal.max(0.1)).clamp(0.0, 1.0);
        // A cross, not a shot, so this keeps the gentler loose-ball scale
        // rather than `SaveModel::strike_power` (which is centred on a
        // struck shot and would read every delivery as maximally easy).
        let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
        let power = ((ball_speed - 1.5) / 6.0).clamp(0.0, 1.0);
        let height_factor = (ball_pos.z / vertical.max(0.1)).clamp(0.0, 1.0);

        let catch_difficulty = (power * 0.30
            + stretch * 0.24
            + height_factor * 0.18
            + (1.0 - prof.condition_mult) * 0.14
            + prof.poor_skill_penalty * 0.10)
            .clamp(0.0, 1.0);

        let mut catch_prob = prof.catch_probability(catch_difficulty);
        // Deflection damping — match `catching/mod.rs` and `diving/mod.rs`.
        if let Some(t) = &ctx.tick_context.ball.cached_shot_target {
            if t.deflected {
                catch_prob *= 0.50;
            }
        }
        catch_prob
    }

    /// Calculate the base jump vector towards the ball
    fn calculate_jump_vector(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let keeper_pos = ctx.player.position;
        let to_ball = ball_pos - keeper_pos;

        if to_ball.magnitude() > 0.0 {
            to_ball.normalize() * ctx.player.skills.physical.acceleration
        } else {
            Vector3::zeros()
        }
    }

    /// Determine if the goalkeeper should dive
    fn should_dive(&self, ctx: &StateProcessingContext) -> bool {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let keeper_pos = ctx.player.position;
        let distance = (ball_pos - keeper_pos).magnitude();

        // Check if the ball is at a distance that requires diving
        if distance < MIN_DIVING_DISTANCE || distance > MAX_DIVING_DISTANCE {
            return false;
        }

        // Check if the ball is moving towards goal
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let to_goal = ctx.ball().direction_to_own_goal() - ball_pos;

        ball_velocity.dot(&to_goal) > 0.0
    }

    /// Calculate the diving motion vector
    fn calculate_diving_vector(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let keeper_pos = ctx.player.position;
        let to_ball = ball_pos - keeper_pos;

        if to_ball.magnitude() > 0.0 {
            // Calculate diving direction considering goalkeeper's diving ability
            let diving_direction = to_ball.normalize();
            let diving_power = ctx.player.skills.physical.jumping as f32 / 20.0;

            diving_direction * diving_power * 2.0
        } else {
            Vector3::zeros()
        }
    }
}

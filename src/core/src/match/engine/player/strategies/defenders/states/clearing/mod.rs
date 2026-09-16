use crate::PlayerSkills;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::engine::ball::ball::{Ball, KICKABLE_DISTANCE};
use crate::r#match::player::PlayerSide;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct DefenderClearingState {}

impl StateProcessingHandler for DefenderClearingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // ⚠ THE BALL CAN BE TAKEN OFF HIM DURING THE WIND-UP.
        //
        // Nothing here asked. The dispatcher's `strike_in_reach` guard is
        // a REACH test and a height test, not an ownership one, so a
        // defender still within `KICKABLE_DISTANCE` of a ball that had
        // become somebody else's had his clearance executed anyway — and
        // he could not be rescued from outside either, because `Clearing`
        // is a committed action and is dropped from the chase table and
        // the `TakeMe` redirect while it runs.
        if !self.contact_available(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        // Wait a few ticks before clearing to allow the player to reach the ball
        if ctx.in_state_time < 5 {
            return None;
        }

        let mut state = StateChangeResult::with(PlayerState::Defender(DefenderState::Standing));

        // Get ball's current position
        let ball_position = ctx.tick_context.positions.ball.position;

        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Check if ball is at or near a boundary
        const BOUNDARY_THRESHOLD: f32 = 5.0;
        let at_left_boundary = ball_position.x <= BOUNDARY_THRESHOLD;
        let at_right_boundary = ball_position.x >= field_width - BOUNDARY_THRESHOLD;
        let at_top_boundary = ball_position.y >= field_height - BOUNDARY_THRESHOLD;
        let at_bottom_boundary = ball_position.y <= BOUNDARY_THRESHOLD;
        let at_boundary =
            at_left_boundary || at_right_boundary || at_top_boundary || at_bottom_boundary;

        // Determine clearance direction based on player's side (always clear AWAY from own goal)
        let is_left_side = ctx.player.side == Some(PlayerSide::Left);

        // Profile-driven clearance: technique/composure/decisions/passing
        // blend governs accuracy, strength scales distance, and a
        // poor_clearance_chance roll occasionally produces a short /
        // miscued / sliced clearance. Replaces a fully deterministic
        // halfway-line target.
        let def_profile = DefenderSkillProfile::from_ctx(ctx);
        let rng = &ctx.context.rng;
        let poor_clearance = rng.random::<f32>() < def_profile.poor_clearance_chance.min(0.95);

        // NB "hook it behind for a corner" — the largest real-world
        // corner source — was implemented here and REMOVED, because this
        // state does not run: `Defender: Clearing` is below 0.25% of AI
        // ticks in the state census, so the logic was correct and dead.
        // Defensive clearances in this engine happen in the cross
        // contest's headed-clear branch (`resolve_cross_contest`) and in
        // the heading states, not here.
        let halfway_x = field_width * 0.5;
        let nominal_target_x = if is_left_side {
            halfway_x.max(ball_position.x + 30.0)
        } else {
            halfway_x.min(ball_position.x - 30.0)
        };
        // Distance multiplier — strong + technique extend the kick;
        // poor clearances land short.
        let distance_mult = (0.75
            + (ctx.player.skills.physical.strength / 20.0).powf(1.20) * 0.25
            + (ctx.player.skills.technical.technique / 20.0).powf(1.30) * 0.15)
            .clamp(0.55, 1.20);
        let distance_mult = if poor_clearance {
            distance_mult * 0.55
        } else {
            distance_mult
        };
        // How far it goes is the man; WHERE it goes is [`Self::aim`], and
        // it is scored at the distance the kick actually covers — who can
        // reach the ball is a question about where it lands.
        let reach = (nominal_target_x - ball_position.x).abs() * distance_mult;
        let aim = self.aim(ctx, is_left_side, reach);

        // Execution error, across the line of the kick rather than
        // always toward the centre of the pitch.
        let y_error_scale = (1.25 - def_profile.clearance_profile * 0.75).max(0.30);
        let y_jitter: f32 = rng.random::<f32>() * 2.0 - 1.0;
        let extra_y_error = if poor_clearance { 22.0 } else { 6.0 };
        let across = Vector3::new(-aim.y, aim.x, 0.0) * (y_jitter * extra_y_error * y_error_scale);

        let target_position = ball_position + aim * reach + across;
        let to_target = target_position - ball_position;
        let to_target_dist = to_target.norm().max(0.1);
        let direction_to_target = to_target / to_target_dist;

        // Lofted clearance, solved the same way a lofted pass is: pick how
        // high the hoof goes (in metres), which fixes its hang time, and
        // the horizontal speed then follows from the distance it has to
        // cover. A clean clearance is a controlled outlet that finds the
        // touchline area; a poor one is a weak skewed strike that drops
        // short and rolls back into trouble.
        //
        // Previously the two components were independent constants — 4-5
        // u/tick horizontally and 5-6 "z" — fitted to a gravity that was
        // 160× too strong in metres. That produced a 40 m hoof with a
        // 0.6 s hang time, and the numbers only worked as a pair, so
        // neither could be reasoned about on its own.
        let apex_metres = if at_boundary { 12.0 } else { 9.0 };
        let apex_mult = 0.85 + (ctx.player.skills.technical.technique / 20.0).powf(1.30) * 0.15;
        let z_velocity = Ball::launch_speed_for_apex(
            apex_metres * apex_mult * if poor_clearance { 0.55 } else { 1.0 },
        );

        let speed_mult =
            (0.85 + def_profile.clearance_profile * 0.30) * def_profile.clearance_condition_mult;
        // Reach the aim point within the hang time, then trim by execution.
        // Clamped to a realistic hoof: 2.6 u/tick = 32 m/s.
        let hang = Ball::hang_ticks(z_velocity).max(1.0);
        let clear_speed =
            ((to_target_dist / hang) * speed_mult * if poor_clearance { 0.65 } else { 1.0 })
                .clamp(0.30, 2.6);
        let horizontal_velocity = direction_to_target * clear_speed;

        let ball_velocity = Vector3::new(horizontal_velocity.x, horizontal_velocity.y, z_velocity);

        state
            .events
            .add_player_event(PlayerEvent::ClearBall(ctx.player.id, ball_velocity));

        Some(state)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        Some(
            SteeringBehavior::Arrive {
                target: ball_position,
                slowing_distance: 5.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Clearing involves powerful kicking action - explosive effort
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl DefenderClearingState {
    /// How wide of straight-upfield the fan of candidate directions
    /// reaches, and how many of them there are.
    const FAN_SPREAD: f32 = 1.31; // 75 degrees
    const FAN_STEPS: i32 = 5;
    /// An opponent this close to the line of the kick can get a body in
    /// the way of it (~2.5 m).
    const BLOCK_CORRIDOR: f32 = 20.0;
    /// How near the touchline a target has to be to count as putting it
    /// out (~5 m). Deliberately tight: ramped over a quarter of the
    /// pitch instead, the term saturates for every angled candidate and
    /// the fan picks the widest one available every time — a defence
    /// that clears for a throw-in whenever it clears at all.
    const TOUCHLINE_BAND: f32 = 40.0;

    /// Can he still play this ball?
    ///
    /// Ownership AND reach: the dispatcher enforces only the second.
    fn contact_available(&self, ctx: &StateProcessingContext) -> bool {
        ctx.ball().owner_id().is_none_or(|id| id == ctx.player.id)
            && ctx.ball().distance() <= KICKABLE_DISTANCE
    }

    /// **Which way it goes**, as a unit vector across the grass.
    ///
    /// A fixed 0.60 pull toward the centre line put a pressured
    /// clearance back into the most dangerous strip on the pitch, and
    /// ignored where anybody was standing. Score a fan of forward
    /// directions on the four things that make a clearance safe, and
    /// take the best.
    fn aim(&self, ctx: &StateProcessingContext, is_left_side: bool, reach: f32) -> Vector3<f32> {
        let ball = ctx.tick_context.positions.ball.position;
        let own_goal = ctx.ball().direction_to_own_goal();
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let forward = if is_left_side { 1.0 } else { -1.0 };
        let probe = reach.max(30.0);

        // Arrival as a SHARE of the slowest man's time to the far end of
        // the kick, so the fan is scored on who is nearer the landing spot.
        // A fixed tick horizon is 36-63u of running against candidates
        // three hundred units away, so every arrival hits the clamp and
        // the two terms it normalises — half the score — go constant.
        let horizon = (probe / PlayerSkills::MIN_MAX_SPEED).max(1e-3);
        let arrival = |pos: Vector3<f32>, id: u32, from: Vector3<f32>| {
            let speed = ctx.tick_context.positions.players.max_speed(id).max(1e-3);
            ((pos - from).magnitude() / speed / horizon).min(1.0)
        };

        let mut best = Vector3::new(forward, 0.0, 0.0);
        let mut best_score = f32::MIN;
        for step in -Self::FAN_STEPS..=Self::FAN_STEPS {
            let angle = Self::FAN_SPREAD * step as f32 / Self::FAN_STEPS as f32;
            let dir = Vector3::new(forward * angle.cos(), angle.sin(), 0.0);
            let target = ball + dir * probe;
            // On the pitch, in both axes: an off-pitch Y folds `to_line`
            // to zero and scores the touchline term at its maximum, which
            // would make the widest candidate the best one every time.
            if target.x < 0.0 || target.x > field_width || target.y < 0.0 || target.y > field_height
            {
                continue;
            }

            let from_danger = ((target - own_goal).magnitude() / field_width).clamp(0.0, 1.0);

            let opponent_margin = ctx
                .players()
                .opponents()
                .all()
                .map(|o| arrival(target, o.id, o.position))
                .fold(1.0, f32::min);

            // …and the man kicking it is not the man who recovers it.
            let teammate_recovery = 1.0
                - ctx
                    .players()
                    .teammates()
                    .all()
                    .filter(|t| t.id != ctx.player.id)
                    .map(|t| arrival(target, t.id, t.position))
                    .fold(1.0, f32::min);

            // Out of play is safe, but only if it actually goes out.
            let to_line = target.y.min(field_height - target.y).max(0.0);
            let touchline = (1.0 - to_line / Self::TOUCHLINE_BAND).clamp(0.0, 1.0);

            // …and a leg in the way is not a clearance at all.
            let blocked = ctx
                .players()
                .opponents()
                .all()
                .filter(|o| {
                    let rel = o.position - ball;
                    let along = rel.x * dir.x + rel.y * dir.y;
                    along > 0.0 && along < probe
                })
                .map(|o| {
                    let rel = o.position - ball;
                    (rel.x * dir.y - rel.y * dir.x).abs()
                })
                .fold(f32::MAX, f32::min);
            let block_risk = (1.0 - blocked / Self::BLOCK_CORRIDOR).clamp(0.0, 1.0);

            let score = 0.40 * from_danger
                + 0.30 * opponent_margin
                + 0.20 * teammate_recovery
                + 0.10 * touchline
                - 0.35 * block_risk;
            if score > best_score {
                best_score = score;
                best = dir;
            }
        }
        best
    }
}

use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperSweepLimit,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    Ball, ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// How long the contact and the landing take.
///
/// UNITS: AI ticks, and the AI runs on every SECOND engine tick, so one is
/// **20 ms** — 22 is ~0.44 s, quicker than a dive because he is punching
/// from his feet or from a leap rather than lying on the floor with the
/// ball. A punch used to resolve and leave inside a single tick, which is
/// invisible and let him re-enter the state on the next one. It is a
/// committed action (`PlayerState::is_committed_action`) so nothing else
/// can pull him out of it either.
const MIN_PUNCH_TICKS: u64 = 22;

#[derive(Default, Clone)]
pub struct GoalkeeperPunchingState {}

impl StateProcessingHandler for GoalkeeperPunchingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        // How far he can put a fist on it. `effective_punch_distance` is
        // the engine's own number for that (8-28u, i.e. 1-3.5 m) and it is
        // what the rest of the goalkeeping model uses.
        //
        // The local formula this replaces was `2.0 * (0.85 + …)` — **21 to
        // 39 CENTIMETRES**, written when units were being read as metres.
        // A ball that close is in his chest, not at the end of a fist, so
        // the branch below fired on essentially every punch and sent him
        // to `Jumping` instead: a standing vertical leap, on the spot,
        // right after he had already made contact. That is the keeper
        // hopping in the air for no reason after a parry.
        // ONE PUNCH PER ENTRY. The contact resolves on the tick he arrives
        // and the rest of the state is the follow-through: he is landing
        // and getting up, not winding up again. Without this the roll below
        // re-ran every tick the ball stayed in reach and could strike it
        // repeatedly, and the state's exit put him back on his feet inside
        // 20 ms — invisible, and free to re-enter on the next tick.
        if ctx.in_state_time > 0 {
            if ctx.in_state_time < MIN_PUNCH_TICKS {
                return None;
            }
            return Some(StateChangeResult::with_goalkeeper_state(
                // Out of the space he defends → jog back; otherwise he is
                // already where he wants to be. See [`KeeperSweepLimit`].
                if KeeperSweepLimit::is_within(ctx, &prof) {
                    GoalkeeperState::Standing
                } else {
                    GoalkeeperState::ReturningToGoal
                },
            ));
        }

        // Out of reach on arrival: he has already made his contact — this
        // is the route the physics parry uses, and the ball is out there
        // BECAUSE he punched it. Hold the follow-through and get up.
        //
        // The bar this replaces was `2.0 * (0.85 + …)` — **21 to 39
        // CENTIMETRES**, written when units were being read as metres. A
        // ball that close is in his chest, not at the end of a fist, so it
        // was true of essentially every punch and sent him to `Jumping`
        // instead: a standing vertical leap, on the spot, right after he
        // had already made contact. That is the keeper hopping in the air
        // for no reason after a parry. `effective_punch_distance` is the
        // engine's own number for a fist's reach (8-28u, i.e. 1-3.5 m).
        if ctx.ball().distance() > prof.effective_punch_distance {
            return None;
        }

        // A fist connects with what a fist can REACH. `distance()` is
        // horizontal, so a delivery arcing five or six metres overhead
        // passed the check and the punch resolved against a ball no
        // keeper could touch — the delivery trace showed corner-won
        // crosses being "punched" out of their flight at z 5.4-6.5 m.
        // The leap ceiling is the honest vertical bar.
        let ball_z = ctx.tick_context.positions.ball.position.z;
        if ball_z
            > crate::r#match::goalkeepers::states::common::KeeperAerialClaim::leap_ceiling(
                ctx.player.skills.physical.jumping,
            )
        {
            return None;
        }

        // …and a delivery an engine-level aerial contest awarded to an
        // opponent is not his to punch: his claim chance was priced into
        // that contest. Double-jeopardy rule, same as the claim assess.
        if ctx
            .tick_context
            .ball
            .aerial_contest_winner
            .is_some_and(|w| w != ctx.player.id)
        {
            return None;
        }

        // Crowd / pressure from nearby opponents — high crowd punishes
        // weak claim quality more.
        let crowd = (ctx.players().opponents().nearby(8.0).count() as f32 / 4.0).clamp(0.0, 1.0);
        let claim_quality = (prof.aerial_command * 0.46
            + prof.positioning * 0.18
            + prof.rushing_out_profile * 0.12
            + prof.communication * 0.10
            + prof.condition_mult * 0.06)
            .clamp(0.0, 1.0);
        let claim_difficulty =
            (crowd * 0.32 + (1.0 - prof.parry_control) * 0.20 + prof.poor_skill_penalty * 0.18)
                .clamp(0.0, 1.0);
        // Punch success rolls between ~0.40 (weak in heavy traffic) and
        // ~0.92 (elite in space).
        let punch_success_prob =
            (claim_quality * 0.85 - claim_difficulty * 0.30 + prof.elite_lift).clamp(0.20, 0.95);
        let punch_success = ctx.context.rng.unit_f32() < punch_success_prob;

        if punch_success {
            // Contact made. He STAYS in the punch — returning his own
            // state is a no-op transition for the processor, so the event
            // fires and the duration branch at the top of `process` owns
            // getting him back on his feet. Exiting to `Standing` here put
            // him upright on the same tick his fist met the ball.
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::KeeperActionDiag::note(6);
            let mut state_change =
                StateChangeResult::with_goalkeeper_state(GoalkeeperState::Punching);

            // Punch AWAY from the own goal, through the ball, with real
            // loft. `direction_to_own_goal()` returns the own-goal
            // POSITION (not a direction), so the outward line is
            // ball-minus-goal. A punch is a shorter, flatter contact than
            // the Clearing hoof: strong aerial keepers push it toward
            // midfield, weak ones only shift the danger a few metres.
            //
            // Both components were pre-metric, calibrated against the
            // Clearing hoof this comment used to quote — and that hoof
            // was itself a 10 km rocket (see `GoalkeeperClearingState`).
            // A `z` of 2.6 is 260 m/s, an apex of ~3.4 km. Solved from an
            // apex now, like every other kick in the engine.
            let ball_pos = ctx.tick_context.positions.ball.position;
            let own_goal = ctx.ball().direction_to_own_goal();
            let fallback_x = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
            let outward = Vector3::new(ball_pos.x - own_goal.x, ball_pos.y - own_goal.y, 0.0);
            let outward_dir = outward
                .try_normalize(1e-4)
                .unwrap_or_else(|| Vector3::new(fallback_x, 0.0, 0.0));
            // Lateral spray — a punch is a deflection under pressure,
            // not an aimed pass.
            let y_jitter: f32 = ctx.context.rng.random_range(-0.35..0.35);
            let punch_dir = Vector3::new(outward_dir.x, outward_dir.y + y_jitter, 0.0)
                .try_normalize(1e-4)
                .unwrap_or(outward_dir);
            // Flatter and shorter than the kicked clearance: a punch is
            // struck off the fist under pressure, so it goes up 6-10 m
            // and travels 15-25 m rather than clearing the halfway line.
            let punch_apex = 6.0 + prof.aerial_command * 4.0;
            let punch_vz = Ball::launch_speed_for_apex(punch_apex);
            let punch_range = 120.0 + prof.aerial_command * 80.0; // 15 - 25 m
            let hang = Ball::hang_ticks(punch_vz).max(1.0);
            let punch_power = (punch_range / hang).clamp(0.30, 2.2);
            let punch_velocity = Vector3::new(
                punch_dir.x * punch_power,
                punch_dir.y * punch_power,
                punch_vz,
            );

            // Generate a punch event
            state_change
                .events
                .add_player_event(PlayerEvent::ClearBall(punch_velocity));

            Some(state_change)
        } else {
            // Punch failed, transition to appropriate state (e.g., Diving)
            Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Diving,
            ))
        }
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Remain stationary while punching
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Punching is a very high intensity activity requiring explosive effort
        GoalkeeperCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

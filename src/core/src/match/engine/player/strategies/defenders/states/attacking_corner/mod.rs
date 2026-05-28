use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::events::Event;
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::players::ShotType;
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// Penetration depth (units from the goal line) for the attacking run —
/// the six-yard-box / near-penalty-spot zone where corner headers are
/// actually scored. Closer than the old 34u (≈8.5m, a long header at a
/// wide angle that converted at ~1%): a CB who wins the aerial here gets a
/// prime-distance header.
const BOX_ATTACK_DEPTH: f32 = 24.0;
/// A jumping attacker reaches a touch further than the 1.5u of a standing
/// defensive header, so an arriving CB contests the cross from here.
const HEADER_REACH: f32 = 6.0;
const HEADER_HEIGHT: f32 = 1.4;
/// Near / far post split from the goal-centre line. Enough that the two
/// CBs don't stack, but tight enough that the header is from a decent
/// central-ish angle (the old 0.13 put them ~27° off-centre, which gutted
/// the header xG). Paired with the corner teleport in goal.rs.
const POST_SPLIT: f32 = 0.085;

/// A centre-back who has pushed up to attack an attacking corner. Holds a
/// central box position (near / far post split between the two CBs),
/// attacks the delivery with a header ON GOAL, and finishes if the ball
/// drops to their feet. Self-terminates the moment the corner is over so
/// the CB sprints straight back into shape — this state only exists for
/// the brief life of a corner, keeping the calibrated open-play defence
/// completely untouched.
#[derive(Default, Clone)]
pub struct DefenderAttackingCornerState {}

impl StateProcessingHandler for DefenderAttackingCornerState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        #[cfg(feature = "match-logs")]
        {
            use std::sync::atomic::Ordering;
            crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::DEF_CORNER_ATTACK_TICKS.fetch_add(1, Ordering::Relaxed);
        }

        // Received / won the ball in the box (cross controlled rather than
        // headed, short corner, knock-down) — SHOOT if there's a sight of
        // goal. Checked BEFORE the corner-over bail: receiving the cross
        // flips the restart to open play, and a CB who has just taken the
        // ball at the penalty spot should finish, not turn and run back.
        if ctx.player.has_ball(ctx) {
            let dist = ctx.ball().distance_to_opponent_goal();
            if dist < 80.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Shooting,
                ));
            }
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // Aerial delivery within a jumping CB's reach — head it ON GOAL.
        // Emitted directly (rather than routing to the defensive heading
        // state, which only contacts within 1.5u and clears away from our
        // own goal) so the attacker gets a realistic reach and the header
        // is a genuine shot. xG / accuracy of the header is resolved by
        // the shooting pipeline, so a poor header still mostly misses.
        let ball_pos = ctx.tick_context.positions.ball.position;
        if ball_pos.z >= HEADER_HEIGHT && ctx.ball().distance() < HEADER_REACH {
            #[cfg(feature = "match-logs")]
            {
                use std::sync::atomic::Ordering;
                crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::DEF_CORNER_HEAD_CHANCE.fetch_add(1, Ordering::Relaxed);
            }
            if self.win_header(ctx) {
                #[cfg(feature = "match-logs")]
                {
                    use std::sync::atomic::Ordering;
                    crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::DEF_CORNER_HEADER.fetch_add(1, Ordering::Relaxed);
                }
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::AttackingCorner,
                    Event::PlayerEvent(PlayerEvent::Shoot(
                        ShootingEventContext::new()
                            .with_player_id(ctx.player.id)
                            .with_target(ctx.player().shooting_direction())
                            .with_reason("DEF_CORNER_HEADER")
                            .with_shot_type(ShotType::Header)
                            .build(ctx),
                    )),
                ));
            }
            // Mistimed the jump — stay and contest the second ball.
            return None;
        }

        // Corner over (ball controlled in open play / possession lost) —
        // recover defensive shape immediately.
        if !ctx.ball().is_team_attacking_corner() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }

        // A ground ball loose right at our feet — pounce on it.
        if !ctx.ball().is_owned() && ctx.ball().distance() < 8.0 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let ball_vel = ctx.tick_context.positions.ball.velocity;

        // Attack an incoming aerial cross at its projected landing spot so
        // the header is timed, not reactive.
        if ball_pos.z >= 1.0 && ctx.ball().is_in_flight() {
            let target = if ball_vel.z < 0.0 {
                let gravity = 9.81;
                let t = (-ball_vel.z / gravity).max(0.0);
                Vector3::new(
                    ball_pos.x + ball_vel.x * t,
                    ball_pos.y + ball_vel.y * t,
                    0.0,
                )
            } else {
                Vector3::new(
                    ball_pos.x + ball_vel.x * 0.4,
                    ball_pos.y + ball_vel.y * 0.4,
                    0.0,
                )
            };
            return Some(
                SteeringBehavior::Pursuit {
                    target,
                    target_velocity: ball_vel,
                }
                .calculate(ctx.player)
                .velocity,
            );
        }

        // Otherwise hold the assigned box-attack position (near / far post).
        let target = self.box_attack_target(ctx);
        Some(
            SteeringBehavior::Arrive {
                target,
                slowing_distance: 8.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Sprinting up for a corner and jumping for the header is high
        // intensity — this run should cost real stamina.
        DefenderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl DefenderAttackingCornerState {
    /// Did the CB win the header contact? Heading + jumping skill roll —
    /// the resulting shot's accuracy is graded separately by the shooting
    /// pipeline, so this only governs whether contact is made at all.
    fn win_header(&self, ctx: &StateProcessingContext) -> bool {
        let heading = ctx.player.skills.technical.heading / 20.0;
        let jumping = ctx.player.skills.physical.jumping / 20.0;
        // The discrete corner contest (engine `resolve_corner_contest`) has
        // ALREADY decided this CB won the aerial duel and dropped the ball
        // on their head — so this only models making CLEAN CONTACT, which a
        // player who won the jump usually does. A high floor avoids
        // double-jeopardy (the old 0.2-0.9 roll silently killed ~40% of
        // won headers). The resulting header's xG / accuracy is still graded
        // by the shooting pipeline, so a poor header mostly misses anyway.
        let p = (0.62 + (heading + jumping) * 0.5 * 0.30).clamp(0.55, 0.95);
        ctx.context.rng.unit_f32() < p
    }

    /// Central box target. The two centre-backs split near / far post by
    /// their starting side so they don't stack, both at penalty-spot
    /// depth — inside the delivery zone and within heading range.
    fn box_attack_target(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let goal = ctx.player().opponent_goal_position();
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let center_y = field_height / 2.0;

        let dir = match ctx.player.side {
            Some(PlayerSide::Left) => 1.0,
            Some(PlayerSide::Right) => -1.0,
            None => 0.0,
        };

        let near_post = ctx.player.start_position.y < center_y;
        let y_off = if near_post {
            -field_height * POST_SPLIT
        } else {
            field_height * POST_SPLIT
        };

        Vector3::new(
            (goal.x - dir * BOX_ATTACK_DEPTH).clamp(10.0, field_width - 10.0),
            (center_y + y_off).clamp(10.0, field_height - 10.0),
            0.0,
        )
    }
}

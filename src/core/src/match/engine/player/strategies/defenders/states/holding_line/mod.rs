use nalgebra::Vector3;

use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{
    ActivityIntensity, DefenderCondition, DefensiveLine,
};
use crate::r#match::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::players::DefensiveRole;
use crate::r#match::{
    ConditionContext, DefensiveDuty, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};

// Line deviation now lives on `DefensiveLine` (shared with the Running
// state, with hysteresis) — see `defenders/states/common`.
const BALL_PROXIMITY_THRESHOLD: f32 = 150.0; // React to ball from further out
const MARKING_DISTANCE_THRESHOLD: f32 = 50.0; // Pick up attackers from further away
const PRESSING_DISTANCE_THRESHOLD: f32 = 60.0; // Step out to press ball carrier earlier
const DANGEROUS_RUN_SCAN_DISTANCE: f32 = 100.0; // Scan wider for dangerous runs
const DANGEROUS_RUN_SPEED: f32 = 0.40; // 5 m/s in u/tick (1u=0.125m, 10ms tick) — a genuine attacking run. Old values 1.0-3.0 exceeded human max speed (0.63 u/tick), so run-tracking never fired.
const DANGEROUS_RUN_ANGLE: f32 = 0.5; // Wider angle detection for goal-bound runs
// Aerial-contest band — same values as the Marking / Intercepting
// hand-offs so an incoming cross reads identically from every defensive
// state.
const AERIAL_HEADING_HEIGHT: f32 = 1.5;
const AERIAL_HEADING_DISTANCE: f32 = 5.0;
/// How far off the line a covering defender drops (~4.5 m). This is the
/// near end of the back line's diagonal.
const COVER_DROP: f32 = 36.0;
/// How far the far-side defender tucks back when the ball is fully on the
/// opposite flank (~7.5 m). Scaled continuously by how far across the
/// pitch the ball actually is, so the diagonal grows and shrinks with the
/// play instead of switching on.
const FAR_SIDE_DROP: f32 = 60.0;

#[derive(Default, Clone)]
pub struct DefenderHoldingLineState {}

impl StateProcessingHandler for DefenderHoldingLineState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Attacking corner: centre-backs push up to attack the delivery
        // (self-terminates the instant the corner is over).
        if !ctx.player.has_ball(ctx)
            && ctx
                .player
                .tactical_position
                .current_position
                .is_central_defender()
            && ctx.ball().is_team_attacking_corner()
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::AttackingCorner,
            ));
        }

        // AERIAL BALL — a cross or long ball dropping onto the line is
        // attacked in the air. Holding the line was the most-occupied
        // defensive state yet had no heading exit, so a defender sitting
        // in shape watched crosses drop past their head and only reacted
        // once the ball was on the floor.
        let ball_position = ctx.tick_context.positions.ball.position;
        if ball_position.z > AERIAL_HEADING_HEIGHT
            && ctx.ball().distance() < AERIAL_HEADING_DISTANCE
            && ctx.ball().is_towards_player_with_angle(0.6)
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Heading,
            ));
        }

        // BOX EMERGENCY — ball is in our penalty area with an opposing
        // carrier. Break shape and engage. The two closest defenders
        // attack; the rest hold line so the far side isn't exposed.
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

        // STEP UP — attacker is approaching the penalty area and I'm
        // the closest defender. Meet them outside the box instead of
        // collapsing deep. Real football: defenders engage at the 18-yard
        // line, not at the 6-yard line.
        if ctx.player().defensive().should_step_up_to_meet_attacker() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
        }

        // Off the line — run back to it. `DefensiveLine::is_in_line`
        // is the SAME predicate `DefenderRunningState` uses to decide
        // we've arrived, so the two states can no longer disagree and
        // bounce the defender between them every tick (see
        // `DefensiveLine`). Holding is the current state here, so this
        // reads the generous `EXIT_BAND`.
        if !DefensiveLine::is_in_line(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // Loose ball nearby — go claim it directly
        if !ctx.ball().is_owned() && ctx.ball().distance() < 40.0 && ctx.ball().speed() < 3.0 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
        }

        // Role-driven engagement: if the opponent has the ball, break
        // from the line according to our defensive role. A counter-press
        // window widens the trigger so even distant defenders commit to
        // chasing just after we lose possession.
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let distance = opponent_with_ball.distance(ctx);

            // Tackle range — but only if I'm the Primary closer than
            // any teammate. Otherwise I hold the line while the closer
            // defender engages. Stops the whole back four lunging at
            // the same carrier.
            let is_primary = matches!(
                ctx.player().defensive().defensive_role_for_ball_carrier(),
                DefensiveRole::Primary
            );
            if distance < 25.0 && is_primary {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Tackling,
                ));
            }

            let counter_press_active = ctx.team().counterpress_window();
            let counter_press_range = if counter_press_active {
                35.0 + ctx.team().press_intensity() * 55.0
            } else {
                0.0
            };

            match ctx.player().defensive().defensive_role_for_ball_carrier() {
                DefensiveRole::Primary => {
                    if distance < PRESSING_DISTANCE_THRESHOLD
                        || (counter_press_active && ctx.ball().distance() < counter_press_range)
                    {
                        return Some(StateChangeResult::with_defender_state(
                            DefenderState::Pressing,
                        ));
                    }
                    // Primary but the carrier is still beyond pressing
                    // range: hold the line and let him come. This used to
                    // return `Running`, and `DefenderRunningState` — for a
                    // defender who is in the line, in a settled block,
                    // with the ball more than 60u away — sends him
                    // straight back to `HoldingLine`. Neither view was
                    // wrong on its own; together they were a two-cycle
                    // that survived the shared-line fix and still ran
                    // ~55k round trips a match.
                    //
                    // Holding is the correct half of the disagreement: a
                    // block's nearest defender does not sprint 60u+ at a
                    // carrier, he keeps his shape until the carrier
                    // arrives — at which point the `Pressing` branch
                    // above fires. Fall through to the dangerous-run,
                    // through-ball and ball-proximity checks below, which
                    // are exactly the reasons he SHOULD break early.
                }
                DefensiveRole::Cover => {
                    if distance < 100.0 && ctx.ball().on_own_side() {
                        return Some(StateChangeResult::with_defender_state(
                            DefenderState::Covering,
                        ));
                    }
                }
                DefensiveRole::Help => {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Marking,
                    ));
                }
                DefensiveRole::Hold => {
                    // Stay on the line — fall through to run/guard checks
                    // for secondary threats below.
                }
            }
        }

        // Break line to track dangerous runners if we're the best
        // positioned defender for them (no ball carrier scenario, or
        // our role was Hold).
        if let Some(dangerous_runner) = self.scan_for_dangerous_runs(ctx) {
            let distance_to_runner = dangerous_runner.distance(ctx);
            if distance_to_runner < 25.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                ));
            }
        }

        // Guard unmarked attackers in our zone who are trying to get open
        if ctx.ball().on_own_side() {
            if let Some(unmarked) = ctx
                .player()
                .defensive()
                .find_unmarked_opponent(MARKING_DISTANCE_THRESHOLD * 2.0)
            {
                let dist = unmarked.distance(ctx);
                if dist < 60.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Guarding,
                    ));
                }
            }
        }

        // React to balls played behind the defensive line (through balls)
        if !ctx.ball().is_owned() && ctx.ball().speed() > 1.0 {
            let ball_pos = ctx.tick_context.positions.ball.position;
            let ball_vel = ctx.tick_context.positions.ball.velocity;
            let own_goal = ctx.ball().direction_to_own_goal();
            let to_goal = (own_goal - ball_pos).normalize();
            let ball_dir = ball_vel.normalize();
            let heading_toward_goal = ball_dir.dot(&to_goal);

            // Ball is moving toward our goal and is close enough to react
            if heading_toward_goal > 0.4 && ctx.ball().distance() < 200.0 {
                // Check if ball is behind us or at our level (through ball)
                let is_behind_or_level = if own_goal.x < ctx.context.field_size.width as f32 / 2.0 {
                    ball_pos.x < ctx.player.position.x + 15.0
                } else {
                    ball_pos.x > ctx.player.position.x - 15.0
                };

                if is_behind_or_level {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Intercepting,
                    ));
                }
            }
        }

        if ctx.ball().distance() < 250.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Intercepting,
            ));
        }

        if ctx.ball().distance() < BALL_PROXIMITY_THRESHOLD {
            let opponent_nearby = self.is_opponent_nearby(ctx);
            return Some(StateChangeResult::with_defender_state(if opponent_nearby {
                DefenderState::Marking
            } else {
                DefenderState::Intercepting
            }));
        }

        // Offside-trap detection used to transition into a dedicated
        // DefenderState::OffsideTrap that was a pass-through — it just
        // bounced back to HoldingLine. Staying in HoldingLine with the
        // same zonal-line logic is the simpler model; if we want trap
        // pressing later, reintroduce as a team-level flag (Phase 2).
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let current_position = ctx.player.position;
        let ball_position = ctx.tick_context.positions.ball.position;

        // Calculate target position based on zonal coverage
        let target_position = self.calculate_zonal_position(ctx, ball_position);

        let to_target = target_position - current_position;
        let distance = to_target.magnitude();

        // Define thresholds for movement
        const MIN_DISTANCE_THRESHOLD: f32 = 1.0;
        const SLOWING_DISTANCE: f32 = 5.0;

        // Base movement speed — line_holding_mult bakes condition,
        // concentration curve, and fatigue into a 0.75..1.03 multiplier.
        // Replaces the raw pace/20 clamp so a tired CB shuffles slower
        // and a fresh, concentrated CB holds the line crisply.
        let def_profile = DefenderSkillProfile::from_ctx(ctx);
        let pace_influence = (ctx.player.skills.physical.pace / 20.0).clamp(0.6, 1.2);
        let base_speed = 3.0 * pace_influence * def_profile.line_holding_mult;

        if distance > MIN_DISTANCE_THRESHOLD {
            let direction = to_target.normalize();

            // Speed factor - slow down as approaching target
            let speed_factor = if distance > SLOWING_DISTANCE {
                1.0
            } else {
                (distance / SLOWING_DISTANCE).clamp(0.25, 1.0)
            };

            Some(direction * base_speed * speed_factor)
        } else {
            // In position - stay still (no artificial jitter)
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Holding line involves minimal movement - allows for recovery
        DefenderCondition::with_velocity(ActivityIntensity::Recovery).process(ctx);
    }
}

impl DefenderHoldingLineState {
    /// Calculate zonal defensive position - creates natural staggered formation
    /// Each defender positions based on their assigned opponent or zone
    fn calculate_zonal_position(
        &self,
        ctx: &StateProcessingContext,
        ball_position: Vector3<f32>,
    ) -> Vector3<f32> {
        let tactical_position = ctx.player.start_position;
        let current_position = ctx.player.position;
        let own_goal = ctx.ball().direction_to_own_goal();
        let field_center_y = ctx.context.field_size.height as f32 / 2.0;

        // Determine if this defender is a wide defender (fullback) or central
        let distance_from_center = (tactical_position.y - field_center_y).abs();
        let is_wide_defender = distance_from_center > 40.0;

        // Find the nearest opponent in this defender's zone
        let zone_half_width = 50.0;
        let nearest_opponent_in_zone = ctx
            .players()
            .opponents()
            .nearby(100.0)
            .filter(|opp| {
                let lateral_dist = (opp.position.y - tactical_position.y).abs();
                lateral_dist < zone_half_width
            })
            .min_by(|a, b| {
                let dist_a = (a.position - current_position).magnitude();
                let dist_b = (b.position - current_position).magnitude();
                dist_a.total_cmp(&dist_b)
            });

        // DEPTH (X) CALCULATION
        let target_x = if let Some(opponent) = nearest_opponent_in_zone {
            // Track the opponent - position between them and goal
            let opponent_x = opponent.position.x;
            let goal_x = own_goal.x;

            // Position goal-side of opponent (between opponent and goal)
            let goal_direction = (goal_x - opponent_x).signum();
            let marking_offset = 8.0 * goal_direction; // Stay 8 units goal-side

            opponent_x + marking_offset
        } else {
            // No opponent in zone - use base tactical position with ball influence
            let ball_influence = (ball_position.x - tactical_position.x) * 0.2;

            // Wide defenders push up more when ball is on their side.
            // "Up" is toward the opponent goal, so the depth offset is
            // signed by the side's forward direction — a raw +x push
            // sent Right-team fullbacks the wrong way (deeper when the
            // ball was on their flank).
            let forward_sign = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
            let wide_push = if is_wide_defender {
                let ball_on_my_side = (ball_position.y - field_center_y).signum()
                    == (tactical_position.y - field_center_y).signum();
                (if ball_on_my_side { 15.0 } else { -5.0 }) * forward_sign
            } else {
                0.0
            };

            tactical_position.x + ball_influence + wide_push
        };

        // LATERAL (Y) CALCULATION
        // compactness_target is the team-shared signal (tactic + phase +
        // game-management). Polish spec: shift = ball_offset * (0.08 +
        // compactness_target * 0.18) so a fully-compact low-block
        // squeezes harder toward the ball side than the legacy 0.12 cap.
        let compactness = ctx.team().compactness_target();

        // ── THE LINE HAS TO NARROW, NOT JUST SLIDE ───────────────────
        //
        // Every lateral term below is an offset from `tactical_position.y`
        // — the KICKOFF FORMATION SLOT — and `shift` applies the same
        // translation to all four defenders. So the back line moved toward
        // the ball as a rigid body and its WIDTH never changed: it was the
        // kickoff width in the 90th minute of a goalmouth siege.
        //
        // Measured: widest gap between adjacent defenders 147u (18.4 m)
        // against a real 3-8 m, 54% of attackers in our own third with
        // nobody within 3 m, and — the consequence that matters — of the
        // opponents inside the block window when a shot is struck, 95.8%
        // are wider than the corridor, mean 102u (12.8 m) off the line.
        // Defenders are not in front of shots because the back line has
        // 18-metre holes in it, so only 0.9% of shots are blocked against
        // a real 18-22%.
        //
        // A defending back four is not its kickoff shape. It squeezes
        // toward its own centre as the ball comes at it — the far-side
        // full-back tucks in, and the unit that spans 50 m at kickoff
        // defends its own box across barely 30. So the slot is pulled
        // toward the middle before anything else is applied to it, by how
        // deep the danger is and how compact the side wants to be.
        let field_len = ctx.context.field_size.width as f32;
        let danger = (1.0 - (ball_position.x - own_goal.x).abs() / field_len.max(1.0))
            .clamp(0.0, 1.0);
        let squeeze = 1.0 - (0.20 + compactness * 0.25) * danger;
        let slot_y = field_center_y + (tactical_position.y - field_center_y) * squeeze;

        let target_y = if let Some(opponent) = nearest_opponent_in_zone {
            // Track opponent laterally but don't go too far from zone.
            //
            // `max_drift` was 25u — 3.1 m — while the zone this defender
            // is responsible for is 50u wide, so he could see a man in his
            // zone and was forbidden from getting to him. That is the
            // marking-duel number: markers sat 6.4 m off their man and the
            // attacker had got away on 47% of samples. He is allowed to go
            // as far as his zone extends; beyond it the man belongs to
            // somebody else.
            let opponent_y = opponent.position.y;
            let max_drift = zone_half_width;
            let drift = (opponent_y - slot_y).clamp(-max_drift, max_drift);
            slot_y + drift
        } else {
            let ball_offset = ball_position.y - field_center_y;
            let shift = ball_offset * (0.08 + compactness * 0.18);
            slot_y + shift
        };

        // ── ROLE STAGGER ─────────────────────────────────────────────
        //
        // Everything above computes depth from the ball and lateral
        // position from `start_position` — the kickoff formation slot.
        // Four defenders whose slots differ by a constant therefore hold
        // a line whose SHAPE is a constant too: flat, evenly spaced, and
        // identical in every situation the match produces.
        //
        // A real back line is never flat. It is a diagonal, and the
        // diagonal comes from ROLES: the man covering the presser drops
        // off him, and the far-side full-back tucks in and drops deeper
        // still because the ball cannot reach him quickly. Both are depth
        // offsets relative to what this defender is DOING, which is
        // exactly the quantity the formation slot cannot express.
        let stagger = match ctx.team().my_duty() {
            // Covering — sit off the line, behind whoever is engaging.
            DefensiveDuty::Cover => COVER_DROP,
            // Holding the far side of a ball-side overload: tuck in and
            // drop, scaled by how far across the pitch the ball is, so
            // the diagonal is proportional rather than switched on.
            DefensiveDuty::HoldZone => {
                let ball_offset = (ball_position.y - field_center_y) / field_center_y.max(1.0);
                let my_offset = (ctx.player.position.y - field_center_y) / field_center_y.max(1.0);
                // Positive when the ball is on the opposite flank to me.
                let far_side = (-ball_offset * my_offset).clamp(0.0, 1.0);
                far_side * FAR_SIDE_DROP
            }
            // A presser or a marker is where his job is; his depth is
            // already set by the duty anchor.
            _ => 0.0,
        };
        let forward_sign = ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
        let staggered_x = target_x - forward_sign * stagger;

        // Through the same unit constraint the individual-duty states now
        // use, so the whole back line — whatever each man happens to be
        // doing — reads ONE reference for its depth and its width. The
        // role stagger above still shapes the diagonal; `hold_shape` only
        // bounds how far the result may sit from the line.
        DefensiveLine::hold_shape(ctx, Vector3::new(staggered_x, target_y, 0.0))
    }

    /// Checks if an opponent player is nearby within the MARKING_DISTANCE_THRESHOLD.
    fn is_opponent_nearby(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().exists(MARKING_DISTANCE_THRESHOLD)
    }

    /// Scan for opponents making dangerous runs toward goal
    fn scan_for_dangerous_runs(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let own_goal_position = ctx.ball().direction_to_own_goal();

        ctx.players()
            .opponents()
            .nearby(DANGEROUS_RUN_SCAN_DISTANCE)
            .filter(|opp| {
                let velocity = opp.velocity(ctx);
                let speed = velocity.norm();

                if speed < DANGEROUS_RUN_SPEED {
                    return false;
                }

                let to_goal = (own_goal_position - opp.position).normalize();
                let velocity_dir = velocity.normalize();
                let alignment = velocity_dir.dot(&to_goal);

                if alignment < DANGEROUS_RUN_ANGLE {
                    return false;
                }

                let defender_x = ctx.player.position.x;
                let is_ahead_or_close =
                    if own_goal_position.x < ctx.context.field_size.width as f32 / 2.0 {
                        opp.position.x < defender_x + 30.0
                    } else {
                        opp.position.x > defender_x - 30.0
                    };

                alignment >= DANGEROUS_RUN_ANGLE && is_ahead_or_close
            })
            .min_by(|a, b| {
                let dist_a = a.distance(ctx);
                let dist_b = b.distance(ctx);
                dist_a.total_cmp(&dist_b)
            })
    }
}

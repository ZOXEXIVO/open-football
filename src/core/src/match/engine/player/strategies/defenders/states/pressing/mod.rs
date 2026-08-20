use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::common::states::TackleEngagement;
use crate::r#match::player::strategies::players::DefensiveRole;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

// Commit distance moved to `TackleEngagement::COMMIT` — same value,
// now shared with `DefenderTacklingState` so the hand-off is ordered.
const BASE_PRESSING_DISTANCE: f32 = 45.0;
const MAX_PRESSING_BONUS: f32 = 35.0; // effective range: 45-80
const BASE_PRESSING_DISTANCE_DEFENSIVE_THIRD: f32 = 40.0;
const MAX_PRESSING_BONUS_DEFENSIVE_THIRD: f32 = 30.0; // effective range: 40-70
const CLOSE_PRESSING_DISTANCE: f32 = 25.0; // Wider close pressing zone for tight approach
const STAMINA_THRESHOLD: f32 = 25.0; // Press until truly exhausted. Lowered
// from 30% to match hysteresis with Resting's 45% crisis re-entry
// gate (see `defenders/resting/mod.rs`) — the 25%–45% band is a
// "stay put, slow walk" zone that prevents Pressing↔Resting flicker
// when a crisis stays active while the defender is exhausted.
const FIELD_THIRD_THRESHOLD: f32 = 0.33;

#[derive(Default, Clone)]
pub struct DefenderPressingState {}

impl StateProcessingHandler for DefenderPressingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // A player who has won the ball is not doing this any more.
        //
        // Nothing here asked whether HE had it, so a defender who
        // intercepted or was simply the nearest body when it arrived
        // carried on with an off-ball job while holding it — the same
        // fixed point `Defender: Marking` was measured freezing on
        // (99% of its stuck ticks with the owner 250-plus AI ticks into
        // the state). `Running` is where a defender's on-ball decisions
        // live, and this is the hand-off `DefenderStandingState` has
        // always made.
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // 1. Skill-aware fatigue gate. Stop pressing when the defender's
        // condition multiplier (stamina/natural_fitness/match_readiness/
        // determination blend with the fatigue curve) drops below the
        // sustainable-press floor. Floors map roughly to 25% raw
        // condition for a baseline-fitness CB and lower for elite-fit
        // ones — preserving the previous hysteresis with Resting's
        // crisis re-entry but skill-graded.
        let def_profile = DefenderSkillProfile::from_ctx(ctx);
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina < STAMINA_THRESHOLD || def_profile.def_condition_mult < 0.55 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Resting,
            ));
        }

        // 2. Back off during foul protection — don't crowd the free kick
        if ctx.ball().is_in_flight() && ctx.ball().is_owned() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ));
        }

        // 3. Identify the opponent player with the ball
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = opponent.distance(ctx);

            // If close enough to tackle, transition to Tackling state.
            // Repeat-tackle prevention lives on the player via
            // `tackle_cooldown` — a single-state cooldown here wouldn't
            // cover the Standing/Running/Covering re-entry paths — and
            // `should_commit` reads it here too, so a defender who has
            // just lunged and missed contains the carrier instead of
            // being handed into a `Tackling` state whose first act is to
            // send him straight back.
            if TackleEngagement::should_commit(ctx, distance_to_opponent) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Tackling,
                ));
            }

            // Pressing distance scales with team intensity AND the
            // defender's own press_profile — work-rate/anticipation/
            // stamina/positioning blend with condition fatigue baked in.
            // Poor pressers run a smaller bubble; elite pressers commit
            // a bit further. Replaces the static base + intensity formula.
            let intensity = ctx.team().press_intensity();
            // Reuse `def_profile` from the top of process() — `from_ctx` is
            // pure over the frozen tick snapshot, so rebuilding here would
            // return the identical value.
            let profile_bonus = def_profile.press_profile * 18.0;
            let pressing_threshold = if ctx.ball().on_own_side()
                && ctx.ball().distance_to_own_goal()
                    < ctx.context.field_size.width as f32 * FIELD_THIRD_THRESHOLD
            {
                BASE_PRESSING_DISTANCE_DEFENSIVE_THIRD
                    + MAX_PRESSING_BONUS_DEFENSIVE_THIRD * intensity
                    + profile_bonus
            } else {
                BASE_PRESSING_DISTANCE + MAX_PRESSING_BONUS * intensity + profile_bonus
            };

            // If the opponent is too far away, stop pressing
            if distance_to_opponent > pressing_threshold {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                ));
            }

            // THE PLAN'S PRESSER DOES NOT GET RE-RANKED OUT OF PRESSING.
            //
            // The block below is a per-tick geometric rank over the back
            // line only, and it ejects anybody who is not the single
            // closest defender to the carrier. `DefensivePlan` nominates
            // the presser over the whole unit, on a 250 ms cadence, with
            // an incumbency bonus so the nomination does not swap on
            // sub-metre movement — and `TackleEngagement::may_engage_carrier`
            // then licenses ONLY that man to challenge. So the two
            // disagreeing meant the licensed presser could be handed out
            // of the state that takes him to the ball, by a rank that has
            // no say in whether he is allowed to challenge when he gets
            // there. Same "THE ASSIGNMENT WINS" precedence
            // `DefenderMarkingState::find_best_marking_target` opens with.
            if TackleEngagement::is_nominated_presser(ctx) {
                return None;
            }

            // Role-based coordination: a defender only stays in Pressing
            // while they're still the Primary for the current carrier.
            // When the carrier dribbles past (or another defender gets
            // closer), role flips to Cover/Help/Hold and we drop back —
            // Standing's role block will reassign us to the right state
            // on the next tick.
            match ctx.player().defensive().defensive_role_for_ball_carrier() {
                DefensiveRole::Primary => {
                    // Stay on the carrier — aggressive press continues.
                    None
                }
                DefensiveRole::Cover => Some(StateChangeResult::with_defender_state(
                    DefenderState::Covering,
                )),
                DefensiveRole::Help => Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                )),
                DefensiveRole::Hold => Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                )),
            }
        } else {
            // No opponent with the ball - ball might be loose
            // Check if we should intercept
            if !ctx.ball().is_owned() && ctx.ball().distance() < 50.0 && ctx.ball().speed() < 3.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::TakeBall,
                ));
            }
            if ctx.ball().distance() < 60.0 && !ctx.ball().is_owned() {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Intercepting,
                ));
            }
            // …AND A NOMINATED PRESSER IS NOT "NOTHING TO DO".
            //
            // Nobody owns the ball for well over half a match — ~1 700
            // passes at ~1.5 s of flight each — and `DutyAssigner`
            // deliberately nominates a presser onto a TRAVELLING ball for
            // exactly that reason: you go WITH the pass, not after it
            // lands. That change took press duties per refresh 0.25 →
            // 0.61.
            //
            // This branch then threw all of it away. A presser whose
            // carrier had just released the ball fell straight through to
            // `HoldingLine`, which declares `ActivityIntensity::Recovery`
            // — a hard **0.12 of top speed**, a slow walk — for as long as
            // he held it. So the man the plan had sent to the ball was
            // capped at walking pace on the half of the match the press is
            // FOR, and `Defender: Pressing` measured 0.5% of all AI ticks
            // against `Marking`'s 13.3%.
            //
            // `velocity()` already knows where to take him: `my_duty_anchor`
            // resolves a `Press` duty to the point of attack, and its own
            // doc says it was extended to cover the in-flight case for
            // precisely this reason.
            if TackleEngagement::is_nominated_presser(ctx) {
                return None;
            }
            Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ))
        }
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move towards the opponent with the ball

        let opponents = ctx.players().opponents();
        let mut opponent_with_ball = opponents.with_ball();

        if let Some(opponent) = opponent_with_ball.next() {
            let distance_to_opponent = opponent.distance(ctx);

            // Intercept the carrier's future position based on their
            // velocity — stops them slipping past because the defender
            // was aiming at where they WERE, not where they're going.
            // Previously this was a 5u goal-side offset which meant a
            // running defender arrived ~5u behind the carrier and kept
            // chasing without ever making contact.
            let opp_velocity = ctx.tick_context.positions.players.velocity(opponent.id);
            let opp_speed = opp_velocity.magnitude();
            // Press boost from the unified profile (press_profile +
            // mobility composite). Tired / low-stamina chasers drop
            // into a softer press; elite pressers genuinely outsprint
            // journeyman carriers. Stays bounded by the original
            // calibrated envelope.
            let minute = sc::minute_from_ms(ctx.context.total_match_time);
            let def_profile = DefenderSkillProfile::from_ctx(ctx);
            let mobility = sc::mobility(ctx.player, minute);
            let press_composite = 0.65 * def_profile.press_profile + 0.35 * mobility;
            let press_boost = (1.40 + (press_composite - 0.50) * 0.50).clamp(1.15, 1.65);
            // Goal-side bias only; `SteeringBehavior::Pursuit` below derives
            // the interception from the defender's real u/tick speed.
            //
            // The lead time used to be `distance / speed` where `speed` is
            // `pace * press_boost` — `pace` being a 1-20 SKILL, not a
            // velocity. Dividing a field-unit distance by a skill rating
            // produced a tick count roughly 30x short that lurched as the
            // gap closed, moving the aim point every tick.
            let predicted = opponent.position;

            // Bias predicted point toward the goal-side so we close the
            // shooting lane even on chase — the defender wants to be
            // BETWEEN the carrier and our goal, not just on top of them.
            // When the carrier is inside shooting range, ramp the
            // goal-side bias hard. This puts the defender squarely in the
            // shot line, which gives him a real chance to block the strike
            // via `try_block_shot`. Real football: a defender closing down
            // shows the shooter his body and steps along the shot line —
            // he doesn't just run at the ball.
            //
            // ⚠ WIDENING THIS TO REAL SHOOTING RANGE WAS TRIED AND
            // MEASURED NULL — do not retry it without a different reason.
            //
            // 80u is TEN METRES, and shots are struck from 124u (15.5 m)
            // on average (`block_diag::SHOT_RANGE_X100`, n=2 800 over 120
            // fixtures), so for the shot that actually happens this step
            // is off and the presser runs at the ball rather than across
            // it. That reads like the whole of the blocking problem, and
            // it is not: taking `SHOT_ZONE` to 240u (the 30 m
            // `DefenderMarkingState` uses for the same question) moved the
            // mean perpendicular distance of the defenders inside the
            // block window by 72.0u → **73.3u**, and blocks by 16.6% →
            // **16.5% of shots**. It cost 1.6 shots per team per match on
            // the way past (12.5 → 10.9 against a real ~13), because a
            // presser holding a containing position over the whole final
            // third suppresses the shot instead of blocking it.
            //
            // The reason is that the presser is ONE man. 41% of the back
            // line is `Marking` when a shot is struck and 14% `Pressing`,
            // so the corridor statistic is owned by the markers, whose
            // line is goal-side of their MAN and not of the ball. The
            // block rate lives in the marking geometry (see
            // `DefensiveLine::hold_shape_on_man`), not here.
            let own_goal = ctx.ball().direction_to_own_goal();
            let to_own_goal = (own_goal - predicted).normalize();
            let carrier_to_goal = (own_goal - predicted).magnitude();
            let shot_zone_bias = if carrier_to_goal < 80.0 {
                // In shot zone: step 8-12u goal-side so we're actually
                // in the shot corridor. Heavier bias closer to goal.
                let zone_factor = 1.0 - (carrier_to_goal / 80.0).clamp(0.0, 1.0);
                8.0 + zone_factor * 4.0
            } else if opp_speed > 0.1 {
                2.0
            } else {
                0.0
            };
            let intercept_target = predicted + to_own_goal * shot_zone_bias;

            // Steer rather than assign — see `MidfielderPressingState` for
            // the same change. `direction * speed` set an absolute
            // velocity of up to ~33 u/tick (pace 20 x press_boost 1.65)
            // against a ~0.63 u/tick top speed, ignoring the velocity the
            // defender already had, so the engine-wide clamp kept whatever
            // heading this tick produced. `press_boost` still applies, as
            // a multiplier on the achievable result.
            let pressing_velocity = SteeringBehavior::Pursuit {
                target: intercept_target,
                target_velocity: opp_velocity,
            }
            .calculate(ctx.player)
            .velocity
                * press_boost;

            // Reduce separation velocity when actively pressing to allow close approach
            // When very close, disable separation entirely to enable tackling
            let separation = if distance_to_opponent < CLOSE_PRESSING_DISTANCE {
                ctx.player().separation_velocity() * 0.05 // Almost no separation when actively pressing
            } else {
                ctx.player().separation_velocity() * 0.15 // Minimal separation when pressing
            };

            return Some(pressing_velocity + separation);
        }

        // Loose ball nearby — pursue it
        if !ctx.ball().is_owned() && ctx.ball().distance() < 80.0 {
            let direction =
                (ctx.tick_context.positions.ball.position - ctx.player.position).normalize();
            let speed = ctx.player.skills.physical.pace;
            return Some(direction * speed);
        }

        // Nobody on the ball and it is further out than the loose-ball
        // leg above: this is the presser going with a pass in flight.
        // `my_duty_anchor` resolves his `Press` duty to the point of
        // attack — the ball, when nobody owns it — which is the same
        // point `DutyAssigner` nominated him over, so the plan and the
        // run agree about where the press is happening.
        //
        // `Pursuit` rather than `Seek`, for the same reason the carrier
        // branch above uses it: a ball in flight is going somewhere, and
        // aiming at where it IS arrives behind it every time.
        if let Some(anchor) = ctx.team().my_duty_anchor() {
            return Some(
                SteeringBehavior::Pursuit {
                    target: anchor,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                .calculate(ctx.player)
                .velocity,
            );
        }

        None
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // ⚠ CLOSING DOWN THE MAN ON THE BALL IS A SPRINT, AND THIS IS A
        // SPEED CAP.
        //
        // `High` is 0.78 of top speed (`MovementEffort::speed_fraction`).
        // The man being pressed is on the ball, and a carrier's ceiling
        // is a flat-out sprint scaled only by the carry cost — so the
        // presser was forbidden, by the movement layer, from ever
        // arriving. Measured before this: the carrier's ceiling 0.525
        // u/tick against his nearest opponent's 0.450, with the chaser
        // slower on 89% of ticks (`dead_ball_diag::CHASE_SAMPLES`), and
        // defenders producing 25.2 pressures a match for 1.12 successes
        // against a real ~11 for ~3.5. He was pressing constantly and
        // winning nothing because he could not close the last two metres.
        //
        // Same defect and same fix as `DefenderMarkingState` — see the
        // note there. The tier is a CEILING, so this does not make
        // defenders sprint all match: a presser shepherding a carrier who
        // is walking still walks.
        DefenderCondition::with_velocity(ActivityIntensity::chase()).process(ctx);
    }
}

impl DefenderPressingState {}

use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::events::Event;
use crate::r#match::player::events::{FoulSeverity, PlayerEvent};
use crate::r#match::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

const TACKLE_DISTANCE_THRESHOLD: f32 = 10.0; // ~1.25m — proper engagement range.
// Previously 14u (~1.75m). Even with that, attempts ran at 174/team/match
// because longer possessions in attacking zones create more chances to
// hit the gate. 10u forces actual contact range — a defender who's
// still half a stride away has to keep pressing rather than committing
// to a slide that won't connect.
const PRESSING_DISTANCE: f32 = 80.0;
const RETURN_DISTANCE: f32 = 120.0;

#[derive(Default, Clone)]
pub struct DefenderTacklingState {}

impl StateProcessingHandler for DefenderTacklingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        #[cfg(feature = "match-logs")]
        crate::tackle_stats::DEF_ENTRIES.fetch_add(1, Ordering::Relaxed);

        // If we have the ball or our team controls it, transition to running
        if ctx.player.has_ball(ctx) || ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // CRITICAL: Don't try to claim ball if it's in protected flight state
        // Transition OUT of tackling to avoid clustering around the ball carrier
        if ctx.ball().is_in_flight() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }

        // Check if there's an opponent with the ball
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = opponent.distance(ctx);

            // If opponent is too far for tackling, press instead
            if distance_to_opponent > PRESSING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // If opponent is close but not in tackle range, keep pressing
            if distance_to_opponent > TACKLE_DISTANCE_THRESHOLD {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // Per-player tackle cooldown: a single per-state-machine gate
            // (e.g. Pressing→Tackling cadence) never held because the
            // Tackling state can be re-entered from Standing / Running /
            // Covering / Guarding / HoldingLine, each with its own
            // distance trigger and no shared cooldown. The cooldown lives
            // on the player itself — whatever path routed us here, if we
            // just tackled, we can't tackle again for ~1 s.
            if !ctx.player.can_attempt_tackle() {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // Closest-teammate duel gate. Without this, 3-4 defenders
            // within `TACKLE_DISTANCE_THRESHOLD` of the same ball carrier
            // all enter Tackling in the same tick and each rolls their
            // own attempt. Instrumentation showed this path was the
            // primary driver of ~370 tackle events/team/match (real
            // football: ~18). Only the best-positioned teammate
            // engages; the rest fall back to Pressing to cover angles.
            if !ctx.team().is_best_player_to_chase_ball() {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // We're close enough to tackle! One shot per Tackling entry,
            // enforced by the cooldown.
            #[cfg(feature = "match-logs")]
            crate::tackle_stats::DEF_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
            let (tackle_success, committed_foul, foul_severity) =
                self.attempt_sliding_tackle(ctx, &opponent);

            return if tackle_success {
                #[cfg(feature = "match-logs")]
                crate::tackle_stats::DEF_SUCCESSES.fetch_add(1, Ordering::Relaxed);
                let mut result = StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::TacklingBall(ctx.player.id)),
                );
                result.start_tackle_cooldown = true;
                Some(result)
            } else if committed_foul {
                let mut result = StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::CommitFoul(ctx.player.id, foul_severity)),
                );
                result.start_tackle_cooldown = true;
                Some(result)
            } else {
                let mut result = StateChangeResult::with_defender_state(DefenderState::Pressing);
                result.start_tackle_cooldown = true;
                Some(result)
            };
        } else {
            // Ball is loose - check for interception
            // Double-check not in flight before claiming
            if self.can_intercept_ball(ctx) && !ctx.ball().is_in_flight() {
                // Ball is loose and we can intercept it
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Running,
                    Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
                ));
            }

            // If ball is too far away and not coming toward us, return to position
            let ball_distance = ctx.ball().distance();
            if ball_distance > RETURN_DISTANCE && !ctx.ball().is_towards_player_with_angle(0.8) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ));
            }

            // Fallback: if ball is loose and very close, try to claim it
            // Double-check not in flight before claiming
            if !ctx.tick_context.ball.is_owned && ball_distance < 5.0 && !ctx.ball().is_in_flight()
            {
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Running,
                    Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
                ));
            }

            // If opponent is near the player but doesn't have the ball, maybe it's better to transition to pressing
            if let Some(close_opponent) = ctx.players().opponents().nearby(15.0).next() {
                if close_opponent.distance(ctx) < 10.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Pressing,
                    ));
                }
            }
        }

        if ctx.in_state_time > 30 {
            let ball_distance = ctx.ball().distance();
            if ball_distance > PRESSING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ));
            }
            // Stuck in tackling too long without engaging — drop back to standing
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let target = self.calculate_intelligent_target(ctx);

        // Closing-speed boost driven by the unified defender profile —
        // press_profile + tackle_profile already combine acceleration,
        // anticipation, work_rate, tackling, balance with fatigue.
        let def_profile = DefenderSkillProfile::from_ctx(ctx);
        let speed_boost = def_profile.tackle_speed_boost();

        Some(
            SteeringBehavior::Pursuit {
                target,
                target_velocity: Vector3::zeros(),
            }
            .calculate(ctx.player)
            .velocity
                * speed_boost
                + ctx.player().separation_velocity() * 0.15,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tackling is explosive and very demanding physically
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl DefenderTacklingState {
    fn calculate_intelligent_target(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let player_position = ctx.player.position;
        let own_goal_position = ctx.ball().direction_to_own_goal();

        // Check if ball is dangerously close to own goal
        let ball_distance_to_own_goal = (ball_position - own_goal_position).magnitude();
        let is_ball_near_own_goal =
            ball_distance_to_own_goal < ctx.context.field_size.width as f32 * 0.2;

        // Check if we're between the ball and our goal
        let player_distance_to_own_goal = (player_position - own_goal_position).magnitude();
        let is_player_closer_to_goal = player_distance_to_own_goal < ball_distance_to_own_goal;

        if is_ball_near_own_goal && !is_player_closer_to_goal {
            // If ball is near our goal and we're not between ball and goal,
            // position ourselves between the ball and the goal
            let ball_to_goal_direction = (own_goal_position - ball_position).normalize();
            let intercept_distance = 5.0; // Stand 5 units in front of the ball towards our goal
            ball_position + ball_to_goal_direction * intercept_distance
        } else {
            // Otherwise, pursue the ball directly
            ball_position
        }
    }

    fn attempt_sliding_tackle(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> (bool, bool, FoulSeverity) {
        let rng = &ctx.context.rng;
        let minute = sc::minute_from_ms(ctx.context.total_match_time);

        // Unified defender profile drives both the success and the foul
        // model. tackle_profile blends tackling/positioning/anticipation/
        // composure/strength/balance/agility; discipline is the
        // composure/decisions/concentration blend that suppresses fouls.
        let def_profile = DefenderSkillProfile::from_ctx(ctx);
        let aggression01 = (ctx.player.skills.mental.aggression / 20.0).clamp(0.0, 1.0);

        // Opponent carry score — composite-led for registered attackers,
        // skill blend fallback when the opponent is missing from the
        // registry.
        let attacker_score = if let Some(att) = ctx.context.players.by_id(opponent.id) {
            sc::dribble_attack(att, minute)
        } else {
            let dribbling;
            let agility;
            {
                let players = ctx.player();
                let s = players.skills(opponent.id);
                dribbling = sc::n(s.technical.dribbling);
                agility = sc::n(s.physical.agility);
            }
            (dribbling + agility) * 0.5
        };

        // Logistic success: tackle_profile vs attacker carry.
        // Sigmoid 3.2 → 2.4 and upper clamp 0.72 → 0.55 trim the
        // strong-defender-vs-weak-attacker dominance. At equal skill
        // `raw_diff = 0` and `sigmoid = 0.5` regardless of coefficient,
        // so calibration-neutral for equal matchups. The 0.55 cap means
        // even an elite CB vs a poor forward leaves the attacker with
        // 45% per-attempt survival — across 3 engagement sequences in
        // the final third that's ~14% accumulated retention, enough to
        // let weak teams complete the occasional shooting chain at
        // extreme skill gaps (the prior 0.62 cap gave only ~5%).
        let raw_diff = def_profile.tackle_profile - attacker_score;
        let success_chance = (1.0 / (1.0 + (-raw_diff * 2.4).exp())).clamp(0.06, 0.55);

        let tackle_success = rng.random::<f32>() < success_chance;

        // Foul chance driven by discipline (composure/decisions/
        // concentration/tackling blend) instead of raw composure +
        // aggression. Tired defenders foul more (low def_condition_mult
        // adds to foul rate). Failed tackles roughly double the rate.
        let mut base_foul = 0.030 + aggression01 * 0.11 - def_profile.discipline * 0.075;
        if !tackle_success {
            base_foul *= 1.80;
        }
        base_foul += (1.0 - def_profile.def_condition_mult).max(0.0) * 0.08;
        let foul_chance = base_foul.clamp(0.006, 0.28);

        let committed_foul = rng.random::<f32>() < foul_chance;

        // Severity classification stays driven by aggression — discipline
        // already gates whether a foul fires at all, so we don't need to
        // double-dip here. Reckless/violent stay tied to aggression +
        // miss-context the same as before.
        let severity = if !committed_foul {
            FoulSeverity::Normal
        } else if aggression01 > 0.75 && !tackle_success && rng.random::<f32>() < 0.12 {
            FoulSeverity::Violent
        } else if !tackle_success && aggression01 > 0.55 {
            FoulSeverity::Reckless
        } else {
            FoulSeverity::Normal
        };

        (tackle_success, committed_foul, severity)
    }

    fn exists_nearby(&self, ctx: &StateProcessingContext) -> bool {
        const DISTANCE: f32 = 30.0;

        ctx.players().opponents().exists(DISTANCE) || ctx.players().teammates().exists(DISTANCE)
    }

    fn can_intercept_ball(&self, ctx: &StateProcessingContext) -> bool {
        if self.exists_nearby(ctx) {
            return false;
        }

        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let player_position = ctx.player.position;
        let player_speed = ctx.player.skills.physical.pace;

        if !ctx.tick_context.ball.is_owned && ball_velocity.magnitude() > 0.1 {
            let time_to_ball = (ball_position - player_position).magnitude() / player_speed;
            let ball_travel_distance = ball_velocity.magnitude() * time_to_ball;
            let ball_intercept_position =
                ball_position + ball_velocity.normalize() * ball_travel_distance;
            let player_intercept_distance = (ball_intercept_position - player_position).magnitude();

            player_intercept_distance <= TACKLE_DISTANCE_THRESHOLD
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::PlayerSkills;
    use crate::club::player::builder::PlayerBuilder;
    use crate::r#match::MatchPlayer;
    use crate::r#match::player::strategies::players::ops::skill_composites as sc;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
    };
    use chrono::NaiveDate;

    fn defender(tackling: f32, marking: f32, positioning: f32) -> MatchPlayer {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = 9000;
        let mut skills = PlayerSkills::default();
        skills.technical.tackling = tackling;
        skills.technical.marking = marking;
        skills.mental.positioning = positioning;
        skills.mental.anticipation = 12.0;
        skills.mental.concentration = 12.0;
        skills.mental.bravery = 12.0;
        skills.physical.strength = 13.0;
        skills.physical.balance = 12.0;
        skills.physical.agility = 12.0;
        skills.physical.stamina = 14.0;
        skills.physical.natural_fitness = 14.0;
        let p = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("D".into(), "Z".into()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::DefenderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(1, &p, PlayerPositionType::DefenderCenter, false)
    }

    fn attacker(dribbling: f32, technique: f32, agility: f32) -> MatchPlayer {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = 9000;
        let mut skills = PlayerSkills::default();
        skills.technical.dribbling = dribbling;
        skills.technical.technique = technique;
        skills.mental.flair = 12.0;
        skills.mental.composure = 12.0;
        skills.mental.decisions = 12.0;
        skills.physical.agility = agility;
        skills.physical.acceleration = 14.0;
        skills.physical.balance = 12.0;
        skills.physical.strength = 11.0;
        skills.physical.stamina = 14.0;
        skills.physical.natural_fitness = 14.0;
        let p = PlayerBuilder::new()
            .id(2)
            .full_name(FullName::new("A".into(), "Z".into()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::ForwardCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(2, &p, PlayerPositionType::ForwardCenter, false)
    }

    #[test]
    fn strong_tackler_dominates_weak_dribbler() {
        let strong = defender(18.0, 17.0, 16.0);
        let weak_attacker = attacker(7.0, 7.0, 9.0);
        let diff = sc::defensive_duel(&strong, 30) - sc::dribble_attack(&weak_attacker, 30);
        assert!(
            diff > 0.20,
            "expected strong defender advantage, got diff={diff}"
        );
    }

    #[test]
    fn weak_tackler_loses_to_strong_dribbler() {
        let weak = defender(7.0, 7.0, 8.0);
        let elite_attacker = attacker(18.0, 17.0, 17.0);
        let diff = sc::defensive_duel(&weak, 30) - sc::dribble_attack(&elite_attacker, 30);
        assert!(diff < -0.10, "expected attacker advantage, got diff={diff}");
    }

    #[test]
    fn marking_and_positioning_help_defender_in_duel() {
        let positional = defender(12.0, 18.0, 18.0);
        let raw_tackler = defender(15.0, 8.0, 8.0);
        // The positional defender should match or exceed a stronger
        // pure tackler thanks to marking + positioning weight (0.13 +
        // 0.17). This validates the spec's expectation that the duel
        // composite isn't dominated by raw tackling alone.
        assert!(sc::defensive_duel(&positional, 30) >= sc::defensive_duel(&raw_tackler, 30));
    }
}

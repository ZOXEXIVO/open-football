use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{
    ActivityIntensity, LaneAhead, MidfielderCondition, Opportunity, TakeOn, U_PER_M,
};
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::{
    ShotDecision, evaluate_forward_shot_decision,
};
use crate::r#match::player::strategies::common::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PassEvaluator, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;
use std::cmp::Ordering;

/// Range at which a shot is point blank rather than a chance — 6 m.
const POINT_BLANK: f32 = 6.0 * U_PER_M;

/// AI ticks in a second (`MATCH_TIME_INCREMENT_MS` is 10).
const TICKS_PER_SECOND: f32 = 100.0;

/// Per-call-site salt for `Opportunity`.
const POINT_BLANK_SALT: u64 = 0x6A09_E667_F3BC_C909;

#[derive(Default, Clone)]
pub struct MidfielderDribblingState {}

impl StateProcessingHandler for MidfielderDribblingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        let mid_profile = MidfielderSkillProfile::from_ctx(ctx);
        let shot_profile = ctx.player().shooting().shot_profile();
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let lane = LaneAhead::read(ctx);

        // Shot pivot from a dribble — every midfielder (AM and CM alike)
        // consults the shared forward helper, which scales the decision
        // continuously with selection / execution / composure, applies
        // the xG floor, the anti-monopoly cap and the willingness roll.
        // The old deterministic side-door here (clear shot + ≤32u +
        // xG ≥ 0.12 + a binary mid_shot_selection bar) fired on every
        // tick those conditions held; once the fatigue-normalization fix
        // let midfielders keep their legs past minute 20, that election
        // alone produced ~26% of all MID shots and helped balloon the
        // MID goal share to 45% (target 32%). Routing through the helper
        // keeps the chance-quality logic in ONE place.
        if let ShotDecision::Shoot { reason } = evaluate_forward_shot_decision(ctx, "MID_DRIB") {
            return Some(
                StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                    .with_shot_reason(reason),
            );
        }

        // Point-blank — with a skill-graded willingness so 5/20
        // midfielders can still pass instead of miscueing the easy
        // chance. Real point-blank shots succeed for composed finishers;
        // panicked low-skill players hit the keeper.
        //
        // The range was 22u. That reads like twenty-two of something and
        // is 2.75 m — inside the six-yard line, closer to the net than a
        // goalkeeper stands — so the branch described a situation that
        // barely occurs. 6 m is point blank.
        if distance_to_goal < POINT_BLANK {
            let point_blank_willingness = (0.10
                + shot_profile.selection_skill * 0.30
                + mid_profile.mid_shot_selection * 0.20)
                .clamp(0.12, 0.65);
            // Once per possession, not once per tick. At 12-65% a tick
            // the answer was "yes" within a few hundredths of a second
            // of entering the box, so the willingness curve above — the
            // whole point of which is that a poor finisher sometimes
            // squares it instead — was being integrated away to a
            // certainty. See `Opportunity`.
            if Opportunity::draw(ctx, POINT_BLANK_SALT) < point_blank_willingness {
                return Some(
                    StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                        .with_shot_reason("MID_DRIB_POINT_BLANK"),
                );
            }
            // Roll failed — try cutback / pass before forcing the shot.
            if PassEvaluator::find_best_pass_option(ctx, 60.0).is_some() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
        }

        // HE'S BEATEN HIM — go back to running with it.
        //
        // Nothing used to notice. The state's only exits were a timeout
        // and a pressure bail, so a successful take-on was followed by
        // the same "find a pass or time out" script as a failed one, and
        // the carry that should follow a beaten man never happened.
        if !lane.has_man_to_beat() && ctx.in_state_time > 8 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Swarmed — two or more of them genuinely on him. `nearby(15.0)`
        // was 1.9 m, so this asked for two opponents inside two metres
        // and effectively never fired; 3 m is being closed down by two.
        // Press resistance buys the extra beat before he has to give it.
        let swarm_radius = (2.4 + mid_profile.press_resistance * 1.4) * U_PER_M;
        if ctx.players().opponents().nearby(swarm_radius).count() >= 2 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Passing,
            ));
        }

        // How long the take-on gets. A dribble is a second or two of
        // football; the old budget was 0.25-0.80 s, over before the
        // duel it models could resolve. Scaled by carry quality so a
        // Zidane keeps it under his foot longer than a holding player.
        let max_dribble_ticks =
            ((0.9 + mid_profile.carry_selection * 1.6) * TICKS_PER_SECOND) as u64;
        if ctx.in_state_time as u64 > max_dribble_ticks {
            if PassEvaluator::find_best_pass_option(ctx, 200.0).is_some() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // He committed and it hasn't come off — the appetite that sent
        // him here has drained (a second man has arrived, or the man in
        // front has closed the space down). Give it rather than run into
        // him. Continuous: the same quantity that opened the take-on
        // closes it, so there is no separate rule to disagree with.
        if ctx.in_state_time > 20 && TakeOn::appetite(ctx, &lane, &mid_profile) < 0.10 {
            if PassEvaluator::find_best_pass_option(ctx, 200.0).is_some() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
        }

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
        let nearest_opponent = ctx.players().opponents().nearby(30.0).min_by(|a, b| {
            let da = (a.position - player_pos).magnitude();
            let db = (b.position - player_pos).magnitude();
            da.partial_cmp(&db).unwrap_or(Ordering::Equal)
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
    #[allow(dead_code)]
    fn find_open_teammate<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        // Use the proper pass evaluator instead of a random nearby pick
        // — random fallbacks fed the ball into covered teammates.
        PassEvaluator::find_best_pass_option(ctx, 200.0).map(|(t, _)| t)
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

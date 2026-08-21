use crate::r#match::engine::ball::ball::HandlingVerdict;
use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperFeetDecision, KeeperRelease, KeeperSetPosition,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// How long a keeper keeps the ball in his hands before releasing it, in
/// ticks (100 = 1 s).
///
/// Law 12 gives him six seconds; in practice a keeper takes two on a quick
/// counter and five when his side is happy to slow the game down, and the
/// referee's whistle is close to a dead letter. These bracket that.
///
/// They were 25 and 60 — half a second to one and a quarter, which is
/// physically impossible: he had not finished catching it. The engine's
/// keepers released the ball almost the instant they claimed it, which
/// removed the natural pause in play after every save and cross.
///
/// UNITS: `in_state_time` counts AI TICKS, not engine ticks. Only full
/// ticks run the state machine — `game_tick_light` deliberately leaves
/// the counter alone — so one unit here is 20 ms, not 10. The first pass
/// at this used 200-550 believing they were 10 ms ticks, which made a
/// keeper hold the ball for four to eleven seconds and put the ball in
/// his gloves for 27.9% of the match against a real 3-6%.
const MIN_HOLDING_DURATION: u64 = 100;
const MAX_HOLDING_DURATION: u64 = 275;

/// How the keeper puts the ball back into play once it is in their hands.
///
/// Each variant maps to a state that was fully implemented but had no
/// inbound transition — `HoldingBall` used to run a fixed timer straight
/// into `Distributing`, so `Throwing` and `Kicking` were unreachable and
/// every keeper in every match released the ball exactly the same way.
#[derive(Debug, Clone, Copy, PartialEq)]
enum DistributionChoice {
    /// Rolled or short-passed to a defender — playing out from the back.
    Short,
    /// Thrown to a free teammate in the middle third — the counter-attack
    /// release: faster and far more accurate than a kick, but bounded by
    /// arm strength.
    Throw,
    /// Punted long towards the forward line. Concedes possession more
    /// often, but it is the only option when the short outlets are shut,
    /// and the correct one for a side playing direct or chasing a game.
    Kick,
}

impl DistributionChoice {
    fn into_state(self) -> GoalkeeperState {
        match self {
            DistributionChoice::Short => GoalkeeperState::Distributing,
            DistributionChoice::Throw => GoalkeeperState::Throwing,
            DistributionChoice::Kick => GoalkeeperState::Kicking,
        }
    }
}

#[derive(Default, Clone)]
pub struct GoalkeeperHoldingState {}

impl StateProcessingHandler for GoalkeeperHoldingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If for some reason we no longer have the ball, return to standing
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // **You cannot HOLD a ball you have not picked up.**
        //
        // `has_ball` is ownership, and ownership says nothing about the
        // gloves. `Catching` and `PickingUpBall` pair their transition with
        // a `CaughtBall` event, so they arrive here holding it; `Diving`
        // and `Jumping` route in on ownership ALONE. Any grant that reaches
        // him without raising `held_in_hands` therefore put him in the one
        // state whose whole meaning is that the ball is in his hands, with
        // the ball on the grass at his feet — for the two to five and a
        // half seconds this state deliberately dwells for, stationary at
        // first and then walking it out. Measured before the fix: 155 ticks
        // a match, 40% of all the foot possession in the game.
        //
        // That is unplayable in both directions: the opposition press and
        // then tackle a man the engine believes is untouchable, and the
        // replay draws a keeper standing over a ball rather than holding
        // one. So the state asserts its own precondition. Hands legal —
        // scoop it up, which is what a keeper does with a ball at his feet
        // in his own box. Hands illegal — it is a foot possession and
        // belongs to the foot states, so hand it to the one that fits.
        if !ctx.tick_context.ball.held_in_hands {
            return Some(StateChangeResult::with_goalkeeper_state(
                match ctx.ball().handling_verdict() {
                    HandlingVerdict::Legal => GoalkeeperState::PickingUpBall,
                    _ => KeeperFeetDecision::without_hands(ctx),
                },
            ));
        }

        // After holding for a skill-based duration, release the ball.
        // Better decision-makers distribute faster.
        let decision = ctx.player.skills.mental.decisions / 20.0;
        let holding_duration = MAX_HOLDING_DURATION
            - ((MAX_HOLDING_DURATION - MIN_HOLDING_DURATION) as f32 * decision) as u64;
        if ctx.in_state_time >= holding_duration {
            return Some(StateChangeResult::with_goalkeeper_state(
                Self::pick_distribution(ctx).into_state(),
            ));
        }

        // No other transitions - goalkeeper should continue holding the ball
        // until ready to distribute it, should not try to catch the same ball
        // they already possess
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Carry it out. This used to return a hard zero, so a keeper stood
        // rooted on the exact spot he gathered it for the whole 2-5.5 s
        // hold — and the ball tracks its owner, so the ball did not move
        // either. Caught on the goal line, that is a ball hanging
        // motionless in the goalmouth at glove height, ~160 times a match:
        // the "the ball always ends up at one point in front of the goal"
        // report. A real keeper gets up and walks it out towards the edge
        // of his area, which is where he releases it from anyway.
        let own_goal = ctx.ball().direction_to_own_goal();
        let release = KeeperSetPosition::release_point(
            own_goal,
            ctx.player.position,
            ctx.context.field_size.width as f32,
        );
        // Walking pace: he is managing the game, not sprinting, and the
        // ball's own tracking speed is the ceiling that keeps it at his
        // feet rather than trailing behind him.
        const CARRY_PACE: f32 = 0.45;
        Some(
            SteeringBehavior::Arrive {
                target: release,
                slowing_distance: 25.0,
            }
            .calculate(ctx.player)
            .velocity
                * CARRY_PACE,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Holding ball is a low intensity activity with minimal physical effort
        GoalkeeperCondition::new(ActivityIntensity::Low).process(ctx);
    }
}

impl GoalkeeperHoldingState {
    /// Choose how to release the ball.
    ///
    /// Three continuous scores, highest wins — no thresholds, so a keeper
    /// slides between options as the picture changes rather than flipping
    /// at a hard boundary. The inputs are the ones a real keeper reads:
    /// how good they are at each technique, whether the short outlets are
    /// actually free, whether the press is on, whether a counter is
    /// available, and what the manager has asked for.
    fn pick_distribution(ctx: &StateProcessingContext) -> DistributionChoice {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let gk = &ctx.player.skills.goalkeeping;

        let throw_skill = (gk.throwing / 20.0).clamp(0.0, 1.0);
        let kick_skill = (gk.kicking / 20.0).clamp(0.0, 1.0);
        let short_skill = ((gk.passing + gk.first_touch) / 40.0).clamp(0.0, 1.0);
        // Composure under pressure gates whether playing out is sane.
        let composure = (ctx.player.skills.mental.composure / 20.0).clamp(0.0, 1.0);

        let free_short = KeeperRelease::free_outlets(ctx, KeeperRelease::SHORT_RANGE);
        let free_throw = KeeperRelease::free_outlets(ctx, KeeperRelease::THROW_RANGE);
        let press = KeeperRelease::press_pressure(ctx);
        let counter = KeeperRelease::counter_opportunity(ctx);
        let directness = KeeperRelease::directness(ctx);

        // Short build-up: needs a free nearby outlet AND the composure to
        // use it. The press directly suppresses it — that is the whole
        // point of pressing a keeper.
        let short = 0.30 + free_short * 0.45 + short_skill * 0.35 + composure * 0.20
            - press * 0.85
            - directness * 0.45;

        // Throw: the counter-attack release. Scales with arm strength and
        // with how exposed the opposition is; less press-sensitive than a
        // roll because the ball travels further and faster.
        let throw = 0.20 + free_throw * 0.50 + throw_skill * 0.55 + counter * 0.60 - press * 0.30;

        // Kick: always available, so it is the fallback the other two have
        // to beat. Rises with kicking ability, with the press, and with how
        // direct the side has been asked to be.
        let kick = 0.34 + kick_skill * 0.45 + press * 0.70 + directness * 0.55
            - free_short * 0.25
            - prof.distribution * 0.15;

        if throw >= short && throw >= kick {
            DistributionChoice::Throw
        } else if short >= kick {
            DistributionChoice::Short
        } else {
            DistributionChoice::Kick
        }
    }
}

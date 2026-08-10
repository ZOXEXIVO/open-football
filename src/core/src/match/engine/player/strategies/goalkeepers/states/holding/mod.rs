use crate::PlayerFieldPositionGroup;
use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::teamplay::coach::CoachInstruction;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
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

/// Furthest a keeper can realistically throw. Beyond this the ball has
/// to be kicked.
const THROW_RANGE: f32 = 110.0;
/// Radius used to judge whether a short outlet is actually free.
const MARKING_RADIUS: f32 = 18.0;
/// How far up the pitch we look for an opponent when deciding whether
/// our own third is being pressed.
const PRESS_RADIUS: f32 = 140.0;

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

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
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

        let free_short = Self::free_outlets(ctx, THROW_RANGE * 0.45);
        let free_throw = Self::free_outlets(ctx, THROW_RANGE);
        let press = Self::press_pressure(ctx);
        let counter = Self::counter_opportunity(ctx);
        let directness = Self::directness(ctx);

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

    /// Fraction (0..1) of nearby teammates inside `range` who are not
    /// tightly marked. This is "have I got someone to give it to?", the
    /// question that decides whether playing out is on at all.
    fn free_outlets(ctx: &StateProcessingContext, range: f32) -> f32 {
        let mut total = 0u32;
        let mut free = 0u32;
        for teammate in ctx.players().teammates().nearby(range) {
            total += 1;
            if ctx
                .tick_context
                .grid
                .opponents(teammate.id, MARKING_RADIUS)
                .next()
                .is_none()
            {
                free += 1;
            }
        }
        if total == 0 {
            return 0.0;
        }
        // Three free outlets is a comfortable picture; more adds nothing.
        (free as f32 / 3.0).min(1.0)
    }

    /// How aggressively the opposition is squeezing our own third (0..1).
    /// Counts opponents who have committed into the area the keeper would
    /// have to play through.
    fn press_pressure(ctx: &StateProcessingContext) -> f32 {
        let committed = ctx.players().opponents().nearby(PRESS_RADIUS).count();
        // Two opponents pressing high is normal; four-plus is a full press.
        ((committed as f32 - 1.0) / 3.0).clamp(0.0, 1.0)
    }

    /// How exposed the opposition is right now (0..1) — the trigger for a
    /// fast throw. Reads how many of our runners are ahead of the ball
    /// against how many opponents are back, which is what a keeper
    /// actually sees after claiming a cross against a committed attack.
    fn counter_opportunity(ctx: &StateProcessingContext) -> f32 {
        let Some(side) = ctx.player.side else {
            return 0.0;
        };
        let field_width = ctx.context.field_size.width as f32;

        let upfield_teammates = ctx
            .players()
            .teammates()
            .all()
            .filter(|t| {
                !matches!(
                    t.tactical_positions.position_group(),
                    PlayerFieldPositionGroup::Goalkeeper
                ) && side.attacking_progress_x(t.position.x, field_width) > 0.45
            })
            .count();
        let recovering_opponents = ctx
            .players()
            .opponents()
            .all()
            .filter(|o| side.attacking_progress_x(o.position.x, field_width) < 0.55)
            .count();

        // A break is on when our runners aren't outnumbered ahead of the
        // ball. Two spare bodies is a full counter.
        let spare = upfield_teammates as f32 - recovering_opponents as f32;
        ((spare + 1.0) / 3.0).clamp(0.0, 1.0)
    }

    /// How direct the manager wants the side to be (0..1). A side told to
    /// push forward or chase the game bypasses midfield; one told to slow
    /// down or keep the ball plays out.
    fn directness(ctx: &StateProcessingContext) -> f32 {
        let from_instruction = match ctx.team().coach_instruction() {
            CoachInstruction::AllOutAttack => 0.90,
            CoachInstruction::PushForward => 0.60,
            CoachInstruction::ParkTheBus => 0.55,
            CoachInstruction::Normal => 0.30,
            CoachInstruction::SlowDown => 0.10,
            CoachInstruction::WasteTime => 0.0,
        };
        // Patient build-up sides go long less often regardless of the
        // in-game instruction.
        let patience = ctx.team().build_up_patience().clamp(0.0, 1.0);
        (from_instruction * (1.0 - patience * 0.5)).clamp(0.0, 1.0)
    }
}

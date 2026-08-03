use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::players::ShotType;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// Ball must be at least this high to be a header rather than a foot
/// contact. Matches the defender / forward heading bands so the same
/// aerial ball reads identically whichever role reaches it first.
const HEADING_HEIGHT_THRESHOLD: f32 = 1.5;
/// Contact reach. Slightly wider than the defender's 1.5 because a
/// midfield aerial duel is a running jump into the flight path rather
/// than a set position in the six-yard box.
const HEADING_DISTANCE_THRESHOLD: f32 = 2.0;
/// Inside this distance of the opponent goal an aerial contact is an
/// attempt on goal (a late-arriving eight meeting a cross), not a
/// knock-down.
const ATTACKING_HEADER_RANGE: f32 = 90.0;
/// A won aerial only becomes an attempt ON GOAL when the position makes
/// it a genuine chance — same "true big chance" philosophy as the
/// deterministic Tier-1 shot in `midfielders/running` (bar 0.240 there;
/// headers carry less power/placement than a set foot shot, so the bar
/// here stays at the earlier 0.180 rung). Below the bar the won header
/// is still won — it becomes the knock-down, which is what a real
/// midfielder does with an aerial ball they can't attack the goal with.
const HEADER_ON_GOAL_XG_BAR: f32 = 0.180;
/// Anti-monopoly parity with the Tier-1 foot-shot path: past this many
/// attempts the aerial win is recycled as a knock-down instead of yet
/// another attempt.
const HEADER_SHOT_CAP: u32 = 4;
/// Beyond this distance from our own goal a won header is a controlled
/// knock-down to a teammate; closer than this it's an emergency
/// clearance away from danger.
const DEFENSIVE_CLEARANCE_RANGE: f32 = 120.0;

/// Midfield aerial duel — the knock-down, the flick-on, the headed
/// clearance from a goal kick dropping on the halfway line.
///
/// Midfielders were the one outfield role with no `Heading` state: every
/// lofted ball into the middle third was resolved by whoever could get a
/// foot to it after the bounce, so second balls in midfield never
/// existed. This closes that gap using the same skill composite the
/// corner contest reads (`aerial_outfield_*`), so a tall destroyer wins
/// midfield headers against a small playmaker for the same reasons they
/// win them in the box.
#[derive(Default, Clone)]
pub struct MidfielderHeadingState {}

impl StateProcessingHandler for MidfielderHeadingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Someone else got there first (or we did, with our feet).
        if ctx.ball().is_owned() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        let ball_position = ctx.tick_context.positions.ball.position;

        // Ball dropped out of the heading band or drifted out of reach —
        // go back to reading the play. Running re-evaluates the chase.
        if ball_position.z < HEADING_HEIGHT_THRESHOLD
            || ctx.ball().distance() > HEADING_DISTANCE_THRESHOLD
        {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        if !self.wins_duel(ctx) {
            // Lost the header — the ball goes on and we react to it.
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Won it. Where the contact goes depends on where on the pitch it
        // happened — attacking the box is a shot, deep in our own third
        // is a clearance, and everything between is a knock-down forward.
        if ctx.ball().distance_to_opponent_goal() < ATTACKING_HEADER_RANGE
            && self.is_genuine_header_chance(ctx)
        {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Running,
                Event::PlayerEvent(PlayerEvent::Shoot(
                    ShootingEventContext::new()
                        .with_player_id(ctx.player.id)
                        .with_target(ctx.player().shooting_direction())
                        .with_reason("MID_HEADER_ON_GOAL")
                        .with_shot_type(ShotType::Header)
                        .build(ctx),
                )),
            ));
        }

        if ctx.ball().distance_to_own_goal() < DEFENSIVE_CLEARANCE_RANGE {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Running,
                Event::PlayerEvent(PlayerEvent::Shoot(
                    ShootingEventContext::new()
                        .with_player_id(ctx.player.id)
                        .with_target(ctx.player().clearing_direction())
                        .with_reason("MID_HEADED_CLEARANCE")
                        .build(ctx),
                )),
            ));
        }

        // Midfield third: a knock-down. Head it into the space ahead so a
        // teammate can run onto it rather than booting it clear — this is
        // the second-ball mechanic the state exists for.
        Some(StateChangeResult::with_midfielder_state_and_event(
            MidfielderState::Running,
            Event::PlayerEvent(PlayerEvent::ClearBall(self.knock_down_velocity(ctx))),
        ))
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Attack the flight path — arrive at the ball, not where it was.
        let target = ctx.tick_context.positions.ball.position;
        Some(
            SteeringBehavior::Arrive {
                target,
                slowing_distance: 3.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // A contested jump is as explosive as it gets for an outfielder.
        MidfielderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl MidfielderHeadingState {
    /// Is this won aerial a genuine attempt-on-goal position, or a ball
    /// to cushion for a teammate? Reads the same skill-and-position xG
    /// the Tier-1 foot-shot gate reads, so "what counts as a chance"
    /// stays one concept engine-wide, plus the Tier-1 anti-monopoly cap.
    fn is_genuine_header_chance(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.memory().shots_taken > HEADER_SHOT_CAP {
            return false;
        }
        let profile = ctx.player().shooting().shot_profile();
        let distance = ctx.ball().distance_to_opponent_goal();
        profile.expected_xg(distance, true) >= HEADER_ON_GOAL_XG_BAR
    }

    /// Aerial duel against the nearest contesting opponent.
    ///
    /// Uses the shared `aerial_outfield_*` composites (heading, jumping,
    /// strength, balance, bravery, fatigue-aware) so midfield duels are
    /// scored on the same axis as the corner contest. An uncontested
    /// ball still needs a clean contact — elite headers effectively
    /// never miss, poor ones can glance it.
    fn wins_duel(&self, ctx: &StateProcessingContext) -> bool {
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let mine = sc::aerial_outfield_attacker(ctx.player, minute);

        // Only an opponent close enough to actually jump with us counts.
        let contest = ctx
            .players()
            .opponents()
            .nearby(HEADING_DISTANCE_THRESHOLD * 2.0)
            .filter_map(|opponent| ctx.context.players.by_id(opponent.id))
            .map(|opponent| sc::aerial_outfield_defender(opponent, minute))
            .fold(0.0_f32, f32::max);

        // Uncontested: a clean contact is the strong default, scaled by
        // how good a header of the ball this player is. Contested: the
        // skill gap moves the odds continuously around an even duel.
        let win_prob = if contest <= 0.0 {
            (0.72 + mine * 0.26).clamp(0.60, 0.97)
        } else {
            (0.50 + (mine - contest) * 0.60).clamp(0.12, 0.88)
        };

        ctx.context.rng.bernoulli(win_prob)
    }

    /// Knock-down into the space ahead: forward along the attacking
    /// direction, low and short. A knock-down travels a few metres —
    /// this is a controlled cushion, not a clearance, so the receiving
    /// teammate has a genuine chance to win the second ball.
    fn knock_down_velocity(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let control = sc::aerial_outfield_attacker(ctx.player, minute);
        let forward_x = ctx.player.side.map_or(1.0, |side| side.forward_dir_x());

        // Better aerial technique = a flatter, more purposeful knock-down;
        // a poor one loops up and gives everyone time to react.
        let power = 1.6 + control * 1.0;
        let lift = 1.4 - control * 0.5;
        let y_drift: f32 = ctx.context.rng.random_range(-0.5..0.5);

        Vector3::new(forward_x * power, y_drift, lift)
    }
}

use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct MidfielderSwitchingPlayState {}

impl StateProcessingHandler for MidfielderSwitchingPlayState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        // Check if there's a good opportunity to switch play
        if let Some((teammate_id, _)) = self.find_switch_play_target(ctx) {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Passing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::new()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(teammate_id)
                        .with_reason("MID_SWITCHING_PLAY")
                        .build(ctx),
                )),
            ));
        }

        // No switch target found — bail to Passing to find any option
        if ctx.in_state_time > 10 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Passing,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move towards the best position to switch play
        if let Some((_, teammate_position)) = self.find_switch_play_target(ctx) {
            let steering = SteeringBehavior::Seek {
                target: teammate_position,
            }
            .calculate(ctx.player);

            Some(steering.velocity)
        } else {
            // If no suitable target position is found, stay in the current position
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Switching play is low intensity - tactical passing
        MidfielderCondition::new(ActivityIntensity::Low).process(ctx);
    }
}

impl MidfielderSwitchingPlayState {
    fn find_switch_play_target(&self, ctx: &StateProcessingContext) -> Option<(u32, Vector3<f32>)> {
        // Find the best position to switch play to
        let player_position = ctx.player.position;
        let field_height = ctx.context.field_size.height as f32;

        // A switch of play travels to the FAR flank, so select by real
        // lateral separation from the carrier. (The old axis was derived
        // from `ball - player`, which is the zero vector while carrying
        // — the ball snaps to its owner — so its normalize() was NaN and
        // no target ever matched; the state always fell through to plain
        // Passing.)
        // …ranked on where it leaves the attack, not on space alone.
        //
        // Space was the only term, and the freest man on the far side is
        // routinely the deepest one — so a winger switching from the edge
        // of the box picked out a full-back forty metres behind the play,
        // and from inside the area that ball travels across his own goal.
        // Reported as "midfielders switching in the GK area".
        //
        // ⚠ RANKED, NOT FILTERED. Excluding the backward options instead
        // was tried and is far worse: `find_switch_play_target` returns
        // `None`, the state has nothing to do, and the carrier stands on
        // the ball for ten ticks before bailing. The switch is a
        // POSSESSION TERMINATOR — cutting it leaves attacks alive in the
        // final third that used to end there, and the engine's chance
        // supply is already ~2.3x real. Measured: `no-decision` holds
        // 3.5% -> 11.4% of on-ball ticks, passes/team 862 -> 1300,
        // goals 5.4 -> 8.5. Same lesson as the `Marking <-> Running`
        // loop: measure the match, not the thing that looks wrong.
        //
        // So every candidate still qualifies and a switch still happens
        // at the same rate; only the destination moves up the pitch.
        let goal = ctx.player().opponent_goal_position();
        let my_goal_distance = (goal - player_position).magnitude();
        let score = |teammate: &MatchPlayerLite| -> f32 {
            let space = self.calculate_space_around_player(ctx, teammate);
            // Ground the switch gains or gives up, in units, folded into
            // the same score. Bounded so it breaks ties and outranks a
            // marginally freer man without ever vetoing the only option.
            let gained = my_goal_distance - (goal - teammate.position).magnitude();
            space + (gained / 40.0).clamp(-6.0, 6.0)
        };

        ctx.players()
            .teammates()
            .all()
            .filter(|teammate| (teammate.position.y - player_position.y).abs() > field_height * 0.3)
            .max_by(|a, b| score(a).total_cmp(&score(b)))
            .map(|teammate| (teammate.id, teammate.position))
    }

    fn calculate_space_around_player(
        &self,
        ctx: &StateProcessingContext,
        player: &MatchPlayerLite,
    ) -> f32 {
        // Calculate the amount of free space around a player
        let space_radius = 10.0; // Adjust the radius as needed
        let num_opponents_nearby = ctx
            .tick_context
            .grid
            .opponents(player.id, space_radius)
            .count();

        space_radius - num_opponents_nearby as f32
    }
}

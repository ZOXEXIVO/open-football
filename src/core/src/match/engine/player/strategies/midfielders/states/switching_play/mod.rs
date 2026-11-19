use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{
    ActivityIntensity, MidfielderCondition, MidfielderRead,
};
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::passing::CrossModel;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};

#[derive(Default, Clone)]
pub struct MidfielderSwitchingPlayState {}

impl StateProcessingHandler for MidfielderSwitchingPlayState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(MidfielderRead::change(ctx));
        }

        // Check if there's a good opportunity to switch play
        if let Some(teammate_id) = self.find_switch_play_target(ctx) {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderRead::released(ctx),
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::builder()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(teammate_id)
                        .with_reason("MID_SWITCHING_PLAY")
                        .build(ctx),
                )),
            ));
        }

        // No switch on: any pass, decided now.
        Some(StateChangeResult::with_midfielder_state(
            MidfielderState::Passing,
        ))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Switching play is low intensity - tactical passing
        MidfielderCondition::new(ActivityIntensity::Low).process(ctx);
    }
}

impl MidfielderSwitchingPlayState {
    fn find_switch_play_target(&self, ctx: &StateProcessingContext) -> Option<u32> {
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
        // The man the team put on the far touchline is the man a switch
        // is FOR. Scored rather than filtered, like everything else here,
        // and worth about as much as a genuinely free opponent-free
        // pocket — because that is what he is, by construction.
        let far_holder = {
            let team = ctx.team();
            let wide = team.wide_plan();
            wide.far_holder()
        };
        let score = |teammate: &MatchPlayerLite| -> f32 {
            let space = self.calculate_space_around_player(ctx, teammate);
            // Ground the switch gains or gives up, in units, folded into
            // the same score. Bounded so it breaks ties and outranks a
            // marginally freer man without ever vetoing the only option.
            let gained = my_goal_distance - (goal - teammate.position).magnitude();
            let assigned = if far_holder == Some(teammate.id) {
                5.0
            } else {
                0.0
            };
            space + (gained / 40.0).clamp(-6.0, 6.0) + assigned
        };

        ctx.players()
            .teammates()
            .all()
            .filter(|teammate| {
                (teammate.position.y - player_position.y).abs() > field_height * 0.3
                    // From the wing a man in the box is a cross, not a switch.
                    && !ctx.player.side.is_some_and(|side| {
                        CrossModel::is_cross(player_position, teammate.position, side, ctx.context)
                    })
            })
            .filter_map(|teammate| {
                let odds = ctx.player().clear_pass(teammate.id)?;
                Some((teammate.id, odds.price(score(&teammate))))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
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

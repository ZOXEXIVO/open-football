use crate::r#match::engine::ball::ball::{AerialReach, Ball, PlayerReach};
use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::shared::{
    ActivityIntensity, MidfielderCondition, MidfielderRead,
};
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::defender_census::{HeaderBooking, HeaderRule};
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::midfielder_census::{HeadingExit, MidfielderCensus};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::players::{ShotType, WonHeader};
use crate::r#match::{
    ConditionContext, MatchContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// Inside this distance of the opponent goal an aerial contact is an
/// attempt on goal (a late-arriving eight meeting a cross), not a
/// knock-down.
const ATTACKING_HEADER_RANGE: f32 = 90.0;
/// A won aerial only becomes an attempt ON GOAL when the position makes
/// it a genuine chance: an expected-goal value of at least 0.260 from
/// where he heads it. Below the bar the won header is still won — it
/// becomes the knock-down, which is what a real midfielder does with an
/// aerial ball they can't attack the goal with.
const HEADER_ON_GOAL_XG_BAR: f32 = 0.260;
/// Anti-monopoly parity with the Tier-1 foot-shot path: past this many
/// attempts the aerial win is recycled as a knock-down instead of yet
/// another attempt.
const HEADER_SHOT_CAP: u32 = 4;
/// Beyond this distance from our own goal a won header is a controlled
/// knock-down to a teammate; closer than this it's an emergency
/// clearance away from danger.
const DEFENSIVE_CLEARANCE_RANGE: f32 = 120.0;

/// How high a knock-down loops ABOVE the contact, in metres, worst
/// technique first.
///
/// A cushioned nod sits down a metre in front of the man; a poor contact
/// balloons and gives everyone time to react. Both are apexes because the
/// vertical axis is metric and a kick is described by the height it
/// reaches — see [`Ball::launch_speed_for_apex`], and see the note on
/// [`MidfielderHeadingState::knock_down_velocity`] for what writing a raw
/// `z` here did instead.
const KNOCK_DOWN_APEX_M: (f32, f32) = (2.8, 1.0);

/// …and how far it carries, in game units, worst technique first. 80 u is
/// 10 m and 144 u is 18 m — the space in front of a midfield runner,
/// which is what a knock-down is for.
const KNOCK_DOWN_RANGE_U: (f32, f32) = (80.0, 144.0);

/// How far a clearing header carries, in game units, worst header first:
/// 16 m to 27 m.
const CLEAR_RANGE_U: (f32, f32) = (130.0, 218.0);

/// How high a clearing header loops, in metres.
const CLEAR_APEX_M: f32 = 6.0;

/// Sideways scatter on the nod, as a fraction of the forward component.
/// 0.25 is about fourteen degrees either way — a header is aimed with the
/// forehead, not passed with the laces.
const KNOCK_DOWN_SCATTER: f32 = 0.25;

/// **Up for a high ball a contest has given him.** He goes to meet it and
/// takes off as it comes down to him. A ball the contest holds in the
/// heading band for him is his to strike, by where he is: at goal from a
/// genuine chance, clear from his own third, and down into the space ahead
/// otherwise. A ball the contest carries an outcome for comes off his head
/// through the delivery that brings it (`Ball::tick_aerial_delivery`).
/// Nothing here rolls for the ball again.
#[derive(Default, Clone)]
pub struct MidfielderHeadingState {}

impl StateProcessingHandler for MidfielderHeadingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let me = ctx.player.id;
        let ball = &ctx.tick_context.ball;
        if ctx.ball().is_owned() || !ctx.player.aerial_award || !ball.awaits_header_from(me) {
            #[cfg(feature = "match-logs")]
            MidfielderCensus::note_heading_exit(if ctx.ball().is_owned() {
                HeadingExit::Owned
            } else {
                HeadingExit::AwardGone
            });
            return Some(MidfielderRead::change(ctx));
        }
        let position = ctx.tick_context.positions.ball.position;
        let falling = ctx.tick_context.positions.ball.velocity.z <= 0.0;
        if falling && !ctx.player.is_airborne() && ctx.ball().distance() <= Self::TAKE_OFF {
            let apex = AerialReach::header_leap_for(position.z, ctx.player.skills.physical.jumping)
                .max(Self::HOP);
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Heading,
                Event::PlayerEvent(PlayerEvent::Leap(me, apex)),
            ));
        }
        let reached = if MatchContext::cross_head_arrival_off() {
            PlayerReach::can_strike_at(position, ctx.player, true)
        } else {
            PlayerReach::can_head(position, ctx.player)
        };
        if ball.aerial_contest_winner != Some(me) || !reached {
            return None;
        }
        Some(self.strike(ctx))
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: WonHeader::meeting_point(ctx),
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
    /// He takes off as the ball comes within 2 m of him on its way down.
    const TAKE_OFF: f32 = 16.0;
    /// He attacks a header on the jump even when it comes at his head: a
    /// hop of 0.2 m.
    const HOP: f32 = 0.2;

    /// Is this won aerial a genuine attempt-on-goal position, or a ball
    /// to cushion for a teammate? Reads the same skill-and-position xG
    /// the Tier-1 foot-shot gate reads, so "what counts as a chance"
    /// stays one concept engine-wide, plus the Tier-1 anti-monopoly cap.
    fn is_genuine_header_chance(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.memory().shots_taken > HEADER_SHOT_CAP {
            return false;
        }
        // A corner the engine's own contest awarded to THIS player IS the
        // chance. The xG bar below exists to stop a midfielder heading at
        // goal from silly positions in open play, and it is set well above
        // any header's xG — a corner header from the penalty spot is
        // 0.10-0.14 — so applied to a set piece it means a midfielder who
        // has just won the corner nods it sideways as a knock-down
        // instead of attacking the goal, every time.
        //
        // That was invisible while the box was empty, because the contest
        // almost always elected a pushed-up centre-back. `CornerShape`
        // loads the box with the side's best heads, so the winner is now
        // routinely a midfielder — and headed attempts from the six-yard
        // band fell with it.
        if ctx.ball().is_team_attacking_corner()
            && ctx.tick_context.ball.aerial_contest_winner == Some(ctx.player.id)
        {
            return true;
        }
        let profile = ctx.player().shooting().shot_profile();
        let distance = ctx.ball().distance_to_opponent_goal();
        profile.expected_xg(distance, true) >= HEADER_ON_GOAL_XG_BAR
    }

    /// The header the contest left him, played by where he meets it: at
    /// goal by [`WonHeader`], the rule every role plays a won header by.
    /// `OF_CROSS_HEAD_ARRIVAL=off` restores the midfielder's own bar.
    fn strike(&self, ctx: &StateProcessingContext) -> StateChangeResult {
        let at_goal = if MatchContext::cross_head_arrival_off() {
            ctx.ball().distance_to_opponent_goal() < ATTACKING_HEADER_RANGE
                && self.is_genuine_header_chance(ctx)
                && ctx.player().can_shoot()
        } else {
            WonHeader::at_goal(ctx)
        };
        if at_goal {
            #[cfg(feature = "match-logs")]
            {
                MidfielderCensus::note_heading_exit(HeadingExit::Shot);
                MidfielderCensus::note_header(
                    HeaderRule::HeadingShot,
                    ctx.player,
                    ctx.player.is_airborne(),
                    HeaderBooking::Shot,
                );
            }
            return StateChangeResult::with_midfielder_state_and_event(
                MidfielderRead::released(ctx),
                Event::PlayerEvent(PlayerEvent::Shoot(
                    ShootingEventContext::builder()
                        .with_player_id(ctx.player.id)
                        .with_target(ctx.player().shooting_direction())
                        .with_reason("MID_HEADER_ON_GOAL")
                        .with_shot_type(ShotType::Header)
                        .build(ctx),
                )),
            );
        }

        if ctx.ball().distance_to_own_goal() < DEFENSIVE_CLEARANCE_RANGE {
            #[cfg(feature = "match-logs")]
            {
                MidfielderCensus::note_heading_exit(HeadingExit::Clearance);
                MidfielderCensus::note_header(
                    HeaderRule::HeadingShot,
                    ctx.player,
                    ctx.player.is_airborne(),
                    HeaderBooking::Clearance,
                );
            }
            return StateChangeResult::with_midfielder_state_and_event(
                MidfielderRead::released(ctx),
                Event::PlayerEvent(PlayerEvent::ClearBall(
                    ctx.player.id,
                    self.clearing_velocity(ctx),
                )),
            );
        }

        // Midfield third: a knock-down into the space ahead, so a team-mate
        // can run onto it — the second ball the header exists for.
        #[cfg(feature = "match-logs")]
        {
            MidfielderCensus::note_heading_exit(HeadingExit::KnockDown);
            MidfielderCensus::note_header(
                HeaderRule::HeadingShot,
                ctx.player,
                ctx.player.is_airborne(),
                HeaderBooking::Nothing,
            );
        }
        StateChangeResult::with_midfielder_state_and_event(
            MidfielderRead::released(ctx),
            Event::PlayerEvent(PlayerEvent::KnockDown(
                ctx.player.id,
                self.knock_down_velocity(ctx),
            )),
        )
    }

    /// A clearing header: away from his own goal and toward the touchline,
    /// 16-27 m and 6 m high, further the better he heads it — the headed
    /// clear the cross contest plays.
    fn clearing_velocity(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let control = sc::aerial_outfield_attacker(ctx.player, minute).clamp(0.0, 1.0);
        let (short, long) = CLEAR_RANGE_U;
        Ball::headed_clear_velocity(
            ctx.tick_context.positions.ball.position,
            ctx.ball().direction_to_own_goal(),
            short + (long - short) * control,
            CLEAR_APEX_M,
        )
    }

    /// Knock-down into the space ahead: forward along the attacking
    /// direction, low and short. A knock-down travels a few metres —
    /// this is a controlled cushion, not a clearance, so the receiving
    /// teammate has a genuine chance to win the second ball.
    ///
    /// # ⚠ This was the last kick in the engine writing a raw `z`
    ///
    /// `x`/`y` are game units a tick (1 u = 12.5 cm) and `z` is METRES a
    /// tick, so the two axes share no scale. The pair here used to be
    /// `power = 1.6 + control` and `lift = 1.4 - control * 0.5` — a
    /// vertical launch of **0.9 to 1.4 m/tick, i.e. 90 to 140 m/s**, an
    /// implied apex of up to **620 metres**. Every other kick site was
    /// converted when gravity went metric (see
    /// `DefenderClearingState`, `Ball::break_stall`, the keeper's punch);
    /// this one was missed because a midfielder only wins a middle-third
    /// header a couple of times a match.
    ///
    /// What it looked like: nothing about it read as a knock-down at all.
    /// `Ball::update_velocity`'s `MAX_APEX_METRES` guard trimmed the
    /// launch to a 40 m apex — which is what that guard is for, and it
    /// held — so instead of 620 m the "controlled cushion" went up **28
    /// metres and hung for five seconds**, coming down fifty metres away
    /// or in the stand. Traced in `dev_match sky`:
    ///
    /// ```text
    ///   181451   696.73  296.26   1.63   v(-1.11, 0.65, 0.12)   loose, dropping
    /// * 181452   695.62  296.90   1.75   v(-2.20, 0.43, 1.10)   headed — 110 m/s UP
    ///   181453   693.43  297.34   2.03   v(-2.19, 0.43, 0.28)   trimmed to the ceiling
    ///   ...
    ///   181659   350.66  365.15  27.81   v(-1.30, 0.26, 0.00)   apex, 4.4 m up the stand
    /// ```
    ///
    /// The engine's own census had been reporting it the whole time —
    /// `dev_match stats`, "worst apex 693.7 m … a kick site is still
    /// writing a raw z instead of solving an apex" — but its
    /// `ABSURD_BY_STATE` attribution reads the striker's state on the
    /// tick the ball's speed changes, and this state has already returned
    /// to `Running` by then, so the row that names the site was blank.
    ///
    /// Solved from an apex now, like every other kick: pick how high the
    /// nod loops, which fixes its hang time, and the pace follows from
    /// how far it has to carry.
    fn knock_down_velocity(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let control = sc::aerial_outfield_attacker(ctx.player, minute).clamp(0.0, 1.0);
        let forward_x = ctx.player.side.map_or(1.0, |side| side.forward_dir_x());

        // Better aerial technique = a flatter, more purposeful knock-down;
        // a poor one loops up and gives everyone time to react.
        let (loop_apex, cushion_apex) = KNOCK_DOWN_APEX_M;
        let apex = loop_apex + (cushion_apex - loop_apex) * control;
        let vz = Ball::launch_speed_for_apex(apex);

        // …and reaches further, because he has got over it. Solved
        // against the height it is actually headed from — the drop is
        // most of a knock-down's carry, and the boot's ceiling
        // (`AerialReach::VOLLEY`) alone is a metre and a half of it.
        let (short, long) = KNOCK_DOWN_RANGE_U;
        let range = short + (long - short) * control;
        let struck_from = ctx.tick_context.positions.ball.position.z.max(0.0);
        let speed = Ball::launch_for_range(range, vz, struck_from);

        // Scatter turns the AIM, so it costs the nod direction rather
        // than pace — a header pulled across the pitch does not also
        // travel further for it, which is what adding a raw `y` did.
        let drift = ctx
            .context
            .rng
            .random_range(-KNOCK_DOWN_SCATTER..KNOCK_DOWN_SCATTER);
        let aim = Vector3::new(forward_x, drift, 0.0)
            .try_normalize(1.0e-4)
            .unwrap_or_else(|| Vector3::new(forward_x, 0.0, 0.0));

        Vector3::new(aim.x * speed, aim.y * speed, vz)
    }
}

use crate::r#match::engine::ball::ball::Ball;
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

/// Sideways scatter on the nod, as a fraction of the forward component.
/// 0.25 is about fourteen degrees either way — a header is aimed with the
/// forehead, not passed with the laces.
const KNOCK_DOWN_SCATTER: f32 = 0.25;

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

        // When an engine-level aerial contest (`resolve_cross_contest` /
        // `resolve_corner_contest`) has already awarded this player the
        // ball, the duel is settled — rolling `wins_duel` on top of it is
        // double jeopardy, the same bug the forward heading state and the
        // CB `AttackingCorner` state both carve out.
        let contest_awarded = ctx.tick_context.ball.aerial_contest_winner == Some(ctx.player.id);
        if !contest_awarded && !self.wins_duel(ctx) {
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
            && ctx.player().can_shoot()
            && ctx.team().can_shoot()
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
        // most of a knock-down's carry, and `HEADING_HEIGHT_THRESHOLD`
        // alone is a metre and a half of it.
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

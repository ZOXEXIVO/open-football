use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperBallClaim,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide,
    StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior,
    VectorExtensions,
};
use nalgebra::Vector3;

const DANGER_ZONE_RADIUS: f32 = 35.0;
const CLOSE_DANGER_DISTANCE: f32 = 100.0;
const FAR_THREAT_DISTANCE: f32 = 300.0;
/// Below this height the ball is on the deck — scooped up, not caught.
const GROUND_BALL_HEIGHT: f32 = 1.2;

#[derive(Default, Clone)]
pub struct GoalkeeperStandingState {}

impl StateProcessingHandler for GoalkeeperStandingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Shot in flight at our goal — react immediately. The
        // `is_towards_player_with_angle` check below would miss shots
        // aimed at the corners (angle between ball velocity and
        // ball→keeper can be ~30°, cosine < 0.6). The cached target is
        // set precisely so the keeper has a deterministic intercept
        // line; honour it.
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            if Some(target.defending_side) == ctx.player.side {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            }
        }

        // Close slow ball. A keeper who can legally use their hands does
        // two different things here depending on where the ball is:
        // a rolling or dead ball ON THE GROUND is picked up, an airborne
        // one is caught. Splitting them wires `PickingUpBall` — fully
        // implemented but previously unreachable — and stops a stationary
        // ball at the keeper's feet being modelled as a claim out of the
        // air.
        // …unless this is the ball we ourselves just put into play and it
        // has not travelled. A keeper whose delivery drops back at his feet
        // used to collect it and distribute again on the spot, over and
        // over — the pick-up path grants ownership directly, so the
        // ownership layer's own guard never saw it. Declining here is what
        // breaks the cycle: the ball stays live and an outfielder takes it.
        let own_dead_delivery = ctx.ball().blocked_from_recollecting();

        let ball_distance = ctx.ball().distance();
        if ball_distance < 10.0
            && !ctx.ball().is_owned()
            && !own_dead_delivery
            && KeeperBallClaim::is_favourite(ctx)
            && ctx.ball().on_own_side()
            && ctx.tick_context.positions.ball.velocity.norm() < 10.0
        {
            let grounded = ctx.tick_context.positions.ball.position.z < GROUND_BALL_HEIGHT;
            // Hands are only legal inside our own penalty area.
            if grounded && ctx.ball().in_own_penalty_area() {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PickingUpBall,
                ));
            }
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Catching,
            ));
        }

        // If goalkeeper has the ball, put it back in play (never run with
        // it). A GOAL KICK is a dead ball from the floor, so the hands are
        // out of play and the choice is short-pass versus long kick — the
        // keeper's kicking ability and how far the opposition have pushed
        // up decide which. Everything else (a claim, a back-pass) routes
        // through Distributing, whose own logic picks the outlet.
        if ctx.player.has_ball(ctx) {
            if ctx.ball().is_goal_kick_restart() {
                return Some(StateChangeResult::with_goalkeeper_state(
                    if Self::goes_long_from_goal_kick(ctx) {
                        GoalkeeperState::Kicking
                    } else {
                        GoalkeeperState::Distributing
                    },
                ));
            }
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Distributing,
            ));
        }

        let ball_on_own_side = ctx.ball().on_own_side();

        // Skill-based threat assessment
        let anticipation = ctx.player.skills.mental.anticipation / 20.0;
        let command_of_area = ctx.player.skills.goalkeeping.command_of_area / 20.0;

        // Check for immediate threats requiring urgent action
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let opponent_distance = opponent.distance(ctx);

            // Opponent very close with ball - prepare for save or come out
            if opponent_distance < CLOSE_DANGER_DISTANCE {
                // Check if should come out or prepare for shot
                if self.should_rush_out_for_ball(ctx, &opponent) {
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::ComingOut,
                    ));
                } else {
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::PreparingForSave,
                    ));
                }
            }

            // Opponent approaching at medium range — the dedicated
            // Attentive state added no behaviour over Standing (both
            // repositioned and rechecked threats every tick), so we stay
            // put and let the next-tick threat scan drive the response.
        }

        // Check if ball is coming toward goal — react faster to shots
        // Shot velocities are ~1.0-2.0/tick, thresholds must match
        if ctx.ball().is_towards_player_with_angle(0.6) && ball_on_own_side {
            let ball_speed = ctx.tick_context.positions.ball.velocity.norm();

            if ball_speed > 0.5 {
                // Ball moving toward goal — prepare immediately
                if ball_distance < FAR_THREAT_DISTANCE * (1.0 + anticipation * 0.5) {
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::PreparingForSave,
                    ));
                }
            }

            // Ball coming slowly — stay in Standing; PreparingForSave
            // will fire above once the ball is close enough and fast
            // enough to matter.
        }

        // NB a wider "sweep up slow balls in my area" branch was tried
        // here and REMOVED: it fired on under 0.25% of keeper ticks and
        // moved nothing. The balls trickling over the goal line are
        // crossing WIDE of the penalty area, near the corner flags, where
        // a keeper correctly does not go — and where a defender who
        // touched it last would be conceding a corner rather than saving
        // one. The endline losses are not a keeper-positioning problem.
        // See `EndlineCensus`.
        //
        // Check for loose ball in dangerous area — same self-recollect bar
        // as the close-ball branch above, or `ComingOut` simply becomes the
        // second door into the same cycle.
        if !ctx.ball().is_owned()
            && !own_dead_delivery
            && ball_on_own_side
            && ball_distance < 40.0 * (1.0 + command_of_area * 0.3)
        {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ComingOut,
            ));
        }

        // Ball on own side but no immediate threat — Standing is the
        // right rest state; Attentive was an identical idle state with
        // slightly different thresholds.

        // Check positioning
        match ctx.player().position_to_distance() {
            PlayerDistanceFromStartPosition::Small => {
                // Good positioning + opponent in danger zone → prepare for save.
                // UnderPressure was a pass-through to Catching/Distributing.
                if self.is_opponent_in_danger_zone(ctx) {
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::PreparingForSave,
                    ));
                }
            }
            PlayerDistanceFromStartPosition::Medium => {
                // Need to adjust position - walk to better spot
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Walking,
                ));
            }
            PlayerDistanceFromStartPosition::Big => {
                // Far from position - walk back
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Walking,
                ));
            }
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Calculate optimal position based on ball and goal
        let optimal_position = self.calculate_optimal_position(ctx);
        let distance_to_optimal = ctx.player.position.distance_to(&optimal_position);

        // GKs need to reposition quickly to track the ball
        if distance_to_optimal < 5.0 {
            // Close to position — small adjustments to stay ready
            Some(
                SteeringBehavior::Arrive {
                    target: optimal_position,
                    slowing_distance: 3.0,
                }
                .calculate(ctx.player)
                .velocity
                    * 0.7,
            )
        } else if distance_to_optimal < 15.0 {
            // Repositioning needed — move with purpose
            Some(
                SteeringBehavior::Arrive {
                    target: optimal_position,
                    slowing_distance: 6.0,
                }
                .calculate(ctx.player)
                .velocity
                    * 1.0,
            )
        } else {
            // Urgently out of position — sprint
            Some(
                SteeringBehavior::Arrive {
                    target: optimal_position,
                    slowing_distance: 10.0,
                }
                .calculate(ctx.player)
                .velocity
                    * 1.3,
            )
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Standing goalkeepers recover condition well due to low activity
        GoalkeeperCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}

impl GoalkeeperStandingState {

    /// Goal kick: go long, or play short to a defender?
    ///
    /// Continuous score, no threshold flip. Going long is driven by the
    /// keeper's kicking leg, by how high the opposition have pushed
    /// (nothing else is on when they squeeze the box), and by how direct
    /// the manager wants the side to be. Playing short is driven by
    /// having a genuinely free defender to give it to and the composure
    /// to do it under pressure — the modern build-out, available only to
    /// keepers who can actually pass.
    fn goes_long_from_goal_kick(ctx: &StateProcessingContext) -> bool {
        let gk = &ctx.player.skills.goalkeeping;
        let kick_skill = (gk.kicking / 20.0).clamp(0.0, 1.0);
        let short_skill = ((gk.passing + gk.first_touch) / 40.0).clamp(0.0, 1.0);
        let composure = (ctx.player.skills.mental.composure / 20.0).clamp(0.0, 1.0);

        // Opposition players who have committed into our build-out zone.
        let squeezing = ctx.players().opponents().nearby(200.0).count() as f32;
        let press = ((squeezing - 1.0) / 3.0).clamp(0.0, 1.0);

        // A defender we could actually give it to: close, and not marked.
        let free_short = ctx
            .players()
            .teammates()
            .nearby(120.0)
            .filter(|t| ctx.tick_context.grid.opponents(t.id, 18.0).next().is_none())
            .count();
        let short_available = (free_short as f32 / 2.0).min(1.0);

        // Patient sides build out; direct sides launch it.
        let patience = ctx.team().build_up_patience().clamp(0.0, 1.0);

        let long = 0.40 + kick_skill * 0.50 + press * 0.75 - patience * 0.35;
        let short =
            0.20 + short_available * 0.55 + short_skill * 0.45 + composure * 0.15 - press * 0.55;

        long >= short
    }

    fn is_opponent_in_danger_zone(&self, ctx: &StateProcessingContext) -> bool {
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let opponent_distance = ctx
                .tick_context
                .grid
                .get(ctx.player.id, opponent_with_ball.id);

            return opponent_distance < DANGER_ZONE_RADIUS;
        }

        false
    }

    /// Determine if goalkeeper should rush out for the ball
    fn should_rush_out_for_ball(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> bool {
        let ball_position = ctx.tick_context.positions.ball.position;
        let keeper_position = ctx.player.position;
        let opponent_position = opponent.position;

        // Distance calculations
        let keeper_to_ball = (ball_position - keeper_position).magnitude();
        let opponent_to_ball = (ball_position - opponent_position).magnitude();

        // Goalkeeper skills
        let anticipation = ctx.player.skills.mental.anticipation / 20.0;
        let decisions = ctx.player.skills.mental.decisions / 20.0;
        let rushing_out = (anticipation + decisions) / 2.0;

        // Opponent skills
        let opponent_control = ctx.player().skills(opponent.id).technical.first_touch / 20.0;
        let opponent_pace = ctx.player().skills(opponent.id).physical.pace / 20.0;

        // Calculate time to reach ball (rough estimate)
        let keeper_speed = ctx.player.skills.physical.acceleration * (1.0 + rushing_out * 0.3);
        let opponent_speed = opponent_pace * 20.0;

        let keeper_time = keeper_to_ball / keeper_speed;
        let opponent_time = opponent_to_ball / opponent_speed;

        // Factors favoring rushing out:
        // 1. Keeper can reach ball first (with skill advantage)
        // 2. Ball is loose or opponent has poor control
        // 3. Ball is within reasonable distance
        //
        // Eccentricity is the keeper's risk appetite — a tendency, not
        // a quality. It widens how speculative a race he's willing to
        // join and how far from goal he'll sweep; it does NOT make him
        // faster or better once committed. Centered at 10/20 so an
        // ordinary keeper keeps the pre-existing gate.
        let risk_appetite =
            (ctx.player.skills.goalkeeping.eccentricity / 20.0).clamp(0.0, 1.0) - 0.5;

        // A man dribbling at the goal has to be MET. This is the constraint
        // that was missing entirely: the race conditions below only ever
        // fired for a loose ball or a poor controller, so a forward with
        // the ball at his feet and decent first touch was free to carry it
        // all the way in while the keeper stood on his line and watched.
        // That is why forwards were seen running to within a couple of feet
        // of the keeper before shooting — there was nothing there to stop
        // them, and no amount of restraint on the FORWARD is the right fix
        // for a goalkeeper who does not come out.
        //
        // He is not racing for the ball here, he is closing the angle, so
        // this deliberately bypasses `can_reach_first` — a keeper who only
        // advances when he can win the ball first never advances at all
        // against a carrier.
        //
        // 150u ≈ 19 m: the carrier is into the area the keeper is
        // responsible for. Risk appetite widens it, so an eccentric sweeper
        // comes further and a cautious one holds his line longer.
        let carrier_bearing_down = ctx.ball().is_owned()
            && keeper_to_ball < 150.0 * (1.0 + risk_appetite * 0.4)
            && !ctx.ball().is_held_by_opponent_goalkeeper();
        if carrier_bearing_down {
            return true;
        }

        let can_reach_first =
            keeper_time < opponent_time * (1.0 + rushing_out * 0.2 + risk_appetite * 0.15);
        let ball_loose_or_poor_control = !ctx.ball().is_owned() || opponent_control < 0.5;
        let reasonable_distance =
            keeper_to_ball < CLOSE_DANGER_DISTANCE * (1.5 + risk_appetite * 0.5);

        can_reach_first && ball_loose_or_poor_control && reasonable_distance
    }

    /// Calculate optimal goalkeeper position based on ball and goal
    fn calculate_optimal_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let goal_center = ctx.ball().direction_to_own_goal();
        let ball_position = ctx.tick_context.positions.ball.position;

        // Goalkeeper skills affecting positioning
        let positioning_skill = ctx.player.skills.mental.positioning / 20.0;
        let command_of_area = ctx.player.skills.goalkeeping.command_of_area / 20.0;

        // Calculate distance from goal to ball
        let goal_to_ball = ball_position - goal_center;
        let distance_to_ball = goal_to_ball.magnitude();

        // Base distance from goal line (in meters/units)
        let mut optimal_distance_from_goal = 10.0; // Start about 10 units from goal line

        // Adjust based on ball position
        if ctx.ball().on_own_side() {
            // Ball on defensive half - position based on threat level
            let threat_distance = distance_to_ball.min(300.0) / 300.0; // Normalize to 0-1

            // Closer ball = come out more (but not too far)
            optimal_distance_from_goal += (1.0 - threat_distance) * 20.0 * command_of_area;

            // Better positioning = more accurate placement
            optimal_distance_from_goal *= 0.8 + positioning_skill * 0.4;

            // Narrow the angle - position on line between goal and ball
            let direction_to_ball = if distance_to_ball > 1.0 {
                goal_to_ball.normalize()
            } else {
                Vector3::new(1.0, 0.0, 0.0) // Fallback if ball too close to goal
            };

            let mut new_position = goal_center + direction_to_ball * optimal_distance_from_goal;

            // Lateral adjustment for angle coverage
            let ball_y_offset = ball_position.y - goal_center.y;
            let lateral_adjustment = ball_y_offset * 0.2 * positioning_skill;
            new_position.y += lateral_adjustment;

            // Keep within penalty area
            self.clamp_to_penalty_area(ctx, new_position)
        } else {
            // Ball on opponent's half - stay closer to goal but ready
            optimal_distance_from_goal = 12.0 + command_of_area * 8.0;

            let mut new_position = goal_center;
            new_position.x += optimal_distance_from_goal
                * (if ctx.player.side == Some(PlayerSide::Left) {
                    1.0
                } else {
                    -1.0
                });

            self.clamp_to_penalty_area(ctx, new_position)
        }
    }

    fn clamp_to_penalty_area(
        &self,
        ctx: &StateProcessingContext,
        position: Vector3<f32>,
    ) -> Vector3<f32> {
        let penalty_area = ctx
            .context
            .penalty_area(ctx.player.side == Some(PlayerSide::Left));
        Vector3::new(
            position.x.clamp(penalty_area.min.x, penalty_area.max.x),
            position.y.clamp(penalty_area.min.y, penalty_area.max.y),
            0.0,
        )
    }
}

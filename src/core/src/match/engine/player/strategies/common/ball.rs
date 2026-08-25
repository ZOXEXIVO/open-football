use crate::r#match::engine::ball::ball::HandlingVerdict;
use crate::r#match::result::VectorExtensions;
use crate::r#match::{BallSide, PlayerSide, StateProcessingContext};
use nalgebra::Vector3;

pub struct BallOperationsImpl<'b> {
    ctx: &'b StateProcessingContext<'b>,
}

impl<'b> BallOperationsImpl<'b> {
    pub fn new(ctx: &'b StateProcessingContext<'b>) -> Self {
        BallOperationsImpl { ctx }
    }
}

impl<'b> BallOperationsImpl<'b> {
    pub fn on_own_side(&self) -> bool {
        match self.side() {
            BallSide::Left => self.ctx.player.side == Some(PlayerSide::Left),
            BallSide::Right => self.ctx.player.side == Some(PlayerSide::Right),
        }
    }

    pub fn distance(&self) -> f32 {
        self.ctx
            .tick_context
            .positions
            .ball
            .position
            .distance_to(&self.ctx.player.position)
    }

    pub fn velocity(&self) -> Vector3<f32> {
        self.ctx.tick_context.positions.ball.velocity
    }

    #[inline]
    pub fn speed(&self) -> f32 {
        self.ctx.tick_context.positions.ball.velocity.norm()
    }

    #[inline]
    pub fn stopped(&self) -> bool {
        let velocity = self.ctx.tick_context.positions.ball.velocity;
        velocity.x == 0.0 && velocity.y == 0.0
    }

    #[inline]
    pub fn is_owned(&self) -> bool {
        self.ctx.tick_context.ball.is_owned
    }

    /// True while THIS player's team has an attacking corner in progress
    /// — from the corner award, through the taker's set-up, the cross
    /// flight, until the ball is next brought under open-play control (a
    /// header / clearance / control resets the restart to OpenPlay).
    /// Drives the corner set-up: the taker waits for the box to load and
    /// centre-backs push up to attack the delivery. Keys off the
    /// current-or-last owner being a teammate so it stays true during the
    /// cross flight (when the ball is briefly unowned).
    ///
    /// # ⚠ While the corner is being SET UP, ask the restart, not the ball
    ///
    /// The current-or-last-owner test below cannot answer this during the
    /// set-up, and it does not fail neutrally — it answers **false**. The
    /// ball is dead, so `tick_awaited_restart` strips `current_owner`
    /// every tick, and the last man to touch it is by definition an
    /// OPPONENT: he is the reason it is a corner.
    ///
    /// `DefenderAttackingCornerState` bails straight to `Returning` on
    /// this being false, so the two pushed-up centre-backs turned round
    /// and ran back to their own half the tick the corner was awarded.
    /// While corners were PLACED that was invisible — the cross left the
    /// taker's boot 50 ms after the award and they had already been
    /// teleported into the box — but it is the whole of why the walked
    /// corner's box would not fill: measured with `OF_CORNER_WALK=on`,
    /// **3.6 attackers in the box at the delivery against a placed
    /// corner's 5.4**, and a per-player probe at the kick showed the
    /// stationed men standing ON their stations (a mean 3 u away) with
    /// the centre-backs missing entirely, in `Covering` and `HoldingLine`.
    ///
    /// A pending corner restart taken by one of my own team IS my team
    /// attacking a corner. That is what the set-up is.
    pub fn is_team_attacking_corner(&self) -> bool {
        use crate::r#match::PassOriginRestart;
        if self.ctx.tick_context.ball.pass_origin_restart != PassOriginRestart::Corner {
            return false;
        }
        if let Some(taker) = self.ctx.tick_context.ball.restart_taker {
            return self
                .ctx
                .context
                .players
                .by_id(taker)
                .map(|p| p.team_id == self.ctx.player.team_id)
                .unwrap_or(false);
        }
        let owner = self.ctx.tick_context.ball.current_owner.or(self
            .ctx
            .tick_context
            .ball
            .last_owner);
        match owner {
            Some(id) => self
                .ctx
                .context
                .players
                .by_id(id)
                .map(|p| p.team_id == self.ctx.player.team_id)
                .unwrap_or(false),
            None => false,
        }
    }

    #[inline]
    pub fn is_in_flight(&self) -> bool {
        self.ctx.tick_context.ball.is_in_flight_state > 0
    }

    #[inline]
    pub fn owner_id(&self) -> Option<u32> {
        self.ctx.tick_context.ball.current_owner
    }

    /// Who is CARRYING the ball — the player another can go and challenge.
    ///
    /// [`Self::owner_id`] with one exception: a goalkeeper with the ball in
    /// his gloves owns it, but he is not carrying it in any sense another
    /// player can act on. You cannot press him, tackle him, or intercept
    /// off him; the ball is out of play until he throws or kicks it.
    ///
    /// Every press / tackle / close-down decision in the engine keys off
    /// "does an opponent have the ball", and while that answered yes for a
    /// keeper holding it, each of those states reached the same conclusion
    /// — go and challenge him — and each one bounced straight back out
    /// again when the state it entered found there was nothing to do. The
    /// result was a one-tick loop (`Pressing → Running → Pressing`) that
    /// showed on the pitch as the whole forward line shaking in place.
    ///
    /// Three earlier attempts fixed that inside the individual states and
    /// every one of them cost goals, because a state is the wrong altitude
    /// for it: eight of them route into pressing alone, and patching each
    /// exit just moved the loop somewhere else. This is the predicate they
    /// all consult, so correcting it here closes all of them at once —
    /// and it is a plain statement of the football, which is why it does
    /// not need a special case per state.
    #[inline]
    pub fn carrier_id(&self) -> Option<u32> {
        if self.ctx.tick_context.ball.held_in_hands {
            return None;
        }
        self.ctx.tick_context.ball.current_owner
    }

    /// True when this player just put the ball into play himself and it
    /// has not gone anywhere yet, so he may not collect it again. Stops
    /// the kick → lands at my feet → kick cycle; see
    /// `Ball::blocked_recollect_player` for the bounds that keep it from
    /// becoming a deadlock of its own.
    #[inline]
    pub fn blocked_from_recollecting(&self) -> bool {
        self.ctx.tick_context.ball.recollect_blocked_player == Some(self.ctx.player.id)
    }

    /// True while this ball is still the DELIVERY this player made — he
    /// played it, it is on its way, and nobody has touched it since.
    ///
    /// The complement of the bar above rather than a wider version of it:
    /// that one is about a ball that has NOT travelled, this one is about
    /// one that has. See `Ball::own_delivery_player` for the measurement
    /// that made it necessary and `KeeperDelivery` for the rule it serves.
    #[inline]
    pub fn is_my_own_delivery(&self) -> bool {
        self.ctx.tick_context.ball.delivered_by == Some(self.ctx.player.id)
    }

    /// Check if the opponent goalkeeper currently holds the ball (in hands — unchallenageable)
    ///
    /// Now genuinely means IN HANDS. It used to return true for any ball
    /// merely owned by the opposing keeper, which called off the press
    /// while he had it at his feet — the one moment a side most wants to
    /// press him.
    pub fn is_held_by_opponent_goalkeeper(&self) -> bool {
        if !self.ctx.tick_context.ball.held_in_hands {
            return false;
        }
        match self.ctx.tick_context.ball.current_owner {
            Some(owner_id) => self
                .ctx
                .context
                .players
                .by_id(owner_id)
                .is_some_and(|owner| owner.team_id != self.ctx.player.team_id),
            None => false,
        }
    }

    /// Whether THIS player — assumed to be a goalkeeper — may legally take
    /// the ball in his hands right now, and if not, why not.
    ///
    /// The three prohibitions of Law 12 in one place, so the two states
    /// that actually close a keeper's gloves on the ball (`Catching` and
    /// `PickingUpBall`) are the only sites that need to ask. Every other
    /// route into a catch — from `Standing`, `Walking`, `ComingOut`,
    /// `TakeBall`, `Diving`, `Jumping`, `PreparingForSave`,
    /// `ReturningToGoal`, eight doors in all — is covered by construction.
    pub fn handling_verdict(&self) -> HandlingVerdict {
        let keeper = self.ctx.player;
        if !self.in_own_penalty_area() {
            return HandlingVerdict::OutsideArea;
        }
        if self.ctx.tick_context.ball.hands_released_by == Some(keeper.id) {
            return HandlingVerdict::SecondTouch;
        }
        if let Some((kicker, team)) = self.ctx.tick_context.ball.deliberate_kick_by {
            if team == keeper.team_id && kicker != keeper.id {
                return HandlingVerdict::BackPass;
            }
        }
        HandlingVerdict::Legal
    }

    #[inline]
    pub fn previous_owner_id(&self) -> Option<u32> {
        self.ctx.tick_context.ball.last_owner
    }

    /// Check if the current player has had stable possession long enough to make controlled actions
    /// Returns true if the player has owned the ball for at least MIN_POSSESSION_FOR_ACTIONS ticks
    #[inline]
    pub fn has_stable_possession(&self) -> bool {
        const MIN_POSSESSION_FOR_ACTIONS: u32 = 2; // Require at least 2 ticks (~0.03 seconds) of possession

        if self.owner_id() == Some(self.ctx.player.id) {
            self.ctx.tick_context.ball.ownership_duration >= MIN_POSSESSION_FOR_ACTIONS
        } else {
            false
        }
    }

    pub fn is_towards_player(&self) -> bool {
        let (is_towards, _) = MatchBallLogic::is_heading_towards_player(
            &self.ctx.tick_context.positions.ball.position,
            &self.ctx.tick_context.positions.ball.velocity,
            &self.ctx.player.position,
            0.95,
        );
        is_towards
    }

    pub fn is_towards_player_with_angle(&self, angle: f32) -> bool {
        let (is_towards, _) = MatchBallLogic::is_heading_towards_player(
            &self.ctx.tick_context.positions.ball.position,
            &self.ctx.tick_context.positions.ball.velocity,
            &self.ctx.player.position,
            angle,
        );
        is_towards
    }

    pub fn distance_to_own_goal(&self) -> f32 {
        let target_goal = match self.ctx.player.side {
            Some(PlayerSide::Left) => Vector3::new(
                self.ctx.context.goal_positions.left.x,
                self.ctx.context.goal_positions.left.y,
                0.0,
            ),
            Some(PlayerSide::Right) => Vector3::new(
                self.ctx.context.goal_positions.right.x,
                self.ctx.context.goal_positions.right.y,
                0.0,
            ),
            _ => Vector3::new(0.0, 0.0, 0.0),
        };

        self.ctx
            .tick_context
            .positions
            .ball
            .position
            .distance_to(&target_goal)
    }

    pub fn direction_to_own_goal(&self) -> Vector3<f32> {
        // Stale side (transient mid-tick state for sent-off / swapping
        // player) returns the left-goal position rather than crashing.
        // The player is filtered out by ownership cleanup one tick
        // later, so the wrong-but-stable read is a non-issue.
        match self.ctx.player.side {
            Some(PlayerSide::Right) => self.ctx.context.goal_positions.right,
            Some(PlayerSide::Left) | None => self.ctx.context.goal_positions.left,
        }
    }

    pub fn direction_to_opponent_goal(&self) -> Vector3<f32> {
        self.ctx.player().opponent_goal_position()
    }

    pub fn distance_to_opponent_goal(&self) -> f32 {
        let target_goal = match self.ctx.player.side {
            Some(PlayerSide::Left) => Vector3::new(
                self.ctx.context.goal_positions.right.x,
                self.ctx.context.goal_positions.right.y,
                0.0,
            ),
            Some(PlayerSide::Right) => Vector3::new(
                self.ctx.context.goal_positions.left.x,
                self.ctx.context.goal_positions.left.y,
                0.0,
            ),
            _ => Vector3::new(0.0, 0.0, 0.0),
        };

        self.ctx
            .tick_context
            .positions
            .ball
            .position
            .distance_to(&target_goal)
    }

    #[inline]
    pub fn on_own_third(&self) -> bool {
        let field_length = self.ctx.context.field_size.width as f32;
        let ball_x = self.ctx.tick_context.positions.ball.position.x;

        if self.ctx.player.side == Some(PlayerSide::Left) {
            // Home team defends the left side (0 to field_length/3)
            ball_x < field_length / 3.0
        } else {
            // Away team defends the right side (2/3 to field_length)
            ball_x > field_length * 2.0 / 3.0
        }
    }

    /// True while the ball is a dead ball waiting on a goal kick. The
    /// restart flag survives until the ball is next touched in open play,
    /// so this reads true from the award through to the keeper striking
    /// it — which is exactly the window in which the keeper has to decide
    /// between playing short and going long.
    pub fn is_goal_kick_restart(&self) -> bool {
        use crate::r#match::PassOriginRestart;
        self.ctx.tick_context.ball.pass_origin_restart == PassOriginRestart::GoalKick
    }

    pub fn in_own_penalty_area(&self) -> bool {
        // TODO
        let penalty_area = self
            .ctx
            .context
            .penalty_area(self.ctx.player.side == Some(PlayerSide::Left));

        let ball_position = self.ctx.tick_context.positions.ball.position;

        (penalty_area.min.x..=penalty_area.max.x).contains(&ball_position.x)
            && (penalty_area.min.y..=penalty_area.max.y).contains(&ball_position.y)
    }

    #[inline]
    pub fn side(&self) -> BallSide {
        if (self.ctx.tick_context.positions.ball.position.x as usize)
            <= self.ctx.context.field_size.half_width
        {
            return BallSide::Left;
        }

        BallSide::Right
    }

    /// Check if current player has been notified to take the ball
    #[inline]
    pub fn is_player_notified(&self) -> bool {
        self.ctx
            .tick_context
            .ball
            .notified_players()
            .contains(&self.ctx.player.id)
    }

    /// Check if ball should be taken immediately (emergency situation)
    /// Common pattern: ball is nearby/notified, unowned, and stopped/slow-moving
    /// Increased distance and velocity thresholds to catch slow rolling balls
    pub fn should_take_ball_immediately(&self) -> bool {
        self.should_take_ball_immediately_with_distance(50.0) // Increased from 33.3
    }

    /// Check if ball should be taken immediately with custom distance threshold
    pub fn should_take_ball_immediately_with_distance(&self, distance_threshold: f32) -> bool {
        let is_nearby = self.distance() < distance_threshold;
        let is_notified = self.is_player_notified();

        if (is_nearby || is_notified) && !self.is_owned() {
            let ball_velocity = self.speed();
            if ball_velocity < 3.0 {
                // Increased from 1.0 to catch slow rolling balls
                return true;
            }
        }
        false
    }

    /// Check if ball is nearby and available (unowned or slow moving)
    pub fn is_nearby_and_available(&self, distance: f32) -> bool {
        self.distance() < distance && (!self.is_owned() || self.speed() < 2.0)
    }

    /// Check if ball is in attacking third relative to player's team
    pub fn in_attacking_third(&self) -> bool {
        let field_length = self.ctx.context.field_size.width as f32;
        self.distance_to_opponent_goal() < field_length / 3.0
    }

    /// Check if ball is in middle third of the field
    pub fn in_middle_third(&self) -> bool {
        !self.on_own_third() && !self.in_attacking_third()
    }

    /// Returns a graduated recency penalty multiplier for a player based on
    /// how recently they appear in the pass history.
    /// Most recent passer gets 0.1 (near-total block), oldest gets 0.85 (gentle nudge).
    /// Players not in history get 1.0 (no penalty).
    pub fn passer_recency_penalty(&self, player_id: u32) -> f32 {
        const PENALTIES: [f32; 5] = [0.1, 0.3, 0.5, 0.7, 0.85];

        let recent_passers = self.ctx.tick_context.ball.recent_passers();

        // Search from end (most recent) to beginning (oldest)
        for (i, &passer_id) in recent_passers.iter().rev().enumerate() {
            if passer_id == player_id {
                return if i < PENALTIES.len() {
                    PENALTIES[i]
                } else {
                    1.0
                };
            }
        }

        1.0 // Not in history
    }

    /// Get field position as percentage (0.0 = own goal, 1.0 = opponent goal)
    pub fn field_position_percentage(&self) -> f32 {
        let field_length = self.ctx.context.field_size.width as f32;
        let distance_to_own = self.distance_to_own_goal();
        (distance_to_own / field_length).clamp(0.0, 1.0)
    }
}

pub struct MatchBallLogic;

impl MatchBallLogic {
    pub fn is_heading_towards_player(
        ball_position: &Vector3<f32>,
        ball_velocity: &Vector3<f32>,
        player_position: &Vector3<f32>,
        angle: f32,
    ) -> (bool, f32) {
        let velocity_xy = Vector3::new(ball_velocity.x, ball_velocity.y, 0.0);
        let ball_to_player_xy = Vector3::new(
            player_position.x - ball_position.x,
            player_position.y - ball_position.y,
            0.0,
        );

        let velocity_norm = velocity_xy.norm();
        let direction_norm = ball_to_player_xy.norm();

        // Stationary or nearly-stationary ball cannot be heading towards anyone
        if velocity_norm < 0.01 || direction_norm < 0.01 {
            return (false, 0.0);
        }

        let normalized_velocity = velocity_xy / velocity_norm;
        let normalized_direction = ball_to_player_xy / direction_norm;
        let dot_product = normalized_velocity.dot(&normalized_direction);

        (dot_product >= angle, dot_product)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::Vector3;

    #[test]
    fn test_is_heading_towards_player_true() {
        let ball_position = Vector3::new(0.0, 0.0, 0.0);
        let ball_velocity = Vector3::new(1.0, 1.0, 0.0);
        let player_position = Vector3::new(5.0, 5.0, 0.0);
        let angle = 0.9;

        let (result, dot_product) = MatchBallLogic::is_heading_towards_player(
            &ball_position,
            &ball_velocity,
            &player_position,
            angle,
        );

        assert!(result);
        assert!(dot_product > angle);
    }

    #[test]
    fn test_is_heading_towards_player_false() {
        let ball_position = Vector3::new(0.0, 0.0, 0.0);
        let ball_velocity = Vector3::new(1.0, 1.0, 0.0);
        let player_position = Vector3::new(-5.0, -5.0, 0.0);
        let angle = 0.9;

        let (result, dot_product) = MatchBallLogic::is_heading_towards_player(
            &ball_position,
            &ball_velocity,
            &player_position,
            angle,
        );

        assert!(!result);
        assert!(dot_product < angle);
    }

    #[test]
    fn test_is_heading_towards_player_perpendicular() {
        let ball_position = Vector3::new(0.0, 0.0, 0.0);
        let ball_velocity = Vector3::new(1.0, 0.0, 0.0);
        let player_position = Vector3::new(0.0, 5.0, 0.0);
        let angle = 0.9;

        let (result, dot_product) = MatchBallLogic::is_heading_towards_player(
            &ball_position,
            &ball_velocity,
            &player_position,
            angle,
        );

        assert!(!result);
        assert!(dot_product < angle);
    }
}

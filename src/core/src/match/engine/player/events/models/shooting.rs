use crate::r#match::PassOriginRestart;
use crate::r#match::StateProcessingContext;
use crate::r#match::player::strategies::players::ShotType;
use nalgebra::Vector3;

#[derive(Debug, Clone)]
pub struct ShootingEventContext {
    pub from_player_id: u32,
    pub target: Vector3<f32>,
    pub force: f64,
    pub reason: &'static str,
    pub tick: u64,
    pub shot_type: ShotType,
}

impl ShootingEventContext {
    pub fn builder() -> ShootingEventBuilder {
        ShootingEventBuilder::new()
    }
}

pub struct ShootingEventBuilder {
    from_player_id: Option<u32>,
    target: Option<Vector3<f32>>,
    reason: Option<&'static str>,
    shot_type: Option<ShotType>,
}

impl Default for ShootingEventBuilder {
    fn default() -> Self {
        ShootingEventBuilder::new()
    }
}

impl ShootingEventBuilder {
    pub fn new() -> Self {
        ShootingEventBuilder {
            from_player_id: None,
            target: None,
            reason: None,
            shot_type: None,
        }
    }

    pub fn with_player_id(mut self, from_player_id: u32) -> Self {
        self.from_player_id = Some(from_player_id);
        self
    }

    pub fn with_target(mut self, target: Vector3<f32>) -> Self {
        self.target = Some(target);
        self
    }

    pub fn with_reason(mut self, reason: &'static str) -> Self {
        self.reason = Some(reason);
        self
    }

    pub fn with_shot_type(mut self, shot_type: ShotType) -> Self {
        self.shot_type = Some(shot_type);
        self
    }

    pub fn build(self, ctx: &StateProcessingContext) -> ShootingEventContext {
        ShootingEventContext {
            from_player_id: self.from_player_id.unwrap(),
            target: self.target.unwrap(),
            force: ctx.player().shoot_goal_power(),
            reason: self.reason.unwrap_or("No reason specified"),
            tick: ctx.current_tick(),
            shot_type: Self::classify(self.shot_type, ctx),
        }
    }

    /// Resolve the final shot type from the caller's hint plus the state
    /// of the ball.
    ///
    /// `ShotType` has always carried `Penalty`, `DirectFreeKick` and
    /// `SetPieceHeader` variants with their own conversion curves, and
    /// nothing in the engine ever produced them — `Header` was the only
    /// type any call site set, so every penalty and every free kick was
    /// scored as an ordinary open-play strike. That is what left
    /// `penalty_taking` and `free_kicks` unable to affect an outcome:
    /// they picked the taker and then the generic model took over.
    ///
    /// Classified here rather than at the ~20 call sites because the set
    /// piece is a property of the *ball*, not of the state that decided
    /// to shoot: any state striking a ball still sitting on its restart
    /// is taking that set piece.
    fn classify(hint: Option<ShotType>, ctx: &StateProcessingContext) -> ShotType {
        let restart = ctx.tick_context.ball.pass_origin_restart;
        let from_dead_ball = matches!(
            restart,
            PassOriginRestart::Corner | PassOriginRestart::DirectFreeKick
        );
        match hint {
            // A header met from a corner or a whipped free kick is a
            // set-piece header, not an open-play one. Same xG multiplier
            // today (0.55), so this is calibration-neutral — it makes the
            // shot taxonomy honest about where chances come from.
            Some(ShotType::Header) if from_dead_ball => ShotType::SetPieceHeader,
            // Only the man standing over the dead ball takes the set piece:
            // the origin outlives the kick, so a rebound off a saved penalty
            // or a shot after a short free kick would read as one too.
            Some(ShotType::FootOpenPlay) | None => {
                let kicker = ctx.tick_context.ball.set_piece_kicker == Some(ctx.player.id);
                kicker
                    .then(|| ShotType::from_restart(restart))
                    .flatten()
                    .unwrap_or_else(|| Self::open_play_kind(ctx))
            }
            Some(explicit) => explicit,
        }
    }

    /// What kind of open-play foot shot this is, read off the ball and the
    /// picture at the strike: off a save or a block, off the ground, from
    /// distance, from a cutback, or through with only the keeper to beat.
    fn open_play_kind(ctx: &StateProcessingContext) -> ShotType {
        /// Two seconds: a strike this soon after a save or a block is the
        /// follow-up.
        const REBOUND_WINDOW: u64 = 200;
        /// Struck with the ball above shin height.
        const VOLLEY_HEIGHT: f32 = 0.4;
        /// 18 m: from outside the area.
        const LONG_SHOT: f32 = 144.0;
        /// Three seconds from the pull-back to the strike.
        const CUTBACK_WINDOW: u64 = 300;
        /// 5 m: nobody this close is a man through on his own.
        const CLEAR_OF_DEFENDERS: f32 = 40.0;

        let ball = &ctx.tick_context.ball;
        let now = ctx.current_tick();
        if ball.last_rebound_tick > 0 && now.saturating_sub(ball.last_rebound_tick) <= REBOUND_WINDOW
        {
            return ShotType::Rebound;
        }
        if ctx.tick_context.positions.ball.position.z > VOLLEY_HEIGHT {
            return ShotType::Volley;
        }
        let goal = ctx.player().opponent_goal_position();
        let to_goal = goal - ctx.player.position;
        let distance = to_goal.norm();
        if distance > LONG_SHOT {
            return ShotType::LongShot;
        }
        if ball.cutback_to.is_some_and(|(receiver, tick)| {
            receiver == ctx.player.id && now.saturating_sub(tick) <= CUTBACK_WINDOW
        }) {
            return ShotType::Cutback;
        }
        let keeper = ctx.players().opponents().goalkeeper().next().map(|gk| gk.id);
        let pressed = ctx
            .players()
            .opponents()
            .nearby(CLEAR_OF_DEFENDERS)
            .any(|o| Some(o.id) != keeper);
        let lane = ctx.tick_context.space.cast_ray(
            ctx.player.position,
            to_goal / distance.max(1.0e-3),
            distance,
            &[ctx.player.id],
        );
        if !pressed && lane.is_none_or(|hit| Some(hit.collider.player_id) == keeper) {
            return ShotType::OneVOne;
        }
        ShotType::FootOpenPlay
    }
}

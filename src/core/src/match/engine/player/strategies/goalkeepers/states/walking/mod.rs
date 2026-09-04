use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperDelivery, KeeperOneOnOne, KeeperRestPosition,
    KeeperSetPieceStance, KeeperSmother, KeeperSweepLimit,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::player::strategies::processor::StateChangeResult;
use crate::r#match::player::strategies::processor::{
    StateProcessingContext, StateProcessingHandler,
};
use crate::r#match::{ConditionContext, SteeringBehavior, VectorExtensions};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperWalkingState {}

impl StateProcessingHandler for GoalkeeperWalkingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Shot in flight at our goal — break off walking and commit
        // to the save.
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            if Some(target.defending_side) == ctx.player.side {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            }
        }

        // **The ball is at a man's feet inside his own spread — take it.**
        //
        // Wired here, and into every other state he can be standing in,
        // because measured over 73 strict 1-v-1s in a recorded match he was
        // in `Walking`, `Standing`, `ReturningToGoal` or `TakeBall` for
        // **37 of them** — states with no route to [`KeeperSmother`] at all,
        // so half of every one-on-one in the game was decided by which state
        // the keeper happened to be in when it started. Every gate lives in
        // `assess`; the wiring is what was missing.
        if let Some(attempt) = KeeperSmother::assess(ctx) {
            return Some(KeeperSmother::commit(ctx, &attempt));
        }

        // …and if he is still too far out for that, go and meet him.
        if KeeperOneOnOne::duel(ctx).is_some() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::PreparingForSave,
            ));
        }

        // Direct catch for very close SLOW balls.
        //
        // ⚠ Both speed bars in this state were above the engine's own
        // `MAX_SHOT_VELOCITY` (3.2 u/tick), so neither excluded anything:
        // "slow ball" meant every ball, and a keeper mid-stroll reached
        // out and collected shots. 2.0 u/tick (25 m/s) is a driven ball —
        // past that he is making a save, not picking it up.
        //
        // ⚠ …and neither bar in this state asked whether the ball was HIS
        // OWN DELIVERY. `Standing`'s equivalents carry the recollect bar;
        // these two carried nothing at all, and they are the widest doors
        // in the keeper's whole tree. See [`KeeperDelivery`].
        let his_own = KeeperDelivery::is_his(ctx);
        if ctx.ball().distance() < 5.0
            && !ctx.ball().is_owned()
            && !his_own
            && ctx.ball().on_own_side()
            && ctx.tick_context.positions.ball.velocity.norm() < 2.0
        {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Catching,
            ));
        }

        // If goalkeeper has the ball, immediately transition to passing
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Distributing,
            ));
        }

        // Notification system: if ball system notified us to take the ball,
        // act immediately — but only for a ball in the ground he defends.
        // `GoalkeeperTakeBallState` gives up on exactly that condition, so
        // without it here the two are a two-cycle: measured, 88 entries a
        // match through this door and 100% of them reversed inside 300 ms.
        // …and about the point he would be going TO, which for a lofted
        // ball is not the point it is at. `GoalkeeperTakeBallState` gives
        // up on the landing position, so this asks about the same
        // quantity — the two used to disagree by the whole flight of a
        // cross. See the note at that give-up.
        if ctx.ball().should_take_ball_immediately()
            && !his_own
            && KeeperSweepLimit::covers(
                ctx,
                ctx.tick_context.positions.ball.landing_position,
                &GoalkeeperSkillProfile::from_ctx(ctx),
            )
        {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::TakeBall,
            ));
        }

        // Loose ball nearby — go claim it directly
        if !ctx.ball().is_owned()
            && !his_own
            && ctx.ball().distance() < 30.0
            && ctx.ball().on_own_side()
        {
            let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
            if ball_speed < 2.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Catching,
                ));
            }
        }

        // Check ball proximity and threat level
        let ball_distance = ctx.ball().distance();
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // Improved threat assessment using goalkeeper skills
        let threat_level = self.assess_threat_level(ctx);

        // **He is where he wants to be: stand there.**
        //
        // ⚠ THIS BRANCH USED TO READ `if ball_on_own_side { Standing }` —
        // unconditionally, above everything below it. Against `Standing`'s
        // own "more than 12.5 m from your kickoff dot → Walking", which the
        // rest model makes permanently true, that is a two-cycle at tick
        // resolution: 1673 transitions per keeper per match, 53% of them
        // reversing inside 300 ms. See the note in `GoalkeeperStandingState`.
        //
        // It also made most of this state unreachable. Every branch below
        // sat underneath it, so `should_come_out_advanced`,
        // `is_significantly_out_of_position` and the threat test were only
        // ever consulted with the ball in the OPPONENT'S half — the one
        // situation none of them is about.
        //
        // The two states differ in one thing and it is not where the ball
        // is: `Walking` is the keeper repositioning, `Standing` is the
        // keeper set. So he hands back when he has ARRIVED — the same
        // anisotropic, concentration-scaled test `velocity` uses to decide
        // whether to move at all — while `Standing` sends him here only
        // once he is `REPOSITION_MARGIN` times further out than that. The
        // band between the two is what stops the pair oscillating.
        if KeeperRestPosition::is_set_with(
            ctx.player.position,
            self.calculate_intelligent_position(ctx),
            prof.concentration,
            ball_distance,
            ctx.context.field_size.width as f32,
        ) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // Check if ball is coming directly at goalkeeper
        if ctx.ball().is_towards_player_with_angle(0.85) && ball_distance < 200.0 {
            // How early he sets himself for a ball coming at him is READING
            // the play, which is the positioning composite — `anticipation`
            // is its second-heaviest term and the profile bands it for
            // fatigue and prices it against the standard of the match, both
            // of which a raw attribute read skips.
            let reaction_distance = 250.0 + (prof.positioning * 100.0);

            if ball_distance < reaction_distance {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            }
        }

        // Use decision-making skill for coming out — never for a dead
        // ball he is setting for. See [`KeeperSetPieceStance`].
        if !his_own
            && self.should_come_out_advanced(ctx)
            && ball_distance < 60.0
            && KeeperSetPieceStance::pending(ctx).is_none()
        {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ComingOut,
            ));
        }

        // Check positioning using goalkeeper-specific skills
        if self.is_significantly_out_of_position(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // High threat while walking — go straight to PreparingForSave.
        // UnderPressure was a pass-through that looked for Catching
        // /Distributing next tick anyway.
        if threat_level > 0.7 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::PreparingForSave,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Same resting position as every other keeper state — see
        // `KeeperRestPosition`. This state used to own a SECOND copy of
        // the positioning model with different constants, so the same
        // keeper wanted to be in two different places depending on which
        // state he happened to be in.
        //
        // And within 10u of it he `Wander`ed, on a 6.25 m radius, forever:
        // measured **0% still** in this state, a keeper pacing aimlessly
        // around his box. Same pattern removed from the outfield walking
        // states. He stands set instead.
        //
        // …and a dead ball at his goal has a mark of its own, read ahead
        // of the rest model for the reason `Standing::velocity` gives.
        if let Some(to_mark) = KeeperSetPieceStance::steer(ctx) {
            return Some(to_mark);
        }
        let optimal_position = self.calculate_intelligent_position(ctx);
        if KeeperRestPosition::is_set_with(
            ctx.player.position,
            optimal_position,
            GoalkeeperSkillProfile::from_ctx(ctx).concentration,
            ctx.ball().distance(),
            ctx.context.field_size.width as f32,
        ) {
            return Some(Vector3::zeros());
        }

        let pace =
            KeeperRestPosition::pace(ctx.ball().distance(), ctx.context.field_size.width as f32);
        Some(
            SteeringBehavior::Arrive {
                target: optimal_position,
                slowing_distance: 8.0,
            }
            .calculate(ctx.player)
            .velocity
                * pace,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Walking state has low intensity but more activity than standing
        GoalkeeperCondition::with_velocity(ActivityIntensity::Low).process(ctx);
    }
}

impl GoalkeeperWalkingState {
    /// Assess threat level using goalkeeper mental skills
    fn assess_threat_level(&self, ctx: &StateProcessingContext) -> f32 {
        let mut threat = 0.0;

        // Reading the danger is a READING skill — positioning,
        // anticipation, decisions, concentration, which is exactly the
        // profile's `positioning` composite. This read `reflexes`, under a
        // variable named `concentration_factor`: reflexes are how fast he
        // moves once he has seen it, not how early he sees it, and the two
        // come apart in precisely the keeper this is about.
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let concentration_factor = prof.positioning;

        // Check for opponents with ball
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = opponent_with_ball
                .position
                .distance_to(&ctx.player.position);

            // Better concentration means better threat assessment
            if distance_to_opponent < 50.0 {
                threat += 0.8 * concentration_factor;
            } else if distance_to_opponent < 100.0 {
                threat += 0.5 * concentration_factor;
            }
        }

        // Check ball velocity and trajectory
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // Use anticipation to predict threats. `> 10.0` u/tick is three
        // times the engine's shot cap, so this term never once fired and
        // a ball travelling at the keeper contributed nothing to his read
        // of the danger.
        if ball_speed > 1.5 && ctx.ball().is_towards_player_with_angle(0.6) {
            threat += 0.4 * prof.positioning;
        }

        threat.min(1.0)
    }

    /// Advanced decision for coming out using goalkeeper skills
    fn should_come_out_advanced(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();

        // Key skills for coming out decisions. Four attributes read raw and
        // averaged; the profile already blends the same four (and pace,
        // composure, one-on-ones, bravery) into the two composites that
        // mean "how far he comes" and "how much ground he owns", with the
        // fatigue band and the match-standard shift the raw reads skipped.
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let coming_out_ability = (prof.rushing_out_profile + prof.command_of_area) * 0.5;

        // Base threshold adjusted by skills
        let base_threshold = 100.0;
        let skill_adjusted_threshold = base_threshold * (0.6 + coming_out_ability * 0.8); // Range: 60-140

        // Check if ball is loose and in dangerous area — and in the ground
        // he defends, which is a different question from how far it is
        // from HIM. See [`KeeperSweepLimit`].
        let ball_loose = !ctx.ball().is_owned();
        let ball_in_danger_zone = ball_distance < skill_adjusted_threshold
            && KeeperSweepLimit::covers(ctx, ctx.tick_context.positions.ball.position, &prof);

        // Check if goalkeeper can reach ball first
        if ball_loose && ball_in_danger_zone {
            let reach_ability = prof.rushing_out_profile;

            // Check if any opponent is closer
            for opponent in ctx.players().opponents().nearby(150.0) {
                let opp_distance_to_ball =
                    (opponent.position - ctx.tick_context.positions.ball.position).magnitude();
                let keeper_distance_to_ball = ball_distance;

                // Factor in goalkeeper's reach ability
                if opp_distance_to_ball < keeper_distance_to_ball * (1.0 - reach_ability * 0.3) {
                    return false; // Opponent will reach first
                }
            }

            return true;
        }

        false
    }

    /// Far enough off his spot that he jogs back rather than strolls.
    ///
    /// Reads the positioning COMPOSITE rather than `mental.positioning`
    /// raw: the composite is the one place in the keeper model that says
    /// how well he reads where he ought to be, it blends anticipation,
    /// decisions and concentration with it, and — unlike a raw attribute —
    /// it is banded for fatigue and priced against the standard of the
    /// match. Centred on `POPULATION_READ` so this is a spread across
    /// keepers and not a re-levelling of every keeper in the game.
    fn is_significantly_out_of_position(&self, ctx: &StateProcessingContext) -> bool {
        let optimal_position = self.calculate_intelligent_position(ctx);
        let current_distance = ctx.player.position.distance_to(&optimal_position);

        let read = GoalkeeperSkillProfile::from_ctx(ctx).positioning;
        let tolerance = 100.0 - (read - GoalkeeperSkillProfile::POPULATION_READ) * 80.0;

        current_distance > tolerance
    }

    /// Calculate intelligent position using multiple goalkeeper skills
    /// Delegates to the ONE shared keeper positioning model.
    ///
    /// This used to be a second, divergent copy of it — same idea,
    /// different constants — so the same keeper wanted to be in two
    /// different places depending on which state he was in. Both copies
    /// were wrong the same way: the whole depth range came to a couple of
    /// metres, so he never left his line. See `KeeperRestPosition`.
    fn calculate_intelligent_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        KeeperRestPosition::for_keeper(ctx)
    }

    // NB the old `limit_to_penalty_area` helper is gone. It clamped the
    // keeper into his own box (plus a token 10% for a commanding one),
    // which is the constraint `KeeperRestPosition` exists to remove: a
    // keeper cannot come and meet a ball played in behind if he is not
    // allowed out of his area, and the Laws only stop him HANDLING it
    // there. `GoalkeeperStandingState::clamp_sweep_range` is the bound
    // that replaced it, and this copy had been dead since.
}

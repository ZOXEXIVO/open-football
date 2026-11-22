use crate::club::player::skills::GoalkeeperSpeedContext;
use crate::r#match::engine::ball::ball::HandlingVerdict;
use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::shared::{
    ActivityIntensity, GoalkeeperCondition, KeeperBallClaim, KeeperFeetDecision, KeeperShotDive,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::keeper_census::{ClaimSite, KeeperCensus};
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, MAX_OWNER_TRACK_DISTANCE, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

/// **The flight and the landing.** Once he is back on the turf he is a man
/// on the floor, and that is `GettingUp`'s: how long he stays down, what he
/// can reach from there, and getting up with the ball or without it.
#[derive(Default, Clone)]
pub struct GoalkeeperDivingState {}

impl GoalkeeperDivingState {
    /// The leap is applied as he enters the state, so his feet are read as
    /// a landing only after it has had a tick to lift him. A dive the
    /// physics save hands him has no leap at all, and this is all of it.
    /// AI ticks, 20 ms each.
    const TAKE_OFF_TICKS: u64 = 2;
}

impl StateProcessingHandler for GoalkeeperDivingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let landed = ctx.in_state_time >= Self::TAKE_OFF_TICKS && !ctx.player.is_airborne();
        if ctx.player.has_ball(ctx) {
            // …but only if he actually has it in his GLOVES. Ownership is
            // not a gather.
            //
            // Measured: 1.3 times a match the keeper is handed the ball at
            // his FEET while in this state — and **more than half of those
            // possessions were then taken off him**, because a ball on the
            // floor inside 5u goes to whichever player has the better
            // `calculate_tackling_score`, and a prone goalkeeper has the
            // worst in the match. It was the single largest remaining
            // source of "a player takes it off the keeper and scores".
            //
            // Same rule as `GoalkeeperHoldingState`: a keeper on top of a
            // loose ball in his own box picks it up, now, which is the
            // whole point of a smother.
            if !ctx.tick_context.ball.held_in_hands {
                return Some(StateChangeResult::with_goalkeeper_state(
                    match ctx.ball().handling_verdict() {
                        HandlingVerdict::Legal => GoalkeeperState::PickingUpBall,
                        _ => KeeperFeetDecision::without_hands(ctx),
                    },
                ));
            }
            // In his gloves he comes down with it, and stays down with it.
            if !landed {
                return None;
            }
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::KeeperActionDiag::note(1);
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::GettingUp,
            ));
        }
        if landed {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::GettingUp,
            ));
        }
        // In the air the catch is the one thing he can still do.
        self.is_ball_caught(ctx)
            .then(|| Self::gather_in_the_dive(ctx))
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let dive_direction = self.calculate_dive_direction(ctx);
        let mut dive_speed = self.calculate_dive_speed(ctx);

        // **A DIVE STOPS AT THE BALL.**
        //
        // It has no `Arrive` and no braking — a body in the air cannot have
        // either — so a keeper aimed at a point 1.5 m away and travelling at
        // his own top speed for the 0.4 s the dive lasts goes THROUGH it and
        // comes down two metres the wrong side. Measured when
        // [`KeeperShotDive`] first launched him during the flight: shots
        // arriving beyond his reach went 29% → 34% and saves/on-target fell
        // two points, because he was now missing them on the far side
        // instead of the near one.
        //
        // What a real keeper does is judge the dive: he leaves the ground
        // with the pace the distance needs and no more. Recomputing the
        // target every tick and capping the step at the distance left makes
        // him decelerate into it — the same trick `SteeringBehavior::Arrive`
        // and `GoalCelebration::steer` use.
        //
        // ⚠ THE CAP HAS TO BE AGAINST A REAL PER-TICK STEP. `calculate_dive_speed`
        // returns a raw magnitude built out of 1-20 attributes — 8 to 50 — which
        // the engine's own limiter clamps to `goalkeeper_max_speed` later on. So
        // capping THAT against the distance did nothing at all until the last
        // centimetre: 20 against a gap of 15 units is still 15, which is still
        // twenty times his real step. It has to be brought onto the same scale
        // first.
        if let Some(target) = ctx
            .tick_context
            .ball
            .cached_shot_target
            .as_ref()
            .filter(|t| Some(t.defending_side) == ctx.player.side)
        {
            let crossing = KeeperShotDive::crossing_at(
                ctx.player.position.x,
                target.struck_from,
                ctx.ball().direction_to_own_goal().x,
                KeeperShotDive::aim_y(ctx, target),
            );
            let step = ctx.player.skills.goalkeeper_max_speed(
                ctx.player.player_attributes.condition,
                GoalkeeperSpeedContext::Dive,
            );
            dive_speed = dive_speed
                .min(step)
                .min((crossing - ctx.player.position).magnitude());
        }

        Some(dive_direction * dive_speed)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Diving is a very high intensity activity requiring maximum energy expenditure
        GoalkeeperCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl GoalkeeperDivingState {
    /// He gathers it, and he STAYS DOWN.
    ///
    /// This used to return `Standing`, which put him back on his feet on
    /// the same tick his gloves closed — so the one thing that makes a
    /// save read as a save, a keeper on the floor with the ball, lasted
    /// 10 ms. Returning his own state is treated as "no transition" by the
    /// processor, so the event still fires, `in_state_time` keeps running,
    /// and the `has_ball` branch owns getting him up again.
    ///
    /// …and only in his hands where Law 12 lets him use them. A dive at a
    /// ball outside his area is a block with his body: he plays it with his
    /// feet, the same answer `Catching` gives.
    fn gather_in_the_dive(ctx: &StateProcessingContext) -> StateChangeResult {
        if !ctx.ball().handling_verdict().is_legal() {
            let mut result =
                StateChangeResult::with_goalkeeper_state(KeeperFeetDecision::without_hands(ctx));
            if !ctx.player.has_ball(ctx) {
                result
                    .events
                    .add_player_event(PlayerEvent::GainBall(ctx.player.id));
            }
            return result;
        }
        StateChangeResult::with_goalkeeper_state_and_event(
            GoalkeeperState::Diving,
            Event::PlayerEvent({
                #[cfg(feature = "match-logs")]
                crate::r#match::engine::ball::ball::ownership::reception_diag::GATHER_SOURCE[2]
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                PlayerEvent::CaughtBall(ctx.player.id)
            }),
        )
    }

    fn calculate_dive_direction(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // A shot in flight: go where it is CROSSING HIS OWN DEPTH, not
        // where it is now. A ball struck from eighteen metres is still
        // fifteen away when he leaves the ground, so aiming at it puts him
        // on a heading up the pitch instead of across his goal — and
        // aiming at the goal line instead would send a keeper who had come
        // out to narrow the angle diving backwards into his own net. Same
        // crossing point the launch decision was made on, so the dive
        // finishes where it was aimed.
        if let Some(target) = ctx
            .tick_context
            .ball
            .cached_shot_target
            .as_ref()
            .filter(|t| Some(t.defending_side) == ctx.player.side)
        {
            // …or the side he GUESSED, at a penalty. See `KeeperShotDive::aim_y`.
            let crossing = KeeperShotDive::crossing_at(
                ctx.player.position.x,
                target.struck_from,
                ctx.ball().direction_to_own_goal().x,
                KeeperShotDive::aim_y(ctx, target),
            );
            if let Some(across) = (crossing - ctx.player.position).try_normalize(1e-3) {
                return across;
            }
        }
        // Prediction time scales with positioning + reaction quality.
        let prediction_time =
            (0.12 + prof.positioning * 0.26 + prof.shot_stopping * 0.22).clamp(0.10, 0.52);
        let future_ball_position = ball_position + ball_velocity * prediction_time;

        let to_future_ball = future_ball_position - ctx.player.position;
        // **NO SKILL-SCALED ANGULAR ERROR HERE.**
        //
        // This used to finish with a random rotation about `z` whose
        // amplitude was a function of the keeper: `0.05 + poor_skill_penalty
        // * 0.34 + (1 − condition) * 0.16 − positioning * 0.10`, clamped at
        // 0.55, halved by the draw — **±15.8° for the worst keeper in the
        // game and exactly 0° for an elite one**. The one thing it decided
        // was WHICH WAY HE WENT, which is not what being a poor goalkeeper
        // means: a bad keeper is slow to read it, slow off his line and
        // short of the corner, and every one of those is already modelled.
        // `dive_reach` scales how far he gets, `explosive_mult` and
        // `calculate_dive_speed` how fast, `KeeperShotReaction::reaction_ticks`
        // how late he starts, and `KeeperShotReaction::crossing_y` how long
        // he takes to read the flight — that last one is where "he guessed
        // wrong" belongs, because it moves his BODY and the ball then beats
        // him honestly.
        //
        // Reached on the fallback branch only (no live shot of his own on
        // the ball) — a loose ball, a rebound, a smother — which is still
        // 48% of all dives, so it was visible.
        if to_future_ball.norm() > f32::EPSILON {
            to_future_ball.normalize()
        } else {
            Vector3::x()
        }
    }

    fn calculate_dive_speed(&self, ctx: &StateProcessingContext) -> f32 {
        let urgency = self.calculate_urgency(ctx);
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let base_speed =
            (ctx.player.skills.physical.acceleration + ctx.player.skills.physical.agility) * 0.5;
        // Elite: 0.70 + 0.95 = 1.65x, weak: ~0.75x. `explosive_mult` is
        // already folded into `dive_reach` so don't double-apply it.
        let skill_boost = 0.70 + prof.dive_reach * 0.95;
        base_speed * urgency * skill_boost
    }

    fn calculate_urgency(&self, ctx: &StateProcessingContext) -> f32 {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;

        let own_goal = ctx.ball().direction_to_own_goal();
        let distance_to_goal = (ball_position - own_goal).magnitude();
        let velocity_towards_goal = ball_velocity
            .dot(&(own_goal - ball_position).normalize())
            .max(0.0);

        // Scale for actual ball speeds (~1.0-2.0/tick)
        let urgency: f32 = (1.0 - distance_to_goal / 100.0) * (1.0 + velocity_towards_goal / 2.0);
        urgency.clamp(1.0, 2.5)
    }

    fn is_ball_caught(&self, ctx: &StateProcessingContext) -> bool {
        // A shot, live or settled, is the save model's alone: one
        // adjudication on the ball, which puts a held save in his hands.
        if ctx.tick_context.ball.cached_shot_target.is_some() {
            return false;
        }

        // He does not take a ball off a man's foot with his hands, on the
        // floor or otherwise — the same rule `GoalkeeperCatchingState`
        // carries. Winning it back from a carrier is [`KeeperSmother`],
        // which is a contest; this is a gather.
        if ctx.ball().is_owned() && !ctx.player.has_ball(ctx) {
            return false;
        }

        let ball_distance = ctx.ball().distance();
        // Must be flying toward the GK or very close.
        if ball_distance > 5.0 && !ctx.ball().is_towards_player_with_angle(0.6) {
            return false;
        }

        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // How far a diving keeper can get a hand on the ball. Scales with
        // skill, but bounded by `MAX_OWNER_TRACK_DISTANCE` for the same
        // hard reason as `effective_catch_distance` — see the note there.
        // This is where the dive ENDS, not how far he travels to make it
        // (that is `effective_dive_distance`, up to 48u), so gathering one
        // beyond the range a ball can be owned at left it dead on the grass
        // a tick later with its velocity already zeroed.
        let catch_distance =
            (6.0 + prof.dive_reach * 10.0 + prof.handling_profile * 4.0 + prof.shot_stopping * 4.0)
                .min(MAX_OWNER_TRACK_DISTANCE);
        if ball_distance > catch_distance {
            return false;
        }

        // ONE DIVE, ONE OPPORTUNITY, and the same gather as any other.
        #[cfg(feature = "match-logs")]
        KeeperCensus::claim_roll_in(ctx, ClaimSite::Diving);
        KeeperBallClaim::gathers_this_tick(ctx)
    }
}

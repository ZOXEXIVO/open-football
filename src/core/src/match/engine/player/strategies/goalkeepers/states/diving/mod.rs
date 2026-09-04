use crate::club::player::skills::GoalkeeperSpeedContext;
use crate::r#match::engine::ball::ball::HandlingVerdict;
use crate::r#match::engine::ball::ball::contest::save::SaveModel;
use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperFeetDecision, KeeperShotDive, KeeperShotSave,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, MAX_OWNER_TRACK_DISTANCE, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

/// Longest a keeper stays down, in AI ticks (20 ms each — see the units
/// note in `process`). 90 is 1.8 s: a full sprawl plus getting back up.
const MAX_DIVE_TICKS: u64 = 90;
const BALL_CLAIM_DISTANCE: f32 = 14.0;

#[derive(Default, Clone)]
pub struct GoalkeeperDivingState {}

impl StateProcessingHandler for GoalkeeperDivingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // A DIVE HAS A DURATION. He leaves his feet, takes the ball, and
        // has to get up again before he can do anything with it.
        //
        // The physics save (`Ball::try_save_shot`) puts him here and hands
        // him the ball on the SAME tick, so without a floor every caught
        // dive lasted one tick — the keeper made ~86 saves a match while
        // `Goalkeeper: Diving` sat below 0.25% of his ticks, which is what
        // "he doesn't dive, he just sits on it" looks like from the stands.
        //
        // A timer was added for that and it did not work, because
        // `is_ball_caught` below rolls a fresh catch every tick and used to
        // exit to `Standing` when it came off. With the ball already in his
        // gloves that roll sees distance ≈ 0 and difficulty ≈ 0, so it came
        // off on the FIRST tick more than half the time and the guard was
        // bypassed on most dives. A man who already has the ball is not
        // catching it again: the timer is now his only way out.
        //
        // UNITS: `in_state_time` counts AI TICKS, and only every second
        // engine tick runs the state machine (`game_tick_light` leaves the
        // counter alone), so one unit here is **20 ms**, not 10 — the same
        // trap `GoalkeeperHoldingState` documents at `MIN_HOLDING_DURATION`.
        // 40 ticks is therefore 0.8 s: leave the ground, take it, land, get
        // back up. `Diving` is already a committed action, so nothing else
        // can abort it either.
        const MIN_DIVE_TICKS: u64 = 40;
        if ctx.player.has_ball(ctx) {
            // …but only if he actually has it in his GLOVES. Ownership is
            // not a gather, and this branch lies on the ball for the whole
            // 0.8 s floor without ever asking which of the two it is.
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
            if ctx.in_state_time >= MIN_DIVE_TICKS {
                #[cfg(feature = "match-logs")]
                crate::mid_run_diag::KeeperActionDiag::note(1);
                // `HoldingBall`, not `Passing`: a keeper who has just made
                // a save gets up, carries it out and CHOOSES how to
                // release it — short, throw or kick — which is what
                // `GoalkeeperHoldingState::pick_distribution` is for, and
                // is the same route `Catching` has always taken. Going
                // straight to `Passing` skipped the choice entirely, so a
                // keeper's `throwing` and `kicking` did nothing after the
                // one kind of possession he most often gets.
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::HoldingBall,
                ));
            }
            return None;
        }

        // A dive he does NOT hold still costs him a trip to the floor, and
        // he is up off it faster than one where he has the ball to
        // protect. Without a floor here every miss let him straight back
        // up — `is_ball_nearby` at 14u catches most parry drops on the
        // first tick, and the parry itself puts the ball 12-30u away — so
        // the engine's mean dive ran well under the 390-660 ms a real one
        // spends airborne alone. 22 AI ticks is 0.44 s (see the units note
        // above). Nothing below happens until then; the catch roll is the
        // one thing he can still do while extended.
        //
        // …AND HE DOES NOT GET UP BEFORE HE LANDS. The tick floor alone
        // was enough while a dive could only START at the moment of
        // contact; [`KeeperShotDive`] now launches him during the flight,
        // so the floor can expire with the keeper still in the air and the
        // ball still coming. `is_airborne` is the physical condition and
        // needs no second clock — the engine has him on a real ballistic
        // arc (see `MatchPlayer::height`).
        const MIN_GROUND_TICKS: u64 = 22;
        if ctx.in_state_time < MIN_GROUND_TICKS || ctx.player.is_airborne() {
            if self.is_ball_caught(ctx) {
                return Some(Self::gather_in_the_dive(ctx));
            }
            return None;
        }

        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_moving_away =
            ball_velocity.dot(&(ctx.player.position - ctx.ball().direction_to_own_goal())) > 0.0;

        // Ball is far away and moving away from goal — the dive missed
        // the line, the shot went wide, or the ball was already deflected
        // by something else. The keeper never touched the ball at this
        // distance, so this is NOT a parry; just exit the dive.
        // (A real parry happens at close range and is handled by the
        // is_ball_caught / is_ball_nearby branches below.)
        if ctx.ball().distance() > 100.0 && ball_moving_away {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        if self.is_ball_caught(ctx) {
            return Some(Self::gather_in_the_dive(ctx));
        } else if self.is_ball_nearby(ctx) {
            let mut result = StateChangeResult::with_goalkeeper_state(GoalkeeperState::Catching);
            // …but only a LOOSE one. A keeper picking himself up beside a
            // striker who still has the ball does not reach out and take
            // it off his foot: that is the "he does not tackle with his
            // hands" rule `GoalkeeperCatchingState` documents, and the
            // legitimate way to win it back from here is the smother.
            if !ctx.ball().is_owned() {
                result
                    .events
                    .add_player_event(PlayerEvent::ClaimBall(ctx.player.id));
            }
            return Some(result);
        }

        // Backstop: he is up and back in the game whatever happened to the
        // ball. This replaces a PAIR of timeouts — a `MAX_DIVE_TIME` of
        // 1.8 "seconds" read as `in_state_time / 100.0`, which in AI ticks
        // is 3.6 s and sat behind this one, so it could never fire.
        if ctx.in_state_time > MAX_DIVE_TICKS {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        None
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
    fn gather_in_the_dive(ctx: &StateProcessingContext) -> StateChangeResult {
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
        // A LIVE SHOT IS RESOLVED BY ONE MODEL, WHEREVER HE IS STANDING.
        //
        // `Catching` used to own the state-machine save roll and its
        // per-tick conversion is calibrated against the keeper spending
        // the whole flight in that state. Now that he can leave his feet
        // part-way through it, running a second, differently-shaped model
        // here would make the realised save rate a function of when he
        // dived. See [`KeeperShotSave`].
        if ctx.tick_context.ball.cached_shot_target.is_some() {
            return KeeperShotSave::roll(ctx);
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

        let ball_speed = ctx.tick_context.positions.ball.velocity.magnitude();
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // How far a diving keeper can get a hand on the ball. Scales with
        // skill, but bounded by `MAX_OWNER_TRACK_DISTANCE` for the same
        // hard reason as `effective_catch_distance` — see the note there.
        // This is where the dive ENDS, not how far he travels to make it
        // (that is `effective_dive_distance`, up to 48u), so gathering one
        // beyond the range a ball can be owned at left it dead on the grass
        // a tick later with its velocity already zeroed.
        //
        // The `stretch` denominator below moves with it, which is correct
        // and not incidental: stretch is "how far out on the edge of my
        // reach is this", so it has to be measured against the reach that
        // actually applies.
        let catch_distance =
            (6.0 + prof.dive_reach * 10.0 + prof.handling_profile * 4.0 + prof.shot_stopping * 4.0)
                .min(MAX_OWNER_TRACK_DISTANCE);
        if ball_distance > catch_distance {
            return false;
        }

        // Catch difficulty: stretch + power + fatigue + poor position.
        //
        // `power` divided by 6.0 against a ball that cannot exceed
        // `MAX_SHOT_VELOCITY` (3.2), so the hardest strike in the game
        // reached 0.28 of the range and how hard the ball was hit barely
        // entered the sum — see the units note in `comingout`. Signed
        // against an ordinary strike, which is what keeps re-opening the
        // axis from also re-calibrating the save rate.
        let stretch = (ball_distance / catch_distance).clamp(0.0, 1.0);
        let power = SaveModel::strike_power(ball_speed);
        let catch_difficulty = (power * 0.36
            + stretch * 0.30
            + (1.0 - prof.condition_mult) * 0.14
            + (1.0 - prof.positioning) * 0.10
            + prof.poor_skill_penalty * 0.10)
            .clamp(0.0, 1.0);

        let mut catch_prob = prof.catch_probability(catch_difficulty);
        // Deflection damping — match the `catching/mod.rs` path. A GK
        // mid-dive after a deflection has committed to the original
        // line; the redirected ball arrives on a trajectory their
        // outstretched arm wasn't shaped for. ~50% reduction matches
        // real PL's deflected-goal share.
        if let Some(t) = &ctx.tick_context.ball.cached_shot_target {
            if t.deflected {
                catch_prob *= 0.50;
            }
        }
        // ONE DIVE, ONE OPPORTUNITY. This is rolled every tick the keeper
        // is within reach, so taken as a per-event probability it compounds
        // to a near-certain catch over a dive of any length — the same
        // defect `GoalkeeperCatchingState` documents at `EXPECTED_SAVE_TICKS`
        // and the same fix: spread the per-opportunity chance across the
        // ticks the opportunity lasts. `CONTACT_TICKS` is how long a
        // diving keeper is actually within a glove's reach of the ball.
        const CONTACT_TICKS: f32 = 14.0;
        let per_tick = prof.per_tick_save(catch_prob, CONTACT_TICKS);
        ctx.context.rng.unit_f32() < per_tick
    }

    fn is_ball_nearby(&self, ctx: &StateProcessingContext) -> bool {
        ctx.ball().distance() < BALL_CLAIM_DISTANCE
    }
}

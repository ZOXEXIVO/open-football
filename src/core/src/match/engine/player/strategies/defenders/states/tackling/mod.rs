use crate::r#match::common_states::LooseBallChase;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::events::Event;
use crate::r#match::player::events::{FoulSeverity, PlayerEvent};
use crate::r#match::player::strategies::common::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::common::states::TackleEngagement;
use crate::r#match::player::strategies::common::states::{
    RecoveryChallenge, TackleDecision, TackleOutcome,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

// Contact / commit / disengage distances live on `TackleEngagement` in
// `defenders::states::common`, shared with `DefenderPressingState` so the
// two states can no longer disagree about the same carrier.
const PRESSING_DISTANCE: f32 = 80.0;
const RETURN_DISTANCE: f32 = 120.0;

#[derive(Default, Clone)]
pub struct DefenderTacklingState {}

impl StateProcessingHandler for DefenderTacklingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        #[cfg(feature = "match-logs")]
        crate::tackle_stats::DEF_ENTRIES.fetch_add(1, Ordering::Relaxed);

        // If we have the ball or our team controls it, transition to running
        if ctx.player.has_ball(ctx) || ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = opponent.distance(ctx);

            // The carrier got away — break off and press the space.
            // `DISENGAGE` sits outside the `COMMIT` distance `Pressing`
            // uses to send us here, so the two states can't hand the
            // defender back and forth (see `TackleEngagement`).
            if distance_to_opponent > TackleEngagement::DISENGAGE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // BEATEN, BUT STILL IN REACH.
            //
            // Above contact range the state used to have exactly one
            // answer — keep closing — and for a defender the carrier has
            // already gone past, "keep closing" is a run at a goal-side
            // contain point he cannot reach (see
            // `RecoveryChallenge`). Measured, that is 47% of the nearest
            // defenders to a moving carrier travelling PARALLEL to him at
            // 2.66 m, rolling nothing for the whole carry.
            //
            // A real defender in that position throws a leg at it. This
            // is that challenge, and it is deliberately the ONLY thing
            // that changes here: a defender who is still goal-side keeps
            // closing exactly as before, because from in front the block
            // tackle is the right challenge and the jockey model already
            // prices it.
            let lead = RecoveryChallenge::lead(ctx, opponent.position);
            if RecoveryChallenge::is_available(ctx, distance_to_opponent, lead)
                && TackleEngagement::may_engage_carrier(ctx)
                && RecoveryChallenge::is_eligible(ctx)
                && ctx.context.rng.bernoulli(RecoveryChallenge::commits_now(
                    ctx,
                    distance_to_opponent,
                    lead,
                ))
            {
                #[cfg(feature = "match-logs")]
                crate::tackle_stats::DEF_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
                let def_profile = DefenderSkillProfile::from_ctx(ctx);
                let minute = sc::minute_from_ms(ctx.context.total_match_time);
                let outcome = RecoveryChallenge::resolve(
                    ctx,
                    sc::defensive_duel(ctx.player, minute),
                    def_profile.discipline,
                    TackleDecision::carrier_threat(ctx, &opponent),
                    distance_to_opponent,
                    lead,
                );
                let mut result = self.settle(ctx, outcome);
                result.start_tackle_cooldown = true;
                return Some(result);
            }

            // Committed but not yet in contact range: keep closing. The
            // velocity fn below is already a Pursuit onto the carrier, so
            // holding the state IS the closing-down run. This branch used
            // to bounce straight back to Pressing, which is why a tackle
            // never survived past its entry tick. The `in_state_time > 30`
            // guard further down still bounds how long we can chase.
            if distance_to_opponent > TackleEngagement::CONTACT {
                return None;
            }

            // Per-player tackle cooldown: a single per-state-machine gate
            // (e.g. Pressing→Tackling cadence) never held because the
            // Tackling state can be re-entered from Standing / Running /
            // Covering / Guarding / HoldingLine, each with its own
            // distance trigger and no shared cooldown. The cooldown lives
            // on the player itself — whatever path routed us here, if we
            // just tackled, we can't tackle again for 5 s (250 AI ticks;
            // see `MatchPlayer::start_tackle_cooldown`).
            if !ctx.player.can_attempt_tackle() {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // ⚠ THE LICENCE GUARDED THE ENTRY TICK, AND THE ENTRY TICK
            // CANNOT REACH IT.
            //
            // This read `ctx.in_state_time == 0`, and the distance guard
            // above returns first for anybody outside `CONTACT` — which
            // is every carrier-based entry the engine makes, because a
            // state is entered at `COMMIT` and challenges at `CONTACT`.
            // So an unlicensed defender was never tested, closed all the
            // way in, and rolled the challenge below with no gate at all.
            // The question belongs to the CHALLENGE, and asked here it is
            // asked once per physical attempt.
            if !TackleEngagement::may_engage_carrier(ctx) {
                return None;
            }

            // JOCKEY FIRST.
            //
            // Reaching contact range is not a reason to lunge. Every tick
            // spent here used to produce an attempt (bounded only by the
            // ~1 s cooldown), which is why defenders committed 11.9
            // tackles a match against a real ~1.6 — they were tackling
            // roughly as often as the physics allowed rather than as
            // often as football does.
            //
            // `TackleDecision` rolls the moment instead: the defender
            // stands his man up, and dives in when the carrier's weight
            // is committed, when there is cover behind, or when the
            // danger makes waiting worse than missing. Declining keeps
            // him IN this state — the velocity below then contains
            // rather than pursues — so he stays on the carrier's
            // shoulder instead of being handed back to `Pressing`, which
            // is what containing actually looks like.
            if !TackleDecision::is_eligible(ctx)
                || !ctx
                    .context
                    .rng
                    .bernoulli(TackleDecision::commits_now(ctx, distance_to_opponent))
            {
                return None;
            }

            // We're close enough to tackle! One shot per Tackling entry,
            // enforced by the cooldown.
            #[cfg(feature = "match-logs")]
            crate::tackle_stats::DEF_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
            let mut result = self.settle(ctx, self.attempt_sliding_tackle(ctx, &opponent));
            result.start_tackle_cooldown = true;
            return Some(result);
        } else {
            // Ball is loose - check for interception
            // Double-check not in flight before claiming
            if self.can_intercept_ball(ctx) && !ctx.ball().is_in_flight() {
                // Ball is loose and we can intercept it
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Running,
                    Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
                ));
            }

            // If ball is too far away and not coming toward us, return to position
            let ball_distance = ctx.ball().distance();
            if ball_distance > RETURN_DISTANCE && !ctx.ball().is_towards_player_with_angle(0.8) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ));
            }

            // Fallback: if ball is loose and very close, try to claim it
            // Double-check not in flight before claiming
            if !ctx.tick_context.ball.is_owned && ball_distance < 5.0 && !ctx.ball().is_in_flight()
            {
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Running,
                    Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
                ));
            }

            // If opponent is near the player but doesn't have the ball, maybe it's better to transition to pressing
            if let Some(close_opponent) = ctx.players().opponents().nearby(15.0).next()
                && close_opponent.distance(ctx) < 10.0
            {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }
        }

        if ctx.in_state_time > 30 {
            let ball_distance = ctx.ball().distance();
            if ball_distance > PRESSING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ));
            }
            // Stuck in tackling too long without engaging — drop back to standing
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let target = self.calculate_intelligent_target(ctx);

        // Closing-speed boost driven by the unified defender profile —
        // press_profile + tackle_profile already combine acceleration,
        // anticipation, work_rate, tackling, balance with fatigue.
        let def_profile = DefenderSkillProfile::from_ctx(ctx);
        let speed_boost = def_profile.tackle_speed_boost();

        Some(
            SteeringBehavior::Pursuit {
                target,
                target_velocity: Vector3::zeros(),
            }
            .calculate(ctx.player)
            .velocity
                * speed_boost
                + ctx.player().separation_velocity() * 0.15,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tackling is explosive and very demanding physically
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl DefenderTacklingState {
    fn calculate_intelligent_target(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let player_position = ctx.player.position;
        let own_goal_position = ctx.ball().direction_to_own_goal();

        // With a live carrier, the MOVEMENT is always the jockey: get
        // goal-side of him and stay on his shoulder. The challenge itself
        // is an instantaneous event decided in `process`, so running
        // through the man is never the right path — it takes the defender
        // past the ball whether or not he commits, and it is the visible
        // half of "defenders lunge at everything".
        if let Some(carrier) = ctx.players().opponents().with_ball().next() {
            let gap = (carrier.position - player_position).magnitude();
            if gap <= TackleEngagement::DISENGAGE {
                return TackleDecision::contain_position(ctx, carrier.position);
            }
        }

        // Check if ball is dangerously close to own goal
        let ball_distance_to_own_goal = (ball_position - own_goal_position).magnitude();
        let is_ball_near_own_goal =
            ball_distance_to_own_goal < ctx.context.field_size.width as f32 * 0.2;

        // Check if we're between the ball and our goal
        let player_distance_to_own_goal = (player_position - own_goal_position).magnitude();
        let is_player_closer_to_goal = player_distance_to_own_goal < ball_distance_to_own_goal;

        if is_ball_near_own_goal && !is_player_closer_to_goal {
            // If ball is near our goal and we're not between ball and goal,
            // position ourselves between the ball and the goal
            let ball_to_goal_direction = (own_goal_position - ball_position).normalize();
            let intercept_distance = 5.0; // Stand 5 units in front of the ball towards our goal
            ball_position + ball_to_goal_direction * intercept_distance
        } else {
            // Otherwise, pursue the ball directly
            ball_position
        }
    }

    /// One resolved challenge, turned into the state change it implies.
    /// Both challenge paths end here so a win, a foul and a miss cannot
    /// mean different things depending on which one was made.
    fn settle(&self, ctx: &StateProcessingContext, outcome: TackleOutcome) -> StateChangeResult {
        match outcome {
            TackleOutcome::Won => {
                #[cfg(feature = "match-logs")]
                crate::tackle_stats::DEF_SUCCESSES.fetch_add(1, Ordering::Relaxed);
                StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::TacklingBall(ctx.player.id)),
                )
            }
            TackleOutcome::Foul(severity) => StateChangeResult::with_defender_state_and_event(
                DefenderState::Standing,
                Event::PlayerEvent(PlayerEvent::CommitFoul(ctx.player.id, severity)),
            ),
            // He missed and the man is gone. Pressing is where a beaten
            // defender goes to get back into the picture.
            TackleOutcome::Missed => {
                StateChangeResult::with_defender_state(DefenderState::Pressing)
            }
        }
    }

    fn attempt_sliding_tackle(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> TackleOutcome {
        let rng = &ctx.context.rng;

        // Unified defender profile drives both the success and the foul
        // model. tackle_profile blends tackling/positioning/anticipation/
        // composure/strength/balance/agility; discipline is the
        // composure/decisions/concentration blend that suppresses fouls.
        let def_profile = DefenderSkillProfile::from_ctx(ctx);
        let aggression01 = (ctx.player.skills.mental.aggression / 20.0).clamp(0.0, 1.0);

        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let attacker_score = TackleDecision::carrier_threat(ctx, opponent);

        // ⚠ BOTH SIDES OF THE DUEL MUST BE ON ONE SCALE.
        //
        // This scored `tackle_profile`, which subtracts
        // `MatchStandard::shift` from every input, against an ABSOLUTE
        // carry composite. Differencing a peer-relative score against an
        // absolute one leaves the standard of the match in the result:
        // the same equal-quality matchup won 55% at the bottom of the
        // pyramid and 24% at the top. `defensive_duel` is the same family
        // of composite as `dribble_attack`, so the difference is level at
        // every level. The profile still drives the decision, the
        // movement and the discipline — it is a selection model, not a
        // contest one.
        let raw_diff = sc::defensive_duel(ctx.player, minute) - attacker_score;
        let success_chance = (1.0 / (1.0 + (-raw_diff * 2.4).exp())).clamp(0.06, 0.55);

        let tackle_success = rng.random::<f32>() < success_chance;

        // Foul chance driven by discipline (composure/decisions/
        // concentration/tackling blend) instead of raw composure +
        // aggression. Tired defenders foul more (low def_condition_mult
        // adds to foul rate). Failed tackles roughly double the rate.
        //
        // Base 0.030 → 0.052 (2026-06 discipline recalibration): the
        // engine whistled 6.3 fouls/team vs real ~12 — duel contact was
        // committing too cleanly given the volume of attempts. Raising
        // the contact-foul rate (paired with the referee Normal-band
        // lift) brings whistled fouls, free kicks, and the
        // persistent-infringement card pipeline toward real rates.
        // Second lift 0.052 → 0.075: the first round's gain was eaten by
        // the Reckless probability gate reclassifying card-fouls into
        // normal contact (whistled at ~0.5 instead of ~0.85), netting
        // only +10% whistles. Real per-duel foul rates are ~15-16%; the
        // engine sat at ~10% after round one.
        // A FOUL IS A FAILED CHALLENGE, and the rate has to be read
        // against the number of challenges actually made.
        //
        // These constants were fitted when the engine attempted ten times
        // as many tackles as football does — the per-attempt rate had to
        // be tiny to keep the foul TOTAL anywhere near real. With the
        // jockey model bringing attempts down to a realistic count, the
        // same rate produced 2.1 fouls per team against a real ~12, 0.88
        // yellows against 3.5-4.5, and 1.5 direct free kicks against
        // 20-24: the whole disciplinary chain starved.
        //
        // Real football makes roughly 30 challenges a team and fouls on
        // about 12 of them, so a challenge that does not win the ball
        // cleanly is a foul a large fraction of the time. Lifted ~3× to
        // match, with the own-box restraint and the booking damping below
        // still holding penalties and second yellows down.
        let mut base_foul = 0.34 + aggression01 * 0.34 - def_profile.discipline * 0.20;
        if !tackle_success {
            base_foul *= 1.80;
        }
        base_foul += (1.0 - def_profile.def_condition_mult).max(0.0) * 0.08;
        // Own-box restraint: real defenders stay on their feet inside
        // their own penalty area — they jockey, contain and block
        // rather than commit, because the downside is a spot kick.
        // Without this, the foul-rate lift would inflate penalties
        // (already over real rate before the lift). Uses the actual
        // penalty-area RECTANGLE (the same one the restart award
        // checks) — an earlier radius-from-goal proxy missed the box
        // corners and let wide-channel box fouls escape the restraint.
        let in_own_box = ctx
            .context
            .penalty_area(ctx.player.side == Some(PlayerSide::Left))
            .contains(&ctx.tick_context.positions.ball.position);
        // ⚠ 0.008 → 0.06, AND THE OLD VALUE WAS BELOW ITS OWN CLAMP.
        //
        // `base_foul` before this line is 0.34-0.68 for a plausible
        // defender, so ×0.008 lands on 0.003-0.005 — under the
        // `clamp(0.006, …)` two lines down. Every in-box challenge
        // therefore fouled at exactly 0.6%, identically, whatever the
        // defender's aggression, discipline or condition and whether or
        // not he won it: the constant was saturating a floor rather than
        // expressing a tendency, and any value below ~0.012 produced the
        // same match.
        //
        // It was also the THIRD restraint stacked on one event.
        // `TackleDecision::box_restraint` already prices whether he
        // commits at all (0.14 with cover, 0.60 as the last man) and
        // `ContactFoul::BOX_PENALTY_RESTRAINT` prices the non-challenge
        // fouls; between them the tackle path was contributing on the
        // order of 0.02 penalties a match and the engine sat at 0.18
        // against a real 0.25-0.30 — under the target while reading as
        // if the penalty budget were spent, which is why every previous
        // pass over this area correctly refused to loosen anything.
        //
        // 0.06 says a challenge in your own box is about fifteen times
        // less likely to be a foul than the same challenge in open play
        // — he goes in with his feet and not through the man — which is
        // a tendency rather than a floor, and lets the attributes
        // upstream reach the outcome again.
        //
        // ⚠ 0.06 → 0.032, PAIRED WITH `TackleDecision::BOX_RESTRAINT`.
        //
        // That constant went 0.14 → 0.32 because a defender was
        // challenging the carrier in his own area less than half as often
        // as in open play, which is not how anybody defends a penalty
        // box. This is a per-CHALLENGE rate and the challenge count
        // roughly doubles with it, so holding it at 0.06 would have
        // bought the defending with penalties: the pair together keep the
        // per-match penalty rate near where it was calibrated.
        //
        // It is also the more honest of the two numbers. The whole reason
        // a defender may commit inside his own box is that he commits
        // DIFFERENTLY there — front foot, ball first, no follow-through —
        // and the rate at which that produces a spot kick is the thing
        // this line is for. ~1.8% of box challenges, against a real ~2.5%
        // of which the shirt-pull path owns a share of its own.
        if in_own_box {
            base_foul *= 0.032;
        }
        // Self-preservation on a booking: a player carrying a yellow
        // measurably tones the challenges down (and managers hook the
        // ones who can't).
        if ctx.player.yellow_cards > 0 {
            base_foul *= 0.70;
        }
        let foul_chance = base_foul.clamp(0.006, 0.60);

        let committed_foul = rng.random::<f32>() < foul_chance;

        // Severity classification stays driven by aggression — discipline
        // already gates whether a foul fires at all, so we don't need to
        // double-dip here.
        //
        // Violent gate 0.12 → 0.02 and a 0.35 probability gate on
        // Reckless (2026-06): the old shape classified most failed
        // aggressive contact as Reckless and produced ~1.0 red
        // cards/match (real ~0.15) — violent conduct is a
        // once-in-ten-matches event, not an every-match one, and the
        // typical failed tackle is just a normal foul.
        //
        // …and the severity TAIL is damped in your own box for the same
        // reason the foul rate above it is. The red-card challenge is the
        // lunge in midfield and the professional foul on a breakaway; the
        // last-ditch block in your own area is the one challenge a
        // defender makes with his weight back, because he already knows
        // what going through the man costs there. Measured over the
        // change that let defenders challenge in their own box at all
        // (`TackleDecision::BOX_RESTRAINT` 0.14 → 0.32), reds went 0.00 →
        // 0.42 a match against a real 0.15-0.20 on a twelve-fixture
        // sample — small, but pointing the one way this pair of changes
        // could plausibly push it.
        let box_severity = if in_own_box { 0.30 } else { 1.0 };
        let severity = if !committed_foul {
            FoulSeverity::Normal
        } else if aggression01 > 0.75
            && !tackle_success
            && rng.random::<f32>() < 0.008 * box_severity
        {
            FoulSeverity::Violent
        } else if !tackle_success
            && aggression01 > 0.55
            && rng.random::<f32>() < 0.16 * box_severity
        {
            FoulSeverity::Reckless
        } else {
            FoulSeverity::Normal
        };

        TackleOutcome::of(tackle_success, committed_foul, severity)
    }

    fn exists_nearby(&self, ctx: &StateProcessingContext) -> bool {
        const DISTANCE: f32 = 30.0;

        ctx.players().opponents().exists(DISTANCE) || ctx.players().teammates().exists(DISTANCE)
    }

    /// Can he get a foot to a ball nobody owns?
    ///
    /// ⚠ The prediction is [`LooseBallChase::meeting_point`], not
    /// `distance / pace`. `pace` is a 1-20 attribute; dividing a
    /// field-unit distance by it produced a tick count wrong by more than
    /// an order of magnitude, and the extrapolated "intercept position"
    /// that followed was meaningless.
    fn can_intercept_ball(&self, ctx: &StateProcessingContext) -> bool {
        if self.exists_nearby(ctx) || ctx.tick_context.ball.is_owned {
            return false;
        }
        let meeting = LooseBallChase::meeting_point(
            ctx,
            ctx.tick_context.positions.ball.position,
            ctx.tick_context.positions.ball.velocity,
        );
        (meeting - ctx.player.position).magnitude() <= TackleEngagement::CONTACT
    }
}

#[cfg(test)]
mod tests {
    use crate::PlayerSkills;
    use crate::club::player::builder::PlayerBuilder;
    use crate::r#match::MatchPlayer;
    use crate::r#match::player::strategies::players::ops::skill_composites as sc;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
    };
    use chrono::NaiveDate;

    fn defender(tackling: f32, marking: f32, positioning: f32) -> MatchPlayer {
        let attrs = PlayerAttributes {
            condition: 9000,
            ..Default::default()
        };
        let mut skills = PlayerSkills::default();
        skills.technical.tackling = tackling;
        skills.technical.marking = marking;
        skills.mental.positioning = positioning;
        skills.mental.anticipation = 12.0;
        skills.mental.concentration = 12.0;
        skills.mental.bravery = 12.0;
        skills.physical.strength = 13.0;
        skills.physical.balance = 12.0;
        skills.physical.agility = 12.0;
        skills.physical.stamina = 14.0;
        skills.physical.natural_fitness = 14.0;
        let p = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("D".into(), "Z".into()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::DefenderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(1, &p, PlayerPositionType::DefenderCenter, false, None)
    }

    fn attacker(dribbling: f32, technique: f32, agility: f32) -> MatchPlayer {
        let attrs = PlayerAttributes {
            condition: 9000,
            ..Default::default()
        };
        let mut skills = PlayerSkills::default();
        skills.technical.dribbling = dribbling;
        skills.technical.technique = technique;
        skills.mental.flair = 12.0;
        skills.mental.composure = 12.0;
        skills.mental.decisions = 12.0;
        skills.physical.agility = agility;
        skills.physical.acceleration = 14.0;
        skills.physical.balance = 12.0;
        skills.physical.strength = 11.0;
        skills.physical.stamina = 14.0;
        skills.physical.natural_fitness = 14.0;
        let p = PlayerBuilder::new()
            .id(2)
            .full_name(FullName::new("A".into(), "Z".into()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::ForwardCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(2, &p, PlayerPositionType::ForwardCenter, false, None)
    }

    #[test]
    fn strong_tackler_dominates_weak_dribbler() {
        let strong = defender(18.0, 17.0, 16.0);
        let weak_attacker = attacker(7.0, 7.0, 9.0);
        let diff = sc::defensive_duel(&strong, 30) - sc::dribble_attack(&weak_attacker, 30);
        assert!(
            diff > 0.20,
            "expected strong defender advantage, got diff={diff}"
        );
    }

    #[test]
    fn weak_tackler_loses_to_strong_dribbler() {
        let weak = defender(7.0, 7.0, 8.0);
        let elite_attacker = attacker(18.0, 17.0, 17.0);
        let diff = sc::defensive_duel(&weak, 30) - sc::dribble_attack(&elite_attacker, 30);
        assert!(diff < -0.10, "expected attacker advantage, got diff={diff}");
    }

    /// Every attribute at `level`, so the two sides of the duel are
    /// matched by construction.
    fn uniform(id: u32, level: f32, position: PlayerPositionType) -> MatchPlayer {
        let attrs = PlayerAttributes {
            condition: 10000,
            jadedness: 0,
            ..Default::default()
        };
        let mut skills = PlayerSkills::default();
        let s = &mut skills;
        s.technical.tackling = level;
        s.technical.marking = level;
        s.technical.heading = level;
        s.technical.passing = level;
        s.technical.technique = level;
        s.technical.first_touch = level;
        s.technical.crossing = level;
        s.technical.dribbling = level;
        s.mental.positioning = level;
        s.mental.anticipation = level;
        s.mental.concentration = level;
        s.mental.decisions = level;
        s.mental.composure = level;
        s.mental.bravery = level;
        s.mental.aggression = level;
        s.mental.teamwork = level;
        s.mental.work_rate = level;
        s.mental.leadership = level;
        s.mental.vision = level;
        s.mental.off_the_ball = level;
        s.mental.determination = level;
        s.mental.flair = level;
        s.physical.strength = level;
        s.physical.jumping = level;
        s.physical.pace = level;
        s.physical.acceleration = level;
        s.physical.agility = level;
        s.physical.balance = level;
        s.physical.stamina = level;
        s.physical.natural_fitness = level;
        s.physical.match_readiness = level;
        let p = PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("U".into(), "Z".into()))
            .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(id, &p, position, false, None)
    }

    /// **The production duel is level for equal quality, at every level
    /// of the pyramid.**
    ///
    /// This is the property `tackle_profile` could not have. It subtracts
    /// `MatchStandard::shift` from each of its inputs and `dribble_attack`
    /// does not, so differencing the two left the standard of the match in
    /// the result: the same matched pair resolved at 0.55 in the fourth
    /// tier and 0.24 in the top flight. Both composites here are absolute,
    /// so the difference is zero wherever the pair is drawn from.
    #[test]
    fn an_equal_duel_resolves_level_at_every_level() {
        let win = |level: f32| {
            let d = uniform(1, level, PlayerPositionType::DefenderCenter);
            let a = uniform(2, level, PlayerPositionType::ForwardCenter);
            let diff = sc::defensive_duel(&d, 30) - sc::dribble_attack(&a, 30);
            (1.0f32 / (1.0 + (-diff * 2.4).exp())).clamp(0.06, 0.55)
        };
        let mut lowest = f32::MAX;
        let mut highest = f32::MIN;
        for level in [4.0f32, 8.0, 12.0, 16.0, 18.0, 20.0] {
            let p = win(level);
            assert!(
                (p - 0.5).abs() <= 0.05,
                "equal quality at level {level} resolved at {p}"
            );
            lowest = lowest.min(p);
            highest = highest.max(p);
        }
        assert!(
            highest - lowest <= 0.05,
            "the duel walks up the pyramid: {lowest} to {highest}"
        );
    }

    /// …and skill advantage is still monotone through it.
    #[test]
    fn the_better_defender_still_wins_more_of_them() {
        let elite = uniform(1, 18.0, PlayerPositionType::DefenderCenter);
        let poor = uniform(3, 6.0, PlayerPositionType::DefenderCenter);
        let carrier = uniform(2, 12.0, PlayerPositionType::ForwardCenter);
        let carry = sc::dribble_attack(&carrier, 30);
        assert!(sc::defensive_duel(&elite, 30) - carry > sc::defensive_duel(&poor, 30) - carry);
    }

    #[test]
    fn marking_and_positioning_help_defender_in_duel() {
        let positional = defender(12.0, 18.0, 18.0);
        let raw_tackler = defender(15.0, 8.0, 8.0);
        // The positional defender should match or exceed a stronger
        // pure tackler thanks to marking + positioning weight (0.13 +
        // 0.17). This validates the spec's expectation that the duel
        // composite isn't dominated by raw tackling alone.
        assert!(sc::defensive_duel(&positional, 30) >= sc::defensive_duel(&raw_tackler, 30));
    }
}

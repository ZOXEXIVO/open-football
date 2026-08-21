use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperAerialClaim, KeeperBallClaim,
    KeeperCarrierThreat, KeeperDebug, KeeperFeetDecision, KeeperOneOnOne, KeeperRestPosition,
    KeeperSmother, KeeperSweepLimit,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const DANGER_ZONE_RADIUS: f32 = 35.0;
const CLOSE_DANGER_DISTANCE: f32 = 100.0;
/// How far out a keeper even CONSIDERS coming for a carrier, before his
/// own rushing-out and eccentricity widen it. 200u = 25 m.
const SWEEP_SCAN_BASE: f32 = 200.0;
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
        // The speed bar is what separates a ball he can gather from one
        // he has to save. It was 10.0 u/tick — above the engine's own
        // `MAX_SHOT_VELOCITY` of 3.2, so it excluded nothing and the
        // keeper "picked up" balls travelling faster than any shot in the
        // game. 2.0 u/tick (25 m/s) is a driven ball; past that he is
        // making a save, not collecting.
        if ball_distance < 10.0
            && !ctx.ball().is_owned()
            && !own_dead_delivery
            && KeeperBallClaim::is_favourite(ctx)
            && ctx.ball().on_own_side()
            && ctx.tick_context.positions.ball.velocity.norm() < 2.0
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
            // …and in open play the FIRST question is whether to pick it
            // up. This branch went straight to `Distributing` — a foot
            // pass — for every possession that was not a goal kick, so a
            // keeper who had just smothered at a striker's feet stood on
            // the ball hunting an outlet with the man he had beaten still
            // next to him. See [`KeeperFeetDecision`].
            return Some(StateChangeResult::with_goalkeeper_state(
                KeeperFeetDecision::state_for(ctx),
            ));
        }

        // **A man has arrived with the ball inside his own spread.** Above
        // the aerial claim and the sweep, because there is nothing left to
        // decide: the ball is at his feet and it is his. Wired here as well
        // as in the three states that already had it — measured, at the
        // moment a strict 1-v-1 starts he is in a state with no route to
        // [`KeeperSmother`] at all half the time, and `Standing` is the
        // largest of them. Every gate is inside `assess`.
        if let Some(attempt) = KeeperSmother::assess(ctx) {
            return Some(KeeperSmother::commit(ctx, &attempt));
        }

        let ball_on_own_side = ctx.ball().on_own_side();

        // A ball in the air over my box is MINE. This is the single most
        // visible thing a goalkeeper does and the engine had no mechanism
        // for it — see [`KeeperAerialClaim`]. He goes and gets it: takes
        // off if it is arriving now, runs to the meeting point if not.
        if let Some(claim) = KeeperAerialClaim::assess(ctx) {
            KeeperAerialClaim::note_start(ctx, &claim);
            return Some(StateChangeResult::with_goalkeeper_state(
                if claim.at_contact(ctx.player.position) {
                    if claim.standing {
                        GoalkeeperState::Catching
                    } else {
                        GoalkeeperState::Jumping
                    }
                } else {
                    GoalkeeperState::ComingOut
                },
            ));
        }

        // Skill-based threat assessment.
        //
        // Through the profile, not off the raw attributes. Everything else
        // about a keeper is banded for fatigue and priced against the
        // standard of the match he is playing in (see
        // [`GoalkeeperSkillProfile`]); these reads were not, so a keeper's
        // willingness to leave his line was the one thing about him that
        // stayed sharp at 88 minutes and read the same in the fourth tier
        // as in the first.
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let anticipation = prof.positioning;
        let command_of_area = prof.command_of_area;

        // ── The decision to come out ──────────────────────────────────
        //
        // ⚠ THE OUTER GATE HAS TO BE WIDER THAN THE DECISION IT GUARDS.
        //
        // This asked `opponent_distance < CLOSE_DANGER_DISTANCE` — 12.5 m
        // — before it would even CONSULT `should_rush_out_for_ball`. That
        // function reasons about a carrier bearing down from 19 m and
        // widens further with the keeper's risk appetite, so most of it
        // was unreachable: by the time it was asked, the striker was
        // already in the six-yard box and the only useful answer was
        // "set yourself". `ComingOut` measured **below 0.25% of keeper
        // ticks** — the keeper never swept, and every sweeper attribute
        // he has was decorative.
        //
        // The scan radius is now the keeper's own: how far he is willing
        // to come is a property of `rushing_out` and `eccentricity`, and
        // it is `should_rush_out_for_ball`'s job to weigh it. This gate
        // only decides whether the question is worth asking.
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::KeeperSweepDiag::note(0);
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::KeeperSweepDiag::note(1);
            let opponent_distance = opponent.distance(ctx);
            // 200u = 25 m for an ordinary keeper, out to ~40 m for a
            // rushing, eccentric one — a Neuer, who genuinely defends
            // that space, against a line-keeper who does not.
            let scan = SWEEP_SCAN_BASE
                * (1.0 + prof.rushing_out_profile * 0.5 + prof.eccentricity * 0.5);

            if opponent_distance < scan {
                #[cfg(feature = "match-logs")]
                crate::mid_run_diag::KeeperSweepDiag::note(2);
                if self.should_rush_out_for_ball(ctx, &opponent) {
                    #[cfg(feature = "match-logs")]
                    crate::mid_run_diag::KeeperSweepDiag::note(3);
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::ComingOut,
                    ));
                }
            }
            // Close, and not coming — then set yourself for the strike.
            if opponent_distance < CLOSE_DANGER_DISTANCE {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            }
            // **A man is through on his goal and he is standing still.**
            //
            // `CLOSE_DANGER_DISTANCE` is 12.5 m, and a keeper who waits for
            // a breakaway to get that close has already lost the decision.
            // Measured over a recorded match, at the moment a strict 1-v-1
            // begins he is in `Standing` or `Walking` a third of the time
            // and stays there. `PreparingForSave` is the state that owns the
            // whole question — the closing point, the smother, the dive —
            // so hand it over. See [`KeeperOneOnOne`].
            if KeeperOneOnOne::duel(ctx).is_some() {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            }
        }

        // Check if ball is coming toward goal — react faster to shots
        // Shot velocities are ~1.0-2.0/tick, thresholds must match
        //
        // ⚠ …but only if it is not OURS. This branch fires on any ball
        // moving roughly goalward inside 37.5 m, and a back-pass, a
        // defender's controlled touch and a centre-half turning infield are
        // all exactly that. `PreparingForSave` stands down when the team
        // has settled control, so without the same condition here the two
        // gates disagree by construction and the keeper re-arms on the tick
        // he stood down: measured, 371 of 971 entries to
        // `PreparingForSave` came within 300 ms of him leaving it.
        //
        // A keeper does not set himself for his own left-back's pass. The
        // opponent-carrier branch above still covers everything genuinely
        // dangerous, and a shot in flight is the very first branch in this
        // state and is not conditioned on possession at all.
        if ctx.ball().is_towards_player_with_angle(0.6)
            && ball_on_own_side
            && (KeeperDebug::calm_off() || !ctx.team().is_control_ball())
        {
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
        // …and only for a ball in the ground he defends. Without the
        // second test this is the largest single door into `ComingOut`,
        // and it opens on a ball 5-6 m from HIM wherever he happens to be
        // standing — so a keeper who had pushed up to sweep set off after
        // a ball rolling into the channel and `ComingOut` then owned him
        // for as long as it kept rolling. Measured, his mean distance from
        // goal while in that state was 19.1 m and the furthest was 62.5 m.
        if !ctx.ball().is_owned()
            && !own_dead_delivery
            && ball_on_own_side
            && ball_distance < 40.0 * (1.0 + command_of_area * 0.3)
            && KeeperSweepLimit::covers(ctx, ctx.tick_context.positions.ball.position, &prof)
        {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ComingOut,
            ));
        }

        // Ball on own side but no immediate threat — Standing is the
        // right rest state; Attentive was an identical idle state with
        // slightly different thresholds.

        // A man in the danger zone with the ball and him already set —
        // hand over to the state that tracks the strike.
        if self.is_opponent_in_danger_zone(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::PreparingForSave,
            ));
        }

        // **He has drifted off his spot: go and adjust.**
        //
        // ⚠ THE KICKOFF-SLOT LEASH AGAIN, for the sixth time in this
        // engine, and this instance ran as a two-cycle at tick resolution.
        // `position_to_distance` measures the distance from his KICKOFF DOT
        // — (20, 272.5), two and a half metres in front of the middle of the
        // goal — and calls anything past 100u (12.5 m) `Medium`. The rest
        // model routinely and correctly wants him further out than that, so
        // the test was permanently true; it sent him to `Walking`, and
        // `Walking`'s first question was whether the ball is on our own side,
        // which sent him straight back here.
        //
        // Measured over 200 matches: **1673 state transitions per keeper per
        // match — eighteen a minute — of which 53% reversed inside 300 ms.**
        // The two states have the same rest point and the same steering, so
        // none of it changed where he went; what it did do is reset
        // `in_state_time` on nearly every tick, which is the clock several
        // of his other gates are written against.
        //
        // The honest question is the one `KeeperRestPosition` already
        // answers — IS HE SET — and it is anisotropic, distance-scaled and
        // reads his concentration, none of which a radius around a fixed dot
        // can be. `Walking` is where he goes when he is not.
        //
        // ⚠ …but NOT on the exact complement of `Walking`'s own hand-back,
        // or the pair is a two-cycle in the degenerate form: an entry and a
        // give-up that are the same test. Measured that way it produced
        // 15,804 round trips per keeper per match. He starts walking when he
        // is well off his spot and stops when he has arrived, with a band
        // between the two. See [`KeeperRestPosition::needs_repositioning`].
        if KeeperRestPosition::needs_repositioning(
            ctx.player.position,
            self.calculate_optimal_position(ctx),
            GoalkeeperSkillProfile::from_ctx(ctx).concentration,
            ball_distance,
            ctx.context.field_size.width as f32,
        ) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Walking,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let optimal_position = self.calculate_optimal_position(ctx);

        // A keeper is not always repositioning. Standing still, set, is a
        // real and common thing for him to be doing, and the target moves
        // continuously with the ball — so without a deadzone he chases it
        // every tick and ends up covering more ground than a midfielder.
        //
        // But the tolerance is ANISOTROPIC, and that is the whole point:
        // slack in depth, where a metre is nothing, and tight across the
        // goal, where a metre is a goal. See
        // `KeeperRestPosition::LATERAL_DEADZONE`.
        if KeeperRestPosition::is_set_with(
            ctx.player.position,
            optimal_position,
            GoalkeeperSkillProfile::from_ctx(ctx).concentration,
            ctx.ball().distance(),
            ctx.context.field_size.width as f32,
        ) {
            return Some(Vector3::zeros());
        }

        // URGENCY. How fast he travels is set by how near the ball is, not
        // by how far he has to go — which is the difference between a
        // keeper strolling out to the edge of his box while play is at the
        // other end, and the same keeper sprinting to the same spot with
        // a striker running through. Without this he moves at repositioning
        // pace always, and covers 12.8 km a match against a real ~5 km.
        let ball_distance = ctx.ball().distance();
        let field_width = ctx.context.field_size.width as f32;
        let urgency = (1.0 - ball_distance / (field_width * 0.55)).clamp(0.0, 1.0);
        // Amble ↔ sprint.
        let pace = 0.18 + urgency * urgency * 1.12;

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

        // Goalkeeper skills. `rushing_out` was a local blend of two mental
        // attributes read raw; the profile's own composite is the same idea
        // built from the attribute the game actually calls `rushing_out`,
        // plus acceleration, pace, decisions, anticipation, composure and
        // bravery — banded for fatigue and priced against the standard of
        // the match, neither of which a raw read is.
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let rushing_out = prof.rushing_out_profile;

        // Opponent skills
        let opponent_control = ctx.player().skills(opponent.id).technical.first_touch / 20.0;

        // The foot race. Both sides read `pace`, so the ratio means
        // something: the keeper's term used `acceleration` against the
        // opponent's `pace` — different attributes on the same 1-20 scale
        // — so a keeper with quick feet and no top end was scored as
        // though he could outrun a winger.
        let keeper_speed = ctx.player.skills.physical.pace * (1.0 + rushing_out * 0.3);
        let opponent_speed = ctx.player().skills(opponent.id).physical.pace.max(1.0);

        let keeper_time = keeper_to_ball / keeper_speed.max(1.0);
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
        let risk_appetite = prof.eccentricity.clamp(0.0, 1.0) - 0.5;

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
        // ⚠ …but "a carrier inside 19 m" is NOT the same question as "is
        // he through". A striker 18 m out with two centre-backs in front of
        // him belongs to the defence, and a keeper who charges out at him
        // leaves an empty net to chip. Worse, `Standing` asks this question
        // BEFORE it asks whether to set himself, so a trigger firing at
        // 19 m made `PreparingForSave` unreachable for every carrier inside
        // 12.5 m — the exact situation that state exists for. He committed,
        // hit the excursion limit, turned round, and committed again: with
        // a carrier inside 25 m he spent 18.6% of those ticks running back
        // to his line against 13.5% coming out.
        //
        // The real rule is the simple one — he comes when there is nobody
        // between him and the ball. See [`KeeperCarrierThreat`].
        //
        // 150u ≈ 19 m: the carrier is into the area the keeper is
        // responsible for. Risk appetite widens it, so an eccentric sweeper
        // comes further and a cautious one holds his line longer.
        let carrier_bearing_down = ctx.ball().is_owned()
            && keeper_to_ball < 150.0 * (1.0 + risk_appetite * 0.4)
            && !ctx.ball().is_held_by_opponent_goalkeeper();
        if carrier_bearing_down {
            if KeeperCarrierThreat::is_through(ctx, opponent) {
                #[cfg(feature = "match-logs")]
                crate::mid_run_diag::KeeperSweepDiag::note(4);
                return true;
            }
            // Covered. He does not charge — he sets himself and narrows
            // the angle, which is `PreparingForSave`'s job and the branch
            // immediately below this call in `process`.
            return false;
        }

        let can_reach_first =
            keeper_time < opponent_time * (1.0 + rushing_out * 0.2 + risk_appetite * 0.15);
        let ball_loose_or_poor_control = !ctx.ball().is_owned() || opponent_control < 0.5;
        let reasonable_distance =
            keeper_to_ball < CLOSE_DANGER_DISTANCE * (1.5 + risk_appetite * 0.5);

        // …and it has to be a ball he would still be defending when he got
        // there. Every other door into `ComingOut` now asks this; a race
        // he wins out by the corner flag is a race not worth entering.
        can_reach_first
            && ball_loose_or_poor_control
            && reasonable_distance
            && KeeperSweepLimit::covers(ctx, ball_position, &prof)
    }

    /// Calculate optimal goalkeeper position based on ball and goal
    /// Where the keeper should be standing.
    ///
    /// # Why this was rebuilt
    ///
    /// A keeper in this engine stood on his line and never left it. He
    /// spent **88% of the match motionless** and sat 11.4 m deeper than
    /// even the modest anchor the team shape gave him, which is the "only
    /// the GK stays in the goal" report.
    ///
    /// The old model had two faults, and the first is the interesting one:
    /// **its depth was inverted.** With the ball in the opponent's half it
    /// put him `12 + command * 8` units — **1.5 to 2.5 metres** — off his
    /// line. That is the moment a real keeper is FURTHEST out, 20-30 m up,
    /// sweeping behind his defence. With the ball near him it brought him
    /// *further* out. Across every input the whole range came to about
    /// 1-4 m, so nothing he did was visible.
    ///
    /// Second, every result was clamped inside the penalty area, so
    /// sweeping up to meet a ball played in behind was impossible by
    /// construction.
    ///
    /// # The model
    ///
    /// A keeper's depth is set by **where the ball is** and **how high his
    /// own defence is**, and his lateral position by **the angle** — he
    /// stands on the line from the centre of his goal to the ball, which
    /// is what "narrowing the angle" physically means.
    ///
    /// Real reference points, which the constants below reproduce:
    ///   * ball in the opponent's half behind a high line — 20-28 m out,
    ///     effectively a sweeper;
    ///   * ball in midfield — 12-18 m;
    ///   * ball entering our final third — 6-10 m;
    ///   * ball in the box — 2-5 m, on the angle, ready to spread.
    fn calculate_optimal_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        self.clamp_sweep_range(ctx, KeeperRestPosition::for_keeper(ctx))
    }

    /// Bound the keeper's standing position.
    ///
    /// Replaces a hard clamp into the penalty area. That clamp made
    /// sweeping impossible by construction — a keeper cannot come and
    /// meet a ball played in behind if he is not allowed out of his box —
    /// and it is not what the laws or the football say: a keeper may go
    /// anywhere, he just cannot handle the ball outside the area, which
    /// the handling code enforces separately.
    ///
    /// What bounds him is [`KeeperSweepLimit`], and
    /// `KeeperRestPosition::for_keeper` has already applied it — to the
    /// same territory the sweep gates use, and for `Walking` and
    /// `ReturningToGoal` as well as for this state. What is left here is
    /// the backstop that is a property of the PITCH rather than of the
    /// keeper: he is never behind his own goal line and never past
    /// halfway.
    ///
    /// ⚠ The lateral half of this clamp — into the width of the penalty
    /// area — used to be the ONLY thing in the engine that bounded a
    /// keeper across the pitch, and it bounded a target rather than the
    /// keeper: the moment he left this state for `ComingOut`, `TakeBall`
    /// or `Catching` there was nothing at all. See the note on
    /// `KeeperSweepLimit` for what that measured.
    fn clamp_sweep_range(
        &self,
        ctx: &StateProcessingContext,
        position: Vector3<f32>,
    ) -> Vector3<f32> {
        let goal_center = ctx.ball().direction_to_own_goal();
        let side = ctx.player.side.unwrap_or(PlayerSide::Left);
        let field_width = ctx.context.field_size.width as f32;

        let far_x = goal_center.x + side.forward_dir_x() * (field_width * 0.5 - 20.0);
        let (lo_x, hi_x) = if side.forward_dir_x() > 0.0 {
            (goal_center.x, far_x)
        } else {
            (far_x, goal_center.x)
        };

        Vector3::new(position.x.clamp(lo_x, hi_x), position.y, 0.0)
    }
}

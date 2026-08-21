use crate::PlayerFieldPositionGroup;
use crate::r#match::PlayerSide;
use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperFeetDecision,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperDistributingState {}

impl StateProcessingHandler for GoalkeeperDistributingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If we no longer have the ball, we must have passed or lost it
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // **A man has arrived while he was looking for a pass.**
        //
        // This state stands perfectly still (`velocity` is a hard zero)
        // for up to twenty ticks hunting an outlet at least 30u away that
        // is not behind him and not in a blocked lane. With the ball on
        // the floor that is the single most dangerous place it can be:
        // `check_ball_ownership` gives a contested ball to the better
        // tackler within 5u, and that is never the goalkeeper. Under real
        // pressure the search is over — pick it up if the Laws allow, get
        // rid if they do not. Only asked for a ball still on the floor;
        // once it is in his gloves nobody can reach it and he has all the
        // time `HoldingBall` gives him.
        if !ctx.tick_context.ball.held_in_hands
            && KeeperFeetDecision::pressure(ctx) > 0.5
            && !ctx.ball().is_goal_kick_restart()
        {
            let escape = KeeperFeetDecision::state_for(ctx);
            if escape != GoalkeeperState::Distributing {
                return Some(StateChangeResult::with_goalkeeper_state(escape));
            }
        }

        // Try to find the best pass option
        if let Some(teammate) = self.find_best_pass_option(ctx) {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::KeeperReleaseDiag::note_short(
                (teammate.position - ctx.player.position).norm(),
            );
            // Execute the pass and transition to returning to goal
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::ReturningToGoal,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::new()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(teammate.id)
                        .with_reason("GK_DISTRIBUTING")
                        .build(ctx),
                )),
            ));
        }

        // Timeout after a short time if no pass is made
        // Clear the ball rather than running with it (GK should never wander with ball)
        if ctx.in_state_time > 20 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Clearing,
            ));
        }

        // If we have the ball but no good passing option yet, wait
        // The goalkeeper should not be trying to catch the ball since they already have it
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Distributing requires moderate intensity with focused effort
        GoalkeeperCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl GoalkeeperDistributingState {
    /// Pick a teammate to kick the ball to, strongly preferring the
    /// *centre of the field* — a midfielder near the halfway line — so
    /// the GK clears the defensive third rather than passing short to a
    /// defender who immediately gets pressed and loses possession back
    /// to the attacking team.
    ///
    /// The earlier "safe short pass to the nearest defender" version
    /// produced a death loop: save → pass to full-back → opponent
    /// closes him down → turnover 20m from goal → shot → save → repeat.
    /// Real GKs under any pressure clear long, landing the ball at the
    /// halfway line where their own midfielders can contest it. That's
    /// the target zone this scoring favours.
    ///
    /// Unit scale: 1u = 0.125m, field 840u = 105m. The halfway line
    /// sits at x = field_width/2. A GK at x ≈ 20u kicks towards x ≈ 420u
    /// (~50m), matching a real goal kick.
    fn find_best_pass_option<'a>(
        &'a self,
        ctx: &'a StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        const MIN_RECEIVE_DISTANCE: f32 = 30.0; // no back-passes / tap-outs
        const BLOCK_CORRIDOR: f32 = 8.0; // lane width for interception check

        // Maximum search range scales with distribution skill: weak
        // keepers can't reliably hit a 70m punt, so they shouldn't even
        // look that far. Range 380..620u (~47..78m).
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let max_search = 380.0 + prof.distribution * 240.0;

        let field_width = ctx.context.field_size.width as f32;
        let halfway_x = field_width * 0.5;

        let mut best_option: Option<MatchPlayerLite> = None;
        let mut best_score = 0.0;
        // Every legal option, kept so a keeper who loses his head can
        // actually pick a bad one. See `rushed_choice` below.
        let mut options: Vec<MatchPlayerLite> = Vec::new();

        for teammate in ctx.players().teammates().nearby(max_search) {
            if teammate.tactical_positions.position_group() == PlayerFieldPositionGroup::Goalkeeper
            {
                continue;
            }
            let distance = (teammate.position - ctx.player.position).norm();
            if distance < MIN_RECEIVE_DISTANCE {
                continue;
            }

            // Ignore anyone behind or level with the GK — never kick
            // back toward own goal.
            let forward_progress = teammate.position.x - ctx.player.position.x;
            let is_forward = match ctx.player.side {
                Some(PlayerSide::Left) => forward_progress > 0.0,
                Some(PlayerSide::Right) => forward_progress < 0.0,
                None => true,
            };
            if !is_forward {
                continue;
            }

            // Pass lane blocked by an opponent — skip.
            let pass_dir = (teammate.position - ctx.player.position).normalize();
            let blocked = ctx.players().opponents().all().any(|opp| {
                let to_opp = opp.position - ctx.player.position;
                let proj = to_opp.dot(&pass_dir);
                if proj < 10.0 || proj > distance {
                    return false;
                }
                let proj_pt = ctx.player.position + pass_dir * proj;
                (opp.position - proj_pt).norm() < BLOCK_CORRIDOR
            });
            if blocked {
                continue;
            }

            let recency_penalty = ctx.ball().passer_recency_penalty(teammate.id);

            // PRIMARY SIGNAL: how close to the halfway line is this
            // teammate? Peaks at x = halfway, falls off in both
            // directions. A receiver at the halfway line scores 3.0;
            // deep in own half or deep in opponent half scores ~0.5.
            // This is what drives "kick to the middle of the field".
            let dist_from_halfway = (teammate.position.x - halfway_x).abs();
            let halfway_score = 3.0 - (dist_from_halfway / halfway_x) * 2.5;
            let halfway_score = halfway_score.max(0.5);

            // Receiver-is-open is the other safety signal — a teammate
            // at the halfway line surrounded by three opponents is still
            // a bad target.
            let nearby_opponents = ctx.tick_context.grid.opponents(teammate.id, 15.0).count();
            let space_bonus = match nearby_opponents {
                0 => 2.0,
                1 => 1.2,
                _ => 0.5,
            };

            // Role bias — kicks to midfielders are the canonical goal
            // kick; forwards OK too (long ball to striker is classic);
            // defenders get a heavy penalty because this is exactly the
            // short-pass-to-full-back trap that spawns the loss-loop.
            let position_bonus = match teammate.tactical_positions.position_group() {
                PlayerFieldPositionGroup::Midfielder => 1.6,
                PlayerFieldPositionGroup::Forward => 1.2,
                PlayerFieldPositionGroup::Defender => 0.4,
                PlayerFieldPositionGroup::Goalkeeper => 0.0,
            };

            // Turnover risk: long, traffic-heavy passes by weak keepers
            // are punished. `pass_difficulty` proxied by distance and
            // pressure on the receiver.
            let pass_difficulty = (distance / max_search).clamp(0.0, 1.0);
            let pressure = (nearby_opponents as f32 / 4.0).clamp(0.0, 1.0);
            let turnover = prof.turnover_risk(pass_difficulty, pressure, 0.0);
            let safety_factor = (1.0 - turnover * 0.6).max(0.2);
            let score =
                halfway_score * space_bonus * position_bonus * recency_penalty * safety_factor;

            if score > best_score {
                best_score = score;
                best_option = Some(teammate);
            }
            options.push(teammate);
        }

        // Does the keeper actually find his best option? `turnover_risk`
        // already models how likely this keeper is to lose the ball
        // playing out — distribution skill, condition, pressure — but it
        // was only ever used to SCORE targets, so it made a weak keeper
        // choose more conservatively and never made him get it wrong. A
        // model of risk with no execution consequence is why senior and
        // youth keepers posted identical error rates (0.164 vs 0.161
        // errors-to-shot per match) despite an eight-point skill gap.
        //
        // Under pressure, a keeper with poor distribution / composure /
        // decisions rushes it: he hits the first man he sees rather than
        // the right one. The consequence is emergent — a pass to a
        // covered teammate in his own third is far likelier to be
        // intercepted, and the existing giveaway machinery then stamps
        // the dangerous turnover and (if a shot follows) the error.
        // Nothing about the punishment is hardcoded here.
        if let Some(best) = best_option {
            if options.len() > 1 {
                let pressure = Self::pressure_on_keeper(ctx);
                let rushed = prof.turnover_risk(0.45, pressure, 0.0);
                if ctx.context.rng.unit_f32() < rushed {
                    let idx = ctx.context.rng.range_u64(0, options.len() as u64) as usize;
                    return options.get(idx).copied().or(Some(best));
                }
            }
            return Some(best);
        }
        best_option
    }

    /// How closed-down the keeper is right now, 0..1. Saturating so a
    /// swarm doesn't read as infinitely worse than two players.
    fn pressure_on_keeper(ctx: &StateProcessingContext) -> f32 {
        const PRESS_RADIUS: f32 = 90.0;
        let close = ctx.players().opponents().nearby(PRESS_RADIUS).count() as f32;
        1.0 - (-close / 1.8).exp()
    }

    pub fn calculate_pass_power(&self, teammate_id: u32, ctx: &StateProcessingContext) -> f64 {
        let distance = ctx.tick_context.grid.get(ctx.player.id, teammate_id);

        let pass_skill = ctx.player.skills.technical.passing;

        (distance / pass_skill * 10.0) as f64
    }
}

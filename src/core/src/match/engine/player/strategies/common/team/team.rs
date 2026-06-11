use crate::r#match::{
    CoachInstruction, GamePhase, MatchCoach, PlayerSide, StateProcessingContext, TeamTacticalState,
};
use crate::{PlayerFieldPositionGroup, Tactics};
use nalgebra::Vector3;
use std::cmp::Ordering;

pub struct TeamOperationsImpl<'b> {
    ctx: &'b StateProcessingContext<'b>,
}

impl<'b> TeamOperationsImpl<'b> {
    pub fn new(ctx: &'b StateProcessingContext<'b>) -> Self {
        TeamOperationsImpl { ctx }
    }
}

impl<'b> TeamOperationsImpl<'b> {
    pub fn tactics(&self) -> &Tactics {
        // A sent-off / mid-swap player can transiently have no side
        // before they're removed from the field. Fall back to the left
        // tactics rather than crashing the live engine — the caller
        // gets stable reads and the player is filtered out one tick
        // later when ownership cleanup runs.
        match self.ctx.player.side {
            Some(PlayerSide::Left) | None => &self.ctx.context.tactics.left,
            Some(PlayerSide::Right) => &self.ctx.context.tactics.right,
        }
    }

    /// Get the coach's current instruction for this player's team
    pub fn coach_instruction(&self) -> CoachInstruction {
        self.coach().instruction
    }

    /// Get the coach state for this player's team
    pub fn coach(&self) -> &MatchCoach {
        self.ctx.context.coach_for_team(self.ctx.player.team_id)
    }

    /// Team-level tactical state for this player's side (phase,
    /// possession timers, defensive line height). All eleven players on
    /// a team read the same value — that's the point, it's the shared
    /// context that pulls their individual decisions into a coherent
    /// pattern. Recomputed every 10 sim ticks.
    pub fn tactical(&self) -> &TeamTacticalState {
        self.ctx.context.tactical_for_team(self.ctx.player.team_id)
    }

    /// Shortcut — most branching just needs the phase.
    pub fn phase(&self) -> GamePhase {
        self.tactical().phase
    }

    /// Is the attack ready to progress? True when at least one of our
    /// forwards / attacking midfielders is positioned in the final
    /// third (≤80 u from goal). Marking is NOT a disqualifier — real
    /// defences always mark forwards in the box, so requiring a
    /// completely-unmarked forward made `is_attack_ready` essentially
    /// always false against organised defences and forced the team
    /// into permanent possession-recycle mode.
    ///
    /// The signal is intentionally generous: the *forward's job* is
    /// to receive the ball under marking and create the chance; the
    /// recycle gate's job is to avoid blind hopeful balls when no one
    /// is anywhere near goal at all. Distance alone covers that.
    pub fn is_attack_ready(&self) -> bool {
        let ctx = self.ctx;
        let goal_pos = ctx.player().opponent_goal_position();
        ctx.players()
            .teammates()
            .nearby_at(goal_pos, 80.0)
            .any(|t| t.tactical_positions.is_forward() || t.tactical_positions.is_midfielder())
    }

    /// Should the team play in "possession mode" right now — i.e. slow
    /// down, recycle the ball, reject risky forward passes? Real football
    /// triggers:
    ///   1. **Just won the ball.** Stabilize for a few seconds before
    ///      looking to attack — avoids the rushed long-ball turnover
    ///      that re-loses possession within a breath of winning it.
    ///   2. **Leading in the match.** Already captured by
    ///      `prefer_possession()` on the coach (WasteTime / ParkTheBus
    ///      / SlowDown); we union it here so one helper covers all
    ///      realistic triggers.
    ///   3. **Team is tired.** A fatigued squad keeps the ball to rest.
    ///   4. **Attack not ready.** No forward in a shooting threat — no
    ///      point pushing forward into nothing.
    ///   5. **Late in a competitive match.** Final 10 minutes, any
    ///      team ahead or drawing plays safer than earlier.
    pub fn should_play_possession(&self) -> bool {
        let ctx = self.ctx;
        // A LIVE COUNTER OVERRIDES EVERY SLOW-DOWN TRIGGER. The first
        // seconds after winning the ball against an overcommitted
        // opponent are real football's single best attacking moment —
        // the entire point of absorbing pressure in a low block is the
        // break that follows. The previous shape of this helper had
        // transitions BACKWARDS: trigger (1) forced possession-recycle
        // mode for 3 s after every ball-win (precisely the counter
        // window), and `prefer_possession()` extended that to the whole
        // match for game-managing leaders — so a team protecting a lead
        // could never score the classic 2-0 counter goal, and trailing
        // teams pushed without risk. That one-way late-game traffic was
        // a measured +18pp draw-correlation surplus at equal strength.
        if self.counter_window() {
            return false;
        }
        let coach = self.coach_instruction();
        if coach.prefer_possession() {
            return true;
        }

        // (1) Just won the ball — stabilize window. The coach tracks
        // `last_possession_gain_tick`; for the first ~300 ticks (3 s)
        // after winning, keep possession rather than counter-rushing.
        // (Bypassed above when the opponent is overcommitted — then
        // these seconds are for breaking, not stabilizing.)
        let current_tick = ctx.context.current_tick();
        let ticks_since_gain = current_tick.saturating_sub(self.coach().last_possession_gain_tick);
        if ticks_since_gain < 300 && ctx.team().is_control_ball() {
            return true;
        }

        // (3) Team tired — use average condition of outfielders within
        // 250u of the ball (cheap proxy for "our involved players").
        let mut total = 0u32;
        let mut count = 0u32;
        for t in ctx.players().teammates().nearby(250.0) {
            if let Some(p) = ctx.context.players.by_id(t.id) {
                total += p.player_attributes.condition_percentage();
                count += 1;
            }
        }
        if count > 0 && (total / count) < 50 {
            return true;
        }

        // (5) Late game + drawing or leading — even Normal-instruction
        // teams slow down in the last 10 min when they don't need goals.
        let half_ms = crate::r#match::engine::engine::MATCH_HALF_TIME_MS as f32;
        let full_ms = half_ms * 2.0;
        let match_progress = (ctx.context.total_match_time as f32 / full_ms).clamp(0.0, 1.0);
        if match_progress > 0.88 && self.score_diff() >= 0 {
            return true;
        }

        // (4) Attack not set up downfield.
        !self.is_attack_ready()
    }

    /// True when a genuine counter-attack window is open: we won the
    /// ball within the last ~4 s AND the opponent is overcommitted
    /// upfield (5+ of their players in OUR half — the same commitment
    /// signal the defender counter-outlet pass uses). While open, the
    /// possession-stabilize window, the coach's prefer-possession mode
    /// and the minimum-hold gates all stand down: real teams break at
    /// full speed in this window regardless of instruction, tiredness
    /// or scoreline — it is where 2-0 counter goals and tired late
    /// winners come from.
    pub fn counter_window(&self) -> bool {
        let ctx = self.ctx;
        let current_tick = ctx.context.current_tick();
        let ticks_since_gain =
            current_tick.saturating_sub(self.coach().last_possession_gain_tick);
        // 400 → 600 ticks (6 s): a real break from deep needs 6-10 s
        // to reach the opposite box; the shorter window expired while
        // the outlet ball was still in flight.
        if ticks_since_gain >= 600 {
            return false;
        }
        if !ctx.team().is_control_ball() {
            return false;
        }
        let half_x = ctx.context.field_size.width as f32 * 0.5;
        let committed = ctx
            .players()
            .opponents()
            .all()
            .filter(|o| match ctx.player.side {
                Some(PlayerSide::Left) => o.position.x < half_x,
                Some(PlayerSide::Right) => o.position.x > half_x,
                None => false,
            })
            .count();
        // ≥4 committed opponents (was 5): with keeper excluded from
        // upfield positions a 4-man commitment already leaves the
        // back line outnumbered against a 3-man break.
        committed >= 4
    }

    /// Game-management intensity from this team's perspective. Rises
    /// when we are leading, late in the game, and/or the weaker side —
    /// drives safe-pass / hold-the-ball / don't-shoot behaviour.
    pub fn game_management_intensity(&self) -> f32 {
        self.tactical().game_management_intensity
    }

    /// Risk appetite — willingness to choose the forward / progressive
    /// option over the safe one. High when chasing late; low when
    /// leading or game-managing. Drives pass-evaluator forward bias and
    /// the forward-shooting willingness floor.
    pub fn risk_appetite(&self) -> f32 {
        self.tactical().risk_appetite
    }

    /// Tempo — speed-of-play target. High in transitions, low in
    /// settled possession or game management.
    pub fn tempo(&self) -> f32 {
        self.tactical().tempo
    }

    /// Build-up patience — how willing the team is to recycle when
    /// progress is hard. High in possession styles + leads.
    pub fn build_up_patience(&self) -> f32 {
        self.tactical().build_up_patience
    }

    /// Press intensity — how aggressively the team hunts the ball when
    /// out of possession. Used by defenders / midfielders to decide
    /// step-up vs drop-off.
    pub fn press_intensity(&self) -> f32 {
        self.tactical().press_intensity
    }

    /// Compactness target — how tight the shape should be vertically
    /// and horizontally.
    pub fn compactness_target(&self) -> f32 {
        self.tactical().compactness_target
    }

    /// Width target — how spread out laterally we want to be.
    pub fn team_width_target(&self) -> f32 {
        self.tactical().team_width_target
    }

    /// Rest-defence count — how many defenders to keep behind the ball
    /// during sustained attack.
    pub fn rest_defense_count(&self) -> u8 {
        self.tactical().rest_defense_count
    }

    /// In the counter-press window: just lost the ball, the nearest 2-3
    /// players should engage instead of falling back.
    pub fn counterpress_window(&self) -> bool {
        self.tactical().counterpress_window
    }

    /// Whether the team's coach allows shooting right now (team-level cooldown).
    ///
    /// A live rebound (dangerous parry / loose block deflection within
    /// the last ~3 s) suspends the team shot-SPACING and build-up
    /// gates: real box scrambles produce shot–parry–tap-in sequences
    /// inside 2-3 seconds, which the 7.5 s spacing gate made
    /// structurally impossible (contradicting the per-possession cap's
    /// own design note that exists to allow exactly that pattern). The
    /// 2-shots-per-possession cap still applies during rebounds.
    pub fn can_shoot(&self) -> bool {
        let current_tick = self.ctx.context.current_tick();
        const REBOUND_WINDOW_TICKS: u64 = 300;
        let rebound_tick = self.ctx.tick_context.ball.last_rebound_tick;
        let rebound_live = rebound_tick > 0
            && current_tick.saturating_sub(rebound_tick) < REBOUND_WINDOW_TICKS;
        self.coach().can_shoot(current_tick, rebound_live)
    }

    /// Score difference from this team's perspective (positive =
    /// leading) — as BEHAVIOR is allowed to see it: level before
    /// score-reactive football engages (final ~28 min, see
    /// `MatchContext::SCORE_REACTION_FROM_MINUTE`), real after.
    pub fn score_diff(&self) -> i8 {
        if !self.ctx.context.behavioral_score_visible() {
            return 0;
        }
        let home_goals = self.ctx.context.score.home_team.get() as i8;
        let away_goals = self.ctx.context.score.away_team.get() as i8;
        if self.ctx.player.team_id == self.ctx.context.field_home_team_id {
            home_goals - away_goals
        } else {
            away_goals - home_goals
        }
    }

    pub fn is_control_ball(&self) -> bool {
        let current_player_team_id = self.ctx.player.team_id;

        // First check: if a player from player's team has the ball
        if let Some(owner_id) = self.ctx.ball().owner_id() {
            if let Some(ball_owner) = self.ctx.context.players.by_id(owner_id) {
                return ball_owner.team_id == current_player_team_id;
            }
        }

        // Second check: if previous owner was from player's team
        if let Some(prev_owner_id) = self.ctx.ball().previous_owner_id() {
            if let Some(prev_ball_owner) = self.ctx.context.players.by_id(prev_owner_id) {
                if prev_ball_owner.team_id == current_player_team_id {
                    // Check if the ball is still heading in a favorable direction for the team
                    // or if a teammate is clearly going to get it
                    let ball_velocity = self.ctx.tick_context.positions.ball.velocity;

                    // If ball has significant velocity and is heading toward opponent's goal
                    if ball_velocity.norm_squared() > 1.0 {
                        // Determine which way is "forward" based on team side
                        let forward_direction = match self.ctx.player.side {
                            Some(PlayerSide::Left) => Vector3::new(1.0, 0.0, 0.0), // Left team attacks right
                            Some(PlayerSide::Right) => Vector3::new(-1.0, 0.0, 0.0), // Right team attacks left
                            None => Vector3::new(0.0, 0.0, 0.0),
                        };

                        // If ball is moving forward or toward a teammate
                        let dot_product = ball_velocity.normalize().dot(&forward_direction);
                        if dot_product > 0.1 {
                            return true;
                        }

                        // If a teammate is clearly going for the ball and is close
                        if self.is_teammate_chasing_ball() {
                            return true;
                        }
                    }
                }
            }
        }

        // If we get here, we need to check if any player from our team
        // is closer to the ball than any opponent
        let ball_pos = self.ctx.tick_context.positions.ball.position;

        let closest_teammate_dist_sq = self
            .ctx
            .players()
            .teammates()
            .all()
            .map(|p| (p.position - ball_pos).norm_squared())
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

        let closest_opponent_dist_sq = self
            .ctx
            .players()
            .opponents()
            .all()
            .map(|p| (p.position - ball_pos).norm_squared())
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

        // If a teammate is significantly closer to the ball than any opponent
        // 0.7 distance ratio = 0.49 squared ratio
        if let (Some(team_sq), Some(opp_sq)) = (closest_teammate_dist_sq, closest_opponent_dist_sq)
        {
            if team_sq < opp_sq * 0.49 {
                return true;
            }
        }

        false
    }

    /// Check if team has JUST lost possession (previous owner was teammate, ball now with opponent).
    /// Used for counter-pressing triggers — defenders immediately press instead of retreating.
    pub fn has_just_lost_possession(&self) -> bool {
        // Already have ball — haven't lost it
        if self.is_control_ball() {
            return false;
        }

        // Check if previous owner was from our team
        if let Some(prev_id) = self.ctx.tick_context.ball.last_owner {
            if let Some(prev_player) = self.ctx.context.players.by_id(prev_id) {
                if prev_player.team_id == self.ctx.player.team_id {
                    // Previous owner was us, now we don't control ball
                    // Only counts as "just lost" if within ~300ms
                    let new_ownership = self.ctx.tick_context.ball.ownership_duration;
                    return new_ownership < 30;
                }
            }
        }

        false
    }

    pub fn is_leading(&self) -> bool {
        !self.is_loosing()
    }

    /// Behavioral "are we behind?" — like `score_diff`, players act on
    /// the scoreline only once score-reactive football engages (final
    /// ~28 min). Before that a trailing side keeps playing its game —
    /// the from-minute-1 reaction was part of the measured equalizer
    /// machine (trailing teams scored at 2.35/90 vs leaders' 1.08).
    pub fn is_loosing(&self) -> bool {
        if !self.ctx.context.behavioral_score_visible() {
            return false;
        }
        if self.ctx.player.team_id == self.ctx.context.score.home_team.team_id {
            self.ctx.context.score.home_team < self.ctx.context.score.away_team
        } else {
            self.ctx.context.score.away_team < self.ctx.context.score.home_team
        }
    }

    pub fn is_teammate_chasing_ball(&self) -> bool {
        let ball_position = self.ctx.tick_context.positions.ball.position;

        self.ctx.players().teammates().all().any(|player| {
            // Check if player is heading toward the ball
            let player_position = self.ctx.tick_context.positions.players.position(player.id);
            let player_velocity = self.ctx.tick_context.positions.players.velocity(player.id);

            if player_velocity.norm_squared() < 0.01 {
                return false;
            }

            let direction_to_ball = (ball_position - player_position).normalize();
            let player_direction = player_velocity.normalize();
            let dot_product = direction_to_ball.dot(&player_direction);

            // Player is moving toward the ball (use norm_squared for distance comparison)
            dot_product > 0.85
                && (ball_position - player_position).norm_squared()
                    < (ball_position - self.ctx.player.position).norm_squared() * 1.44 // 1.2^2
        })
    }

    // Determine if this player is the best positioned to chase the ball
    pub fn is_best_player_to_chase_ball(&self) -> bool {
        let ball_position = self.ctx.tick_context.positions.ball.position;

        // Don't chase the ball if a teammate already has it
        if let Some(owner_id) = self.ctx.ball().owner_id() {
            if let Some(owner) = self.ctx.context.players.by_id(owner_id) {
                if owner.team_id == self.ctx.player.team_id {
                    return false;
                }
            }
        }

        // Score for current player (use norm_squared to avoid sqrt)
        let player_dist_sq = (ball_position - self.ctx.player.position).norm_squared();
        let player_score = {
            let skills = &self.ctx.player.skills;
            let pace_factor = skills.physical.pace / 20.0;
            let acceleration_factor = skills.physical.acceleration / 20.0;
            let position_factor = match self
                .ctx
                .player
                .tactical_position
                .current_position
                .position_group()
            {
                PlayerFieldPositionGroup::Forward => 1.2,
                PlayerFieldPositionGroup::Midfielder => 1.1,
                PlayerFieldPositionGroup::Defender => 0.9,
                PlayerFieldPositionGroup::Goalkeeper => 0.5,
            };
            let ability = pace_factor * acceleration_factor * position_factor * 0.5 + 0.5;
            player_dist_sq / (ability * ability)
        };

        let threshold = player_score * 0.64; // 0.8^2

        // Compare against teammates
        !self.ctx.players().teammates().all().any(|teammate| {
            let dist_sq = (ball_position - teammate.position).norm_squared();
            // Quick distance check
            if dist_sq > player_dist_sq {
                return false;
            }

            let skills = match self.ctx.context.players.by_id(teammate.id) {
                Some(p) => &p.skills,
                None => return false,
            };

            let pace_factor = skills.physical.pace / 20.0;
            let acceleration_factor = skills.physical.acceleration / 20.0;
            let position_factor = match teammate.tactical_positions.position_group() {
                PlayerFieldPositionGroup::Forward => 1.2,
                PlayerFieldPositionGroup::Midfielder => 1.1,
                PlayerFieldPositionGroup::Defender => 0.9,
                PlayerFieldPositionGroup::Goalkeeper => 0.5,
            };

            let ability = pace_factor * acceleration_factor * position_factor * 0.5 + 0.5;
            let score = dist_sq / (ability * ability);
            score < threshold
        })
    }
}

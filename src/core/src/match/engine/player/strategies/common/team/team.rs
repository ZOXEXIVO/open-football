use crate::r#match::player::strategies::common::players::ops::box_movement::BoxMovement;
use crate::r#match::{
    AttackPlan, BoxSlot, CoachInstruction, DefensiveDuty, DefensivePlan, Flank, GamePhase,
    MatchCoach, MatchPlayerLite, PlayerSide, StateProcessingContext, TeamShape, TeamTacticalState,
    WidePlan,
};
use crate::{PlayerFieldPositionGroup, Tactics};
use nalgebra::Vector3;
// Only the debug-assert reference recomputation still needs `Ordering`;
// in release the cfg-gated block compiles out along with this import.
#[cfg(debug_assertions)]
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
    /// How far goal-side of the carrier a covering defender sits (~2.5 m).
    const COVER_DEPTH: f32 = 20.0;
    /// How far goal-side of his man a marker sits (~1.5 m) — touch-tight
    /// enough to contest, not so tight he is turned every time.
    const MARK_SHOULDER: f32 = 12.0;
    /// How far up the pitch, and how far outside the width holder, the
    /// overlapping runner aims (~7.5 m ahead, ~2.5 m wider). Beyond the
    /// man he is overlapping and outside him: both halves are what make
    /// the pass simple and the full-back's decision hard.
    const OVERLAP_LEAD: f32 = 60.0;
    const OVERLAP_OUTSIDE: f32 = 20.0;

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

    /// This team's attacking role assignment for the current possession:
    /// who the attack is for, who occupies which patch of the box, who
    /// holds. The counterpart to [`tactical`](Self::tactical) — that says
    /// how we are playing, this says who is doing what. All eleven read
    /// the same plan, which is the point: the destinations are exclusive,
    /// so two players cannot be sent to the same patch of grass.
    pub fn attack_plan(&self) -> &AttackPlan {
        self.ctx
            .context
            .attack_plan_for_team(self.ctx.player.team_id)
    }

    /// The box slot this player has been given, if any.
    pub fn my_box_slot(&self) -> Option<BoxSlot> {
        self.attack_plan().slot_of(self.ctx.player.id)
    }

    /// This team's defensive duty assignment — who presses, who covers,
    /// and which opponent each remaining defender has. The counterpart to
    /// [`attack_plan`](Self::attack_plan): with the ball it says who the
    /// move is for, without it says who each man is responsible for.
    pub fn defensive_plan(&self) -> &DefensivePlan {
        self.ctx
            .context
            .defence_plan_for_team(self.ctx.player.team_id)
    }

    /// What THIS player is responsible for defensively. `HoldZone` when
    /// nothing has been assigned, which is the pre-plan behaviour.
    pub fn my_duty(&self) -> DefensiveDuty {
        self.defensive_plan().duty_of(self.ctx.player.id)
    }

    /// The opponent this player is man-marking, resolved to a live
    /// position. `None` when he holds a zone, presses, or covers — or
    /// when his man has left the pitch.
    pub fn my_mark(&self) -> Option<MatchPlayerLite> {
        let target = self.my_duty().target()?;
        self.ctx
            .players()
            .opponents()
            .all()
            .find(|opp| opp.id == target)
    }

    /// Where this player should be standing to do his duty: goal-side of
    /// his man, goal-side of the carrier if he is covering, on the
    /// carrier if he is pressing. `None` for a zone-holder, whose
    /// position is his state's business.
    ///
    /// This is the single geometry that replaces the shared `ball_x ± 24`
    /// every defender used to steer at — see `teamplay::defence`.
    pub fn my_duty_anchor(&self) -> Option<Vector3<f32>> {
        let own_goal = self.ctx.ball().direction_to_own_goal();
        // The point of attack, in the same terms `DutyAssigner` nominated
        // over: the man on the ball, or — while the ball is travelling
        // and nobody has it — the ball. Without the second half a presser
        // nominated onto a pass in flight had NO anchor at all (`?` on an
        // empty `with_ball()`), and that is the half of the press that
        // runs between one man releasing the ball and the next taking it.
        let point_of_attack = || {
            self.ctx
                .players()
                .opponents()
                .with_ball()
                .next()
                .map(|c| c.position)
                .or_else(|| {
                    (!self.ctx.ball().is_owned())
                        .then(|| self.ctx.tick_context.positions.ball.position)
                })
        };
        match self.my_duty() {
            DefensiveDuty::HoldZone => None,
            DefensiveDuty::Press => point_of_attack(),
            DefensiveDuty::Cover => {
                let point = point_of_attack()?;
                // A stride and a half behind the presser's man, on the
                // line he would run if he got past him.
                let to_goal = (own_goal - point).try_normalize(0.01)?;
                Some(point + to_goal * Self::COVER_DEPTH)
            }
            DefensiveDuty::Mark(_) => {
                let man = self.my_mark()?;
                // Goal-side shoulder, leaning toward the ball — the same
                // blend `DefenderMarkingState` steers onto, so the anchor
                // and the state agree about where marking happens.
                let to_goal = (own_goal - man.position).try_normalize(0.01)?;
                Some(man.position + to_goal * Self::MARK_SHOULDER)
            }
        }
    }

    /// **Where his assigned box slot wants him right now.** `None` when
    /// he has no slot in the current attack.
    ///
    /// Not the slot's coordinate: [`BoxMovement::hold`] stages him from
    /// his place in the block, through the point he waits at, to the
    /// patch itself, as the ball comes. A slot is handed out from
    /// `Progression` onward so the run can START early — returning the
    /// finishing point from the moment it is handed out is what made it
    /// FINISH early instead, and put a forward inside 16.5 m of goal for
    /// 39% of a match. See `BoxMovement`.
    pub fn my_box_slot_target(&self) -> Option<Vector3<f32>> {
        let slot = self.my_box_slot()?;
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::PlanDiag::note_slot_tick();
        Some(BoxMovement::hold(self.ctx, slot))
    }

    /// This side's live positional block.
    pub fn shape(&self) -> &TeamShape {
        self.ctx.context.shape_for_team(self.ctx.player.team_id)
    }

    /// **The one place this player is trying to be right now**, when he
    /// has no ball to play.
    ///
    /// This is the single-target resolver: four team-level systems can
    /// each have an opinion about where a player belongs, and this is the
    /// order they win in.
    ///
    /// 1. **Box slot** — he has been given a patch of the penalty area in
    ///    the current attack ([`AttackPlan`]). Most specific, so it wins.
    /// 2. **Defensive duty** — he has been given a man, a cover angle, or
    ///    the ball carrier ([`DefensivePlan`]). Only applies while his
    ///    side is actually defending.
    /// 3. **Width** — he has been told to hold a touchline, or to run
    ///    beyond the man who is ([`WidePlan`]). Above the block because
    ///    that is the entire point: the block is a rectangle sized by an
    ///    average, and an average never puts anybody on the paint.
    /// 4. **Team shape** — his anchor inside the live block
    ///    ([`TeamShape`]). Always available, covers all eleven, and is
    ///    what makes the side one body rather than eleven agents.
    /// 5. **Kickoff dot** — the pre-plan fallback, kept only for the
    ///    first tick of a match and for a player who has somehow left
    ///    every roster.
    ///
    /// Every off-ball state steers at this instead of deriving its own
    /// destination, which is what stops two players computing the same
    /// answer from the same global geometry and running to the same patch
    /// of grass.
    pub fn my_anchor(&self) -> Vector3<f32> {
        if let Some(slot) = self.my_box_slot_target() {
            return slot;
        }
        if !self.tactical().in_possession {
            if let Some(duty) = self.my_duty_anchor() {
                return duty;
            }
        }
        if let Some(wide) = self.my_width_anchor() {
            return wide;
        }
        if let Some(anchor) = self.shape().anchor_of(self.ctx.player.id) {
            return anchor;
        }
        self.ctx.player.start_position
    }

    /// This side's width assignment — who is on each touchline and who
    /// is running beyond them.
    pub fn wide_plan(&self) -> &WidePlan {
        &self.attack_plan().wide
    }

    /// The touchline this player has been told to hold, if any. `None`
    /// whenever his side is not in possession.
    pub fn my_width_flank(&self) -> Option<Flank> {
        self.wide_plan().flank_of(self.ctx.player.id)
    }

    /// Is this player the one licensed to run beyond the ball-side width
    /// holder this possession?
    pub fn is_overlap_runner(&self) -> bool {
        self.wide_plan().is_overlap_runner(self.ctx.player.id)
    }

    /// Where a width holder — or the man overlapping him — should be
    /// standing.
    ///
    /// The DEPTH is whatever the block already wanted: holding width is
    /// a lateral instruction, and a winger who abandons his line to
    /// stand on a touchline is not holding width, he is lost. The
    /// LATERAL is the whole assignment, and it scales with
    /// `team_width_target` so a deliberately narrow side still plays
    /// narrow — at width 0 this returns the block anchor unchanged and
    /// the engine behaves exactly as it did before the width plan
    /// existed.
    ///
    /// The overlap runner is placed a stride *outside* the holder's line
    /// and further forward, because the run that matters is the one that
    /// gets beyond him. Being outside is what makes the pass a simple
    /// one and the defender's decision a hard one.
    pub fn my_width_anchor(&self) -> Option<Vector3<f32>> {
        let plan = self.attack_plan();
        if !plan.wide.active {
            return None;
        }
        let field_height = self.ctx.context.field_size.height as f32;
        let block = self.shape().anchor_of(self.ctx.player.id)?;
        // 0.35 floor: a man given the touchline goes at least part of
        // the way there whatever the tactic says, or the assignment is
        // decorative.
        let hug = 0.35 + 0.65 * self.team_width_target().clamp(0.0, 1.0);

        if let Some(flank) = plan.wide.flank_of(self.ctx.player.id) {
            return Some(WidePlan::width_anchor(block, flank, field_height, hug));
        }
        if plan.wide.is_overlap_runner(self.ctx.player.id) {
            let flank = plan.wide.ball_flank;
            let base = WidePlan::width_anchor(block, flank, field_height, hug);
            let forward = self.ctx.player.side.map_or(1.0, |s| s.forward_dir_x());
            let field_width = self.ctx.context.field_size.width as f32;
            return Some(Vector3::new(
                (base.x + forward * Self::OVERLAP_LEAD).clamp(14.0, field_width - 14.0),
                (base.y + flank.sign() * Self::OVERLAP_OUTSIDE).clamp(10.0, field_height - 10.0),
                0.0,
            ));
        }
        None
    }

    /// How far this player is from the place his team wants him.
    pub fn distance_from_anchor(&self) -> f32 {
        (self.ctx.player.position - self.my_anchor()).magnitude()
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
        let player_id = self.ctx.player.id;
        let tick = self.ctx.current_tick();
        let cached = self
            .ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(player_id, tick)
            .is_attack_ready;
        if let Some(v) = cached {
            debug_assert_eq!(
                v,
                self.compute_is_attack_ready(),
                "is_attack_ready cache mismatch"
            );
            return v;
        }
        let v = self.compute_is_attack_ready();
        self.ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(player_id, tick)
            .is_attack_ready = Some(v);
        v
    }

    fn compute_is_attack_ready(&self) -> bool {
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
        let player_id = self.ctx.player.id;
        let tick = self.ctx.current_tick();
        let cached = self
            .ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(player_id, tick)
            .counter_window;
        if let Some(v) = cached {
            debug_assert_eq!(
                v,
                self.compute_counter_window(),
                "counter_window cache mismatch"
            );
            return v;
        }
        let v = self.compute_counter_window();
        self.ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(player_id, tick)
            .counter_window = Some(v);
        v
    }

    fn compute_counter_window(&self) -> bool {
        let ctx = self.ctx;
        let current_tick = ctx.context.current_tick();
        let ticks_since_gain = current_tick.saturating_sub(self.coach().last_possession_gain_tick);
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
        let rebound_live =
            rebound_tick > 0 && current_tick.saturating_sub(rebound_tick) < REBOUND_WINDOW_TICKS;
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
        let player_id = self.ctx.player.id;
        let tick = self.ctx.current_tick();
        let cached = self
            .ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(player_id, tick)
            .is_control_ball;
        if let Some(v) = cached {
            debug_assert_eq!(
                v,
                self.compute_is_control_ball(),
                "is_control_ball cache mismatch"
            );
            return v;
        }
        let v = self.compute_is_control_ball();
        self.ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(player_id, tick)
            .is_control_ball = Some(v);
        v
    }

    fn compute_is_control_ball(&self) -> bool {
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
        // is closer to the ball than any opponent. Reads the roster
        // join's per-team control table instead of re-scanning both
        // teams — the min values are identical (same entries, same
        // operand order); the table just computes them once per tick.
        let my_id = self.ctx.player.id;
        let closest_teammate_dist_sq = self
            .ctx
            .tick_context
            .roster
            .control_min_excluding(current_player_team_id, my_id);

        // Opponent lookup: `my_id` is not on the opposing team, so the
        // exclusion is a no-op — matching the old opponents().all() scan
        // (which filtered `entry.id == player_id` to no effect).
        let closest_opponent_dist_sq = self
            .ctx
            .tick_context
            .roster
            .control_min_other_team(current_player_team_id, my_id);

        #[cfg(debug_assertions)]
        {
            let ball_pos = self.ctx.tick_context.positions.ball.position;
            let ref_team = self
                .ctx
                .players()
                .teammates()
                .all()
                .map(|p| (p.position - ball_pos).norm_squared())
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let ref_opp = self
                .ctx
                .players()
                .opponents()
                .all()
                .map(|p| (p.position - ball_pos).norm_squared())
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            debug_assert_eq!(
                closest_teammate_dist_sq, ref_team,
                "control table teammate-min mismatch"
            );
            debug_assert_eq!(
                closest_opponent_dist_sq, ref_opp,
                "control table opponent-min mismatch"
            );
        }

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
        let player_id = self.ctx.player.id;
        let tick = self.ctx.current_tick();
        let cached = self
            .ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(player_id, tick)
            .is_teammate_chasing_ball;
        if let Some(v) = cached {
            debug_assert_eq!(
                v,
                self.compute_is_teammate_chasing_ball(),
                "is_teammate_chasing_ball cache mismatch"
            );
            return v;
        }
        let v = self.compute_is_teammate_chasing_ball();
        self.ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(player_id, tick)
            .is_teammate_chasing_ball = Some(v);
        v
    }

    fn compute_is_teammate_chasing_ball(&self) -> bool {
        let ball_position = self.ctx.tick_context.positions.ball.position;
        let my_id = self.ctx.player.id;
        let my_team = self.ctx.player.team_id;
        // Same per-element expression as before, hoisted out of the loop
        // (it doesn't depend on the candidate).
        let my_dist_sq_scaled = (ball_position - self.ctx.player.position).norm_squared() * 1.44; // 1.2^2

        // Walk the per-tick roster join — position AND velocity are
        // pre-joined, replacing three hash probes per candidate.
        self.ctx.tick_context.roster.iter().any(|entry| {
            if entry.id == my_id || entry.team_id != my_team {
                return false;
            }

            if entry.velocity.norm_squared() < 0.01 {
                return false;
            }

            let direction_to_ball = (ball_position - entry.position).normalize();
            let player_direction = entry.velocity.normalize();
            let dot_product = direction_to_ball.dot(&player_direction);

            // Player is moving toward the ball (use norm_squared for distance comparison)
            dot_product > 0.85
                && (ball_position - entry.position).norm_squared() < my_dist_sq_scaled
        })
    }

    // Determine if this player is the best positioned to chase the ball.
    // Memoized per (player, tick): `should_follow_waypoints` asks this on
    // nearly every off-ball velocity evaluation and the state decision
    // trees ask again in the same tick — the full teammate scan below is
    // deterministic over the frozen tick snapshot, so one compute per
    // player per tick is bit-identical.
    pub fn is_best_player_to_chase_ball(&self) -> bool {
        let player_id = self.ctx.player.id;
        let tick = self.ctx.current_tick();
        let cached = self
            .ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(player_id, tick)
            .is_best_to_chase_ball;
        if let Some(v) = cached {
            debug_assert_eq!(
                v,
                self.compute_is_best_player_to_chase_ball(),
                "is_best_to_chase_ball cache mismatch"
            );
            return v;
        }
        let v = self.compute_is_best_player_to_chase_ball();
        self.ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(player_id, tick)
            .is_best_to_chase_ball = Some(v);
        v
    }

    fn compute_is_best_player_to_chase_ball(&self) -> bool {
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

        // Compare against teammates via the per-tick roster join. The
        // per-candidate `(pace·accel·pos_factor·0.5+0.5)²` denominator is
        // precomputed once per tick (`RosterEntryLive::chase_ability_sq`,
        // identical operand order), replacing a `by_id` skill lookup per
        // candidate per call. A NaN denominator (missing player) makes
        // the comparison false — same as the old `by_id → None` skip.
        let my_id = self.ctx.player.id;
        let my_team = self.ctx.player.team_id;
        !self.ctx.tick_context.roster.iter().any(|entry| {
            if entry.id == my_id || entry.team_id != my_team {
                return false;
            }
            let dist_sq = (ball_position - entry.position).norm_squared();
            // Quick distance check
            if dist_sq > player_dist_sq {
                return false;
            }

            let score = dist_sq / entry.chase_ability_sq;
            score < threshold
        })
    }
}

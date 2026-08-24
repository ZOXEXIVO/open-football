#[cfg(feature = "match-logs")]
use crate::r#match::PlayerSide;
use crate::r#match::engine::ball::ball::Ball;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::PassOriginRestart;
use crate::r#match::events::EventCollection;
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer};

impl Ball {
    /// Update cached landing position. Call after physics changes position/velocity.
    #[inline]
    pub fn update_landing_cache(&mut self) {
        self.cached_landing_position = self.calculate_landing_position();
    }

    /// Hand the woodwork trace the ball as tick `tick` left it.
    ///
    /// Sampled at the TOP of the update rather than the bottom because the
    /// update has three exits — the netting, the awaited restart and the
    /// ordinary one — and a ball that stops being sampled the moment it goes
    /// in the goal is a ball whose trace ends exactly where the report says
    /// it goes wrong. One call site covers all three, and any relocation
    /// applied BETWEEN two updates (a set-piece teleport) shows up in the
    /// gap. The post-goal celebration runs with the whole tick body skipped,
    /// so it samples itself — see `advance_goal_celebration`.
    #[cfg(feature = "match-logs")]
    pub(crate) fn trace_tick(&self, tick: u64, players: &[MatchPlayer]) {
        use crate::r#match::engine::ball::ball::frame_trace::{FrameTrace, Sample};

        if !FrameTrace::armed() {
            return;
        }
        let owner_role = self
            .current_owner
            .and_then(|id| players.iter().find(|p| p.id == id))
            .map(
                |p| match p.tactical_position.current_position.position_group() {
                    crate::PlayerFieldPositionGroup::Goalkeeper => 'G',
                    crate::PlayerFieldPositionGroup::Defender => 'D',
                    crate::PlayerFieldPositionGroup::Midfielder => 'M',
                    crate::PlayerFieldPositionGroup::Forward => 'F',
                },
            )
            .unwrap_or('-');
        // Nearest keeper, so the trace can answer whether he was on the
        // floor when the ball came to him — see `Sample::gk`.
        let gk = players
            .iter()
            .filter(|p| {
                p.tactical_position.current_position.position_group()
                    == crate::PlayerFieldPositionGroup::Goalkeeper
            })
            .map(|p| {
                let gap = (p.position.x - self.position.x).hypot(p.position.y - self.position.y);
                (gap, p.height, p.state.compact_id())
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        // A ball on its way UP through `SKY_HEIGHT`. Latched so one flight
        // opens one window rather than one a tick for the whole climb, and
        // read off the sample rather than off any launch site because the
        // report does not say which site launched it.
        //
        // The latch is per PROCESS, like the store it feeds. That is the
        // right grain for `dev_match sky`, which plays its matches one
        // after another; two matches running side by side would share it
        // and lose windows, which is a reason not to run this trace under
        // the parallel harness rather than a reason to widen it.
        if FrameTrace::captures_skied() {
            use std::sync::atomic::{AtomicBool, Ordering};
            static ALOFT: AtomicBool = AtomicBool::new(false);
            let aloft = self.position.z > FrameTrace::SKY_HEIGHT;
            if aloft && !ALOFT.swap(true, Ordering::Relaxed) {
                FrameTrace::open(format!(
                    "SKIED through {:.0} m at ({:.1}, {:.1}, {:.2}) v({:.2},{:.2},{:.3}) owner {:?} held {} awaiting {}",
                    FrameTrace::SKY_HEIGHT,
                    self.position.x,
                    self.position.y,
                    self.position.z,
                    self.velocity.x,
                    self.velocity.y,
                    self.velocity.z,
                    self.current_owner,
                    self.held_in_hands,
                    self.awaiting_restart.is_some(),
                ));
            } else if !aloft {
                ALOFT.store(false, Ordering::Relaxed);
            }
        }
        FrameTrace::note_tick(Sample {
            tick,
            pos: self.position,
            vel: self.velocity,
            owner: self.current_owner,
            owner_role,
            in_net: self.in_net.is_some(),
            awaiting_restart: self.awaiting_restart.is_some(),
            held: self.held_in_hands,
            gk,
        });
    }

    pub fn update(
        &mut self,
        context: &mut MatchContext,
        players: &[MatchPlayer],
        tick_context: &GameTickContext,
        events: &mut EventCollection,
    ) {
        #[cfg(feature = "match-logs")]
        self.trace_tick(self.current_tick_cached, players);
        self.current_tick_cached = context.current_tick();
        #[cfg(feature = "match-logs")]
        let owner_at_entry = self.current_owner;
        #[cfg(feature = "match-logs")]
        let held_at_entry = self.held_in_hands;
        #[cfg(feature = "match-logs")]
        let spell_at_entry = self.ownership_duration;
        // The ball's own pass, split three ways for the whole-tick
        // relocation census. Both early returns below sit above the point
        // `flight_diag`'s `StageProbe` starts booking, so the netting and
        // the entire restart machinery have never been in a census.
        #[cfg(feature = "match-logs")]
        let census = crate::r#match::engine::ball::ball::teleport::BallPass::open(self);
        #[cfg(feature = "match-logs")]
        {
            use std::sync::atomic::Ordering;
            crate::r#match::engine::ball::ball::ownership::reception_diag::TOTAL_TICKS
                .fetch_add(1, Ordering::Relaxed);
            if self.held_in_hands {
                crate::r#match::engine::ball::ball::ownership::reception_diag::HELD_TICKS
                    .fetch_add(1, Ordering::Relaxed);
            }
        }

        // The ball is in the goal: the netting owns it and play is dead
        // until the restart. Nothing below applies — there is no pass to
        // intercept, no shot to save, no owner to track and no boundary to
        // clamp against — and several of those passes would actively
        // misread it (see the guards in `goal.rs`). The celebration drives
        // the ball from here; this covers the ticks between the goal and
        // the flow layer noticing it, plus any caller driving `update`
        // directly.
        if self.in_net.is_some() {
            self.tick_net(&context.goal_positions);
            #[cfg(feature = "match-logs")]
            census.close(
                self,
                crate::r#match::engine::ball::ball::teleport::STAGE_BALL_NET,
            );
            return;
        }

        // Decrement claim cooldown
        if self.claim_cooldown > 0 {
            self.claim_cooldown -= 1;
        }

        // A vertical speed that is higher than the one this ball's own
        // physics produced last tick was put there by a kick — see
        // `settled_vz`. Sampled before `update_velocity` so the bounce
        // it applies is not mistaken for one.
        #[cfg(feature = "match-logs")]
        {
            if self.velocity.z > self.settled_vz + 1.0e-5 && self.velocity.z > 0.0 {
                let striker = self
                    .current_owner
                    .or(self.previous_owner)
                    .and_then(|id| players.iter().find(|p| p.id == id))
                    .map(|p| p.state.compact_id() as usize);
                crate::r#match::engine::ball::ball::flight_diag::FlightDiag::note_launch(
                    self.velocity.z,
                    self.position.z,
                    striker,
                );
            }
        }
        #[cfg(feature = "match-logs")]
        let mut probe =
            crate::r#match::engine::ball::ball::flight_diag::StageProbe::new(self.position);

        // ── A ball that is OUT OF PLAY ────────────────────────────────
        //
        // Everything below this point is the machinery of a live ball:
        // interception, blocks, saves, the loose-ball chase signals, the
        // stall detectors and the ownership scan. None of it applies to a
        // ball lying on the touchline waiting to be thrown in, and every
        // one of them would fight the restart — the chase signals would
        // send an OPPONENT to fetch it, and `check_ball_ownership` would
        // simply give it to whoever was nearest. So the restart is ticked
        // here and the rest of the update is skipped outright.
        //
        // The physics below is skipped too, but the physics does not stop:
        // a ball that has just been put out of play is still travelling,
        // and `tick_awaited_restart` integrates it itself until the
        // hoardings stop it, then pins it where it comes to rest. What is
        // skipped is everything that would let somebody TOUCH it. See
        // [`AwaitedRestart`] and [`RunOff`].
        if self.awaiting_restart.is_some() {
            self.tick_awaited_restart(context, players, events);
            if self.awaiting_restart.is_some() {
                self.update_landing_cache();
                #[cfg(feature = "match-logs")]
                census.close(
                    self,
                    crate::r#match::engine::ball::ball::teleport::STAGE_BALL_RESTART,
                );
                return;
            }
        }

        self.update_velocity();
        self.tick_aerial_delivery(players);

        self.try_intercept(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            crate::r#match::engine::ball::ball::flight_diag::STAGE_INTERCEPT,
            self.position,
            self.velocity,
            0.0,
        );
        self.try_block_shot(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            crate::r#match::engine::ball::ball::flight_diag::STAGE_BLOCK,
            self.position,
            self.velocity,
            0.0,
        );
        self.try_save_shot(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            crate::r#match::engine::ball::ball::flight_diag::STAGE_SAVE,
            self.position,
            self.velocity,
            0.0,
        );
        self.try_notify_standing_ball(players, events);

        // NUCLEAR OPTION: Force claiming if ball unowned and stopped for too long
        self.force_claim_if_deadlock(players, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            crate::r#match::engine::ball::ball::flight_diag::STAGE_DEADLOCK,
            self.position,
            self.velocity,
            0.0,
        );

        // Unconditional unowned safety net - forces nearest players to TakeBall
        self.force_takeball_if_unowned_too_long(players, events);
        // `detect_owned_stuck` was too sensitive — it fired on legitimate
        // possession play (defender holding in back line for 6-12s is
        // normal). `detect_position_stall` is the stricter signal: ball
        // hasn't moved ANYWHERE in 1000 ticks, regardless of who owns
        // it. That's a real stall.
        self.detect_position_stall(players);
        #[cfg(feature = "match-logs")]
        probe.note(
            crate::r#match::engine::ball::ball::flight_diag::STAGE_STALL,
            self.position,
            self.velocity,
            0.0,
        );

        self.process_ownership(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            crate::r#match::engine::ball::ball::flight_diag::STAGE_OWNERSHIP,
            self.position,
            self.velocity,
            0.0,
        );
        self.tick_carry_tracker(events);
        // **The keeper's own body**, last of everything that can touch the
        // ball and immediately before the step it would otherwise travel
        // through him on. After ownership because a ball he is entitled to
        // control is a reception rather than a collision; before the move
        // because the step it sweeps is the one about to be taken. See
        // [`KeeperBody`](crate::r#match::engine::ball::ball::contest::body::KeeperBody).
        #[cfg(feature = "match-logs")]
        let body_allowance = (self.velocity.x * self.velocity.x
            + self.velocity.y * self.velocity.y)
            .sqrt()
            .max(1.5);
        self.try_keeper_body_block(context, players);
        #[cfg(feature = "match-logs")]
        probe.note(
            crate::r#match::engine::ball::ball::flight_diag::STAGE_KEEPER_BODY,
            self.position,
            self.velocity,
            body_allowance,
        );

        // Move ball FIRST, then check goal/boundary on new position
        // `move_to` is entitled to a tick of its own velocity, plus the
        // owner-tracking step it uses instead when the ball is carried.
        #[cfg(feature = "match-logs")]
        let move_allowance = (self.velocity.x * self.velocity.x
            + self.velocity.y * self.velocity.y)
            .sqrt()
            .max(1.5);
        self.move_to(tick_context);
        #[cfg(feature = "match-logs")]
        probe.note(
            crate::r#match::engine::ball::ball::flight_diag::STAGE_MOVE,
            self.position,
            self.velocity,
            move_allowance,
        );
        // The woodwork, ahead of every out-of-play resolver: a ball that has
        // hit the frame has not crossed the line, gone over the bar or gone
        // out, and each of those would otherwise claim it.
        self.check_frame_rebound(context, events);
        self.check_goal(context, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            crate::r#match::engine::ball::ball::flight_diag::STAGE_GOAL,
            self.position,
            self.velocity,
            0.0,
        );
        self.check_over_goal(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            crate::r#match::engine::ball::ball::flight_diag::STAGE_OVER_BAR,
            self.position,
            self.velocity,
            0.0,
        );
        self.check_wide_of_goal(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            crate::r#match::engine::ball::ball::flight_diag::STAGE_WIDE,
            self.position,
            self.velocity,
            0.0,
        );
        self.check_throw_in(context, players, events);
        #[cfg(feature = "match-logs")]
        probe.note(
            crate::r#match::engine::ball::ball::flight_diag::STAGE_THROW_IN,
            self.position,
            self.velocity,
            0.0,
        );
        self.check_boundary_collision(context);
        #[cfg(feature = "match-logs")]
        probe.note(
            crate::r#match::engine::ball::ball::flight_diag::STAGE_BOUNDARY,
            self.position,
            self.velocity,
            0.0,
        );
        self.expire_offside_snapshot(context);
        self.update_landing_cache();

        #[cfg(feature = "match-logs")]
        census.close(
            self,
            crate::r#match::engine::ball::ball::teleport::STAGE_BALL_LIVE,
        );

        #[cfg(feature = "match-logs")]
        {
            crate::r#match::engine::ball::ball::flight_diag::FlightDiag::note_tick(
                self.position,
                self.velocity,
                self.current_owner.is_some(),
            );
            self.settled_vz = self.velocity.z;

            // Possession churn, sampled once per full tick around the
            // whole ball update — so it catches every release site,
            // including the ones inside `move_to` and the boundary
            // checks, without a counter planted at each.
            use crate::r#match::engine::ball::ball::stall::dead_ball_diag as dbd;
            use std::sync::atomic::Ordering;

            // Pressure on the man in possession — the "is anybody coming
            // to him" number. Sampled here rather than in any state
            // because it is a property of the SITUATION, and every state
            // that could measure it has already decided not to engage.
            if let Some(owner) = self
                .current_owner
                .and_then(|id| players.iter().find(|p| p.id == id))
            {
                let mut nearest = f32::MAX;
                let mut engagers = 0u64;
                for opp in players.iter() {
                    if opp.team_id == owner.team_id || opp.is_sent_off {
                        continue;
                    }
                    let d = (opp.position - owner.position).magnitude();
                    nearest = nearest.min(d);
                    if d < 80.0 {
                        engagers += 1;
                    }
                }
                if nearest < f32::MAX {
                    let m = nearest * 0.125;
                    let bucket = if m < 2.0 {
                        0
                    } else if m < 5.0 {
                        1
                    } else if m < 10.0 {
                        2
                    } else if m < 20.0 {
                        3
                    } else {
                        4
                    };
                    dbd::CARRIER_PRESSURE[bucket].fetch_add(1, Ordering::Relaxed);
                    // Thirds from the CARRIER's point of view, so "own
                    // third" means his own regardless of which way he
                    // is playing.
                    let attacking_right = owner.side == Some(crate::r#match::PlayerSide::Left);
                    let progress = if attacking_right {
                        self.position.x / self.field_width
                    } else {
                        1.0 - self.position.x / self.field_width
                    };
                    let third = if progress < 0.333 {
                        0
                    } else if progress < 0.667 {
                        1
                    } else {
                        2
                    };
                    dbd::CARRIER_PRESSURE_BY_THIRD[third * 5 + bucket]
                        .fetch_add(1, Ordering::Relaxed);
                    dbd::CARRIER_NEAREST_X10.fetch_add((nearest * 10.0) as u64, Ordering::Relaxed);
                    dbd::CARRIER_ENGAGERS.fetch_add(engagers, Ordering::Relaxed);
                    dbd::CARRIER_SAMPLES.fetch_add(1, Ordering::Relaxed);
                    // …and can he actually stay with him? See
                    // `CHASE_SAMPLES` — the ceilings, not the positions.
                    if let Some(chaser) = players
                        .iter()
                        .filter(|p| p.team_id != owner.team_id && !p.is_sent_off)
                        .filter(|p| {
                            p.tactical_position.current_position.position_group()
                                != crate::PlayerFieldPositionGroup::Goalkeeper
                        })
                        .min_by(|a, b| {
                            (a.position - owner.position)
                                .magnitude()
                                .total_cmp(&(b.position - owner.position).magnitude())
                        })
                    {
                        use crate::r#match::engine::teamplay::standard::MatchStandard;
                        use crate::r#match::player::strategies::players::ops::skill_composites as sc;
                        use crate::r#match::{ActivityIntensity, MovementEffort};
                        let minute = sc::minute_from_ticks(self.current_tick_cached);
                        // ⚠ THROUGH `carrier_ceiling`, NOT A COPY OF IT.
                        // The first version of this sampler re-derived the
                        // carry formula inline and went stale the moment
                        // the live path changed, reporting the OLD ceiling
                        // against the new chaser's.
                        let carrier_cap = owner.max_speed_with_condition_cached()
                            * MovementEffort::carrier_ceiling(
                                owner,
                                minute,
                                owner.player_attributes.condition_percentage(),
                                MatchStandard::shift(context),
                            );
                        let chaser_cap = chaser.max_speed_with_condition_cached()
                            * MovementEffort::speed_fraction(
                                chaser.last_activity_intensity,
                                chaser.player_attributes.condition_percentage(),
                            );
                        dbd::CHASE_SAMPLES.fetch_add(1, Ordering::Relaxed);
                        dbd::CHASE_CARRIER_CAP_X1000
                            .fetch_add((carrier_cap * 1000.0) as u64, Ordering::Relaxed);
                        dbd::CHASE_CHASER_CAP_X1000
                            .fetch_add((chaser_cap * 1000.0) as u64, Ordering::Relaxed);
                        dbd::CHASE_CARRIER_SPD_X1000
                            .fetch_add((owner.velocity.norm() * 1000.0) as u64, Ordering::Relaxed);
                        dbd::CHASE_CHASER_SPD_X1000
                            .fetch_add((chaser.velocity.norm() * 1000.0) as u64, Ordering::Relaxed);
                        if chaser_cap < carrier_cap {
                            dbd::CHASE_OUTPACED.fetch_add(1, Ordering::Relaxed);
                        }
                        let tier = match chaser.last_activity_intensity {
                            ActivityIntensity::VeryHigh => 0,
                            ActivityIntensity::High => 1,
                            ActivityIntensity::Moderate => 2,
                            ActivityIntensity::Low => 3,
                            ActivityIntensity::Recovery => 4,
                        };
                        dbd::CHASE_TIER[tier].fetch_add(1, Ordering::Relaxed);
                    }
                }
            }

            // Whole-match TakeBall ownership, not just stalls: is this a
            // state that holds the ball, or the state everybody is in on
            // the tick they claim it?
            let tb_now = self
                .current_owner
                .and_then(|id| players.iter().find(|p| p.id == id))
                .is_some_and(|p| p.state.is_take_ball());
            if tb_now {
                dbd::TAKEBALL_OWN_TICKS.fetch_add(1, Ordering::Relaxed);
                if !self.takeball_owned_last_tick || owner_at_entry != self.current_owner {
                    dbd::TAKEBALL_OWN_SPELLS.fetch_add(1, Ordering::Relaxed);
                }
            }
            self.takeball_owned_last_tick = tb_now;

            if owner_at_entry != self.current_owner {
                // Turnovers that happen while the ball is already judged
                // stuck. Cross-team means a real scramble; same-team
                // means it is bouncing around one side, which would be a
                // passing problem rather than a contest.
                if self.stall_anchor_tick >= 250 && self.current_owner.is_some() {
                    dbd::STALL_TURNOVERS.fetch_add(1, Ordering::Relaxed);
                    let team_of = |id: Option<u32>| {
                        id.and_then(|i| players.iter().find(|p| p.id == i))
                            .map(|p| p.team_id)
                    };
                    let before = team_of(owner_at_entry);
                    let after = team_of(self.current_owner);
                    if before.is_some() && after.is_some() && before != after {
                        dbd::STALL_TURNOVERS_CROSS_TEAM.fetch_add(1, Ordering::Relaxed);
                    }
                }
                if owner_at_entry.is_some() {
                    dbd::OWNERSHIP_LOST.fetch_add(1, Ordering::Relaxed);
                    dbd::SPELL_LENGTH[dbd::spell_bucket(spell_at_entry)]
                        .fetch_add(1, Ordering::Relaxed);
                }
                if self.current_owner.is_some() {
                    dbd::OWNERSHIP_GAINED.fetch_add(1, Ordering::Relaxed);
                    if self.current_owner == self.previous_owner {
                        dbd::OWNERSHIP_RECLAIMED_SELF.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
        #[cfg(feature = "match-logs")]
        self.census_keeper_possession(context, players, owner_at_entry, held_at_entry);
        #[cfg(feature = "match-logs")]
        self.census_shot_fate(context, players);
    }

    /// One sample per tick of what the keeper is doing with the ball, and
    /// how he stops doing it. See
    /// [`crate::r#match::engine::ball::ball::ownership::reception_diag::KEEPER_BALL`] for why the hand/foot
    /// split is the whole point.
    #[cfg(feature = "match-logs")]
    fn census_keeper_possession(
        &self,
        context: &MatchContext,
        players: &[MatchPlayer],
        owner_at_entry: Option<u32>,
        held_at_entry: bool,
    ) {
        use crate::PlayerFieldPositionGroup;
        use crate::r#match::engine::ball::ball::ownership::reception_diag as d;

        let keeper_of = |id: Option<u32>| {
            id.and_then(|i| players.iter().find(|p| p.id == i))
                .filter(|p| {
                    p.tactical_position.current_position.position_group()
                        == PlayerFieldPositionGroup::Goalkeeper
                })
        };

        // How the possession ENDED. Read against the owner at entry so a
        // hand-off resolved anywhere inside this tick is caught, whichever
        // of the fifteen ownership-granting sites did it.
        if owner_at_entry != self.current_owner {
            if let (Some(was), Some(now_id)) = (keeper_of(owner_at_entry), self.current_owner) {
                let stolen = players
                    .iter()
                    .find(|p| p.id == now_id)
                    .is_some_and(|p| p.team_id != was.team_id);
                if stolen {
                    d::keeper_ball_note(if held_at_entry { 9 } else { 8 });
                    if let crate::r#match::player::state::PlayerState::Goalkeeper(gk) = was.state {
                        d::keeper_robbed_state(gk as usize);
                    }
                }
            }
        }

        let Some(keeper) = keeper_of(self.current_owner) else {
            return;
        };

        // Did his gloves come open under him? Same possession, same
        // player, hands lowered — nobody touched the ball, so nothing in
        // the Laws or the physics can explain it and something in the
        // engine cleared the flag. Must read zero.
        if held_at_entry && !self.held_in_hands && owner_at_entry == self.current_owner {
            d::keeper_ball_note(12);
        }
        let area = context.penalty_area(keeper.side == Some(PlayerSide::Left));
        let in_area = (area.min.x..=area.max.x).contains(&self.position.x)
            && (area.min.y..=area.max.y).contains(&self.position.y);
        // 5.0u is `BALL_DISTANCE_THRESHOLD` — the radius inside which
        // `check_ball_ownership` will consider handing the ball over.
        const CLAIM_RADIUS_SQ: f32 = 5.0 * 5.0;
        let mut closest = false;
        let mut opponents_in_area = 0u64;
        for p in players.iter().filter(|p| p.team_id != keeper.team_id) {
            if (p.position - self.position).norm_squared() < CLAIM_RADIUS_SQ {
                closest = true;
            }
            if (area.min.x..=area.max.x).contains(&p.position.x)
                && (area.min.y..=area.max.y).contains(&p.position.y)
            {
                opponents_in_area += 1;
            }
        }

        if self.held_in_hands {
            d::keeper_ball_note(4);
            if opponents_in_area > 0 {
                d::keeper_ball_note(5);
            }
            if closest {
                d::keeper_ball_note(6);
            }
            d::keeper_ball_add(7, opponents_in_area);
            // 50 engine ticks to the second.
            let phase = if self.ownership_duration < 50 {
                14
            } else if self.ownership_duration < 100 {
                16
            } else {
                18
            };
            d::keeper_ball_note(phase);
            d::keeper_ball_add(phase + 1, opponents_in_area);
            if !held_at_entry || owner_at_entry != self.current_owner {
                d::keeper_ball_note(11);
            }
            return;
        }

        // At his feet. Would the Laws let him pick it up? Same three
        // prohibitions `BallOperationsImpl::handling_verdict` asks about.
        let legal = in_area
            && !self.awaiting_touch_after_release_by(keeper.id)
            && !self.is_backpass_to(keeper.id, keeper.team_id);
        d::keeper_ball_note(0);
        if let crate::r#match::player::state::PlayerState::Goalkeeper(gk) = keeper.state {
            d::keeper_feet_state(gk as usize);
        }
        if closest {
            d::keeper_ball_note(1);
        }
        if legal {
            d::keeper_ball_note(2);
            if closest {
                d::keeper_ball_note(3);
            }
        }
        if held_at_entry || owner_at_entry != self.current_owner {
            d::keeper_ball_note(10);
            if let crate::r#match::player::state::PlayerState::Goalkeeper(gk) = keeper.state {
                d::keeper_feet_start_state(gk as usize);
            }
        }
    }

    /// Classify how the shot in flight ended, exactly once, at the end of
    /// the tick it ended on. Diagnostic only — see the `FATE_*` counters
    /// in `ownership::reception_diag` for why this exists.
    ///
    /// Deliberately central rather than a flag planted at each exit: the
    /// per-site counters that came before it accounted for ~20 of every
    /// 3500 shots struck, because most shots do not leave through any of
    /// the sites that had one.
    #[cfg(feature = "match-logs")]
    fn census_shot_fate(&mut self, context: &MatchContext, players: &[MatchPlayer]) {
        use crate::r#match::engine::ball::ball::ownership::reception_diag as d;
        use std::sync::atomic::Ordering;

        if !self.census_shot_live {
            return;
        }
        d::FATE_LIVE_TICKS.fetch_add(1, Ordering::Relaxed);

        let dist_x100 = (self.census_shot_dist * 100.0) as u64;
        let mut resolve = |counter: &'static std::sync::atomic::AtomicU64, reached_goal: bool| {
            counter.fetch_add(1, Ordering::Relaxed);
            if reached_goal {
                d::FATE_REACHED_DIST_X100.fetch_add(dist_x100, Ordering::Relaxed);
            }
        };

        if self.goal_scored {
            resolve(&d::FATE_GOAL, true);
        } else if self.pass_origin_restart != PassOriginRestart::OpenPlay {
            // A restart was staged this tick — corner, goal kick or
            // throw. The shot went out of play.
            resolve(&d::FATE_OUT, false);
        } else if let Some(owner) = self.current_owner {
            let owner_p = players.iter().find(|p| p.id == owner);
            let is_gk = owner_p
                .map(|p| p.tactical_position.current_position.is_goalkeeper())
                .unwrap_or(false);
            let same_side = owner_p.and_then(|p| p.side) == self.census_shot_side;
            if is_gk && !same_side {
                resolve(&d::FATE_GK, true);
            } else if same_side {
                resolve(&d::FATE_CLAIMED_ATT, false);
            } else {
                resolve(&d::FATE_CLAIMED_DEF, false);
            }
        } else if self.is_delivery_spent() {
            resolve(&d::FATE_STOPPED, false);
        } else if context
            .current_tick()
            .saturating_sub(self.last_shot_struck_tick)
            > 400
        {
            resolve(&d::FATE_TIMEOUT, false);
        } else {
            return; // still in the air
        }
        self.census_shot_live = false;
    }

    /// Light update: full ball logic but reads owner position from players slice directly.
    pub fn update_light(
        &mut self,
        context: &mut MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        #[cfg(feature = "match-logs")]
        self.trace_tick(self.current_tick_cached, players);
        self.current_tick_cached = context.current_tick();
        // See `Ball::update`. `update_light` carries no `StageProbe` at
        // all, so before this the light tick — about half of them — was
        // outside every relocation census there was.
        #[cfg(feature = "match-logs")]
        let census = crate::r#match::engine::ball::ball::teleport::BallPass::open(self);

        // See `Ball::update` — the netting owns a ball that has gone in.
        if self.in_net.is_some() {
            self.tick_net(&context.goal_positions);
            #[cfg(feature = "match-logs")]
            census.close(
                self,
                crate::r#match::engine::ball::ball::teleport::STAGE_BALL_NET,
            );
            return;
        }

        if self.claim_cooldown > 0 {
            self.claim_cooldown -= 1;
        }

        #[cfg(feature = "match-logs")]
        if self.velocity.z > self.settled_vz + 1.0e-5 && self.velocity.z > 0.0 {
            let striker = self
                .current_owner
                .or(self.previous_owner)
                .and_then(|id| players.iter().find(|p| p.id == id))
                .map(|p| p.state.compact_id() as usize);
            crate::r#match::engine::ball::ball::flight_diag::FlightDiag::note_launch(
                self.velocity.z,
                self.position.z,
                striker,
            );
        }

        // Out of play — same skip as the full update above, and it has to
        // be here too or the ball waits for its taker on alternate ticks
        // and is fought over on the others.
        if self.awaiting_restart.is_some() {
            self.tick_awaited_restart(context, players, events);
            if self.awaiting_restart.is_some() {
                self.update_landing_cache();
                #[cfg(feature = "match-logs")]
                census.close(
                    self,
                    crate::r#match::engine::ball::ball::teleport::STAGE_BALL_RESTART,
                );
                return;
            }
        }

        self.update_velocity();
        self.tick_aerial_delivery(players);
        self.try_intercept(context, players, events);
        self.try_block_shot(context, players, events);
        self.try_save_shot(context, players, events);
        self.process_ownership(context, players, events);
        self.tick_carry_tracker(events);
        // See the full tick for why this sits between ownership and the
        // move rather than beside the other contests.
        self.try_keeper_body_block(context, players);

        // Move ball: find owner position from players slice directly
        self.move_to_with_players(players);
        self.check_frame_rebound(context, events);
        self.check_goal(context, events);
        self.check_over_goal(context, players, events);
        self.check_wide_of_goal(context, players, events);
        self.check_throw_in(context, players, events);
        self.check_boundary_collision(context);
        self.expire_offside_snapshot(context);
        self.update_landing_cache();

        #[cfg(feature = "match-logs")]
        census.close(
            self,
            crate::r#match::engine::ball::ball::teleport::STAGE_BALL_LIVE,
        );

        #[cfg(feature = "match-logs")]
        {
            crate::r#match::engine::ball::ball::flight_diag::FlightDiag::note_tick(
                self.position,
                self.velocity,
                self.current_owner.is_some(),
            );
            self.settled_vz = self.velocity.z;
        }
    }
}

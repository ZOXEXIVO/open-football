//! Ball-ownership flow: pass-target receive, deadlock and unowned
//! safety nets, the standing-ball notification dance, and the per-tick
//! ownership claim that decides who is on the ball.

use super::Ball;
#[cfg(feature = "match-logs")]
use crate::match_log_debug;
use crate::r#match::ball::events::BallEvent;
use crate::r#match::events::EventCollection;
use crate::r#match::{MatchContext, MatchPlayer, PassOriginRestart};

impl Ball {
    pub fn process_ownership(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        if self.flags.in_flight_state > 0 {
            self.flags.in_flight_state -= 1;
            // Allow pass target to claim during flight
            self.try_pass_target_claim(players, events);
        } else {
            self.check_ball_ownership(context, players, events);
        }

        self.flags.running_for_ball = self.is_players_running_to_ball(players);
    }

    fn try_pass_target_claim(&mut self, players: &[MatchPlayer], events: &mut EventCollection) {
        // Check if pass target can claim the ball
        if let Some(target_id) = self.pass_target_player_id {
            if let Some(target_player) = players.iter().find(|p| p.id == target_id) {
                // Use cached landing position for aerial balls, current position for ground balls
                let effective_ball_pos = if self.is_aerial() {
                    self.cached_landing_position
                } else {
                    self.position
                };

                let dx = target_player.position.x - effective_ball_pos.x;
                let dy = target_player.position.y - effective_ball_pos.y;
                let dist_sq = dx * dx + dy * dy;

                // Receiver claim radius: 40u (~5m). The accuracy metric
                // rose monotonically with this radius: 14u→21%, 20u→38%,
                // 26u→47%, 32u→72%, 40u→real-football ~85%. The claim is
                // strictly gated by `pass_target_player_id`, so only the
                // INTENDED receiver gets this generous window — opponents
                // in range still can't poach during in-flight. Matches the
                // real definition of a completed pass: "the ball found
                // its target" within a reasonable stride radius — a
                // receiver making a 1-2 step adjustment to collect the
                // ball is still a completed pass.
                const RECEIVER_CLAIM_DISTANCE_SQ: f32 = 40.0 * 40.0;
                const RECEIVER_MAX_HEIGHT: f32 = 2.8;

                if dist_sq < RECEIVER_CLAIM_DISTANCE_SQ && self.position.z <= RECEIVER_MAX_HEIGHT {
                    // Receiver is becoming active. If we have an offside
                    // snapshot for this receiver and it's offside per the
                    // kick-time geometry, fire offside instead of a clean
                    // claim. Set-piece-origin snapshots were never built
                    // (exempt origins skip snapshot creation), so there's
                    // no need to re-check the origin here.
                    if let Some(snap) = self.offside_snapshot {
                        if snap.receiver_id == target_id && snap.is_offside() {
                            let restart_pos = nalgebra::Vector3::new(
                                snap.receiver_x_at_kick.clamp(0.0, self.field_width),
                                snap.receiver_y_at_kick.clamp(0.0, self.field_height),
                                0.0,
                            );
                            self.offside_snapshot = None;
                            self.pass_target_player_id = None;
                            self.flags.in_flight_state = 0;
                            self.cached_shot_target = None;
                            self.pass_origin_restart = PassOriginRestart::FreeKick;
                            events.add_ball_event(BallEvent::Offside(target_id, restart_pos));
                            return;
                        }
                    }
                    let passer_id = self.previous_owner;
                    let target_team = target_player.team_id;
                    self.current_owner = Some(target_id);
                    self.pass_target_player_id = None;
                    self.ownership_duration = 0;
                    self.flags.in_flight_state = 0;
                    let tick = self.current_tick_cached;
                    self.record_touch(target_id, target_team, tick, true);
                    self.offside_snapshot = None;
                    self.pass_origin_restart = PassOriginRestart::OpenPlay;
                    // Post-receive possession protection. Real football:
                    // a player who controls a pass has ~1.5-2 s of settle
                    // time before a challenging defender arrives — they
                    // take the ball in stride, look up, and start their
                    // next action. Our old 50-tick (0.5 s) floor let
                    // counter-pressing opponents strip the ball on the
                    // very next tick after a receive, which turned every
                    // possession into a 1-second ping-pong cycle and
                    // drove the 80-300 shots-per-team metric. 150 ticks
                    // (1.5 s) is a realistic floor — enough for the
                    // receiver to survive the initial close-down without
                    // shutting the game off to defensive pressure entirely.
                    self.claim_cooldown = self.claim_cooldown.max(150);
                    if let Some(pid) = passer_id {
                        events.add_ball_event(BallEvent::PassCompleted(target_id, pid));
                    } else {
                        events.add_ball_event(BallEvent::Claimed(target_id));
                    }
                    return;
                }
            }
        }

        // Also allow previous owner (passer) to reclaim if ball bounced back
        // BUT only after the ball has had time to travel away (in_flight_state < 10)
        // This prevents the passer from immediately reclaiming on low-force passes
        if self.flags.in_flight_state < 10 {
            if let Some(prev_id) = self.previous_owner {
                if let Some(prev_player) = players.iter().find(|p| p.id == prev_id) {
                    let dx = prev_player.position.x - self.position.x;
                    let dy = prev_player.position.y - self.position.y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist < 2.0 && self.position.z <= 2.8 {
                        // Check ball is moving toward passer (bounced back)
                        let ball_speed = self.velocity.norm();
                        if ball_speed > 0.1 {
                            let to_passer_x = dx / dist;
                            let to_passer_y = dy / dist;
                            let dot = (self.velocity.x / ball_speed) * to_passer_x
                                + (self.velocity.y / ball_speed) * to_passer_y;
                            if dot > 0.3 {
                                // Ball moving toward passer
                                let passer_team = prev_player.team_id;
                                self.current_owner = Some(prev_id);
                                self.pass_target_player_id = None;
                                self.ownership_duration = 0;
                                self.flags.in_flight_state = 0;
                                self.claim_cooldown = 15;
                                let tick = self.current_tick_cached;
                                self.record_touch(prev_id, passer_team, tick, true);
                                self.offside_snapshot = None;
                                self.pass_origin_restart = PassOriginRestart::OpenPlay;
                                events.add_ball_event(BallEvent::Claimed(prev_id));
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn try_notify_standing_ball(
        &mut self,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // Don't treat ball as "standing" during in-flight (just passed)
        // Short passes have low velocity that triggers is_ball_stopped_on_field(),
        // but the ball is still in transit to the intended receiver
        if self.flags.in_flight_state > 0 {
            return;
        }

        // Decrement cooldown timer
        if self.notification_cooldown > 0 {
            self.notification_cooldown -= 1;
        }

        // Check if ball is stopped (either outside or inside field) and no one owns it
        let is_ball_stopped = self.is_stands_outside() || self.is_ball_stopped_on_field();

        // Check if ball has moved significantly from last boundary position
        let has_escaped_boundary = if let Some(last_pos) = self.last_boundary_position {
            let dist_sq = (self.position - last_pos).norm_squared();
            dist_sq > 4.0 // 2.0^2
        } else {
            true // No previous boundary position recorded
        };

        if (is_ball_stopped)
            && self.take_ball_notified_players.is_empty()
            && self.current_owner.is_none()
            && self.notification_cooldown == 0 // Only notify if cooldown expired
            && has_escaped_boundary
        // Only notify if ball escaped from previous boundary loop
        {
            let notified_players = self.notify_nearest_player(players, events);
            if !notified_players.is_empty() {
                self.take_ball_notified_players = notified_players;
                self.notification_timeout = 0; // Reset timeout when new players are notified

                // If ball is at boundary, set short cooldown and record position
                if self.is_ball_outside() {
                    self.notification_cooldown = 5; // Short cooldown to prevent spam
                    self.last_boundary_position = Some(self.position);
                }
            }
        } else if !self.take_ball_notified_players.is_empty() {
            // Increment timeout counter
            self.notification_timeout += 1;

            // If players haven't claimed the ball within reasonable time, reset and try again
            const MAX_NOTIFICATION_TIMEOUT: u32 = 60; // ~1 second - reduced from 200 for faster response
            if self.notification_timeout > MAX_NOTIFICATION_TIMEOUT {
                self.take_ball_notified_players.clear();
                self.notification_timeout = 0;
                self.notification_cooldown = 0; // Clear cooldown to allow immediate re-notification
                // Clear boundary position to allow re-notification even if ball hasn't moved
                self.last_boundary_position = None;
                return; // Will re-notify on next tick
            }
            // Check if any notified player reached the ball
            const CLAIM_DISTANCE: f32 = 5.0; // Claim distance for notified players (generous to avoid corner deadlocks)
            const MAX_CLAIM_VELOCITY: f32 = 5.0; // Ball must be slow enough to claim

            let target_position = if self.is_aerial() {
                self.cached_landing_position
            } else {
                self.position
            };

            let ball_speed = self.velocity.norm();
            let can_claim_by_speed = ball_speed < MAX_CLAIM_VELOCITY;

            let mut claiming_player_id: Option<u32> = None;
            let mut all_players_missing = true;

            for notified_player_id in &self.take_ball_notified_players {
                if let Some(player) = players.iter().find(|p| p.id == *notified_player_id) {
                    all_players_missing = false;

                    let dx = player.position.x - target_position.x;
                    let dy = player.position.y - target_position.y;
                    let dz = target_position.z;
                    let distance_3d = (dx * dx + dy * dy + dz * dz).sqrt();

                    // Simple distance check - if close enough and ball is slow, claim it
                    if distance_3d < CLAIM_DISTANCE
                        && self.current_owner.is_none()
                        && can_claim_by_speed
                    {
                        if !self.is_aerial() || self.position.z < 2.5 {
                            claiming_player_id = Some(*notified_player_id);
                            break;
                        }
                    }
                }
            }

            // If all notified players are missing from the players slice, clear the list
            // This can happen if players were substituted or if there's a data inconsistency
            if all_players_missing {
                self.take_ball_notified_players.clear();
            }

            // Process the claim after iteration to avoid borrow checker issues
            if let Some(player_id) = claiming_player_id {
                self.current_owner = Some(player_id);
                self.pass_target_player_id = None;
                self.take_ball_notified_players.clear();
                self.notification_timeout = 0;
                events.add_ball_event(BallEvent::Claimed(player_id));

                // Reset boundary tracking when ball is claimed
                if has_escaped_boundary {
                    self.last_boundary_position = None;
                }
            }
        }
    }

    /// Deadlock resolution: Force the nearest player to claim the ball if it's been sitting unowned for too long
    /// Uses progressive radius - starts strict, expands if stuck to ensure game never deadlocks
    pub(super) fn force_claim_if_deadlock(
        &mut self,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        const DEADLOCK_VELOCITY_ENTER: f32 = 3.0;
        const DEADLOCK_VELOCITY_EXIT: f32 = 4.0;
        const DEADLOCK_HEIGHT_THRESHOLD: f32 = 1.5;

        // Progressive timing thresholds — faster initial response prevents corner deadlocks
        const TICK_PHASE_1: u32 = 15; // ~0.25s - try close range quickly
        const TICK_PHASE_2: u32 = 35; // ~0.6s - expand range
        const TICK_PHASE_3: u32 = 60; // ~1.0s - further expand
        const TICK_PHASE_4: u32 = 100; // ~1.6s - last resort

        // Progressive claim distances — generous to handle boundary/corner situations
        const CLAIM_DISTANCE_PHASE_1: f32 = 5.0; // Close range - matches notification claim distance
        const CLAIM_DISTANCE_PHASE_2: f32 = 8.0; // Medium range - acceptable
        const CLAIM_DISTANCE_PHASE_3: f32 = 12.0; // Extended range - noticeable but not terrible
        const CLAIM_DISTANCE_PHASE_4: f32 = 15.0; // Last resort - better than stuck forever

        let is_unowned = self.current_owner.is_none();

        if !is_unowned {
            self.unowned_stopped_ticks = 0;
            return;
        }

        // Don't interfere with passed/kicked balls
        if self.flags.in_flight_state > 0 {
            self.unowned_stopped_ticks = 0;
            return;
        }

        let velocity_threshold = if self.unowned_stopped_ticks > 0 {
            DEADLOCK_VELOCITY_EXIT
        } else {
            DEADLOCK_VELOCITY_ENTER
        };

        let velocity_sq = self.velocity.norm_squared();
        let threshold_sq = velocity_threshold * velocity_threshold;
        let is_slow = velocity_sq < threshold_sq;
        let is_low = self.position.z < DEADLOCK_HEIGHT_THRESHOLD;

        if is_slow && is_low {
            self.unowned_stopped_ticks += 1;

            // Determine claim distance based on how long we've been waiting
            let (should_claim, claim_distance) = if self.unowned_stopped_ticks >= TICK_PHASE_4 {
                (true, CLAIM_DISTANCE_PHASE_4)
            } else if self.unowned_stopped_ticks >= TICK_PHASE_3 {
                (true, CLAIM_DISTANCE_PHASE_3)
            } else if self.unowned_stopped_ticks >= TICK_PHASE_2 {
                (true, CLAIM_DISTANCE_PHASE_2)
            } else if self.unowned_stopped_ticks >= TICK_PHASE_1 {
                (true, CLAIM_DISTANCE_PHASE_1)
            } else {
                (false, 0.0)
            };

            if should_claim {
                // Find nearest player within current claim distance (use squared to avoid sqrt)
                let claim_distance_sq = claim_distance * claim_distance;
                if let Some(nearest_player) = players
                    .iter()
                    .filter_map(|p| {
                        let dx = p.position.x - self.position.x;
                        let dy = p.position.y - self.position.y;
                        let dist_sq = dx * dx + dy * dy;
                        if dist_sq <= claim_distance_sq {
                            Some((p, dist_sq))
                        } else {
                            None
                        }
                    })
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
                    .map(|(p, _)| p)
                {
                    // Grant ownership
                    self.current_owner = Some(nearest_player.id);
                    self.previous_owner = None;
                    self.pass_target_player_id = None;
                    self.ownership_duration = 0;
                    self.flags.in_flight_state = 0;
                    self.take_ball_notified_players.clear();
                    self.notification_timeout = 0;
                    self.claim_cooldown = 15; // Prevent immediate re-claiming by another player

                    if self.position.z > 0.1 && self.position.z < DEADLOCK_HEIGHT_THRESHOLD {
                        self.position.z = 0.0;
                        self.velocity.z = 0.0;
                    }

                    self.unowned_stopped_ticks = 0;
                    events.add_ball_event(BallEvent::Claimed(nearest_player.id));
                } else if self.unowned_stopped_ticks >= TICK_PHASE_2
                    && self.take_ball_notified_players.is_empty()
                {
                    // No one close enough - notify nearest players to come get it
                    let notified = self.notify_nearest_player(players, events);
                    self.take_ball_notified_players = notified;
                    self.notification_timeout = 0;
                }
            }
        } else {
            if velocity_sq >= threshold_sq {
                self.unowned_stopped_ticks = 0;
            }
        }
    }

    /// Unconditional safety net: if ball has been unowned for too long (regardless of speed/height),
    /// force the nearest player from each team into TakeBall state.
    pub(super) fn force_takeball_if_unowned_too_long(
        &mut self,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        const UNOWNED_THRESHOLD: u32 = 300;
        /// Only a genuinely stuck stall is interesting — a single pass is
        /// unowned briefly by design. 300 ticks is the same threshold as
        /// the nuclear-option force-claim: if the system needed that
        /// intervention, the stall was real. Emitted once per stall, on
        /// resolution, with the full duration.
        const STALL_RESOLVE_LOG_THRESHOLD: u32 = 300;

        if self.current_owner.is_some() {
            if self.unowned_ticks >= STALL_RESOLVE_LOG_THRESHOLD {
                #[cfg(feature = "match-logs")]
                {
                    let claimed_by = self.current_owner.unwrap_or(0);
                    let snapshot = self
                        .stall_start_snapshot
                        .as_deref()
                        .unwrap_or("<no snapshot>");
                    match_log_debug!(
                        "ball stall resolved: uncontrolled for {} ticks, claimed by player {} at ({:.1}, {:.1})\n  [start of period]\n{}",
                        self.unowned_ticks,
                        claimed_by,
                        self.position.x,
                        self.position.y,
                        snapshot,
                    );
                }
            }
            self.unowned_ticks = 0;
            self.stall_start_snapshot = None;
            return;
        }

        self.unowned_ticks += 1;

        // Period just started — capture the state snapshot while it's
        // still fresh. Every transition from owned → unowned triggers
        // this, so routine passes also allocate; negligible for match
        // runtime (~100 passes × 1–2KB string ≈ 200KB discarded across
        // a 90-minute match). Held until resolution, then discarded if
        // below the log threshold or emitted with the log otherwise.
        if self.unowned_ticks == 1 {
            self.stall_start_snapshot = Some(self.format_stall_snapshot(players));
        }

        // Force-takeball fires every UNOWNED_THRESHOLD ticks while the
        // stall persists. The counter is NOT reset — it keeps climbing
        // so the resolution log reports the true total duration.
        if self.unowned_ticks > 0 && self.unowned_ticks % UNOWNED_THRESHOLD == 0 {
            let notified = self.notify_nearest_player(players, events);
            if !notified.is_empty() {
                self.take_ball_notified_players = notified;
                self.notification_timeout = 0;
            }
        }
    }

    pub(super) fn notify_nearest_player(
        &self,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) -> Vec<u32> {
        let ball_position = self.position;
        const NOTIFICATION_RADIUS_SQ: f32 = 500.0 * 500.0;

        // Only 2 teams — use fixed variables instead of HashMap
        let mut team_a_id: u32 = 0;
        let mut team_a_best: Option<(u32, f32)> = None; // (player_id, dist_sq)
        let mut team_b_best: Option<(u32, f32)> = None;

        for player in players {
            let dx = player.position.x - ball_position.x;
            let dy = player.position.y - ball_position.y;
            let dist_sq = dx * dx + dy * dy;

            if dist_sq >= NOTIFICATION_RADIUS_SQ {
                continue;
            }

            // Assign first team encountered as team_a
            if team_a_best.is_none() {
                team_a_id = player.team_id;
            }

            let slot = if player.team_id == team_a_id {
                &mut team_a_best
            } else {
                &mut team_b_best
            };

            match slot {
                Some((_, best_dist)) if dist_sq < *best_dist => {
                    *slot = Some((player.id, dist_sq));
                }
                None => {
                    *slot = Some((player.id, dist_sq));
                }
                _ => {}
            }
        }

        let mut notified_players = Vec::new();
        if let Some((id, _)) = team_a_best {
            events.add_ball_event(BallEvent::TakeMe(id));
            notified_players.push(id);
        }
        if let Some((id, _)) = team_b_best {
            events.add_ball_event(BallEvent::TakeMe(id));
            notified_players.push(id);
        }

        notified_players
    }

    fn is_players_running_to_ball(&self, players: &[MatchPlayer]) -> bool {
        let ball_position = self.position;

        for player in players {
            let vel_sq = player.velocity.norm_squared();
            if vel_sq < 0.001 {
                continue; // Standing still
            }
            let to_ball = ball_position - player.position;
            let dot_product = to_ball.dot(&player.velocity);
            if dot_product > 0.0 {
                return true;
            }
        }

        false
    }

    fn check_ball_ownership(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // COOLDOWN CHECK: If cooldown is active and there's an owner, skip ownership checks
        // This prevents rapid ping-pong between players
        if self.claim_cooldown > 0 && self.current_owner.is_some() {
            // Just increment ownership duration and return
            self.ownership_duration += 1;
            return;
        }

        // Distance threshold for claiming ball.
        // Bumped from 3.5 → 5.0: a clearance that lands and bounces
        // travels 4-5 units/tick horizontally. With a 3.5-unit claim
        // zone the closest chaser only has a 1-tick window to touch
        // the ball as it flies past, and a small positional error
        // (plus Arrive's braking at the landing spot) means they
        // routinely miss by 4-6 units and the ball runs free for
        // 300+ ticks through multiple bounces before anyone catches
        // up. 5.0 is still a realistic first-touch distance (one step
        // to the ball) and gives a wider interception window without
        // affecting actual contact semantics. Genuinely fast balls
        // (> 10 u/t) still get the tighter 1-unit rule below.
        const BALL_DISTANCE_THRESHOLD: f32 = 5.0;
        const BALL_DISTANCE_THRESHOLD_SQUARED: f32 =
            BALL_DISTANCE_THRESHOLD * BALL_DISTANCE_THRESHOLD;
        const PLAYER_HEIGHT: f32 = 1.8; // Average player height in meters
        #[allow(dead_code)]
        const PLAYER_REACH_HEIGHT: f32 = PLAYER_HEIGHT + 0.5; // Player can reach ~2.3m when standing
        // 3.5m includes a proper jump header reach — real elite leapers
        // win aerials closer to 3m, and a chest/thigh trap works all
        // the way up to about shoulder height on the way down. The
        // tighter 2.8 was missing any ball descending through 2.8-3.5,
        // which with the bouncy 0.6 coefficient was most of the window.
        const PLAYER_JUMP_REACH: f32 = PLAYER_HEIGHT + 1.7;
        const MAX_BALL_HEIGHT: f32 = PLAYER_JUMP_REACH + 0.5; // Absolute max reachable height

        // CRITICAL: Early validation - if current owner is too far AND ball is moving, clear ownership
        // This catches cases where ball flies away from owner but ownership wasn't properly cleared
        const MAX_OWNERSHIP_DISTANCE: f32 = 2.0; // Maximum distance to maintain ownership (tightened)
        const MAX_OWNERSHIP_DISTANCE_SQUARED: f32 = MAX_OWNERSHIP_DISTANCE * MAX_OWNERSHIP_DISTANCE;
        const MIN_VELOCITY_FOR_DISTANCE_CHECK: f32 = 0.5; // Check distance if ball is moving at all

        if let Some(current_owner_id) = self.current_owner {
            if let Some(owner) = context.players.by_id(current_owner_id) {
                let dx = owner.position.x - self.position.x;
                let dy = owner.position.y - self.position.y;
                let distance_squared = dx * dx + dy * dy;
                let ball_speed_sq = self.velocity.norm_squared();
                let min_vel_sq = MIN_VELOCITY_FOR_DISTANCE_CHECK * MIN_VELOCITY_FOR_DISTANCE_CHECK;

                // Only clear ownership if ball is moving AND far from owner
                // This prevents interference with deadlock claiming and boundary situations
                if distance_squared > MAX_OWNERSHIP_DISTANCE_SQUARED && ball_speed_sq > min_vel_sq {
                    // Use unnormalized dot product — sign is what matters
                    // dot(velocity, to_owner) < 0 means ball moving away from owner
                    let dot = self.velocity.x * dx + self.velocity.y * dy;

                    if distance_squared > 0.01 {
                        if dot < 0.0 {
                            // Ball is moving away from owner
                            // Owner is too far and ball is flying away - clear ownership
                            self.previous_owner = self.current_owner;
                            self.current_owner = None;
                            self.ownership_duration = 0;
                            // Don't return - continue to allow new ownership claim
                        }
                    }
                }
            } else {
                // Owner player not found - clear ownership
                self.previous_owner = self.current_owner;
                self.current_owner = None;
                self.ownership_duration = 0;
            }
        }

        // Ball is too high to be claimed by any player (flying over everyone's heads)
        if self.position.z > MAX_BALL_HEIGHT {
            return;
        }

        // Check if previous owner is still within range
        // Clear previous_owner once they're far enough away to allow normal claiming
        if let Some(previous_owner_id) = self.previous_owner {
            if let Some(owner) = context.players.by_id(previous_owner_id) {
                let dx = owner.position.x - self.position.x;
                let dy = owner.position.y - self.position.y;
                let dz = self.position.z;
                let dist_3d_sq = dx * dx + dy * dy + dz * dz;

                // Clear previous owner once they're far enough
                if dist_3d_sq > BALL_DISTANCE_THRESHOLD_SQUARED {
                    self.previous_owner = None;
                }
                // Don't block claiming - just track who previously had the ball
            } else {
                self.previous_owner = None;
            }
        }

        // Priority claim for pass target receiver (larger radius before normal competition)
        if let Some(target_id) = self.pass_target_player_id {
            if let Some(target_player) = players.iter().find(|p| p.id == target_id) {
                let dx = target_player.position.x - self.position.x;
                let dy = target_player.position.y - self.position.y;
                let dist_sq = dx * dx + dy * dy;

                // Matches try_pass_target_claim; see rationale there.
                const RECEIVER_PRIORITY_DISTANCE_SQ: f32 = 32.0 * 32.0;
                const RECEIVER_MAX_HEIGHT: f32 = 2.8;

                if dist_sq < RECEIVER_PRIORITY_DISTANCE_SQ && self.position.z <= RECEIVER_MAX_HEIGHT
                {
                    let passer_id = self.current_owner.or(self.previous_owner);
                    self.previous_owner = self.current_owner;
                    self.current_owner = Some(target_id);
                    self.pass_target_player_id = None;
                    self.ownership_duration = 0;
                    self.claim_cooldown = 15;
                    if let Some(pid) = passer_id.filter(|&id| id != target_id) {
                        events.add_ball_event(BallEvent::PassCompleted(target_id, pid));
                    } else {
                        events.add_ball_event(BallEvent::Claimed(target_id));
                    }
                    return;
                }
            }
        }

        // Velocity thresholds (squared for comparison without sqrt)
        const MAX_CLAIMABLE_VELOCITY_SQ: f32 = 10.0 * 10.0;
        const SLOW_BALL_VELOCITY_SQ: f32 = 4.0 * 4.0;

        let ball_speed_sq = self.velocity.norm_squared();

        // Collect nearby player IDs into a small inline buffer (no heap allocation)
        const MAX_NEARBY: usize = 8;
        let mut nearby_ids: [u32; MAX_NEARBY] = [0; MAX_NEARBY];
        let mut nearby_count: usize = 0;

        let ball_height_reachable = self.position.z <= PLAYER_JUMP_REACH;

        for player in players.iter() {
            let dx = player.position.x - self.position.x;
            let dy = player.position.y - self.position.y;
            let dist_sq = dx * dx + dy * dy;

            if dist_sq > BALL_DISTANCE_THRESHOLD_SQUARED {
                continue;
            }

            if ball_speed_sq <= SLOW_BALL_VELOCITY_SQ {
                if !ball_height_reachable {
                    continue;
                }
            } else if ball_speed_sq > MAX_CLAIMABLE_VELOCITY_SQ {
                if dist_sq > 1.0 {
                    continue;
                }
                if !ball_height_reachable {
                    continue;
                }
            } else {
                if !ball_height_reachable {
                    continue;
                }
            }

            if nearby_count < MAX_NEARBY {
                nearby_ids[nearby_count] = player.id;
                nearby_count += 1;
            }
        }

        // Early exit if no nearby players
        if nearby_count == 0 {
            return;
        }

        let nearby_slice = &nearby_ids[..nearby_count];

        // Check if current owner is nearby
        if let Some(current_owner_id) = self.current_owner {
            let current_owner_nearby = nearby_slice.contains(&current_owner_id);

            if current_owner_nearby {
                let owner_team_id = context.players.by_id(current_owner_id).map(|p| p.team_id);

                let opponent_nearby = owner_team_id.is_some_and(|team_id| {
                    nearby_slice.iter().any(|&id| {
                        context
                            .players
                            .by_id(id)
                            .is_some_and(|p| p.team_id != team_id)
                    })
                });

                if !opponent_nearby {
                    self.ownership_duration += 1;
                    return;
                }
            } else {
                self.previous_owner = self.current_owner;
                self.current_owner = None;
            }
        }

        // Ownership stability constants
        let min_ownership_duration: u32 = if self.contested_claim_count > 3 {
            60
        } else {
            25
        };
        const TAKEOVER_ADVANTAGE_THRESHOLD: f32 = 1.25;

        // Determine the best tackler from nearby players (no Vec allocation)
        let best_tackler = if nearby_count == 1 {
            players.iter().find(|p| p.id == nearby_ids[0])
        } else {
            let mut best: Option<&MatchPlayer> = None;
            let mut best_score: f32 = -1.0;
            for &pid in nearby_slice {
                if let Some(p) = players.iter().find(|p| p.id == pid) {
                    let score = Self::calculate_tackling_score(p);
                    if score > best_score {
                        best_score = score;
                        best = Some(p);
                    }
                }
            }
            best
        };

        // Transfer ownership to the best tackler (with stability checks)
        if let Some(player) = best_tackler {
            // Check if this is a new owner or maintaining current ownership
            let is_ownership_change = self.current_owner.map_or(true, |id| id != player.id);

            if is_ownership_change {
                // Prevent rapid ownership changes by requiring significant advantage
                if self.ownership_duration < min_ownership_duration {
                    if let Some(current_owner_id) = self.current_owner {
                        // Check if current owner is among nearby players
                        if nearby_slice.contains(&current_owner_id) {
                            if let (Some(current_owner_full), Some(challenger_full)) = (
                                context.players.by_id(current_owner_id),
                                context.players.by_id(player.id),
                            ) {
                                let current_score =
                                    Self::calculate_tackling_score(current_owner_full);
                                let challenger_score =
                                    Self::calculate_tackling_score(challenger_full);

                                // Require challenger to be significantly better
                                if challenger_score < current_score * TAKEOVER_ADVANTAGE_THRESHOLD {
                                    // Challenger not strong enough - maintain current ownership
                                    self.ownership_duration += 1;
                                    return;
                                }
                            }
                        }
                    }
                }

                // Ownership change approved - reset duration and set cooldown
                self.previous_owner = self.current_owner;
                self.current_owner = Some(player.id);
                self.pass_target_player_id = None;
                self.ownership_duration = 0;

                // Track contested ownership changes and escalate cooldown
                self.contested_claim_count += 1;
                let cooldown = if self.contested_claim_count > 6 {
                    90 // ~1.5s - force resolution
                } else if self.contested_claim_count > 3 {
                    45 // ~0.75s
                } else {
                    15 // Normal cooldown
                };
                self.claim_cooldown = cooldown;
                // Also set in_flight to prevent ClaimBall events from tackling states
                self.flags.in_flight_state = cooldown as usize;

                events.add_ball_event(BallEvent::Claimed(player.id));
            } else {
                // Same owner - just increment duration
                self.ownership_duration += 1;
                // Gradually decay contested counter when ownership is truly stable
                // Require long stability AND no opponents nearby to consider it resolved
                if self.ownership_duration > 100 && self.contested_claim_count > 0 {
                    self.contested_claim_count = self.contested_claim_count.saturating_sub(1);
                }
            }
        }
    }

    fn calculate_tackling_score(player: &MatchPlayer) -> f32 {
        let technical_skills = &player.skills.technical;
        let mental_skills = &player.skills.mental;
        let physical_skills = &player.skills.physical;

        let tackling_weight = 0.45;
        let aggression_weight = 0.15;
        let bravery_weight = 0.10;
        let strength_weight = 0.20;
        let agility_weight = 0.10;

        technical_skills.tackling * tackling_weight
            + mental_skills.aggression * aggression_weight
            + mental_skills.bravery * bravery_weight
            + physical_skills.strength * strength_weight
            + physical_skills.agility * agility_weight
    }
}

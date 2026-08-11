//! Position-anchor stall detector. Catches the case where the ball
//! ping-pongs in a small region (each ownership flip resets the
//! owned/unowned counters but the ball physically goes nowhere). The
//! anchor advances naturally during normal play; only a genuinely
//! stuck region trips the safety net and force-kicks the ball clear.

use super::Ball;
use crate::r#match::{MatchPlayer, PlayerSide};
use nalgebra::Vector3;

/// Who the ball dies on.
///
/// A stalled ball is nearly always a state machine with no way to act on
/// possession: the player owns the ball, his state has no `has_ball`
/// exit (or returns "stay put"), his `velocity()` is zero, and the same
/// evaluation runs again next tick. It is a fixed point, and the only
/// thing that ends it is `detect_position_stall` force-kicking the ball
/// clear — 1000 ticks later.
///
/// Three of those were found by hand off a replay dump, which does not
/// scale: by the time the obvious ones are fixed the rest are too rare to
/// catch in a handful of matches. This attributes every stuck tick to the
/// state that was holding the ball, so the next one shows up as a row in
/// a 200-match table instead of a needle in a replay.
///
/// TICKS ARE FULL TICKS (~20 ms) — `detect_position_stall` runs from
/// `Ball::update` only, never `update_light`. That is also why the
/// safety net fires at ~19.5 s rather than the 10 s its own constant
/// comment claims.
#[cfg(feature = "match-logs")]
pub mod dead_ball_diag {
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Widest `PlayerState::compact_id()` plus headroom (forwards top
    /// out at 418 today).
    pub const STATES: usize = 448;
    const ZERO: AtomicU64 = AtomicU64::new(0);

    /// Full ticks the ball spent stuck while a player in this state held
    /// it, and how many separate episodes that was.
    pub static STUCK_TICKS_BY_STATE: [AtomicU64; STATES] = [ZERO; STATES];
    pub static STUCK_EPISODES_BY_STATE: [AtomicU64; STATES] = [ZERO; STATES];
    /// Same, for a stall nobody owned — a loose ball sitting untouched.
    pub static STUCK_TICKS_UNOWNED: AtomicU64 = AtomicU64::new(0);
    pub static STUCK_EPISODES_UNOWNED: AtomicU64 = AtomicU64::new(0);
    /// Longest single stall seen, in full ticks.
    pub static LONGEST_STUCK: AtomicU64 = AtomicU64::new(0);

    pub fn reset() {
        for c in STUCK_TICKS_BY_STATE
            .iter()
            .chain(STUCK_EPISODES_BY_STATE.iter())
        {
            c.store(0, Ordering::Relaxed);
        }
        for c in [
            &STUCK_TICKS_UNOWNED,
            &STUCK_EPISODES_UNOWNED,
            &LONGEST_STUCK,
        ] {
            c.store(0, Ordering::Relaxed);
        }
    }

    /// `(rows of (compact_id, ticks, episodes), unowned ticks, unowned
    /// episodes, longest stall in ticks)`. Rows are only the states that
    /// actually stalled, so the caller can print a short table.
    pub fn snapshot() -> (Vec<(u16, u64, u64)>, u64, u64, u64) {
        let mut rows = Vec::new();
        for id in 0..STATES {
            let t = STUCK_TICKS_BY_STATE[id].load(Ordering::Relaxed);
            if t > 0 {
                rows.push((
                    id as u16,
                    t,
                    STUCK_EPISODES_BY_STATE[id].load(Ordering::Relaxed),
                ));
            }
        }
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        (
            rows,
            STUCK_TICKS_UNOWNED.load(Ordering::Relaxed),
            STUCK_EPISODES_UNOWNED.load(Ordering::Relaxed),
            LONGEST_STUCK.load(Ordering::Relaxed),
        )
    }
}

impl Ball {
    /// Position-based stall: the ball hasn't left a small region in N
    /// ticks, regardless of who owns it. Catches the case where
    /// ownership rapidly flips between teammates (each flip resets
    /// owned/unowned counters) but the ball physically stays put.
    /// The anchor resets whenever the ball travels outside the radius,
    /// so normal play keeps advancing the anchor every few ticks.
    pub(super) fn detect_position_stall(&mut self, players: &[MatchPlayer]) {
        // Raised thresholds so normal possession play doesn't trigger.
        // A team can legitimately keep the ball in a 15-unit zone for
        // 8-10 seconds during sideline passing or defensive possession;
        // 1000 ticks = 10 sec is the floor for "genuinely stuck".
        const STALL_RADIUS: f32 = 15.0;
        const STALL_RADIUS_SQ: f32 = STALL_RADIUS * STALL_RADIUS;
        const STALL_TICKS: u32 = 1000;

        let ball_xy = Vector3::new(self.position.x, self.position.y, 0.0);
        let anchor_xy = Vector3::new(self.stall_anchor_pos.x, self.stall_anchor_pos.y, 0.0);
        let drift_sq = (ball_xy - anchor_xy).norm_squared();

        if drift_sq > STALL_RADIUS_SQ {
            self.stall_anchor_pos = self.position;
            self.stall_anchor_tick = 0;
            return;
        }

        self.stall_anchor_tick += 1;

        // Attribute the stall long before the safety net fires. Inside a
        // 15u (1.9 m) circle for five seconds is not build-up play — the
        // ball is stuck, and whoever is standing on it is the bug.
        #[cfg(feature = "match-logs")]
        {
            use std::sync::atomic::Ordering;
            const DEAD_AFTER: u32 = 250;
            if self.stall_anchor_tick >= DEAD_AFTER {
                let first = self.stall_anchor_tick == DEAD_AFTER;
                let bucket = self
                    .current_owner
                    .and_then(|id| players.iter().find(|p| p.id == id))
                    .map(|p| p.state.compact_id() as usize)
                    .filter(|b| *b < dead_ball_diag::STATES);
                match bucket {
                    Some(b) => {
                        dead_ball_diag::STUCK_TICKS_BY_STATE[b].fetch_add(1, Ordering::Relaxed);
                        if first {
                            dead_ball_diag::STUCK_EPISODES_BY_STATE[b]
                                .fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    None => {
                        dead_ball_diag::STUCK_TICKS_UNOWNED.fetch_add(1, Ordering::Relaxed);
                        if first {
                            dead_ball_diag::STUCK_EPISODES_UNOWNED.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                dead_ball_diag::LONGEST_STUCK
                    .fetch_max(self.stall_anchor_tick as u64, Ordering::Relaxed);
            }
        }

        if self.stall_anchor_tick == STALL_TICKS {
            #[cfg(feature = "match-logs")]
            {
                let owner_str = self
                    .current_owner
                    .map(|id| format!("Some({})", id))
                    .unwrap_or_else(|| "None".to_string());
                let owner_state = self
                    .current_owner
                    .and_then(|id| players.iter().find(|p| p.id == id))
                    .map(|p| format!("{:?}", p.state))
                    .unwrap_or_else(|| "-".to_string());
                crate::match_log_debug!(
                    "ball position-stall: stayed within {}u of ({:.1}, {:.1}) for {} ticks — owner={} state={} ball_vel=({:.2}, {:.2})",
                    STALL_RADIUS,
                    self.stall_anchor_pos.x,
                    self.stall_anchor_pos.y,
                    STALL_TICKS,
                    owner_str,
                    owner_state,
                    self.velocity.x,
                    self.velocity.y,
                );
            }
            // Force-kick out of the zone. Previous attempts with a
            // small push got immediately re-claimed by the same player
            // in `process_ownership` the SAME tick — ball never
            // escaped the 12-unit radius. Solution: kick harder AND
            // set `in_flight_state` so normal ownership checks are
            // suppressed long enough for the ball to actually leave.
            let owner_side = self
                .current_owner
                .and_then(|id| players.iter().find(|p| p.id == id))
                .and_then(|p| p.side);
            let push_x: f32 = match owner_side {
                Some(PlayerSide::Left) => 7.0,
                Some(PlayerSide::Right) => -7.0,
                _ => 7.0,
            };
            self.velocity = Vector3::new(push_x, 0.0, 1.5);
            self.previous_owner = self.current_owner;
            self.current_owner = None;
            self.ownership_duration = 0;
            self.claim_cooldown = 0;
            // 40 ticks of protected flight — matches a short pass,
            // long enough for the ball to clear the stall radius.
            self.flags.in_flight_state = 40;
            self.pass_target_player_id = None;
            self.owned_stuck_ticks = 0;
            self.owned_stuck_logged = false;
            self.stall_anchor_tick = 0;
            // Teleport anchor so post-release ball travel advances
            // the anchor naturally instead of re-triggering.
            self.stall_anchor_pos = self.position;
        }
    }

    /// Only the stall-resolution log (match-logs builds) consumes the
    /// snapshot — production builds never call this (the capture site in
    /// `force_takeball_if_unowned_too_long` is feature-gated). `write!`
    /// straight into the output buffer: the previous `push_str(&format!)`
    /// pattern built ~23 intermediate Strings per capture, and captures
    /// fire on every owned→unowned transition.
    #[cfg(feature = "match-logs")]
    pub(super) fn format_stall_snapshot(&self, players: &[MatchPlayer]) -> String {
        use std::fmt::Write;

        let mut out = String::with_capacity(2048);
        let _ = write!(
            out,
            "  ball pos=({:.1}, {:.1}, {:.1}) velocity=({:.2}, {:.2}, {:.2}) in_flight={} previous_owner={:?}",
            self.position.x,
            self.position.y,
            self.position.z,
            self.velocity.x,
            self.velocity.y,
            self.velocity.z,
            self.flags.in_flight_state,
            self.previous_owner,
        );
        for p in players {
            if p.is_sent_off {
                continue;
            }
            let _ = write!(
                out,
                "\n  id={} team={} pos=({:.1}, {:.1}) vel=({:.2}, {:.2}) state={} tactical={:?}",
                p.id,
                p.team_id,
                p.position.x,
                p.position.y,
                p.velocity.x,
                p.velocity.y,
                p.state,
                p.tactical_position.current_position,
            );
        }
        out
    }
}

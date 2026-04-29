//! Position-anchor stall detector. Catches the case where the ball
//! ping-pongs in a small region (each ownership flip resets the
//! owned/unowned counters but the ball physically goes nowhere). The
//! anchor advances naturally during normal play; only a genuinely
//! stuck region trips the safety net and force-kicks the ball clear.

use super::Ball;
use crate::r#match::{MatchPlayer, PlayerSide};
use nalgebra::Vector3;

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

        if self.stall_anchor_tick == STALL_TICKS {
            #[cfg(feature = "match-logs")]
            {
                let owner_str = self.current_owner
                    .map(|id| format!("Some({})", id))
                    .unwrap_or_else(|| "None".to_string());
                let owner_state = self.current_owner
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
            let owner_side = self.current_owner
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

    pub(super) fn format_stall_snapshot(&self, players: &[MatchPlayer]) -> String {
        let mut out = String::with_capacity(2048);
        out.push_str(&format!(
            "  ball pos=({:.1}, {:.1}, {:.1}) velocity=({:.2}, {:.2}, {:.2}) in_flight={} previous_owner={:?}",
            self.position.x, self.position.y, self.position.z,
            self.velocity.x, self.velocity.y, self.velocity.z,
            self.flags.in_flight_state,
            self.previous_owner,
        ));
        for p in players {
            if p.is_sent_off {
                continue;
            }
            out.push_str(&format!(
                "\n  id={} team={} pos=({:.1}, {:.1}) vel=({:.2}, {:.2}) state={} tactical={:?}",
                p.id,
                p.team_id,
                p.position.x, p.position.y,
                p.velocity.x, p.velocity.y,
                p.state,
                p.tactical_position.current_position,
            ));
        }
        out
    }
}

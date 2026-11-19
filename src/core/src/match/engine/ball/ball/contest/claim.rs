//! **The ball's half of a goalkeeper's claim.** The aerial contest decides
//! a claim once (`FootballEngine::resolve_keeper_claim`). What the decision
//! does to the ball lives here: a ball he punches waits at his fist for the
//! punch, and one he flaps at drops loose where the box can get to it.

use crate::r#match::engine::ball::ball::{Ball, BallRoll};
use crate::r#match::{ClaimOutcome, MatchPlayer, PlayerSide};
use nalgebra::Vector3;

/// A claim the contest decided as a cross was delivered, before the ball
/// reached the keeper coming for it. Carried out when it does.
#[derive(Debug, Clone, Copy)]
pub struct KeeperClaimCall {
    pub keeper_id: u32,
    pub outcome: ClaimOutcome,
}

impl Ball {
    /// His fist has met it. It waits there, his, for the punch his state
    /// throws next.
    pub fn hold_for_punch(&mut self, keeper_id: u32) {
        self.velocity = Vector3::new(0.0, 0.0, self.velocity.z.min(0.0));
        self.aerial_contest_winner = Some(keeper_id);
        self.pass_target_player_id = None;
    }

    /// He got to it and lost it. The ball has to end up where an attacker
    /// can reach it: a spill that dies at the keeper's feet is one he picks
    /// straight up again, so it is pushed `out` units away from his goal and
    /// `lateral` across, around the six-yard line and the penalty spot where
    /// the box contest happens, and comes to rest there. His is the last
    /// touch, so a shot from it is charged to him.
    pub fn flap_off(&mut self, keeper: &MatchPlayer, out: f32, lateral: f32) {
        let away = match keeper.side {
            Some(PlayerSide::Left) => out,
            Some(PlayerSide::Right) => -out,
            None => 0.0,
        };
        let push = Vector3::new(away, lateral, 0.0);
        let distance = push.norm().max(1.0);
        let speed = BallRoll::speed_to_rest_at(distance);
        self.velocity.x = push.x / distance * speed;
        self.velocity.y = push.y / distance * speed;
        self.sink_to_ground();
        self.current_owner = None;
        self.previous_owner = Some(keeper.id);
        self.pass_target_player_id = None;
        self.pending_pass_passer = None;
        self.pending_pass_was_cross = false;
        self.flags.in_flight_state = 10;
        self.claim_cooldown = 0;
        let tick = self.current_tick_cached;
        self.last_rebound_tick = tick;
        self.pending_failed_claim_gk_id = Some(keeper.id);
        self.pending_failed_claim_tick = tick;
        self.pending_failed_claim_charged = false;
        self.record_touch(keeper.id, keeper.team_id, tick, false);
    }
}

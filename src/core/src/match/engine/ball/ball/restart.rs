//! Restart resolution: throw-ins on the touchlines and the lifetime
//! management of the in-flight offside snapshot. Endline restarts
//! (corner / goal kick) live in `goal.rs` because they share machinery
//! with the actual goal check.
//!
//! Real football rule the ball is restarted by the team that did NOT
//! last touch the ball. If we have no record of a touch (kickoff /
//! genuine bug) we fall back to the team currently holding ownership;
//! if even that is missing, we leave the boundary inset as the safety
//! net for `check_boundary_collision`.

use super::Ball;
use crate::PlayerFieldPositionGroup;
use crate::r#match::ball::events::BallEvent;
use crate::r#match::events::EventCollection;
use crate::r#match::{MatchContext, MatchPlayer, PlayerSide};
use nalgebra::Vector3;

impl Ball {
    /// Touchline check: if the ball crossed y<=0 or y>=field_height, set
    /// up a throw-in for the team that did NOT last touch it. Routes
    /// through `pending_set_piece_teleport` like corners / goal kicks so
    /// the engine can place the thrower onto the ball after the tick.
    pub(super) fn check_throw_in(
        &mut self,
        context: &MatchContext,
        players: &[MatchPlayer],
        events: &mut EventCollection,
    ) {
        // Already resolved this tick (goal scored, etc.).
        if self.goal_scored {
            return;
        }
        let field_height = context.field_size.height as f32;
        let crossed_top = self.position.y <= 0.0;
        let crossed_bottom = self.position.y >= field_height;
        if !crossed_top && !crossed_bottom {
            return;
        }

        // Last toucher's side decides which team gets the throw-in.
        let last_toucher_side = self
            .last_touch_player_id
            .or(self.previous_owner)
            .or(self.current_owner)
            .and_then(|pid| players.iter().find(|p| p.id == pid))
            .and_then(|p| p.side);

        let throwing_side = match last_toucher_side {
            Some(PlayerSide::Left) => PlayerSide::Right,
            Some(PlayerSide::Right) => PlayerSide::Left,
            None => return, // Safety net: let boundary_collision handle it.
        };

        // Inset the throw-in slightly inside the touchline so subsequent
        // physics doesn't immediately re-trigger a boundary cross.
        let field_width = context.field_size.width as f32;
        let throw_x = self.position.x.clamp(2.0, field_width - 2.0);
        let throw_y = if crossed_top { 2.0 } else { field_height - 2.0 };
        let throw_pos = Vector3::new(throw_x, throw_y, 0.0);

        let thrower = pick_thrower(players, throwing_side, throw_pos);
        let thrower_id = match thrower {
            Some(id) => id,
            None => return,
        };

        self.position = throw_pos;
        self.velocity = Vector3::zeros();

        self.previous_owner = self.current_owner;
        self.current_owner = Some(thrower_id);
        self.ownership_duration = 0;
        self.claim_cooldown = 45;
        self.flags.in_flight_state = 20;
        self.pass_target_player_id = None;
        self.recent_passers.clear();
        self.cached_shot_target = None;
        self.offside_snapshot = None;
        self.pass_origin_restart = crate::r#match::PassOriginRestart::ThrowIn;

        let team_id = players
            .iter()
            .find(|p| p.id == thrower_id)
            .map(|p| p.team_id)
            .unwrap_or(0);
        self.record_touch(thrower_id, team_id, context.current_tick(), true);

        self.pending_set_piece_teleport = Some((thrower_id, throw_pos));
        events.add_ball_event(BallEvent::Claimed(thrower_id));
    }

    /// Drop the offside snapshot once its lifetime expires. Real-world
    /// passes that don't reach a receiver should end the offside
    /// pretence — anything older than ~220 ticks is stale.
    pub(super) fn expire_offside_snapshot(&mut self, context: &MatchContext) {
        const OFFSIDE_LIFETIME_TICKS: u64 = 220;
        if let Some(snap) = self.offside_snapshot {
            let now = context.current_tick();
            if now.saturating_sub(snap.set_tick) > OFFSIDE_LIFETIME_TICKS {
                self.offside_snapshot = None;
                if self.pass_origin_restart != crate::r#match::PassOriginRestart::OpenPlay {
                    self.pass_origin_restart = crate::r#match::PassOriginRestart::OpenPlay;
                }
            }
        }
    }
}

/// Score-based thrower selection.
///   0.35 distance (closer to throw point is better)
///   0.25 position fit (fullback / wing-back / wide mid preferred)
///   0.20 long_throws scaled
///   0.10 decisions scaled
///   0.05 technique scaled
///   0.05 strength scaled
fn pick_thrower(
    players: &[MatchPlayer],
    throwing_side: PlayerSide,
    throw_pos: Vector3<f32>,
) -> Option<u32> {
    let mut best: Option<(u32, f32)> = None;
    // Furthest-search radius — a player 200u from the touchline isn't
    // realistically the thrower. We still score them; this just bounds
    // the normalisation.
    const MAX_REASONABLE_DISTANCE: f32 = 200.0;

    for p in players {
        if p.side != Some(throwing_side) {
            continue;
        }
        if p.is_sent_off {
            continue;
        }
        if p.tactical_position.current_position.position_group()
            == PlayerFieldPositionGroup::Goalkeeper
        {
            continue;
        }
        let dx = p.position.x - throw_pos.x;
        let dy = p.position.y - throw_pos.y;
        let dist = (dx * dx + dy * dy).sqrt();
        let dist_score = (1.0 - (dist / MAX_REASONABLE_DISTANCE).min(1.0)).max(0.0);

        // Position fit: defenders + midfielders get a small bump for
        // throw-ins (fullbacks / wide mids most often take throws). The
        // exact PlayerPositionType subdivision (LB/RB/LM/RM) lives one
        // crate boundary away, so we lean on the position-group buckets
        // that are visible here.
        let position_fit = match p.tactical_position.current_position.position_group() {
            PlayerFieldPositionGroup::Defender => 1.0,
            PlayerFieldPositionGroup::Midfielder => 0.8,
            PlayerFieldPositionGroup::Forward => 0.5,
            PlayerFieldPositionGroup::Goalkeeper => 0.0,
        };

        let long_throws = (p.skills.technical.long_throws / 20.0).clamp(0.0, 1.0);
        let decisions = (p.skills.mental.decisions / 20.0).clamp(0.0, 1.0);
        let technique = (p.skills.technical.technique / 20.0).clamp(0.0, 1.0);
        let strength = (p.skills.physical.strength / 20.0).clamp(0.0, 1.0);

        let score = dist_score * 0.35
            + position_fit * 0.25
            + long_throws * 0.20
            + decisions * 0.10
            + technique * 0.05
            + strength * 0.05;

        let candidate = (p.id, score);
        match best {
            None => best = Some(candidate),
            Some((_, best_score)) if score > best_score => best = Some(candidate),
            _ => {}
        }
    }
    best.map(|(id, _)| id)
}

/// Throw-in delivery range, in field units. Used by tactical states that
/// pick a target from a throw-in.
///
/// 35u baseline + up to 35u extra at long_throws=20. Minimum useful
/// range 12u — anything shorter is a recycle pass.
#[allow(dead_code)]
pub fn throw_in_range(long_throws_skill: f32) -> (f32, f32) {
    let scaled = (long_throws_skill / 20.0).clamp(0.0, 1.0);
    let max_range = 35.0 + scaled * 35.0;
    (12.0, max_range)
}

/// Whether the player can deliver a "long throw into the box" — only
/// allowed in the attacking third with a strong long_throws skill.
#[allow(dead_code)]
pub fn can_long_throw_into_box(long_throws_skill: f32, in_attacking_third: bool) -> bool {
    in_attacking_third && long_throws_skill >= 14.0
}

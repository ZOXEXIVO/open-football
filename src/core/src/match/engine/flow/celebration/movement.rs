//! How a celebrating player is actually moved: the steering step, the
//! bound that keeps him on (or just behind) the pitch, and the stable
//! per-player offset that gives a pile-on a shape.
//!
//! Split out from the choreography because it is the only part of the
//! window that touches a player's coordinates directly, and because the
//! offset must stay a hash rather than a draw — see the module note on
//! the shared RNG stream.

use super::choreography::GoalCelebration;
use crate::r#match::MatchPlayer;
use crate::r#match::engine::ball::ball::net::GoalNet;
use nalgebra::Vector3;

impl GoalCelebration {
    /// Nobody walks through a touchline, even in a celebration.
    const TOUCHLINE_MARGIN: f32 = 12.0;

    /// Move `player` toward `target` at `speed` units per tick, and let
    /// whatever leap he is in the middle of run its course.
    pub(in crate::r#match::engine::flow) fn steer(
        player: &mut MatchPlayer,
        target: Vector3<f32>,
        speed: f32,
        field_width: f32,
        field_height: f32,
    ) {
        let (height, rise) = MatchPlayer::fall(player.height, player.vertical_speed);
        player.height = height;
        player.vertical_speed = rise;

        let dx = target.x - player.position.x;
        let dy = target.y - player.position.y;
        let distance = (dx * dx + dy * dy).sqrt();
        if speed <= 0.0 || distance < 0.05 {
            player.velocity = Vector3::zeros();
            return;
        }

        let step = speed.min(distance);
        let (ux, uy) = (dx / distance, dy / distance);
        player.velocity = Vector3::new(ux * step, uy * step, 0.0);
        player.position.x += ux * step;
        player.position.y += uy * step;

        // A celebration can take a player behind the goal line — that is
        // where the corner flag is, and it is where the ball is — but not
        // off the pitch entirely. The bound is the back of the BAGGED net
        // rather than the net's nominal depth: a ball driven in hard sits
        // inside the mesh's give, and a keeper who can only reach the back
        // bar can never quite get to it.
        let reach_behind = GoalNet::DEPTH + GoalNet::GIVE_BACK;
        #[cfg(feature = "match-logs")]
        let before = player.position;
        player.position.x = player
            .position
            .x
            .clamp(-reach_behind, field_width + reach_behind);
        player.position.y = player.position.y.clamp(
            Self::TOUCHLINE_MARGIN,
            field_height - Self::TOUCHLINE_MARGIN,
        );
        // The celebration's own boundary, booked against the same
        // `boundary_clamp` control row as the match one: it can only undo
        // the step that took him out, so it should never appear either.
        #[cfg(feature = "match-logs")]
        crate::r#match::engine::ball::ball::teleport::PlayerTeleportCensus::note(
            crate::r#match::engine::ball::ball::teleport::PSITE_BOUNDARY,
            before,
            player.position,
        );
    }

    /// A stable per-player offset direction, so the pile-on has a shape.
    ///
    /// A hash of the id rather than a draw from the match RNG: the stream is
    /// shared with every calibrated roll in the engine and a celebration must
    /// not consume from it.
    pub(in crate::r#match::engine::flow) fn spread(player_id: u32, index: usize) -> Vector3<f32> {
        let mixed = player_id
            .wrapping_mul(2_654_435_761)
            .wrapping_add(index as u32);
        let angle = (mixed % 3600) as f32 * (std::f32::consts::TAU / 3600.0);
        let radius = 0.5 + ((mixed >> 12) % 100) as f32 / 100.0;
        Vector3::new(angle.cos() * radius, angle.sin() * radius, 0.0)
    }
}

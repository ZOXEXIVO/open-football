use crate::PlayerFieldPositionGroup;
use crate::r#match::ball::events::GoalSide;
use crate::r#match::field::MatchField;
use crate::r#match::{MatchContext, PlayerSide};
use nalgebra::Vector3;

use super::engine::MatchFieldSize;

pub const GOAL_WIDTH: f32 = 29.0; // half-width in game units (full goal = 58 units, real = 7.32m)
pub const GOAL_HEIGHT: f32 = 2.44; // Crossbar height in meters (z-axis is in meters)

#[derive(Clone)]
pub struct GoalPosition {
    pub left: Vector3<f32>,
    pub right: Vector3<f32>,
}

impl From<&MatchFieldSize> for GoalPosition {
    fn from(value: &MatchFieldSize) -> Self {
        // Left goal at x = 0, centered on width
        let left_goal = Vector3::new(0.0, value.height as f32 / 2.0, 0.0);

        // Right goal at x = length, centered on width
        let right_goal = Vector3::new(value.width as f32, (value.height / 2usize) as f32, 0.0);

        GoalPosition {
            left: left_goal,
            right: right_goal,
        }
    }
}

impl GoalPosition {
    pub fn is_goal(&self, ball_position: Vector3<f32>) -> Option<GoalSide> {
        if ball_position.z > GOAL_HEIGHT {
            return None;
        }
        self.check_goal_line(ball_position)
    }

    /// Check if ball crossed the goal line within goal width but ABOVE the crossbar.
    /// Returns which side the ball went over (goal kick for the defending team).
    pub fn is_over_goal(&self, ball_position: Vector3<f32>) -> Option<GoalSide> {
        if ball_position.z <= GOAL_HEIGHT {
            return None;
        }
        self.check_goal_line(ball_position)
    }

    fn check_goal_line(&self, ball_position: Vector3<f32>) -> Option<GoalSide> {
        if ball_position.x <= self.left.x {
            if (self.left.y - GOAL_WIDTH..=self.left.y + GOAL_WIDTH).contains(&ball_position.y) {
                return Some(GoalSide::Home);
            }
        }

        if ball_position.x >= self.right.x {
            if (self.right.y - GOAL_WIDTH..=self.right.y + GOAL_WIDTH).contains(&ball_position.y) {
                return Some(GoalSide::Away);
            }
        }

        None
    }
}

/// Place an outfield player from `side` on the centre spot and give
/// them protected possession. Used by every restart that puts the
/// ball on the centre circle — goals, match start, halftime, start of
/// extra time. Without this, `reset_players_positions` leaves the
/// whole squad at formation start and the ball sits with no claimant
/// — once `in_flight_state` expires nobody is close enough to keep
/// it, ownership gets nulled, and the period stalls for ~14 seconds
/// until the emergency chaser-override fires.
pub fn assign_kickoff(field: &mut MatchField, side: PlayerSide) {
    let ball_pos = field.ball.position;
    let kickoff_player_id = field
        .players
        .iter()
        .filter(|p| p.side == Some(side))
        .filter(|p| {
            p.tactical_position.current_position.position_group()
                != PlayerFieldPositionGroup::Goalkeeper
        })
        .min_by(|a, b| {
            let da = (a.position - ball_pos).norm_squared();
            let db = (b.position - ball_pos).norm_squared();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|p| p.id);

    if let Some(player_id) = kickoff_player_id {
        if let Some(kicker) = field.players.iter_mut().find(|p| p.id == player_id) {
            kicker.position = ball_pos;
            kicker.velocity = Vector3::zeros();
            kicker.set_default_state();
            kicker.in_state_time = 0;
        }
        field.ball.current_owner = Some(player_id);
        // Short ping-pong guard only — the kicker needs to take the
        // ball forward, not hold on to it for 1.2 s while the whole
        // pack watches. A 30-tick cooldown is enough to stop the
        // ownership logic from immediately ripping the ball back out
        // of their feet and falls away by the time the state machine
        // decides to pass.
        field.ball.claim_cooldown = 30;
        field.ball.flags.in_flight_state = 0;
        field.ball.contested_claim_count = 0;
    }
}

/// Reset field after a goal: reposition players, assign kickoff possession.
pub fn handle_goal_reset(field: &mut MatchField, context: &mut MatchContext) {
    if !field.ball.goal_scored {
        return;
    }

    let kickoff_side = field.ball.kickoff_team_side;

    field.reset_players_positions();
    field.ball.reset();

    if let Some(side) = kickoff_side {
        assign_kickoff(field, side);
    }

    field.ball.goal_scored = false;
    field.ball.kickoff_team_side = None;
    context.record_goal_tick();
}

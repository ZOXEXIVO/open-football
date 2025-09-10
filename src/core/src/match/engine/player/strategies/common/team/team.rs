use nalgebra::Vector3;
use crate::{PlayerFieldPositionGroup, Tactics};
use crate::r#match::{MatchContext, MatchPlayerLite, PlayerSide, StateProcessingContext};

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
        match self.ctx.player.side {
            Some(PlayerSide::Left) => &self.ctx.context.tactics.left,
            Some(PlayerSide::Right) => &self.ctx.context.tactics.right,
            None => {
                panic!("unknown player side")
            }
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
                    if ball_velocity.magnitude() > 1.0 {
                        // Determine which way is "forward" based on team side
                        let forward_direction = match self.ctx.player.side {
                            Some(PlayerSide::Left) => Vector3::new(1.0, 0.0, 0.0),  // Left team attacks right
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

        let closest_teammate_dist = self.ctx.players().teammates().all()
            .map(|p| (p.position - ball_pos).magnitude())
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let closest_opponent_dist = self.ctx.players().opponents().all()
            .map(|p| (p.position - ball_pos).magnitude())
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // If a teammate is significantly closer to the ball than any opponent
        if let (Some(team_dist), Some(opp_dist)) = (closest_teammate_dist, closest_opponent_dist) {
            if team_dist < opp_dist * 0.7 { // Teammate is at least 30% closer
                return true;
            }
        }

        false
    }

    pub fn is_leading(&self) -> bool {
        !self.is_loosing()
    }

    pub fn is_loosing(&self) -> bool {
        if self.ctx.player.team_id == self.ctx.context.score.home_team.team_id {
            self.ctx.context.score.home_team < self.ctx.context.score.away_team
        } else {
            self.ctx.context.score.away_team < self.ctx.context.score.home_team
        }
    }

    pub fn is_teammate_chasing_ball(&self) -> bool {
        let ball_position = self.ctx.tick_context.positions.ball.position;

        self.ctx
            .players()
            .teammates()
            .all()
            .any(|player| {
                // Check if player is heading toward the ball
                let player_position = self.ctx.tick_context.positions.players.position(player.id);
                let player_velocity = self.ctx.tick_context.positions.players.velocity(player.id);

                if player_velocity.magnitude() < 0.1 {
                    return false;
                }

                let direction_to_ball = (ball_position - player_position).normalize();
                let player_direction = player_velocity.normalize();
                let dot_product = direction_to_ball.dot(&player_direction);

                // Player is moving toward the ball
                dot_product > 0.85 &&
                    // And is closer or has better position
                    (ball_position - player_position).magnitude() <
                        (ball_position - self.ctx.player.position).magnitude() * 1.2
            })
    }

    // Determine if this player is the best positioned to chase the ball
    pub fn is_best_player_to_chase_ball(&self) -> bool {
        let ball_position = self.ctx.tick_context.positions.ball.position;

        // Don't chase the ball if a teammate already has it
        if let Some(owner_id) = self.ctx.ball().owner_id() {
            if let Some(owner) = self.ctx.context.players.by_id(owner_id) {
                if owner.team_id == self.ctx.player.team_id {
                    // A teammate has the ball, don't try to take it
                    return false;
                }
            }
        }

        // Check if the player is already the closest to the ball on their team
        // Calculate player's "ball-chasing score" based on distance, position, and attributes
        let calculate_score = |player: &MatchPlayerLite, context: &MatchContext| -> f32 {
            let pos = self.ctx.tick_context.positions.players.position(player.id);
            let dist = (ball_position - pos).magnitude();

            let player_ops = self.ctx.player();

            let player = player_ops.get(player.id);
            let skills = player_ops.skills(player.id);

            let pace_factor = skills.physical.pace / 20.0;
            let acceleration_factor = skills.physical.acceleration / 20.0;
            let position_factor = match player.tactical_positions.position_group() {
                // Forwards and midfielders are more likely to chase the ball
                PlayerFieldPositionGroup::Forward => 1.2,
                PlayerFieldPositionGroup::Midfielder => 1.1,
                PlayerFieldPositionGroup::Defender => 0.9,
                PlayerFieldPositionGroup::Goalkeeper => 0.5,
            };

            // Lower score is better
            dist * (1.0 / (pace_factor * acceleration_factor * position_factor * 0.5 + 0.5))
        };

        let player_score = calculate_score(&self.ctx.player.into(), &self.ctx.context);

        // Compare against other teammates
        !self.ctx
            .players()
            .teammates()
            .all()
            .any(|player| calculate_score(&player, &self.ctx.context) < player_score * 0.8) // 20% threshold to avoid constant switching
    }
}

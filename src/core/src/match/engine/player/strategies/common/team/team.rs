use crate::Tactics;
use crate::r#match::{PlayerSide, StateProcessingContext};

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

        let ball_owner: Option<u32> = {
            if let Some(owner_id) = self.ctx.ball().owner_id() {
                Some(owner_id)
            } else {
                self.ctx.ball().previous_owner_id()
            }
        };

        if let Some(owner_id) = ball_owner {
            if let Some(ball_owner) = self.ctx.context.players.by_id(owner_id) {
                return ball_owner.team_id == current_player_team_id;
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

        // Calculate player's "ball-chasing score" based on distance, position, and attributes
        let calculate_score = |player: &crate::r#match::MatchPlayerLite| -> f32 {
            let pos = self.ctx.tick_context.positions.players.position(player.id);
            let dist = (ball_position - pos).magnitude();

            let player = self.ctx.player();

            let skills = player.skills(self.ctx.player.id);

            let pace_factor = skills.physical.pace / 20.0;
            let acceleration_factor = skills.physical.acceleration / 20.0;

            // Lower score is better
            dist * (1.0 / (pace_factor * acceleration_factor * 0.5 + 0.5))
        };

        let player_score = calculate_score(&self.ctx.player.into());

        // Compare against other teammates
        !self.ctx
            .players()
            .teammates()
            .all()
            .any(|player| calculate_score(&player) < player_score * 0.9) // 10% threshold to avoid constant switching
    }
}

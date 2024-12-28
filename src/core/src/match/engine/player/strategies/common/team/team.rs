use std::fmt::Pointer;
use crate::r#match::{PlayerSide, StateProcessingContext};
use crate::Tactics;

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
        match self.ctx.player.side  {
            Some(PlayerSide::Left) => {
                &self.ctx.context.tactics.left
            },
            Some(PlayerSide::Right) => {
                &self.ctx.context.tactics.right
            }
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

    fn get_home_team_score(&self) -> u8 {
        if self.ctx.player.team_id == self.ctx.context.score.home_team.team_id {
            self.ctx.context.score.home_team.get()
        } else {
            self.ctx.context.score.away_team.get()
        }
    }

    fn get_away_score(&self) -> u8 {
        if self.ctx.player.team_id == self.ctx.context.score.home_team.team_id {
            self.ctx.context.score.away_team.get()
        } else {
            self.ctx.context.score.home_team.get()
        }
    }
}

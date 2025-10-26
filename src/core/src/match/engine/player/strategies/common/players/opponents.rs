use crate::r#match::{MatchPlayerLite, StateProcessingContext};
use crate::PlayerFieldPositionGroup;

pub struct PlayerOpponentsOperationsImpl<'b> {
    ctx: &'b StateProcessingContext<'b>,
}

impl<'b> PlayerOpponentsOperationsImpl<'b> {
    pub fn new(ctx: &'b StateProcessingContext<'b>) -> Self {
        PlayerOpponentsOperationsImpl { ctx }
    }
}

impl<'b> PlayerOpponentsOperationsImpl<'b> {
    pub fn all(&'b self) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.opponents_for_team(self.ctx.player.id, self.ctx.player.team_id, None)
    }

    pub fn with_ball(&'b self) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.opponents_for_team(self.ctx.player.id, self.ctx.player.team_id, Some(true))
    }

    pub fn without_ball(&'b self) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.opponents_for_team(self.ctx.player.id, self.ctx.player.team_id, Some(false))
    }

    pub fn nearby(&self, distance: f32) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.ctx
            .tick_context
            .distances
            .opponents(self.ctx.player.id, distance)
            .map(|(pid, _)| MatchPlayerLite {
                id: pid,
                position: self.ctx.tick_context.positions.players.position(pid),
                tactical_positions: self.ctx.context.players.by_id(pid).expect(&format!(
                    "unknown player = {}", pid
                )).tactical_position.current_position
            })
    }

    pub fn nearby_raw(&self, distance: f32) -> impl Iterator<Item = (u32, f32)> + 'b {
        self.ctx
            .tick_context
            .distances
            .opponents(self.ctx.player.id, distance)
    }

    pub fn exists(&self, distance: f32) -> bool {
        self.ctx
            .tick_context
            .distances
            .opponents(self.ctx.player.id, distance)
            .any(|_| true)
    }

    pub fn goalkeeper(&'b self) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.opponents_by_position(
            PlayerFieldPositionGroup::Goalkeeper,
            self.ctx.player.team_id,
        )
    }

    pub fn forwards(&'b self) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.opponents_by_position(PlayerFieldPositionGroup::Forward, self.ctx.player.team_id)
    }

    fn opponents_by_position(
        &'b self,
        position_group: PlayerFieldPositionGroup,
        team_id: u32,
    ) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.ctx
            .context
            .players
            .players
            .values()
            .filter(move |player| {
                player.team_id != team_id
                    && player.tactical_position.current_position.position_group() == position_group
            })
            .map(|player| MatchPlayerLite {
                id: player.id,
                position: self.ctx.tick_context.positions.players.position(player.id),
                tactical_positions: player.tactical_position.current_position
            })
    }

    fn opponents_for_team(
        &'b self,
        player_id: u32,
        team_id: u32,
        has_ball: Option<bool>,
    ) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.ctx
            .context
            .players
            .players
            .values()
            .filter(move |player| {
                // Check if player matches team criteria (different team)
                if player.id == player_id || player.team_id == team_id {
                    return false;
                }

                // Check if player matches has_ball criteria
                let matches_ball_filter = match has_ball {
                    None => true,  // No filter, include all opponents
                    Some(true) => self.ctx.ball().owner_id() == Some(player.id),  // Only opponents with ball
                    Some(false) => self.ctx.ball().owner_id() != Some(player.id)  // Only opponents without ball
                };

                matches_ball_filter
            })
            .map(|player| MatchPlayerLite {
                id: player.id,
                position: self.ctx.tick_context.positions.players.position(player.id),
                tactical_positions: player.tactical_position.current_position
            })
    }
}

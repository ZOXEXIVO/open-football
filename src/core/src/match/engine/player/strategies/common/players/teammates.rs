use crate::PlayerFieldPositionGroup;
use crate::r#match::{MatchPlayerLite, PlayerSide, StateProcessingContext};

pub struct PlayerTeammatesOperationsImpl<'b> {
    ctx: &'b StateProcessingContext<'b>,
}

impl<'b> PlayerTeammatesOperationsImpl<'b> {
    pub fn new(ctx: &'b StateProcessingContext<'b>) -> Self {
        PlayerTeammatesOperationsImpl { ctx }
    }
}

impl<'b> PlayerTeammatesOperationsImpl<'b> {
    pub fn all(&'b self) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.teammates_for_team(self.ctx.player.id, self.ctx.player.team_id, None)
    }

    pub fn players_with_ball(&'b self) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.teammates_for_team(self.ctx.player.id, self.ctx.player.team_id, Some(true))
    }

    pub fn players_without_ball(&'b self) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.teammates_for_team(self.ctx.player.id, self.ctx.player.team_id, Some(false))
    }

    pub fn defenders(&'b self) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.teammates_by_position(PlayerFieldPositionGroup::Defender, self.ctx.player.team_id)
    }

    pub fn forwards(&'b self) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.teammates_by_position(PlayerFieldPositionGroup::Forward, self.ctx.player.team_id)
    }

    fn teammates_by_position(
        &'b self,
        position_group: PlayerFieldPositionGroup,
        team_id: u32,
    ) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.ctx
            .context
            .players
            .entries
            .iter()
            .filter(move |entry| {
                entry.team_id == team_id && entry.position.position_group() == position_group
            })
            .map(|entry| MatchPlayerLite {
                id: entry.id,
                position: self.ctx.tick_context.positions.players.position(entry.id),
                tactical_positions: entry.position,
            })
    }

    fn teammates_for_team(
        &'b self,
        player_id: u32,
        team_id: u32,
        has_ball: Option<bool>,
    ) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.ctx
            .context
            .players
            .entries
            .iter()
            .filter(move |entry| {
                if entry.id == player_id || entry.team_id != team_id {
                    return false;
                }

                match has_ball {
                    None => true,
                    Some(true) => self.ctx.ball().owner_id() == Some(entry.id),
                    Some(false) => self.ctx.ball().owner_id() != Some(entry.id),
                }
            })
            .map(|entry| MatchPlayerLite {
                id: entry.id,
                position: self.ctx.tick_context.positions.players.position(entry.id),
                tactical_positions: entry.position,
            })
    }

    pub fn nearby(&'b self, max_distance: f32) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.nearby_range(1.0, max_distance)
    }

    pub fn nearby_range(
        &'b self,
        min_distance: f32,
        max_distance: f32,
    ) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.ctx
            .tick_context
            .grid
            .teammates_full(
                self.ctx.player.id,
                self.ctx.player.team_id,
                self.ctx.player.position,
                min_distance,
                max_distance,
            )
            .map(|(gp, _dist)| MatchPlayerLite {
                id: gp.id,
                position: gp.position,
                tactical_positions: gp.tactical_position,
            })
    }

    pub fn nearby_to_opponent_goal(&'b self) -> Option<MatchPlayerLite> {
        let want_min_x = self.ctx.player.side == Some(PlayerSide::Right);

        self.nearby(300.0).reduce(|best, candidate| {
            if want_min_x {
                if candidate.position.x < best.position.x {
                    candidate
                } else {
                    best
                }
            } else {
                if candidate.position.x > best.position.x {
                    candidate
                } else {
                    best
                }
            }
        })
    }

    pub fn nearby_ids(&self, max_distance: f32) -> impl Iterator<Item = (u32, f32)> + 'b {
        const MIN_DISTANCE: f32 = 1.0; // Changed from 50.0 to allow closer teammates

        self.ctx
            .tick_context
            .grid
            .teammates(self.ctx.player.id, MIN_DISTANCE, max_distance)
    }

    pub fn exists(&self, max_distance: f32) -> bool {
        const MIN_DISTANCE: f32 = 0.0;

        self.ctx
            .tick_context
            .grid
            .teammates(self.ctx.player.id, MIN_DISTANCE, max_distance)
            .any(|_| true)
    }
}

use crate::r#match::{MatchPlayerLite, PlayerSide, StateProcessingContext};
use crate::PlayerFieldPositionGroup;

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
            .players
            .values()
            .filter(move |player| {
                player.team_id == team_id
                    && player.tactical_position.current_position.position_group() == position_group
            })
            .map(|player| MatchPlayerLite {
                id: player.id,
                position: self.ctx.tick_context.positions.players.position(player.id),
                tactical_positions: player.tactical_position.current_position
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
            .players
            .values()
            .filter(move |player| {
                // Check if player matches team criteria
                if player.id == player_id || player.team_id != team_id {
                    return false;
                }

                // Check if player matches has_ball criteria
                match has_ball {
                    None => true,  // No filter, include all teammates
                    Some(true) => self.ctx.ball().owner_id() == Some(player.id),  // Only teammates with ball
                    Some(false) => self.ctx.ball().owner_id() != Some(player.id)  // Only teammates without ball
                }
            })
            .map(|player| MatchPlayerLite {
                id: player.id,
                position: self.ctx.tick_context.positions.players.position(player.id),
                tactical_positions: player.tactical_position.current_position
            })
    }

    pub fn nearby(&'b self, max_distance: f32) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.nearby_range(1.0, max_distance)
    }

    pub fn nearby_range(&'b self, min_distance: f32, max_distance: f32) -> impl Iterator<Item = MatchPlayerLite> + 'b {
        self.ctx
            .tick_context
            .distances
            .teammates(self.ctx.player.id, min_distance, max_distance)
            .map(|(pid, _)| MatchPlayerLite {
                id: pid,
                position: self.ctx.tick_context.positions.players.position(pid),
                tactical_positions: self.ctx.context.players.by_id(pid).expect(&format!(
                    "unknown player = {}", pid
                )).tactical_position.current_position
            })
    }

    pub fn nearby_to_opponent_goal(&'b self) -> Option<MatchPlayerLite> {
        let mut teammates: Vec<MatchPlayerLite> = self.nearby(300.0).collect();

        if teammates.is_empty() {
            return None;
        }

        teammates.sort_by(|a, b| a.position.x.partial_cmp(&b.position.x).unwrap());

        if self.ctx.player.side == Some(PlayerSide::Right) {
            Some(teammates[0])
        } else {
            Some(teammates[teammates.len() - 1])
        }
    }

    pub fn nearby_ids(&self, max_distance: f32) -> impl Iterator<Item = (u32, f32)> + 'b {
        const MIN_DISTANCE: f32 = 1.0; // Changed from 50.0 to allow closer teammates

        self.ctx
            .tick_context
            .distances
            .teammates(self.ctx.player.id, MIN_DISTANCE, max_distance)
    }

    pub fn exists(&self, max_distance: f32) -> bool {
        const MIN_DISTANCE: f32 = 0.0;

        self.ctx
            .tick_context
            .distances
            .teammates(self.ctx.player.id, MIN_DISTANCE, max_distance)
            .any(|_| true)
    }
}

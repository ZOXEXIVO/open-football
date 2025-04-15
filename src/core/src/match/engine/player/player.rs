use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::tactics::TacticalPositions;
use crate::r#match::events::EventCollection;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::{PlayerMatchState, PlayerState};
use crate::r#match::player::statistics::MatchPlayerStatistics;
use crate::r#match::player::waypoints::WaypointManager;
use crate::r#match::{GameTickContext, MatchContext, StateProcessingContext};
use crate::{
    PersonAttributes, Player, PlayerAttributes, PlayerFieldPositionGroup, PlayerPositionType,
    PlayerSkills,
};
use nalgebra::Vector3;
use std::fmt::*;

#[derive(Debug, Clone)]
pub struct MatchPlayer {
    pub id: u32,
    pub position: Vector3<f32>,
    pub start_position: Vector3<f32>,
    pub attributes: PersonAttributes,
    pub team_id: u32,
    pub player_attributes: PlayerAttributes,
    pub skills: PlayerSkills,
    pub tactical_position: TacticalPositions,
    pub velocity: Vector3<f32>,
    pub side: Option<PlayerSide>,
    pub state: PlayerState,
    pub in_state_time: u64,
    pub statistics: MatchPlayerStatistics,
    pub use_extended_state_logging: bool,

    pub waypoint_manager: WaypointManager,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerSide {
    Left,
    Right,
}

impl MatchPlayer {
    pub fn from_player(
        team_id: u32,
        player: &Player,
        position: PlayerPositionType,
        use_extended_state_logging: bool,
    ) -> Self {
        MatchPlayer {
            id: player.id,
            position: Vector3::zeros(),
            start_position: Vector3::zeros(),
            attributes: player.attributes,
            team_id,
            player_attributes: player.player_attributes,
            skills: player.skills,
            velocity: Vector3::zeros(),
            tactical_position: TacticalPositions::new(position),
            side: None,
            state: Self::default_state(position),
            in_state_time: 0,
            statistics: MatchPlayerStatistics::new(),
            waypoint_manager: WaypointManager::new(),
            use_extended_state_logging,
        }
    }

    pub fn update(
        &mut self,
        context: &MatchContext,
        tick_context: &GameTickContext,
        events: &mut EventCollection,
    ) {
        let player_events = PlayerMatchState::process(self, context, tick_context);

        events.add_from_collection(player_events);

        self.update_waypoint_index(tick_context);

        self.check_boundary_collision(context);
        self.move_to();
    }

    fn check_boundary_collision(&mut self, context: &MatchContext) {
        let field_width = context.field_size.width as f32 + 1.0;
        let field_height = context.field_size.height as f32 + 1.0;

        // Check if ball hits the boundary and reverse its velocity if it does
        if self.position.x <= 0.0 || self.position.x >= field_width {
            self.velocity.x = 0.0;
        }

        if self.position.y <= 0.0 || self.position.y >= field_height {
            self.velocity.y = 0.0;
        }
    }

    pub fn set_default_state(&mut self) {
        self.state = Self::default_state(self.tactical_position.current_position);
    }

    fn default_state(position: PlayerPositionType) -> PlayerState {
        match position.position_group() {
            PlayerFieldPositionGroup::Goalkeeper => {
                PlayerState::Goalkeeper(GoalkeeperState::Standing)
            }
            PlayerFieldPositionGroup::Defender => PlayerState::Defender(DefenderState::Standing),
            PlayerFieldPositionGroup::Midfielder => {
                PlayerState::Midfielder(MidfielderState::Standing)
            }
            PlayerFieldPositionGroup::Forward => PlayerState::Forward(ForwardState::Standing),
        }
    }

    pub fn run_for_ball(&mut self) {
        self.state = match self.tactical_position.current_position.position_group() {
            PlayerFieldPositionGroup::Goalkeeper => {
                PlayerState::Goalkeeper(GoalkeeperState::TakeBall)
            }
            PlayerFieldPositionGroup::Defender => PlayerState::Defender(DefenderState::TakeBall),
            PlayerFieldPositionGroup::Midfielder => {
                PlayerState::Midfielder(MidfielderState::TakeBall)
            }
            PlayerFieldPositionGroup::Forward => PlayerState::Forward(ForwardState::TakeBall),
        }
    }

    fn move_to(&mut self) {
        if !self.velocity.x.is_nan() {
            self.position.x += self.velocity.x;
        }

        if !self.velocity.y.is_nan() {
            self.position.y += self.velocity.y;
        }
    }

    pub fn heading(&self) -> f32 {
        self.velocity.y.atan2(self.velocity.x)
    }

    pub fn has_ball(&self, ctx: &StateProcessingContext<'_>) -> bool {
        ctx.ball().owner_id() == Some(self.id)
    }

    pub fn update_waypoint_index(&mut self, tick_context: &GameTickContext) {
        self.waypoint_manager.update(
            &tick_context.positions.players.position(self.id),
            &self.get_waypoints_as_vectors(),
        );
    }

    pub fn get_waypoints_as_vectors(&self) -> Vec<Vector3<f32>> {
        self.tactical_position
            .tactical_positions
            .iter()
            .filter(|tp| tp.position == self.tactical_position.current_position)
            .flat_map(|tp| &tp.waypoints)
            .map(|(x, y)| Vector3::new(*x, *y, 0.0))
            .collect()
    }

    pub fn should_follow_waypoints(&self, ctx: &StateProcessingContext) -> bool {
        // Check if player has the ball - immediate decision
        if self.has_ball(ctx) {
            return false;
        }

        // Check if a teammate has the ball
        let team_has_ball = if let Some(owner_id) = ctx.ball().owner_id() {
            if let Some(owner) = ctx.context.players.by_id(owner_id) {
                owner.team_id == self.team_id
            } else {
                false
            }
        } else {
            false
        };

        let ball_distance = ctx.ball().distance();
        let is_ball_close = ball_distance < 100.0;

        // If ball is free and close, evaluate if this player should go for it
        if !ctx.ball().is_owned() && is_ball_close {
            return !ctx.team().is_best_player_to_chase_ball();
        }

        match self.tactical_position.current_position.position_group() {
            PlayerFieldPositionGroup::Goalkeeper => {
                // Goalkeepers mostly stay in position
                !is_ball_close && ball_distance > 200.0
            },
            PlayerFieldPositionGroup::Defender => {
                // Defenders are more positionally disciplined
                if team_has_ball {
                    // When team has ball, defenders can push up but generally maintain position
                    !is_ball_close
                } else {
                    // When opposing team has ball, maintain defensive shape
                    !is_ball_close && !ctx.ball().on_own_side()
                }
            },
            PlayerFieldPositionGroup::Midfielder => {
                // Midfielders balance between position and involvement
                if team_has_ball {
                    // Follow waypoints to create passing options when team has ball
                    !is_ball_close
                } else {
                    // When defending, maintain shape unless ball is close
                    !is_ball_close && !ctx.ball().on_own_side()
                }
            },
            PlayerFieldPositionGroup::Forward => {
                // Forwards make more runs and movement
                if team_has_ball {
                    // Make forward runs when team has ball, but not all forwards should rush to ball
                    !is_ball_close && !ctx.team().is_best_player_to_chase_ball()
                } else {
                    // When defending, forwards maintain some structure but press less
                    !is_ball_close && ball_distance > 150.0
                }
            }
        }
    }
}

#[derive(Copy, Clone)]
pub struct MatchPlayerLite {
    pub id: u32,
    pub position: Vector3<f32>,
    pub tactical_positions: PlayerPositionType,
}

impl MatchPlayerLite {
    pub fn has_ball(&self, ctx: &StateProcessingContext<'_>) -> bool {
        ctx.ball().owner_id() == Some(self.id)
    }

    pub fn velocity(&self, ctx: &StateProcessingContext<'_>) -> Vector3<f32> {
        ctx.tick_context.positions.players.velocity(ctx.player.id)
    }

    pub fn distance(&self, ctx: &StateProcessingContext<'_>) -> f32 {
        ctx.tick_context.distances.get(self.id, ctx.player.id)
    }
}

impl Into<MatchPlayerLite> for &MatchPlayer{
    fn into(self) -> MatchPlayerLite {
        MatchPlayerLite {
            id: self.id,
            position: self.position,
            tactical_positions: self.tactical_position.current_position
        }
    }
}
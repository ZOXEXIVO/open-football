use crate::r#match::StateProcessingContext;
use crate::r#match::player::strategies::passing::CrossType;
use nalgebra::Vector3;

#[derive(Debug, Clone)]
pub struct PassingEventContext {
    pub from_player_id: u32,
    pub to_player_id: u32,
    pub pass_target: Vector3<f32>,
    pub pass_force: f32,
    pub reason: &'static str,
    /// Set when this delivery is a modelled cross rather than a pass.
    /// The trajectory solver reads it directly instead of re-deriving the
    /// ball's shape from lane traffic — see `passing::cross` for why those
    /// two rules cannot both hold.
    pub cross_type: Option<CrossType>,
    /// True when `pass_target` is an aim POINT in space rather than the
    /// receiver's feet. Suppresses the receiver-velocity lead, because an
    /// aim point already anticipates the run.
    pub target_is_space: bool,
}

impl PassingEventContext {
    pub fn new() -> PassingEventBuilder {
        PassingEventBuilder::new()
    }
}

pub struct PassingEventBuilder {
    from_player_id: Option<u32>,
    to_player_id: Option<u32>,
    pass_force: Option<f32>,
    reason: Option<&'static str>,
    cross_type: Option<CrossType>,
    target_override: Option<Vector3<f32>>,
}

impl Default for PassingEventBuilder {
    fn default() -> Self {
        PassingEventBuilder::new()
    }
}

impl PassingEventBuilder {
    pub fn new() -> Self {
        PassingEventBuilder {
            from_player_id: None,
            to_player_id: None,
            pass_force: None,
            reason: None,
            cross_type: None,
            target_override: None,
        }
    }

    pub fn with_from_player_id(mut self, from_player_id: u32) -> Self {
        self.from_player_id = Some(from_player_id);
        self
    }

    pub fn with_to_player_id(mut self, to_player_id: u32) -> Self {
        self.to_player_id = Some(to_player_id);
        self
    }

    pub fn with_pass_force(mut self, pass_force: f32) -> Self {
        self.pass_force = Some(pass_force);
        self
    }

    pub fn with_reason(mut self, reason: &'static str) -> Self {
        self.reason = Some(reason);
        self
    }

    /// Mark this delivery as a cross of the given type.
    pub fn with_cross_type(mut self, cross_type: CrossType) -> Self {
        self.cross_type = Some(cross_type);
        self
    }

    /// Aim the delivery at a point on the pitch instead of at the
    /// receiver's current position.
    pub fn with_target_point(mut self, target: Vector3<f32>) -> Self {
        self.target_override = Some(target);
        self
    }

    pub fn build(self, ctx: &StateProcessingContext) -> PassingEventContext {
        let to_player_id = self.to_player_id.unwrap();
        let target_is_space = self.target_override.is_some();

        PassingEventContext {
            from_player_id: self.from_player_id.unwrap(),
            to_player_id,
            pass_target: self
                .target_override
                .unwrap_or_else(|| ctx.tick_context.positions.players.position(to_player_id)),
            pass_force: self
                .pass_force
                .unwrap_or_else(|| ctx.player().pass_teammate_power(to_player_id)),
            reason: self.reason.unwrap_or("No reason specified"),
            cross_type: self.cross_type,
            target_is_space,
        }
    }
}

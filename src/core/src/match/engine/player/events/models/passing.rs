use crate::r#match::StateProcessingContext;
use nalgebra::Vector3;

#[derive(Debug)]
pub struct PassingEventContext {
    pub from_player_id: u32,
    pub to_player_id: u32,
    pub pass_target: Vector3<f32>,
    pub pass_force: f32,
    pub reason: String,
}

impl PassingEventContext {
    pub fn new() -> PassingEventBuilder{
        PassingEventBuilder::new()
    }
}

pub struct PassingEventBuilder {
    from_player_id: Option<u32>,
    to_player_id: Option<u32>,
    pass_force: Option<f32>,
    reason: Option<String>,
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

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn build(self, ctx: &StateProcessingContext) -> PassingEventContext {
        let to_player_id = self.to_player_id.unwrap();

        PassingEventContext {
            from_player_id: self.from_player_id.unwrap(),
            to_player_id,
            pass_target: ctx.tick_context.positions.players.position(to_player_id),
            pass_force: self.pass_force.unwrap_or_else(|| ctx.player().pass_teammate_power(to_player_id)),
            reason: self.reason.unwrap_or_else(|| "No reason specified".to_string()),
        }
    }
}

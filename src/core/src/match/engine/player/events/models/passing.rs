use nalgebra::Vector3;
use crate::r#match::StateProcessingContext;

#[derive(Debug)]
pub struct PassingEventContext {
    pub from_player_id: u32,
    pub to_player_id: u32,
    pub pass_target: Vector3<f32>,
    pub pass_force: f32
}

impl PassingEventContext {
    pub fn new() -> PassingEventBuilder{
        PassingEventBuilder::new()
    }
}

pub struct PassingEventBuilder {
    from_player_id: Option<u32>,
    to_player_id: Option<u32>
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
            to_player_id: None
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

    pub fn build(self, ctx: &StateProcessingContext) -> PassingEventContext {
        let to_player_id = self.to_player_id.unwrap();    
        
        PassingEventContext {
            from_player_id: self.from_player_id.unwrap(),
            to_player_id,
            pass_target: ctx.tick_context.positions.players.position(to_player_id),
            pass_force: ctx.player().pass_teammate_power(to_player_id),
        }
    }
}

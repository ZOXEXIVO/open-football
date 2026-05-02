use crate::r#match::StateProcessingContext;
use crate::r#match::player::strategies::players::ShotType;
use nalgebra::Vector3;

#[derive(Debug, Clone)]
pub struct ShootingEventContext {
    pub from_player_id: u32,
    pub target: Vector3<f32>,
    pub force: f64,
    pub reason: &'static str,
    pub tick: u64,
    pub shot_type: ShotType,
}

impl ShootingEventContext {
    pub fn new() -> ShootingEventBuilder {
        ShootingEventBuilder::new()
    }
}

pub struct ShootingEventBuilder {
    from_player_id: Option<u32>,
    target: Option<Vector3<f32>>,
    reason: Option<&'static str>,
    shot_type: Option<ShotType>,
}

impl Default for ShootingEventBuilder {
    fn default() -> Self {
        ShootingEventBuilder::new()
    }
}

impl ShootingEventBuilder {
    pub fn new() -> Self {
        ShootingEventBuilder {
            from_player_id: None,
            target: None,
            reason: None,
            shot_type: None,
        }
    }

    pub fn with_player_id(mut self, from_player_id: u32) -> Self {
        self.from_player_id = Some(from_player_id);
        self
    }

    pub fn with_target(mut self, target: Vector3<f32>) -> Self {
        self.target = Some(target);
        self
    }

    pub fn with_reason(mut self, reason: &'static str) -> Self {
        self.reason = Some(reason);
        self
    }

    pub fn with_shot_type(mut self, shot_type: ShotType) -> Self {
        self.shot_type = Some(shot_type);
        self
    }

    pub fn build(self, ctx: &StateProcessingContext) -> ShootingEventContext {
        ShootingEventContext {
            from_player_id: self.from_player_id.unwrap(),
            target: self.target.unwrap(),
            force: ctx.player().shoot_goal_power(),
            reason: self.reason.unwrap_or("No reason specified"),
            tick: ctx.current_tick(),
            shot_type: self.shot_type.unwrap_or(ShotType::FootOpenPlay),
        }
    }
}

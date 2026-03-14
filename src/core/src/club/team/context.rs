#[derive(Clone)]
pub struct TeamContext {
    pub id: u32,
    pub reputation: f32,
}

impl TeamContext {
    pub fn new(id: u32) -> Self {
        TeamContext { id, reputation: 0.0 }
    }

    pub fn with_reputation(id: u32, reputation: f32) -> Self {
        TeamContext { id, reputation }
    }
}

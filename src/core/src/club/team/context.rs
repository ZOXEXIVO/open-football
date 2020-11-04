#[derive(Clone)]
pub struct TeamContext {
    id: u32
}

impl TeamContext {
    pub fn new(id: u32) -> Self {
        TeamContext {
            id
        }
    }
}
#[derive(Clone)]
pub struct PlayerContext {
    pub id: Option<u32>,
}

impl PlayerContext {
    pub fn new(id: Option<u32>) -> Self {
        PlayerContext { id }
    }
}

#[derive(Clone)]
pub struct ContinentContext {
    id: u32
}

impl ContinentContext {
    pub fn new(id: u32) -> Self {
        ContinentContext { id }
    }
}

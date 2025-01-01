#[derive(Clone)]
pub struct ContinentContext {
    _id: u32,
}

impl ContinentContext {
    pub fn new(id: u32) -> Self {
        ContinentContext { _id: id }
    }
}

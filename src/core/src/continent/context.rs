#[derive(Clone)]
pub struct ContinentContext {
    id: u32,
}

impl ContinentContext {
    pub fn new(id: u32) -> Self {
        ContinentContext { id }
    }

    /// Continent id matching the values documented in
    /// `transfers::scouting_region` (1 = Europe, 3 = South America, …).
    pub fn id(&self) -> u32 {
        self.id
    }
}

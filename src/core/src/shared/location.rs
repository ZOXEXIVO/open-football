#[derive(Debug, Clone)]
pub struct Location {
    pub city_id: u32,
}

impl Location {
    pub fn new(city_id: u32) -> Self {
        Location { city_id }
    }
}

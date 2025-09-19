#[derive(Clone)]
pub struct CountryContext {
    pub id: u32,
}

impl CountryContext {
    pub fn new(id: u32) -> Self {
        CountryContext { id }
    }
}

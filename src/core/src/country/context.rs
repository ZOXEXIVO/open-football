#[derive(Clone)]
pub struct CountryContext {
    _id: u32,
}

impl CountryContext {
    pub fn new(id: u32) -> Self {
        CountryContext { _id: id }
    }
}

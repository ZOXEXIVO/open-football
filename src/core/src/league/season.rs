#[derive(Debug, Clone)]
pub struct Season {
    pub display: String,
    pub start_year: u16,
}

impl Season {
    pub fn new(start_year: u16) -> Self {
        let end_year = start_year + 1;
        Season {
            display: format!("{}/{}", start_year, end_year % 100),
            start_year,
        }
    }
}

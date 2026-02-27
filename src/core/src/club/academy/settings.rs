use std::ops::Range;

#[derive(Debug, Clone)]
pub struct AcademySettings {
    pub players_count_range: Range<u8>,
}

impl AcademySettings {
    pub fn default() -> Self {
        AcademySettings {
            players_count_range: 30..50,
        }
    }
}

use std::ops::Range;

/// Per-club academy population window. The pathway-review tick adjusts
/// `players_count_range` based on academy tier and pipeline health; the
/// intake and backfill paths read it to keep the resident squad in
/// range.
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

use std::ops::Range;

#[derive(Debug, Clone)]
pub struct AcademySettings {
    pub players_count_range: Range<u8>,
    /// Weekly training sessions per academy level tier
    pub sessions_per_week_by_tier: [u8; 4],
}

impl AcademySettings {
    pub fn default() -> Self {
        AcademySettings {
            players_count_range: 30..50,
            // Indexed by tier: [low(1-3), mid(4-6), high(7-9), elite(10)]
            sessions_per_week_by_tier: [3, 4, 5, 6],
        }
    }

    /// Get weekly training sessions for the given academy level (1-10)
    pub fn sessions_per_week(&self, level: u8) -> u8 {
        let tier = match level {
            1..=3 => 0,
            4..=6 => 1,
            7..=9 => 2,
            _ => 3,
        };
        self.sessions_per_week_by_tier[tier]
    }
}

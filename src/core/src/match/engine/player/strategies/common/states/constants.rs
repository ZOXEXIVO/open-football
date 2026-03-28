/// Maximum condition value
pub const MAX_CONDITION: i16 = 10000;

/// Absolute minimum condition during a match (30% — FM-like floor)
pub const MATCH_CONDITION_FLOOR: i16 = 3000;

/// Global fatigue rate multiplier (lower = slower condition decrease)
pub const FATIGUE_RATE_MULTIPLIER: f32 = 0.013;

/// Recovery rate multiplier (lower = slower condition recovery)
pub const RECOVERY_RATE_MULTIPLIER: f32 = 0.233;

/// Condition threshold below which jadedness increases (35%)
pub const LOW_CONDITION_THRESHOLD: i16 = 3500;

/// Condition threshold for goalkeepers jadedness (30%)
pub const GOALKEEPER_LOW_CONDITION_THRESHOLD: i16 = 3000;

/// Jadedness check interval for field players (ticks)
pub const FIELD_PLAYER_JADEDNESS_INTERVAL: u64 = 100;

/// Jadedness check interval for goalkeepers (ticks)
pub const GOALKEEPER_JADEDNESS_INTERVAL: u64 = 150;

/// Maximum jadedness value
pub const MAX_JADEDNESS: i16 = 10000;

/// Jadedness increase per check when condition is low (field players)
pub const JADEDNESS_INCREMENT: i16 = 5;

/// Jadedness increase per check when condition is low (goalkeepers)
pub const GOALKEEPER_JADEDNESS_INCREMENT: i16 = 3;

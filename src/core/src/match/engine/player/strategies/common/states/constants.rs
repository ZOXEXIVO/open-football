/// Maximum condition value (Football Manager style)
pub const MAX_CONDITION: i16 = 10000;

/// Global fatigue rate multiplier (lower = slower condition decrease)
/// Set to 0.4 to make condition decrease 2.5x slower
pub const FATIGUE_RATE_MULTIPLIER: f32 = 0.4;

/// Recovery rate multiplier (lower = slower condition recovery)
/// Set to 0.5 to make condition recovery 2x slower (more visible than 3x slower)
pub const RECOVERY_RATE_MULTIPLIER: f32 = 0.5;

/// Condition threshold below which jadedness increases
pub const LOW_CONDITION_THRESHOLD: i16 = 2000;

/// Condition threshold for goalkeepers jadedness
pub const GOALKEEPER_LOW_CONDITION_THRESHOLD: i16 = 1500;

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

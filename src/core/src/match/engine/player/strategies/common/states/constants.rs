/// Maximum condition value
pub const MAX_CONDITION: i16 = 10000;

/// Absolute minimum condition during a match (15%). Lowered from 30% so
/// genuinely exhausted players actually slow down — the previous floor
/// combined with a fast recovery rate meant nobody ever truly tired.
pub const MATCH_CONDITION_FLOOR: i16 = 1500;

/// Global fatigue rate multiplier (lower = slower condition decrease)
pub const FATIGUE_RATE_MULTIPLIER: f32 = 0.013;

/// Recovery rate multiplier (lower = slower condition recovery).
/// Set to ~2× the fatigue rate. The prior 0.233 was 17.9× the drain rate,
/// which let any forward sprint indefinitely (10s sprint cancelled by
/// 2.6s walk). With this calibration, sprint:rest break-even sits at
/// ~1:5 — matching real-football aerobic load and producing actual
/// late-game fatigue dynamics.
pub const RECOVERY_RATE_MULTIPLIER: f32 = 0.025;

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

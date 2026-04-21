/// Maximum condition value
pub const MAX_CONDITION: i16 = 10000;

/// Absolute minimum condition during a match (15%). Lowered from 30% so
/// genuinely exhausted players actually slow down — the previous floor
/// combined with a fast recovery rate meant nobody ever truly tired.
pub const MATCH_CONDITION_FLOOR: i16 = 1500;

/// Global fatigue rate multiplier (lower = slower condition decrease).
/// Raised another 20% (0.020 → 0.024) so stamina changes hit harder
/// per tick — sprints now drain visibly faster and a tired late-game
/// defender really does lose half a step. Paired with a matching 20%
/// bump to recovery so the sprint:rest ratio stays realistic.
pub const FATIGUE_RATE_MULTIPLIER: f32 = 0.024;

/// Recovery rate multiplier (lower = slower condition recovery).
/// Tuned down 30% (0.048 → 0.0336) so recovery lags behind drain —
/// fatigued players stay fatigued longer, sprints leave a bigger
/// lasting mark. Combined with the 20% drain bump, the sprint:rest
/// break-even now favours the fatigue side of the ledger, so a
/// player who presses hard early in the match genuinely fades later.
pub const RECOVERY_RATE_MULTIPLIER: f32 = 0.0336;

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

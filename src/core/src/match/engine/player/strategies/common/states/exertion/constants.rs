/// Maximum condition value
pub const MAX_CONDITION: i16 = 10000;

/// Absolute minimum condition during a match (15%). Lowered from 30% so
/// genuinely exhausted players actually slow down — the previous floor
/// combined with a fast recovery rate meant nobody ever truly tired.
pub const MATCH_CONDITION_FLOOR: i16 = 1500;

/// Global fatigue rate multiplier (lower = slower condition decrease).
///
/// Recalibrated 0.024 → 0.0035 after the dev_match condition-trajectory
/// diagnostic showed outfield players collapsing from 100% to the 15%
/// floor inside the first ~20 minutes and playing the remaining 70 as
/// zombies. That cliff was THE driver of the goal-timing anomaly (36%
/// of all goals in minutes 0-15 vs real ~11%): fresh-legs attack volume
/// in band 0 ran ~3× the late-match rate, then everything sagged once
/// every outfielder hit the floor.
///
/// Why the old value was an order of magnitude hot: the velocity-band
/// occupancy counter (`time_band_diag::VELOCITY_BAND_TICKS`) shows the
/// engine's movement layer keeps outfielders at >85% of max speed for
/// ~46% of ticks and 60-85% for another ~22% — real players sprint
/// ~2-4% of match time. The drain coefficients assumed a realistic
/// sprint share, so against the engine's actual movement profile they
/// produced ~4.5%/min net drain (floor in ~19 min) instead of the real
/// ~0.33%/min (finish at ~70%). Until the off-ball movement layer
/// learns to walk, the rate multipliers below are calibrated to the
/// engine's metabolic scale, not real-world band rates.
///
/// Target trajectory (FM convention, average-stamina starter):
///   minute 15 ≈ 95% / HT ≈ 88% / minute 75 ≈ 76% / FT ≈ 70%.
/// Stamina still differentiates: the 0.5-1.5× stamina factor and the
/// late-match ramp (1.15→1.50×) are unchanged, so pressing sides and
/// low-stamina squads genuinely fade while elite-stamina players keep
/// their legs deep into the second half.
pub const FATIGUE_RATE_MULTIPLIER: f32 = 0.0035;

/// Recovery rate multiplier (lower = slower condition recovery).
/// Scaled down with the fatigue multiplier (0.0336 → 0.0080) to keep
/// the drain:recovery break-even on the fatigue side of the ledger —
/// the ~25% of ticks players spend near-stationary must not refill
/// what running burns, or nobody ever tires. Ratio chosen so the
/// equilibrium net drain lands on the target trajectory above given
/// the measured ~68% running/sprinting occupancy.
pub const RECOVERY_RATE_MULTIPLIER: f32 = 0.0080;

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

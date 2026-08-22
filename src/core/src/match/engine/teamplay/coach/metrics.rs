//! **What the coach watches.** The rolling 10-15 minute window
//! ([`RollingTeamMetrics`]) the smarter evaluator branches on, and the
//! cumulative [`MetricSnapshot`] the match loop rotates to produce it.
//!
//! Plain data — the match loop fills both, `evaluate_with_metrics` and
//! [`TacticalNeed`](super::needs::TacticalNeed) read them.

/// Rolling team metrics consumed by the smarter coach evaluator. The
/// match loop is responsible for keeping these up to date (sliding
/// window — minute-window data trimmed every tick).
#[derive(Debug, Clone, Copy, Default)]
pub struct RollingTeamMetrics {
    pub xg_for_last_15: f32,
    pub xg_against_last_15: f32,
    pub shots_for_last_15: u16,
    pub deep_entries_for_last_15: u16,
    /// Possession-by-position metric (0.0..1.0): fraction of recent ticks
    /// where the ball was in the opposition half.
    pub field_tilt_last_10: f32,
    pub possession_last_10: f32,
    pub dangerous_turnovers_last_10: u16,
    /// Successful pressures / total pressures in the last 10 minutes.
    pub press_success_rate_last_10: f32,
    /// Rolling average of how many times the opposition played through
    /// our defensive line (per minute, last 10).
    pub avg_defensive_line_breaks: f32,
}

/// Snapshot of cumulative match counters captured a window ago.
/// `evaluate_coaches` rotates a fresh snapshot whenever the gap to
/// `current_tick` exceeds the rolling-metrics window so the deltas
/// always represent ~15 sim-minutes of play.
#[derive(Debug, Clone, Copy, Default)]
pub struct MetricSnapshot {
    pub tick: u64,
    pub xg_for: f32,
    pub xg_against: f32,
    pub shots_for: u32,
    pub pressures: u32,
    pub successful_pressures: u32,
    pub deep_entries_for: u32,
    pub dangerous_turnovers: u32,
    pub possession_ticks: u32,
    pub field_tilt_ticks: u32,
}

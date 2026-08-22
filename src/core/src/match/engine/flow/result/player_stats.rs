//! What one player's match looked like when he left the pitch: the
//! stat line the rating is derived from, and the physical state the
//! post-match condition model reads.

use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::zones::ZoneStats;
use serde::{Deserialize, Serialize};

/// Final physical state of a player at the moment they left the pitch
/// (substitution time or full time). The match engine drains the
/// `MatchPlayer` copy's condition tick-by-tick during the sim; without
/// this snapshot the persisted `Player` only sees minute counts and
/// load — never the actual end-of-match energy. Post-match exertion
/// feeds `final_match_energy` into a duration-scaled depletion model
/// so the persisted condition reflects how empty the tank really got.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PlayerMatchPhysicalSnapshot {
    pub player_id: u32,
    /// Minutes spent on the pitch. Computed from `entry_match_time_ms`
    /// to the exit time (substitution) or current time (full time).
    pub minutes_played: f32,
    /// Condition (0..10000) at kickoff or at the moment this player
    /// entered the pitch as a substitute.
    pub starting_condition: i16,
    /// Condition (0..10000) at the moment this player left the pitch
    /// or at full time. Engine floor is 1500 (15%) — values this low
    /// indicate the player was running on fumes, which the post-match
    /// formula uses to amplify the persisted condition drop.
    pub final_match_energy: i16,
    /// Engine-side hint for high-intensity work done by this player —
    /// sprint distance, pressure bursts, etc. Currently seeded from
    /// the position-group default share (0.05..0.32) so the model has
    /// a starting value before the engine grows per-player tracking.
    pub high_intensity_load_hint: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerMatchEndStats {
    pub shots_on_target: u16,
    pub shots_total: u16,
    pub passes_attempted: u16,
    pub passes_completed: u16,
    pub tackles: u16,
    pub interceptions: u16,
    pub saves: u16,
    /// Shots-on-target the player (typically a GK) had to deal with —
    /// `saves` + goals conceded. Drives the save-percentage component
    /// of the rating helper.
    pub shots_faced: u16,
    pub goals: u16,
    pub assists: u16,
    /// Public/effective match rating. This is the value every downstream
    /// reader (match page DTO, awards, scouting showcase, league stat
    /// rebuild, season averages) consumes. The engine writes the pure
    /// stat-line value here at first; the league pipeline then overwrites
    /// it with the settlement-adjusted public rating in
    /// `process_match_events` so there's a single canonical field across
    /// the codebase. The original engine rating is preserved on
    /// `raw_match_rating` for diagnostics / calibration scripts.
    pub match_rating: f32,
    /// Pure stat-line rating produced by `RatingContext::calculate()`,
    /// frozen before settlement / chemistry / personality adjustments
    /// rewrite `match_rating`. Kept reachable for calibration tests and
    /// debug surfaces that need the unfiltered engine verdict. Defaults
    /// to `0.0` when deserialised from a save written before this field
    /// existed — readers should treat `0.0` as "raw rating not recorded
    /// separately" and fall back to `match_rating`.
    #[serde(default)]
    pub raw_match_rating: f32,
    /// Sum of expected goals from this player's shots in this match.
    pub xg: f32,
    /// Player's position group for position-aware rating calculation.
    pub position_group: PlayerFieldPositionGroup,
    /// Fouls committed by the player in this match.
    pub fouls: u16,
    /// Yellow cards received (0, 1, or 2).
    pub yellow_cards: u16,
    /// 1 if the player was sent off (either two yellows or direct red).
    pub red_cards: u16,
    /// Match minutes played. Used by the rating helper to dampen event
    /// bonuses for short cameos.
    pub minutes_played: u16,
    /// Modern build-up / chance-creation stats — feed the rating helper
    /// and end-of-match calibration. All zero for legacy callers.
    pub key_passes: u16,
    pub progressive_passes: u16,
    pub progressive_carries: u16,
    pub successful_dribbles: u16,
    pub attempted_dribbles: u16,
    pub successful_pressures: u16,
    /// Total close-range pressures applied — superset of
    /// `successful_pressures`. Used for a small "pressing volume" credit
    /// that's worth less per event than a successful pressure.
    pub pressures: u16,
    pub blocks: u16,
    pub clearances: u16,
    /// Completed passes finishing inside the opposition penalty area —
    /// chance-creation indicator independent of the eventual shot.
    pub passes_into_box: u16,
    pub crosses_attempted: u16,
    pub crosses_completed: u16,
    /// xG of all shots in possessions this player participated in. Used
    /// for build-up credit (small) without double-counting goals.
    pub xg_chain: f32,
    /// xG of build-up chains excluding the player's own shots / assists.
    /// Pure "made the chance happen" signal.
    pub xg_buildup: f32,
    /// First-touch resolutions that fluffed the ball.
    pub miscontrols: u16,
    /// First-touch resolutions in the heavy-touch band — kept the ball
    /// alive but gave it away in tempo.
    pub heavy_touches: u16,
    /// Cumulative pitch-units carried under control. Tie-breaker only.
    pub carry_distance: u32,
    pub errors_leading_to_shot: u16,
    pub errors_leading_to_goal: u16,
    /// (GK) Post-shot xG faced minus goals conceded. Positive values
    /// indicate above-expectation shot-stopping.
    pub xg_prevented: f32,
    /// (GK) Chance value of every shot on target the keeper had to deal
    /// with, saved or conceded — the expectation term of the rating's
    /// goals-prevented model. `0.0` on stat lines written before the
    /// counter existed (and on hand-built fixtures), in which case the
    /// rating falls back to a flat per-shot conversion baseline.
    #[serde(default)]
    pub xg_faced: f32,
    /// Offside calls against this player.
    pub offsides: u16,
    /// Auto-goals scored by this player. Treated as a -1.0 base
    /// penalty in the rating with an extra -0.30 because the OG sits
    /// inside the player's own goal mouth.
    pub own_goals: u16,
    /// Per-zone counters mirrored from `MatchPlayerStatistics`. Used
    /// by the rating helper to apply zone-aware multipliers without
    /// re-deriving the action stream.
    pub zone_stats: ZoneStats,
}

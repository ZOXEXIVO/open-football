use super::LeagueResult;
use super::data_access::LeagueProcessAccess;
use crate::club::player::events::match_exertion::MatchExertionInputs;
use crate::r#match::FieldSquad;
use crate::r#match::MatchResultRaw;
use crate::r#match::engine::result::PlayerMatchPhysicalSnapshot;
use chrono::NaiveDate;
use std::collections::HashMap;

/// Regulation match duration in minutes. Used as the upper bound for
/// fallback minute synthesis when no snapshot is supplied.
const REGULATION_MATCH_MINUTES: f32 = 90.0;
/// Extra-time match duration in minutes (regulation + 2×15 ET halves).
/// Treated as the upper bound when the engine reports a total elapsed
/// time materially beyond regulation — knockout ties played to a
/// conclusion before penalty kicks.
const EXTRA_TIME_MATCH_MINUTES: f32 = 120.0;
/// Buffer over 90 (in ms) used to detect extra-time matches. Ten
/// minutes of net stoppage time across both halves is the practical
/// upper bound for a regulation match — anything past 100 in-engine
/// minutes is by definition extra time, not stoppage.
const EXTRA_TIME_DETECTION_MS: u64 = 100 * 60_000;

impl LeagueResult {
    pub(super) fn apply_post_match_physical_effects<D: LeagueProcessAccess>(
        details: &MatchResultRaw,
        data: &mut D,
        is_friendly: bool,
    ) {
        let now = data.date().date();
        // Engine match-time → actual match duration in minutes. A
        // regulation match reports ~90 (plus stoppage); extra-time
        // matches sail well past 100. Snapshots already encode minutes
        // precisely; the fallback path needs this duration to assume
        // the right kickoff-to-finish span for starters and to clamp
        // sub-on durations sensibly.
        let actual_match_minutes = derive_actual_minutes(details.match_time_ms);

        let mut subbed_out_at: HashMap<u32, u64> = HashMap::new();
        let mut subbed_in_at: HashMap<u32, u64> = HashMap::new();
        for sub in &details.substitutions {
            subbed_out_at.insert(sub.player_out_id, sub.match_time_ms);
            subbed_in_at.insert(sub.player_in_id, sub.match_time_ms);
        }

        for team in [&details.left_team_players, &details.right_team_players] {
            apply_side(
                team,
                &subbed_out_at,
                &subbed_in_at,
                &details.physical_snapshots,
                data,
                now,
                is_friendly,
                actual_match_minutes,
            );
        }
    }
}

/// Convert the engine's `total_match_time` (ms) into the match
/// duration to use for fallback minute synthesis. Regulation matches
/// clamp to 90 minutes; matches that ran into extra time clamp to
/// 120 minutes. Anything in between (10+ minutes of stoppage time
/// across both halves) is treated as extra time so the fallback path
/// doesn't underbill starters.
fn derive_actual_minutes(match_time_ms: u64) -> f32 {
    if match_time_ms >= EXTRA_TIME_DETECTION_MS {
        EXTRA_TIME_MATCH_MINUTES
    } else {
        REGULATION_MATCH_MINUTES
    }
}

fn apply_side<D: LeagueProcessAccess>(
    team: &FieldSquad,
    subbed_out_at: &HashMap<u32, u64>,
    subbed_in_at: &HashMap<u32, u64>,
    physical_snapshots: &HashMap<u32, PlayerMatchPhysicalSnapshot>,
    data: &mut D,
    now: NaiveDate,
    is_friendly: bool,
    actual_match_minutes: f32,
) {
    for &player_id in &team.main {
        let fallback_minutes = subbed_out_at
            .get(&player_id)
            .map(|ms| ((*ms as f32) / 60_000.0).clamp(0.0, actual_match_minutes))
            .unwrap_or(actual_match_minutes);
        apply_to_player(
            data,
            player_id,
            physical_snapshots.get(&player_id).copied(),
            fallback_minutes,
            now,
            is_friendly,
        );
    }

    for &player_id in &team.substitutes_used {
        let fallback_minutes = subbed_in_at
            .get(&player_id)
            .map(|ms| {
                let on_at = (*ms as f32) / 60_000.0;
                (actual_match_minutes - on_at).clamp(0.0, actual_match_minutes)
            })
            // No swap recorded but the player is in substitutes_used —
            // best-effort default of "came on for the last 30 minutes".
            // Clamped against actual duration so an extra-time fallback
            // doesn't read negative for a sub on at minute 150.
            .unwrap_or((actual_match_minutes * (1.0 / 3.0)).clamp(0.0, actual_match_minutes));
        apply_to_player(
            data,
            player_id,
            physical_snapshots.get(&player_id).copied(),
            fallback_minutes,
            now,
            is_friendly,
        );
    }
}

fn apply_to_player<D: LeagueProcessAccess>(
    data: &mut D,
    player_id: u32,
    snapshot: Option<PlayerMatchPhysicalSnapshot>,
    fallback_minutes: f32,
    now: NaiveDate,
    is_friendly: bool,
) {
    if let Some(player) = data.player_mut(player_id) {
        // Prefer the engine snapshot (real end-of-match energy);
        // fall back to a minutes-only synthesis for legacy callers
        // who didn't supply snapshots. The persisted condition drop
        // is materially smaller without the snapshot, so this path
        // should be temporary — we keep it for backwards compat,
        // not as a design choice.
        let inputs = match snapshot {
            Some(snap) => MatchExertionInputs {
                minutes: snap.minutes_played,
                starting_condition: snap.starting_condition,
                final_match_energy: snap.final_match_energy,
                high_intensity_load_hint: snap.high_intensity_load_hint,
            },
            None => MatchExertionInputs::from_minutes(player, fallback_minutes),
        };
        player.on_match_exertion(inputs, now, is_friendly);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_minutes_regulation_under_threshold() {
        // A normal match runs ~90 minutes + a few minutes of stoppage.
        // 92 minutes elapsed is still regulation for the fallback.
        let ms = 92 * 60_000;
        assert_eq!(derive_actual_minutes(ms), REGULATION_MATCH_MINUTES);
    }

    #[test]
    fn derive_minutes_extra_time_above_threshold() {
        // A knockout that went to extra time elapses 120 in-engine
        // minutes (plus stoppage). The fallback duration must reflect
        // that so starters don't get underbilled by 30 minutes.
        let ms = 115 * 60_000;
        assert_eq!(derive_actual_minutes(ms), EXTRA_TIME_MATCH_MINUTES);
    }

    #[test]
    fn derive_minutes_at_threshold_boundary() {
        // Exactly the detection threshold (100 min) flips to extra
        // time — that's at or above where regulation-stoppage caps out.
        let ms = EXTRA_TIME_DETECTION_MS;
        assert_eq!(derive_actual_minutes(ms), EXTRA_TIME_MATCH_MINUTES);
    }
}

use super::LeagueResult;
use crate::club::player::events::match_exertion::MatchExertionInputs;
use crate::r#match::FieldSquad;
use crate::r#match::MatchResultRaw;
use crate::r#match::engine::result::PlayerMatchPhysicalSnapshot;
use crate::simulator::SimulatorData;
use std::collections::HashMap;

impl LeagueResult {
    pub(super) fn apply_post_match_physical_effects(
        details: &MatchResultRaw,
        data: &mut SimulatorData,
        is_friendly: bool,
    ) {
        let now = data.date.date();

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
            );
        }
    }
}

fn apply_side(
    team: &FieldSquad,
    subbed_out_at: &HashMap<u32, u64>,
    subbed_in_at: &HashMap<u32, u64>,
    physical_snapshots: &HashMap<u32, PlayerMatchPhysicalSnapshot>,
    data: &mut SimulatorData,
    now: chrono::NaiveDate,
    is_friendly: bool,
) {
    for &player_id in &team.main {
        let fallback_minutes = subbed_out_at
            .get(&player_id)
            .map(|ms| (*ms / 60000) as f32)
            .unwrap_or(90.0);
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
            .map(|ms| 90.0 - (*ms / 60000) as f32)
            .unwrap_or(30.0);
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

fn apply_to_player(
    data: &mut SimulatorData,
    player_id: u32,
    snapshot: Option<PlayerMatchPhysicalSnapshot>,
    fallback_minutes: f32,
    now: chrono::NaiveDate,
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

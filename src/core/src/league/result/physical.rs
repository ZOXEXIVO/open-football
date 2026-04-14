use crate::r#match::FieldSquad;
use crate::r#match::MatchResultRaw;
use crate::simulator::SimulatorData;
use std::collections::HashMap;
use super::LeagueResult;

impl LeagueResult {
    pub(super) fn apply_post_match_physical_effects(details: &MatchResultRaw, data: &mut SimulatorData) {
        let now = data.date.date();

        let mut subbed_out_at: HashMap<u32, u64> = HashMap::new();
        let mut subbed_in_at: HashMap<u32, u64> = HashMap::new();
        for sub in &details.substitutions {
            subbed_out_at.insert(sub.player_out_id, sub.match_time_ms);
            subbed_in_at.insert(sub.player_in_id, sub.match_time_ms);
        }

        for team in [&details.left_team_players, &details.right_team_players] {
            apply_side(team, &subbed_out_at, &subbed_in_at, data, now);
        }
    }
}

fn apply_side(
    team: &FieldSquad,
    subbed_out_at: &HashMap<u32, u64>,
    subbed_in_at: &HashMap<u32, u64>,
    data: &mut SimulatorData,
    now: chrono::NaiveDate,
) {
    for &player_id in &team.main {
        let minutes = subbed_out_at
            .get(&player_id)
            .map(|ms| (*ms / 60000) as f32)
            .unwrap_or(90.0);
        if let Some(player) = data.player_mut(player_id) {
            player.on_match_exertion(minutes, now);
        }
    }

    for &player_id in &team.substitutes_used {
        let minutes = subbed_in_at
            .get(&player_id)
            .map(|ms| 90.0 - (*ms / 60000) as f32)
            .unwrap_or(30.0);
        if let Some(player) = data.player_mut(player_id) {
            player.on_match_exertion(minutes, now);
        }
    }
}

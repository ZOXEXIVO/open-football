use crate::club::player::injury::InjuryType;
use crate::r#match::FieldSquad;
use crate::r#match::MatchResultRaw;
use crate::simulator::SimulatorData;
use crate::utils::DateUtils;
use crate::{HappinessEventType, PlayerStatusType};
use std::collections::HashMap;
use super::LeagueResult;

impl LeagueResult {
    pub(super) fn apply_post_match_physical_effects(details: &MatchResultRaw, data: &mut SimulatorData) {
        let now = data.date.date();

        // Build substitution lookup: player_id -> time in ms they were subbed out/in
        let mut subbed_out_at: HashMap<u32, u64> = HashMap::new();
        let mut subbed_in_at: HashMap<u32, u64> = HashMap::new();
        for sub in &details.substitutions {
            subbed_out_at.insert(sub.player_out_id, sub.match_time_ms);
            subbed_in_at.insert(sub.player_in_id, sub.match_time_ms);
        }

        let teams = [&details.left_team_players, &details.right_team_players];

        for team in teams {
            Self::apply_physical_effects_for_team(team, &subbed_out_at, &subbed_in_at, data, now);
        }
    }

    fn apply_physical_effects_for_team(
        team: &FieldSquad,
        subbed_out_at: &HashMap<u32, u64>,
        subbed_in_at: &HashMap<u32, u64>,
        data: &mut SimulatorData,
        now: chrono::NaiveDate,
    ) {
        // Process starters
        for &player_id in &team.main {
            let minutes = if let Some(&out_time_ms) = subbed_out_at.get(&player_id) {
                (out_time_ms / 60000) as f32
            } else {
                90.0
            };

            Self::apply_player_physical_effects(player_id, minutes, data, now);
        }

        // Process used substitutes
        for &player_id in &team.substitutes_used {
            let minutes = if let Some(&in_time_ms) = subbed_in_at.get(&player_id) {
                90.0 - (in_time_ms / 60000) as f32
            } else {
                30.0 // fallback
            };

            Self::apply_player_physical_effects(player_id, minutes, data, now);
        }

        // Process unused substitutes — frustration
        for &player_id in &team.substitutes {
            if !team.substitutes_used.contains(&player_id) {
                if let Some(player) = data.player_mut(player_id) {
                    player.happiness.add_event(HappinessEventType::MatchDropped, -1.5);
                }
            }
        }
    }

    fn apply_player_physical_effects(
        player_id: u32,
        minutes: f32,
        data: &mut SimulatorData,
        now: chrono::NaiveDate,
    ) {
        if let Some(player) = data.player_mut(player_id) {
            let age = DateUtils::age(player.birth_date, now);
            let natural_fitness = player.skills.physical.natural_fitness;

            // 1. Condition floor enforcement
            // The match engine already drains condition during simulation.
            // Here we only enforce the FM-like minimum floor (30%).
            // A full 90 min match should leave players at ~55-70% condition.
            let condition_floor: i16 = 3000; // 30%
            if player.player_attributes.condition < condition_floor {
                player.player_attributes.condition = condition_floor;
            }

            // 2. Match readiness boost (minimum 15 minutes)
            if minutes >= 15.0 {
                let readiness_boost = minutes / 90.0 * 3.0;
                player.skills.physical.match_readiness =
                    (player.skills.physical.match_readiness + readiness_boost).min(20.0);
            }

            // 3. Jadedness accumulation
            if minutes > 60.0 {
                player.player_attributes.jadedness += 400;
            } else if minutes >= 30.0 {
                player.player_attributes.jadedness += 200;
            }

            if player.player_attributes.jadedness > 7000 {
                if !player.statuses.get().contains(&PlayerStatusType::Rst) {
                    player.statuses.add(now, PlayerStatusType::Rst);
                }
            }

            // 4. Reset days since last match
            player.player_attributes.days_since_last_match = 0;

            // 5. Match selection morale boost
            player.happiness.add_event(HappinessEventType::MatchSelection, 2.0);

            // 6. Match injury generation (~2.5% per 90 minutes base rate)
            if !player.player_attributes.is_injured {
                let injury_proneness = player.player_attributes.injury_proneness;
                let proneness_modifier = injury_proneness as f32 / 10.0;
                let condition_pct = player.player_attributes.condition_percentage();

                // Base injury chance: 0.5% per 90 minutes, scaled by minutes played
                let mut injury_chance: f32 = 0.005 * (minutes / 90.0);

                // Age >30: +0.1% per year over 30
                if age > 30 {
                    injury_chance += (age as f32 - 30.0) * 0.001;
                }

                // Low condition (<40%): +0.2-0.4%
                if condition_pct < 40 {
                    injury_chance += (40.0 - condition_pct as f32) * 0.0001;
                }

                // High jadedness (>7000): +0.2%
                if player.player_attributes.jadedness > 7000 {
                    injury_chance += 0.002;
                }

                // Low natural fitness (<8): +0.1%
                if natural_fitness < 8.0 {
                    injury_chance += 0.001;
                }

                // Injury proneness multiplier
                injury_chance *= proneness_modifier;

                // Recently recovered: +0.2% recurring injury risk
                if player.player_attributes.last_injury_body_part != 0 {
                    injury_chance += 0.002;
                }

                if rand::random::<f32>() < injury_chance {
                    let injury = InjuryType::random_match_injury(
                        minutes,
                        age,
                        condition_pct,
                        natural_fitness,
                        injury_proneness,
                    );
                    player.player_attributes.set_injury(injury);
                    player.statuses.add(now, PlayerStatusType::Inj);
                }
            }
        }
    }
}

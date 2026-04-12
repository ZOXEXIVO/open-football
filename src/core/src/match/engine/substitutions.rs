use crate::r#match::field::MatchField;
use crate::r#match::{MatchContext, MatchPlayer};
use crate::r#match::PlayerMatchEndStats;
use crate::{PlayerFieldPositionGroup, PlayerPositionType};

/// Process substitutions for both teams.
///
/// Two strategies:
/// 1. **Fatigue subs** — replace the most tired players
/// 2. **Development subs** — when winning comfortably, bring on bench players
///    who need match experience (loan players, youth, etc.)
pub fn process_substitutions(
    field: &mut MatchField,
    context: &mut MatchContext,
    max_subs_per_team: usize,
) {
    let team_ids = [field.home_team_id, field.away_team_id];

    for &team_id in &team_ids {
        if !context.can_substitute(team_id) {
            continue;
        }

        let has_bench = field.substitutes.iter().any(|p| p.team_id == team_id);
        if !has_bench {
            continue;
        }

        // Determine match situation for this team
        let (own_goals, opp_goals) = if team_id == context.field_home_team_id {
            (context.score.home_team.get() as i32, context.score.away_team.get() as i32)
        } else {
            (context.score.away_team.get() as i32, context.score.home_team.get() as i32)
        };
        let goal_diff = own_goals - opp_goals;
        let match_minutes = context.total_match_time / 60_000;

        // Collect outfield players sorted by condition (worst first)
        let mut candidates: Vec<(u32, i16, PlayerPositionType)> = field
            .players
            .iter()
            .filter(|p| p.team_id == team_id)
            .filter(|p| p.tactical_position.current_position != PlayerPositionType::Goalkeeper)
            .map(|p| (p.id, p.player_attributes.condition, p.tactical_position.current_position))
            .collect();

        candidates.sort_by_key(|&(_, cond, _)| cond);

        // Determine sub strategy based on match situation:
        // - Tired subs: always replace the most fatigued player
        // - Comfortable lead (2+ goals, 65+ min): also use development subs
        //   to give bench players match experience
        let comfortable_lead = goal_diff >= 2 && match_minutes >= 65;
        let late_comfort = goal_diff >= 3 && match_minutes >= 75;

        let mut subs_made = 0;

        // First: replace tired players (condition-based)
        for (player_out_id, condition, position) in &candidates {
            if subs_made >= max_subs_per_team || !context.can_substitute(team_id) {
                break;
            }

            // Only sub tired players (condition < 40%) or moderately tired
            // in comfortable situations (< 55%)
            let fatigue_threshold = if comfortable_lead { 5500 } else { 4000 };
            if *condition >= fatigue_threshold {
                continue;
            }

            let position_group = position.position_group();
            if let Some(player_in_id) = find_best_substitute(field, team_id, position_group) {
                if execute_substitution(field, context, team_id, *player_out_id, player_in_id) {
                    subs_made += 1;
                }
            }
        }

        // Second: development subs when winning comfortably.
        // Real coaches use comfortable leads to give bench players minutes —
        // especially loan players, youth players, and returning-from-injury players.
        if comfortable_lead && subs_made < max_subs_per_team && context.can_substitute(team_id) {
            // Find bench players who haven't played much (low games played)
            // and prefer those with higher ability (they deserve a chance)
            let mut dev_subs: Vec<(u32, u8, PlayerFieldPositionGroup)> = field
                .substitutes
                .iter()
                .filter(|p| p.team_id == team_id)
                .filter(|p| p.tactical_position.current_position.position_group() != PlayerFieldPositionGroup::Goalkeeper)
                .map(|p| (p.id, p.player_attributes.current_ability, p.tactical_position.current_position.position_group()))
                .collect();

            // Sort by ability descending — best bench players get chances first
            dev_subs.sort_by(|a, b| b.1.cmp(&a.1));

            let dev_sub_limit = if late_comfort { 2 } else { 1 };
            let mut dev_subs_made = 0;

            for (sub_id, _, sub_group) in &dev_subs {
                if dev_subs_made >= dev_sub_limit
                    || subs_made >= max_subs_per_team
                    || !context.can_substitute(team_id)
                {
                    break;
                }

                // Find the lowest-condition on-field player in a compatible position
                let player_out = candidates
                    .iter()
                    .filter(|(id, _, _)| {
                        // Must still be on the field (not already subbed)
                        field.get_player(*id).is_some()
                    })
                    .filter(|(_, _, pos)| {
                        let pg = pos.position_group();
                        pg == *sub_group
                            || matches!(
                                (pg, sub_group),
                                (PlayerFieldPositionGroup::Defender, PlayerFieldPositionGroup::Midfielder)
                                | (PlayerFieldPositionGroup::Midfielder, PlayerFieldPositionGroup::Defender)
                                | (PlayerFieldPositionGroup::Midfielder, PlayerFieldPositionGroup::Forward)
                                | (PlayerFieldPositionGroup::Forward, PlayerFieldPositionGroup::Midfielder)
                            )
                    })
                    .map(|(id, cond, _)| (*id, *cond))
                    .min_by_key(|&(_, cond)| cond);

                if let Some((out_id, _)) = player_out {
                    if execute_substitution(field, context, team_id, out_id, *sub_id) {
                        subs_made += 1;
                        dev_subs_made += 1;
                    }
                }
            }
        }
    }
}

/// Execute a single substitution: save stats, swap players, update context.
fn execute_substitution(
    field: &mut MatchField,
    context: &mut MatchContext,
    team_id: u32,
    player_out_id: u32,
    player_in_id: u32,
) -> bool {
    // Save subbed-out player's stats before they're replaced
    if let Some(player_out) = field.get_player(player_out_id) {
        let goals = player_out.statistics.goals_count();
        let assists = player_out.statistics.assists_count();

        context.substituted_out_stats.push((player_out_id, PlayerMatchEndStats {
            shots_on_target: player_out.memory.shots_on_target as u16,
            shots_total: player_out.memory.shots_taken as u16,
            passes_attempted: player_out.statistics.passes_attempted,
            passes_completed: player_out.statistics.passes_completed,
            tackles: player_out.statistics.tackles,
            interceptions: player_out.statistics.interceptions,
            saves: player_out.statistics.saves,
            goals,
            assists,
            match_rating: 0.0,
            xg: player_out.memory.xg_total,
            position_group: player_out.tactical_position.current_position.position_group(),
        }));
    }

    if !field.substitute_player(player_out_id, player_in_id) {
        return false;
    }

    context.record_substitution(team_id, player_out_id, player_in_id, context.total_match_time);
    context.players.remove_player(player_out_id);

    if let Some(field_player) = field.get_player(player_in_id) {
        context.players.update_player(player_in_id, field_player.clone());
    }

    let left_squad = field.left_side_players.as_mut();
    let right_squad = field.right_side_players.as_mut();
    if let Some(squad) = left_squad {
        if squad.team_id == team_id {
            squad.mark_substitute_used(player_in_id);
        }
    }
    if let Some(squad) = right_squad {
        if squad.team_id == team_id {
            squad.mark_substitute_used(player_in_id);
        }
    }

    true
}

fn find_best_substitute(
    field: &MatchField,
    team_id: u32,
    position_group: PlayerFieldPositionGroup,
) -> Option<u32> {
    let team_subs: Vec<&MatchPlayer> = field
        .substitutes
        .iter()
        .filter(|p| p.team_id == team_id)
        .collect();

    if team_subs.is_empty() {
        return None;
    }

    // Try to find a sub with matching position group
    let position_match = team_subs
        .iter()
        .filter(|p| p.tactical_position.current_position.position_group() == position_group)
        .max_by_key(|p| p.player_attributes.current_ability);

    if let Some(sub) = position_match {
        return Some(sub.id);
    }

    // Fallback: best available outfield sub (never use GK as outfield replacement)
    team_subs
        .iter()
        .filter(|p| p.tactical_position.current_position.position_group() != PlayerFieldPositionGroup::Goalkeeper)
        .max_by_key(|p| p.player_attributes.current_ability)
        .map(|p| p.id)
}

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
    // Roll for explicit in-match injuries first so the force-sub logic
    // downstream picks them up. A match can now produce genuine injury-
    // driven substitutions instead of waiting for condition to drift down
    // naturally.
    roll_in_match_injuries(field, context);

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

        // Collect outfield players sorted by condition (worst first).
        // Force-selected players are excluded — the manager pinned them in,
        // so neither fatigue rotation nor development subs are allowed to
        // pull them off. The injury-driven force-sub pass below runs over a
        // separate, unfiltered list so a critical condition still wins.
        let mut candidates: Vec<(u32, i16, PlayerPositionType)> = field
            .players
            .iter()
            .filter(|p| p.team_id == team_id)
            .filter(|p| p.tactical_position.current_position != PlayerPositionType::Goalkeeper)
            .filter(|p| !p.is_force_match_selection)
            .map(|p| (p.id, p.player_attributes.condition, p.tactical_position.current_position))
            .collect();

        candidates.sort_by_key(|&(_, cond, _)| cond);

        // Critical-injury candidates ignore the force-selection flag —
        // a sub-2000 condition models an in-match injury the coach can't
        // ignore even for a pinned player.
        let mut critical_candidates: Vec<(u32, i16, PlayerPositionType)> = field
            .players
            .iter()
            .filter(|p| p.team_id == team_id)
            .filter(|p| p.tactical_position.current_position != PlayerPositionType::Goalkeeper)
            .filter(|p| p.player_attributes.condition < 2000)
            .map(|p| (p.id, p.player_attributes.condition, p.tactical_position.current_position))
            .collect();

        critical_candidates.sort_by_key(|&(_, cond, _)| cond);

        // Determine sub strategy based on match situation:
        // - Tired subs: always replace the most fatigued player
        // - Comfortable lead (2+ goals, 65+ min): also use development subs
        //   to give bench players match experience
        let comfortable_lead = goal_diff >= 2 && match_minutes >= 65;
        let late_comfort = goal_diff >= 3 && match_minutes >= 75;

        let mut subs_made = 0;

        // Zero: force-sub critically-broken players regardless of score /
        // minute. Condition below 20% models an in-match injury the coach
        // can't ignore — a real manager pulls them straight off, even if
        // the player carries the manager's force-selection flag. Takes
        // priority over strategic fatigue rotation.
        const CRITICAL_CONDITION: i16 = 2000;
        for (player_out_id, _condition, position) in &critical_candidates {
            if subs_made >= max_subs_per_team || !context.can_substitute(team_id) {
                break;
            }
            let position_group = position.position_group();
            if let Some(player_in_id) = find_best_substitute(field, team_id, position_group) {
                if execute_substitution(field, context, team_id, *player_out_id, player_in_id) {
                    subs_made += 1;
                }
            }
        }

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

            // Skip — already handled in the injury pass above.
            if *condition < CRITICAL_CONDITION {
                continue;
            }

            let position_group = position.position_group();
            if let Some(player_in_id) = find_best_substitute(field, team_id, position_group) {
                if execute_substitution(field, context, team_id, *player_out_id, player_in_id) {
                    subs_made += 1;
                }
            }
        }

        // Tactical subs — if we're chasing the game late and have bench
        // options, bring on a fresh attacker for a tired defender/midfielder.
        // If we're hanging on late, swap an attacker for a defender.
        let chasing_late = goal_diff < 0 && match_minutes >= 65;
        let hanging_on_late = goal_diff > 0 && match_minutes >= 75 && !comfortable_lead;

        if (chasing_late || hanging_on_late)
            && subs_made < max_subs_per_team
            && context.can_substitute(team_id)
        {
            let need_group = if chasing_late {
                PlayerFieldPositionGroup::Forward
            } else {
                PlayerFieldPositionGroup::Defender
            };
            let sacrifice_group = if chasing_late {
                PlayerFieldPositionGroup::Defender
            } else {
                PlayerFieldPositionGroup::Forward
            };

            // Pick the fittest-but-non-critical bench player of need_group.
            let sub_in: Option<u32> = field
                .substitutes
                .iter()
                .filter(|p| p.team_id == team_id)
                .filter(|p| {
                    p.tactical_position.current_position.position_group() == need_group
                })
                .max_by_key(|p| p.player_attributes.current_ability)
                .map(|p| p.id);

            // Pick the most tired on-field player of sacrifice_group still active.
            let sub_out: Option<u32> = candidates
                .iter()
                .filter(|(id, _, _)| field.get_player(*id).is_some())
                .filter(|(_, _, pos)| pos.position_group() == sacrifice_group)
                .min_by_key(|(_, cond, _)| *cond)
                .map(|(id, _, _)| *id);

            if let (Some(in_id), Some(out_id)) = (sub_in, sub_out) {
                if execute_substitution(field, context, team_id, out_id, in_id) {
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

/// Per-tick in-match injury roll. A small per-player chance scaled by
/// jadedness, low condition, age, and low natural_fitness. When triggered,
/// condition is slammed down to 1500 — just below the CRITICAL_CONDITION
/// threshold (2000) so the next pass of the force-sub loop pulls the
/// player off. The actual injury type / recovery days are decided by the
/// post-match path (`on_match_exertion` rolls the injury from minutes +
/// existing proneness); this function only models the **in-match event**.
fn roll_in_match_injuries(field: &mut MatchField, context: &mut MatchContext) {
    let match_minute = context.total_match_time / 60_000;
    if match_minute < 5 {
        return; // No opening-minute theatre
    }

    let mut victims: Vec<u32> = Vec::new();

    for player in field.players.iter() {
        // Skip subs (they're not on the pitch) and goalkeepers (rarely
        // forced off for non-contact injury mid-match).
        if player.tactical_position.current_position
            == crate::PlayerPositionType::Goalkeeper
        {
            continue;
        }
        // Already destroyed condition — no extra work needed.
        if player.player_attributes.condition < 2000 {
            continue;
        }
        if player.is_sent_off {
            continue;
        }

        let jaded = (player.player_attributes.jadedness as f32 / 10_000.0).clamp(0.0, 1.0);
        let cond = (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0);
        let nat_fit = (player.skills.physical.natural_fitness / 20.0).clamp(0.1, 1.0);
        let minutes_factor = (match_minute as f32 / 90.0).clamp(0.0, 1.2);

        // Base rate per substitution window (~10-15 minutes between calls).
        // Starts at 0.0005 for a fresh prime player and climbs toward
        // 0.01 for a jaded, tired 35-year-old late in the match. This
        // delivers an injury roughly every 15-20 matches at the team
        // level, which matches real-world "one injury per match" noise.
        let base = 0.0005
            + jaded * 0.004
            + (1.0 - cond) * 0.003
            + (1.0 - nat_fit) * 0.002
            + minutes_factor * 0.001;

        if rand::random::<f32>() < base {
            victims.push(player.id);
        }
    }

    if !victims.is_empty() {
        context.record_stoppage_time(60_000 * victims.len() as u64);
    }

    for pid in victims {
        if let Some(p) = field.get_player_mut(pid) {
            // Smack the condition down — the critical-condition path in
            // `process_substitutions` will now pull them off on this tick.
            p.player_attributes.condition = 1500;
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
            shots_faced: player_out.statistics.shots_faced,
            goals,
            assists,
            match_rating: 0.0,
            xg: player_out.memory.xg_total,
            position_group: player_out.tactical_position.current_position.position_group(),
            fouls: player_out.fouls_committed as u16,
            yellow_cards: player_out.statistics.yellow_cards_count(),
            red_cards: player_out.statistics.red_cards_count(),
        }));
    }

    if !field.substitute_player(player_out_id, player_in_id) {
        return false;
    }

    context.record_substitution(team_id, player_out_id, player_in_id, context.total_match_time);
    context.record_stoppage_time(30_000);
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

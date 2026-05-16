use chrono::NaiveDate;

use crate::r#match::engine::coach::TacticalNeed;
use crate::r#match::field::MatchField;
use crate::r#match::{MatchContext, MatchPlayer};
use crate::{PlayerFieldPositionGroup, PlayerPositionType};

/// In-match youth-protection thresholds and the candidate predicate.
/// Encapsulates the "should this kid be hooked even though the manager
/// pinned him in?" decision so the substitution loop reads as one branch
/// and tests can pin behaviour without standing up a full
/// `MatchField`/`MatchContext` fixture.
///
/// Football model: real coaches pull young players off the pitch when
/// they look gone — even if a more experienced peer would be left on.
/// The thresholds match the in-game "Rst" status logic (jadedness > 7000
/// triggers the post-match rest flag) but apply during play so a 15yo
/// pinned into a top-flight first team still gets protected mid-match.
pub(super) struct YouthProtection;

impl YouthProtection {
    /// Condition (0-10000) below which an under-17 in a senior match is
    /// pulled off even if the manager force-selected them. Pushing a kid
    /// through 90 minutes on a dangerously low tank is how you end
    /// careers — the protection takes precedence over a roster pin.
    const CONDITION_THRESHOLD: i16 = 4500;

    /// Jadedness (0-10000) above which an under-17 is hooked for
    /// protection. Roughly the "Rst" threshold but applied during the
    /// match instead of post-match.
    const JADEDNESS_THRESHOLD: i16 = 8500;

    /// Age at or below which the protection thresholds override the
    /// manager's force-selection flag. 17 is the FIFA-standard age at
    /// which a player can be registered for senior international
    /// football, but until then the body is still maturing — and the
    /// in-match guardrails follow the body, not the paperwork.
    const MAX_AGE: u8 = 17;

    /// True when the player is young enough and drained enough that the
    /// force-selection flag should be ignored in favour of pulling him
    /// off the pitch. Goalkeepers and players already in the critical
    /// pass (condition < 2000) are excluded so the predicate does not
    /// double-fire.
    pub(super) fn is_candidate(player: &MatchPlayer, today: NaiveDate) -> bool {
        if player.tactical_position.current_position == PlayerPositionType::Goalkeeper {
            return false;
        }
        if player.player_attributes.condition < 2000 {
            return false; // critical pass owns it
        }
        if player.age_at(today) > Self::MAX_AGE {
            return false;
        }
        player.player_attributes.condition < Self::CONDITION_THRESHOLD
            || player.player_attributes.jadedness > Self::JADEDNESS_THRESHOLD
    }
}

/// Process substitutions for both teams.
///
/// Three strategies, in priority order:
/// 0. **Critical injury** — anyone (force-selected or not) with condition
///    < 2000 is pulled off; under-17 protection runs alongside.
/// 1. **Fatigue subs** — replace the most tired players
/// 2. **Development subs** — when winning comfortably, bring on bench players
///    who need match experience (loan players, youth, etc.)
pub fn process_substitutions(
    field: &mut MatchField,
    context: &mut MatchContext,
    max_subs_per_team: usize,
    today: NaiveDate,
) {
    // Roll for explicit in-match injuries first so the force-sub logic
    // downstream picks them up. A match can now produce genuine injury-
    // driven substitutions instead of waiting for condition to drift down
    // naturally.
    Substitutions::roll_in_match_injuries(field, context);

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
            (
                context.score.home_team.get() as i32,
                context.score.away_team.get() as i32,
            )
        } else {
            (
                context.score.away_team.get() as i32,
                context.score.home_team.get() as i32,
            )
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
            .map(|p| {
                (
                    p.id,
                    p.player_attributes.condition,
                    p.tactical_position.current_position,
                )
            })
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
            .map(|p| {
                (
                    p.id,
                    p.player_attributes.condition,
                    p.tactical_position.current_position,
                )
            })
            .collect();

        critical_candidates.sort_by_key(|&(_, cond, _)| cond);

        // Youth-protection candidates: under-17 players whose condition or
        // jadedness has dropped into the danger band. The force-selection
        // flag does not protect them — a manager who pinned a 15-year-old
        // into the XI cannot keep him on the pitch at 35% condition.
        // Goalkeepers stay on the pitch (substitute keepers are usually a
        // bigger risk than a tired starter). The predicate is extracted
        // to a helper so tests can pin its behaviour without standing up
        // a full MatchField/MatchContext fixture.
        let mut youth_protection_candidates: Vec<(u32, i16, PlayerPositionType)> = field
            .players
            .iter()
            .filter(|p| p.team_id == team_id)
            .filter(|p| YouthProtection::is_candidate(p, today))
            .map(|p| {
                (
                    p.id,
                    p.player_attributes.condition,
                    p.tactical_position.current_position,
                )
            })
            .collect();

        youth_protection_candidates.sort_by_key(|&(_, cond, _)| cond);

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
            if let Some(player_in_id) =
                Substitutions::find_best_substitute(field, team_id, position_group)
            {
                if Substitutions::execute_substitution(
                    field,
                    context,
                    team_id,
                    *player_out_id,
                    player_in_id,
                ) {
                    subs_made += 1;
                }
            }
        }

        // Zero-and-a-half: youth-protection. A 15-year-old at 35% condition
        // or 90%+ jadedness is hooked even when the manager pinned him in.
        // Real coaches pull young players off the pitch when they look
        // gone — the force-selection flag should not be a back door for
        // overloading a body that hasn't finished growing.
        for (player_out_id, _condition, position) in &youth_protection_candidates {
            if subs_made >= max_subs_per_team || !context.can_substitute(team_id) {
                break;
            }
            // Don't double-process if the critical pass already pulled them.
            if field.get_player(*player_out_id).is_none() {
                continue;
            }
            let position_group = position.position_group();
            if let Some(player_in_id) =
                Substitutions::find_best_substitute(field, team_id, position_group)
            {
                if Substitutions::execute_substitution(
                    field,
                    context,
                    team_id,
                    *player_out_id,
                    player_in_id,
                ) {
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
            if let Some(player_in_id) =
                Substitutions::find_best_substitute(field, team_id, position_group)
            {
                if Substitutions::execute_substitution(
                    field,
                    context,
                    team_id,
                    *player_out_id,
                    player_in_id,
                ) {
                    subs_made += 1;
                }
            }
        }

        // Tactical sub driven by `TacticalNeed::from_state` instead of a
        // binary chasing/hanging-on flag. Each need maps to a target
        // position group to bring on AND a sacrifice group to pull off,
        // so the bench actually responds to *why* the team is
        // struggling — not just the scoreline. The `Fatigue` need is
        // skipped because the condition-based pass above already
        // handles that case.
        let need = if match_minutes >= 55 {
            let progress =
                (context.total_match_time as f32 / crate::r#match::MATCH_TIME_MS as f32).min(1.0);
            let coach = context.coach_for_team(team_id);
            let condition_avg = field
                .players
                .iter()
                .filter(|p| p.team_id == team_id)
                .map(|p| p.player_attributes.condition as f32 / 10000.0)
                .sum::<f32>()
                / field
                    .players
                    .iter()
                    .filter(|p| p.team_id == team_id)
                    .count()
                    .max(1) as f32;
            Some(TacticalNeed::from_state(
                goal_diff as i8,
                progress,
                condition_avg,
                coach.metrics,
            ))
        } else {
            None
        };

        if let Some(need) = need {
            let (need_group, sacrifice_group) = match need {
                TacticalNeed::Chasing => (
                    PlayerFieldPositionGroup::Forward,
                    PlayerFieldPositionGroup::Defender,
                ),
                TacticalNeed::ProtectingLead => (
                    PlayerFieldPositionGroup::Defender,
                    PlayerFieldPositionGroup::Forward,
                ),
                TacticalNeed::LosingMidfield | TacticalNeed::BeingPressed => (
                    PlayerFieldPositionGroup::Midfielder,
                    PlayerFieldPositionGroup::Forward,
                ),
                TacticalNeed::NeedingCrosses => (
                    PlayerFieldPositionGroup::Midfielder,
                    PlayerFieldPositionGroup::Defender,
                ),
                // Fatigue is owned by the condition-based pass above —
                // no extra tactical sub here.
                TacticalNeed::Fatigue => (
                    PlayerFieldPositionGroup::Goalkeeper,
                    PlayerFieldPositionGroup::Goalkeeper,
                ),
            };
            let do_swap = !matches!(need, TacticalNeed::Fatigue);

            if do_swap && subs_made < max_subs_per_team && context.can_substitute(team_id) {
                // Coherence guard: don't pull a player from a group
                // the live tactic only has one of (e.g. a 4-1-4-1
                // protecting a lead has just ONE forward; sacrificing
                // it leaves the side with no out-ball). When the
                // chosen sacrifice group is too thin in the current
                // shape, we either fall back to the next-thickest
                // attacking-style group or skip the swap entirely.
                //
                // Limitation: this engine doesn't redraw shape on
                // sub. The tactical-shape probe in
                // `evaluate_situational_shape` and this position-
                // group sub are independent — we keep them coherent
                // by group, not by formation slot.
                let count_in_group = |group: PlayerFieldPositionGroup| {
                    field
                        .players
                        .iter()
                        .filter(|p| p.team_id == team_id && !p.is_sent_off)
                        .filter(|p| p.tactical_position.current_position.position_group() == group)
                        .count()
                };
                let sacrifice_supply = count_in_group(sacrifice_group);
                // Refuse to pull the last forward / last defender —
                // sub_out_group must keep at least 1 in the group
                // so the side stays balanced enough to play.
                let effective_sacrifice = if sacrifice_supply > 1 {
                    Some(sacrifice_group)
                } else {
                    // Try midfield as the fallback sacrifice group:
                    // it's almost always the best-stocked area.
                    let mid_supply = count_in_group(PlayerFieldPositionGroup::Midfielder);
                    if mid_supply > 2 {
                        Some(PlayerFieldPositionGroup::Midfielder)
                    } else {
                        None
                    }
                };

                if let Some(actual_sacrifice) = effective_sacrifice {
                    let sub_in: Option<u32> = field
                        .substitutes
                        .iter()
                        .filter(|p| p.team_id == team_id)
                        .filter(|p| {
                            p.tactical_position.current_position.position_group() == need_group
                        })
                        .max_by_key(|p| Substitutions::role_score_for_need(p, need))
                        .map(|p| p.id);

                    let sub_out: Option<u32> = candidates
                        .iter()
                        .filter(|(id, _, _)| field.get_player(*id).is_some())
                        .filter(|(_, _, pos)| pos.position_group() == actual_sacrifice)
                        .min_by_key(|(_, cond, _)| *cond)
                        .map(|(id, _, _)| *id);

                    if let (Some(in_id), Some(out_id)) = (sub_in, sub_out) {
                        if Substitutions::execute_substitution(
                            field, context, team_id, out_id, in_id,
                        ) {
                            subs_made += 1;
                        }
                    }
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
                .filter(|p| {
                    p.tactical_position.current_position.position_group()
                        != PlayerFieldPositionGroup::Goalkeeper
                })
                .map(|p| {
                    (
                        p.id,
                        p.player_attributes.current_ability,
                        p.tactical_position.current_position.position_group(),
                    )
                })
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
                                (
                                    PlayerFieldPositionGroup::Defender,
                                    PlayerFieldPositionGroup::Midfielder
                                ) | (
                                    PlayerFieldPositionGroup::Midfielder,
                                    PlayerFieldPositionGroup::Defender
                                ) | (
                                    PlayerFieldPositionGroup::Midfielder,
                                    PlayerFieldPositionGroup::Forward
                                ) | (
                                    PlayerFieldPositionGroup::Forward,
                                    PlayerFieldPositionGroup::Midfielder
                                )
                            )
                    })
                    .map(|(id, cond, _)| (*id, *cond))
                    .min_by_key(|&(_, cond)| cond);

                if let Some((out_id, _)) = player_out {
                    if Substitutions::execute_substitution(field, context, team_id, out_id, *sub_id)
                    {
                        subs_made += 1;
                        dev_subs_made += 1;
                    }
                }
            }
        }
    }
}

/// Match-side helpers grouped under one namespace. The free-function
/// versions of these helpers all lived at module scope; bundling them
/// under a struct keeps `process_substitutions` readable, lets tests
/// reach in via stable `Substitutions::xxx` paths, and gives the file a
/// single place to grow per-difficulty / per-rule-set knobs later.
pub(super) struct Substitutions;

impl Substitutions {
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
            if player.tactical_position.current_position == crate::PlayerPositionType::Goalkeeper {
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
        // Save subbed-out player's stats before they're replaced. Minutes
        // are computed from the player's entry tick so a 60th-minute sub-
        // off correctly records ~60 minutes (or less, if the player came
        // on after kickoff).
        if let Some(player_out) = field.get_player(player_out_id) {
            let minutes = player_out.minutes_played_at(context.total_match_time);
            let snapshot = player_out.to_match_end_stats(minutes);
            context
                .substituted_out_stats
                .push((player_out_id, snapshot));
            // Capture the physical snapshot BEFORE the swap so the
            // post-match exertion path can size the persisted condition
            // drop from the actual in-match drain, not the minute count
            // alone. Stamped at the moment the player leaves the pitch;
            // a 60th-minute sub at 5500 condition imprints "you were
            // 5500 at the 60th minute" forever, even though the same
            // shirt belongs to a fresh sub for the rest of the match.
            let phys_snapshot = player_out.to_physical_snapshot(context.total_match_time);
            context
                .substituted_out_physical_snapshots
                .push(phys_snapshot);
        }

        if !field.substitute_player(player_out_id, player_in_id) {
            return false;
        }

        // Stamp the incoming player's entry tick so their end-of-match
        // minute count reflects only the time they were actually on the
        // pitch. Also re-stamp `starting_condition` to the value the
        // sub is bringing onto the pitch — without this, the engine
        // would compare their final energy against a kickoff-time
        // starting_condition that was never relevant for this player
        // (a sub coming on at 80 min started AT 80 min, not at
        // kickoff).
        //
        // Note: `starting_recovery_debt` is intentionally NOT re-stamped
        // here. The field is copied once from `Player::load::recovery_debt`
        // when the `MatchPlayer` is built (see
        // `MatchPlayer::from_player`), and nothing during the match
        // mutates `MatchPlayer::starting_recovery_debt` between
        // construction and the substitution swap. The bench-warming
        // sub's `starting_recovery_debt` therefore already holds the
        // pre-match persisted value — re-stamping from
        // `player_in.player_attributes.condition` would not make sense
        // (those are different scales), and reaching back through
        // `PlayerLoad` would duplicate the work already done at squad
        // build time. If a future change ever lets bench-debt drift
        // during a match, this is the place that needs to learn how to
        // pull the authoritative value.
        if let Some(player_in) = field.get_player_mut(player_in_id) {
            player_in.entry_match_time_ms = context.total_match_time;
            player_in.starting_condition = player_in.player_attributes.condition;
        }

        context.record_substitution(
            team_id,
            player_out_id,
            player_in_id,
            context.total_match_time,
        );
        context.record_stoppage_time(30_000);
        context.players.remove_player(player_out_id);

        if let Some(field_player) = field.get_player(player_in_id) {
            context
                .players
                .update_player(player_in_id, field_player.clone());
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

    /// Per-need bench scoring. The legacy code picked the highest-CA
    /// bench player of the position group; that ignored *why* the
    /// coach wanted a sub. Now each `TacticalNeed` weights different
    /// attributes — composure for "BeingPressed", crossing for
    /// "NeedingCrosses", passing/vision for "LosingMidfield". Returns
    /// an integer score so `max_by_key` stays valid.
    pub(super) fn role_score_for_need(player: &MatchPlayer, need: TacticalNeed) -> u32 {
        let ca = player.player_attributes.current_ability as u32;
        let bonus = match need {
            TacticalNeed::Chasing => {
                let finishing = player.skills.technical.finishing as u32;
                let pace = player.skills.physical.pace as u32;
                finishing * 4 + pace * 3
            }
            TacticalNeed::ProtectingLead => {
                let marking = player.skills.technical.marking as u32;
                let tackling = player.skills.technical.tackling as u32;
                let positioning = player.skills.mental.positioning as u32;
                marking * 3 + tackling * 3 + positioning * 2
            }
            TacticalNeed::LosingMidfield => {
                let passing = player.skills.technical.passing as u32;
                let vision = player.skills.mental.vision as u32;
                let work_rate = player.skills.mental.work_rate as u32;
                passing * 3 + vision * 3 + work_rate * 2
            }
            TacticalNeed::BeingPressed => {
                let composure = player.skills.mental.composure as u32;
                let first_touch = player.skills.technical.first_touch as u32;
                let passing = player.skills.technical.passing as u32;
                composure * 4 + first_touch * 3 + passing * 2
            }
            TacticalNeed::NeedingCrosses => {
                let crossing = player.skills.technical.crossing as u32;
                let dribbling = player.skills.technical.dribbling as u32;
                let pace = player.skills.physical.pace as u32;
                crossing * 4 + dribbling * 2 + pace * 2
            }
            TacticalNeed::Fatigue => 0,
        };
        ca * 10 + bonus
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
            .filter(|p| {
                p.tactical_position.current_position.position_group()
                    != PlayerFieldPositionGroup::Goalkeeper
            })
            .max_by_key(|p| p.player_attributes.current_ability)
            .map(|p| p.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositions, PlayerSkills,
    };

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn build_match_player(birth: NaiveDate, pos: PlayerPositionType) -> MatchPlayer {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = 9000;
        attrs.jadedness = 1000;
        let player = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(birth)
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: pos,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(1, &player, pos, false)
    }

    fn today() -> NaiveDate {
        d(2025, 6, 1)
    }

    #[test]
    fn youth_protection_fires_for_under_17_with_low_condition() {
        let mut p = build_match_player(d(2010, 1, 1), PlayerPositionType::ForwardLeft); // 15
        p.player_attributes.condition = 4000;
        assert!(YouthProtection::is_candidate(&p, today()));
    }

    #[test]
    fn youth_protection_fires_for_under_17_with_high_jadedness() {
        let mut p = build_match_player(d(2009, 1, 1), PlayerPositionType::MidfielderCenter); // 16
        p.player_attributes.condition = 6000;
        p.player_attributes.jadedness = 9000;
        assert!(YouthProtection::is_candidate(&p, today()));
    }

    #[test]
    fn youth_protection_silent_for_under_17_with_normal_condition() {
        let mut p = build_match_player(d(2010, 1, 1), PlayerPositionType::ForwardLeft);
        p.player_attributes.condition = 7000;
        p.player_attributes.jadedness = 3000;
        assert!(!YouthProtection::is_candidate(&p, today()));
    }

    #[test]
    fn youth_protection_silent_for_18yo_with_low_condition() {
        let mut p = build_match_player(d(2007, 1, 1), PlayerPositionType::ForwardLeft); // 18
        p.player_attributes.condition = 3000;
        assert!(!YouthProtection::is_candidate(&p, today()));
    }

    #[test]
    fn youth_protection_skips_goalkeepers() {
        let mut p = build_match_player(d(2010, 1, 1), PlayerPositionType::Goalkeeper);
        p.player_attributes.condition = 3000;
        assert!(!YouthProtection::is_candidate(&p, today()));
    }

    #[test]
    fn youth_protection_skips_critical_pass_owners() {
        // Below 2000 → critical-injury pass owns it; predicate must
        // defer to that to avoid double-firing.
        let mut p = build_match_player(d(2010, 1, 1), PlayerPositionType::ForwardLeft);
        p.player_attributes.condition = 1500;
        assert!(!YouthProtection::is_candidate(&p, today()));
    }

    #[test]
    fn youth_protection_fires_even_when_force_selected() {
        // The force-selection flag must NOT make the predicate skip the
        // player — that's the whole point.
        let mut p = build_match_player(d(2010, 1, 1), PlayerPositionType::ForwardLeft);
        p.player_attributes.condition = 4000;
        p.is_force_match_selection = true;
        assert!(YouthProtection::is_candidate(&p, today()));
    }

    /// Sub-test for the comment in [`super`] explaining that
    /// `starting_recovery_debt` is NOT re-stamped at substitution
    /// time — the bench-warm value the `MatchPlayer` was built with
    /// IS the value the engine should read for the sub's in-match
    /// drain. This test pins that contract: two identical incoming
    /// subs with the only difference being `starting_recovery_debt`
    /// must drain condition at materially different rates once on
    /// the pitch. If a future change ever zeroed out the field at
    /// swap time (or accidentally re-stamped it to 0.0), this test
    /// would catch it.
    #[test]
    fn higher_starting_recovery_debt_drains_condition_faster_in_match() {
        use crate::r#match::ConditionContext;
        use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
        use nalgebra::Vector3;

        let mut fresh =
            build_match_player(d(1998, 1, 1), PlayerPositionType::MidfielderCenter);
        let mut heavy_legs =
            build_match_player(d(1998, 1, 1), PlayerPositionType::MidfielderCenter);

        // Identical condition / stamina / NF / jadedness on both —
        // only `starting_recovery_debt` differs. Sub coming on with
        // fresh legs vs sub coming on with chronic accumulated debt.
        fresh.player_attributes.condition = 9_000;
        heavy_legs.player_attributes.condition = 9_000;
        fresh.starting_recovery_debt = 50.0;
        heavy_legs.starting_recovery_debt = 1_400.0;

        // Set both moving at a clearly-running pace so the
        // ConditionProcessor's positive-fatigue branch (where the
        // debt multiplier actually applies) fires. The exact speed
        // isn't load-bearing, just has to land above the running
        // threshold for the `low_fatigue / moderate / high` curve.
        let running_speed = 6.0;
        fresh.velocity = Vector3::new(running_speed, 0.0, 0.0);
        heavy_legs.velocity = Vector3::new(running_speed, 0.0, 0.0);

        // Run a comparable number of ticks through the condition
        // processor — enough that the per-tick deltas accumulate
        // into a measurable difference. The processor reads
        // `starting_recovery_debt` every tick, so the
        // heavy-legs sub should accumulate a larger fatigue
        // total.
        for tick in 0..400 {
            let fresh_ctx = ConditionContext {
                in_state_time: tick,
                player: &mut fresh,
                match_progress: 0.5,
            };
            DefenderCondition::new(ActivityIntensity::High).process(fresh_ctx);

            let heavy_ctx = ConditionContext {
                in_state_time: tick,
                player: &mut heavy_legs,
                match_progress: 0.5,
            };
            DefenderCondition::new(ActivityIntensity::High).process(heavy_ctx);
        }

        assert!(
            heavy_legs.player_attributes.condition < fresh.player_attributes.condition,
            "heavy-legs sub ({}) must drain faster than fresh sub ({})",
            heavy_legs.player_attributes.condition,
            fresh.player_attributes.condition
        );
        // And the gap must be meaningful — a 1-point drift would
        // technically pass `<` but wouldn't reflect the design
        // intent. The debt_mult curve produces a 1.0..1.35 range,
        // so a few hundred ticks at full running should leave a
        // visible gap.
        let gap = fresh.player_attributes.condition - heavy_legs.player_attributes.condition;
        assert!(
            gap >= 30,
            "fresh-vs-heavy gap {} too small — starting_recovery_debt isn't moving the needle",
            gap
        );
    }
}

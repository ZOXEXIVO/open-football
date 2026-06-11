use chrono::NaiveDate;

use crate::club::staff::{CoachDecisionEngine, CoachLiveMatchContext};
use crate::r#match::engine::coach::TacticalNeed;
use crate::r#match::engine::sub_scoring::{LiveSubstitutionStats, SubScoring};
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
///    < 2000 is pulled off; under-17 protection runs alongside. These
///    bypass `allowed_in_window` and ignore star protection.
/// 1. **Discretionary scored-pair subs** — fatigue / tactical /
///    development cases are evaluated as `(out, in)` pairs using
///    [`sub_off_score_protected`] + [`sub_in_score`] so goal scorers and
///    high-rated starters are protected unless the case for removing
///    them is strong (extreme fatigue, comfortable late lead, or a
///    decisive tactical need). The window gate from
///    [`allowed_in_window`] applies — discretionary subs only fire in
///    real-football timing bands.
pub fn process_substitutions(
    field: &mut MatchField,
    context: &mut MatchContext,
    max_subs_per_team: usize,
    today: NaiveDate,
) {
    // Take the snapshots by `mem::take`. The substitution layer needs
    // an immutable borrow of the snapshot to build the
    // `CoachDecisionEngine`, while `process_with_coaches` needs a
    // mutable borrow of `field` for the actual swap. Borrowing the
    // snapshot back at the end of the call (so a future substitution
    // pass on the same field still sees it) avoids cloning the memory
    // map twice per match. Empty snapshots (tests / dev_match /
    // wire-format reconstruction) keep the legacy memory-less path.
    let home_snapshot = std::mem::take(&mut field.home_coach_snapshot);
    let away_snapshot = std::mem::take(&mut field.away_coach_snapshot);
    let home_engine = home_snapshot
        .as_ref()
        .map(|s| CoachDecisionEngine::new(&s.memory, &s.profile, s.strategy));
    let away_engine = away_snapshot
        .as_ref()
        .map(|s| CoachDecisionEngine::new(&s.memory, &s.profile, s.strategy));
    Substitutions::process_with_coaches(
        field,
        context,
        max_subs_per_team,
        today,
        home_engine.as_ref(),
        away_engine.as_ref(),
    );
    drop(home_engine);
    drop(away_engine);
    field.home_coach_snapshot = home_snapshot;
    field.away_coach_snapshot = away_snapshot;
}

/// Match-side helpers grouped under one namespace. The free-function
/// versions of these helpers all lived at module scope; bundling them
/// under a struct keeps `process_substitutions` readable, lets tests
/// reach in via stable `Substitutions::xxx` paths, and gives the file a
/// single place to grow per-difficulty / per-rule-set knobs later.
pub(super) struct Substitutions;

impl Substitutions {
    /// Internal entry point that carries optional coach decision
    /// engines per side. Mirrors the public `process_substitutions`
    /// signature but adds the two `Option<&CoachDecisionEngine>`
    /// handles — used by the public wrapper above (with `None`) and
    /// by tests / wiring code that already holds the head coach.
    pub(crate) fn process_with_coaches(
        field: &mut MatchField,
        context: &mut MatchContext,
        max_subs_per_team: usize,
        today: NaiveDate,
        home_coach: Option<&CoachDecisionEngine<'_>>,
        away_coach: Option<&CoachDecisionEngine<'_>>,
    ) {
        Self::process_inner(
            field,
            context,
            max_subs_per_team,
            today,
            home_coach,
            away_coach,
        );
    }

    fn process_inner(
        field: &mut MatchField,
        context: &mut MatchContext,
        max_subs_per_team: usize,
        today: NaiveDate,
        home_coach: Option<&CoachDecisionEngine<'_>>,
        away_coach: Option<&CoachDecisionEngine<'_>>,
    ) {
        // Roll for explicit in-match injuries first so the force-sub logic
        // downstream picks them up.
        Self::roll_in_match_injuries(field, context);

        let team_ids = [field.home_team_id, field.away_team_id];

        for &team_id in &team_ids {
            if !context.can_substitute(team_id) {
                continue;
            }
            if !field.substitutes.iter().any(|p| p.team_id == team_id) {
                continue;
            }

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
            let coach = if team_id == context.field_home_team_id {
                home_coach
            } else {
                away_coach
            };

            // Critical and youth-protection candidates (same as the legacy path).
            let mut critical_candidates: Vec<(u32, i16, PlayerPositionType)> = field
                .players
                .iter()
                .filter(|p| p.team_id == team_id)
                .filter(|p| {
                    p.tactical_position.current_position != PlayerPositionType::Goalkeeper
                })
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

            let comfortable_lead = goal_diff >= 2 && match_minutes >= 65;
            let late_comfort = goal_diff >= 3 && match_minutes >= 75;

            let mut subs_made = 0;

            for (player_out_id, _condition, position) in &critical_candidates {
                if subs_made >= max_subs_per_team || !context.can_substitute(team_id) {
                    break;
                }
                let position_group = position.position_group();
                if let Some(player_in_id) =
                    Self::find_best_substitute(field, team_id, position_group)
                {
                    if Self::execute_substitution(
                        field,
                        context,
                        team_id,
                        *player_out_id,
                        player_in_id,
                        crate::r#match::engine::flow::result::SubstitutionReason::CriticalInjury,
                    ) {
                        subs_made += 1;
                    }
                }
            }

            for (player_out_id, _condition, position) in &youth_protection_candidates {
                if subs_made >= max_subs_per_team || !context.can_substitute(team_id) {
                    break;
                }
                if field.get_player(*player_out_id).is_none() {
                    continue;
                }
                let position_group = position.position_group();
                if let Some(player_in_id) =
                    Self::find_best_substitute(field, team_id, position_group)
                {
                    if Self::execute_substitution(
                        field,
                        context,
                        team_id,
                        *player_out_id,
                        player_in_id,
                        crate::r#match::engine::flow::result::SubstitutionReason::YouthProtection,
                    ) {
                        subs_made += 1;
                    }
                }
            }

            let need = if match_minutes >= 55 {
                let progress = (context.total_match_time as f32
                    / crate::r#match::MATCH_TIME_MS as f32)
                    .min(1.0);
                let match_coach = context.coach_for_team(team_id);
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
                TacticalNeed::from_state(goal_diff as i8, progress, condition_avg, match_coach.metrics)
            } else {
                TacticalNeed::Fatigue
            };

            let match_minute_u32 = match_minutes as u32;
            loop {
                if subs_made >= max_subs_per_team || !context.can_substitute(team_id) {
                    break;
                }
                let used = context.subs_used_by_team(team_id) as u8;
                if !SubScoring::allowed_in_window(used, match_minute_u32, false) {
                    break;
                }

                let (threshold, protection_dampening) = if late_comfort {
                    (0.60, 0.5)
                } else if comfortable_lead {
                    (0.70, 0.75)
                } else {
                    (0.85, 1.0)
                };

                let chosen = Self::best_discretionary_pair_with_coach(
                    field,
                    team_id,
                    need,
                    own_goals as u8,
                    opp_goals as u8,
                    context.total_match_time,
                    today,
                    protection_dampening,
                    threshold,
                    coach,
                );

                match chosen {
                    Some((out_id, in_id)) => {
                        if !Self::execute_substitution(
                            field,
                            context,
                            team_id,
                            out_id,
                            in_id,
                            crate::r#match::engine::flow::result::SubstitutionReason::Discretionary,
                        ) {
                            break;
                        }
                        subs_made += 1;
                    }
                    None => break,
                }
            }
        }
    }
}

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
            if player.tactical_position.current_position == PlayerPositionType::Goalkeeper {
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
            // Floor lowered 0.10 → 0.02 so a sub-5 natural_fitness
            // player is meaningfully more injury-prone than a 10/20.
            let nat_fit = (player.skills.physical.natural_fitness / 20.0).clamp(0.02, 1.0);
            let minutes_factor = (match_minute as f32 / 90.0).clamp(0.0, 1.2);

            // Base rate per substitution window (~10-15 minutes between calls).
            // Starts at 0.0005 for a fresh prime player and climbs toward
            // 0.01 for a jaded, tired 35-year-old late in the match. This
            // delivers an injury roughly every 15-20 matches at the team
            // level, which matches real-world "one injury per match" noise.
            let mut base = 0.0005
                + jaded * 0.004
                + (1.0 - cond) * 0.003
                + (1.0 - nat_fit) * 0.002
                + minutes_factor * 0.001;
            // Environment shifts injury baseline — heavy rain, muddy pitch,
            // cold pitch all raise risk. The env modifier is clamped 0..0.1
            // and acts as an additive bump on top of the per-player rate.
            base += context
                .environment
                .modifiers()
                .injury_risk
                .clamp(0.0, 0.1);

            if context.rng.unit_f32() < base {
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
        reason: crate::r#match::engine::flow::result::SubstitutionReason,
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
            reason,
        );
        context.record_stoppage_time(30_000);
        context.players.remove_player(player_out_id);
        // Active XI changed — invalidate cached per-team skill
        // composites so the next tactical refresh re-walks the
        // roster.
        context.invalidate_skill_aggregates();

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

    /// Position-fit score in [0.0, 1.0] for putting `sub` into the slot
    /// vacated by `out`. Exact position-group match → 1.0; adjacent
    /// groups (DEF↔MID, MID↔FWD) get partial credit; cross-group fits
    /// (DEF↔FWD) are heavily discounted.
    fn position_fit(out: &MatchPlayer, sub: &MatchPlayer) -> f32 {
        let out_group = out.tactical_position.current_position.position_group();
        let sub_group = sub.tactical_position.current_position.position_group();
        if sub_group == PlayerFieldPositionGroup::Goalkeeper {
            return 0.0;
        }
        if out_group == sub_group {
            return 1.0;
        }
        use PlayerFieldPositionGroup::*;
        match (out_group, sub_group) {
            (Midfielder, Forward) | (Forward, Midfielder) => 0.65,
            (Defender, Midfielder) | (Midfielder, Defender) => 0.55,
            (Defender, Forward) | (Forward, Defender) => 0.25,
            _ => 0.30,
        }
    }

    /// Crude development-priority signal in [0.0, 1.0]. Young bench
    /// players score higher — the engine doesn't currently track
    /// per-player matches-played for the in-match decision, so age is
    /// the cleanest available proxy for "this player needs minutes".
    fn development_priority(sub: &MatchPlayer, today: NaiveDate) -> f32 {
        let age = sub.age_at(today);
        if age <= 19 {
            1.0
        } else if age <= 22 {
            0.6
        } else if age <= 25 {
            0.2
        } else {
            0.0
        }
    }

    /// Disruption penalty for hollowing out a thin position group.
    /// Returns ∞ when removing the player would leave the group empty
    /// (so the pair is impossible) and a modest penalty when only one
    /// would remain. Keepers are always treated as fixed.
    fn disruption_penalty(
        field: &MatchField,
        team_id: u32,
        out: &MatchPlayer,
        sub: &MatchPlayer,
    ) -> f32 {
        let out_group = out.tactical_position.current_position.position_group();
        let sub_group = sub.tactical_position.current_position.position_group();

        // If we're replacing with the same group, the swap doesn't
        // change shape supply — no disruption.
        if sub_group == out_group {
            return 0.0;
        }

        let in_group_count = field
            .players
            .iter()
            .filter(|p| p.team_id == team_id && !p.is_sent_off)
            .filter(|p| p.tactical_position.current_position.position_group() == out_group)
            .count();

        // Removing the last representative of a group → forbidden.
        if in_group_count <= 1 {
            return f32::INFINITY;
        }
        // Reducing a thin group (down to 1) → measurable penalty.
        if in_group_count == 2 {
            return 0.25;
        }
        0.0
    }

    /// Small additive bonus when the swap shape matches the tactical
    /// need — chasing pulls a defender for a forward, protecting a
    /// lead pulls a forward for a defender, etc. The point of this
    /// bonus is to break ties between equally tired pairs in favour
    /// of the one that actually shifts the team toward the need.
    fn tactical_fit_bonus(out: &MatchPlayer, sub: &MatchPlayer, need: TacticalNeed) -> f32 {
        let out_group = out.tactical_position.current_position.position_group();
        let sub_group = sub.tactical_position.current_position.position_group();
        use PlayerFieldPositionGroup::*;
        match need {
            TacticalNeed::Chasing => {
                if sub_group == Forward && matches!(out_group, Defender | Midfielder) {
                    0.15
                } else {
                    0.0
                }
            }
            TacticalNeed::ProtectingLead => {
                if sub_group == Defender && matches!(out_group, Forward | Midfielder) {
                    0.15
                } else {
                    0.0
                }
            }
            TacticalNeed::LosingMidfield | TacticalNeed::BeingPressed => {
                if sub_group == Midfielder && matches!(out_group, Forward | Midfielder) {
                    0.12
                } else {
                    0.0
                }
            }
            TacticalNeed::NeedingCrosses => {
                if sub_group == Midfielder
                    || matches!(
                        sub.tactical_position.current_position,
                        PlayerPositionType::WingbackLeft
                            | PlayerPositionType::WingbackRight
                            | PlayerPositionType::ForwardLeft
                            | PlayerPositionType::ForwardRight
                    )
                {
                    0.10
                } else {
                    0.0
                }
            }
            TacticalNeed::Fatigue => 0.0,
        }
    }

    /// Score every legal `(out, in)` pair and return the highest-
    /// scoring one that clears the threshold. Goalkeepers, force-
    /// selected starters, sent-off players, and substitutes that
    /// would leave the side without a position group are all filtered
    /// out. The score is:
    ///
    /// ```text
    /// pair_score = sub_off_score_protected(out, need, dampening)
    ///            + position_fit(out, sub) * sub_in_score(sub, need, fit, dev)
    ///            + tactical_fit_bonus(out, sub, need)
    ///            - disruption_penalty(out, sub)
    /// ```
    ///
    /// We multiply the in-score by position fit so a great fresh
    /// forward doesn't get crowned as the "best replacement" for an
    /// exhausted centre-back. Star protection comes from
    /// `sub_off_score_protected` and is dampened by `protection_dampening`
    /// (1.0 = full protection in ordinary states, 0.5 in a late
    /// comfortable lead).
    /// Back-compat thin wrapper without coach handles. Used by the
    /// substitution layer's existing tests and any caller that
    /// doesn't yet hold a coach engine — passes `None` straight
    /// through. Marked `allow(dead_code)` because the public match
    /// loop calls the `_with_coach` variant directly; this wrapper
    /// exists for the existing test surface only.
    #[allow(dead_code)]
    fn best_discretionary_pair(
        field: &MatchField,
        team_id: u32,
        need: TacticalNeed,
        own_goals: u8,
        opp_goals: u8,
        total_match_time_ms: u64,
        today: NaiveDate,
        protection_dampening: f32,
        min_threshold: f32,
    ) -> Option<(u32, u32)> {
        Self::best_discretionary_pair_with_coach(
            field,
            team_id,
            need,
            own_goals,
            opp_goals,
            total_match_time_ms,
            today,
            protection_dampening,
            min_threshold,
            None,
        )
    }

    /// Coach-aware variant of [`best_discretionary_pair`]. When
    /// `coach` is `Some`, the pair score folds in two small memory-
    /// driven nudges: a sub-off urgency nudge derived from the
    /// coach's read of the on-field player, and a sub-in preference
    /// nudge derived from the coach's read of the candidate
    /// substitute. The nudges are bounded by
    /// [`AssessmentMath::LIVE_SCALE`] inside the coach module, so a
    /// fresh coach with no memory cannot move the pair score by more
    /// than a fraction. When `coach` is `None`, the behaviour matches
    /// the legacy `best_discretionary_pair` exactly.
    fn best_discretionary_pair_with_coach(
        field: &MatchField,
        team_id: u32,
        need: TacticalNeed,
        own_goals: u8,
        opp_goals: u8,
        total_match_time_ms: u64,
        today: NaiveDate,
        protection_dampening: f32,
        min_threshold: f32,
        coach: Option<&CoachDecisionEngine<'_>>,
    ) -> Option<(u32, u32)> {
        let outfield_starters: Vec<&MatchPlayer> = field
            .players
            .iter()
            .filter(|p| p.team_id == team_id)
            .filter(|p| !p.is_sent_off)
            .filter(|p| !p.is_force_match_selection)
            .filter(|p| {
                p.tactical_position.current_position.position_group()
                    != PlayerFieldPositionGroup::Goalkeeper
            })
            .collect();

        if outfield_starters.is_empty() {
            return None;
        }

        let bench: Vec<&MatchPlayer> = field
            .substitutes
            .iter()
            .filter(|p| p.team_id == team_id)
            .filter(|p| {
                p.tactical_position.current_position.position_group()
                    != PlayerFieldPositionGroup::Goalkeeper
            })
            .collect();

        if bench.is_empty() {
            return None;
        }

        let live_snapshots: Vec<LiveSubstitutionStats> = outfield_starters
            .iter()
            .map(|p| {
                LiveSubstitutionStats::from_player(p, total_match_time_ms, own_goals, opp_goals)
            })
            .collect();

        let mut best: Option<(u32, u32, f32)> = None;

        for (out_idx, out) in outfield_starters.iter().enumerate() {
            let live = &live_snapshots[out_idx];

            let local_dampening = if live.errors_leading_to_goal >= 1 || live.red_cards >= 1 {
                0.0
            } else {
                protection_dampening
            };
            let out_score = SubScoring::sub_off_score_protected(out, live, need, local_dampening);
            let coach_off_nudge = coach
                .map(|c| c.sub_off_adjustment(out.id, &CoachLiveAdapter::live_ctx(out, live, true)))
                .unwrap_or(0.0);

            for sub in &bench {
                let fit = Self::position_fit(out, sub);
                if fit <= 0.0 {
                    continue;
                }

                let disruption = Self::disruption_penalty(field, team_id, out, sub);
                if !disruption.is_finite() {
                    continue;
                }

                let dev = Self::development_priority(sub, today);
                let in_score = SubScoring::sub_in_score(sub, need, fit, dev);
                let tactical_bonus = Self::tactical_fit_bonus(out, sub, need);

                // Live coach memory adapter — read tactical_trust / big-
                // match flags for both the on-field and bench players.
                let coach_in_nudge = coach
                    .map(|c| {
                        c.sub_in_adjustment(
                            sub.id,
                            &CoachLiveAdapter::live_ctx_sub(sub, total_match_time_ms),
                        )
                    })
                    .unwrap_or(0.0);

                let pair_score = out_score
                    + in_score
                    + tactical_bonus
                    + coach_off_nudge
                    + coach_in_nudge
                    - disruption;
                if pair_score < min_threshold {
                    continue;
                }

                match best {
                    Some((_, _, current)) if current >= pair_score => {}
                    _ => best = Some((out.id, sub.id, pair_score)),
                }
            }
        }

        best.map(|(out, in_id, _)| (out, in_id))
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

/// Tiny adapter mapping the substitution scorer's per-player snapshot
/// onto the [`CoachLiveMatchContext`] the coach engine consumes.
/// Bundled in its own struct so the conversion is in one place and the
/// pair scorer reads as orchestration.
struct CoachLiveAdapter;

impl CoachLiveAdapter {
    fn live_ctx(
        player: &MatchPlayer,
        live: &LiveSubstitutionStats,
        is_starter: bool,
    ) -> CoachLiveMatchContext {
        CoachLiveMatchContext {
            date: chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
            match_minute: live.match_minute,
            goal_diff: live.goal_diff,
            live_rating: live.live_rating,
            goals: live.goals,
            assists: live.assists,
            errors_leading_to_goal: live.errors_leading_to_goal,
            yellow_cards: live.yellow_cards,
            red_cards: live.red_cards,
            condition_pct: (player.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0),
            is_starter,
        }
    }

    /// Build a neutral live context for a bench player — the sub-in
    /// scorer doesn't have a live rating for him; the coach engine
    /// reads memory only.
    fn live_ctx_sub(sub: &MatchPlayer, total_match_time_ms: u64) -> CoachLiveMatchContext {
        CoachLiveMatchContext {
            date: chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
            match_minute: (total_match_time_ms / 60_000) as u32,
            goal_diff: 0,
            live_rating: 6.7,
            goals: 0,
            assists: 0,
            errors_leading_to_goal: 0,
            yellow_cards: 0,
            red_cards: 0,
            condition_pct: (sub.player_attributes.condition as f32 / 10_000.0).clamp(0.0, 1.0),
            is_starter: false,
        }
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

        let mut fresh = build_match_player(d(1998, 1, 1), PlayerPositionType::MidfielderCenter);
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
        // total. 400 → 2500 ticks after the 2026-06-11 fatigue
        // normalization (FATIGUE_RATE_MULTIPLIER 0.024 → 0.0035)
        // rescaled per-tick drain ~7× down; the debt mechanism is
        // unchanged, so the same visible gap just needs a
        // proportionally longer running stretch (~25 sim-seconds).
        for tick in 0..2500 {
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

    // ─────────────────────────────────────────────────────────────────
    // Scored-pair substitution behaviour — covers Section "Improve
    // fatigue / tactical / development logic" from the rework brief.
    // The scenarios are taken from the acceptance criteria: a goal
    // scorer is protected from routine removal; critical condition
    // still wins; late comfortable leads can rest stars; tactical
    // need (chasing) lifts the right replacement type; thin groups
    // are not hollowed out.
    // ─────────────────────────────────────────────────────────────────

    use crate::Tactics;
    use crate::club::team::tactics::MatchTacticType;
    use crate::r#match::ball::Ball;
    use crate::r#match::engine::result::{Score, TeamScore};
    use crate::r#match::engine::sub_scoring::SubScoring;
    use crate::r#match::squad::squad::MatchSquad;
    use crate::r#match::{MatchContext, MatchField, MatchFieldSize, MatchPlayerCollection};

    /// Build an outfield `MatchPlayer` with the given id, team, age,
    /// position, and condition. Sensible defaults for everything else
    /// — fresh stamina, 14.0 across all skills, no traits, age 24 by
    /// default (so the development-priority bonus is small).
    fn build_player(
        id: u32,
        team_id: u32,
        birth: NaiveDate,
        pos: PlayerPositionType,
        condition: i16,
    ) -> MatchPlayer {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = condition;
        attrs.jadedness = 1000;
        attrs.current_ability = 150;

        // Mid-tier skills across the board so trait_fit / need_fit
        // scores are stable, not zeroed out by missing skill data.
        let mut skills = PlayerSkills::default();
        skills.technical.finishing = 14.0;
        skills.technical.passing = 14.0;
        skills.technical.first_touch = 14.0;
        skills.technical.technique = 14.0;
        skills.technical.dribbling = 14.0;
        skills.technical.tackling = 14.0;
        skills.technical.marking = 14.0;
        skills.technical.crossing = 14.0;
        skills.mental.composure = 14.0;
        skills.mental.decisions = 14.0;
        skills.mental.vision = 14.0;
        skills.mental.anticipation = 14.0;
        skills.mental.concentration = 14.0;
        skills.mental.positioning = 14.0;
        skills.mental.off_the_ball = 14.0;
        skills.mental.work_rate = 14.0;
        skills.mental.determination = 14.0;
        skills.mental.aggression = 10.0;
        skills.physical.pace = 14.0;
        skills.physical.acceleration = 14.0;
        skills.physical.agility = 14.0;
        skills.physical.balance = 14.0;
        skills.physical.strength = 14.0;
        skills.physical.stamina = 14.0;
        skills.physical.natural_fitness = 14.0;

        let player = PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("T".to_string(), format!("P{}", id)))
            .birth_date(birth)
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: pos,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        MatchPlayer::from_player(team_id, &player, pos, false)
    }

    /// Stamp the player with `goals` goals and `assists` assists in
    /// the statistics log so `to_match_end_stats` reports the values
    /// and `LiveSubstitutionStats::from_player` picks them up.
    fn record_goals_and_assists(player: &mut MatchPlayer, goals: u16, assists: u16) {
        for _ in 0..goals {
            player.statistics.add_goal(60, false);
        }
        for _ in 0..assists {
            player.statistics.add_assist(60);
        }
    }

    /// Roster of 11 outfield positions matching `MatchTacticType::T442`
    /// (minus the goalkeeper, which the test inserts separately).
    const T442_OUTFIELD: [PlayerPositionType; 10] = [
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderCenterLeft,
        PlayerPositionType::DefenderCenterRight,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenterLeft,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardRight,
    ];

    /// Build a fresh 11-player roster for `team_id`. Players are ids
    /// `base_id..base_id + 11`. Tests mutate specific slots after the
    /// fact to set up the scenario they care about.
    fn build_roster(team_id: u32, base_id: u32, birth: NaiveDate) -> Vec<MatchPlayer> {
        let mut roster = Vec::with_capacity(11);
        roster.push(build_player(
            base_id,
            team_id,
            birth,
            PlayerPositionType::Goalkeeper,
            9000,
        ));
        for (i, pos) in T442_OUTFIELD.iter().enumerate() {
            roster.push(build_player(
                base_id + 1 + i as u32,
                team_id,
                birth,
                *pos,
                9000,
            ));
        }
        roster
    }

    fn build_bench(team_id: u32, base_id: u32, birth: NaiveDate) -> Vec<MatchPlayer> {
        // One sub per outfield group, plus a backup GK. Conditions
        // start fresh — the "incoming player" picture.
        vec![
            build_player(
                base_id,
                team_id,
                birth,
                PlayerPositionType::Goalkeeper,
                9500,
            ),
            build_player(
                base_id + 1,
                team_id,
                birth,
                PlayerPositionType::DefenderCenter,
                9500,
            ),
            build_player(
                base_id + 2,
                team_id,
                birth,
                PlayerPositionType::MidfielderCenter,
                9500,
            ),
            build_player(
                base_id + 3,
                team_id,
                birth,
                PlayerPositionType::ForwardCenter,
                9500,
            ),
        ]
    }

    fn make_squad(team_id: u32, main: Vec<MatchPlayer>, subs: Vec<MatchPlayer>) -> MatchSquad {
        MatchSquad {
            team_id,
            team_name: format!("Team{}", team_id),
            tactics: Tactics::new(MatchTacticType::T442),
            main_squad: main,
            substitutes: subs,
            captain_id: None,
            vice_captain_id: None,
            penalty_taker_id: None,
            free_kick_taker_id: None,
            selection_omissions: vec![],
            coach_snapshot: None,
        }
    }

    /// Build a struct-literal `MatchField` straight from two rosters.
    /// The full `MatchField::new` path runs formation-position
    /// assignment which would drop our hand-built players if their
    /// `tactical_position` doesn't match the formation slots — these
    /// tests sidestep that by writing the struct directly, since
    /// `best_discretionary_pair` only reads `players` / `substitutes`.
    fn make_test_field(
        home_main: Vec<MatchPlayer>,
        home_subs: Vec<MatchPlayer>,
        away_main: Vec<MatchPlayer>,
        away_subs: Vec<MatchPlayer>,
    ) -> MatchField {
        let players: Vec<MatchPlayer> = home_main.into_iter().chain(away_main).collect();
        let substitutes: Vec<MatchPlayer> = home_subs.into_iter().chain(away_subs).collect();
        MatchField {
            size: MatchFieldSize::new(840, 545),
            ball: Ball::with_coord(840.0, 545.0),
            players,
            substitutes,
            home_team_id: 1,
            away_team_id: 2,
            left_side_players: None,
            left_team_tactics: Tactics::new(MatchTacticType::T442),
            right_side_players: None,
            right_team_tactics: Tactics::new(MatchTacticType::T442),
            home_coach_snapshot: None,
            away_coach_snapshot: None,
        }
    }

    fn adult_birth() -> NaiveDate {
        // 26-year-old — past the development-priority threshold so
        // the dev bonus on bench players is 0.0 by default.
        d(1999, 1, 1)
    }

    #[test]
    fn scoring_striker_at_60pct_protected_from_routine_fatigue_sub() {
        // ForwardLeft on the home team has scored, has a high live
        // rating, and 60% condition. The team also has a knackered
        // anonymous winger (MidfielderLeft) at 35% who is the real
        // candidate for hooking. The ordinary-threshold pair scorer
        // must NOT pick the scorer over the tired winger.
        let mut home = build_roster(1, 100, adult_birth());
        let bench = build_bench(1, 200, adult_birth());
        let away = build_roster(2, 300, adult_birth());

        // Anonymous tired winger.
        let winger_idx = home
            .iter()
            .position(|p| {
                p.tactical_position.current_position == PlayerPositionType::MidfielderLeft
            })
            .unwrap();
        home[winger_idx].player_attributes.condition = 3500;

        // Star scorer — 1 goal, high rating, 60% condition.
        let scorer_idx = home
            .iter()
            .position(|p| p.tactical_position.current_position == PlayerPositionType::ForwardLeft)
            .unwrap();
        record_goals_and_assists(&mut home[scorer_idx], 1, 1);
        let scorer_id = home[scorer_idx].id;
        home[scorer_idx].player_attributes.condition = 6000;
        // Pretend the scorer played 70 minutes.
        home[scorer_idx].entry_match_time_ms = 0;

        let field = make_test_field(home, bench, away, vec![]);

        // Match minute 75, scoreline 1-0 (winning by one — adds
        // decisive protection on top of the goal protection).
        let total_ms = 75 * 60_000;
        let pair = Substitutions::best_discretionary_pair(
            &field,
            1,
            TacticalNeed::Fatigue,
            1,
            0,
            total_ms,
            d(2025, 1, 1),
            1.0,  // full star protection (ordinary state)
            0.85, // ordinary discretionary threshold
        );

        if let Some((out_id, _)) = pair {
            assert_ne!(
                out_id, scorer_id,
                "scorer with 60% condition + 1G/1A + decisive lead should be protected; \
                 instead the engine picked them as the sub-off candidate"
            );
        }
        // The tired winger pair clearing the threshold is fine, but
        // the *scorer* must never be chosen here.
    }

    #[test]
    fn star_scorer_can_be_rested_in_late_comfortable_lead() {
        // Same scorer setup, but the team is 3-0 up at minute 78. The
        // production loop drops protection_dampening to 0.5 in that
        // state and uses a 0.60 threshold — resting the star is OK.
        let mut home = build_roster(1, 100, adult_birth());
        let bench = build_bench(1, 200, adult_birth());
        let away = build_roster(2, 300, adult_birth());

        let scorer_idx = home
            .iter()
            .position(|p| p.tactical_position.current_position == PlayerPositionType::ForwardLeft)
            .unwrap();
        record_goals_and_assists(&mut home[scorer_idx], 1, 0);
        home[scorer_idx].player_attributes.condition = 3500;
        home[scorer_idx].entry_match_time_ms = 0;

        let field = make_test_field(home, bench, away, vec![]);

        // Match minute 78, 3-0 up — late_comfort branch.
        let total_ms = 78 * 60_000;
        let pair = Substitutions::best_discretionary_pair(
            &field,
            1,
            TacticalNeed::Fatigue,
            3,
            0,
            total_ms,
            d(2025, 1, 1),
            0.5,  // late_comfort dampening from production
            0.60, // late_comfort threshold from production
        );

        // The scorer at 35% with halved protection should be eligible.
        assert!(
            pair.is_some(),
            "in a late comfortable lead the engine should be willing \
             to make at least one rest sub"
        );
    }

    #[test]
    fn chasing_a_goal_picks_attacker_pair_over_defender_pair() {
        // Team is chasing. Best pair should bring on a forward to
        // sacrifice a tired midfielder/defender — not the other way
        // around. We seed one tired defender and one tired forward
        // and confirm the forward bench player is selected.
        let mut home = build_roster(1, 100, adult_birth());
        let bench = build_bench(1, 200, adult_birth());
        let away = build_roster(2, 300, adult_birth());

        // Tired defender (DefenderLeft) and a tired midfielder.
        let def_idx = home
            .iter()
            .position(|p| p.tactical_position.current_position == PlayerPositionType::DefenderLeft)
            .unwrap();
        home[def_idx].player_attributes.condition = 3500;

        let mid_idx = home
            .iter()
            .position(|p| {
                p.tactical_position.current_position == PlayerPositionType::MidfielderCenterLeft
            })
            .unwrap();
        home[mid_idx].player_attributes.condition = 4000;

        let bench_fwd_id = bench
            .iter()
            .find(|p| p.tactical_position.current_position == PlayerPositionType::ForwardCenter)
            .unwrap()
            .id;

        let field = make_test_field(home, bench, away, vec![]);

        // Match minute 78, trailing 0-1.
        let total_ms = 78 * 60_000;
        let pair = Substitutions::best_discretionary_pair(
            &field,
            1,
            TacticalNeed::Chasing,
            0,
            1,
            total_ms,
            d(2025, 1, 1),
            1.0,
            0.85,
        );

        let (_, in_id) = pair.expect("a discretionary chase sub should fire");
        assert_eq!(
            in_id, bench_fwd_id,
            "chasing a goal should bring on the bench forward; got {}",
            in_id
        );
    }

    #[test]
    fn disruption_penalty_blocks_emptying_a_thin_group() {
        // The "thin group" guard only matters when the bench has no
        // like-for-like sub: pulling the last Forward to bring on a
        // Midfielder would empty the Forward group. We strip the
        // bench Forward to force that situation; the disruption
        // penalty of ∞ should keep the surviving forward on the
        // pitch even though their 2500 condition would otherwise
        // make them an obvious fatigue candidate.
        let mut home = build_roster(1, 100, adult_birth());
        let bench: Vec<MatchPlayer> = build_bench(1, 200, adult_birth())
            .into_iter()
            .filter(|p| {
                p.tactical_position.current_position.position_group()
                    != PlayerFieldPositionGroup::Forward
            })
            .collect();
        let away = build_roster(2, 300, adult_birth());

        // ForwardRight is sent off → only ForwardLeft remains in the
        // Forward group on the pitch.
        let fr_idx = home
            .iter()
            .position(|p| p.tactical_position.current_position == PlayerPositionType::ForwardRight)
            .unwrap();
        home[fr_idx].is_sent_off = true;

        let fl_idx = home
            .iter()
            .position(|p| p.tactical_position.current_position == PlayerPositionType::ForwardLeft)
            .unwrap();
        home[fl_idx].player_attributes.condition = 2500;
        let fl_id = home[fl_idx].id;

        let field = make_test_field(home, bench, away, vec![]);

        let total_ms = 75 * 60_000;
        let pair = Substitutions::best_discretionary_pair(
            &field,
            1,
            TacticalNeed::Fatigue,
            1,
            0,
            total_ms,
            d(2025, 1, 1),
            1.0,
            0.85,
        );

        if let Some((out_id, _)) = pair {
            assert_ne!(
                out_id, fl_id,
                "engine pulled the last forward off the pitch despite the \
                 disruption-penalty guard"
            );
        }
    }

    #[test]
    fn one_goal_lead_scorer_protected_against_routine_removal() {
        // Spec: "Protecting a one-goal lead does not remove the only
        // goal scorer unless condition/card risk is high." The decisive
        // bonus (+0.15 when leading by exactly one) keeps the scorer
        // off the candidate list at the ordinary threshold.
        let mut home = build_roster(1, 100, adult_birth());
        let bench = build_bench(1, 200, adult_birth());
        let away = build_roster(2, 300, adult_birth());

        let scorer_idx = home
            .iter()
            .position(|p| p.tactical_position.current_position == PlayerPositionType::ForwardLeft)
            .unwrap();
        record_goals_and_assists(&mut home[scorer_idx], 1, 0);
        let scorer_id = home[scorer_idx].id;
        // Decent condition — no fatigue case to override protection.
        home[scorer_idx].player_attributes.condition = 7000;

        let field = make_test_field(home, bench, away, vec![]);

        let total_ms = 70 * 60_000;
        let pair = Substitutions::best_discretionary_pair(
            &field,
            1,
            TacticalNeed::ProtectingLead,
            1,
            0,
            total_ms,
            d(2025, 1, 1),
            1.0,
            0.85,
        );

        if let Some((out_id, _)) = pair {
            assert_ne!(
                out_id, scorer_id,
                "engine pulled the 1-0 scorer despite decisive-lead protection"
            );
        }
    }

    #[test]
    fn allowed_in_window_gates_discretionary_subs() {
        // The discretionary loop calls `allowed_in_window` with
        // force_critical=false. The window function is what enforces
        // the 55+/65+/75+/85+ slot calendar.
        assert!(!SubScoring::allowed_in_window(0, 40, false));
        assert!(!SubScoring::allowed_in_window(0, 54, false));
        assert!(SubScoring::allowed_in_window(0, 55, false));
        assert!(SubScoring::allowed_in_window(0, 88, false));
        assert!(!SubScoring::allowed_in_window(0, 89, false));

        // force_critical bypasses the window (post-5'). Critical-injury
        // and youth-protection passes use this carve-out.
        assert!(SubScoring::allowed_in_window(0, 10, true));
        assert!(SubScoring::allowed_in_window(2, 6, true));
    }

    // ─────────────────────────────────────────────────────────────────
    // End-to-end: process_substitutions integrates the new pair
    // scorer with the existing critical / youth / window gating.
    // Build a real MatchField via MatchField::new and a real
    // MatchContext so the rule-set + sub bookkeeping all run.
    // ─────────────────────────────────────────────────────────────────

    fn make_match_context(
        score_home: u8,
        score_away: u8,
        total_match_time: u64,
    ) -> (MatchField, MatchContext) {
        let home = build_roster(1, 100, adult_birth());
        let home_subs = build_bench(1, 200, adult_birth());
        let away = build_roster(2, 300, adult_birth());
        let away_subs = build_bench(2, 400, adult_birth());

        let home_squad = make_squad(1, home, home_subs);
        let away_squad = make_squad(2, away, away_subs);
        let players = MatchPlayerCollection::from_squads(&home_squad, &away_squad);
        let field = MatchField::new(840, 545, home_squad, away_squad);
        let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
        context.score.home_team = TeamScore::new_with_score(1, score_home);
        context.score.away_team = TeamScore::new_with_score(2, score_away);
        context.total_match_time = total_match_time;
        (field, context)
    }

    #[test]
    fn process_substitutions_removes_tired_winger_over_high_rated_scorer() {
        // Integration: a non-scoring tired winger should be picked
        // for substitution before a high-rated scoring forward. The
        // pair scorer's star protection makes the scorer-pair score
        // far less than the winger-pair score, so the loop pulls the
        // winger off when it fires the first sub.
        let (mut field, mut context) = make_match_context(2, 1, 78 * 60_000);

        // Set up the home roster's interesting players.
        let scorer_id = field
            .players
            .iter()
            .find(|p| {
                p.team_id == 1
                    && p.tactical_position.current_position == PlayerPositionType::ForwardLeft
            })
            .map(|p| p.id)
            .expect("home forward exists");
        let winger_id = field
            .players
            .iter()
            .find(|p| {
                p.team_id == 1
                    && p.tactical_position.current_position == PlayerPositionType::MidfielderLeft
            })
            .map(|p| p.id)
            .expect("home left mid exists");

        for p in field.players.iter_mut() {
            if p.id == scorer_id {
                record_goals_and_assists(p, 1, 0);
                p.player_attributes.condition = 7000;
            } else if p.id == winger_id {
                p.player_attributes.condition = 3500;
            }
        }

        process_substitutions(&mut field, &mut context, 5, d(2025, 1, 1));

        // The scorer must not be on the substituted_out list. The
        // winger should be (the only discretionary sub-eligible
        // outfielder with deeply tired condition + no protection).
        let subbed_out_ids: Vec<u32> = context
            .substituted_out_stats
            .iter()
            .map(|(id, _)| *id)
            .collect();
        assert!(
            !subbed_out_ids.contains(&scorer_id),
            "process_substitutions removed the high-rated scorer ({}); subbed out = {:?}",
            scorer_id,
            subbed_out_ids
        );
        assert!(
            subbed_out_ids.contains(&winger_id),
            "process_substitutions should have removed the tired non-scoring winger \
             ({}); subbed out = {:?}",
            winger_id,
            subbed_out_ids
        );
    }

    #[test]
    fn process_substitutions_forces_off_critical_condition_scorer() {
        // Sanity check the critical-injury override: even a goal
        // scorer with high rating is pulled off when condition drops
        // below the CRITICAL_CONDITION threshold of 2000. The pair
        // scorer's star protection does NOT apply to this branch.
        let (mut field, mut context) = make_match_context(1, 0, 78 * 60_000);

        let scorer_id = field
            .players
            .iter()
            .find(|p| {
                p.team_id == 1
                    && p.tactical_position.current_position == PlayerPositionType::ForwardLeft
            })
            .map(|p| p.id)
            .expect("home forward exists");

        for p in field.players.iter_mut() {
            if p.id == scorer_id {
                record_goals_and_assists(p, 1, 0);
                // Simulates a mid-match injury — under the
                // CRITICAL_CONDITION = 2000 threshold.
                p.player_attributes.condition = 1500;
            }
        }

        process_substitutions(&mut field, &mut context, 5, d(2025, 1, 1));

        let subbed_out_ids: Vec<u32> = context
            .substituted_out_stats
            .iter()
            .map(|(id, _)| *id)
            .collect();
        assert!(
            subbed_out_ids.contains(&scorer_id),
            "critical-condition scorer ({}) must be force-subbed; subbed out = {:?}",
            scorer_id,
            subbed_out_ids
        );
    }
}

//! Post-match player effects: stats bookkeeping, morale events,
//! reputation update.
//!
//! All cross-cutting effects of "a match happened" live here instead
//! of leaking into the league-result pipeline. Role-transition tracking
//! (the `WonStartingPlace` / `LostStartingPlace` one-shots) is dispatched
//! to [`super::role`]; physical exertion / injury rolls live in
//! [`super::match_exertion`].

use super::scaling;
use super::types::{MatchOutcome, MatchParticipation};
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::player::Player;
use crate::{
    HappinessEventCause, HappinessEventContext, HappinessEventEvidence, HappinessEventFollowUp,
    HappinessEventScope, HappinessEventSeverity, HappinessEventType, ManagerCriticismReason,
    ManagerInteractionEventContext, ManagerInteractionTone, ManagerInteractionTopic,
    MatchPerformanceEventContext, MatchPerformanceEvidence, MatchPerformanceKind,
    MatchSelectionContext, MediaFanEventContext, MediaFanEventKind, MediaFanSource,
    PlayerAcceptance, PlayerSquadStatus, PlayerStatistics, SelectionDecisionScope,
    SelectionOmissionReason, SelectionRole, SupportEventContext, SupportSetting, SupportSource,
    SupportTrigger,
};

impl Player {
    /// React to finishing a match: stats bookkeeping, morale events,
    /// reputation update. All cross-cutting effects of "a match happened"
    /// live here instead of leaking into the league-result pipeline.
    pub fn on_match_played(&mut self, o: &MatchOutcome<'_>) {
        // The team the player physically turns out for in his career
        // history: his active (non-departed) current-season spell. A
        // league appearance for any OTHER team of the same club — a
        // reserve pulled up to the main XI, or a senior fielded for the
        // "2" side — is a borrowed appearance and books into a per-team
        // secondary bucket so it shows as its own history row instead of
        // folding under this spell. Empty when there is no active spell
        // (freshly built player) — the appearance then books to the home
        // bucket as before.
        let home_slug = self
            .statistics_history
            .current
            .iter()
            .find(|e| e.departed_date.is_none())
            .map(|e| e.team_slug.clone())
            .unwrap_or_default();
        self.record_match_appearance(o, &home_slug);
        self.record_match_stats(o, &home_slug);
        // Cup appearances land in the per-competition buckets; rebuild the
        // rolled-up aggregate before the event pass reads it for
        // milestones (first-club goal, appearance / goal milestones, …).
        if o.is_cup {
            self.recompute_cup_statistics();
        }
        self.record_match_events(o);
        self.record_match_reputation(o);
        // After the routine post-match bookkeeping, see if this fixture
        // also clears the dedicated "big match" trust bar — derby / cup
        // final / continental knockout starters get the dedicated
        // `TrustedInBigMatch` row on top of the regular debrief.
        self.maybe_emit_big_match_trust(o);
    }

    /// Named to a squad but never got off the bench. Synthesises a
    /// minimal `MatchSelectionContext` (UnusedSubstitute / BenchBalance)
    /// when the squad selector didn't record a specific omission reason
    /// — the renderer still has scope to say "named to the bench but
    /// never came on" instead of the bare "Dropped from match squad".
    ///
    /// Routes through [`Self::on_match_dropped_with_context`] so every
    /// `MatchDropped` event carries structured selection metadata; the
    /// payload-invariant guard in `add_event_full` keeps that contract
    /// honest under tests.
    pub fn on_match_dropped(&mut self) {
        let ctx = MatchSelectionContext {
            scope: SelectionDecisionScope::UnusedSubstitute,
            reason: SelectionOmissionReason::BenchBalance,
            comparison: None,
            role: SelectionRole::Other,
            match_importance: 0.5,
            repeated: false,
            is_friendly: false,
        };
        self.on_match_dropped_with_context(ctx);
    }

    /// Same as [`Self::on_match_dropped`] but carries the structured
    /// selection-explanation payload built by the squad selector. The
    /// stored event therefore knows the scope (left out / dropped to
    /// bench / unused sub), the dominant football reason, and the
    /// chosen replacement — the player-events renderer turns that into
    /// the "Lost out to Marco Silva because he was sharper" line.
    ///
    /// The starter-ratio bookkeeping is identical to the legacy method
    /// for `UnusedSubstitute` and `DroppedToBench` scopes — both still
    /// represent a missed chance to start. `LeftOutOfMatchdaySquad`
    /// also feeds the same EMA: the player wasn't in the matchday squad
    /// at all, so it's a 0-share appearance just like an unused sub.
    pub fn on_match_dropped_with_context(&mut self, ctx: MatchSelectionContext) {
        // Post-transfer match-opportunity tracking — a match the club
        // actually played that this player was available for but didn't
        // feature in. Friendlies never count. Neither do matches the
        // player missed through injury / suspension / not-yet-eligible
        // status: those aren't a manager snub and the spec's zero-match
        // invariant must not be tripped by them. Captured before `ctx`
        // is moved into the event payload below.
        let counts_as_opportunity = !ctx.is_friendly
            && !self.player_attributes.is_injured
            && !matches!(ctx.scope, SelectionDecisionScope::UnavailableButNotInjured)
            && !matches!(
                ctx.reason,
                SelectionOmissionReason::ReturningFromInjury
                    | SelectionOmissionReason::FitnessProtection
            );
        let left_out = matches!(ctx.scope, SelectionDecisionScope::LeftOutOfMatchdaySquad);

        let magnitude = compute_drop_magnitude(self, &ctx);
        let severity = HappinessEventSeverity::from_magnitude(magnitude);
        let cause = drop_cause(&ctx);
        let follow_up = drop_follow_up(self, &ctx);
        let mut event_ctx =
            HappinessEventContext::new(cause, severity, HappinessEventScope::MatchDay);
        if let Some(fu) = follow_up {
            event_ctx = event_ctx.with_follow_up(fu);
        }
        // Clone before move: the tactical-role / big-match detectors
        // below need the same metadata the renderer just took ownership
        // of, and the borrow checker doesn't let us read after move.
        let snapshot = ctx.clone();
        event_ctx = event_ctx.with_selection_context(ctx);

        self.happiness.add_event_with_context(
            HappinessEventType::MatchDropped,
            magnitude,
            None,
            event_ctx,
        );

        const ALPHA: f32 = 0.25;
        self.happiness.starter_ratio = self.happiness.starter_ratio * (1.0 - ALPHA);
        self.happiness.appearances_tracked = self.happiness.appearances_tracked.saturating_add(1);

        if counts_as_opportunity {
            self.happiness.note_official_non_appearance(left_out);
        }

        self.evaluate_role_transition();

        // Two follow-on emits keyed off the same selection snapshot:
        //
        // * Tactical-role frustration aggregator — a player whose recent
        //   drop history is loaded with "no natural role" / "tactical
        //   mismatch" omissions hears it as a system problem, not a one-
        //   off snub. Gated to `>= 3` recent same-flavour drops by the
        //   helper so casual rotation never trips it.
        // * Big-match bench — derby / cup final / continental knockout
        //   omission of an expected starter. Gated by the helper on
        //   match importance, scope, and protective reasons (rest /
        //   returning from injury are not "benched", they're protected).
        self.maybe_emit_tactical_role_mismatch(&snapshot);
        self.maybe_emit_big_match_bench(&snapshot);
    }

    fn record_match_appearance(&mut self, o: &MatchOutcome<'_>, home_slug: &str) {
        // Tag the spell with the league_slug of the friendly we just
        // played, so a later `drain_match_stats` can stamp the canonical
        // Friendly ledger entry with the real source — youth-league
        // loanees keep their "U19 League" breakdown label after the
        // loan ends. Empty slugs (senior pre-season friendlies) are
        // ignored so the drain falls back to the team's league_slug
        // and the row renders as the generic "Friendly".
        if o.is_friendly && !o.competition_slug.is_empty() {
            self.friendly_source_slug = Some(o.competition_slug.to_string());
        }
        let s = stats_bucket_mut(self, o, home_slug);
        match o.participation {
            MatchParticipation::Starter => s.played += 1,
            MatchParticipation::Substitute => s.played_subs += 1,
        }

        // Post-transfer match-opportunity tracking. Only official
        // (competitive) matches count toward the eligible-match
        // denominator behind the playing-time frustration gate — a
        // pre-season friendly run-out tells us nothing about the
        // manager's competitive trust.
        if !o.is_friendly {
            self.happiness
                .note_official_appearance(matches!(o.participation, MatchParticipation::Starter));
        }
    }

    fn record_match_stats(&mut self, o: &MatchOutcome<'_>, home_slug: &str) {
        // Feed the per-player form EMA before we mutate any stat bucket —
        // `effective_rating` is the post-settlement rating already used for
        // season averages and POM selection, so form stays consistent.
        if !o.is_friendly {
            self.load.update_form(o.effective_rating);
        }

        let s = stats_bucket_mut(self, o, home_slug);
        s.goals += o.stats.goals;
        s.assists += o.stats.assists;
        s.shots_on_target += o.stats.shots_on_target as f32;
        s.tackling += o.stats.tackles as f32;
        s.yellow_cards = s.yellow_cards.saturating_add(o.stats.yellow_cards as u8);
        s.red_cards = s.red_cards.saturating_add(o.stats.red_cards as u8);

        if o.stats.passes_attempted > 0 {
            let match_pct =
                (o.stats.passes_completed as f32 / o.stats.passes_attempted as f32 * 100.0) as u8;
            let games = s.played + s.played_subs;
            s.passes = if games <= 1 {
                match_pct
            } else {
                let prev = s.passes as f32;
                ((prev * (games - 1) as f32 + match_pct as f32) / games as f32) as u8
            };
        }

        // Minutes-weighted rolling average — a 10-minute cameo no
        // longer counts the same as a 90-minute start. We feed the
        // ledger the *effective* (post-settlement, post-personality)
        // rating so the season average, awards, POTM, scouting
        // observations, form EMA, and reputation deltas all read the
        // same number. The raw engine rating stays on `stats` for
        // diagnostics / calibration but is no longer the public face
        // of "how the player did" — otherwise a fresh signing could
        // farm a high season average from raw 8s while every
        // downstream consumer of `effective_rating` saw the dampened
        // value, leaving the user staring at two different numbers
        // for the same match.
        let is_starter = matches!(o.participation, MatchParticipation::Starter);
        s.record_match_rating(
            o.effective_rating,
            o.stats.minutes_played as u16,
            is_starter,
        );

        if o.is_motm {
            s.player_of_the_match = s.player_of_the_match.saturating_add(1);
        }

        // GK conceded / clean-sheet bookkeeping — only for starting GKs.
        // Subs who came on briefly don't get attributed the full team conceded.
        if self.position().is_goalkeeper() && matches!(o.participation, MatchParticipation::Starter)
        {
            let s = stats_bucket_mut(self, o, home_slug);
            s.conceded += o.team_goals_against as u16;
            if o.team_goals_against == 0 {
                s.clean_sheets += 1;
            }
        }
    }

    fn record_match_events(&mut self, o: &MatchOutcome<'_>) {
        if !o.is_friendly {
            // Rolling starter-share tracking — drives the WonStartingPlace /
            // LostStartingPlace one-shot transitions. Only competitive
            // matches count: pre-season minutes don't tell us anything
            // about the manager's matchday trust.
            self.update_role_state(o);
        }

        if o.is_motm {
            self.happiness
                .add_event_default(HappinessEventType::PlayerOfTheMatch);
        }

        // Friendlies don't generate the rest of the football-life events —
        // pre-season form, suspensions, derby narratives don't apply.
        if o.is_friendly {
            return;
        }

        // Senior debut, drought tracking, milestones — derived purely
        // from the post-match competitive totals so we don't add a
        // per-match history Vec. Cooldowns gate against duplicate fires
        // when the simulator re-enters this path on the same day.
        self.record_senior_debut(o);
        self.record_drought_signals(o);
        self.record_milestones(o);
        self.record_hat_tricks(o);
        self.record_fans_chant_and_media_pressure(o);
        self.record_leadership_emergence(o);

        // Sent off — embarrassing, plus the suspension fallout. Flat hit.
        if o.stats.red_cards > 0 {
            let mp = MatchPerfContextBuilder::costly_error(self, o);
            let happiness_ctx = HappinessEventContext::new(
                HappinessEventCause::PoorFormPressure,
                HappinessEventSeverity::Serious,
                HappinessEventScope::MatchDay,
            )
            .with_match_performance_context(mp);
            let mag = HappinessConfig::default()
                .catalog
                .magnitude(HappinessEventType::RedCardFallout);
            self.happiness.add_event_with_context(
                HappinessEventType::RedCardFallout,
                mag,
                None,
                happiness_ctx,
            );
        }

        // First competitive goal at this club. Stats are reset on club
        // change (see `on_transfer` / `on_loan`), so the only way the
        // running competitive total equals this match's goals is when
        // this is the first scoring match of the tenure. Long cooldown
        // prevents the milestone from firing again later in the spell.
        if o.stats.goals > 0 {
            let total_competitive = self.statistics.goals + self.cup_statistics.goals;
            if total_competitive == o.stats.goals
                && !self
                    .happiness
                    .has_recent_event(&HappinessEventType::FirstClubGoal, 300)
            {
                let mp = MatchPerfContextBuilder::standout(
                    self,
                    o,
                    MatchPerformanceKind::FirstClubGoalMoment,
                );
                let happiness_ctx = HappinessEventContext::new(
                    HappinessEventCause::Other,
                    HappinessEventSeverity::Major,
                    HappinessEventScope::MatchDay,
                )
                .with_match_performance_context(mp);
                let mag = HappinessConfig::default()
                    .catalog
                    .magnitude(HappinessEventType::FirstClubGoal);
                self.happiness.add_event_with_context(
                    HappinessEventType::FirstClubGoal,
                    mag,
                    None,
                    happiness_ctx,
                );
            }
        }

        // Substitute impact: came on and made it count. Skip if already
        // tagged POM — no point double-firing for the same standout shift.
        if !o.is_motm
            && o.participation == MatchParticipation::Substitute
            && (o.stats.goals > 0 || o.stats.assists > 0 || o.effective_rating >= 7.3)
        {
            let mp = MatchPerfContextBuilder::standout(
                self,
                o,
                MatchPerformanceKind::ChangedGameFromBench,
            );
            let happiness_ctx = HappinessEventContext::new(
                HappinessEventCause::Other,
                HappinessEventSeverity::Moderate,
                HappinessEventScope::MatchDay,
            )
            .with_match_performance_context(mp);
            let mag = HappinessConfig::default()
                .catalog
                .magnitude(HappinessEventType::SubstituteImpact);
            self.happiness.add_event_with_context(
                HappinessEventType::SubstituteImpact,
                mag,
                None,
                happiness_ctx,
            );
        }

        // Clean sheet pride for goalkeepers and defenders — both roles
        // genuinely care about a shutout. Starters get the full event;
        // unused subs aren't on the field but still share the team result
        // (skipped here — they don't even hit `record_match_events`).
        if o.team_goals_against == 0
            && (self.position().is_goalkeeper() || self.position().is_defender())
        {
            let mp = MatchPerfContextBuilder::standout(
                self,
                o,
                MatchPerformanceKind::DefensiveLeaderPerformance,
            );
            let happiness_ctx = HappinessEventContext::new(
                HappinessEventCause::Other,
                HappinessEventSeverity::Minor,
                HappinessEventScope::MatchDay,
            )
            .with_match_performance_context(mp);
            let mag = HappinessConfig::default()
                .catalog
                .magnitude(HappinessEventType::CleanSheetPride);
            self.happiness.add_event_with_context(
                HappinessEventType::CleanSheetPride,
                mag,
                None,
                happiness_ctx,
            );
        }

        // Match-rating debrief. The catastrophic floor is now its own event
        // (`CostlyMistake`) instead of overloaded ManagerCriticism, mid-low
        // ratings still fire the manager event for a routine talking-to,
        // and the derby effect is moved out to `DerbyHero` / `DerbyDefeat`
        // so we don't double-count derby weight on top of personal form.
        if o.stats.match_rating >= 1.0 {
            if o.effective_rating < 5.5 {
                let extra = (5.5 - o.effective_rating).clamp(0.0, 2.0);
                let mut mp = MatchPerformanceEventContext::new(
                    MatchPerformanceKind::CostlyErrorUnderPressure,
                )
                .with_rating(o.effective_rating)
                .with_minutes(o.stats.minutes_played as u16)
                .with_team_won(o.team_won)
                .with_goal_margin(o.goal_margin() as i8)
                .with_derby(o.is_derby)
                .with_cup(o.is_cup)
                .with_evidence(MatchPerformanceEvidence::LowRating);
                if o.is_derby {
                    mp = mp.with_evidence(MatchPerformanceEvidence::DerbyFixture);
                }
                if o.is_cup {
                    mp = mp.with_evidence(MatchPerformanceEvidence::CupTie);
                }
                if self.attributes.pressure <= 7.0 {
                    mp = mp.with_evidence(MatchPerformanceEvidence::LowPressurePersonality);
                }
                let happiness_ctx = HappinessEventContext::new(
                    HappinessEventCause::PoorFormPressure,
                    HappinessEventSeverity::from_magnitude(2.0 + extra),
                    HappinessEventScope::MatchDay,
                )
                .with_match_performance_context(mp);
                self.happiness.add_event_with_context(
                    HappinessEventType::CostlyMistake,
                    -(2.0 + extra),
                    None,
                    happiness_ctx,
                );
            } else if o.effective_rating < 6.3 {
                let mag = -(2.0 + (6.3 - o.effective_rating).clamp(0.0, 0.8));
                let recent_mgr_criticism = self.happiness.recent_events.iter().any(|e| {
                    e.event_type == HappinessEventType::ManagerCriticism && e.days_ago <= 30
                });
                // Concrete football reason picked from the post-match
                // outcome. Computed independently of whether earlier
                // criticism is on file — the reason is the stable
                // identity the cooldown gate keys on, while
                // `recent_mgr_criticism` only escalates tone /
                // follow-up below. Conflating "repeated incident" into
                // the reason itself would silently bypass the
                // reason-aware cooldown the next match (a new reason
                // would appear to fire from the gate's perspective).
                let reason = if o.stats.red_cards > 0 {
                    ManagerCriticismReason::PublicComplaint
                } else if self.skills.mental.work_rate <= 8.0 {
                    ManagerCriticismReason::PoorPressing
                } else if self.skills.mental.teamwork <= 8.0 {
                    ManagerCriticismReason::MissedAssignment
                } else if self.attributes.professionalism <= 8.0 {
                    ManagerCriticismReason::PoorBodyLanguage
                } else {
                    ManagerCriticismReason::IgnoredTacticalInstruction
                };
                let topic = match reason {
                    ManagerCriticismReason::PoorBodyLanguage => ManagerInteractionTopic::Attitude,
                    ManagerCriticismReason::IgnoredTacticalInstruction
                    | ManagerCriticismReason::MissedAssignment
                    | ManagerCriticismReason::PoorPressing => ManagerInteractionTopic::Tactical,
                    ManagerCriticismReason::PublicComplaint => ManagerInteractionTopic::Discipline,
                    _ => ManagerInteractionTopic::Performance,
                };
                let tone = if recent_mgr_criticism {
                    ManagerInteractionTone::Stern
                } else {
                    ManagerInteractionTone::Honest
                };
                let acceptance = if self.skills.mental.determination >= 15.0 {
                    PlayerAcceptance::Motivated
                } else if self.attributes.professionalism <= 8.0 {
                    PlayerAcceptance::Resented
                } else {
                    PlayerAcceptance::Discouraged
                };
                let mctx = ManagerInteractionEventContext::new(topic, tone, acceptance)
                    .with_criticism_reason(reason)
                    .with_match_rating(o.effective_rating)
                    .with_repeated_recently(recent_mgr_criticism);
                let happiness_ctx = HappinessEventContext::new(
                    HappinessEventCause::PoorFormPressure,
                    HappinessEventSeverity::from_magnitude(mag),
                    HappinessEventScope::MatchDay,
                )
                .with_manager_interaction_context(mctx)
                .with_follow_up(if recent_mgr_criticism {
                    HappinessEventFollowUp::DressingRoomDamageRisk
                } else {
                    HappinessEventFollowUp::ManagerInterventionRisk
                });
                // Reason-aware 14-day cooldown — only the SAME criticism
                // reason inside the window is throttled. A poor-form
                // stretch that earned a "PoorPressing" row will let a
                // subsequent red-card "PublicComplaint" through, since
                // they're materially different manager talks. The
                // suppressed magnitude still wears the player down via
                // a durable hidden form-pressure accumulator (read in
                // `recalculate_morale`); we don't push to
                // `adjust_morale` directly because the next weekly
                // recalculation rebuilds morale from factors + events
                // and would silently drop a transient nudge.
                let blocked_by_reason = self
                    .happiness
                    .has_recent_manager_criticism_with_reason(reason, 14);
                let emitted = if blocked_by_reason {
                    false
                } else {
                    self.happiness.add_event_with_context(
                        HappinessEventType::ManagerCriticism,
                        mag,
                        None,
                        happiness_ctx,
                    );
                    true
                };
                if !emitted {
                    self.happiness.accumulate_hidden_form_pressure(mag);
                }
            } else if o.effective_rating >= 7.5 {
                let mag = 1.5 + (o.effective_rating - 7.5).clamp(0.0, 2.5);
                let event_ctx = MatchSupportContextBuilder::manager_encouragement(self, o, mag);
                self.happiness.add_event_with_context_and_cooldown(
                    HappinessEventType::ManagerEncouragement,
                    mag,
                    None,
                    event_ctx,
                    14,
                );
            }
        }

        // ── Decisive goal / fan / media reactions ───────────────────
        let cfg = HappinessConfig::default();
        let had_contribution = o.stats.goals > 0 || o.stats.assists > 0;

        // DecisiveGoal — scored or assisted in a single-goal team win.
        // Captures the late winner / only-goal-of-the-game moment without
        // needing minute-of-goal data. Cooldown 14d so a hot scoring run
        // still feels punctuated rather than fired every weekend.
        if had_contribution && o.team_won && o.goal_margin() == 1 {
            let pressure_mul = scaling::pressure_amplifier(
                self.attributes.important_matches,
                self.attributes.pressure,
            );
            let scene_mul = if o.is_cup || o.is_derby { 1.25 } else { 1.0 };
            let rep_mul = scaling::reputation_amplifier(self.player_attributes.current_reputation);
            let mag = cfg.catalog.decisive_goal * pressure_mul * scene_mul * rep_mul;
            self.happiness
                .add_event_with_cooldown(HappinessEventType::DecisiveGoal, mag, 14);
        }

        // FanPraise — supporters latch onto a stand-out display. Triggered
        // by POM, an excellent rating, or a goal/assist contribution in a
        // win. Reputation-amplified so high-profile players feel it more.
        let fan_praise_trigger =
            o.is_motm || o.effective_rating >= 8.0 || (o.team_won && had_contribution);
        if fan_praise_trigger {
            let rep_mul = scaling::reputation_amplifier(self.player_attributes.current_reputation);
            let scene_mul = if o.is_cup || o.is_derby { 1.2 } else { 1.0 };
            let mag = cfg.catalog.fan_praise * rep_mul * scene_mul;
            let event_ctx = MatchSupportContextBuilder::fan_praise(self, o, had_contribution, mag);
            self.happiness.add_event_with_context_and_cooldown(
                HappinessEventType::FanPraise,
                mag,
                None,
                event_ctx,
                21,
            );
        }

        // FanCriticism — fans turn on a poor display, especially in
        // defeat or after a red card. Amplified by controversy/low
        // temperament; dampened by professionalism (settles ego).
        let fan_criticism_trigger = o.stats.red_cards > 0
            || o.effective_rating < 5.7
            || (o.team_lost && o.effective_rating < 6.2);
        if fan_criticism_trigger {
            let rep_mul = scaling::reputation_amplifier(self.player_attributes.current_reputation);
            let provoke_mul = scaling::criticism_amplifier(
                self.attributes.controversy,
                self.attributes.temperament,
            );
            let prof_dampen = scaling::criticism_dampener(self.attributes.professionalism);
            let mag = cfg.catalog.fan_criticism * rep_mul * provoke_mul * prof_dampen;
            let mfctx = MediaFanEventContext::new(
                MediaFanEventKind::AwayFansHostile,
                MediaFanSource::HomeSupporters,
            )
            .with_form_trigger();
            let happiness_ctx = HappinessEventContext::new(
                HappinessEventCause::PoorFormPressure,
                HappinessEventSeverity::from_magnitude(mag),
                HappinessEventScope::Media,
            )
            .with_media_fan_context(mfctx);
            self.happiness.add_event_with_context_and_cooldown(
                HappinessEventType::FanCriticism,
                mag,
                None,
                happiness_ctx,
                21,
            );
        }

        // MediaPraise — strictly rarer than fan reaction; only fires for
        // a genuinely elite shift or a story-defining moment. Cooldown
        // 30d so press inches don't pile up week after week.
        let exceptional_gk_shutout = self.position().is_goalkeeper()
            && o.team_goals_against == 0
            && (o.is_cup || o.is_derby)
            && matches!(o.participation, MatchParticipation::Starter);
        let media_praise_trigger = o.effective_rating >= 8.3
            || (o.is_motm && (o.is_cup || o.is_derby))
            || exceptional_gk_shutout;
        if media_praise_trigger {
            let rep_mul = scaling::reputation_amplifier(self.player_attributes.current_reputation);
            let mag = cfg.catalog.media_praise * rep_mul;
            let mfctx = MediaFanEventContext::new(
                MediaFanEventKind::MediaNarrativeChanged,
                MediaFanSource::NationalPress,
            )
            .with_big_match_trigger();
            let happiness_ctx = HappinessEventContext::new(
                HappinessEventCause::Other,
                HappinessEventSeverity::from_magnitude(mag),
                HappinessEventScope::Media,
            )
            .with_media_fan_context(mfctx);
            self.happiness.add_event_with_context_and_cooldown(
                HappinessEventType::MediaPraise,
                mag,
                None,
                happiness_ctx,
                30,
            );
        }

        // Derby outcome — proper rivalry-day events instead of recycled
        // manager talks. DerbyHero is reserved for standout performers
        // (scored, assisted, POM, ≥7.5 rating, or GK/DEF clean sheet
        // ≥7.2). Ordinary squad members on the winning side get the
        // squad-wide DerbyWin instead, so the event log doesn't claim
        // every fullback was the hero of the match.
        if o.is_derby {
            if o.team_won {
                let is_back_line = self.position().is_goalkeeper() || self.position().is_defender();
                let standout = o.stats.goals > 0
                    || o.stats.assists > 0
                    || o.is_motm
                    || o.effective_rating >= 7.5
                    || (is_back_line && o.team_goals_against == 0 && o.effective_rating >= 7.2);
                if standout {
                    let bonus = if o.stats.goals > 0 || o.is_motm {
                        2.0
                    } else if o.effective_rating >= 7.5 {
                        1.0
                    } else {
                        0.0
                    };
                    self.happiness.add_event(
                        HappinessEventType::DerbyHero,
                        cfg.catalog.derby_hero + bonus,
                    );
                } else {
                    self.happiness
                        .add_event_default(HappinessEventType::DerbyWin);
                }
            } else if o.team_lost {
                // Squad-wide base hit, with extra for poor performers /
                // red cards. Base around -3 (catalog), extra up to -3.0
                // for a red-card collapse, capped to keep magnitudes sane.
                let mut extra = 0.0f32;
                if o.effective_rating < 6.0 {
                    extra += (6.0 - o.effective_rating).clamp(0.0, 1.0) * 1.5;
                }
                if o.stats.red_cards > 0 {
                    extra += 1.5;
                }
                let extra = extra.clamp(0.0, 3.0);
                self.happiness.add_event(
                    HappinessEventType::DerbyDefeat,
                    cfg.catalog.derby_defeat - extra,
                );
            }
        }
    }

    fn record_senior_debut(&mut self, _o: &MatchOutcome<'_>) {
        if self.made_senior_debut {
            return;
        }
        // Friendlies return earlier in `record_match_events`, so reaching here
        // with the latch unset IS this player's first senior competitive
        // appearance. Latch it permanently — `self.statistics` /
        // `self.cup_statistics` are wiped at every season / transfer boundary,
        // so the old `apps == 1` check re-fired this milestone on the opening
        // match of every season (the 3650-day cooldown couldn't stop it:
        // `recent_events` is pruned after 365 days, so the original event was
        // long gone by the time the next season started).
        self.made_senior_debut = true;
        self.happiness
            .add_event_default(HappinessEventType::SeniorDebut);
    }

    /// Track competitive scoring drought for forwards/midfielders. Updates
    /// the per-player `apps_since_last_competitive_goal` counter and
    /// emits at most one drought-related event per match (mutually
    /// exclusive — a goal that ends a drought never co-fires the
    /// concern).
    fn record_drought_signals(&mut self, o: &MatchOutcome<'_>) {
        let pos = self.position();
        let is_attacker = pos.is_forward() || pos.is_midfielder();
        if !is_attacker {
            return;
        }

        let drought_apps = self.happiness.apps_since_last_competitive_goal;
        if o.stats.goals > 0 {
            if drought_apps >= 8 {
                let extra = (((drought_apps as i32 - 8) as f32) * 0.25).clamp(0.0, 3.0);
                let mag = 3.5 + extra;
                self.happiness.add_event_with_cooldown(
                    HappinessEventType::GoalDroughtEnded,
                    mag,
                    21,
                );
            }
            self.happiness.apps_since_last_competitive_goal = 0;
        } else {
            self.happiness.apps_since_last_competitive_goal = self
                .happiness
                .apps_since_last_competitive_goal
                .saturating_add(1);
            // ScoringDroughtConcern is forward-only: midfielder drought
            // is real but doesn't carry the "what's wrong with our striker"
            // narrative.
            if pos.is_forward()
                && self.happiness.apps_since_last_competitive_goal >= 6
                && o.effective_rating < 6.8
            {
                let cfg = HappinessConfig::default();
                let prof_dampen = scaling::criticism_dampener(self.attributes.professionalism);
                let amb_amp = scaling::ambition_amplifier(self.attributes.ambition);
                let mag = cfg.catalog.scoring_drought_concern * prof_dampen * amb_amp;
                self.happiness.add_event_with_cooldown(
                    HappinessEventType::ScoringDroughtConcern,
                    mag,
                    30,
                );
            }
        }
    }

    /// Single-fire milestone events when competitive totals cross fixed
    /// thresholds in this match. Stats have already been updated, so the
    /// crossing is detected by inspecting `total_after` against the
    /// per-match contribution.
    fn record_milestones(&mut self, o: &MatchOutcome<'_>) {
        let games_after = self.statistics.played
            + self.statistics.played_subs
            + self.cup_statistics.played
            + self.cup_statistics.played_subs;
        // Apps ladder: this match contributed exactly +1 appearance.
        for &(threshold, mul) in &[
            (50u16, 0.8f32),
            (100, 1.0),
            (250, 1.25),
            (500, 1.6),
            (750, 2.0),
        ] {
            if games_after == threshold {
                let cfg = HappinessConfig::default();
                let mag = cfg.catalog.appearance_milestone * mul;
                self.happiness.add_event_with_cooldown(
                    HappinessEventType::AppearanceMilestone,
                    mag,
                    365,
                );
            }
        }

        let goals_after = self.statistics.goals + self.cup_statistics.goals;
        let goals_before = goals_after.saturating_sub(o.stats.goals);
        for &(threshold, mul) in &[
            (25u16, 0.8f32),
            (50, 1.0),
            (100, 1.25),
            (200, 1.6),
            (400, 2.0),
        ] {
            if goals_before < threshold && goals_after >= threshold {
                let cfg = HappinessConfig::default();
                let mag = cfg.catalog.goal_milestone * mul;
                self.happiness
                    .add_event_with_cooldown(HappinessEventType::GoalMilestone, mag, 365);
            }
        }

        if self.position().is_goalkeeper()
            && matches!(o.participation, MatchParticipation::Starter)
            && o.team_goals_against == 0
        {
            let cs_after = self.statistics.clean_sheets + self.cup_statistics.clean_sheets;
            let cs_before = cs_after.saturating_sub(1);
            for &(threshold, mul) in &[(25u16, 0.8f32), (50, 1.0), (100, 1.25), (200, 1.6)] {
                if cs_before < threshold && cs_after >= threshold {
                    let cfg = HappinessConfig::default();
                    let mag = cfg.catalog.clean_sheet_milestone * mul;
                    self.happiness.add_event_with_cooldown(
                        HappinessEventType::CleanSheetMilestone,
                        mag,
                        365,
                    );
                }
            }
        }
    }

    /// Hat-trick and assist hat-trick events. Reputation- and scene-amplified
    /// (cup / derby outings carry more narrative weight). Cooldown 30 days
    /// stops a freak two-hat-trick week from saturating the morale buffer.
    fn record_hat_tricks(&mut self, o: &MatchOutcome<'_>) {
        let cfg = HappinessConfig::default();
        let scene_mul = if o.is_cup || o.is_derby { 1.35 } else { 1.0 };
        let rep_mul = scaling::reputation_amplifier(self.player_attributes.current_reputation);

        if o.stats.goals >= 3 {
            let mag = cfg.catalog.hat_trick * scene_mul * rep_mul;
            self.happiness
                .add_event_with_cooldown(HappinessEventType::HatTrick, mag, 30);
        }
        if o.stats.assists >= 3 {
            let base = cfg.catalog.assist_hat_trick;
            let mag = base * scene_mul * rep_mul;
            self.happiness
                .add_event_with_cooldown(HappinessEventType::AssistHatTrick, mag, 30);
        }
    }

    /// `FansChantPlayerName` — joyous moment after a standout home shift.
    /// `MediaPressureMounting` — high-profile player accumulating poor
    /// performances or a red-card disgrace in a marquee fixture.
    fn record_fans_chant_and_media_pressure(&mut self, o: &MatchOutcome<'_>) {
        let cfg = HappinessConfig::default();
        let rep_mul = scaling::reputation_amplifier(self.player_attributes.current_reputation);
        let scene_mul = if o.is_cup || o.is_derby { 1.35 } else { 1.0 };

        let derby_hero_now = o.is_derby
            && o.team_won
            && (o.stats.goals > 0 || o.is_motm || o.effective_rating >= 7.5);
        let chant_trigger =
            o.team_won && (o.effective_rating >= 8.2 || o.stats.goals >= 3 || derby_hero_now);
        if chant_trigger {
            let mag = cfg.catalog.fans_chant_player_name * rep_mul * scene_mul;
            let event_ctx = MatchSupportContextBuilder::fans_chant(self, o, derby_hero_now, mag);
            self.happiness.add_event_with_context_and_cooldown(
                HappinessEventType::FansChantPlayerName,
                mag,
                None,
                event_ctx,
                45,
            );
        }

        // Sliding 5-bit window of "this match's rating was poor". Bit 0
        // is the most recent appearance, so a poor showing N matches ago
        // falls off naturally instead of waiting for a block boundary.
        const MEDIA_WINDOW: u8 = 5;
        const WINDOW_MASK: u8 = (1 << MEDIA_WINDOW) - 1; // 0b11111
        let poor_bit: u8 = if o.effective_rating < 6.0 { 1 } else { 0 };
        self.happiness.recent_low_rating_mask =
            ((self.happiness.recent_low_rating_mask << 1) | poor_bit) & WINDOW_MASK;
        self.happiness.recent_low_rating_len =
            (self.happiness.recent_low_rating_len + 1).min(MEDIA_WINDOW);

        let high_profile = self.player_attributes.current_reputation >= 5000;
        let poor_marquee = (o.is_cup || o.is_derby) && o.stats.red_cards > 0;
        let two_of_five = self.happiness.recent_low_rating_len >= MEDIA_WINDOW
            && self.happiness.recent_low_rating_mask.count_ones() >= 2;

        if high_profile && (two_of_five || poor_marquee) {
            let provoke_mul = scaling::criticism_amplifier(
                self.attributes.controversy,
                self.attributes.temperament,
            );
            let prof_dampen = scaling::criticism_dampener(self.attributes.professionalism);
            let mag = cfg.catalog.media_pressure_mounting * provoke_mul * prof_dampen;
            self.happiness.add_event_with_cooldown(
                HappinessEventType::MediaPressureMounting,
                mag,
                45,
            );
        }
    }

    /// `LeadershipEmergence` — senior, professional, high-leadership
    /// players step up after a heavy defeat (margin ≤ -3). Long
    /// cooldown so it stays a meaningful career marker, not a
    /// per-bad-result tag.
    fn record_leadership_emergence(&mut self, o: &MatchOutcome<'_>) {
        if !o.team_lost || o.goal_margin() > -3 {
            return;
        }
        let leadership = self.skills.mental.leadership;
        let prof = self.attributes.professionalism;
        if leadership < 16.0 || prof < 14.0 {
            return;
        }
        let cfg = HappinessConfig::default();
        let mag =
            cfg.catalog.leadership_emergence * scaling::loyalty_amplifier(self.attributes.loyalty);
        self.happiness
            .add_event_with_cooldown(HappinessEventType::LeadershipEmergence, mag, 120);
    }

    fn record_match_reputation(&mut self, o: &MatchOutcome<'_>) {
        let rating_delta = (o.effective_rating - 6.0) * 20.0;
        let goal_bonus = o.stats.goals.min(3) as f32 * 15.0;
        let assist_bonus = o.stats.assists.min(3) as f32 * 8.0;
        let motm_bonus = if o.is_motm { 25.0 } else { 0.0 };
        let raw_delta = rating_delta + goal_bonus + assist_bonus + motm_bonus;

        if o.is_friendly {
            let home_delta = (raw_delta * 0.4 * o.league_weight) as i16;
            self.player_attributes.update_reputation(0, home_delta, 0);
        } else {
            let current_delta = (raw_delta * o.league_weight) as i16;
            let home_delta = (raw_delta * 0.6 * o.league_weight) as i16;
            let world_delta = (raw_delta * o.world_weight * o.league_weight) as i16;
            self.player_attributes
                .update_reputation(current_delta, home_delta, world_delta);
        }
    }
}

/// Pick the right `PlayerStatistics` bucket for the match — league,
/// cup, or pre-season friendly — so the call sites read declaratively
/// (`stats_bucket_mut(p, o).goals += …`).
///
/// Cup matches route into the per-competition bucket keyed by the
/// match's competition slug; the rolled-up `cup_statistics` aggregate is
/// rebuilt from those buckets in `on_match_played` once recording is
/// done.
fn stats_bucket_mut<'a>(
    player: &'a mut Player,
    o: &MatchOutcome<'_>,
    home_slug: &str,
) -> &'a mut PlayerStatistics {
    if o.is_cup {
        player.cup_competition_statistics_mut(o.competition_slug)
    } else if o.is_friendly {
        &mut player.friendly_statistics
    } else if let Some(team) = o
        .played_for
        .as_ref()
        .filter(|t| !home_slug.is_empty() && t.slug != home_slug)
    {
        // Borrowed league appearance for another of the club's teams —
        // book it under that team (stored in the player's history) so it
        // surfaces as its own career-history row.
        player.statistics_history.secondary_team_statistics_mut(
            o.match_season_year,
            team.slug,
            team.name,
            team.reputation,
            team.league_slug,
            team.league_name,
        )
    } else {
        &mut player.statistics
    }
}

/// Magnitude scaling for `MatchDropped` events with structured
/// context. The base hurts more for players who *expected* to feature
/// (KeyPlayer / FirstTeamRegular), softens for protective scopes
/// (rest, returning from injury, low-importance rotation), and bumps
/// for repeated omissions. Friendlies always dampen — a missed
/// pre-season run-out doesn't sting like a league snub.
fn compute_drop_magnitude(player: &Player, ctx: &MatchSelectionContext) -> f32 {
    let cfg = HappinessConfig::default();
    let base = cfg.catalog.magnitude(HappinessEventType::MatchDropped);

    let status = player
        .contract
        .as_ref()
        .map(|c| c.squad_status.clone())
        .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
    let status_mul = match status {
        PlayerSquadStatus::KeyPlayer => 2.4,
        PlayerSquadStatus::FirstTeamRegular => 1.7,
        PlayerSquadStatus::FirstTeamSquadRotation => 1.0,
        PlayerSquadStatus::HotProspectForTheFuture => 0.7,
        PlayerSquadStatus::MainBackupPlayer => 0.6,
        _ => 0.5,
    };

    let scope_mul = match ctx.scope {
        SelectionDecisionScope::LeftOutOfMatchdaySquad => 1.4,
        SelectionDecisionScope::DroppedToBench => 1.1,
        SelectionDecisionScope::UnusedSubstitute => 0.7,
        SelectionDecisionScope::Rested => 0.4,
        SelectionDecisionScope::Rotation => 0.5,
        SelectionDecisionScope::UnavailableButNotInjured => 0.8,
    };

    let reason_mul = match ctx.reason {
        SelectionOmissionReason::FatigueManagement
        | SelectionOmissionReason::FitnessProtection
        | SelectionOmissionReason::ReturningFromInjury => 0.5,
        SelectionOmissionReason::CupRotation
        | SelectionOmissionReason::LowMatchImportanceRotation
        | SelectionOmissionReason::YouthDevelopmentRotation => 0.6,
        SelectionOmissionReason::TacticalMismatch
        | SelectionOmissionReason::PositionFitIssue
        | SelectionOmissionReason::NoNaturalRoleInFormation
        | SelectionOmissionReason::TeammatePreferredForTacticalBalance => 1.0,
        SelectionOmissionReason::PoorRecentForm
        | SelectionOmissionReason::ManagerDoesNotTrustPlayer => 1.3,
        SelectionOmissionReason::TeammatePreferredOnAbility
        | SelectionOmissionReason::TeammatePreferredOnForm
        | SelectionOmissionReason::TeammatePreferredOnFitness
        | SelectionOmissionReason::TeammatePreferredOnTrust => 1.0,
        SelectionOmissionReason::SquadStatusMismatch => 1.4,
        SelectionOmissionReason::DisciplinarySelection => 1.2,
        SelectionOmissionReason::NewcomerStillIntegrating => 0.7,
        SelectionOmissionReason::BenchBalance => 0.6,
        SelectionOmissionReason::LowerMatchReadiness => 0.8,
        // Opponent / role-duty / lineup-balance / bench-scenario reads
        // are all tactical / planning-driven explanations — they sting
        // less than a perceived-ability drop because the player can
        // see the football reason.
        SelectionOmissionReason::OpponentMatchupMismatch
        | SelectionOmissionReason::LineupBalanceCall
        | SelectionOmissionReason::BenchScenarioCoverage => 0.9,
        // A medical-caution call carries the same gentleness as the
        // existing fitness-protection variant.
        SelectionOmissionReason::MedicalRecurrenceRisk => 0.5,
        // An eligibility-rule block isn't a coaching judgement at all —
        // the player accepts the rule and the morale hit is minimal.
        SelectionOmissionReason::EligibilityRuleBlock => 0.4,
        // A player rested because he's agreed / is close to a move understands
        // the protection and is leaving anyway — almost no morale hit.
        SelectionOmissionReason::RestedDueToAgreedTransfer => 0.4,
        // Benching a disaffected want-away player is a consequence of his own
        // stance; he's already unhappy, so it's a moderate, not severe, hit.
        SelectionOmissionReason::OmittedDueToDisaffection => 0.9,
    };

    let repeat_mul = if ctx.repeated { 1.4 } else { 1.0 };
    let friendly_mul = if ctx.is_friendly { 0.3 } else { 1.0 };
    let importance_mul = (0.5_f32 + ctx.match_importance).clamp(0.5, 1.4);

    base * status_mul * scope_mul * reason_mul * repeat_mul * friendly_mul * importance_mul
}

fn drop_cause(ctx: &MatchSelectionContext) -> HappinessEventCause {
    match ctx.reason {
        SelectionOmissionReason::TacticalMismatch
        | SelectionOmissionReason::PositionFitIssue
        | SelectionOmissionReason::NoNaturalRoleInFormation
        | SelectionOmissionReason::TeammatePreferredForTacticalBalance => {
            HappinessEventCause::TacticalDisagreement
        }
        SelectionOmissionReason::PoorRecentForm => HappinessEventCause::PoorFormPressure,
        SelectionOmissionReason::TeammatePreferredOnAbility
        | SelectionOmissionReason::TeammatePreferredOnForm
        | SelectionOmissionReason::TeammatePreferredOnFitness
        | SelectionOmissionReason::TeammatePreferredOnTrust => {
            HappinessEventCause::PositionalRivalry
        }
        SelectionOmissionReason::ManagerDoesNotTrustPlayer => {
            HappinessEventCause::LeadershipDispute
        }
        SelectionOmissionReason::DisciplinarySelection => HappinessEventCause::PersonalityClash,
        SelectionOmissionReason::NewcomerStillIntegrating => {
            HappinessEventCause::AdaptationIsolation
        }
        _ => HappinessEventCause::Other,
    }
}

/// Pick the "what may happen next" hint that fits the scope. Repeated
/// drops escalate to the dressing-room damage warning; one-off
/// rotation calls keep the calmer "likely to settle" copy.
fn drop_follow_up(player: &Player, ctx: &MatchSelectionContext) -> Option<HappinessEventFollowUp> {
    if ctx.repeated {
        return Some(HappinessEventFollowUp::DressingRoomDamageRisk);
    }
    if matches!(
        ctx.scope,
        SelectionDecisionScope::Rested | SelectionDecisionScope::Rotation
    ) {
        return Some(HappinessEventFollowUp::LikelyToSettle);
    }
    let status = player
        .contract
        .as_ref()
        .map(|c| c.squad_status.clone())
        .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
    if matches!(
        status,
        PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
    ) {
        return Some(HappinessEventFollowUp::ManagerInterventionRisk);
    }
    Some(HappinessEventFollowUp::LikelyToSettle)
}

/// Builder for the structured `HappinessEventContext` payloads that
/// match-driven support events (`ManagerEncouragement`, `FanPraise`,
/// `FansChantPlayerName`) attach at emit time. Bundled under a named
/// type so the per-event constructors share a single namespace and the
/// call sites in `record_match_events` read as thin orchestration.
pub struct MatchSupportContextBuilder;

impl MatchSupportContextBuilder {
    /// Pick the dominant trigger for a manager-encouragement event from
    /// a post-match outcome. Higher-signal triggers (POM, decisive
    /// moment) beat lower-signal ones (high rating).
    fn manager_encouragement_trigger(
        player: &Player,
        o: &MatchOutcome<'_>,
        contributed: bool,
    ) -> SupportTrigger {
        if o.is_motm {
            SupportTrigger::PlayerOfMatch
        } else if contributed && o.team_won && o.goal_margin() == 1 {
            SupportTrigger::DecisiveMoment
        } else if contributed {
            SupportTrigger::GoalContribution
        } else if player.happiness.morale < 35.0 {
            SupportTrigger::PoorMorale
        } else if o.is_derby {
            SupportTrigger::Derby
        } else if o.is_cup {
            SupportTrigger::CupTie
        } else {
            SupportTrigger::HighRating
        }
    }

    /// Build the `HappinessEventContext` for a `ManagerEncouragement`
    /// event fired after a high post-match rating. Captures rating /
    /// contribution / setting so the renderer can describe what the
    /// manager liked.
    pub fn manager_encouragement(
        player: &Player,
        o: &MatchOutcome<'_>,
        magnitude: f32,
    ) -> HappinessEventContext {
        let contributed = o.stats.goals > 0 || o.stats.assists > 0;
        let trigger = Self::manager_encouragement_trigger(player, o, contributed);

        let support =
            SupportEventContext::new(SupportSource::Manager, SupportSetting::PostMatch, trigger)
                .with_match_rating(o.effective_rating)
                .with_goals(o.stats.goals as u8)
                .with_assists(o.stats.assists as u8)
                .with_team_won(o.team_won)
                .with_derby(o.is_derby)
                .with_cup(o.is_cup);

        let mut ctx = HappinessEventContext::new(
            HappinessEventCause::ManagerSupport,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::MatchDay,
        )
        .with_support_context(support);

        if o.is_motm {
            ctx = ctx.with_evidence(HappinessEventEvidence::PlayerOfTheMatch);
        }
        if contributed {
            ctx = ctx.with_evidence(HappinessEventEvidence::GoalContribution);
        }
        if o.team_won && contributed && o.goal_margin() == 1 {
            ctx = ctx.with_evidence(HappinessEventEvidence::DecisiveContribution);
        }
        if o.effective_rating >= 7.5 {
            ctx = ctx.with_evidence(HappinessEventEvidence::ExcellentPerformance);
        }
        if o.is_derby {
            ctx = ctx.with_evidence(HappinessEventEvidence::DerbyPerformance);
        }
        if o.is_cup {
            ctx = ctx.with_evidence(HappinessEventEvidence::CupPerformance);
        }
        if player.happiness.morale < 35.0 {
            ctx = ctx.with_evidence(HappinessEventEvidence::PoorMoraleBeforeTalk);
        }
        if player.attributes.professionalism >= 15.0 {
            ctx = ctx.with_evidence(HappinessEventEvidence::HighProfessionalism);
        }
        if player.attributes.pressure >= 15.0 {
            ctx = ctx.with_evidence(HappinessEventEvidence::HighPressurePersonality);
        } else if player.attributes.pressure <= 7.0 {
            ctx = ctx.with_evidence(HappinessEventEvidence::LowPressurePersonality);
        }

        ctx.with_follow_up(HappinessEventFollowUp::ManagerTrustRising)
    }

    /// Build the `HappinessEventContext` for a `FanPraise` event fired
    /// after a stand-out display. Captures the trigger (POM /
    /// contribution / high rating) plus scene flags.
    pub fn fan_praise(
        _player: &Player,
        o: &MatchOutcome<'_>,
        contributed: bool,
        magnitude: f32,
    ) -> HappinessEventContext {
        let trigger = if o.is_motm {
            SupportTrigger::PlayerOfMatch
        } else if contributed && o.team_won && o.goal_margin() == 1 {
            SupportTrigger::DecisiveMoment
        } else if contributed && o.team_won {
            SupportTrigger::GoalContribution
        } else if o.is_derby {
            SupportTrigger::Derby
        } else if o.is_cup {
            SupportTrigger::CupTie
        } else {
            SupportTrigger::HighRating
        };

        // The match-event pipeline does not surface home/away on
        // `MatchOutcome`; default to `HomeCrowd` for the supporters-of-
        // the-player-in-question — that's the side the renderer's
        // headline / reason copy is targeted at.
        let support = SupportEventContext::new(
            SupportSource::Supporters,
            SupportSetting::HomeCrowd,
            trigger,
        )
        .with_match_rating(o.effective_rating)
        .with_goals(o.stats.goals as u8)
        .with_assists(o.stats.assists as u8)
        .with_team_won(o.team_won)
        .with_derby(o.is_derby)
        .with_cup(o.is_cup);

        let mut ctx = HappinessEventContext::new(
            HappinessEventCause::SupporterAppreciation,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::MatchDay,
        )
        .with_support_context(support);

        if o.is_motm {
            ctx = ctx.with_evidence(HappinessEventEvidence::PlayerOfTheMatch);
        }
        if contributed {
            ctx = ctx.with_evidence(HappinessEventEvidence::GoalContribution);
        }
        if o.team_won && contributed && o.goal_margin() == 1 {
            ctx = ctx.with_evidence(HappinessEventEvidence::DecisiveContribution);
        }
        if o.effective_rating >= 8.0 {
            ctx = ctx.with_evidence(HappinessEventEvidence::ExcellentPerformance);
        }
        if o.is_derby {
            ctx = ctx.with_evidence(HappinessEventEvidence::DerbyPerformance);
        }
        if o.is_cup {
            ctx = ctx.with_evidence(HappinessEventEvidence::CupPerformance);
        }
        ctx = ctx.with_evidence(HappinessEventEvidence::HomeCrowdMoment);

        ctx.with_follow_up(HappinessEventFollowUp::FanStandingRising)
    }

    /// Build the `HappinessEventContext` for a `FansChantPlayerName`
    /// event. More selective than `FanPraise`: only fires for moments
    /// that change the match — the renderer should treat it as a
    /// stronger signal.
    pub fn fans_chant(
        _player: &Player,
        o: &MatchOutcome<'_>,
        derby_hero_now: bool,
        magnitude: f32,
    ) -> HappinessEventContext {
        let trigger = if o.stats.goals >= 3 {
            SupportTrigger::DecisiveMoment
        } else if derby_hero_now && o.is_derby {
            SupportTrigger::Derby
        } else if o.is_motm {
            SupportTrigger::PlayerOfMatch
        } else if o.is_cup {
            SupportTrigger::CupTie
        } else if o.stats.goals > 0 || o.stats.assists > 0 {
            SupportTrigger::GoalContribution
        } else {
            SupportTrigger::DecisiveMoment
        };

        let support = SupportEventContext::new(
            SupportSource::Supporters,
            SupportSetting::HomeCrowd,
            trigger,
        )
        .with_match_rating(o.effective_rating)
        .with_goals(o.stats.goals as u8)
        .with_assists(o.stats.assists as u8)
        .with_team_won(o.team_won)
        .with_derby(o.is_derby)
        .with_cup(o.is_cup);

        let mut ctx = HappinessEventContext::new(
            HappinessEventCause::SupporterIdentification,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::MatchDay,
        )
        .with_support_context(support);

        if o.stats.goals >= 3 {
            ctx = ctx.with_evidence(HappinessEventEvidence::DecisiveContribution);
        }
        if derby_hero_now {
            ctx = ctx.with_evidence(HappinessEventEvidence::DerbyPerformance);
        }
        if o.is_motm {
            ctx = ctx.with_evidence(HappinessEventEvidence::PlayerOfTheMatch);
        }
        if o.effective_rating >= 8.2 {
            ctx = ctx.with_evidence(HappinessEventEvidence::ExcellentPerformance);
        }
        ctx = ctx.with_evidence(HappinessEventEvidence::HomeCrowdMoment);

        ctx.with_follow_up(HappinessEventFollowUp::FanStandingRising)
    }
}

struct MatchPerfContextBuilder;

impl MatchPerfContextBuilder {
    fn standout(
        player: &Player,
        o: &MatchOutcome<'_>,
        kind: MatchPerformanceKind,
    ) -> MatchPerformanceEventContext {
        let mut mp = MatchPerformanceEventContext::new(kind)
            .with_rating(o.effective_rating)
            .with_minutes(o.stats.minutes_played as u16)
            .with_goals(o.stats.goals as u8)
            .with_assists(o.stats.assists as u8)
            .with_team_won(o.team_won)
            .with_goal_margin(o.goal_margin() as i8)
            .with_derby(o.is_derby)
            .with_cup(o.is_cup);
        if o.effective_rating >= 7.5 {
            mp = mp.with_evidence(MatchPerformanceEvidence::HighRating);
        }
        if o.stats.goals > 0 || o.stats.assists > 0 {
            mp = mp.with_evidence(MatchPerformanceEvidence::GoalContribution);
        }
        if o.team_won && (o.stats.goals > 0 || o.stats.assists > 0) && o.goal_margin() == 1 {
            mp = mp.with_evidence(MatchPerformanceEvidence::DecisiveContribution);
        }
        if o.is_derby {
            mp = mp.with_evidence(MatchPerformanceEvidence::DerbyFixture);
        }
        if o.is_cup {
            mp = mp.with_evidence(MatchPerformanceEvidence::CupTie);
        }
        if o.participation == MatchParticipation::Substitute {
            mp = mp.with_evidence(MatchPerformanceEvidence::SubstituteAppearance);
        }
        if player.attributes.pressure >= 15.0 {
            mp = mp.with_evidence(MatchPerformanceEvidence::HighPressurePersonality);
        }
        if player.attributes.important_matches >= 15.0 {
            mp = mp.with_evidence(MatchPerformanceEvidence::ImportantMatchTemperament);
        }
        mp
    }

    fn costly_error(player: &Player, o: &MatchOutcome<'_>) -> MatchPerformanceEventContext {
        let mut mp =
            MatchPerformanceEventContext::new(MatchPerformanceKind::CostlyErrorUnderPressure)
                .with_rating(o.effective_rating)
                .with_minutes(o.stats.minutes_played as u16)
                .with_team_won(o.team_won)
                .with_goal_margin(o.goal_margin() as i8)
                .with_derby(o.is_derby)
                .with_cup(o.is_cup)
                .with_evidence(MatchPerformanceEvidence::LowRating);
        if o.is_derby {
            mp = mp.with_evidence(MatchPerformanceEvidence::DerbyFixture);
        }
        if o.is_cup {
            mp = mp.with_evidence(MatchPerformanceEvidence::CupTie);
        }
        if player.attributes.pressure <= 7.0 {
            mp = mp.with_evidence(MatchPerformanceEvidence::LowPressurePersonality);
        }
        mp
    }
}

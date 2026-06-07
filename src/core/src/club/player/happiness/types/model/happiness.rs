use super::super::HappinessEventContext;
use super::HappinessEventType;
use crate::club::player::behaviour_config::HappinessConfig;
use chrono::NaiveDate;

#[derive(Debug, Clone)]
pub struct PlayerHappiness {
    pub morale: f32,
    pub factors: HappinessFactors,
    pub recent_events: Vec<HappinessEvent>,
    pub last_salary_negotiation: Option<NaiveDate>,
    /// EMA of "did I start this competitive match?" — updated on every
    /// non-friendly match. Drives the WonStartingPlace / LostStartingPlace
    /// transitions instead of raw season totals so a mid-season turnaround
    /// is felt promptly. Range 0.0..=1.0; 0.5 baseline before first match.
    pub starter_ratio: f32,
    /// Rolling count of recent competitive appearances feeding `starter_ratio`.
    /// Caps at u8::MAX; only the first 5 appearances are required before role
    /// transitions can fire (avoids one good week swinging the verdict).
    pub appearances_tracked: u8,
    /// Sticky flag — true once the player has been recognised as an
    /// established starter, false until they fall back below the bench
    /// threshold. Used to emit one-shot WonStartingPlace / LostStartingPlace
    /// events on the crossing rather than every matchday in range.
    pub is_established_starter: bool,
    /// Competitive appearances since the player's last competitive goal.
    /// Drives `GoalDroughtEnded` / `ScoringDroughtConcern` without an
    /// unbounded per-match history. Saturates at u8::MAX.
    pub apps_since_last_competitive_goal: u8,
    /// Bit-ring of the last 5 competitive appearances: 1 = rating < 6.0,
    /// 0 otherwise. Bit 0 is the most recent appearance. Drives the
    /// "two of the last five" trigger for `MediaPressureMounting` as a
    /// true sliding window — bad games never fall off a block boundary.
    pub recent_low_rating_mask: u8,
    /// Number of appearances currently encoded in `recent_low_rating_mask`,
    /// capped at 5. The trigger only fires once the mask is full, so a
    /// player who has just made his second poor appearance after one good
    /// one doesn't fire on a 1-of-2 ratio.
    pub recent_low_rating_len: u8,

    // ── Post-transfer playing-time opportunity tracking ──────────
    //
    // These counters power the match-opportunity gate: playing-time
    // frustration (complaints, LackOfPlayingTime, loan-minutes concern,
    // broken playing-time promises) is judged against the real official
    // fixtures the club has played since this player joined — never on
    // calendar days alone. Because `PlayerHappiness` is rebuilt on every
    // club change (`new()` / `clear()`), all five counters reset to 0 at
    // the moment of a transfer / loan / manual move, so they always count
    // "since join". Friendlies, and matches where the player was injured /
    // suspended / not yet eligible, are excluded. Saturate at `u16::MAX`.
    /// Official (non-friendly) matches the club played while this player
    /// was registered and fit since joining — the denominator for every
    /// playing-time judgement.
    pub eligible_official_matches_since_join: u16,
    /// Of those eligible matches, the ones the player started.
    pub starts_since_join: u16,
    /// …the ones the player came on as a substitute.
    pub sub_apps_since_join: u16,
    /// …the ones the player was named to the bench but never used.
    pub unused_bench_since_join: u16,
    /// …the ones the player was left out of the matchday squad entirely
    /// (still available — not injured / suspended).
    pub left_out_since_join: u16,

    /// Cumulative morale weight of poor-match criticism that was
    /// suppressed by the visible-row cooldown but is still wearing the
    /// player down. Always non-positive. Decays gently each weekly
    /// tick (see [`Self::decay_events`]) so a player on a sustained
    /// bad run keeps soaking damage even when the history shows only
    /// the first "Manager criticised performance" headline, while a
    /// player who recovers form sees the pressure fade. Summed into
    /// the morale recalculation as a hidden form-pressure factor.
    pub hidden_form_pressure: f32,

    /// Consecutive weekly ticks the player's CoachPlayerBond
    /// `conflict_risk` has stayed above the elevated band (>0.80).
    /// Reset to 0 the moment the bond eases. Drives the escalation
    /// ladder (private complaint → Unhappy → transfer request /
    /// public criticism) in the conflict-escalation pass, so the
    /// escalation can only fire once a problem has *persisted* — a
    /// single bad week alone never triggers a transfer request.
    /// Saturates at u8::MAX.
    pub conflict_risk_streak: u8,
}

#[derive(Debug, Clone, Default)]
pub struct HappinessFactors {
    pub playing_time: f32,
    pub salary_satisfaction: f32,
    pub manager_relationship: f32,
    pub ambition_fit: f32,
    pub injury_frustration: f32,
    pub recent_praise: f32,
    pub recent_discipline: f32,

    // ── Derived "life in the team" factors ──────────────────────
    /// Does the player understand his role and how he's being used?
    /// Drops on RoleMismatch / repeated tactical-role talks; rises
    /// when the player is in his preferred position with consistent
    /// minutes. Range roughly -8..+5.
    pub role_clarity: f32,
    /// Does the player believe the coaching staff is competent enough
    /// to coach him? Reads coach attribute scores against the player's
    /// own ability. A world-class player at a club with weak coaching
    /// loses respect quickly. Range roughly -8..+6.
    pub coach_credibility: f32,
    /// Where does the player sit in the dressing room — respected,
    /// resented, isolated, or influential? Built from leadership,
    /// reputation, and relations. Range roughly -6..+8.
    pub dressing_room_status: f32,
    /// Cultural / structural fit with the club — facilities, league
    /// level, language, lifestyle, ambition match. Range roughly -8..+6.
    pub club_fit: f32,
    /// Pressure load from fans, media, board expectations relative to
    /// the player's `pressure` personality. Range roughly -8..+3.
    pub pressure_load: f32,
    /// Trust the player has in the manager's word — distinct from the
    /// general manager_relationship. Built from kept-vs-broken
    /// promises and recent broken-promise count. Range roughly -10..+6.
    pub promise_trust: f32,
}

#[derive(Debug, Clone)]
pub struct HappinessEvent {
    pub event_type: HappinessEventType,
    pub magnitude: f32,
    pub days_ago: u16,
    /// Optional teammate / partner involved in this event. Lets the UI
    /// link the event description to a specific player (e.g. who the
    /// player bonded with, who the close friend was, who the mentor was).
    /// `None` for events that don't naturally involve a specific peer.
    pub partner_player_id: Option<u32>,
    /// Structured cause/evidence/impact payload attached at emit time.
    /// `None` for legacy events whose emit-site has not been upgraded yet
    /// (renderer falls back to the i18n string for those).
    pub context: Option<HappinessEventContext>,
}

impl PlayerHappiness {
    /// Maximum visible `ConflictWithTeammate` rows a single player can
    /// accrue in one processing tick across ALL emitters (behaviour
    /// pass, controversy roll, mentorship friction, training friction,
    /// match-day post-incident reactions). Real dressing rooms surface
    /// one or two big incidents per day, not five. The cap is enforced
    /// via [`Self::try_add_partner_context_with_same_tick_budget`] so
    /// every emitter consumes the same shared budget regardless of run
    /// order.
    pub const MAX_CONFLICT_WITH_TEAMMATE_PER_TICK: u8 = 2;
    /// Maximum visible `TeammateBonding` rows per player per tick. A
    /// touch higher than the conflict cap — bonding events are softer
    /// signals and a sociable player on a winning streak realistically
    /// hits a handful of "got on well with X today" moments.
    pub const MAX_TEAMMATE_BONDING_PER_TICK: u8 = 3;

    pub fn new() -> Self {
        let cfg = HappinessConfig::default();
        PlayerHappiness {
            morale: cfg.default_morale,
            factors: HappinessFactors::default(),
            recent_events: Vec::new(),
            last_salary_negotiation: None,
            starter_ratio: 0.5,
            appearances_tracked: 0,
            is_established_starter: false,
            apps_since_last_competitive_goal: 0,
            recent_low_rating_mask: 0,
            recent_low_rating_len: 0,
            eligible_official_matches_since_join: 0,
            starts_since_join: 0,
            sub_apps_since_join: 0,
            unused_bench_since_join: 0,
            left_out_since_join: 0,
            hidden_form_pressure: 0.0,
            conflict_risk_streak: 0,
        }
    }

    /// Record one official (non-friendly) match the player started or came
    /// on in. Bumps the eligible-match denominator alongside the relevant
    /// involvement counter. No-op bookkeeping helper — call sites stay thin.
    pub fn note_official_appearance(&mut self, started: bool) {
        self.eligible_official_matches_since_join =
            self.eligible_official_matches_since_join.saturating_add(1);
        if started {
            self.starts_since_join = self.starts_since_join.saturating_add(1);
        } else {
            self.sub_apps_since_join = self.sub_apps_since_join.saturating_add(1);
        }
    }

    /// Record one official (non-friendly) match the club played in which
    /// the player was available but did not feature — an unused-bench or
    /// left-out opportunity. Counts toward the eligible-match denominator
    /// so a player who is repeatedly overlooked accrues a real deficit,
    /// while a club that simply hasn't played leaves the counters at zero.
    pub fn note_official_non_appearance(&mut self, left_out: bool) {
        self.eligible_official_matches_since_join =
            self.eligible_official_matches_since_join.saturating_add(1);
        if left_out {
            self.left_out_since_join = self.left_out_since_join.saturating_add(1);
        } else {
            self.unused_bench_since_join = self.unused_bench_since_join.saturating_add(1);
        }
    }

    pub fn recalculate_morale(&mut self) {
        let cfg = HappinessConfig::default();
        let core_factor_sum = self.factors.playing_time
            + self.factors.salary_satisfaction
            + self.factors.manager_relationship
            + self.factors.ambition_fit
            + self.factors.injury_frustration
            + self.factors.recent_praise
            + self.factors.recent_discipline;

        // Derived "life in the team" factors. Weighted to 0.6× of their
        // raw range so they enrich morale without dominating the core
        // axes the audit already balances around. Each factor is
        // independently clamped at compute time.
        let derived_sum = (self.factors.role_clarity
            + self.factors.coach_credibility
            + self.factors.dressing_room_status
            + self.factors.club_fit
            + self.factors.pressure_load
            + self.factors.promise_trust)
            * 0.6;

        let event_sum: f32 = self
            .recent_events
            .iter()
            .map(|e| e.magnitude * cfg.event_decay(e.days_ago))
            .sum();

        // Hidden form pressure: poor-match criticism that was throttled
        // out of the visible feed but is still wearing the player down.
        // Clamped to the same -10..0 band the visible `recent_discipline`
        // factor uses, so suppression never produces a bigger sting than
        // an emitted event would have.
        let hidden_pressure = self.hidden_form_pressure.clamp(-10.0, 0.0);

        self.morale = cfg.clamp_morale(
            cfg.default_morale + core_factor_sum + derived_sum + event_sum + hidden_pressure,
        );
    }

    /// Record a suppressed poor-match criticism into the hidden
    /// form-pressure accumulator. `magnitude` is the negative morale
    /// delta the visible event would have applied; we add a fraction
    /// (matching the legacy `adjust_morale(mag * 0.5)` semantics) so the
    /// player keeps absorbing pressure even when their history feed
    /// only shows the first headline. Clamped to the floor so a
    /// sustained slump can't push morale arbitrarily low.
    pub fn accumulate_hidden_form_pressure(&mut self, magnitude: f32) {
        // Only negative magnitudes feed pressure — a clamped positive
        // input would otherwise reduce earlier-accrued damage on its
        // own.
        if magnitude >= 0.0 {
            return;
        }
        self.hidden_form_pressure = (self.hidden_form_pressure + magnitude * 0.5).clamp(-10.0, 0.0);
    }

    pub fn adjust_morale(&mut self, delta: f32) {
        let cfg = HappinessConfig::default();
        self.morale = cfg.clamp_morale(self.morale + delta);
    }

    pub fn decay_events(&mut self) {
        let cfg = HappinessConfig::default();
        for event in &mut self.recent_events {
            event.days_ago += cfg.decay_step_days;
        }
        self.recent_events
            .retain(|e| e.days_ago <= cfg.event_retention_days);

        if self.recent_events.len() > cfg.recent_events_cap {
            self.recent_events
                .sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
            self.recent_events.truncate(cfg.recent_events_cap);
        }

        // Hidden form pressure fades a touch faster than the headline
        // event-magnitude decay so a player on a long quiet streak
        // doesn't keep absorbing suppressed criticism indefinitely.
        // Snap to zero once the residual is below the renderer's
        // noise floor.
        self.hidden_form_pressure *= 0.85;
        if self.hidden_form_pressure.abs() < 0.05 {
            self.hidden_form_pressure = 0.0;
        }
    }

    pub fn add_event(&mut self, event_type: HappinessEventType, magnitude: f32) {
        self.add_event_with_partner(event_type, magnitude, None);
    }

    /// Same as `add_event` but tags the event with a teammate / partner
    /// player id so the UI can render an inline link. Use this for events
    /// that naturally involve a specific peer (TeammateBonding,
    /// ConflictWithTeammate, CloseFriendSold, MentorDeparted,
    /// CompatriotJoined). The partner id has no effect on morale — it's
    /// purely informational.
    ///
    /// Enforcement: events listed in `requires_partner_id` MUST be emitted
    /// with a `Some(_)` partner id. Calls that pass `None` for those types
    /// are dropped here — the event would otherwise reach the UI as
    /// orphaned text ("bonded with a teammate" — which one?), be filtered
    /// out at render time, and waste a slot in `recent_events`. Failing
    /// silently at the source forces the emit-site to either supply the
    /// partner id or pick a different event type.
    pub fn add_event_with_partner(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: Option<u32>,
    ) {
        self.add_event_full(event_type, magnitude, partner_player_id, None);
    }

    /// Same as [`Self::add_event_with_partner`] but also attaches a
    /// structured [`HappinessEventContext`] for the renderer. Used by
    /// the upgraded emit sites (PlayerBehaviourResult, controversy
    /// pipeline, transfer-social, squad integration) so the UI can
    /// produce a real explanation instead of a static black-box line.
    pub fn add_event_with_context(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: Option<u32>,
        context: HappinessEventContext,
    ) {
        self.add_event_full(event_type, magnitude, partner_player_id, Some(context));
    }

    /// Cooldown-gated counterpart of `add_event_with_context`.
    pub fn add_event_with_context_and_cooldown(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: Option<u32>,
        context: HappinessEventContext,
        cooldown_days: u16,
    ) -> bool {
        if self.has_recent_event(&event_type, cooldown_days) {
            return false;
        }
        self.add_event_full(event_type, magnitude, partner_player_id, Some(context));
        true
    }

    /// One-call wrapper for partner events that need both a shared
    /// same-tick visible budget AND the standard per-partner cooldown.
    /// Checks happen in a specific order — the cheapest reject first:
    ///
    ///   1. **Same-tick budget**: if `same_tick_event_count(type)` has
    ///      already hit `max_same_tick`, refuse. Lets every emitter
    ///      (behaviour pass, controversy, mentorship, training) share
    ///      a single visible-row budget regardless of run order — the
    ///      first to land claims a slot, later ones bounce off.
    ///   2. **Partner cooldown**: standard per-`(type, partner_id)`
    ///      gate via `has_recent_event_with_partner` so a chronic
    ///      friction pair doesn't refire its row every weekly tick.
    ///   3. **Emit**: push the event.
    ///
    /// Returns `true` only if the event was actually pushed. Both gate
    /// rejects return `false` so callers can short-circuit any
    /// post-emit bookkeeping. The underlying relation update should
    /// always happen at the call site BEFORE this is called — the
    /// helper governs the *visible row*, not the drift.
    pub fn try_add_partner_context_with_same_tick_budget(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: u32,
        context: HappinessEventContext,
        partner_cooldown_days: u16,
        max_same_tick: u8,
    ) -> bool {
        if self.same_tick_event_count(&event_type) >= max_same_tick as usize {
            return false;
        }
        self.add_event_with_partner_context_and_cooldown(
            event_type,
            magnitude,
            partner_player_id,
            context,
            partner_cooldown_days,
        )
    }

    /// Cooldown-gated, partner-aware counterpart of
    /// `add_event_with_context`. Cooldown is keyed by `(event_type,
    /// partner_id)` so a chronic friction pair doesn't suppress a
    /// different teammate's first incident with the same type.
    pub fn add_event_with_partner_context_and_cooldown(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: u32,
        context: HappinessEventContext,
        cooldown_days: u16,
    ) -> bool {
        if self.has_recent_event_with_partner(&event_type, partner_player_id, cooldown_days) {
            return false;
        }
        self.add_event_full(
            event_type,
            magnitude,
            Some(partner_player_id),
            Some(context),
        );
        true
    }

    fn add_event_full(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: Option<u32>,
        context: Option<HappinessEventContext>,
    ) {
        if requires_partner_id(&event_type) && partner_player_id.is_none() {
            debug_assert!(
                false,
                "{:?} requires a partner_player_id; use add_event_with_partner",
                event_type
            );
            return;
        }
        if let Some(ctx) = context.as_ref() {
            // Specialized payloads are mutually exclusive — an event is
            // *either* a selection event, *or* a transfer-interest event,
            // etc. Attaching two at the same emit site is a bug that would
            // confuse the renderer's dispatch and produce mixed copy.
            debug_assert!(
                ctx.specialized_payload_count() <= 1,
                "{:?} carries {} specialized payloads (max 1); emit site attached more than one with_*_context",
                event_type,
                ctx.specialized_payload_count()
            );
        }
        let cfg = HappinessConfig::default();
        self.recent_events.push(HappinessEvent {
            event_type,
            magnitude,
            days_ago: 0,
            partner_player_id,
            context,
        });

        if self.recent_events.len() > cfg.recent_events_cap {
            self.recent_events
                .sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
            self.recent_events.truncate(cfg.recent_events_cap);
        }
    }

    /// True if an event of `event_type` was recorded within the last
    /// `days` days (inclusive). Cheap O(n) scan — `recent_events` is
    /// capped, so this is bounded.
    pub fn has_recent_event(&self, event_type: &HappinessEventType, days: u16) -> bool {
        self.recent_events
            .iter()
            .any(|e| e.event_type == *event_type && e.days_ago <= days)
    }

    /// Same as [`Self::has_recent_event`] but filtered to events tagged
    /// with the given partner. Use this for per-pair cooldowns — e.g.
    /// to avoid emitting "ConflictWithTeammate (vs player X)" every
    /// week when the underlying friction is constant.
    pub fn has_recent_event_with_partner(
        &self,
        event_type: &HappinessEventType,
        partner_player_id: u32,
        days: u16,
    ) -> bool {
        self.recent_events.iter().any(|e| {
            e.event_type == *event_type
                && e.partner_player_id == Some(partner_player_id)
                && e.days_ago <= days
        })
    }

    /// Count visible events of this type recorded in the current
    /// processing tick (i.e. `days_ago == 0`). Use this as a soft
    /// cross-emitter budget: controversy / mentorship / training
    /// friction / behaviour-pass relationship rows all push to the same
    /// history; without a shared budget a young player on a noisy day
    /// can pick up six "ConflictWithTeammate" rows that each came from
    /// a different subsystem. Cheap O(n) scan — `recent_events` is
    /// bounded.
    pub fn same_tick_event_count(&self, event_type: &HappinessEventType) -> usize {
        self.recent_events
            .iter()
            .filter(|e| e.event_type == *event_type && e.days_ago == 0)
            .count()
    }

    /// Has a `ManagerCriticism` event with this specific criticism
    /// reason been emitted within `days`? Drives the reason-aware
    /// cooldown gate in [`Player::on_match_played`] — a fresh sub-6.3
    /// rating for the same reason inside the window is suppressed, but
    /// a materially different reason (e.g. red-card `PublicComplaint`
    /// after a string of `PoorPressing` rows) is still allowed
    /// through. Events without a `manager_interaction_context` or
    /// without a populated `criticism_reason` never match — they're
    /// counted as "reason unknown" and don't block the new event.
    pub fn has_recent_manager_criticism_with_reason(
        &self,
        reason: crate::ManagerCriticismReason,
        days: u16,
    ) -> bool {
        self.recent_events.iter().any(|e| {
            if e.event_type != HappinessEventType::ManagerCriticism || e.days_ago > days {
                return false;
            }
            e.context
                .as_ref()
                .and_then(|c| c.manager_interaction_context.as_ref())
                .and_then(|m| m.criticism_reason)
                == Some(reason)
        })
    }

    /// Add an event only if no event of this type was emitted in the
    /// last `cooldown_days`. Returns `true` if the event was recorded.
    /// Centralised cooldown gate so emit sites don't reimplement the
    /// "did we already fire this recently" pattern (the audit found
    /// inline copies in `process_contract_jealousy` and
    /// `process_periodic_wage_envy`).
    pub fn add_event_with_cooldown(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        cooldown_days: u16,
    ) -> bool {
        if self.has_recent_event(&event_type, cooldown_days) {
            return false;
        }
        self.add_event(event_type, magnitude);
        true
    }

    /// Cooldown-gated counterpart of `add_event_with_partner`. Use this
    /// for partner-required events that also want a cooldown — emitting
    /// via the partner-less variant would be silently dropped by the
    /// `requires_partner_id` guard.
    pub fn add_event_with_partner_and_cooldown(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: Option<u32>,
        cooldown_days: u16,
    ) -> bool {
        if self.has_recent_event(&event_type, cooldown_days) {
            return false;
        }
        self.add_event_with_partner(event_type, magnitude, partner_player_id);
        true
    }

    /// Catalog-default counterpart of [`add_event_with_cooldown`].
    pub fn add_event_default_with_cooldown(
        &mut self,
        event_type: HappinessEventType,
        cooldown_days: u16,
    ) -> bool {
        if self.has_recent_event(&event_type, cooldown_days) {
            return false;
        }
        self.add_event_default(event_type);
        true
    }

    /// Record an event using the catalog's default magnitude. Equivalent
    /// to `add_event(event_type, catalog.magnitude(event_type))`. Use this
    /// for emit sites whose magnitude is the canonical default — single-
    /// magnitude events that don't depend on context.
    pub fn add_event_default(&mut self, event_type: HappinessEventType) {
        let cfg = HappinessConfig::default();
        let magnitude = cfg.catalog.magnitude(event_type.clone());
        self.add_event(event_type, magnitude);
    }

    /// Record an event with a magnitude scaled relative to the catalog
    /// default. `factor=1.0` is equivalent to `add_event_default`. Use
    /// this for emit sites where the magnitude varies by context (severity,
    /// loan damp, etc.) but the *base* should still come from the catalog.
    pub fn add_event_scaled(&mut self, event_type: HappinessEventType, factor: f32) {
        let cfg = HappinessConfig::default();
        let magnitude = cfg.catalog.magnitude(event_type.clone()) * factor;
        self.add_event(event_type, magnitude);
    }

    /// Reset happiness to neutral state (fresh start at a new club).
    /// `HappinessFactors::default()` zeroes all six derived factors —
    /// they're recomputed on the first weekly tick at the new club.
    pub fn clear(&mut self) {
        let cfg = HappinessConfig::default();
        self.morale = cfg.default_morale;
        self.factors = HappinessFactors::default();
        self.recent_events.clear();
        self.last_salary_negotiation = None;
        self.starter_ratio = 0.5;
        self.appearances_tracked = 0;
        self.is_established_starter = false;
        self.apps_since_last_competitive_goal = 0;
        self.recent_low_rating_mask = 0;
        self.recent_low_rating_len = 0;
        self.eligible_official_matches_since_join = 0;
        self.starts_since_join = 0;
        self.sub_apps_since_join = 0;
        self.unused_bench_since_join = 0;
        self.left_out_since_join = 0;
        self.hidden_form_pressure = 0.0;
        self.conflict_risk_streak = 0;
    }

    /// Backward compatible: morale >= happy_threshold means happy.
    pub fn is_happy(&self) -> bool {
        self.morale >= HappinessConfig::default().happy_threshold
    }

    /// Backward compatible: push a positive event
    pub fn add_positive(&mut self, _item: PositiveHappiness) {
        self.add_event_default(HappinessEventType::GoodTraining);
    }

    /// Backward compatible: push a negative event
    pub fn add_negative(&mut self, _item: NegativeHappiness) {
        self.add_event_default(HappinessEventType::PoorTraining);
    }
}

/// Event types that name a specific teammate and therefore must carry a
/// `partner_player_id`. Mirrors the web layer's `is_partner_required`
/// gate — kept here as the source of truth so emit-side and render-side
/// agree. Adding a new partner-style event type means listing it both
/// here (to enforce at emit) and in the web filter (to render the link).
fn requires_partner_id(event_type: &HappinessEventType) -> bool {
    matches!(
        event_type,
        HappinessEventType::TeammateBonding
            | HappinessEventType::ConflictWithTeammate
            | HappinessEventType::CloseFriendSold
            | HappinessEventType::MentorDeparted
            | HappinessEventType::CompatriotJoined
    )
}

/// Kept for backward compatibility
#[derive(Debug, Clone)]
pub struct PositiveHappiness {
    pub description: String,
}

/// Kept for backward compatibility
#[derive(Debug, Clone)]
pub struct NegativeHappiness {
    pub description: String,
}

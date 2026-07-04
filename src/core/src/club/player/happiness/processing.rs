use crate::PlayerPositionType;
use crate::club::player::adaptation::ReputationGap;
use crate::club::player::calculators::{
    ContractValuation, ValuationContext, expected_annual_value, package_inputs_from_contract,
};
use crate::club::player::interaction::InteractionTopic;
use crate::club::player::player::Player;
use crate::club::{PlayerResult, PlayerStatusType};
use crate::utils::DateUtils;
use crate::{
    ContractType, HappinessEventEvidence, HappinessEventFollowUp, HappinessEventScope,
    HappinessEventSeverity, HappinessEventType, PlayerSquadStatus,
};
use chrono::NaiveDate;

/// Club / coaching context fed into the six derived morale factors.
/// Decoupled from `GlobalContext` so the helpers can be unit-tested
/// without spinning up a simulator. Built once per weekly tick by the
/// caller (see `Player::simulate`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ClubMoraleContext {
    /// Best technical coach score on the club staff (0..20). Drives
    /// coach_credibility for outfield players.
    pub coach_best_technical: u8,
    /// Best mental coach score on the club staff (0..20). Drives
    /// coach_credibility weight for high-pressure / tactical players.
    pub coach_best_mental: u8,
    /// Best fitness coach score on the club staff (0..20). Drives
    /// coach_credibility for athleticism-dependent roles.
    pub coach_best_fitness: u8,
    /// Best goalkeeping coach score on the club staff (0..20). Drives
    /// coach_credibility for goalkeepers specifically.
    pub coach_best_goalkeeping: u8,
    /// Training facility quality (0..1) — feeds club_fit for ambitious
    /// pros who expect modern facilities.
    pub training_facility_quality: f32,
    /// Youth facility quality (0..1) — feeds club_fit for young players.
    pub youth_facility_quality: f32,
}

/// Snapshot of the team's current competitive standing — fed into
/// `calculate_ambition_fit` so a high-ambition player at a club who's
/// bottom of the table reacts to the season, not just to the club badge.
#[derive(Debug, Clone, Copy, Default)]
pub struct TeamSeasonState {
    /// 1-based league position; 0 if unknown.
    pub league_position: u8,
    /// Number of teams in the league; 0 if unknown.
    pub league_size: u8,
    /// Season progress 0.0..1.0 (matches played / total league matches).
    pub season_progress: f32,
    /// League reputation (0..10000). Contextualises position — top of
    /// a Tier-4 league isn't the same as top of the Premier League.
    pub league_reputation: u16,
}

/// Post-transfer playing-time opportunity snapshot. Built from the
/// `since_join` counters on [`PlayerHappiness`] plus the days elapsed
/// since the move. The whole point of this type is that playing-time
/// frustration is judged on **real eligible fixtures**, not calendar
/// days: a player who joined a club that hasn't kicked a ball yet has
/// `eligible_official_matches_since_join == 0` and can never be unhappy
/// about minutes he was never denied.
#[derive(Debug, Clone, Copy)]
pub struct PlayingTimeOpportunityContext {
    pub days_since_join: i64,
    /// Official (non-friendly) matches the club played since the player
    /// joined. Tracked at the player level, so it equals the
    /// `eligible_*` count (matches the player missed through injury /
    /// suspension are not observable here and are excluded by design).
    pub official_team_matches_since_join: u16,
    /// Of those, the ones the player was registered and fit for.
    pub eligible_official_matches_since_join: u16,
    pub player_starts_since_join: u16,
    pub player_sub_apps_since_join: u16,
    pub player_unused_bench_since_join: u16,
    pub player_left_out_since_join: u16,
    /// True when the player has been registered and available (not
    /// currently injured). Mirrors the per-match eligibility filter.
    pub was_registered_and_fit: bool,
    pub is_loan: bool,
}

impl PlayingTimeOpportunityContext {
    /// Weighted involvement score — starts count fully, cameos partly, an
    /// unused-bench spot a token amount (the player at least travelled and
    /// warmed up). Left-out matches contribute nothing.
    pub fn actual_involvement_score(&self, cfg: &PlayingTimeFrustrationConfig) -> f32 {
        self.player_starts_since_join as f32 * cfg.start_weight
            + self.player_sub_apps_since_join as f32 * cfg.sub_app_weight
            + self.player_unused_bench_since_join as f32 * 0.10
    }

    /// Grace ramp applied to negative (frustration) magnitudes. 0.0 inside
    /// the hard grace window, linearly up to 1.0 across the soft window,
    /// then full weight. Never overrides the zero-match hard block — that
    /// lives in [`Self::can_judge`].
    pub fn frustration_multiplier(&self, cfg: &PlayingTimeFrustrationConfig) -> f32 {
        if self.days_since_join < cfg.hard_grace_days_after_transfer {
            0.0
        } else if self.days_since_join < cfg.soft_grace_days_after_transfer {
            let span =
                (cfg.soft_grace_days_after_transfer - cfg.hard_grace_days_after_transfer).max(1);
            ((self.days_since_join - cfg.hard_grace_days_after_transfer) as f32 / span as f32)
                .clamp(0.0, 1.0)
        } else {
            1.0
        }
    }

    /// The full match-opportunity gate. Returns `Some(frustration_multiplier)`
    /// when a playing-time complaint / `LackOfPlayingTime` event /
    /// loan-minutes concern / broken playing-time promise may fire, and
    /// `None` when every such signal must be suppressed.
    ///
    /// `None` is returned — the zero-match invariant — whenever:
    ///   * there are no eligible official matches since the player joined;
    ///   * the player is `NotNeeded` (accepts their fate);
    ///   * we're still inside the hard grace window;
    ///   * the club hasn't played the minimum number of matches yet;
    ///   * the status-specific eligible-match sample isn't met.
    pub fn can_judge(
        &self,
        status: Option<&PlayerSquadStatus>,
        cfg: &PlayingTimeFrustrationConfig,
        loan_min_appearances: Option<u16>,
    ) -> Option<f32> {
        // ── Zero-match hard block — never overridden by grace ──
        if self.eligible_official_matches_since_join == 0 {
            return None;
        }
        if matches!(status, Some(PlayerSquadStatus::NotNeeded)) {
            return None;
        }
        if self.days_since_join < cfg.hard_grace_days_after_transfer {
            return None;
        }
        if self.eligible_official_matches_since_join < cfg.min_team_matches_after_transfer {
            return None;
        }
        let min_eligible = cfg.min_eligible_matches_for_status(status, loan_min_appearances);
        if self.eligible_official_matches_since_join < min_eligible {
            return None;
        }
        Some(self.frustration_multiplier(cfg))
    }
}

/// Tunable coefficients for the match-opportunity playing-time model.
/// All defaults follow the design spec; kept as a struct so a future
/// per-save override can be threaded through without touching call sites.
#[derive(Debug, Clone, Copy)]
pub struct PlayingTimeFrustrationConfig {
    pub hard_grace_days_after_transfer: i64,
    pub soft_grace_days_after_transfer: i64,
    pub min_team_matches_after_transfer: u16,
    pub min_player_apps_sample: u16,
    pub friendlies_weight: f32,
    pub unused_sub_weight: f32,
    pub left_out_weight: f32,
    pub start_weight: f32,
    pub sub_app_weight: f32,
    pub complaint_threshold: f32,
    pub promise_breach_threshold: f32,
    pub max_negative_playing_time_factor: f32,
    pub max_positive_playing_time_factor: f32,
}

impl Default for PlayingTimeFrustrationConfig {
    fn default() -> Self {
        PlayingTimeFrustrationConfig {
            hard_grace_days_after_transfer: 14,
            soft_grace_days_after_transfer: 45,
            min_team_matches_after_transfer: 2,
            min_player_apps_sample: 5,
            friendlies_weight: 0.25,
            unused_sub_weight: 0.35,
            left_out_weight: 1.0,
            start_weight: 1.0,
            sub_app_weight: 0.45,
            complaint_threshold: -10.0,
            promise_breach_threshold: -12.0,
            max_negative_playing_time_factor: -20.0,
            max_positive_playing_time_factor: 20.0,
        }
    }
}

impl PlayingTimeFrustrationConfig {
    /// Expected share of the club's eligible matches a player of this
    /// squad status counts on starting. Drives the deficit model — the
    /// gap between expectation and actual involvement is what frustrates.
    pub fn expected_start_share(status: Option<&PlayerSquadStatus>) -> f32 {
        match status {
            Some(PlayerSquadStatus::KeyPlayer) => 0.70,
            Some(PlayerSquadStatus::FirstTeamRegular) => 0.50,
            Some(PlayerSquadStatus::FirstTeamSquadRotation) => 0.25,
            Some(PlayerSquadStatus::MainBackupPlayer) => 0.15,
            Some(PlayerSquadStatus::HotProspectForTheFuture) => 0.10,
            Some(PlayerSquadStatus::DecentYoungster) => 0.08,
            _ => 0.30,
        }
    }

    /// Minimum eligible official matches before a playing-time judgement
    /// is allowed, per squad status. A loan with an explicit
    /// minimum-appearance clause uses `max(3, ceil(loan_min * 0.15))`.
    pub fn min_eligible_matches_for_status(
        &self,
        status: Option<&PlayerSquadStatus>,
        loan_min_appearances: Option<u16>,
    ) -> u16 {
        if let Some(min_apps) = loan_min_appearances {
            let scaled = ((min_apps as f32 * 0.15).ceil()) as u16;
            return scaled.max(3);
        }
        match status {
            Some(PlayerSquadStatus::KeyPlayer) => 2,
            Some(PlayerSquadStatus::FirstTeamRegular) => 3,
            Some(PlayerSquadStatus::FirstTeamSquadRotation) => 5,
            Some(PlayerSquadStatus::MainBackupPlayer) => 6,
            Some(PlayerSquadStatus::HotProspectForTheFuture) => 6,
            Some(PlayerSquadStatus::DecentYoungster) => 6,
            Some(PlayerSquadStatus::NotNeeded) => u16::MAX, // never complains
            _ => 5,
        }
    }
}

/// Post-transfer "honeymoon" ramp applied to the soft *settling-in* morale
/// factors (role clarity, coach credibility, dressing-room status, club fit,
/// pressure load). For the first few weeks at a new club these axes are
/// noisy — the player is still finding his feet — so their *negative*
/// contribution is scaled down and recovers linearly to full weight by
/// [`Self::RAMP_DAYS`]. The hard, real signals (wages, ambition fit, playing
/// time, broken promises) are deliberately NOT routed through here: a genuine
/// wage betrayal or a no-role formation still bites on day one.
#[derive(Debug, Clone, Copy)]
struct TransferHoneymoon {
    negative_scale: f32,
}

impl TransferHoneymoon {
    /// Days over which the damp recovers from [`Self::DAY0_SCALE`] to 1.0.
    const RAMP_DAYS: i64 = 28;
    /// Scale applied to negative settling factors on the day of the move.
    const DAY0_SCALE: f32 = 0.25;

    /// Build the ramp from days elapsed since the last transfer. No recent
    /// transfer (or a long-settled player) → no damping.
    fn since(days_since_transfer: Option<i64>) -> Self {
        let negative_scale = match days_since_transfer {
            Some(d) if (0..Self::RAMP_DAYS).contains(&d) => {
                let progress = d as f32 / Self::RAMP_DAYS as f32;
                Self::DAY0_SCALE + progress * (1.0 - Self::DAY0_SCALE)
            }
            _ => 1.0,
        };
        TransferHoneymoon { negative_scale }
    }

    /// Scale a settling factor's *negative* magnitude by the ramp. Positive
    /// values pass through untouched — a newcomer who's already a respected
    /// dressing-room presence keeps that lift.
    fn damp_negative(&self, value: f32) -> f32 {
        if value < 0.0 {
            value * self.negative_scale
        } else {
            value
        }
    }
}

/// Verdict on whether a player should hold the formal `Unh` status this
/// weekly tick. Separates three concerns the old inline check conflated:
///   * **eligibility** — is morale low enough, and is there a real concern
///     behind it (rather than a transient settling dip)?
///   * **severity** — is a *major* trigger present (broken promise, severe
///     wage betrayal, public conflict, repeated official-match omission)
///     that should harden the status immediately, skipping the persistence
///     wait?
///   * **recovery** — has morale climbed clear of the unhappy band?
#[derive(Debug, Clone, Copy)]
struct UnhappyAssessment {
    eligible: bool,
    major_trigger: bool,
    recover: bool,
}

impl UnhappyAssessment {
    /// Below this morale the player is eligible for `Unh` on its own.
    const CRITICAL_MORALE: f32 = 30.0;
    /// In the `[CRITICAL_MORALE, CONCERN_MORALE)` band a serious concern
    /// factor/event must also be present for eligibility.
    const CONCERN_MORALE: f32 = 35.0;
    /// Morale above this clears the status outright.
    const RECOVER_MORALE: f32 = 50.0;
    /// …or above this with the player visibly getting his minutes.
    const RECOVER_WITH_MINUTES_MORALE: f32 = 40.0;
    /// Weekly ticks an ordinary low mood must persist before it hardens.
    const PERSISTENCE_TICKS: u8 = 2;

    fn evaluate(player: &Player, playing_time_factor: f32) -> Self {
        let h = &player.happiness;
        let morale = h.morale;
        let f = &h.factors;
        let major_trigger = Self::has_major_event(player) || playing_time_factor <= -12.0;

        // A "serious concern" is a hard, real grievance — not the soft
        // settling factors the honeymoon already damps. Severe wage
        // betrayal, repeated benching, a major ambition/role mismatch, a
        // broken-down manager relationship, or eroded promise trust.
        let serious_concern = major_trigger
            || f.salary_satisfaction <= -8.0
            || playing_time_factor <= -8.0
            || f.ambition_fit <= -8.0
            || f.manager_relationship <= -8.0
            || f.promise_trust <= -6.0;

        let eligible =
            morale < Self::CRITICAL_MORALE || (morale < Self::CONCERN_MORALE && serious_concern);

        let recover = morale > Self::RECOVER_MORALE
            || (morale > Self::RECOVER_WITH_MINUTES_MORALE && playing_time_factor >= 10.0);

        UnhappyAssessment {
            eligible,
            major_trigger,
            recover,
        }
    }

    /// Lookback window (days) for a "fresh" major trigger — short enough
    /// that the bypass reflects a recent shock, not a stale one already
    /// priced into morale.
    const MAJOR_EVENT_WINDOW_DAYS: u16 = 14;

    /// A genuinely major morale trigger that justifies hardening `Unh`
    /// without waiting out the persistence window. Deliberately tight —
    /// these are the "real triggers" the design calls out, not routine
    /// settling friction. `ConflictWithTeammate` is handled separately
    /// because a single routine private row must NOT bypass persistence.
    fn has_major_event(player: &Player) -> bool {
        const MAJOR: [HappinessEventType; 5] = [
            HappinessEventType::PromiseBroken,
            HappinessEventType::SalaryShock,
            HappinessEventType::ContractTerminated,
            HappinessEventType::SquadRegistrationOmitted,
            HappinessEventType::ControversyIncident,
        ];
        if MAJOR.iter().any(|t| {
            player
                .happiness
                .has_recent_event(t, Self::MAJOR_EVENT_WINDOW_DAYS)
        }) {
            return true;
        }
        Self::has_serious_conflict(player)
    }

    /// True when a recent teammate conflict is serious enough to bypass the
    /// persistence wait. A lone routine private row (a `-2` "had words")
    /// does NOT qualify — only a serious/major-severity blow-up, a public
    /// (media) incident, a flagged dressing-room-damage follow-up, an
    /// explicitly repeated incident, or simply more than one conflict row in
    /// the window. Severity falls back to magnitude (|mag| >= 4 → Serious+)
    /// when the legacy emit site attached no structured context.
    fn has_serious_conflict(player: &Player) -> bool {
        let mut count = 0u32;
        let mut any_serious = false;
        for e in player.happiness.recent_events.iter().filter(|e| {
            e.event_type == HappinessEventType::ConflictWithTeammate
                && e.days_ago <= Self::MAJOR_EVENT_WINDOW_DAYS
        }) {
            count += 1;
            if any_serious {
                continue;
            }
            let serious_by_magnitude = e.magnitude.abs() >= 4.0;
            let serious_by_context = e.context.as_ref().is_some_and(|c| {
                matches!(
                    c.severity,
                    HappinessEventSeverity::Serious | HappinessEventSeverity::Major
                ) || c.scope == HappinessEventScope::Media
                    || c.follow_up == Some(HappinessEventFollowUp::DressingRoomDamageRisk)
                    || c.evidence
                        .contains(&HappinessEventEvidence::RepeatedIncident)
                    || c.evidence.contains(&HappinessEventEvidence::MediaIncident)
            });
            any_serious = serious_by_magnitude || serious_by_context;
        }
        // Repeated conflict (≥2 rows in the window) is serious on its own.
        count >= 2 || any_serious
    }
}

impl Player {
    /// Build the post-transfer playing-time opportunity snapshot for this
    /// player. The `since_join` counters live on `happiness` and reset on
    /// every club change. For a long-settled player whose counters are
    /// cold (they are not persisted across save reloads), fall back to
    /// lifetime competitive stats so an established regular is never
    /// wrongly reported as having "zero opportunities".
    pub fn playing_time_opportunity(&self, now: NaiveDate) -> PlayingTimeOpportunityContext {
        let h = &self.happiness;
        let days_since_join = self.days_since_transfer(now).unwrap_or(i64::MAX);
        // Settled = never transferred, or transferred long ago. Inside the
        // post-transfer window the counters are authoritative (a genuine
        // zero means the club really hasn't played).
        let settled = self
            .days_since_transfer(now)
            .map(|d| d > 60)
            .unwrap_or(true);
        let use_lifetime_fallback = settled && h.eligible_official_matches_since_join == 0;

        let (eligible, starts, subs, unused, left_out) = if use_lifetime_fallback {
            let starts = self.statistics.played;
            let subs = self.statistics.played_subs;
            (starts.saturating_add(subs), starts, subs, 0, 0)
        } else {
            (
                h.eligible_official_matches_since_join,
                h.starts_since_join,
                h.sub_apps_since_join,
                h.unused_bench_since_join,
                h.left_out_since_join,
            )
        };

        PlayingTimeOpportunityContext {
            days_since_join,
            official_team_matches_since_join: eligible,
            eligible_official_matches_since_join: eligible,
            player_starts_since_join: starts,
            player_sub_apps_since_join: subs,
            player_unused_bench_since_join: unused,
            player_left_out_since_join: left_out,
            was_registered_and_fit: !self.player_attributes.is_injured,
            is_loan: self.contract_loan.is_some(),
        }
    }

    /// Weekly happiness evaluation. Computes the seven legacy factors
    /// plus six derived "life in the team" factors (role clarity,
    /// coach credibility, dressing-room status, club fit, pressure
    /// load, promise trust). Takes `ClubMoraleContext` so the derived
    /// axes can read coach scores and facility quality.
    pub(crate) fn process_happiness_full(
        &mut self,
        result: &mut PlayerResult,
        now: NaiveDate,
        team_reputation: f32,
        season_state: TeamSeasonState,
        club_ctx: ClubMoraleContext,
    ) {
        let age = DateUtils::age(self.birth_date, now);
        let age_sensitivity = if age >= 24 && age <= 30 { 1.3 } else { 1.0 };

        // Decay old events weekly
        self.happiness.decay_events();

        // Unresolved transfer-interest speculation drag — counts visible
        // interest events landed in the past ~6 weeks and lets the
        // owner-side method decide whether to add a distraction event.
        let recent_interest_count = self.count_recent_transfer_interest_events(45);
        self.on_unresolved_speculation_pressure(recent_interest_count);

        // 1. Playing time vs squad status
        let playing_time_factor = self.calculate_playing_time_factor(age_sensitivity, now);
        self.happiness.factors.playing_time = playing_time_factor;

        // 2. Salary vs ability
        let mut salary_factor =
            self.calculate_salary_factor(age, team_reputation, season_state.league_reputation);

        // After 2 years of unresolved salary unhappiness, player accepts situation
        // and salary frustration dampens — prevents permanent unhappiness loops.
        // Must be applied BEFORE recalculate_morale() so dampening actually affects morale.
        let gave_up_on_salary = salary_factor <= -5.0
            && self
                .happiness
                .last_salary_negotiation
                .map(|d| (now - d).num_days() > 730)
                .unwrap_or(false);

        if gave_up_on_salary {
            salary_factor = (salary_factor * 0.5).clamp(-5.0, 0.0);
        }

        self.happiness.factors.salary_satisfaction = salary_factor;

        // 3. Manager relationship
        let manager_factor = self.calculate_manager_relationship_factor();
        self.happiness.factors.manager_relationship = manager_factor;

        // 4. Injury frustration
        let injury_factor = self.calculate_injury_frustration();
        self.happiness.factors.injury_frustration = injury_factor;

        // 5. Ambition vs club level (structural) plus season trajectory
        // (dynamic). A high-ambition player at a big club fighting
        // relegation is unhappy even though the prestige fits.
        let ambition_factor = self.calculate_ambition_fit(team_reputation, &season_state, now);
        self.happiness.factors.ambition_fit = ambition_factor;

        // 6. Praise/discipline from recent events (tracked separately)
        let praise: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::ManagerPraise)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 60.0).max(0.0))
            .sum();
        self.happiness.factors.recent_praise = praise.clamp(0.0, 10.0);

        let discipline: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::ManagerDiscipline)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 60.0).max(0.0))
            .sum();

        self.happiness.factors.recent_discipline = discipline.clamp(-10.0, 0.0);

        // Loan-specific weekly modulation — extends the loan audit with
        // per-player morale signals (out-of-position, too-good, young
        // enjoying responsibility, veteran humiliation).
        if self.contract_loan.is_some() {
            self.process_loan_morale(now, team_reputation, season_state.league_reputation);
        }

        // ── Derived factors ───────────────────────────────────
        // The five soft "settling-in" axes pass through the transfer
        // honeymoon ramp — their negative contribution is scaled down for
        // the first few weeks at a new club so a fresh signing isn't tipped
        // into a formal grievance by noise he hasn't had a chance to settle.
        // promise_trust is a hard, real signal and is left undamped.
        let honeymoon = TransferHoneymoon::since(self.days_since_transfer(now));
        self.happiness.factors.role_clarity =
            honeymoon.damp_negative(self.calculate_role_clarity());
        self.happiness.factors.coach_credibility =
            honeymoon.damp_negative(self.calculate_coach_credibility(&club_ctx));
        self.happiness.factors.dressing_room_status =
            honeymoon.damp_negative(self.calculate_dressing_room_status());
        self.happiness.factors.club_fit = honeymoon.damp_negative(self.calculate_club_fit(
            team_reputation,
            season_state.league_reputation,
            &club_ctx,
        ));
        self.happiness.factors.pressure_load = honeymoon.damp_negative(
            self.calculate_pressure_load(team_reputation, season_state.league_reputation, now),
        );
        self.happiness.factors.promise_trust = self.calculate_promise_trust(now);

        // Recalculate overall morale (now uses dampened salary factor + derived axes)
        self.happiness.recalculate_morale();

        // Salary unhappy: player wants contract renegotiation (with 1-year cooldown)
        if salary_factor <= -5.0 && !gave_up_on_salary {
            let cooldown_passed = self
                .happiness
                .last_salary_negotiation
                .map(|d| (now - d).num_days() >= 365)
                .unwrap_or(true);

            if cooldown_passed {
                result.contract.want_improve_contract = true;
                if self.happiness.last_salary_negotiation.is_none() {
                    self.happiness.last_salary_negotiation = Some(now);
                }
            }
        } else if salary_factor > -5.0 && !gave_up_on_salary {
            // Salary is acceptable now — reset negotiation tracking
            self.happiness.last_salary_negotiation = None;
        }
        // If gave_up_on_salary: keep last_salary_negotiation but don't request improvements

        // Manager-relationship arc aggregator + private-talk request.
        // Run after factors but before the unhappy status flag so a
        // brand-new ManagerTrustEroding row can also drag the unhappy
        // verdict in the same tick. Each has its own cooldown so the
        // weekly tick produces at most one of these rows per player.
        self.maybe_emit_manager_trust_arc();
        self.maybe_emit_asked_for_private_talk();

        // Unhappy-status verdict with hysteresis. The formal `Unh` status
        // is no longer a bare `morale < 35` flag: a fresh signing's
        // first-week settling dip must either be backed by a *serious*
        // concern AND persist across two weekly ticks, or be driven by a
        // genuinely major trigger (broken promise, severe wage betrayal,
        // public conflict, repeated official-match omission). Recovery
        // hysteresis is unchanged — the status clears only once morale
        // climbs clear of the unhappy band (or above 40 with real minutes).
        let assessment = UnhappyAssessment::evaluate(self, playing_time_factor);
        let already_unhappy = self.statuses.get().contains(&PlayerStatusType::Unh);

        // `formal_unhappy` is the single source of truth for the hard `Unh`
        // status after this tick. `result.unhappy` mirrors it exactly;
        // `result.morale_concern` is the softer "low mood, not yet a
        // grievance" signal so consumers can tell the two apart.
        let formal_unhappy = if assessment.eligible {
            self.happiness.unhappy_streak = self.happiness.unhappy_streak.saturating_add(1);
            let persisted = self.happiness.unhappy_streak >= UnhappyAssessment::PERSISTENCE_TICKS;
            if already_unhappy || assessment.major_trigger || persisted {
                if !already_unhappy {
                    self.statuses.add(now, PlayerStatusType::Unh);
                }
                true
            } else {
                // Concern building but not yet a formal grievance — let the
                // mood ride for another tick before it hardens.
                false
            }
        } else {
            self.happiness.unhappy_streak = 0;
            if assessment.recover {
                self.statuses.remove(PlayerStatusType::Unh);
                false
            } else {
                // Middle band: don't harden, and don't auto-clear an existing
                // Unh (recovery still needs morale above 50). The status, and
                // therefore `result.unhappy`, is whatever it already was.
                already_unhappy
            }
        };

        result.unhappy = formal_unhappy;
        result.morale_concern = !formal_unhappy && !self.happiness.is_happy();
    }

    /// Playing-time morale factor, built on the match-opportunity model
    /// rather than calendar time or the raw start/appearance ratio. The
    /// denominator is the eligible official matches the club has actually
    /// played since the player joined — so a player at a club that hasn't
    /// kicked a ball can't be frustrated about minutes he was never
    /// denied, and a player who is repeatedly overlooked accrues a real
    /// deficit even though the few games he *did* play were all starts.
    fn calculate_playing_time_factor(&self, age_sensitivity: f32, now: NaiveDate) -> f32 {
        let cfg = PlayingTimeFrustrationConfig::default();
        let opp = self.playing_time_opportunity(now);

        // ── Zero-match hard block — never overridden ──
        if opp.eligible_official_matches_since_join == 0 {
            return 0.0;
        }
        // Sample-size guard — don't judge on a handful of fixtures.
        if opp.eligible_official_matches_since_join < cfg.min_player_apps_sample {
            return 0.0;
        }

        // Only skilled players care strongly about playing time; sub-40 CA
        // bench warmers accept their role without fretting.
        let ability = self.player_attributes.current_ability as f32;
        if ability < 40.0 {
            return 0.0;
        }
        let ability_factor = ((ability - 40.0) / 80.0).clamp(0.0, 1.0);

        let status = self.contract.as_ref().map(|c| &c.squad_status);
        let expected_share = PlayingTimeFrustrationConfig::expected_start_share(status);
        let eligible = opp.eligible_official_matches_since_join as f32;
        let expected_raw = eligible * expected_share;
        let expected = expected_raw.max(1.0);
        let actual = opp.actual_involvement_score(&cfg);

        if actual >= expected_raw {
            // Meeting / exceeding expectations — positive contribution
            // scaled across the headroom above expectation (so a full
            // starter still earns the top of the band, matching the
            // historical calibration).
            let headroom = (eligible - expected_raw).max(1.0);
            let surplus = ((actual - expected_raw) / headroom).clamp(0.0, 1.0);
            (surplus * cfg.max_positive_playing_time_factor * ability_factor)
                .clamp(0.0, cfg.max_positive_playing_time_factor)
        } else {
            // Below expectation — frustration scaled by ability, age
            // sensitivity, and the post-transfer grace ramp.
            let deficit_ratio = ((expected_raw - actual) / expected).clamp(0.0, 1.0);
            let frustration_multiplier = opp.frustration_multiplier(&cfg);
            (cfg.max_negative_playing_time_factor
                * deficit_ratio
                * ability_factor
                * age_sensitivity
                * frustration_multiplier)
                .clamp(cfg.max_negative_playing_time_factor, 0.0)
        }
    }

    /// Salary factor uses the same `ContractValuation` as the renewal AI
    /// and personal-terms negotiation. Otherwise the three systems disagree
    /// on what a fair wage looks like — happiness might shout "underpaid"
    /// while the renewal AI is happily renewing on the same terms.
    ///
    /// Inputs the valuation already accounts for: ability, age, position,
    /// reputation, league prestige, club tier, status premium. The factor
    /// here is the gap between actual salary and expected; bonuses and
    /// recent renewals dampen frustration.
    fn calculate_salary_factor(
        &self,
        age: u8,
        team_reputation: f32,
        league_reputation: u16,
    ) -> f32 {
        let Some(ref contract) = self.contract else {
            return -5.0;
        };

        // Players on loan accept their temporary salary — no frustration
        if self.contract_loan.is_some() {
            return 0.0;
        }

        // Youth/amateur players don't evaluate salary competitively
        match contract.contract_type {
            ContractType::Youth | ContractType::Amateur | ContractType::NonContract => return 0.0,
            _ => {}
        }

        // Pass the real club + league reputation so an elite Premier League
        // player's expectation isn't computed against a generic "mid-tier"
        // baseline. Falls back to the neutral 0.5 / 5000 only when the
        // caller couldn't provide context (zero values).
        let club_rep = if team_reputation > 0.0 {
            team_reputation.clamp(0.0, 1.0)
        } else {
            0.5
        };
        let league_rep = if league_reputation > 0 {
            league_reputation
        } else {
            5_000
        };
        let ctx = ValuationContext {
            age,
            club_reputation_score: club_rep,
            league_reputation: league_rep,
            squad_status: contract.squad_status.clone(),
            current_salary: contract.salary,
            // months_remaining doesn't affect expected_wage (only the
            // leverage band), so a constant keeps the factor stable.
            months_remaining: 24,
            has_market_interest: false,
        };

        let valuation = ContractValuation::evaluate(self, &ctx);
        let expected = valuation.expected_wage as f32;
        if expected < 1.0 {
            return 0.0;
        }

        // Use the shared package-value helper so happiness, acceptance,
        // and renewal scoring agree on what the package is "really
        // worth" annually. The helper amortises the signing bonus
        // (zeroing it out once paid via `signing_bonus_paid`),
        // probability-weights promotion / avoid-relegation bonuses, and
        // values per-event bonuses by realistic season frequencies.
        let inputs = package_inputs_from_contract(contract, self);
        let effective_salary = expected_annual_value(&inputs) as f32;

        let ratio = effective_salary / expected;
        let mut factor = if ratio >= 1.20 {
            (5.0 + (ratio - 1.20) * 8.0).min(12.0)
        } else if ratio >= 1.00 {
            (ratio - 1.0) * 25.0
        } else if ratio >= 0.80 {
            (ratio - 1.0) * 30.0
        } else if ratio >= 0.60 {
            -6.0 + (ratio - 0.80) * 30.0
        } else {
            -12.0 + (ratio - 0.60) * 15.0
        };

        // Loyalty veterans tolerate slightly below market — agent isn't
        // pushing them to chase every dollar.
        if self.attributes.loyalty >= 16.0 && ratio >= 0.85 {
            factor = (factor + 2.0).min(10.0);
        }

        // Just signed → don't resent the wage you just negotiated. The
        // renewal handler stamps `last_salary_negotiation` on accept; if
        // it's set and the factor is negative, soften the blow.
        if self.happiness.last_salary_negotiation.is_some() && factor < 0.0 {
            factor *= 0.85;
        }

        factor.clamp(-15.0, 15.0)
    }

    fn calculate_manager_relationship_factor(&mut self) -> f32 {
        // Manager-relationship factor is now a **derived** weekly summary,
        // not a free-floating accumulator. Three sources feed it:
        //
        //   * Staff relation level — long-term trust/respect with the
        //     coaching staff. The strongest negative or strongest positive
        //     dominates so a single broken-down relationship with the head
        //     coach isn't drowned out by neutral assistants.
        //   * Coach rapport — short-term delivery / training-talk rapport,
        //     stored on `PlayerRapport`. Maps the strongest existing rapport
        //     score onto a smaller −5..+5 contribution.
        //   * Recent promise & praise/discipline events — kept promises and
        //     manager praise lift the factor; broken promises and discipline
        //     drag it down.
        //
        // Each weekly evaluation overwrites the stored factor so a single
        // good (or bad) talk doesn't anchor morale forever — the underlying
        // staff relation has to actually be there for the factor to persist.
        // Old in-place writes from team_talks / behaviour result still feed
        // the staff relation + rapport stores, so their effect now decays
        // naturally with the rest of the social graph.

        // Strongest staff relation level — pick the signed maximum-magnitude
        // entry rather than averaging, so a single really bad coach
        // relationship registers.
        let staff_level: f32 = {
            let mut strongest = 0.0f32;
            for (_id, rel) in self.relations.staff_relations_iter() {
                if rel.level.abs() > strongest.abs() {
                    strongest = rel.level;
                }
            }
            // Map [-100, 100] → [-8, +8].
            (strongest / 100.0 * 8.0).clamp(-8.0, 8.0)
        };

        // Strongest rapport entry — same magnitude logic.
        let rapport_score: f32 = {
            let mut strongest: i16 = 0;
            for entry in self.rapport.coaches.iter() {
                if entry.score.abs() > strongest.abs() {
                    strongest = entry.score;
                }
            }
            // Map roughly [-50, 100] → [-5, +5]. Asymmetric because rapport
            // floor is -50 not -100.
            let normalised = if strongest >= 0 {
                (strongest as f32 / 100.0) * 5.0
            } else {
                (strongest as f32 / 50.0) * 5.0
            };
            normalised.clamp(-5.0, 5.0)
        };

        // Recent promise outcomes — kept lifts, broken hits hard.
        // Limit window to 60 days so the contribution decays naturally.
        let promise_kept: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::PromiseKept && e.days_ago <= 60)
            .count() as f32;
        let promise_broken: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::PromiseBroken && e.days_ago <= 60)
            .count() as f32;
        // Cap so a flurry of promise events doesn't dominate.
        let promise_contribution = (promise_kept * 4.0 - promise_broken * 8.0).clamp(-12.0, 8.0);

        // Recent praise / discipline — softer than promises, since the
        // dedicated factors `recent_praise` / `recent_discipline` already
        // count those events. We include them here too to anchor the
        // relationship summary on actual interactions.
        let praise_count: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::ManagerPraise && e.days_ago <= 30)
            .count() as f32;
        let discipline_count: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::ManagerDiscipline && e.days_ago <= 30)
            .count() as f32;
        let praise_contribution = (praise_count * 2.0 - discipline_count * 3.0).clamp(-6.0, 5.0);

        let derived = (staff_level + rapport_score + promise_contribution + praise_contribution)
            .clamp(-15.0, 15.0);

        // Persist the snapshot so external consumers (UI, debug tools) see
        // the same number this week.
        self.happiness.factors.manager_relationship = derived;
        derived
    }

    fn calculate_injury_frustration(&self) -> f32 {
        if !self.player_attributes.is_injured {
            return 0.0;
        }

        let injury_days = self.player_attributes.injury_days_remaining as f32;
        if injury_days <= 14.0 {
            return -2.0;
        }

        // Longer injuries cause more frustration: -5 to -10
        let severity = ((injury_days - 14.0) / 60.0).min(1.0);
        -(5.0 + severity * 5.0)
    }

    fn calculate_ambition_fit(
        &self,
        team_reputation: f32,
        season: &TeamSeasonState,
        now: NaiveDate,
    ) -> f32 {
        let ambition = self.attributes.ambition;
        if ambition <= 10.0 {
            return 0.0;
        }

        let status_dampening =
            ambition_status_dampening(self.contract.as_ref().map(|c| &c.squad_status));
        let prestige = self.prestige_fit_component(ambition, team_reputation, status_dampening);
        let trajectory = self.season_trajectory_component(ambition, season, status_dampening);
        // Itchy feet: many years at one club erode ambition satisfaction even
        // when the club's size still fits — "I've done it all here".
        let restlessness = self.tenure_restlessness_component(ambition, now);

        (prestige + trajectory - restlessness).clamp(-15.0, 12.0)
    }

    /// "Itchy feet": an ambitious player who has spent many years at ONE club
    /// grows restless for a new challenge — even a contented one-club man
    /// eventually wants to test himself elsewhere. Returns a NON-negative
    /// magnitude the caller SUBTRACTS from ambition_fit, so a long stay dents
    /// the happy value and, once it bites, feeds the same Unh /
    /// `AmbitionMismatch` path a too-small club does. Scales with ambition
    /// and tenure beyond a settled threshold, suppressed by loyalty (a true
    /// club legend stays). Bounded so tenure alone never dominates the
    /// structural club-size signal.
    fn tenure_restlessness_component(&self, ambition: f32, now: NaiveDate) -> f32 {
        // Years at the club. A one-club man (never transferred) has been here
        // his whole career — approximate from age since a typical debut.
        let age = DateUtils::age(self.birth_date, now);
        let years_at_club = self
            .last_transfer_date
            .map(|d| ((now - d).num_days() as f32 / 365.0).max(0.0))
            .unwrap_or_else(|| (age as f32 - 17.0).max(0.0));

        // A stable, settled spell is healthy — nothing stirs until the stay
        // is genuinely long, then restlessness builds with each extra year.
        const SETTLED_YEARS: f32 = 6.0;
        if years_at_club <= SETTLED_YEARS {
            return 0.0;
        }
        let over = years_at_club - SETTLED_YEARS;

        // Ambition drives the appetite for a new challenge; loyalty holds him.
        let ambition_drive = ((ambition - 10.0) / 10.0).clamp(0.0, 1.0); // 0 at 10 → 1 at 20
        let loyalty_hold = (self.attributes.loyalty / 20.0).clamp(0.0, 1.0); // 1 at max loyalty

        // ~1 per year over the threshold at full ambition, halved by full
        // loyalty, capped so tenure never exceeds the club-size component.
        (over * ambition_drive * (1.0 - 0.5 * loyalty_hold)).min(5.0)
    }

    /// Classic "I joined a club befitting my stature" piece — compares
    /// the player's ambition against the club's all-time reputation.
    fn prestige_fit_component(
        &self,
        ambition: f32,
        team_reputation: f32,
        status_dampening: f32,
    ) -> f32 {
        let club_rep = team_reputation * 10000.0;
        let expected_rep = (ambition - 10.0) * 800.0;

        let raw = if club_rep >= expected_rep {
            let excess = ((club_rep - expected_rep) / 2000.0).min(1.0);
            excess * 5.0
        } else {
            let deficit = ((expected_rep - club_rep) / expected_rep.max(1.0)).min(1.0);
            -deficit * 10.0 * status_dampening
        };

        raw.clamp(-10.0, 5.0)
    }

    /// "Where is this team actually going this season?" — league position
    /// relative to where a player of this ambition expects to finish,
    /// weighted by how far into the season we are.
    ///
    /// Drives the relegation / mid-table-slump exodus: a Key Player at a
    /// Premier League club sitting 18th with 30 matches played piles up
    /// enough negative magnitude to tip into Unh → Req.
    ///
    /// League reputation contextualises expectations: "top of a Tier-4
    /// league" doesn't satisfy a world-class ambition. An ambitious
    /// player at a minnow over-performing in the bottom division
    /// doesn't feel ambition is satisfied, just mildly less frustrated.
    fn season_trajectory_component(
        &self,
        ambition: f32,
        s: &TeamSeasonState,
        status_dampening: f32,
    ) -> f32 {
        if s.league_position == 0 || s.league_size < 4 {
            return 0.0;
        }

        // 0.0 = top, 1.0 = bottom
        let pos_pct = (s.league_position as f32 - 1.0) / (s.league_size as f32 - 1.0).max(1.0);

        // Ambition 20 expects top (~5%), ambition 15 expects top-third
        // (~33%), ambition 10 accepts mid-table (~70%).
        let expected_pct = ((20.0 - ambition) / 14.0).clamp(0.05, 0.7);

        let gap = pos_pct - expected_pct;

        // Early season is noisy — a 10-game blip isn't fate. Weight scales
        // from 0.25 at season start to 1.0 by the two-thirds mark.
        let weight = (s.season_progress * 1.5).clamp(0.25, 1.0);

        // Prestige ambition (20) scoring anywhere outside the top of a
        // top-tier league is disappointing. For a tier-4 league a high
        // ambition player already feels out of place — league reputation
        // shrinks the positive side of the factor here.
        let league_rep_factor = (s.league_reputation as f32 / 8000.0).clamp(0.2, 1.2);

        let raw = if gap <= 0.0 {
            // Better than expected — positive, but scaled by league rep
            // so "top of non-league" feels flatter than "top of Serie A".
            let excess = (-gap).min(0.5) / 0.5;
            excess * 4.0 * league_rep_factor
        } else {
            // Worse than expected — dampened by squad status.
            // Relegation zone (bottom 15%) gets an extra penalty.
            let mut deficit_mag = gap * 18.0;
            if pos_pct >= 0.85 {
                deficit_mag += 3.0;
            }
            -deficit_mag * status_dampening
        };

        (raw * weight).clamp(-10.0, 5.0)
    }

    // ── Six derived morale factors ───────────────────────────
    //
    // Each factor returns a signed value in roughly the band stated on
    // its `HappinessFactors` field doc. They're recomputed every weekly
    // tick and rolled into morale at 0.6× weight inside
    // `recalculate_morale`.

    /// role_clarity: does the player understand his role?
    fn calculate_role_clarity(&self) -> f32 {
        let mut score: f32 = 0.0;

        // Recent RoleMismatch events drag clarity down hard.
        let mismatch_pull: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::RoleMismatch)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 90.0).max(0.0))
            .sum();
        score += mismatch_pull * 0.5;

        // Squad status alignment with appearances. KeyPlayer with a
        // healthy starter ratio knows where he stands; KeyPlayer barely
        // playing has zero clarity even though the badge says regular.
        if let Some(c) = self.contract.as_ref() {
            let starter = self.happiness.starter_ratio;
            let alignment = match c.squad_status {
                PlayerSquadStatus::KeyPlayer => starter - 0.65,
                PlayerSquadStatus::FirstTeamRegular => starter - 0.50,
                PlayerSquadStatus::FirstTeamSquadRotation => starter - 0.30,
                PlayerSquadStatus::MainBackupPlayer => 0.20 - (starter - 0.20).abs(),
                _ => 0.0,
            };
            score += alignment * 6.0;
        }

        // Repeated tactical-role talks in the log signal the player
        // has been chasing clarity. One ask is normal; three+ in 90
        // days means he's not getting it.
        let tactical_asks = self
            .interactions
            .entries
            .iter()
            .filter(|e| matches!(e.topic, InteractionTopic::TacticalRole))
            .count() as f32;
        if tactical_asks >= 3.0 {
            score -= (tactical_asks - 2.0) * 1.5;
        }

        score.clamp(-8.0, 5.0)
    }

    /// coach_credibility: do the coaches feel competent enough to coach
    /// this player? Compares the best-coach-on-staff scores against
    /// the player's CA. Goalkeepers weight the goalkeeping coach;
    /// outfield players weight technical+mental.
    fn calculate_coach_credibility(&self, ctx: &ClubMoraleContext) -> f32 {
        let player_ca = self.player_attributes.current_ability as f32;
        if player_ca < 60.0 {
            // Sub-60 CA players don't outgrow their coaches.
            return 0.0;
        }

        let is_gk = matches!(self.position(), PlayerPositionType::Goalkeeper);

        // Source guard: the caller builds `ClubMoraleContext` via
        // `unwrap_or_default()` when the player has no resolved club context
        // this tick, leaving every coach score at 0. Zero is "unknown", not
        // "amateur hour" — without it we'd punish an elite player up to -8
        // for coaching we never actually measured. Treat an all-zero
        // discipline as unknown and stay neutral; only judge once there's
        // real coach data behind the club context.
        let coach_data_known = if is_gk {
            ctx.coach_best_goalkeeping > 0
        } else {
            ctx.coach_best_technical > 0 || ctx.coach_best_mental > 0
        };
        if !coach_data_known {
            return 0.0;
        }

        let coach_score = if is_gk {
            ctx.coach_best_goalkeeping as f32
        } else {
            (ctx.coach_best_technical as f32 + ctx.coach_best_mental as f32) / 2.0
        };

        // Player expects coach quality scaled with own ability:
        // CA 100 → expect coach ≥ 10
        // CA 150 → expect coach ≥ 15
        // CA 180+ → expect coach ≥ 18 (top ten in the world)
        let expected_coach = (player_ca / 10.0).clamp(6.0, 19.0);
        let gap = coach_score - expected_coach;

        // Above expectations: respect, capped. Below: contempt scaling
        // by how big a star the player is. World-class player at a
        // small club coached by amateurs feels this most.
        if gap >= 0.0 {
            (gap * 0.8).clamp(0.0, 6.0)
        } else {
            let star_factor = (player_ca / 160.0).clamp(0.6, 1.8);
            (gap * 1.4 * star_factor).clamp(-8.0, 0.0)
        }
    }

    /// dressing_room_status: where does the player sit in the squad
    /// pecking order? Built from leadership skill, world reputation,
    /// and the player's own social-graph signals — bonding events lift,
    /// conflict events drag.
    fn calculate_dressing_room_status(&self) -> f32 {
        let leadership = self.skills.mental.leadership;
        let reputation = self.player_attributes.current_reputation as f32;

        // Base — leadership 0..20 → -2..+4, reputation lifts top end.
        let mut score: f32 = ((leadership - 10.0) * 0.3).clamp(-3.0, 4.0);
        score += (reputation / 10000.0).clamp(0.0, 1.0) * 3.0;

        // Recent bonding lifts standing; conflicts drag.
        let bonding: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::TeammateBonding)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 60.0).max(0.0))
            .sum();
        let conflict: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::ConflictWithTeammate)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 60.0).max(0.0))
            .sum();
        score += (bonding * 0.4).clamp(0.0, 3.0);
        score += (conflict * 0.6).clamp(-4.0, 0.0);

        // Isolation events knock standing further.
        let isolated = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::FeelingIsolated && e.days_ago <= 60)
            .count() as f32;
        score -= isolated * 0.5;

        score.clamp(-6.0, 8.0)
    }

    /// club_fit: cultural / structural fit with the club. Reads
    /// language fluency, league level, and facilities against the
    /// player's ambition. Distinct from ambition_fit (which is purely
    /// reputation-vs-expectation) — club_fit captures the *day-to-day*
    /// experience of being at this club.
    fn calculate_club_fit(
        &self,
        team_reputation: f32,
        league_reputation: u16,
        ctx: &ClubMoraleContext,
    ) -> f32 {
        let mut score: f32 = 0.0;

        // Facility fit — ambitious pros expect modern training. Below
        // 0.4 facility quality and ambition ≥ 14 → noticeable hit.
        let ambition = self.attributes.ambition;
        let avg_facility = (ctx.training_facility_quality + ctx.youth_facility_quality) / 2.0;
        if avg_facility > 0.0 {
            let facility_gap = avg_facility - (ambition / 30.0);
            score += facility_gap * 6.0;
        }

        // League prestige fit — pros at a tier-4 league with high
        // ambition feel out of place even if the club is doing well.
        let league_norm = (league_reputation as f32 / 8000.0).clamp(0.2, 1.2);
        score += (league_norm - 0.5) * (ambition - 10.0) * 0.3;

        // Compatriot count + language progress events — cultural roots.
        let compatriots = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::CompatriotJoined && e.days_ago <= 180)
            .count() as f32;
        score += compatriots.min(2.0) * 0.8;

        let lang_progress = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::LanguageProgress && e.days_ago <= 180)
            .count() as f32;
        score += lang_progress.min(3.0) * 0.5;

        // (Favorite-club bonus is applied at signing time via the
        // DreamMove pathway — duplicating it weekly would double-count.)

        // Use team reputation only as a tiny tie-breaker — ambition_fit
        // already covers the "wrong size club" axis.
        score += (team_reputation - 0.5) * 1.5;

        score.clamp(-8.0, 6.0)
    }

    /// pressure_load: how heavy is the fan/media/board expectation
    /// relative to the player's pressure tolerance? High-rep players at
    /// big clubs always carry pressure; low-pressure personalities
    /// crack first. Outlier-above players (e.g. Messi at a small club)
    /// get an extra spotlight multiplier — the press follows them
    /// regardless of where the badge sits.
    fn calculate_pressure_load(
        &self,
        team_reputation: f32,
        league_reputation: u16,
        now: NaiveDate,
    ) -> f32 {
        let rep_gap = ReputationGap::compute(self, team_reputation, league_reputation);
        let pressure = self.attributes.pressure;
        let player_rep = self.player_attributes.current_reputation as f32;
        let club_rep_score = team_reputation.clamp(0.0, 1.0) * 100.0;
        let league_score = (league_reputation as f32 / 100.0).clamp(0.0, 100.0);

        // Pressure index — bigger club, bigger league, higher player
        // rep → more eyes. Player_rep doubled because the public talks
        // about the player, not just the badge.
        let pressure_index =
            (club_rep_score * 0.4 + league_score * 0.3 + (player_rep / 100.0) * 0.6)
                .clamp(0.0, 100.0);

        // Tolerance: pressure attribute 0..20 → 0..100.
        let tolerance = pressure * 5.0;
        let index_gap = pressure_index - tolerance;

        // High rep player having a poor recent stretch under spotlight
        // (low form rating) — extra hit. Regressed value so a one-bad-
        // match dip doesn't trigger morale fallout for a player with
        // a long body of solid form.
        let pos = self.position().position_group();
        let form = self.statistics.average_rating_realistic(pos);
        let form_penalty = if form > 0.0 && form < 6.0 && pressure_index > 50.0 {
            -1.5
        } else {
            0.0
        };

        // Outlier-above amplifier: Messi at a small club draws every
        // camera regardless of league or club rep. Adds a floor of
        // pressure that scales with how far the rep gap is.
        let outlier_pull: f32 = if rep_gap.is_outlier_above() {
            (rep_gap.player_vs_club.max(rep_gap.player_vs_league) as f32 / 1000.0).clamp(0.0, 8.0)
        } else {
            0.0
        };

        let raw = if index_gap <= 0.0 {
            (-index_gap * 0.05).clamp(0.0, 3.0) - outlier_pull * 0.5
        } else {
            (-(index_gap / 12.0) + form_penalty - outlier_pull * 0.5).clamp(-8.0, 0.0)
        };

        // Early-window framing: in the first month at a new club the
        // spotlight a high-profile signing draws is *attention/pressure*,
        // not a settled morale collapse. Floor the negative at a mild level
        // unless the player is genuinely struggling — poor recent form or
        // actual public criticism — which is real on-pitch evidence that the
        // pressure has started to bite. The outlier-above spotlight (Messi
        // at a small club) still shows through as a visible concern.
        let in_early_window = self
            .days_since_transfer(now)
            .map(|d| d < 30)
            .unwrap_or(false);
        let public_criticism = self
            .happiness
            .has_recent_event(&HappinessEventType::MediaCriticism, 30)
            || self
                .happiness
                .has_recent_event(&HappinessEventType::FanCriticism, 30);
        let poor_form = form > 0.0 && form < 6.0;
        let raw = if in_early_window && !poor_form && !public_criticism {
            raw.max(-3.0)
        } else {
            raw
        };

        raw.clamp(-8.0, 3.0)
    }

    /// promise_trust: the player's belief in the manager's word. Built
    /// from kept-vs-broken promise events, recent broken-promise
    /// frequency, and current credibility of the manager-relationship.
    fn calculate_promise_trust(&self, _now: NaiveDate) -> f32 {
        let kept: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::PromiseKept && e.days_ago <= 180)
            .count() as f32;
        let broken: f32 = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::PromiseBroken && e.days_ago <= 180)
            .count() as f32;

        // Kept builds slowly; broken cuts trust hard. Asymmetry
        // mirrors the rapport "easy to lose" model.
        let mut score = kept * 1.5 - broken * 3.5;

        // Pending promises with high importance and low credibility
        // erode trust just by sitting there — the player notices the
        // manager is overpromising.
        let overhang: f32 = self
            .promises
            .iter()
            .filter(|p| p.credibility_at_creation < 40 && p.importance_to_player >= 60)
            .count() as f32;
        score -= overhang * 1.5;

        score.clamp(-10.0, 6.0)
    }

    /// Weekly loan-life modulation — only called when the player is on
    /// loan. Emits the "too good for this level" frustration for elite
    /// veterans, the "first taste of responsibility" lift for young
    /// loanees, an "out of position" hit when role mismatch lingers,
    /// and an underperformance signal when the player's form is poor
    /// at a smaller club (the loan isn't working out).
    fn process_loan_morale(
        &mut self,
        now: NaiveDate,
        team_reputation: f32,
        league_reputation: u16,
    ) {
        let gap = ReputationGap::compute(self, team_reputation, league_reputation);
        // Match-opportunity gate: a loanee at a club that hasn't given him
        // a competitive fixture yet has no playing-time grievance to voice,
        // however long he's been on the books.
        let has_match_opportunity = self
            .playing_time_opportunity(now)
            .eligible_official_matches_since_join
            > 0;
        let age = DateUtils::age(
            self.birth_date,
            self.last_transfer_date.unwrap_or_else(|| {
                self.contract_loan
                    .as_ref()
                    .and_then(|c| c.started)
                    .unwrap_or(self.birth_date)
            }),
        );
        let world_rep = self.player_attributes.world_reputation;

        // Veteran humiliation — elite vet (32+, world_rep ≥ 6500)
        // sitting in a clearly smaller league. Gentle ongoing drag.
        if age >= 32 && world_rep >= 6500 && gap.is_outlier_above() {
            self.happiness
                .add_event_with_cooldown(HappinessEventType::AmbitionShock, -3.0, 30);
        }

        // Young loanee enjoying responsibility — under-23, in a
        // smaller club / lower league than parent, getting starts.
        // Trigger only if the player has ≥ 5 starts since arriving.
        let starts = self.statistics.played;
        if age <= 22
            && world_rep < 5000
            && starts >= 5
            && (gap.player_vs_club <= 0 || gap.player_vs_league <= 0)
        {
            self.happiness
                .add_event_with_cooldown(HappinessEventType::SettledIntoSquad, 2.5, 60);
        }

        // Used out of position — RoleMismatch event still active and the
        // player hasn't been moved back. This is a *role* grievance, and
        // it only becomes a *playing-time* one once the club has actually
        // played official matches the loanee was overlooked for. Without
        // any eligible fixtures the RoleMismatch event already on the log
        // stands on its own — we must not escalate it to LackOfPlayingTime
        // and trip the zero-match invariant.
        let recent_mismatch = self
            .happiness
            .recent_events
            .iter()
            .any(|e| e.event_type == HappinessEventType::RoleMismatch && e.days_ago <= 28);
        if recent_mismatch && has_match_opportunity {
            self.happiness
                .add_event_with_cooldown(HappinessEventType::LackOfPlayingTime, -2.0, 21);
        }

        // Loan underperformance — apps but rating sits clearly below the
        // positional neutral means the loan isn't yielding the kind of
        // minutes the parent club hoped for. Surface this as a *loan*
        // event (parent club concerned) rather than a fake training
        // report — it's about competitive form, not attitude on the
        // training ground. Using the regressed value, the trigger
        // threshold rises to 6.2 (matches the neutral-minus-0.4 band)
        // so a small-sample bad spell still triggers but a single
        // off-week doesn't fake a loan crisis.
        let apps = self.statistics.played + self.statistics.played_subs;
        let loan_pos = self.position().position_group();
        let form = self.statistics.average_rating_realistic(loan_pos);
        if apps >= 6 && form > 0.0 && form < 6.2 {
            use crate::{
                HappinessEventCause, HappinessEventContext, HappinessEventScope,
                HappinessEventSeverity, LoanEventContext, LoanEventKind,
            };
            let lctx = LoanEventContext::new(LoanEventKind::ParentClubConcerned);
            let ctx = HappinessEventContext::new(
                HappinessEventCause::Other,
                HappinessEventSeverity::Moderate,
                HappinessEventScope::Boardroom,
            )
            .with_loan_context(lctx);
            self.happiness.add_event_with_context_and_cooldown(
                HappinessEventType::LackOfPlayingTime,
                -1.5,
                None,
                ctx,
                28,
            );
        }
    }
}

fn ambition_status_dampening(status: Option<&PlayerSquadStatus>) -> f32 {
    match status {
        Some(PlayerSquadStatus::KeyPlayer) => 1.0,
        Some(PlayerSquadStatus::FirstTeamRegular) => 0.8,
        Some(PlayerSquadStatus::FirstTeamSquadRotation) => 0.4,
        Some(PlayerSquadStatus::MainBackupPlayer) => 0.2,
        Some(PlayerSquadStatus::HotProspectForTheFuture)
        | Some(PlayerSquadStatus::DecentYoungster) => 0.1,
        Some(PlayerSquadStatus::NotNeeded) => 0.3,
        _ => 0.5,
    }
}

#[cfg(test)]
mod loan_morale_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        LoanEventKind, PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills,
    };

    fn build_loan_player_with_form(apps: u16, rating: f32) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.world_reputation = 4_000;
        attrs.current_reputation = 4_000;
        let person = PersonAttributes::default();
        let mut player = PlayerBuilder::new()
            .id(101)
            .full_name(FullName::new("Loan".into(), "Tester".into()))
            .birth_date(NaiveDate::from_ymd_opt(2003, 1, 1).unwrap())
            .country_id(1)
            .attributes(person)
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        player.contract_loan = Some(PlayerClubContract::new_loan(
            50_000,
            NaiveDate::from_ymd_opt(2027, 6, 30).unwrap(),
            1,
            1,
            2,
        ));
        player.statistics.played = apps;
        player.statistics.played_subs = 0;
        player.statistics.average_rating = rating;
        player
    }

    #[test]
    fn loan_underperformance_emits_loan_event_not_poor_training() {
        let mut p = build_loan_player_with_form(8, 5.4);
        // Hit the actual loan-morale branch. Reputations passed in are
        // arbitrary - the underperformance check only reads stats.
        p.process_loan_morale(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(), 2_500.0, 3_000);

        assert!(
            p.happiness
                .recent_events
                .iter()
                .all(|e| e.event_type != HappinessEventType::PoorTraining),
            "loan underperformance must never emit PoorTraining"
        );

        let lop_event = p
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::LackOfPlayingTime)
            .expect("loan underperformance must emit LackOfPlayingTime");
        let loan_ctx = lop_event
            .context
            .as_ref()
            .and_then(|c| c.loan_context.as_ref())
            .expect("loan event must carry a LoanEventContext");
        assert_eq!(loan_ctx.kind, LoanEventKind::ParentClubConcerned);
    }

    #[test]
    fn loan_branch_silent_for_decent_form() {
        let mut p = build_loan_player_with_form(8, 6.7);
        p.process_loan_morale(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(), 2_500.0, 3_000);
        assert!(
            p.happiness
                .recent_events
                .iter()
                .all(|e| e.event_type != HappinessEventType::LackOfPlayingTime
                    && e.event_type != HappinessEventType::PoorTraining),
            "decent form on loan must not fire the underperformance event"
        );
    }
}

#[cfg(test)]
mod playing_time_opportunity_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerClubContract, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };

    fn now() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()
    }

    /// Outfield player, age ~27, with a permanent contract at the given
    /// squad status and a transfer `days_ago` in the past.
    fn build_player(ca: u8, status: PlayerSquadStatus, days_ago: i64) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ca;
        attrs.world_reputation = 5_000;
        attrs.current_reputation = 5_000;
        let mut player = PlayerBuilder::new()
            .id(201)
            .full_name(FullName::new("PT".into(), "Tester".into()))
            .birth_date(NaiveDate::from_ymd_opt(1999, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap();
        let mut contract =
            PlayerClubContract::new(50_000, NaiveDate::from_ymd_opt(2029, 6, 30).unwrap());
        contract.squad_status = status;
        player.contract = Some(contract);
        player.last_transfer_date = Some(now() - chrono::Duration::days(days_ago));
        player
    }

    fn cfg() -> PlayingTimeFrustrationConfig {
        PlayingTimeFrustrationConfig::default()
    }

    // ── Scenario 1: permanent transfer, 7 days, club played 0 matches ──
    #[test]
    fn zero_eligible_matches_blocks_factor_and_gate() {
        let p = build_player(130, PlayerSquadStatus::KeyPlayer, 7);
        let opp = p.playing_time_opportunity(now());
        assert_eq!(opp.eligible_official_matches_since_join, 0);
        assert!(
            opp.can_judge(Some(&PlayerSquadStatus::KeyPlayer), &cfg(), None)
                .is_none(),
            "no eligible matches → gate must be closed"
        );
        // The morale factor must read neutral — the player was never
        // denied minutes he had a chance at.
        assert_eq!(p.calculate_playing_time_factor(1.3, now()), 0.0);
    }

    // ── Scenario 2: club plays matches, KeyPlayer gets 0 minutes ──
    #[test]
    fn keyplayer_left_out_after_grace_can_complain() {
        let mut p = build_player(130, PlayerSquadStatus::KeyPlayer, 60);
        // Club played 10 official matches; player left out of all of them.
        p.happiness.eligible_official_matches_since_join = 10;
        p.happiness.left_out_since_join = 10;

        let opp = p.playing_time_opportunity(now());
        let mult = opp
            .can_judge(Some(&PlayerSquadStatus::KeyPlayer), &cfg(), None)
            .expect("gate should be open past grace with a full sample");
        assert!(
            (mult - 1.0).abs() < f32::EPSILON,
            "past soft grace → full weight"
        );

        // Morale factor strongly negative (10 eligible ≥ 5 sample).
        let factor = p.calculate_playing_time_factor(1.0, now());
        assert!(factor < -10.0, "benched KeyPlayer factor was {factor}");
    }

    // ── Scenario 5: prospect moved, 14 days, 0 matches → no request ──
    #[test]
    fn prospect_no_matches_gate_closed() {
        let p = build_player(70, PlayerSquadStatus::HotProspectForTheFuture, 14);
        let opp = p.playing_time_opportunity(now());
        assert_eq!(opp.eligible_official_matches_since_join, 0);
        assert!(
            opp.can_judge(
                Some(&PlayerSquadStatus::HotProspectForTheFuture),
                &cfg(),
                None
            )
            .is_none(),
            "prospect with no fixtures must not request a loan"
        );
    }

    // ── Scenario 6: 5+ apps with a poor start/sub ratio still complains ──
    #[test]
    fn established_under_involved_regular_is_unhappy() {
        let mut p = build_player(130, PlayerSquadStatus::FirstTeamRegular, 200);
        // 20 eligible matches: 2 starts, 3 sub apps, 15 left out.
        p.happiness.eligible_official_matches_since_join = 20;
        p.happiness.starts_since_join = 2;
        p.happiness.sub_apps_since_join = 3;
        p.happiness.left_out_since_join = 15;

        let factor = p.calculate_playing_time_factor(1.0, now());
        assert!(factor < -8.0, "under-involved regular factor was {factor}");
    }

    // A settled regular getting his minutes reads positive.
    #[test]
    fn established_regular_meeting_expectations_is_content() {
        let mut p = build_player(130, PlayerSquadStatus::FirstTeamRegular, 200);
        // 20 eligible matches, 18 of them starts.
        p.happiness.eligible_official_matches_since_join = 20;
        p.happiness.starts_since_join = 18;
        p.happiness.sub_apps_since_join = 2;

        let factor = p.calculate_playing_time_factor(1.0, now());
        assert!(factor > 0.0, "ever-present regular factor was {factor}");
    }

    // ── Scenario 3 & 4: loan audit gate keys off matches, not days ──
    #[test]
    fn loan_no_matches_gate_closed_but_matches_open_it() {
        // Loan with a minimum-appearance clause of 20 over the season.
        let mut p = build_player(120, PlayerSquadStatus::NotYetSet, 15);
        p.contract_loan = Some(
            PlayerClubContract::new_loan(
                40_000,
                NaiveDate::from_ymd_opt(2027, 6, 30).unwrap(),
                1,
                1,
                2,
            )
            .with_loan_min_appearances(20),
        );

        // Day 15, club played nothing → audit must skip.
        let opp = p.playing_time_opportunity(now());
        assert_eq!(opp.eligible_official_matches_since_join, 0);
        assert!(opp.can_judge(None, &cfg(), Some(20)).is_none());

        // Now 5 eligible matches with zero appearances, 40 days in.
        p.last_transfer_date = Some(now() - chrono::Duration::days(40));
        p.happiness.eligible_official_matches_since_join = 5;
        p.happiness.left_out_since_join = 5;
        let opp = p.playing_time_opportunity(now());
        // min_eligible for loan = max(3, ceil(20*0.15)=3) = 3 ≤ 5.
        assert!(
            opp.can_judge(None, &cfg(), Some(20)).is_some(),
            "5 eligible matches with a min-apps clause should open the audit"
        );
    }

    // The grace ramp never overrides the zero-match hard block.
    #[test]
    fn grace_never_overrides_zero_match_block() {
        // Player transferred a year ago but counters are genuinely zero
        // *within* the post-transfer window would be a contradiction, so
        // simulate a fresh move (5 days) — grace would otherwise be 0 too,
        // but the point is the zero block returns None irrespective.
        let p = build_player(150, PlayerSquadStatus::KeyPlayer, 5);
        let opp = p.playing_time_opportunity(now());
        assert!(opp.frustration_multiplier(&cfg()) == 0.0);
        assert!(
            opp.can_judge(Some(&PlayerSquadStatus::KeyPlayer), &cfg(), None)
                .is_none()
        );
    }
}

#[cfg(test)]
mod morale_timeline_tests {
    //! Realistic post-transfer morale timelines. These exercise the full
    //! weekly pipeline (day-one `process_transfer_shock` → first weekly
    //! `process_happiness_full`) the way the simulator runs it, and pin the
    //! "8 days and already unhappy" behaviour the rebalance targets:
    //! ordinary signings settle, foreign signings carry a visible settling
    //! concern without collapsing, and only genuinely extreme moves (huge pay
    //! cut, no role) drop morale hard early.
    use super::*;
    use crate::club::player::adaptation::PendingSigning;
    use crate::club::player::builder::PlayerBuilder;
    use crate::context::{GlobalContext, SimulationContext};
    use crate::shared::fullname::FullName;
    use crate::{
        HappinessEventCause, HappinessEventContext, PersonAttributes, PlayerAttributes,
        PlayerClubContract, PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills,
    };
    use chrono::Duration;

    /// A configurable "fresh signing" timeline. Stage the pending signing,
    /// fire the day-one transfer shock, then run one or more weekly happiness
    /// ticks starting `days_since` after the move. Every knob has a calm,
    /// fair-move default so a test only sets what it is probing.
    struct Signing {
        world_rep: i16,
        current_ability: u8,
        ambition: f32,
        adaptability: f32,
        pressure: f32,
        controversy: f32,
        squad_status: PlayerSquadStatus,
        speaks_local: bool,
        previous_salary: u32,
        new_salary: u32,
        club_rep: f32,
        league_rep: u16,
        source_club_rep: u16,
        source_league_rep: u16,
        fits_formation: bool,
        coach_score: u8,
        facility: f32,
        days_since: i64,
    }

    impl Default for Signing {
        fn default() -> Self {
            // Calm, fair mid-table move: a CA-120 player whose level fits a
            // league-rep-7000 division, on a market-fair wage (the contract
            // valuation prices CA-120 at roughly 2.0M/yr at this tier).
            Signing {
                world_rep: 3_000,
                current_ability: 120,
                ambition: 12.0,
                adaptability: 12.0,
                pressure: 12.0,
                controversy: 8.0,
                squad_status: PlayerSquadStatus::FirstTeamRegular,
                speaks_local: true,
                previous_salary: 2_000_000,
                new_salary: 2_000_000,
                club_rep: 0.55,
                league_rep: 7_000,
                source_club_rep: 5_500,
                source_league_rep: 7_000,
                fits_formation: true,
                coach_score: 12,
                facility: 0.6,
                days_since: 8,
            }
        }
    }

    impl Signing {
        fn now() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()
        }

        /// High-ambition key player at a tiny club on a fair (tiny) wage. A
        /// real ambition concern drags morale into the unhappy-eligible band
        /// with no major trigger — so the formal status must wait out the
        /// persistence window. Coach context kept unknown to isolate the
        /// ambition signal.
        fn ambitious_at_tiny_club() -> Self {
            Signing {
                world_rep: 5_000,
                current_ability: 120,
                ambition: 20.0,
                squad_status: PlayerSquadStatus::KeyPlayer,
                previous_salary: 80_000,
                new_salary: 80_000,
                club_rep: 0.10,
                league_rep: 1_500,
                source_club_rep: 1_500,
                source_league_rep: 1_500,
                coach_score: 0,
                facility: 0.0,
                ..Default::default()
            }
        }

        /// Elite key player (CA 165, world-rep 7500) joining a much smaller
        /// club from a good-but-not-elite one, on a market-fair marquee wage
        /// (~5.0M — the valuation prices a CA-165 key player above a
        /// regular's). Tests override `new_salary` to model a pay cut.
        fn elite_star_at_small_club() -> Self {
            Signing {
                world_rep: 7_500,
                current_ability: 165,
                ambition: 17.0,
                pressure: 11.0,
                squad_status: PlayerSquadStatus::KeyPlayer,
                previous_salary: 5_000_000,
                new_salary: 5_000_000,
                club_rep: 0.48,
                league_rep: 4_500,
                source_club_rep: 6_800,
                source_league_rep: 6_000,
                coach_score: 10,
                facility: 0.5,
                ..Default::default()
            }
        }

        fn country_code(&self) -> &'static str {
            // Empty code → `speaks_local_language` returns true (domestic).
            // "es" maps to Spanish, which our test players never speak.
            if self.speaks_local { "" } else { "es" }
        }

        /// All-10 personality with the four axes a scenario varies pulled out.
        fn person_attributes(&self) -> PersonAttributes {
            PersonAttributes {
                adaptability: self.adaptability,
                ambition: self.ambition,
                controversy: self.controversy,
                loyalty: 10.0,
                pressure: self.pressure,
                professionalism: 10.0,
                sportsmanship: 10.0,
                temperament: 10.0,
                consistency: 10.0,
                important_matches: 10.0,
                dirtiness: 10.0,
            }
        }

        /// 4-4-2 containing the player's natural MidfielderCenter slot, or a
        /// back-five + five-forward shape with no midfield at all when the
        /// move offers no natural role.
        fn formation(&self) -> [PlayerPositionType; 11] {
            if self.fits_formation {
                [
                    PlayerPositionType::Goalkeeper,
                    PlayerPositionType::DefenderLeft,
                    PlayerPositionType::DefenderCenter,
                    PlayerPositionType::DefenderCenter,
                    PlayerPositionType::DefenderRight,
                    PlayerPositionType::MidfielderLeft,
                    PlayerPositionType::MidfielderCenter,
                    PlayerPositionType::MidfielderCenter,
                    PlayerPositionType::MidfielderRight,
                    PlayerPositionType::Striker,
                    PlayerPositionType::Striker,
                ]
            } else {
                [
                    PlayerPositionType::Goalkeeper,
                    PlayerPositionType::DefenderLeft,
                    PlayerPositionType::DefenderCenter,
                    PlayerPositionType::DefenderCenter,
                    PlayerPositionType::DefenderCenter,
                    PlayerPositionType::DefenderRight,
                    PlayerPositionType::Striker,
                    PlayerPositionType::Striker,
                    PlayerPositionType::Striker,
                    PlayerPositionType::Striker,
                    PlayerPositionType::Striker,
                ]
            }
        }

        fn club_ctx(&self) -> ClubMoraleContext {
            ClubMoraleContext {
                coach_best_technical: self.coach_score,
                coach_best_mental: self.coach_score,
                coach_best_fitness: self.coach_score,
                coach_best_goalkeeping: self.coach_score,
                training_facility_quality: self.facility,
                youth_facility_quality: self.facility,
            }
        }

        fn build_player(&self) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.world_reputation = self.world_rep;
            attrs.current_reputation = self.world_rep;
            attrs.current_ability = self.current_ability;
            attrs.potential_ability = self.current_ability;

            let mut player = PlayerBuilder::new()
                .id(301)
                .full_name(FullName::new("Fresh".into(), "Signing".into()))
                .birth_date(NaiveDate::from_ymd_opt(1999, 1, 1).unwrap())
                .country_id(1)
                .attributes(self.person_attributes())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .build()
                .unwrap();

            let mut contract = PlayerClubContract::new(
                self.new_salary,
                NaiveDate::from_ymd_opt(2030, 6, 30).unwrap(),
            );
            contract.squad_status = self.squad_status.clone();
            player.contract = Some(contract);
            player
        }

        fn pending_signing(&self) -> PendingSigning {
            PendingSigning {
                previous_salary: Some(self.previous_salary),
                fee: 1_000_000.0,
                is_loan: false,
                destination_club_id: 999,
                had_return_home_desire: false,
                had_european_desire: false,
                had_libertadores_desire: false,
                source_club_reputation: self.source_club_rep,
                source_league_reputation: self.source_league_rep,
                dest_position_depth_rank: Some(2),
            }
        }

        fn stage(&self, player: &mut Player, join: NaiveDate) {
            player.last_transfer_date = Some(join);
            player.pending_signing = Some(self.pending_signing());
            let formation = self.formation();
            player.process_transfer_shock(
                join,
                self.club_rep,
                self.league_rep,
                self.country_code(),
                Some(&formation),
            );
        }

        /// Run `n` weekly happiness ticks, the first `days_since` after the
        /// move and each subsequent one a week later. Returns the processed
        /// player plus the final weekly result.
        fn run_ticks(&self, n: u32) -> (Player, PlayerResult) {
            let now = Self::now();
            let join = now - Duration::days(self.days_since);
            let mut player = self.build_player();
            self.stage(&mut player, join);

            // Season state carries the club's real league reputation (as the
            // production caller does) so the salary / club-fit / pressure
            // valuations are evaluated against the same league the move used.
            let season_state = TeamSeasonState {
                league_reputation: self.league_rep,
                ..Default::default()
            };
            let mut result = PlayerResult::new(player.id);
            for i in 0..n {
                let tick_now = now + Duration::days(7 * i as i64);
                result = PlayerResult::new(player.id);
                player.process_happiness_full(
                    &mut result,
                    tick_now,
                    self.club_rep,
                    season_state,
                    self.club_ctx(),
                );
            }
            (player, result)
        }

        /// The common case — a single first weekly tick.
        fn run(&self) -> (Player, PlayerResult) {
            self.run_ticks(1)
        }

        fn is_unhappy(player: &Player) -> bool {
            player.statuses.get().contains(&PlayerStatusType::Unh)
        }

        /// Total negative magnitude of every first-tick transfer-shock event
        /// still on the log (used to assert the stacking budget held).
        fn shock_event_load(player: &Player) -> f32 {
            const SHOCK: [HappinessEventType; 13] = [
                HappinessEventType::AmbitionShock,
                HappinessEventType::SalaryShock,
                HappinessEventType::RoleMismatch,
                HappinessEventType::FeelingIsolated,
                HappinessEventType::OverawedByEliteClub,
                HappinessEventType::RolePathBlockedAtEliteClub,
                HappinessEventType::DressingRoomStatusShock,
                HappinessEventType::TooGoodForLevel,
                HappinessEventType::StepDownEmbarrassment,
                HappinessEventType::TrainingStandardFrustration,
                HappinessEventType::FanExpectationBurden,
                HappinessEventType::MediaSpotlightPressure,
                HappinessEventType::LoanLevelMismatch,
            ];
            player
                .happiness
                .recent_events
                .iter()
                .filter(|e| SHOCK.contains(&e.event_type) && e.magnitude < 0.0)
                .map(|e| e.magnitude)
                .sum()
        }
    }

    // ── Acceptance: ordinary domestic signing settles ──────────────
    #[test]
    fn same_country_fair_salary_signing_stays_settled() {
        let (player, result) = Signing::default().run();
        let morale = player.happiness.morale;
        assert!(
            morale >= 45.0,
            "fair domestic signing should stay settled at 8 days, morale was {morale}"
        );
        assert!(
            !Signing::is_unhappy(&player),
            "fair domestic signing must not be formally Unhappy at 8 days"
        );
        assert!(!result.unhappy);
    }

    // ── Acceptance: foreign, no language, low adaptability ─────────
    #[test]
    fn foreign_low_adaptability_settles_with_concern_not_collapse() {
        let signing = Signing {
            speaks_local: false,
            adaptability: 5.0,
            ambition: 10.0, // neutral ambition → isolate the language signal
            ..Default::default()
        };
        let (player, _) = signing.run();
        let morale = player.happiness.morale;

        // A real settling concern is surfaced...
        assert!(
            player
                .happiness
                .has_recent_event(&HappinessEventType::FeelingIsolated, 30),
            "foreign signing with no local language should show a settling concern"
        );
        // ...but morale holds up and the status does not harden.
        assert!(
            morale >= 40.0,
            "foreign settling signing should stay >= 40 at 8 days, morale was {morale}"
        );
        assert!(
            !Signing::is_unhappy(&player),
            "settling foreign signing must not be Unhappy at 8 days, morale {morale}"
        );
    }

    // ── Acceptance: elite star joins a much smaller club, fair terms ─
    #[test]
    fn elite_star_smaller_club_fair_salary_stays_above_floor() {
        let signing = Signing::elite_star_at_small_club();
        let (player, _) = signing.run();
        let morale = player.happiness.morale;
        let f = &player.happiness.factors;

        // The status mismatch is *visible* — pressure/ambition concerns show.
        assert!(
            f.pressure_load < 0.0 || f.ambition_fit < 0.0,
            "elite star at a smaller club should show a pressure/ambition concern \
             (pressure {}, ambition {})",
            f.pressure_load,
            f.ambition_fit
        );
        // ...without a fair, voluntary move collapsing morale.
        assert!(
            morale >= 35.0,
            "elite star, fair salary, normal role should stay >= 35 at 8 days, morale was {morale}"
        );
        assert!(
            !Signing::is_unhappy(&player),
            "elite star on fair terms must not be Unhappy at 8 days, morale {morale}"
        );
    }

    // ── Acceptance: elite star + huge pay cut drops hard ───────────
    #[test]
    fn elite_star_huge_pay_cut_drops_hard_and_wants_improvement() {
        // Was on a 5.0M deal; the new club can only offer 1.0M — an 80% pay
        // cut, well past the SalaryShock floor.
        let signing = Signing {
            new_salary: 1_000_000,
            ..Signing::elite_star_at_small_club()
        };
        let (player, result) = signing.run();
        let morale = player.happiness.morale;

        assert!(
            morale < 35.0,
            "an ~80% pay cut for an elite star should drop morale hard, morale was {morale}"
        );
        assert!(
            result.contract.want_improve_contract,
            "a huge wage cut should make the player ask for a better contract"
        );
        // The wage cut is a hard signal that lands in full.
        assert!(
            player
                .happiness
                .has_recent_event(&HappinessEventType::SalaryShock, 14),
            "an 80% pay cut should record a SalaryShock"
        );
        // …but the move does not automatically become a transfer request.
        assert!(
            !player.statuses.get().contains(&PlayerStatusType::Req),
            "a pay-cut grievance should not auto-escalate to a transfer request"
        );
    }

    // ── Acceptance: playing-time guard at 0-1 official matches ─────
    #[test]
    fn key_player_zero_official_matches_has_no_playing_time_complaint() {
        // 8 days in, club has played 0 (and separately 1) official matches.
        let signing = Signing {
            squad_status: PlayerSquadStatus::KeyPlayer,
            current_ability: 130,
            ..Default::default()
        };
        let now = Signing::now();
        let mut player = signing.build_player();
        player.last_transfer_date = Some(now - Duration::days(8));

        // 0 official matches → no complaint.
        assert_eq!(
            player.calculate_playing_time_factor(1.0, now),
            0.0,
            "0 official matches: playing-time factor must be neutral"
        );

        // 1 official match (sub-sample): still no complaint.
        player.happiness.eligible_official_matches_since_join = 1;
        player.happiness.left_out_since_join = 1;
        assert_eq!(
            player.calculate_playing_time_factor(1.0, now),
            0.0,
            "1 official match is below the sample floor: still neutral"
        );
    }

    // ── Acceptance: real frustration once there's a real sample ────
    #[test]
    fn key_player_benched_after_grace_is_meaningfully_negative() {
        let signing = Signing {
            squad_status: PlayerSquadStatus::KeyPlayer,
            current_ability: 130,
            ..Default::default()
        };
        let now = Signing::now();
        let mut player = signing.build_player();
        // Past the grace window with a real, full sample of fixtures missed.
        player.last_transfer_date = Some(now - Duration::days(60));
        player.happiness.eligible_official_matches_since_join = 10;
        player.happiness.left_out_since_join = 10;

        let factor = player.calculate_playing_time_factor(1.0, now);
        assert!(
            factor <= -8.0,
            "a benched key player with 10 missed eligible matches should be \
             meaningfully unhappy about minutes, factor was {factor}"
        );
    }

    // ── Acceptance: coach-credibility source guard ─────────────────
    #[test]
    fn coach_credibility_neutral_when_context_unknown() {
        let signing = Signing {
            current_ability: 170,
            ..Default::default()
        };
        let player = signing.build_player();
        // The caller builds this via `unwrap_or_default()` when the player has
        // no resolved club context — every coach score is 0.
        let unknown = ClubMoraleContext::default();
        assert_eq!(
            player.calculate_coach_credibility(&unknown),
            0.0,
            "unknown (all-zero) coach context must read neutral, not contempt"
        );
    }

    #[test]
    fn coach_credibility_negative_when_context_is_real() {
        let signing = Signing {
            current_ability: 170,
            ..Default::default()
        };
        let player = signing.build_player();
        // Real, weak coaching for a top player → genuine contempt.
        let weak = ClubMoraleContext {
            coach_best_technical: 8,
            coach_best_mental: 8,
            coach_best_fitness: 8,
            coach_best_goalkeeping: 8,
            ..Default::default()
        };
        assert!(
            player.calculate_coach_credibility(&weak) < 0.0,
            "a top player under genuinely weak coaching should lose credibility"
        );
    }

    // ── Honeymoon ramp shape ───────────────────────────────────────
    #[test]
    fn transfer_honeymoon_damps_early_negatives_and_recovers() {
        let day0 = TransferHoneymoon::since(Some(0));
        let day_late = TransferHoneymoon::since(Some(40));
        let settled = TransferHoneymoon::since(None);

        // Day 0 strongly damps a negative settling factor...
        assert!(day0.damp_negative(-10.0) > -5.0);
        // ...positives always pass through untouched...
        assert_eq!(day0.damp_negative(6.0), 6.0);
        // ...the damp eases monotonically as the player settles...
        assert!(
            TransferHoneymoon::since(Some(5)).damp_negative(-10.0)
                > TransferHoneymoon::since(Some(20)).damp_negative(-10.0)
        );
        // ...and past the ramp (or with no recent move) there is no damping.
        assert_eq!(day_late.damp_negative(-10.0), -10.0);
        assert_eq!(settled.damp_negative(-10.0), -10.0);
    }

    // ── Unhappy hysteresis: persistence required, except majors ────
    #[test]
    fn unhappy_status_needs_persistence_without_a_major_trigger() {
        // High-ambition player at a tiny club, fair wage, domestic, normal
        // role: a real ambition concern drags morale into the unhappy band,
        // but there is no major trigger — so the status must wait a second
        // weekly tick before it hardens.
        let signing = Signing::ambitious_at_tiny_club();

        let (after_one, _) = signing.run_ticks(1);
        assert!(
            !Signing::is_unhappy(&after_one),
            "a single low-mood tick without a major trigger must not harden to Unhappy \
             (morale {})",
            after_one.happiness.morale
        );

        let (after_two, _) = signing.run_ticks(2);
        assert!(
            Signing::is_unhappy(&after_two),
            "a persistent low mood (two ticks) with a serious concern should harden to Unhappy \
             (morale {})",
            after_two.happiness.morale
        );
    }

    // ── First-tick stacking budget ─────────────────────────────────
    #[test]
    fn ordinary_star_move_caps_first_tick_shock_stack() {
        // Star at a weaker club, fair wage, normal role — not an extreme move,
        // so the combined soft-shock load is held within the ordinary budget.
        let signing = Signing::elite_star_at_small_club();
        let now = Signing::now();
        let join = now - Duration::days(8);
        let mut player = signing.build_player();
        signing.stage(&mut player, join);

        let load = Signing::shock_event_load(&player);
        assert!(
            load >= -10.5,
            "ordinary star move first-tick shock load should be capped near -10, was {load}"
        );
    }

    #[test]
    fn extreme_pay_cut_unlocks_higher_ceiling_and_keeps_salary_shock_full() {
        // 5.0M → 1.0M is an 80% cut: huge_salary_cut unlocks the extreme
        // ceiling and the SalaryShock is a hard event.
        let signing = Signing {
            new_salary: 1_000_000,
            ..Signing::elite_star_at_small_club()
        };
        let now = Signing::now();
        let join = now - Duration::days(8);
        let mut player = signing.build_player();
        signing.stage(&mut player, join);

        // The hard SalaryShock is never scaled — it lands at its full
        // (severity-scaled) magnitude.
        let salary_shock = player
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::SalaryShock)
            .expect("an 80% pay cut must record a SalaryShock");
        assert!(
            salary_shock.magnitude <= -6.0,
            "SalaryShock should land in full, was {}",
            salary_shock.magnitude
        );

        // The extreme ceiling allows a deeper total load than an ordinary
        // move would (which the previous test pinned near -10).
        let load = Signing::shock_event_load(&player);
        assert!(
            load < -10.5,
            "an extreme move should be allowed a deeper first-tick load than the \
             ordinary cap, was {load}"
        );
    }

    // ── Formal-Unh semantics: concern vs hard verdict ──────────────
    #[test]
    fn first_low_morale_tick_reports_concern_not_formal_unhappy() {
        // The first low-mood tick (no major trigger) must NOT apply the
        // formal Unh status, and `result.unhappy` must mirror that — it stays
        // false. The softer `morale_concern` flag carries the settling signal.
        let signing = Signing::ambitious_at_tiny_club();
        let (player, result) = signing.run_ticks(1);

        assert!(
            !Signing::is_unhappy(&player),
            "no formal Unh after a single low-mood tick (morale {})",
            player.happiness.morale
        );
        assert!(
            !result.unhappy,
            "result.unhappy mirrors the formal status — must stay false before it hardens"
        );
        assert!(
            result.morale_concern,
            "a settling low mood should surface as a soft concern (morale {})",
            player.happiness.morale
        );
    }

    // ── Morale-composition audit helper ────────────────────────────
    #[test]
    fn morale_breakdown_components_stay_in_sane_bands() {
        // Elite star at a smaller club on fair terms — a known, exercised
        // scenario. The breakdown must reconstruct morale and no single
        // component should dominate pathologically.
        let signing = Signing::elite_star_at_small_club();
        let (player, _) = signing.run();
        let b = player.happiness.morale_breakdown();

        // Internally consistent: the components reconstruct the morale, and
        // match the committed value on the player.
        let reconstructed =
            50.0 + b.core_factor_sum + b.derived_factor_sum + b.event_sum + b.hidden_pressure;
        assert!(
            (reconstructed - b.morale).abs() < 0.01,
            "breakdown components must reconstruct morale: {reconstructed} vs {}",
            b.morale
        );
        assert!((b.morale - player.happiness.morale).abs() < 0.01);

        // Sane bands — no component runaway.
        assert!(
            b.core_factor_sum.abs() <= 20.0,
            "core factor sum out of band: {}",
            b.core_factor_sum
        );
        assert!(
            (-10.0..=6.0).contains(&b.derived_factor_sum),
            "derived factor sum out of band: {}",
            b.derived_factor_sum
        );
        assert!(
            (-20.0..=0.0).contains(&b.event_sum),
            "event sum out of band: {}",
            b.event_sum
        );
        assert_eq!(
            b.hidden_pressure, 0.0,
            "no hidden form pressure expected here"
        );
        assert!(
            (35.0..=55.0).contains(&b.morale),
            "morale out of band: {}",
            b.morale
        );
    }

    // ── Simulator-cadence integration test (Goal 4) ────────────────
    #[test]
    fn simulator_cadence_normal_signing_no_unh_first_week() {
        // Drive the real weekly pipeline through `Player::simulate_with_options`
        // over the player's first eight days at the club, one tick per day, so
        // the day-one transfer shock and the Monday week-beginning happiness
        // tick fire exactly as the simulator schedules them.
        let signing = Signing::default();
        // Tuesday 2026-02-24 → the first week-beginning lands on Monday
        // 2026-03-02 (day six); the loop runs through day eight.
        let join = NaiveDate::from_ymd_opt(2026, 2, 24).unwrap();
        let mut player = signing.build_player();
        player.last_transfer_date = Some(join);
        player.pending_signing = Some(signing.pending_signing());

        let mut saw_week_beginning = false;
        let mut day = join;
        let end = join + Duration::days(8);
        while day <= end {
            let dt = day.and_hms_opt(0, 0, 0).unwrap();
            let ctx = GlobalContext::new(SimulationContext::new(dt))
                .with_country(1)
                .with_team_reputation(1, signing.club_rep)
                .with_club(1, "Test FC");
            saw_week_beginning |= ctx.simulation.is_week_beginning();
            // skip_natural_development = true keeps the tick focused on the
            // morale pipeline.
            let _ = player.simulate_with_options(ctx, true);
            day += Duration::days(1);
        }

        assert!(
            saw_week_beginning,
            "the eight-day window must contain at least one Monday happiness tick"
        );
        assert!(
            player.pending_signing.is_none(),
            "day-one simulate should consume the pending signing via transfer shock"
        );
        assert!(
            !Signing::is_unhappy(&player),
            "a normal signing must not be formally Unhappy in its first week (morale {})",
            player.happiness.morale
        );
        assert!(
            player.happiness.morale >= 45.0,
            "a normal signing's morale should stay stable in week one, was {}",
            player.happiness.morale
        );
    }

    /// Adds a single `ConflictWithTeammate` row with the given severity /
    /// scope / follow-up so the conflict-trigger tests can dial each axis.
    struct ConflictRow;
    impl ConflictRow {
        fn add(
            player: &mut Player,
            magnitude: f32,
            severity: HappinessEventSeverity,
            scope: HappinessEventScope,
            follow_up: Option<HappinessEventFollowUp>,
            partner: u32,
        ) {
            let mut ctx =
                HappinessEventContext::new(HappinessEventCause::PersonalityClash, severity, scope);
            if let Some(f) = follow_up {
                ctx = ctx.with_follow_up(f);
            }
            player
                .happiness
                .add_event_with_partner_context_and_cooldown(
                    HappinessEventType::ConflictWithTeammate,
                    magnitude,
                    partner,
                    ctx,
                    0,
                );
        }
    }

    // ── Conflict major-trigger gating (Goal 2) ─────────────────────
    #[test]
    fn minor_private_conflict_is_not_a_major_trigger() {
        let mut p = Signing::default().build_player();
        ConflictRow::add(
            &mut p,
            -2.0,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::DressingRoom,
            None,
            99,
        );
        assert!(
            !UnhappyAssessment::has_major_event(&p),
            "a lone routine -2 private conflict must not bypass the persistence window"
        );
    }

    #[test]
    fn serious_repeated_or_public_conflict_is_a_major_trigger() {
        // Serious by magnitude (|mag| >= 4).
        let mut by_magnitude = Signing::default().build_player();
        ConflictRow::add(
            &mut by_magnitude,
            -5.0,
            HappinessEventSeverity::Serious,
            HappinessEventScope::DressingRoom,
            None,
            1,
        );
        assert!(
            UnhappyAssessment::has_major_event(&by_magnitude),
            "a serious-magnitude conflict should bypass persistence"
        );

        // Serious by context severity, even at a routine magnitude.
        let mut by_severity = Signing::default().build_player();
        ConflictRow::add(
            &mut by_severity,
            -2.0,
            HappinessEventSeverity::Serious,
            HappinessEventScope::DressingRoom,
            None,
            1,
        );
        assert!(
            UnhappyAssessment::has_major_event(&by_severity),
            "a serious-severity conflict should bypass persistence"
        );

        // Public (media) blow-up.
        let mut public = Signing::default().build_player();
        ConflictRow::add(
            &mut public,
            -2.0,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::Media,
            None,
            1,
        );
        assert!(
            UnhappyAssessment::has_major_event(&public),
            "a public conflict should bypass persistence"
        );

        // Dressing-room-damage follow-up.
        let mut dressing_room = Signing::default().build_player();
        ConflictRow::add(
            &mut dressing_room,
            -2.0,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::DressingRoom,
            Some(HappinessEventFollowUp::DressingRoomDamageRisk),
            1,
        );
        assert!(
            UnhappyAssessment::has_major_event(&dressing_room),
            "a flagged dressing-room-damage conflict should bypass persistence"
        );

        // Repeated — two routine private rows in the window.
        let mut repeated = Signing::default().build_player();
        ConflictRow::add(
            &mut repeated,
            -2.0,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::DressingRoom,
            None,
            1,
        );
        ConflictRow::add(
            &mut repeated,
            -2.0,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::DressingRoom,
            None,
            2,
        );
        assert!(
            UnhappyAssessment::has_major_event(&repeated),
            "two conflict rows in the window should bypass persistence"
        );
    }
}

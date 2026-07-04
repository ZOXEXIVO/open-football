//! Emit-site logic for the manager-relationship arc and match-day
//! trust events that don't fit cleanly into the per-match handler:
//!
//! * `AskedForPrivateTalk` — formal request when a serious unresolved
//!   concern accumulates over recent weeks.
//! * `ManagerTrustGrowing` / `ManagerTrustEroding` — aggregate moods
//!   built from repeated kept-promise / praise / drop / criticism
//!   signals so the per-match noise rolls up into one weekly headline.
//! * `UnhappyWithTacticalRole` — tactical-usage frustration when the
//!   player is repeatedly slotted into a role he cannot deliver.
//! * `ThreatenedByNewSigning` — positional / status competition reaction
//!   to a new arrival in the same lane.
//! * `BenchedForBigMatch` / `TrustedInBigMatch` — major-fixture
//!   selection / drop, called from the match selection layer.
//! * `SubstitutionFrustration` — early hooks / hooks-while-playing-well
//!   / removed-early-in-big-match.
//! * `InjurySetback` — re-injury or recovery delay.
//! * `ConcernedByClubDirection` / `EncouragedBySquadInvestment` —
//!   transfer-window aggregate reaction.
//!
//! Each emit path is cooldowned and gated to keep the player event feed
//! readable. The catalog magnitudes live in `MoraleEventCatalog`; this
//! module owns the gating + context wiring.

use super::MatchOutcome;
use super::MatchParticipation;
use super::scaling;
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::player::Player;
use crate::{
    BigMatchDecision, BigMatchKind, BigMatchSelectionContext, ClubDirectionContext,
    ClubDirectionEvidence, ClubDirectionKind, HappinessEventCause, HappinessEventContext,
    HappinessEventFollowUp, HappinessEventScope, HappinessEventSeverity, HappinessEventType,
    InjuryRecoveryEventContext, InjuryRecoveryEvidence, InjuryRecoveryStage,
    ManagerInteractionEventContext, ManagerInteractionTone, ManagerInteractionTopic,
    MatchSelectionContext, NewSigningThreatContext, NewSigningThreatReason, PlayerAcceptance,
    PlayerSquadStatus, PrivateTalkReason, PrivateTalkRequestContext, RoleStatusEventContext,
    RoleStatusKind, SelectionDecisionScope, SelectionOmissionReason, SelectionRole,
    SubstitutionFrustrationContext, SubstitutionFrustrationKind,
};

/// Result of a private-talk detection pass. The driver is "what's the
/// dominant grievance", computed from the player's recent event history
/// and current morale state.
#[derive(Debug, Clone, Copy)]
struct PrivateTalkSignal {
    reason: PrivateTalkReason,
    /// Weight of the dominant concern — used to scale severity, not the
    /// catalog magnitude (the catalog already encodes the base).
    severity_score: f32,
}

impl Player {
    // ───────────────────────────────────────────────────────────
    // AskedForPrivateTalk
    // ───────────────────────────────────────────────────────────

    /// Called from the weekly happiness tick AFTER factor recalculation.
    /// Detects a dominant unresolved concern and, when severe enough,
    /// fires an `AskedForPrivateTalk` event with a structured payload
    /// describing which axis the player flagged. Rare and cooldowned —
    /// don't emit for every unhappy player every week.
    pub fn maybe_emit_asked_for_private_talk(&mut self) {
        if self
            .happiness
            .has_recent_event(&HappinessEventType::AskedForPrivateTalk, 45)
        {
            return;
        }
        // High-pro players rarely escalate to a private talk — they
        // brood, sulk, or just leave. Soft gate, not absolute.
        let prof = self.attributes.professionalism;
        let morale = self.happiness.morale;
        let Some(signal) = self.classify_private_talk_signal() else {
            return;
        };
        // Severity gate: the dominant grievance must clear a threshold.
        // High-pro players need a sharper signal before they walk in.
        let gate = if prof >= 16.0 { 4.0 } else { 2.5 };
        if signal.severity_score < gate {
            return;
        }
        // Also gate on morale — a player who's content despite isolated
        // signals doesn't need a private chat.
        if morale >= 55.0 {
            return;
        }

        let trust = self.happiness.factors.promise_trust;
        let repeated =
            self.happiness.recent_events.iter().any(|e| {
                e.event_type == HappinessEventType::AskedForPrivateTalk && e.days_ago <= 180
            });

        let private_ctx = PrivateTalkRequestContext::new(signal.reason)
            .with_trust(trust)
            .with_morale(morale)
            .with_repeated(repeated);

        let base = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::AskedForPrivateTalk);
        let severity_mul = (signal.severity_score / 4.0).clamp(0.6, 1.6);
        let magnitude = base * severity_mul;

        let happiness_ctx = HappinessEventContext::new(
            cause_for_private_talk(signal.reason),
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::Personal,
        )
        .with_private_talk_context(private_ctx)
        .with_follow_up(if repeated {
            HappinessEventFollowUp::ContractRequestRisk
        } else {
            HappinessEventFollowUp::ManagerInterventionRisk
        });
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::AskedForPrivateTalk,
            magnitude,
            None,
            happiness_ctx,
            45,
        );
    }

    /// Pick the dominant grievance from the player's recent event
    /// history + current state. Returns `None` when no axis is loud
    /// enough to warrant a private conversation.
    fn classify_private_talk_signal(&self) -> Option<PrivateTalkSignal> {
        // Count recent signals by axis. Use 60-day windows so a single
        // bad week doesn't push the player to escalate.
        let dropped = self.count_recent(&HappinessEventType::MatchDropped, 60) as f32;
        let lack_minutes = self.count_recent(&HappinessEventType::LackOfPlayingTime, 90) as f32;
        let playing_time_signal = dropped * 1.0
            + lack_minutes * 2.0
            + (-self.happiness.factors.playing_time * 0.4).max(0.0);

        let contract_signals =
            self.count_recent(&HappinessEventType::ContractTalksStalled, 120) as f32 * 2.0
                + self.count_recent(&HappinessEventType::RejectedContractOffer, 120) as f32 * 1.5
                + (-self.happiness.factors.salary_satisfaction * 0.4).max(0.0);

        let transfer_signals =
            self.count_recent(&HappinessEventType::TransferBidRejected, 90) as f32 * 2.0
                + self.count_recent(&HappinessEventType::DreamMoveCollapsed, 90) as f32 * 1.5;

        let captaincy_signals =
            self.count_recent(&HappinessEventType::CaptaincyRemoved, 120) as f32 * 3.0
                + self.count_recent(&HappinessEventType::LostStartingPlace, 120) as f32 * 1.5;

        let tactical_signals =
            self.count_recent(&HappinessEventType::UnhappyWithTacticalRole, 120) as f32 * 2.5
                + self.count_recent(&HappinessEventType::RoleMismatch, 120) as f32 * 2.0;

        // Manager-relationship: erosion events plus broken promises plus
        // a heavily negative manager_relationship factor.
        let mgr_signals = self.count_recent(&HappinessEventType::ManagerTrustEroding, 120) as f32
            * 3.0
            + self.count_recent(&HappinessEventType::PromiseBroken, 120) as f32 * 2.5
            + self.count_recent(&HappinessEventType::ManagerCriticism, 60) as f32 * 0.5
            + (-self.happiness.factors.manager_relationship * 0.3).max(0.0);

        let candidates = [
            (PrivateTalkReason::PlayingTime, playing_time_signal),
            (PrivateTalkReason::Contract, contract_signals),
            (PrivateTalkReason::TransferStatus, transfer_signals),
            (PrivateTalkReason::CaptaincyOrStatus, captaincy_signals),
            (PrivateTalkReason::TacticalRole, tactical_signals),
            (PrivateTalkReason::ManagerRelationship, mgr_signals),
        ];

        let (reason, severity_score) = candidates
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
        if severity_score <= 0.0 {
            return None;
        }
        Some(PrivateTalkSignal {
            reason,
            severity_score,
        })
    }

    fn count_recent(&self, event_type: &HappinessEventType, days: u16) -> usize {
        self.happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == *event_type && e.days_ago <= days)
            .count()
    }

    // ───────────────────────────────────────────────────────────
    // ManagerTrustGrowing / ManagerTrustEroding
    // ───────────────────────────────────────────────────────────

    /// Aggregate weekly read of "is the manager-relationship axis
    /// trending up or down?". Fires a single growing/eroding row at most
    /// once per 60d so the feed doesn't repeat last week's bench
    /// story-line. Picks the more specific event over duplicates: if
    /// `TrustedInBigMatch` already landed this week, the growing
    /// counterpart suppresses; same for `BenchedForBigMatch` blocking
    /// the eroding aggregate.
    pub fn maybe_emit_manager_trust_arc(&mut self) {
        let cooldown_days: u16 = 60;
        if self
            .happiness
            .has_recent_event(&HappinessEventType::ManagerTrustGrowing, cooldown_days)
            || self
                .happiness
                .has_recent_event(&HappinessEventType::ManagerTrustEroding, cooldown_days)
        {
            return;
        }
        // Specific big-match events trump the generic aggregate this tick.
        if self
            .happiness
            .has_recent_event(&HappinessEventType::TrustedInBigMatch, 14)
            || self
                .happiness
                .has_recent_event(&HappinessEventType::BenchedForBigMatch, 14)
        {
            return;
        }

        let positive: f32 = self.count_recent(&HappinessEventType::PromiseKept, 90) as f32 * 1.5
            + self.count_recent(&HappinessEventType::ManagerPraise, 60) as f32 * 0.75
            + self.count_recent(&HappinessEventType::ManagerEncouragement, 60) as f32 * 0.75
            + self.count_recent(&HappinessEventType::WonStartingPlace, 90) as f32 * 1.0;
        let negative: f32 = self.count_recent(&HappinessEventType::PromiseBroken, 90) as f32 * 2.0
            + self.count_recent(&HappinessEventType::ManagerCriticism, 60) as f32 * 0.5
            + self.count_recent(&HappinessEventType::MatchDropped, 45) as f32 * 0.5
            + self.count_recent(&HappinessEventType::LostStartingPlace, 90) as f32 * 1.5
            + self.count_recent(&HappinessEventType::UnhappyWithTacticalRole, 90) as f32 * 1.5;

        let delta = positive - negative;
        // Higher gate than a single-event drop — this is an aggregate
        // mood, not a one-off reaction.
        if delta >= 3.0 {
            self.emit_manager_trust_growing(positive, negative);
        } else if delta <= -3.0 {
            self.emit_manager_trust_eroding(positive, negative);
        }
    }

    fn emit_manager_trust_growing(&mut self, positive: f32, negative: f32) {
        let base = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::ManagerTrustGrowing);
        let delta = (positive - negative).clamp(3.0, 12.0);
        let magnitude = base * (delta / 6.0).clamp(0.6, 1.6);
        let ctx_inner = ManagerInteractionEventContext::new(
            ManagerInteractionTopic::Performance,
            ManagerInteractionTone::Supportive,
            PlayerAcceptance::Motivated,
        )
        .with_trust(self.happiness.factors.promise_trust);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::ManagerSupport,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::DressingRoom,
        )
        .with_manager_interaction_context(ctx_inner)
        .with_follow_up(HappinessEventFollowUp::ManagerTrustRising);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::ManagerTrustGrowing,
            magnitude,
            None,
            happiness_ctx,
            60,
        );
    }

    fn emit_manager_trust_eroding(&mut self, positive: f32, negative: f32) {
        let base = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::ManagerTrustEroding);
        let delta = (negative - positive).clamp(3.0, 12.0);
        let magnitude = base * (delta / 6.0).clamp(0.6, 1.6);
        let ctx_inner = ManagerInteractionEventContext::new(
            ManagerInteractionTopic::Performance,
            ManagerInteractionTone::Stern,
            PlayerAcceptance::Discouraged,
        )
        .with_trust(self.happiness.factors.promise_trust)
        .with_repeated_recently(true);
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::LeadershipDispute,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::DressingRoom,
        )
        .with_manager_interaction_context(ctx_inner)
        .with_follow_up(HappinessEventFollowUp::ManagerInterventionRisk);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::ManagerTrustEroding,
            magnitude,
            None,
            happiness_ctx,
            60,
        );
    }

    // ───────────────────────────────────────────────────────────
    // UnhappyWithTacticalRole
    // ───────────────────────────────────────────────────────────

    /// Fires when the player has been deployed in an unsuitable role
    /// repeatedly. Distinct from `RoleMismatch` (formation has no slot
    /// at all) — this is tactical-usage frustration when the slot
    /// exists but the player isn't suited to how it's being asked of
    /// him. Detection signal comes from the caller (squad selection
    /// layer); this is the visible event wrapper.
    ///
    /// `repeated_mismatch_count` is the number of recent matches the
    /// player was used in a poor-fit role.
    pub fn on_tactical_role_mismatch(
        &mut self,
        repeated_mismatch_count: u8,
        formation_slot: Option<SelectionRole>,
    ) {
        if repeated_mismatch_count < 3 {
            return;
        }
        if self
            .happiness
            .has_recent_event(&HappinessEventType::UnhappyWithTacticalRole, 45)
        {
            return;
        }
        let base = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::UnhappyWithTacticalRole);
        let amb_mul = scaling::ambition_amplifier(self.attributes.ambition);
        let prof_dampen = scaling::criticism_dampener(self.attributes.professionalism);
        let count_factor = ((repeated_mismatch_count as f32 - 2.0) * 0.15).clamp(0.0, 0.6);
        let magnitude = base * amb_mul * prof_dampen * (1.0 + count_factor);

        let mut role_ctx = RoleStatusEventContext::new(RoleStatusKind::TacticalRoleChanged)
            .with_repeated_omissions(repeated_mismatch_count);
        if let Some(slot) = formation_slot {
            role_ctx = role_ctx.with_formation_slot(slot);
        }
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::TacticalDisagreement,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::MatchDay,
        )
        .with_role_status_context(role_ctx)
        .with_follow_up(HappinessEventFollowUp::ManagerInterventionRisk);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::UnhappyWithTacticalRole,
            magnitude,
            None,
            happiness_ctx,
            45,
        );
    }

    // ───────────────────────────────────────────────────────────
    // ThreatenedByNewSigning
    // ───────────────────────────────────────────────────────────

    /// React to a new signing arriving in this player's positional /
    /// status lane. Cooldown 90d so an aggressive transfer window
    /// doesn't produce a row for every overlap; the caller is expected
    /// to filter to direct competition before calling this. Magnitude
    /// scales by player's existing fragility (already fringe, older,
    /// already lacking minutes).
    pub fn on_new_signing_threat(&mut self, ctx: NewSigningThreatContext) {
        let rival_id = ctx.rival_player_id;
        let base = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::ThreatenedByNewSigning);
        let age_mul = match ctx.player_age {
            Some(a) if a >= 32 => 1.35,
            Some(a) if a >= 28 => 1.15,
            Some(a) if a <= 21 => 0.80,
            _ => 1.0,
        };
        let status_mul = match ctx.player_squad_status.clone() {
            Some(PlayerSquadStatus::KeyPlayer) => 0.80,
            Some(PlayerSquadStatus::FirstTeamRegular) => 0.95,
            Some(PlayerSquadStatus::FirstTeamSquadRotation) => 1.10,
            Some(PlayerSquadStatus::MainBackupPlayer) => 1.25,
            Some(PlayerSquadStatus::HotProspectForTheFuture) => 1.10,
            _ => 1.0,
        };
        let prof_dampen = scaling::criticism_dampener(self.attributes.professionalism);
        let amb_mul = scaling::ambition_amplifier(self.attributes.ambition);
        let magnitude = base * age_mul * status_mul * prof_dampen * amb_mul;

        let cause = match ctx.primary_reason {
            NewSigningThreatReason::SamePosition
            | NewSigningThreatReason::HigherAbility
            | NewSigningThreatReason::YoungerAndHighPotential => {
                HappinessEventCause::PositionalRivalry
            }
            NewSigningThreatReason::LargerWageDeal => HappinessEventCause::WageJealousy,
            NewSigningThreatReason::SimilarSquadStatus => HappinessEventCause::ReputationTension,
            NewSigningThreatReason::AlreadyFringe => HappinessEventCause::PoorFormPressure,
        };
        let happiness_ctx = HappinessEventContext::new(
            cause,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::DressingRoom,
        )
        .with_new_signing_threat_context(ctx)
        .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);
        // Per-rival cooldown so two different signings each surface their
        // own row, while a single rival doesn't refire weekly.
        self.happiness.add_event_with_partner_context_and_cooldown(
            HappinessEventType::ThreatenedByNewSigning,
            magnitude,
            rival_id,
            happiness_ctx,
            90,
        );
    }

    // ───────────────────────────────────────────────────────────
    // BenchedForBigMatch / TrustedInBigMatch
    // ───────────────────────────────────────────────────────────

    /// Manager picked the player for a major fixture. Stronger for
    /// young / fringe players and for those returning from poor form.
    /// Cooldown 30d so stars don't spam this every big night.
    pub fn on_trusted_in_big_match(&mut self, ctx: BigMatchSelectionContext) {
        debug_assert!(
            matches!(ctx.decision, BigMatchDecision::StartedTrusted),
            "TrustedInBigMatch requires StartedTrusted decision"
        );
        if self
            .happiness
            .has_recent_event(&HappinessEventType::TrustedInBigMatch, 30)
        {
            return;
        }
        let base = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::TrustedInBigMatch);
        let importance_mul = (0.7 + ctx.match_importance * 0.6).clamp(0.7, 1.4);
        let young_mul = if ctx.is_young_or_fringe { 1.4 } else { 1.0 };
        let captain_mul = if ctx.was_captain { 1.15 } else { 1.0 };
        let recovery_mul = if !ctx.recent_hot_form { 1.10 } else { 1.0 };
        let magnitude = base * importance_mul * young_mul * captain_mul * recovery_mul;

        let cause = HappinessEventCause::ManagerSupport;
        let mut happiness_ctx = HappinessEventContext::new(
            cause,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::MatchDay,
        )
        .with_big_match_selection_context(ctx)
        .with_follow_up(HappinessEventFollowUp::ManagerTrustRising);
        if self.attributes.pressure >= 15.0 {
            happiness_ctx =
                happiness_ctx.with_evidence(crate::HappinessEventEvidence::HighPressurePersonality);
        }
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::TrustedInBigMatch,
            magnitude,
            None,
            happiness_ctx,
            30,
        );
    }

    /// Manager dropped the player from the expected XI for a major
    /// fixture. Suppressed when the player was injured / suspended /
    /// rested — those go through their own paths.
    pub fn on_benched_for_big_match(&mut self, ctx: BigMatchSelectionContext) {
        debug_assert!(
            matches!(ctx.decision, BigMatchDecision::BenchedUnexpectedly),
            "BenchedForBigMatch requires BenchedUnexpectedly decision"
        );
        if self
            .happiness
            .has_recent_event(&HappinessEventType::BenchedForBigMatch, 30)
        {
            return;
        }
        let base = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::BenchedForBigMatch);
        let importance_mul = (0.7 + ctx.match_importance * 0.6).clamp(0.7, 1.5);
        let status_mul = match ctx.squad_status.clone() {
            Some(PlayerSquadStatus::KeyPlayer) => 1.45,
            Some(PlayerSquadStatus::FirstTeamRegular) => 1.25,
            Some(PlayerSquadStatus::FirstTeamSquadRotation) => 0.85,
            _ => 1.0,
        };
        let captain_mul = if ctx.was_captain { 1.20 } else { 1.0 };
        let form_mul = if ctx.recent_hot_form { 1.25 } else { 1.0 };
        let amb_mul = scaling::ambition_amplifier(self.attributes.ambition);
        let magnitude = base * importance_mul * status_mul * captain_mul * form_mul * amb_mul;

        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::LeadershipDispute,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::MatchDay,
        )
        .with_big_match_selection_context(ctx)
        .with_follow_up(HappinessEventFollowUp::DressingRoomDamageRisk);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::BenchedForBigMatch,
            magnitude,
            None,
            happiness_ctx,
            30,
        );
    }

    // ───────────────────────────────────────────────────────────
    // SubstitutionFrustration
    // ───────────────────────────────────────────────────────────

    /// React to a substitution the player didn't take well. Caller
    /// filters out injury / fatigue / red-card tactical responses
    /// before invoking. Cooldowned 21d.
    pub fn on_substitution_frustration(&mut self, ctx: SubstitutionFrustrationContext) {
        if self
            .happiness
            .has_recent_event(&HappinessEventType::SubstitutionFrustration, 21)
        {
            return;
        }
        let base = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::SubstitutionFrustration);
        let kind_mul = match ctx.kind {
            SubstitutionFrustrationKind::RepeatedEarlyHook => 1.20,
            SubstitutionFrustrationKind::HookedWhilePlayingWell => 1.35,
            SubstitutionFrustrationKind::RemovedInBigMatchEarly => 1.50,
            SubstitutionFrustrationKind::TacticalSwapResented => 1.0,
        };
        let early_hooks_mul = 1.0 + (ctx.recent_early_hooks as f32 * 0.10).min(0.40);
        let big_mul = if ctx.is_big_match { 1.20 } else { 1.0 };
        let prof_dampen = scaling::criticism_dampener(self.attributes.professionalism);
        let temperament_mul = 1.0 + ((20.0 - self.attributes.temperament.min(20.0)) / 20.0) * 0.25;
        let magnitude = base * kind_mul * early_hooks_mul * big_mul * prof_dampen * temperament_mul;

        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::TacticalDisagreement,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::MatchDay,
        )
        .with_substitution_frustration_context(ctx)
        .with_follow_up(HappinessEventFollowUp::ManagerInterventionRisk);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::SubstitutionFrustration,
            magnitude,
            None,
            happiness_ctx,
            21,
        );
    }

    // ───────────────────────────────────────────────────────────
    // InjurySetback
    // ───────────────────────────────────────────────────────────

    /// Fire when the player suffers a setback during recovery (reinjury,
    /// failed fitness test, recurrence concern). Distinct from
    /// `InjuryReturn`. The caller supplies the stage / severity; this
    /// is the visible wrapper. Cooldowned 30d so the same setback
    /// doesn't fire twice in the same week.
    pub fn on_injury_setback(
        &mut self,
        stage: InjuryRecoveryStage,
        recovery_days_total: u16,
        match_readiness: f32,
        recurrence_risk_high: bool,
    ) {
        debug_assert!(
            matches!(
                stage,
                InjuryRecoveryStage::RecoverySetback | InjuryRecoveryStage::InjuryRecurrenceConcern
            ),
            "InjurySetback requires a setback / recurrence stage"
        );
        if self
            .happiness
            .has_recent_event(&HappinessEventType::InjurySetback, 30)
        {
            return;
        }
        let base = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::InjurySetback);
        let stage_mul = match stage {
            InjuryRecoveryStage::InjuryRecurrenceConcern => 1.25,
            InjuryRecoveryStage::RecoverySetback => 1.10,
            _ => 1.0,
        };
        let layoff_mul = if recovery_days_total >= 90 {
            1.20
        } else if recovery_days_total >= 28 {
            1.0
        } else {
            0.85
        };
        let prof_dampen = scaling::criticism_dampener(self.attributes.professionalism);
        let magnitude = base * stage_mul * layoff_mul * prof_dampen;

        let mut injury_ctx =
            InjuryRecoveryEventContext::new(stage, recovery_days_total, match_readiness);
        if recovery_days_total >= 90 {
            injury_ctx = injury_ctx.with_evidence(InjuryRecoveryEvidence::LongTermLayoff);
        } else {
            injury_ctx = injury_ctx.with_evidence(InjuryRecoveryEvidence::ShortTermLayoff);
        }
        if recurrence_risk_high {
            injury_ctx = injury_ctx.with_evidence(InjuryRecoveryEvidence::PriorRecurringIssue);
        }
        if match_readiness < 0.5 {
            injury_ctx = injury_ctx.with_evidence(InjuryRecoveryEvidence::MatchSharpnessLow);
        }
        if self.happiness.is_established_starter {
            injury_ctx = injury_ctx.with_evidence(InjuryRecoveryEvidence::FearLosingPlace);
        }
        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::PoorFormPressure,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::Personal,
        )
        .with_injury_context(injury_ctx)
        .with_follow_up(HappinessEventFollowUp::ManagerInterventionRisk);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::InjurySetback,
            magnitude,
            None,
            happiness_ctx,
            30,
        );
    }

    // ───────────────────────────────────────────────────────────
    // ConcernedByClubDirection / EncouragedBySquadInvestment
    // ───────────────────────────────────────────────────────────

    /// Negative aggregate after a transfer window: sales unreplaced,
    /// squad weakened, board ambition concern. Stronger for ambitious
    /// / senior / influential players. Cooldowned 120d.
    pub fn on_club_direction_concern(&mut self, ctx: ClubDirectionContext) {
        debug_assert!(
            matches!(ctx.kind, ClubDirectionKind::Concern),
            "Use ConcernedByClubDirection emit path for the concern kind"
        );
        if self
            .happiness
            .has_recent_event(&HappinessEventType::ConcernedByClubDirection, 120)
        {
            return;
        }
        let base = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::ConcernedByClubDirection);
        let amb_mul = scaling::ambition_amplifier(self.attributes.ambition);
        let influence_signal = if ctx.evidence.contains(&ClubDirectionEvidence::HighInfluence) {
            1.15
        } else {
            1.0
        };
        let prof_dampen = scaling::criticism_dampener(self.attributes.professionalism);
        let magnitude = base * amb_mul * influence_signal * prof_dampen;

        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::LeadershipDispute,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::Boardroom,
        )
        .with_club_direction_context(ctx)
        .with_follow_up(HappinessEventFollowUp::ContractRequestRisk);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::ConcernedByClubDirection,
            magnitude,
            None,
            happiness_ctx,
            120,
        );
    }

    // ───────────────────────────────────────────────────────────
    // Substitution-frustration detector
    // ───────────────────────────────────────────────────────────

    /// Real substitution-frustration entry point. The league pipeline
    /// calls this for every player who was substituted off in a
    /// non-friendly via the `Discretionary` pass (critical injury /
    /// youth protection swaps are filtered out by the caller).
    ///
    /// Three flavours are recognised, in dominance order:
    /// * **HookedWhilePlayingWell** — minute ≥ 25 and rating ≥ 7.0.
    /// * **RemovedInBigMatchEarly** — derby / cup-final / continental
    ///   knockout fixture, hooked before minute 75.
    /// * **RepeatedEarlyHook** — three or more early substitutions
    ///   (minute < 70) in the last 30 days; the recent-count drives
    ///   the kind regardless of the current match flavour.
    ///
    /// Returns silently for normal late substitutions (≥ minute 80) or
    /// when the player had a poor rating — those reads as routine
    /// fatigue / tactical-reshuffle calls, not a snub.
    pub fn on_match_substituted_for_frustration(
        &mut self,
        minute_off: u8,
        rating_when_off: f32,
        is_big_match: bool,
    ) {
        if minute_off >= 80 {
            return;
        }
        if rating_when_off < 6.4 {
            return;
        }
        // New-manager review window: in the first weeks after a
        // managerial change every player expects to be tried, hooked,
        // and shuffled — a single early hook under the new man isn't a
        // snub yet. Repeated hooks still qualify below once they
        // accumulate past the window.
        if self
            .happiness
            .has_recent_event(&HappinessEventType::NewManagerBounce, 56)
            && rating_when_off < 7.5
        {
            return;
        }
        let recent_early_hooks = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                e.event_type == HappinessEventType::SubstitutionFrustration && e.days_ago <= 30
            })
            .count()
            .min(u8::MAX as usize) as u8;

        let kind = if recent_early_hooks >= 2 && minute_off < 70 {
            SubstitutionFrustrationKind::RepeatedEarlyHook
        } else if is_big_match && minute_off < 75 {
            SubstitutionFrustrationKind::RemovedInBigMatchEarly
        } else if minute_off >= 25 && rating_when_off >= 7.0 {
            SubstitutionFrustrationKind::HookedWhilePlayingWell
        } else {
            // Late routine hook with mediocre rating — no event.
            return;
        };

        let mut ctx = SubstitutionFrustrationContext::new(kind)
            .with_minute(minute_off)
            .with_match_rating(rating_when_off)
            .with_recent_early_hooks(recent_early_hooks)
            .with_big_match(is_big_match);
        // Renderer reads kind first, but a final routing safety pass
        // re-tags the explicit big-match overlap so a "playing well"
        // hook in a final doesn't read as routine.
        if is_big_match && minute_off < 75 && rating_when_off >= 7.0 {
            ctx = ctx.with_match_rating(rating_when_off);
        }
        self.on_substitution_frustration(ctx);
    }

    // ───────────────────────────────────────────────────────────
    // Selection-driven big-match + tactical-role detectors
    // ───────────────────────────────────────────────────────────

    /// Fire `BenchedForBigMatch` when an expected starter was dropped
    /// for a high-importance fixture. Gated to skip rest, returning-
    /// from-injury, suspension, and low-importance rotations — those
    /// are protective / routine calls, not a snub.
    pub fn maybe_emit_big_match_bench(&mut self, ctx: &MatchSelectionContext) {
        if ctx.is_friendly {
            return;
        }
        // New-manager review window: the new man picking his own big-
        // match XI is expected squad-assessment, not a demotion story.
        if self
            .happiness
            .has_recent_event(&HappinessEventType::NewManagerBounce, 56)
        {
            return;
        }
        // Protective scopes / reasons are explicitly NOT a "benched for
        // a big match" story.
        if matches!(
            ctx.scope,
            SelectionDecisionScope::Rested | SelectionDecisionScope::UnavailableButNotInjured
        ) {
            return;
        }
        if matches!(
            ctx.reason,
            SelectionOmissionReason::FitnessProtection
                | SelectionOmissionReason::FatigueManagement
                | SelectionOmissionReason::ReturningFromInjury
                | SelectionOmissionReason::DisciplinarySelection
                | SelectionOmissionReason::YouthDevelopmentRotation
                | SelectionOmissionReason::NewcomerStillIntegrating
                | SelectionOmissionReason::CupRotation
                | SelectionOmissionReason::LowMatchImportanceRotation
        ) {
            return;
        }
        if self.player_attributes.is_injured {
            return;
        }
        // Only an expected starter is "benched" — a fringe rotation
        // player being left out of a final isn't headline news.
        let status = self
            .contract
            .as_ref()
            .map(|c| c.squad_status.clone())
            .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
        let expects_to_start = matches!(
            status,
            PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
        ) || self.happiness.is_established_starter;
        if !expects_to_start {
            return;
        }
        // Match-importance gate: cup-final / continental-knockout
        // contexts ride at ~1.0; an "important" league weekend (0.8)
        // is NOT a big match — derbies and final rounds clear 0.9+.
        // Set the bar so a routine match-importance value never trips
        // the bench-for-big-match event by accident.
        if ctx.match_importance < 0.90 {
            return;
        }
        // Recent hot form bumps the magnitude — defying form is a
        // bigger blow than dropping a player on a poor stretch.
        let hot_form = self.load.form_rating >= 7.2;
        let big_ctx = BigMatchSelectionContext::new(
            // Without an explicit BigMatchKind from the caller, fall
            // back to the most generic high-stakes label. The renderer
            // distinguishes derby / cup-final via the selection
            // context's role + importance rather than guessing here.
            BigMatchKind::TitleDecider,
            BigMatchDecision::BenchedUnexpectedly,
        )
        .with_squad_status(status)
        .with_hot_form(hot_form)
        .with_match_importance(ctx.match_importance);
        self.on_benched_for_big_match(big_ctx);
    }

    /// Fire `TrustedInBigMatch` when the player starts in a fixture
    /// the engine recognises as "big" (derby / cup final / continental
    /// knockout / late-stage cup). Skips routine league rounds and
    /// substitute appearances — coming off the bench is its own
    /// `SubstituteImpact` event already.
    pub fn maybe_emit_big_match_trust(&mut self, o: &MatchOutcome<'_>) {
        let Some(kind) = o.big_match_kind() else {
            return;
        };
        if !matches!(o.participation, MatchParticipation::Starter) {
            return;
        }
        let status = self
            .contract
            .as_ref()
            .map(|c| c.squad_status.clone())
            .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
        // Fringe / development-tier players feel this more — a 31-year-
        // old key player starting a derby is expected, while a fringe
        // rotation player or a HotProspect getting the nod reads as the
        // manager backing them. We can't cheaply derive age inside this
        // helper without threading `now: NaiveDate` through every
        // `on_match_played` call site; squad status is the proxy.
        let is_young_or_fringe = matches!(
            status,
            PlayerSquadStatus::MainBackupPlayer
                | PlayerSquadStatus::FirstTeamSquadRotation
                | PlayerSquadStatus::HotProspectForTheFuture
                | PlayerSquadStatus::DecentYoungster
        );
        let hot_form = self.load.form_rating >= 7.0;
        let mut big_ctx = BigMatchSelectionContext::new(kind, BigMatchDecision::StartedTrusted)
            .with_squad_status(status)
            .with_young_or_fringe(is_young_or_fringe)
            .with_hot_form(hot_form)
            .with_match_importance(if o.is_continental || o.is_derby {
                1.0
            } else {
                0.85
            });
        if let Some(opp) = o.opponent_team_id {
            big_ctx = big_ctx.with_opponent(opp);
        }
        self.on_trusted_in_big_match(big_ctx);
    }

    /// Aggregator that fires `UnhappyWithTacticalRole` once the recent
    /// drop history shows the player being used / overlooked for
    /// tactical-fit reasons repeatedly. The per-call cooldown lives in
    /// `on_tactical_role_mismatch`; this method counts the recent
    /// signal and triggers on the third occurrence in a 90-day window.
    pub fn maybe_emit_tactical_role_mismatch(&mut self, ctx: &MatchSelectionContext) {
        // Only repeated tactical-flavour omissions count toward this
        // signal. A bench-balance call or fatigue rotation is unrelated.
        let counts = matches!(
            ctx.reason,
            SelectionOmissionReason::TacticalMismatch
                | SelectionOmissionReason::PositionFitIssue
                | SelectionOmissionReason::NoNaturalRoleInFormation
                | SelectionOmissionReason::TeammatePreferredForTacticalBalance
        );
        if !counts {
            return;
        }
        // Count recent tactical-flavour drops in the last 90 days,
        // including this one (it was just pushed).
        let recent = self
            .happiness
            .recent_events
            .iter()
            .filter(|e| {
                if e.event_type != HappinessEventType::MatchDropped || e.days_ago > 90 {
                    return false;
                }
                let r = e
                    .context
                    .as_ref()
                    .and_then(|c| c.selection_context.as_ref())
                    .map(|sc| sc.reason);
                matches!(
                    r,
                    Some(SelectionOmissionReason::TacticalMismatch)
                        | Some(SelectionOmissionReason::PositionFitIssue)
                        | Some(SelectionOmissionReason::NoNaturalRoleInFormation)
                        | Some(SelectionOmissionReason::TeammatePreferredForTacticalBalance)
                )
            })
            .count();
        let slot = Some(ctx.role);
        // Reuse the public emit path — it owns the magnitude / cooldown
        // / context wiring. The `>= 3` gate inside that function makes
        // this method's `recent` count load-bearing.
        self.on_tactical_role_mismatch(recent.min(u8::MAX as usize) as u8, slot);
    }

    /// Positive aggregate after a transfer window: meaningful signing,
    /// squad quality up, board invested visibly.
    pub fn on_club_direction_encouragement(&mut self, ctx: ClubDirectionContext) {
        debug_assert!(
            matches!(ctx.kind, ClubDirectionKind::Encouragement),
            "Use EncouragedBySquadInvestment emit path for encouragement kind"
        );
        if self
            .happiness
            .has_recent_event(&HappinessEventType::EncouragedBySquadInvestment, 120)
        {
            return;
        }
        let base = HappinessConfig::default()
            .catalog
            .magnitude(HappinessEventType::EncouragedBySquadInvestment);
        let amb_mul = scaling::ambition_amplifier(self.attributes.ambition);
        let prior_concern_mul = if self
            .happiness
            .has_recent_event(&HappinessEventType::WantsStrongerSquad, 180)
            || self
                .happiness
                .has_recent_event(&HappinessEventType::ConcernedByClubDirection, 180)
        {
            1.30
        } else {
            1.0
        };
        let magnitude = base * amb_mul * prior_concern_mul;

        let happiness_ctx = HappinessEventContext::new(
            HappinessEventCause::ReputationAdmiration,
            HappinessEventSeverity::from_magnitude(magnitude),
            HappinessEventScope::Boardroom,
        )
        .with_club_direction_context(ctx)
        .with_follow_up(HappinessEventFollowUp::TrendImproving);
        self.happiness.add_event_with_context_and_cooldown(
            HappinessEventType::EncouragedBySquadInvestment,
            magnitude,
            None,
            happiness_ctx,
            120,
        );
    }
}

/// Map a private-talk reason to the right top-level `HappinessEventCause`
/// label. Drives the cause i18n key on the rendered row.
fn cause_for_private_talk(reason: PrivateTalkReason) -> HappinessEventCause {
    match reason {
        PrivateTalkReason::PlayingTime => HappinessEventCause::PoorFormPressure,
        PrivateTalkReason::Contract => HappinessEventCause::WageJealousy,
        PrivateTalkReason::TransferStatus => HappinessEventCause::ReputationTension,
        PrivateTalkReason::CaptaincyOrStatus => HappinessEventCause::LeadershipDispute,
        PrivateTalkReason::TacticalRole => HappinessEventCause::TacticalDisagreement,
        PrivateTalkReason::ManagerRelationship => HappinessEventCause::LeadershipDispute,
    }
}

/// Big-match kind inference helper for callers that don't know which
/// `BigMatchKind` fits. Designed so a single import gets the typical
/// derby / cup / continental knockout / league-decider mapping right.
pub struct BigMatchClassifier;

impl BigMatchClassifier {
    /// Pick the closest `BigMatchKind` for a fixture. `is_cup` /
    /// `is_continental` etc. come from the league pipeline. Returns
    /// `None` when the fixture is not high enough importance to count
    /// as a big match.
    pub fn classify(
        is_derby: bool,
        is_cup_final: bool,
        is_continental_knockout: bool,
        is_cup_semi_or_later: bool,
        is_title_decider: bool,
        is_promotion_decider: bool,
        is_relegation_decider: bool,
    ) -> Option<BigMatchKind> {
        if is_cup_final {
            return Some(BigMatchKind::CupFinal);
        }
        if is_continental_knockout {
            return Some(BigMatchKind::ContinentalKnockout);
        }
        if is_title_decider {
            return Some(BigMatchKind::TitleDecider);
        }
        if is_promotion_decider {
            return Some(BigMatchKind::PromotionDecider);
        }
        if is_relegation_decider {
            return Some(BigMatchKind::RelegationDecider);
        }
        if is_cup_semi_or_later {
            return Some(BigMatchKind::NationalCupSemiOrLater);
        }
        if is_derby {
            return Some(BigMatchKind::Derby);
        }
        None
    }
}

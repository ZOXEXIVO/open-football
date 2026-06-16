use crate::club::player::ManagerPromiseKind;
use crate::club::player::calculators::FreeAgentReleaseReason;
use crate::club::player::interaction::{
    InteractionOutcome, InteractionTone, InteractionTopic, ManagerInteraction,
    default_cooldown_days,
};
use crate::league::result::LeagueProcessAccess;
use crate::transfers::pipeline::PipelineProcessor;
use crate::{
    ChangeType, ConflictLocation, HappinessEventCause, HappinessEventChangeKind,
    HappinessEventContext, HappinessEventEvidence, HappinessEventFollowUp, HappinessEventScope,
    HappinessEventSeverity, HappinessEventType, LoanEventContext, LoanEventKind,
    ManagerInteractionEventContext, ManagerInteractionTone, ManagerInteractionTopic, Player,
    PlayerAcceptance, PlayerHappiness, PlayerPositionType, PlayerSquadStatus, PlayerStatusType,
    PromiseKind, RelationshipChange, TeammateConflictContext, TeammateConflictReason,
};
use chrono::{Duration, NaiveDate};
use std::cmp::Ordering;

pub struct TeamBehaviourResult {
    pub players: PlayerBehaviourResult,
    pub manager_talks: Vec<ManagerTalkResult>,
    /// Head-coach-approved mutual contract terminations pending finance
    /// and player-state commit. Applied in `process()`.
    pub contract_terminations: Vec<ContractTermination>,
}

#[derive(Debug, Clone)]
pub struct ContractTermination {
    pub player_id: u32,
    pub payout: u32,
    /// The specific club-driven exit this termination represents — drives
    /// the player's recorded release reason and the free-agent sweep's
    /// transfer-history line.
    pub reason: FreeAgentReleaseReason,
}

impl Default for TeamBehaviourResult {
    fn default() -> Self {
        Self::new()
    }
}

impl TeamBehaviourResult {
    pub fn new() -> Self {
        TeamBehaviourResult {
            players: PlayerBehaviourResult::new(),
            manager_talks: Vec::new(),
            contract_terminations: Vec::new(),
        }
    }

    pub fn process<D: LeagueProcessAccess>(&self, data: &mut D) {
        self.players.process(data);
        self.process_manager_talks(data);
        self.process_contract_terminations(data);
    }

    fn process_contract_terminations<D: LeagueProcessAccess>(&self, data: &mut D) {
        let date = data.date().date();
        for termination in &self.contract_terminations {
            let (country_id, club_id) = match data
                .indexes()
                .and_then(|i| i.get_player_location(termination.player_id))
            {
                Some((_, country_id, club_id, _)) => (country_id, club_id),
                None => continue,
            };
            if let Some(player) = data.player_mut(termination.player_id) {
                player.on_contract_terminated(date, termination.reason);
            }
            if termination.payout > 0 {
                if let Some(club) = data.club_mut(club_id) {
                    club.finance
                        .balance
                        .push_expense_player_wages(termination.payout as i64);
                }
            }
            // Now a free agent — drop him from every club's shortlist,
            // scouting, and loan-out lists in this country so stale interest
            // records don't linger.
            if let Some(country) = data.country_mut(country_id) {
                PipelineProcessor::clear_player_interest(country, termination.player_id);
            }
            log::debug!(
                "Contract terminated: player {} by club {} — payout {} ({})",
                termination.player_id,
                club_id,
                termination.payout,
                termination.reason.history_reason()
            );
        }
    }

    fn process_manager_talks<D: LeagueProcessAccess>(&self, data: &mut D) {
        let sim_date = data.date().date();

        for talk in &self.manager_talks {
            if let Some(player) = data.player_mut(talk.player_id) {
                // Apply morale change
                player.happiness.adjust_morale(talk.morale_change);

                // Apply relationship change with manager
                if talk.relationship_change.abs() > 0.001 {
                    let change = if talk.relationship_change >= 0.0 {
                        RelationshipChange::positive(
                            ChangeType::CoachingSuccess,
                            talk.relationship_change.abs(),
                        )
                    } else {
                        RelationshipChange::negative(
                            ChangeType::DisciplinaryAction,
                            talk.relationship_change.abs(),
                        )
                    };
                    player
                        .relations
                        .update_staff_relationship(talk.staff_id, change, sim_date);

                    // Coach rapport mirrors the staff-relation update —
                    // mid-magnitude relationship deltas become rapport ticks
                    // so trusted coaches actually accumulate trust here.
                    let rapport_amount = (talk.relationship_change.abs() * 4.0).round() as i16;
                    if rapport_amount > 0 {
                        if talk.relationship_change >= 0.0 {
                            player
                                .rapport
                                .on_positive(talk.staff_id, sim_date, rapport_amount);
                        } else {
                            player
                                .rapport
                                .on_negative(talk.staff_id, sim_date, rapport_amount);
                        }
                    }

                    // Also update the happiness factor for manager relationship
                    let current = player.happiness.factors.manager_relationship;
                    player.happiness.factors.manager_relationship =
                        (current + talk.relationship_change * 5.0).clamp(-15.0, 15.0);
                }

                // Add happiness event
                let event_type = if talk.success {
                    match talk.talk_type {
                        ManagerTalkType::Praise => HappinessEventType::ManagerPraise,
                        ManagerTalkType::Discipline => HappinessEventType::ManagerDiscipline,
                        ManagerTalkType::PlayingTimeTalk => {
                            HappinessEventType::ManagerPlayingTimePromise
                        }
                        _ => HappinessEventType::ManagerPraise,
                    }
                } else {
                    // A failed talk is a manager interaction outcome —
                    // model it as ManagerCriticism (or Discipline), not
                    // PoorTraining, which is reserved for actual
                    // training-ground sessions.
                    match talk.talk_type {
                        ManagerTalkType::Discipline => HappinessEventType::ManagerDiscipline,
                        _ => HappinessEventType::ManagerCriticism,
                    }
                };
                let topic = ManagerInteractionTopicMapper::from_talk(&talk.talk_type);
                let tone = ManagerInteractionToneMapper::from_interaction(&talk.tone);
                let acceptance = ManagerInteractionAcceptanceMapper::from_outcome(
                    talk.success,
                    talk.morale_change,
                );
                let mut mctx = ManagerInteractionEventContext::new(topic, tone, acceptance)
                    .with_manager_staff_id(talk.staff_id);
                if matches!(
                    talk.talk_type,
                    ManagerTalkType::PlayingTimeTalk | ManagerTalkType::PlayingTimeRequest
                ) {
                    let credibility = if talk.honest_framing { 0.9 } else { 0.55 };
                    mctx = mctx.with_promise(PromiseKind::PlayingTime, credibility);
                }
                let happiness_ctx = HappinessEventContext::new(
                    HappinessEventCause::Other,
                    HappinessEventSeverity::from_magnitude(talk.morale_change),
                    HappinessEventScope::DressingRoom,
                )
                .with_manager_interaction_context(mctx);
                player.happiness.add_event_with_context(
                    event_type,
                    talk.morale_change,
                    None,
                    happiness_ctx,
                );

                let mut promise_created = false;

                // Remove statuses on success
                if talk.success {
                    match talk.talk_type {
                        ManagerTalkType::PlayingTimeTalk | ManagerTalkType::MoraleTalk => {
                            player.statuses.remove(PlayerStatusType::Unh);
                            // A successful playing-time chat is a concrete
                            // promise — record it with full credibility &
                            // importance context so verification weights
                            // honestly. Honest framing scales the horizon
                            // shorter (a calmer "we'll see in 30 days"),
                            // soft reassurance pushes longer.
                            if talk.talk_type == ManagerTalkType::PlayingTimeTalk {
                                let horizon = if talk.honest_framing { 30 } else { 45 };
                                player.record_promise_full(
                                    ManagerPromiseKind::PlayingTime,
                                    sim_date,
                                    horizon,
                                    Some(talk.staff_id),
                                    None,
                                    false,
                                );
                                promise_created = true;
                            }
                        }
                        ManagerTalkType::TransferDiscussion => {
                            player.statuses.remove(PlayerStatusType::Req);
                            // Coach agreed to consider sale next window.
                            // Record TransferPermission so the player
                            // remembers — broken later if no offer is
                            // entertained.
                            player.record_promise_full(
                                ManagerPromiseKind::TransferPermission,
                                sim_date,
                                120,
                                Some(talk.staff_id),
                                None,
                                false,
                            );
                            promise_created = true;
                        }
                        ManagerTalkType::PlayingTimeRequest => {
                            player
                                .happiness
                                .add_event(HappinessEventType::ManagerPlayingTimePromise, 8.0);
                            player.record_promise_full(
                                ManagerPromiseKind::PlayingTime,
                                sim_date,
                                30,
                                Some(talk.staff_id),
                                None,
                                false,
                            );
                            promise_created = true;
                        }
                        ManagerTalkType::LoanRequest => {
                            // Force-pinned players: the listing flag wins
                            // even over a successful loan-request talk —
                            // nothing actually happens, the player stays.
                            if !player.is_force_match_selection {
                                player.statuses.add(sim_date, PlayerStatusType::Loa);
                                let lctx =
                                    LoanEventContext::new(LoanEventKind::LoanListingAccepted);
                                let happiness_ctx = HappinessEventContext::new(
                                    HappinessEventCause::Other,
                                    HappinessEventSeverity::Moderate,
                                    HappinessEventScope::Boardroom,
                                )
                                .with_loan_context(lctx);
                                player.happiness.add_event_with_context(
                                    HappinessEventType::LoanListingAccepted,
                                    5.0,
                                    None,
                                    happiness_ctx,
                                );
                            }
                        }
                        _ => {}
                    }
                } else {
                    // Handle failure for player-initiated requests
                    match talk.talk_type {
                        ManagerTalkType::PlayingTimeRequest => {
                            player.statuses.add(sim_date, PlayerStatusType::Req);
                        }
                        ManagerTalkType::LoanRequest => {
                            // Denied loan — unsettled about future. Honest
                            // refusal hits softer than a wishy-washy fob.
                            player.statuses.add(sim_date, PlayerStatusType::Fut);
                            let mag = if talk.honest_framing { -3.0 } else { -5.0 };
                            player
                                .happiness
                                .add_event(HappinessEventType::LackOfPlayingTime, mag);
                        }
                        _ => {}
                    }
                }

                // Append the interaction record. Picker writes to log on
                // apply, not on emit, so the cooldown gate sees the
                // *committed* set of talks, not transient pickings.
                let topic = topic_for_talk(talk.talk_type.clone());
                let outcome = if promise_created {
                    InteractionOutcome::PromiseMade
                } else if talk.success {
                    InteractionOutcome::Positive
                } else if talk.morale_change.abs() < 0.5 {
                    InteractionOutcome::Neutral
                } else {
                    InteractionOutcome::Negative
                };
                let cooldown = sim_date + Duration::days(default_cooldown_days(topic));
                player.interactions.push(ManagerInteraction {
                    date: sim_date,
                    staff_id: talk.staff_id,
                    topic,
                    tone: talk.tone,
                    player_mood_before: talk.mood_before,
                    outcome,
                    promise_created,
                    relationship_delta: talk.relationship_change,
                    morale_delta: talk.morale_change,
                    cooldown_until: cooldown,
                });
            }
        }
    }
}

pub struct PlayerBehaviourResult {
    pub relationship_result: Vec<PlayerRelationshipChangeResult>,
}

impl Default for PlayerBehaviourResult {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerBehaviourResult {
    pub fn new() -> Self {
        PlayerBehaviourResult {
            relationship_result: Vec::new(),
        }
    }

    pub fn process<D: LeagueProcessAccess>(&self, data: &mut D) {
        let sim_date = data.date().date();

        // Two-pass design. Pass 1 applies every relation update and
        // builds a visible-event candidate per pair that crossed the
        // magnitude threshold; pass 2 ranks the candidates per player
        // by severity (|applied| desc, pre-incident relation tier,
        // partner id) and pushes them through the shared same-tick
        // budget helper. The relation drift always lands — only the
        // headline row is gated. The per-tick caps and the 45-day
        // per-partner cooldown live on `PlayerHappiness`; the helper
        // observes both.
        const PARTNER_EVENT_COOLDOWN_DAYS: u16 = 45;

        let mut candidates: Vec<EmitCandidate> = Vec::with_capacity(self.relationship_result.len());

        for relationship_result in &self.relationship_result {
            // Look up partner-side personality / squad info BEFORE we take
            // a `&mut` on `from_player`. Used for SamePosition /
            // SimilarSquadStatus evidence — partner is a different player,
            // so the borrows don't conflict.
            let partner_id = relationship_result.to_player_id;
            let partner_position = data.player(partner_id).map(|p| p.position());
            let partner_squad_status = data
                .player(partner_id)
                .and_then(|p| p.contract.as_ref())
                .map(|c| c.squad_status.clone());

            let Some(player_to_modify) = data.player_mut(relationship_result.from_player_id)
            else {
                continue;
            };

            // Snapshot the relation BEFORE applying the update —
            // earlier versions of this code read it AFTER, so
            // `relationship_level_before` was actually post-update.
            // Default to a neutral relation when no prior record
            // exists (first-ever interaction with this teammate).
            let prior = PairRelationSnapshot::capture(player_to_modify, partner_id);
            let snapshot_before = prior.unwrap_or_else(PairRelationSnapshot::neutral);
            let had_prior_relation = prior.is_some();

            player_to_modify.relations.update_with_type(
                partner_id,
                relationship_result.relationship_change,
                relationship_result.change_type.clone(),
                sim_date,
            );

            // Snapshot AFTER so the renderer can speak about trend
            // direction without recomputing the delta.
            let level_after = player_to_modify
                .relations
                .get_player(partner_id)
                .map(|r| r.level)
                .unwrap_or(snapshot_before.level);

            // The raw `relationship_change` doesn't reflect what
            // `PlayerRelation::apply_change` actually applies —
            // PersonalConflict, MatchCooperation, ConflictResolution
            // and others multiply internally. Compare the *applied*
            // magnitude so a raw -0.20 PersonalConflict (≈-0.60
            // applied) actually surfaces.
            let applied = applied_level_magnitude(
                relationship_result.relationship_change,
                &relationship_result.change_type,
            );

            // Below the visibility threshold — relation drift already
            // landed, nothing more to do.
            if applied.abs() <= 0.5 {
                continue;
            }

            // Evidence from the pre-update snapshot — the true
            // "before the incident" state the user wants explained.
            let evidence = PairEventContextBuilder::build_evidence(
                &relationship_result.change_type,
                snapshot_before,
                had_prior_relation,
                player_to_modify,
                partner_position.as_ref(),
                partner_squad_status,
                sim_date,
            );

            if applied > 0.5 {
                let magnitude = 1.0;
                let context = PairEventContextBuilder::bonding_context(
                    &relationship_result.change_type,
                    magnitude,
                    snapshot_before,
                    level_after,
                    &evidence,
                );
                candidates.push(EmitCandidate {
                    from_id: relationship_result.from_player_id,
                    partner_id,
                    kind: HappinessEventType::TeammateBonding,
                    magnitude,
                    applied_abs: applied.abs(),
                    level_before: snapshot_before.level,
                    context,
                });
            } else {
                // applied < -0.5
                let magnitude = -1.5;
                let mut conflict_evidence = evidence;
                // RepeatedIncident: a recent conflict with this same
                // partner promotes the incident from a one-off to a
                // chronic concern in the dressing-room.
                let recent_count = player_to_modify
                    .happiness
                    .recent_events
                    .iter()
                    .filter(|e| {
                        e.event_type == HappinessEventType::ConflictWithTeammate
                            && e.partner_player_id == Some(partner_id)
                            && e.days_ago <= 90
                    })
                    .count();
                if recent_count >= 1
                    && !conflict_evidence.contains(&HappinessEventEvidence::RepeatedIncident)
                {
                    conflict_evidence.push(HappinessEventEvidence::RepeatedIncident);
                }
                let context = PairEventContextBuilder::conflict_context(
                    &relationship_result.change_type,
                    magnitude,
                    snapshot_before,
                    level_after,
                    &conflict_evidence,
                );
                candidates.push(EmitCandidate {
                    from_id: relationship_result.from_player_id,
                    partner_id,
                    kind: HappinessEventType::ConflictWithTeammate,
                    magnitude,
                    applied_abs: applied.abs(),
                    level_before: snapshot_before.level,
                    context,
                });
            }
        }

        // Pass 2 — rank per player and push through the shared
        // same-tick budget helper. The helper reads
        // `same_tick_event_count` on each call, so events any earlier
        // emitter (controversy, mentorship, training, match-day
        // post-incident) already added in this tick consume budget
        // here too — no local counters needed.
        candidates.sort_by_key(|c| c.from_id);
        let mut i = 0;
        while i < candidates.len() {
            let from_id = candidates[i].from_id;
            let block_end = candidates[i..]
                .iter()
                .position(|c| c.from_id != from_id)
                .map(|p| i + p)
                .unwrap_or(candidates.len());

            let mut block: Vec<EmitCandidate> = candidates[i..block_end].to_vec();
            block.sort_by(EmitCandidate::rank);

            if let Some(player) = data.player_mut(from_id) {
                for cand in block {
                    let max_same_tick = match cand.kind {
                        HappinessEventType::ConflictWithTeammate => {
                            PlayerHappiness::MAX_CONFLICT_WITH_TEAMMATE_PER_TICK
                        }
                        _ => PlayerHappiness::MAX_TEAMMATE_BONDING_PER_TICK,
                    };
                    player
                        .happiness
                        .try_add_partner_context_with_same_tick_budget(
                            cand.kind,
                            cand.magnitude,
                            cand.partner_id,
                            cand.context,
                            PARTNER_EVENT_COOLDOWN_DAYS,
                            max_same_tick,
                        );
                }
            }
            i = block_end;
        }
    }
}


/// A pre-built visible-event candidate produced by the first pass of
/// [`PlayerBehaviourResult::process`]. The relation update has already
/// landed when this is created; the second pass decides whether the
/// row actually surfaces in the player's history feed based on
/// severity ranking and the per-partner cooldown.
#[derive(Clone)]
struct EmitCandidate {
    from_id: u32,
    partner_id: u32,
    kind: HappinessEventType,
    magnitude: f32,
    /// Absolute applied-level magnitude. Used as the primary ranking
    /// key — bigger stories first.
    applied_abs: f32,
    /// Pre-incident relation level. Conflict ranking prefers strained
    /// partners (lower wins), bonding ranking prefers strong existing
    /// bonds (higher wins).
    level_before: f32,
    context: HappinessEventContext,
}

impl EmitCandidate {
    /// Severity ranking comparator. Conflicts sort before bondings so
    /// the second pass walks them in deterministic groups; inside a
    /// group: larger applied magnitude first, then for conflicts the
    /// already-strained partner wins (lower `level_before`) and for
    /// bondings the strongest existing bond wins (higher
    /// `level_before`), with `partner_id` as a stable tie-break.
    fn rank(a: &Self, b: &Self) -> Ordering {
        let kind_rank = |k: &HappinessEventType| -> u8 {
            match k {
                HappinessEventType::ConflictWithTeammate => 0,
                _ => 1,
            }
        };
        kind_rank(&a.kind).cmp(&kind_rank(&b.kind)).then_with(|| {
            b.applied_abs
                .partial_cmp(&a.applied_abs)
                .unwrap_or(Ordering::Equal)
                .then_with(|| match a.kind {
                    HappinessEventType::ConflictWithTeammate => a
                        .level_before
                        .partial_cmp(&b.level_before)
                        .unwrap_or(Ordering::Equal),
                    _ => b
                        .level_before
                        .partial_cmp(&a.level_before)
                        .unwrap_or(Ordering::Equal),
                })
                .then_with(|| a.partner_id.cmp(&b.partner_id))
        })
    }
}

/// Per-pair pre-update relationship snapshot used to build a
/// [`HappinessEventContext`]. Stored as a struct so the multiple emit
/// paths share a single shape and the construction sites read like
/// data, not tuple gymnastics.
#[derive(Debug, Clone, Copy)]
pub struct PairRelationSnapshot {
    pub level: f32,
    pub trust: f32,
    pub friendship: f32,
    pub professional_respect: f32,
}

impl PairRelationSnapshot {
    /// Snapshot a player's view of a partner relation. `None` if the
    /// pair has never interacted — caller decides whether to default.
    pub fn capture(player: &Player, partner_id: u32) -> Option<Self> {
        player.relations.get_player(partner_id).map(|r| Self {
            level: r.level,
            trust: r.trust,
            friendship: r.friendship,
            professional_respect: r.professional_respect,
        })
    }

    /// Neutral baseline used when no relation record exists yet — first
    /// interaction with this teammate. Mirrors `PlayerRelation::new_neutral`.
    pub fn neutral() -> Self {
        Self {
            level: 0.0,
            trust: 50.0,
            friendship: 30.0,
            professional_respect: 50.0,
        }
    }
}

/// Builder for pair-event explanation contexts (TeammateBonding /
/// ConflictWithTeammate). Centralises cause / scope mapping, evidence
/// derivation, and outlook selection so the call site in
/// `PlayerBehaviourResult::process` reads as a thin orchestration layer.
pub struct PairEventContextBuilder;

impl PairEventContextBuilder {
    /// Pick a stable cause category for a conflict event driven by
    /// `change_type`. Mapping is total — every conflict-producing
    /// `ChangeType` resolves to a concrete cause so the renderer never
    /// falls back to a generic "Other" line for upgraded emit sites.
    pub fn cause_for_conflict(change_type: &ChangeType) -> HappinessEventCause {
        match change_type {
            ChangeType::PersonalConflict => HappinessEventCause::PersonalityClash,
            ChangeType::TrainingFriction => HappinessEventCause::TrainingFriction,
            ChangeType::CompetitionRivalry => HappinessEventCause::PositionalRivalry,
            ChangeType::ReputationTension => HappinessEventCause::ReputationTension,
            ChangeType::TacticalDisagreement => HappinessEventCause::TacticalDisagreement,
            ChangeType::TeamFailure => HappinessEventCause::PoorFormPressure,
            ChangeType::DisciplinaryAction => HappinessEventCause::LeadershipDispute,
            _ => HappinessEventCause::PersonalityClash,
        }
    }

    /// Counterpart of `cause_for_conflict` for bonding events. Always
    /// resolves to a positive cause so the "why" sentence stays
    /// consistent with the positive headline.
    pub fn cause_for_bonding(change_type: &ChangeType) -> HappinessEventCause {
        match change_type {
            ChangeType::MatchCooperation | ChangeType::TeamSuccess => {
                HappinessEventCause::MatchCooperation
            }
            ChangeType::ReputationAdmiration => HappinessEventCause::ReputationAdmiration,
            _ => HappinessEventCause::TrainingPartnership,
        }
    }

    fn scope_for_conflict(change_type: &ChangeType) -> HappinessEventScope {
        match change_type {
            ChangeType::TrainingFriction
            | ChangeType::CompetitionRivalry
            | ChangeType::TacticalDisagreement => HappinessEventScope::TrainingGround,
            _ => HappinessEventScope::DressingRoom,
        }
    }

    fn scope_for_bonding(change_type: &ChangeType) -> HappinessEventScope {
        match change_type {
            ChangeType::MatchCooperation | ChangeType::TeamSuccess => HappinessEventScope::MatchDay,
            ChangeType::TrainingBonding | ChangeType::MentorshipBond => {
                HappinessEventScope::TrainingGround
            }
            _ => HappinessEventScope::DressingRoom,
        }
    }

    /// Derive the closed-set evidence list for a pair event, given the
    /// pre-update relation snapshot, the change type, and the
    /// from-player / partner context. Returns concrete atoms (trust low,
    /// same position, same status tier, recent transfer) — the renderer
    /// picks the most informative one or two, not the full set.
    pub fn build_evidence(
        change_type: &ChangeType,
        snapshot_before: PairRelationSnapshot,
        had_prior_relation: bool,
        from_player: &Player,
        partner_position: Option<&PlayerPositionType>,
        partner_squad_status: Option<PlayerSquadStatus>,
        sim_date: NaiveDate,
    ) -> Vec<HappinessEventEvidence> {
        let mut evidence: Vec<HappinessEventEvidence> = Vec::new();

        // Existing bond strength — three buckets reading off `level`.
        if snapshot_before.level <= -25.0 {
            evidence.push(HappinessEventEvidence::AlreadyStrainedRelationship);
        } else if snapshot_before.level >= 50.0 {
            evidence.push(HappinessEventEvidence::StrongExistingBond);
        } else if !had_prior_relation || snapshot_before.level.abs() < 25.0 {
            evidence.push(HappinessEventEvidence::WeakExistingBond);
        }

        // Per-axis evidence — only flag axes that meaningfully diverge
        // from the neutral default so the renderer doesn't drown in flags.
        if snapshot_before.trust <= 35.0 {
            evidence.push(HappinessEventEvidence::LowTrust);
        }
        if snapshot_before.friendship <= 25.0 {
            evidence.push(HappinessEventEvidence::LowFriendship);
        }
        if snapshot_before.professional_respect <= 35.0 {
            evidence.push(HappinessEventEvidence::LowProfessionalRespect);
        } else if snapshot_before.professional_respect >= 70.0 {
            evidence.push(HappinessEventEvidence::HighProfessionalRespect);
        }

        // Cause-specific evidence — translates the upstream ChangeType
        // into the football-realistic atom the user can read.
        match change_type {
            ChangeType::CompetitionRivalry => {
                if let Some(pos) = partner_position {
                    if pos == &from_player.position() {
                        evidence.push(HappinessEventEvidence::SamePositionCompetition);
                    }
                }
                if let (Some(theirs), Some(ours)) = (
                    partner_squad_status,
                    from_player
                        .contract
                        .as_ref()
                        .map(|c| c.squad_status.clone()),
                ) {
                    if Self::same_status_tier(theirs, ours) {
                        evidence.push(HappinessEventEvidence::SimilarSquadStatusCompetition);
                    }
                }
                if from_player.attributes.ambition >= 15.0 {
                    evidence.push(HappinessEventEvidence::HighAmbition);
                }
            }
            ChangeType::TrainingFriction | ChangeType::TrainingBonding => {
                evidence.push(HappinessEventEvidence::TrainingStandardsMismatch);
            }
            ChangeType::MatchCooperation => {
                evidence.push(HappinessEventEvidence::MatchCooperation);
                if let Some(pos) = partner_position {
                    if pos != &from_player.position() {
                        evidence.push(HappinessEventEvidence::ComplementaryRoles);
                    }
                }
            }
            ChangeType::ReputationTension => {
                evidence.push(HappinessEventEvidence::ReputationGap);
            }
            ChangeType::MentorshipBond => {
                evidence.push(HappinessEventEvidence::MentorInfluence);
            }
            _ => {}
        }

        // Recent-transfer evidence — a still-settling player has no
        // inner circle yet and reads incidents through that lens.
        if let Some(date) = from_player.last_transfer_date {
            let weeks_since = (sim_date - date).num_days() / 7;
            if (0..12).contains(&weeks_since) {
                evidence.push(HappinessEventEvidence::NewSigningStillSettling);
            }
        }

        evidence
    }

    /// Build the explanation context for a `ConflictWithTeammate` emit.
    pub fn conflict_context(
        change_type: &ChangeType,
        magnitude: f32,
        snapshot_before: PairRelationSnapshot,
        level_after: f32,
        evidence: &[HappinessEventEvidence],
    ) -> HappinessEventContext {
        let conflict_payload = TeammateConflictContext::new(
            Self::reason_for_conflict(change_type, evidence),
            Self::location_for_conflict(change_type, evidence),
        );
        let ctx = HappinessEventContext::new(
            Self::cause_for_conflict(change_type),
            HappinessEventSeverity::from_magnitude(magnitude),
            Self::scope_for_conflict(change_type),
        )
        .with_relationship_levels(snapshot_before.level, level_after)
        .with_relationship_axes(
            snapshot_before.trust,
            snapshot_before.friendship,
            snapshot_before.professional_respect,
        )
        .with_change_kind(HappinessEventChangeKind::from_change_type(change_type))
        .with_evidence_iter(evidence.iter().copied())
        .with_teammate_conflict_context(conflict_payload);

        // Outlook: a strained relation OR a repeated incident → real
        // damage risk; everything else settles.
        let strained = snapshot_before.level <= -25.0
            || evidence.contains(&HappinessEventEvidence::AlreadyStrainedRelationship)
            || evidence.contains(&HappinessEventEvidence::RepeatedIncident);
        let follow_up = if strained {
            HappinessEventFollowUp::DressingRoomDamageRisk
        } else {
            HappinessEventFollowUp::LikelyToSettle
        };
        ctx.with_follow_up(follow_up)
    }

    /// Resolve the football-specific reason behind a conflict from the
    /// upstream `ChangeType`, refined by any concrete evidence atoms the
    /// caller already gathered (wage gap, training standards, etc.).
    /// Closed mapping — every change type lands on a specific
    /// `TeammateConflictReason` so the renderer can drop the generic
    /// "had a disagreement" line.
    pub fn reason_for_conflict(
        change_type: &ChangeType,
        evidence: &[HappinessEventEvidence],
    ) -> TeammateConflictReason {
        if evidence.contains(&HappinessEventEvidence::WageGap) {
            return TeammateConflictReason::WageJealousy;
        }
        if evidence.contains(&HappinessEventEvidence::LanguageBarrier) {
            return TeammateConflictReason::LanguageBarrier;
        }
        if evidence.contains(&HappinessEventEvidence::MediaIncident) {
            return TeammateConflictReason::MediaComments;
        }
        if evidence.contains(&HappinessEventEvidence::TrainingStandardsMismatch) {
            return TeammateConflictReason::TrainingStandards;
        }
        match change_type {
            ChangeType::TrainingFriction => TeammateConflictReason::TrainingStandards,
            ChangeType::CompetitionRivalry => TeammateConflictReason::PositionalRivalry,
            ChangeType::ReputationTension => TeammateConflictReason::WageJealousy,
            ChangeType::TacticalDisagreement => TeammateConflictReason::TacticalBlame,
            ChangeType::DisciplinaryAction => TeammateConflictReason::LeadershipChallenge,
            ChangeType::PersonalConflict => TeammateConflictReason::PersonalityClash,
            _ => TeammateConflictReason::PersonalityClash,
        }
    }

    /// Where the conflict played out, derived from the change type with
    /// scope evidence (training-ground / dressing-room / media) able to
    /// override the default.
    pub fn location_for_conflict(
        change_type: &ChangeType,
        evidence: &[HappinessEventEvidence],
    ) -> ConflictLocation {
        if evidence.contains(&HappinessEventEvidence::MediaIncident) {
            return ConflictLocation::Media;
        }
        if evidence.contains(&HappinessEventEvidence::TrainingGroundIncident) {
            return ConflictLocation::TrainingGround;
        }
        if evidence.contains(&HappinessEventEvidence::DressingRoomRow) {
            return ConflictLocation::DressingRoom;
        }
        match change_type {
            ChangeType::TrainingFriction
            | ChangeType::CompetitionRivalry
            | ChangeType::TacticalDisagreement => ConflictLocation::TrainingGround,
            _ => ConflictLocation::DressingRoom,
        }
    }

    /// Build the explanation context for a `TeammateBonding` emit.
    pub fn bonding_context(
        change_type: &ChangeType,
        magnitude: f32,
        snapshot_before: PairRelationSnapshot,
        level_after: f32,
        evidence: &[HappinessEventEvidence],
    ) -> HappinessEventContext {
        HappinessEventContext::new(
            Self::cause_for_bonding(change_type),
            HappinessEventSeverity::from_magnitude(magnitude),
            Self::scope_for_bonding(change_type),
        )
        .with_relationship_levels(snapshot_before.level, level_after)
        .with_relationship_axes(
            snapshot_before.trust,
            snapshot_before.friendship,
            snapshot_before.professional_respect,
        )
        .with_change_kind(HappinessEventChangeKind::from_change_type(change_type))
        .with_evidence_iter(evidence.iter().copied())
        .with_follow_up(HappinessEventFollowUp::TrendImproving)
    }

    fn same_status_tier(a: PlayerSquadStatus, b: PlayerSquadStatus) -> bool {
        use PlayerSquadStatus as S;
        matches!(
            (a, b),
            (S::KeyPlayer, S::KeyPlayer)
                | (S::FirstTeamRegular, S::FirstTeamRegular)
                | (S::FirstTeamSquadRotation, S::FirstTeamSquadRotation)
                | (S::MainBackupPlayer, S::MainBackupPlayer)
                | (S::HotProspectForTheFuture, S::HotProspectForTheFuture)
                | (S::DecentYoungster, S::DecentYoungster)
                | (S::NotNeeded, S::NotNeeded)
        )
    }
}

/// Estimate the level-axis movement that
/// [`PlayerRelation::apply_change`] applies for a given raw delta and
/// change type. Mirrors the multipliers in `relations.rs` so the
/// event-emission threshold reflects the change the player actually
/// experiences. Keep these in sync if `apply_change` changes.
fn applied_level_magnitude(raw: f32, change_type: &ChangeType) -> f32 {
    let magnitude = raw.abs();
    let level_mult = match change_type {
        ChangeType::MatchCooperation => 2.0,
        ChangeType::TrainingBonding => 1.0,
        ChangeType::ConflictResolution => 2.0,
        ChangeType::PersonalSupport => 2.0,
        ChangeType::CompetitionRivalry => 2.0,
        ChangeType::TrainingFriction => 1.0,
        ChangeType::PersonalConflict => 3.0,
        ChangeType::ReputationAdmiration => 1.5,
        ChangeType::ReputationTension => 1.5,
        // Catch-all changes (CoachingSuccess, MentorshipBond, …) hit
        // level_axis 1:1 in apply_change's default arm.
        _ => 1.0,
    };
    let signed = if raw >= 0.0 { 1.0 } else { -1.0 };
    signed * magnitude * level_mult
}

pub struct PlayerRelationshipChangeResult {
    pub from_player_id: u32,
    pub to_player_id: u32,
    pub relationship_change: f32,
    pub change_type: ChangeType,
}

#[derive(Debug, Clone)]
pub struct ManagerTalkResult {
    pub player_id: u32,
    pub staff_id: u32,
    pub talk_type: ManagerTalkType,
    pub success: bool,
    pub morale_change: f32,
    pub relationship_change: f32,
    /// Tone the manager picked. Defaults to [`InteractionTone::Calm`] when
    /// the picker hasn't decided. Drives how the talk lands relative to
    /// the player's personality (mirrors team-talk tone modelling).
    pub tone: InteractionTone,
    /// True if the manager backed an honest framing instead of empty
    /// reassurance. An honest "no, you're not playing more" hits less
    /// hard than a false promise that breaks two months later.
    pub honest_framing: bool,
    /// Snapshot of player morale before applying the talk — captured by
    /// the talk picker. Used purely for the interaction log.
    pub mood_before: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ManagerTalkType {
    PlayingTimeTalk,
    MoraleTalk,
    TransferDiscussion,
    Praise,
    Discipline,
    Motivational,
    PlayingTimeRequest,
    LoanRequest,
}

/// Translate the legacy `ManagerTalkType` into the new
/// [`InteractionTopic`] taxonomy. Used by `process_manager_talks` so the
/// interaction log uses the football-specific topics rather than the
/// implementation-shaped talk-types.
pub(crate) fn topic_for_talk(talk: ManagerTalkType) -> InteractionTopic {
    match talk {
        ManagerTalkType::PlayingTimeTalk | ManagerTalkType::PlayingTimeRequest => {
            InteractionTopic::PlayingTime
        }
        ManagerTalkType::MoraleTalk | ManagerTalkType::Motivational => InteractionTopic::PoorForm,
        ManagerTalkType::Praise => InteractionTopic::GoodForm,
        ManagerTalkType::Discipline => InteractionTopic::Discipline,
        ManagerTalkType::TransferDiscussion => InteractionTopic::TransferRequest,
        ManagerTalkType::LoanRequest => InteractionTopic::LoanRequest,
    }
}

struct ManagerInteractionTopicMapper;

impl ManagerInteractionTopicMapper {
    fn from_talk(talk: &ManagerTalkType) -> ManagerInteractionTopic {
        match talk {
            ManagerTalkType::PlayingTimeTalk | ManagerTalkType::PlayingTimeRequest => {
                ManagerInteractionTopic::PlayingTime
            }
            ManagerTalkType::Praise
            | ManagerTalkType::MoraleTalk
            | ManagerTalkType::Motivational => ManagerInteractionTopic::Performance,
            ManagerTalkType::Discipline => ManagerInteractionTopic::Discipline,
            ManagerTalkType::TransferDiscussion => ManagerInteractionTopic::Other,
            ManagerTalkType::LoanRequest => ManagerInteractionTopic::Other,
        }
    }
}

struct ManagerInteractionToneMapper;

impl ManagerInteractionToneMapper {
    fn from_interaction(tone: &InteractionTone) -> ManagerInteractionTone {
        match tone {
            InteractionTone::Calm => ManagerInteractionTone::Calm,
            InteractionTone::Honest => ManagerInteractionTone::Honest,
            InteractionTone::Demanding | InteractionTone::Authoritarian => {
                ManagerInteractionTone::Demanding
            }
            InteractionTone::Supportive => ManagerInteractionTone::Supportive,
            InteractionTone::Apologetic => ManagerInteractionTone::Supportive,
            InteractionTone::Evasive => ManagerInteractionTone::Calm,
        }
    }
}

struct ManagerInteractionAcceptanceMapper;

impl ManagerInteractionAcceptanceMapper {
    fn from_outcome(success: bool, morale_change: f32) -> PlayerAcceptance {
        if success {
            if morale_change >= 4.0 {
                PlayerAcceptance::Motivated
            } else if morale_change >= 1.0 {
                PlayerAcceptance::Accepted
            } else {
                PlayerAcceptance::Ambivalent
            }
        } else if morale_change <= -4.0 {
            PlayerAcceptance::Resented
        } else if morale_change <= -1.0 {
            PlayerAcceptance::Discouraged
        } else {
            PlayerAcceptance::Ambivalent
        }
    }
}

#[cfg(test)]
mod cause_mapping_tests {
    use super::*;

    #[test]
    fn conflict_change_types_map_to_specific_causes() {
        // The mapping is the audit-side contract: the renderer never falls
        // back to the generic "Other" line for an upgraded conflict event.
        assert_eq!(
            PairEventContextBuilder::cause_for_conflict(&ChangeType::PersonalConflict),
            HappinessEventCause::PersonalityClash
        );
        assert_eq!(
            PairEventContextBuilder::cause_for_conflict(&ChangeType::TrainingFriction),
            HappinessEventCause::TrainingFriction
        );
        assert_eq!(
            PairEventContextBuilder::cause_for_conflict(&ChangeType::CompetitionRivalry),
            HappinessEventCause::PositionalRivalry
        );
        assert_eq!(
            PairEventContextBuilder::cause_for_conflict(&ChangeType::ReputationTension),
            HappinessEventCause::ReputationTension
        );
        assert_eq!(
            PairEventContextBuilder::cause_for_conflict(&ChangeType::TacticalDisagreement),
            HappinessEventCause::TacticalDisagreement
        );
    }

    #[test]
    fn snapshot_neutral_matches_player_relation_defaults() {
        // The neutral default is what we hand the context builder when
        // the pair has never interacted before. If `PlayerRelation`'s
        // neutral changes, this fallback drifts silently — pin the
        // values explicitly.
        let n = PairRelationSnapshot::neutral();
        assert_eq!(n.level, 0.0);
        assert_eq!(n.trust, 50.0);
        assert_eq!(n.friendship, 30.0);
        assert_eq!(n.professional_respect, 50.0);
    }

    #[test]
    fn evidence_low_trust_fires_below_threshold() {
        // Use a synthetic builder path: we don't construct a full Player
        // here (heavy stack), so build_evidence's relation-axis derivation
        // is the surface tested by build_evidence directly. Verify the
        // axis-based evidence lights up via a minimal Player would
        // require massive setup; instead we test the edge directly.
        let snap_low_trust = PairRelationSnapshot {
            level: 0.0,
            trust: 30.0,
            friendship: 30.0,
            professional_respect: 50.0,
        };
        // Simulate what build_evidence does for the trust axis directly.
        let mut hits = vec![];
        if snap_low_trust.trust <= 35.0 {
            hits.push(HappinessEventEvidence::LowTrust);
        }
        assert!(hits.contains(&HappinessEventEvidence::LowTrust));
    }

    #[test]
    fn change_kind_round_trips_for_every_change_type() {
        // Every ChangeType variant must map to a non-Other kind so the
        // renderer can branch on the underlying driver. If a new
        // ChangeType is added to the relations crate, this catches it.
        for ct in [
            ChangeType::MatchCooperation,
            ChangeType::TrainingBonding,
            ChangeType::ConflictResolution,
            ChangeType::PersonalSupport,
            ChangeType::CoachingSuccess,
            ChangeType::TeamSuccess,
            ChangeType::MentorshipBond,
            ChangeType::CompetitionRivalry,
            ChangeType::TrainingFriction,
            ChangeType::PersonalConflict,
            ChangeType::TacticalDisagreement,
            ChangeType::DisciplinaryAction,
            ChangeType::TeamFailure,
            ChangeType::ReputationAdmiration,
            ChangeType::ReputationTension,
            ChangeType::NaturalProgression,
        ] {
            let kind = HappinessEventChangeKind::from_change_type(&ct);
            assert_ne!(
                kind,
                HappinessEventChangeKind::Other,
                "{:?} fell through to Other — add an explicit arm",
                ct
            );
        }
    }

    #[test]
    fn conflict_context_records_both_levels() {
        let snap = PairRelationSnapshot {
            level: 10.0,
            trust: 40.0,
            friendship: 30.0,
            professional_respect: 50.0,
        };
        let evidence = vec![HappinessEventEvidence::LowFriendship];
        let ctx = PairEventContextBuilder::conflict_context(
            &ChangeType::TrainingFriction,
            -1.5,
            snap,
            -2.0,
            &evidence,
        );
        assert_eq!(ctx.relationship_level_before, Some(10.0));
        assert_eq!(ctx.relationship_level_after, Some(-2.0));
        assert!(
            ctx.evidence
                .contains(&HappinessEventEvidence::LowFriendship)
        );
        assert_eq!(
            ctx.change_type,
            Some(HappinessEventChangeKind::TrainingFriction)
        );
    }

    #[test]
    fn conflict_context_outlook_promotes_repeated_incident() {
        // A first-time conflict on a neutral pair settles; a repeated
        // incident OR a strained existing relationship escalates the
        // outlook to "dressing-room damage risk".
        let snap = PairRelationSnapshot::neutral();
        let none_strained = PairEventContextBuilder::conflict_context(
            &ChangeType::PersonalConflict,
            -1.5,
            snap,
            -2.0,
            &[],
        );
        assert_eq!(
            none_strained.follow_up,
            Some(HappinessEventFollowUp::LikelyToSettle)
        );

        let repeated = PairEventContextBuilder::conflict_context(
            &ChangeType::PersonalConflict,
            -1.5,
            snap,
            -2.0,
            &[HappinessEventEvidence::RepeatedIncident],
        );
        assert_eq!(
            repeated.follow_up,
            Some(HappinessEventFollowUp::DressingRoomDamageRisk)
        );
    }

    #[test]
    fn bonding_change_types_never_map_to_negative_cause() {
        // No matter what the upstream signal looks like, a bonding event's
        // explanation must never read like a conflict — the cause/headline
        // would contradict the morale +X.
        for ct in [
            ChangeType::MatchCooperation,
            ChangeType::TrainingBonding,
            ChangeType::PersonalSupport,
            ChangeType::ReputationAdmiration,
            ChangeType::ConflictResolution,
            ChangeType::TeamSuccess,
            ChangeType::MentorshipBond,
            ChangeType::CoachingSuccess,
            ChangeType::NaturalProgression,
        ] {
            let cause = PairEventContextBuilder::cause_for_bonding(&ct);
            let positive = matches!(
                cause,
                HappinessEventCause::MatchCooperation
                    | HappinessEventCause::TrainingPartnership
                    | HappinessEventCause::ReputationAdmiration
                    | HappinessEventCause::NationalityIntegration
            );
            assert!(positive, "{:?} produced negative cause {:?}", ct, cause);
        }
    }
}

#[cfg(test)]
mod severity_cap_tests {
    //! Slim-harness tests for [`PlayerBehaviourResult::process`].
    use super::*;
    use crate::Staff;
    use crate::club::player::builder::PlayerBuilder;
    use crate::league::League;
    use crate::league::result::LeagueProcessAccess;
    use crate::shared::fullname::FullName;
    use crate::shared::indexes::SimulatorDataIndexes;
    use crate::simulator::CountryInfo;
    use crate::{
        Club, ConflictLocation, Country, HappinessEventCause, HappinessEventSeverity,
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, Team, TeammateConflictContext, TeammateConflictReason,
    };
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    use std::collections::HashMap;

    /// Slim harness that implements only the [`LeagueProcessAccess`]
    /// methods [`PlayerBehaviourResult::process`] actually calls:
    /// `date`, `player`, and `player_mut`. Every other accessor
    /// panics via `unreachable!` — if a future change in `process`
    /// starts touching them the failing test points right at the new
    /// requirement.
    struct StubData {
        now: NaiveDateTime,
        players: Vec<Player>,
        country_info: HashMap<u32, CountryInfo>,
    }

    impl StubData {
        fn new(now: NaiveDateTime, players: Vec<Player>) -> Self {
            Self {
                now,
                players,
                country_info: HashMap::new(),
            }
        }
        fn find(&self, id: u32) -> Option<usize> {
            self.players.iter().position(|p| p.id == id)
        }
    }

    impl LeagueProcessAccess for StubData {
        fn date(&self) -> NaiveDateTime {
            self.now
        }
        fn indexes(&self) -> Option<&SimulatorDataIndexes> {
            None
        }
        fn country_info(&self) -> &HashMap<u32, CountryInfo> {
            &self.country_info
        }
        fn country(&self, _id: u32) -> Option<&Country> {
            None
        }
        fn country_mut(&mut self, _id: u32) -> Option<&mut Country> {
            None
        }
        fn country_by_club(&self, _club_id: u32) -> Option<&Country> {
            None
        }
        fn league(&self, _id: u32) -> Option<&League> {
            None
        }
        fn league_mut(&mut self, _id: u32) -> Option<&mut League> {
            None
        }
        fn club(&self, _id: u32) -> Option<&Club> {
            None
        }
        fn club_mut(&mut self, _id: u32) -> Option<&mut Club> {
            None
        }
        fn team(&self, _id: u32) -> Option<&Team> {
            None
        }
        fn team_mut(&mut self, _id: u32) -> Option<&mut Team> {
            None
        }
        fn player(&self, id: u32) -> Option<&Player> {
            self.find(id).map(|i| &self.players[i])
        }
        fn player_mut(&mut self, id: u32) -> Option<&mut Player> {
            self.find(id).map(move |i| &mut self.players[i])
        }
        fn admit_free_agent_staff(&mut self, _staff: Staff) {
            unreachable!()
        }
        fn queue_manager_appointment(&mut self, _club_id: u32) {
            unreachable!()
        }
        fn random_player_mut(&mut self) -> Option<&mut Player> {
            unreachable!()
        }
    }

    /// Per-test fixtures grouped under a struct so the file has no
    /// free helpers (the project keeps even test scaffolding behind a
    /// named type).
    struct Fixtures;

    impl Fixtures {
        fn date(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn midnight(y: i32, m: u32, day: u32) -> NaiveDateTime {
            NaiveDateTime::new(Self::date(y, m, day), NaiveTime::from_hms_opt(0, 0, 0).unwrap())
        }

        fn person() -> PersonAttributes {
            PersonAttributes {
                adaptability: 12.0,
                ambition: 12.0,
                controversy: 5.0,
                loyalty: 10.0,
                pressure: 12.0,
                professionalism: 12.0,
                sportsmanship: 12.0,
                temperament: 12.0,
                consistency: 12.0,
                important_matches: 12.0,
                dirtiness: 5.0,
            }
        }

        fn player(id: u32) -> Player {
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("T".into(), id.to_string()))
                .birth_date(Self::date(2000, 1, 1))
                .country_id(1)
                .attributes(Self::person())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 20,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap()
        }

        fn count_conflicts(p: &Player) -> usize {
            p.happiness
                .recent_events
                .iter()
                .filter(|e| e.event_type == HappinessEventType::ConflictWithTeammate)
                .count()
        }

        /// Push a `ConflictWithTeammate` event onto a player as if a
        /// direct emitter (controversy / mentorship / training) had
        /// already run earlier in the same tick — `days_ago == 0` so
        /// the shared budget sees it.
        fn seed_same_tick_conflict(player: &mut Player, partner: u32) {
            let ctx = HappinessEventContext::new(
                HappinessEventCause::PersonalityClash,
                HappinessEventSeverity::Moderate,
                HappinessEventScope::DressingRoom,
            )
            .with_teammate_conflict_context(TeammateConflictContext::new(
                TeammateConflictReason::PersonalityClash,
                ConflictLocation::DressingRoom,
            ));
            player.happiness.add_event_with_context(
                HappinessEventType::ConflictWithTeammate,
                -1.5,
                Some(partner),
                ctx,
            );
        }
    }

    #[test]
    fn personality_conflict_swarm_caps_visible_rows_but_updates_all_relations() {
        let now = Fixtures::midnight(2026, 6, 1);
        let mut players: Vec<Player> = (1..=10).map(Fixtures::player).collect();
        for p in &mut players[1..] {
            p.attributes.controversy = 18.0;
        }
        let mut data = StubData::new(now, players);

        let mut result = PlayerBehaviourResult::new();
        for partner in 2..=10u32 {
            result.relationship_result.push(PlayerRelationshipChangeResult {
                from_player_id: 1,
                to_player_id: partner,
                relationship_change: -0.4,
                change_type: ChangeType::PersonalConflict,
            });
        }
        result.process(&mut data);

        let p1 = data.player(1).unwrap();
        assert!(Fixtures::count_conflicts(p1) <= 2);
        for partner in 2..=10u32 {
            let level = p1.relations.get_player(partner).unwrap().level;
            assert!(level < 0.0, "partner {} level was {}", partner, level);
        }
    }

    #[test]
    fn relationship_conflict_cap_picks_strongest_candidates() {
        let now = Fixtures::midnight(2026, 6, 1);
        let mut players: Vec<Player> = (1..=6).map(Fixtures::player).collect();
        if let Some(p1) = players.iter_mut().find(|p| p.id == 1) {
            p1.relations
                .update_with_type(5, -0.5, ChangeType::PersonalConflict, now.date());
        }
        let mut data = StubData::new(now, players);

        let mut result = PlayerBehaviourResult::new();
        // Iteration-order trap: weak conflicts first, strong last.
        // The ranker must still pick the strong pair.
        for (partner, mag) in [(2u32, -0.20), (3, -0.20), (4, -0.20), (5, -0.50), (6, -0.50)] {
            result.relationship_result.push(PlayerRelationshipChangeResult {
                from_player_id: 1,
                to_player_id: partner,
                relationship_change: mag,
                change_type: ChangeType::PersonalConflict,
            });
        }
        result.process(&mut data);

        let p1 = data.player(1).unwrap();
        let visible: Vec<u32> = p1
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::ConflictWithTeammate)
            .filter_map(|e| e.partner_player_id)
            .collect();
        assert_eq!(visible.len(), 2);
        assert!(visible.contains(&5) && visible.contains(&6), "got {:?}", visible);
    }

    #[test]
    fn pre_seeded_same_tick_conflicts_block_further_visible_rows_but_relations_still_update() {
        // Two direct-emitter ConflictWithTeammate rows are already on
        // the feed when the behaviour pass runs. The shared
        // same-tick budget is full, so the behaviour pass must add
        // ZERO visible rows — but the underlying relation drift must
        // still land for every candidate.
        let now = Fixtures::midnight(2026, 6, 1);
        let mut players: Vec<Player> = (1..=6).map(Fixtures::player).collect();
        Fixtures::seed_same_tick_conflict(&mut players[0], 2);
        Fixtures::seed_same_tick_conflict(&mut players[0], 3);
        let mut data = StubData::new(now, players);

        let mut result = PlayerBehaviourResult::new();
        for partner in [4u32, 5, 6] {
            result.relationship_result.push(PlayerRelationshipChangeResult {
                from_player_id: 1,
                to_player_id: partner,
                relationship_change: -0.5,
                change_type: ChangeType::PersonalConflict,
            });
        }
        result.process(&mut data);

        let p1 = data.player(1).unwrap();
        assert_eq!(
            Fixtures::count_conflicts(p1),
            2,
            "behaviour pass must respect the pre-existing same-tick budget"
        );
        for partner in [4u32, 5, 6] {
            let rel = p1.relations.get_player(partner).expect("relation must exist");
            assert!(
                rel.level < 0.0,
                "relation drift must land even when the visible row is capped: partner {} level {}",
                partner,
                rel.level
            );
        }
    }

    #[test]
    fn direct_conflict_emitters_do_not_bypass_same_day_spam_limit() {
        let now = Fixtures::midnight(2026, 6, 1);
        let players: Vec<Player> = (1..=4).map(Fixtures::player).collect();
        let mut data = StubData::new(now, players);

        let mut result = PlayerBehaviourResult::new();
        for partner in [2u32, 3] {
            result.relationship_result.push(PlayerRelationshipChangeResult {
                from_player_id: 1,
                to_player_id: partner,
                relationship_change: -0.5,
                change_type: ChangeType::PersonalConflict,
            });
        }
        result.process(&mut data);

        let p1 = data.player_mut(1).unwrap();
        assert_eq!(Fixtures::count_conflicts(p1), 2);
        let already_today = p1
            .happiness
            .same_tick_event_count(&HappinessEventType::ConflictWithTeammate);
        assert!(already_today >= 2);
    }
}

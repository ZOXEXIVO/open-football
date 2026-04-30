use crate::club::player::ManagerPromiseKind;
use crate::club::player::interaction::{
    InteractionOutcome, InteractionTone, InteractionTopic, ManagerInteraction,
    default_cooldown_days,
};
use crate::{ChangeType, HappinessEventType, PlayerStatusType, RelationshipChange, SimulatorData};
use chrono::Duration;

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
    pub reason: &'static str,
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

    pub fn process(&self, data: &mut SimulatorData) {
        self.players.process(data);
        self.process_manager_talks(data);
        self.process_contract_terminations(data);
    }

    fn process_contract_terminations(&self, data: &mut SimulatorData) {
        let date = data.date.date();
        for termination in &self.contract_terminations {
            let (country_id, club_id) = match data
                .indexes
                .as_ref()
                .and_then(|i| i.get_player_location(termination.player_id))
            {
                Some((_, country_id, club_id, _)) => (country_id, club_id),
                None => continue,
            };
            if let Some(player) = data.player_mut(termination.player_id) {
                player.on_contract_terminated(date);
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
                crate::transfers::pipeline::PipelineProcessor::clear_player_interest(
                    country,
                    termination.player_id,
                );
            }
            log::debug!(
                "Contract terminated: player {} by club {} — payout {} ({})",
                termination.player_id,
                club_id,
                termination.payout,
                termination.reason
            );
        }
    }

    fn process_manager_talks(&self, data: &mut SimulatorData) {
        let sim_date = data.date.date();

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
                    match talk.talk_type {
                        ManagerTalkType::Discipline => HappinessEventType::ManagerDiscipline,
                        _ => HappinessEventType::PoorTraining,
                    }
                };
                player.happiness.add_event(event_type, talk.morale_change);

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
                                player
                                    .happiness
                                    .add_event(HappinessEventType::LoanListingAccepted, 5.0);
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

    pub fn process(&self, data: &mut SimulatorData) {
        let sim_date = data.date.date();

        for relationship_result in &self.relationship_result {
            if let Some(player_to_modify) = data.player_mut(relationship_result.from_player_id) {
                player_to_modify.relations.update_with_type(
                    relationship_result.to_player_id,
                    relationship_result.relationship_change,
                    relationship_result.change_type.clone(),
                    sim_date,
                );

                // Generate teammate relationship events visible in player
                // history. The raw `relationship_change` doesn't reflect
                // what `PlayerRelation::apply_change` actually applies —
                // PersonalConflict, MatchCooperation, ConflictResolution
                // and others multiply the magnitude internally. Compare
                // the *applied* magnitude against the threshold so a
                // raw -0.20 PersonalConflict (≈-0.60 applied) actually
                // surfaces. Tag the partner so the UI can link to the
                // specific teammate.
                let applied = applied_level_magnitude(
                    relationship_result.relationship_change,
                    &relationship_result.change_type,
                );
                let partner_id = relationship_result.to_player_id;
                // 45-day per-partner cooldown so a chronic friction
                // pair (or a long-running bonding pair) doesn't spam
                // history with one event per weekly tick.
                const PARTNER_EVENT_COOLDOWN_DAYS: u16 = 45;
                if applied > 0.5
                    && !player_to_modify.happiness.has_recent_event_with_partner(
                        &HappinessEventType::TeammateBonding,
                        partner_id,
                        PARTNER_EVENT_COOLDOWN_DAYS,
                    )
                {
                    player_to_modify.happiness.add_event_with_partner(
                        HappinessEventType::TeammateBonding,
                        1.0,
                        Some(partner_id),
                    );
                } else if applied < -0.5
                    && !player_to_modify.happiness.has_recent_event_with_partner(
                        &HappinessEventType::ConflictWithTeammate,
                        partner_id,
                        PARTNER_EVENT_COOLDOWN_DAYS,
                    )
                {
                    player_to_modify.happiness.add_event_with_partner(
                        HappinessEventType::ConflictWithTeammate,
                        -1.5,
                        Some(partner_id),
                    );
                }
            }
        }
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

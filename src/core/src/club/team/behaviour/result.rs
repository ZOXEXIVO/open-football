use crate::club::player::ManagerPromiseKind;
use crate::{ChangeType, HappinessEventType, PlayerStatusType, RelationshipChange, SimulatorData};

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
                    club.finance.balance.push_expense_player_wages(termination.payout as i64);
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
                termination.player_id, club_id, termination.payout, termination.reason
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
                    player.relations.update_staff_relationship(talk.staff_id, change, sim_date);

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
                        ManagerTalkType::PlayingTimeTalk => HappinessEventType::ManagerPlayingTimePromise,
                        _ => HappinessEventType::ManagerPraise,
                    }
                } else {
                    match talk.talk_type {
                        ManagerTalkType::Discipline => HappinessEventType::ManagerDiscipline,
                        _ => HappinessEventType::PoorTraining,
                    }
                };
                player.happiness.add_event(event_type, talk.morale_change);

                // Remove statuses on success
                if talk.success {
                    match talk.talk_type {
                        ManagerTalkType::PlayingTimeTalk | ManagerTalkType::MoraleTalk => {
                            player.statuses.remove(PlayerStatusType::Unh);
                            // A successful playing-time chat is a concrete
                            // promise. 30-day horizon; verified weekly.
                            if talk.talk_type == ManagerTalkType::PlayingTimeTalk {
                                player.record_promise(
                                    ManagerPromiseKind::PlayingTime,
                                    sim_date,
                                    30,
                                );
                            }
                        }
                        ManagerTalkType::TransferDiscussion => {
                            player.statuses.remove(PlayerStatusType::Req);
                        }
                        ManagerTalkType::PlayingTimeRequest => {
                            player.happiness.add_event(
                                HappinessEventType::ManagerPlayingTimePromise,
                                8.0,
                            );
                        }
                        ManagerTalkType::LoanRequest => {
                            player.statuses.add(sim_date, PlayerStatusType::Loa);
                            player.happiness.add_event(
                                HappinessEventType::LoanListingAccepted,
                                5.0,
                            );
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
                            // Denied loan — player becomes unsettled about their future
                            player.statuses.add(sim_date, PlayerStatusType::Fut);
                            player.happiness.add_event(
                                HappinessEventType::LackOfPlayingTime,
                                -5.0,
                            );
                        }
                        _ => {}
                    }
                }
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

                // Generate teammate relationship events visible in player history.
                // Use a higher threshold to prevent event spam from routine friction.
                // Tag the partner so the UI can link to the specific teammate.
                let partner_id = Some(relationship_result.to_player_id);
                if relationship_result.relationship_change > 0.5 {
                    player_to_modify.happiness.add_event_with_partner(
                        HappinessEventType::TeammateBonding,
                        1.0,
                        partner_id,
                    );
                } else if relationship_result.relationship_change < -0.5 {
                    player_to_modify.happiness.add_event_with_partner(
                        HappinessEventType::ConflictWithTeammate,
                        -1.5,
                        partner_id,
                    );
                }
            }
        }
    }
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

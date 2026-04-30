//! Weekly recruitment meeting.
//!
//! Brings the recruitment department together: every scout monitoring
//! a meeting-ready player casts a vote; chief scout (if present) gets
//! amplified weight; data analyst contributes a soft signal independent
//! of scouting; DoF / manager / head of recruitment sit in to weigh
//! board fit. The meeting produces `RecruitmentDecision` records that
//! gate downstream shortlist and board behaviour:
//!
//! * `PromoteToShortlist` — high consensus + budget OK → goes onto
//!   the shortlist with a meeting-stamped score multiplier.
//! * `AskBoardApproval` — high consensus, budget elevated → still
//!   shortlisted, but the board is more likely to pause (handled by
//!   the dossier path).
//! * `KeepMonitoring` — split or low-confidence; nothing happens
//!   downstream this week, scouts keep watching.
//! * `Reject` — vote consensus negative or risks fatal → adds to
//!   rejection blocklist.
//!
//! The implementation follows the rest of the pipeline's two-pass
//! borrow rule: an immutable read pass collects everything (scout
//! attributes, monitoring rows, request context, club context),
//! then a mutable pass writes the decision back into each club's
//! `ClubTransferPlan`.

use chrono::{Datelike, NaiveDate};
use log::debug;

use crate::club::staff::contract::StaffPosition;
use crate::club::staff::staff::Staff;
use crate::transfers::pipeline::processor::PipelineProcessor;
use crate::transfers::pipeline::recruitment::{
    BoardRecruitmentDossier, RecruitmentDecision, RecruitmentDecisionType, RecruitmentMeeting,
    ScoutMonitoringStatus, ScoutPlayerMonitoring, ScoutVote, ScoutVoteChoice, ScoutVoteReason,
};
use crate::transfers::pipeline::{
    ReportRiskFlag, ShortlistCandidate, ShortlistCandidateStatus, TransferRequest,
    TransferRequestStatus, TransferShortlist,
};
use crate::utils::IntegerUtils;
use crate::{Country, StaffEventType};

/// Captured snapshot of a scout for vote calculation. Decoupling
/// from the live `Staff` borrow lets us compute votes in pass 1
/// without holding the country immutably across pass 2.
#[derive(Debug, Clone, Copy)]
struct ScoutSnapshot {
    staff_id: u32,
    is_chief: bool,
    judging_ability: u8,
    judging_potential: u8,
    /// Higher discipline = stricter scout. Drives the threshold for
    /// flipping Approve into StrongApprove and Monitor into Reject.
    discipline: u8,
    /// Adaptability lifts the scout's tolerance for unusual profiles.
    adaptability: u8,
    /// Determination doesn't change the choice but lifts the score
    /// weight a touch — a determined scout pushes their position harder.
    determination: u8,
    /// Tactical knowledge sharpens the role-fit reading.
    tactical_knowledge: u8,
}

/// Output of pass 1: a single decision plus its participants and
/// votes, ready to be stamped into the club's plan.
struct PendingMeeting {
    club_id: u32,
    meeting: RecruitmentMeeting,
    /// Players to surface to the manager via a staff event when the
    /// decision lands. Tuple is (staff_id, event_type).
    staff_events: Vec<(u32, StaffEventType)>,
    /// Players being added to the rejection blocklist in months.
    rejections: Vec<(u32, i64)>,
    /// `(player_id, request_id)` pairs the meeting promoted to the shortlist.
    promotions: Vec<PromotionPlan>,
}

struct PromotionPlan {
    player_id: u32,
    request_id: u32,
    consensus_score: f32,
    estimated_fee: f64,
    assessed_ability: u8,
    role_fit: f32,
    risk_flag_count: u8,
    chief_scout_support: bool,
    lead_scout_staff_id: Option<u32>,
}

impl PipelineProcessor {
    /// Public entry point. Run weekly (Monday) inside an open transfer
    /// window. Walks every initialised club, builds an agenda from the
    /// `ReportReady`/high-confidence monitoring rows + strong staff
    /// recommendations + shadow reports matching active requests, and
    /// produces a `RecruitmentMeeting` per club.
    ///
    /// No-op outside windows / non-Monday dates so it can be called
    /// unconditionally from the country tick.
    pub fn run_recruitment_meetings(country: &mut Country, date: NaiveDate) {
        if date.weekday() != chrono::Weekday::Mon {
            return;
        }

        let mut pending: Vec<PendingMeeting> = Vec::new();

        for club in &country.clubs {
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }
            if club.teams.teams.is_empty() {
                continue;
            }
            let team = &club.teams.teams[0];
            let resolved = team.staffs.resolve_for_transfers();
            if resolved.scouts.is_empty() && resolved.director_of_football.is_none() {
                // No recruitment department to speak of — meeting is
                // skipped and the manager continues to drive shortlists.
                continue;
            }

            // Build scout snapshots — one per real scout/chief on the books.
            let scout_snapshots: Vec<ScoutSnapshot> = resolved
                .scouts
                .iter()
                .filter_map(|s| {
                    let pos = s.contract.as_ref().map(|c| &c.position)?;
                    if !matches!(pos, StaffPosition::Scout | StaffPosition::ChiefScout) {
                        return None;
                    }
                    Some(ScoutSnapshot {
                        staff_id: s.id,
                        is_chief: matches!(pos, StaffPosition::ChiefScout),
                        judging_ability: s.staff_attributes.knowledge.judging_player_ability,
                        judging_potential: s.staff_attributes.knowledge.judging_player_potential,
                        discipline: s.staff_attributes.mental.discipline,
                        adaptability: s.staff_attributes.mental.adaptability,
                        determination: s.staff_attributes.mental.determination,
                        tactical_knowledge: s.staff_attributes.knowledge.tactical_knowledge,
                    })
                })
                .collect();

            // Data analyst — soft signal, NOT a vote. Drives data_support
            // on each decision so the meeting can lean on objective
            // numbers when scout judgement is split.
            let data_analyst_skill: Option<u8> = team
                .staffs
                .find_by_position(StaffPosition::DataAnalyst)
                .map(|s| s.staff_attributes.data_analysis.judging_player_data);

            let chief_scout_id: Option<u32> = team
                .staffs
                .find_by_position(StaffPosition::ChiefScout)
                .map(|s| s.id);
            let head_of_recruitment_id: Option<u32> = team
                .staffs
                .find_by_position(StaffPosition::HeadOfRecruitment)
                .map(|s| s.id);
            let dof_id: Option<u32> = resolved.director_of_football.map(|s| s.id);
            let manager_id: Option<u32> = team.staffs.manager().map(|s| s.id);

            // Meeting participants — id list for the record.
            let mut participants: Vec<u32> = Vec::new();
            for s in &scout_snapshots {
                participants.push(s.staff_id);
            }
            for id in [chief_scout_id, head_of_recruitment_id, dof_id, manager_id]
                .into_iter()
                .flatten()
            {
                if !participants.contains(&id) {
                    participants.push(id);
                }
            }
            // Data analyst attended even though they don't vote.
            if let Some(da) = team.staffs.find_by_position(StaffPosition::DataAnalyst) {
                if !participants.contains(&da.id) {
                    participants.push(da.id);
                }
            }

            let meeting_id = club.transfer_plan.next_meeting_id;
            let mut meeting = RecruitmentMeeting::new(meeting_id, date);
            meeting.participants = participants.clone();

            // Agenda: meeting-ready monitoring + high-confidence active
            // monitoring + strong staff recommendations + shadow
            // reports tied to active requests. Capped at 12 to keep
            // weekly meetings tractable.
            let mut agenda_player_ids: Vec<u32> = Vec::new();
            for m in &plan.scout_monitoring {
                if m.is_ready_for_meeting() && !agenda_player_ids.contains(&m.player_id) {
                    agenda_player_ids.push(m.player_id);
                }
            }
            // Strong staff recommendations push their candidates onto the
            // agenda even if no scout has logged enough observations yet.
            for rec in &plan.staff_recommendations {
                if rec.confidence >= 0.55
                    && !agenda_player_ids.contains(&rec.player_id)
                    && agenda_player_ids.len() < 12
                {
                    agenda_player_ids.push(rec.player_id);
                }
            }
            // Shadow reports linked to currently-active requests get a turn.
            for shadow in &plan.shadow_reports {
                if agenda_player_ids.len() >= 12 {
                    break;
                }
                let group = shadow.position_group;
                let has_open_request = plan.transfer_requests.iter().any(|r| {
                    r.position.position_group() == group
                        && r.status != TransferRequestStatus::Fulfilled
                        && r.status != TransferRequestStatus::Abandoned
                });
                if has_open_request && !agenda_player_ids.contains(&shadow.report.player_id) {
                    agenda_player_ids.push(shadow.report.player_id);
                }
            }
            if agenda_player_ids.is_empty() {
                continue;
            }

            // Track which transfer requests came up in this meeting.
            let mut agenda_request_ids: Vec<u32> = Vec::new();

            let mut decisions: Vec<RecruitmentDecision> = Vec::new();
            let mut votes: Vec<ScoutVote> = Vec::new();
            let mut staff_events: Vec<(u32, StaffEventType)> = Vec::new();
            let mut rejections: Vec<(u32, i64)> = Vec::new();
            let mut promotions: Vec<PromotionPlan> = Vec::new();

            // Per-meeting attendance event for every participant.
            for staff_id in &participants {
                staff_events.push((*staff_id, StaffEventType::RecruitmentMeeting));
            }

            for player_id in &agenda_player_ids {
                let monitorings: Vec<&ScoutPlayerMonitoring> = plan
                    .scout_monitoring
                    .iter()
                    .filter(|m| m.player_id == *player_id && m.is_active_interest())
                    .collect();

                // The transfer request the meeting will tie this decision to,
                // if any. Pick the first active request in the player's
                // position group (we already restrict by group when assigning
                // scouts, so this is essentially an alignment check).
                let request_id = monitorings
                    .iter()
                    .find_map(|m| m.transfer_request_id)
                    .or_else(|| {
                        // Shadow / staff-recommendation paths: try to align
                        // against an open request in the same group.
                        let group = monitorings.first().and_then(|m| {
                            // Position group is implicit — fall back to
                            // looking up the player position via the
                            // assignment.
                            plan.scouting_assignments
                                .iter()
                                .find(|a| Some(a.id) == m.origin_assignment_id)
                                .map(|a| a.target_position.position_group())
                        });
                        if let Some(group) = group {
                            plan.transfer_requests
                                .iter()
                                .find(|r| {
                                    r.position.position_group() == group
                                        && r.status != TransferRequestStatus::Fulfilled
                                        && r.status != TransferRequestStatus::Abandoned
                                })
                                .map(|r| r.id)
                        } else {
                            None
                        }
                    });
                if let Some(id) = request_id {
                    if !agenda_request_ids.contains(&id) {
                        agenda_request_ids.push(id);
                    }
                }

                // Aggregate role-fit, fee, etc from the strongest
                // monitoring so the dossier reflects the best-informed
                // scout's read.
                let strongest = monitorings.iter().max_by(|a, b| {
                    a.confidence
                        .partial_cmp(&b.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                let estimated_fee = strongest
                    .map(|m| m.estimated_value)
                    .or_else(|| {
                        plan.staff_recommendations
                            .iter()
                            .find(|r| r.player_id == *player_id)
                            .map(|r| r.estimated_fee)
                    })
                    .or_else(|| {
                        plan.shadow_reports
                            .iter()
                            .find(|s| s.report.player_id == *player_id)
                            .map(|s| s.report.estimated_value)
                    })
                    .unwrap_or(0.0);

                let assessed_ability = strongest
                    .map(|m| m.current_assessed_ability)
                    .or_else(|| {
                        plan.staff_recommendations
                            .iter()
                            .find(|r| r.player_id == *player_id)
                            .map(|r| r.assessed_ability)
                    })
                    .or_else(|| {
                        plan.shadow_reports
                            .iter()
                            .find(|s| s.report.player_id == *player_id)
                            .map(|s| s.report.assessed_ability)
                    })
                    .unwrap_or(0);

                let role_fit = strongest.map(|m| m.role_fit).unwrap_or(1.0);
                let risk_flag_count = strongest.map(|m| m.risk_flags.len() as u8).unwrap_or(0);

                // Budget allocation for the linked request.
                let allocated_budget = request_id
                    .and_then(|id| plan.transfer_requests.iter().find(|r| r.id == id))
                    .map(|r| r.budget_allocation.max(1.0))
                    .unwrap_or(plan.total_budget.max(1.0));
                let budget_fit = (estimated_fee / allocated_budget) as f32;

                // Cast a vote for each scout monitoring this player.
                let mut consensus_score: f32 = 0.0;
                let mut chief_scout_support = false;
                let mut player_votes: Vec<ScoutVote> = Vec::new();
                let mut total_weight: f32 = 0.0;
                for m in &monitorings {
                    let scout = match scout_snapshots
                        .iter()
                        .find(|s| s.staff_id == m.scout_staff_id)
                    {
                        Some(s) => *s,
                        None => continue,
                    };
                    let (choice, reason) = vote_from_monitoring(m, &scout, budget_fit);
                    let mut weight = choice.weight();
                    // Confidence dampens the vote — a low-confidence
                    // approval doesn't count for as much as a deeply
                    // informed one.
                    weight *= (m.confidence as f32).max(0.4);
                    // Determination amplifies a touch.
                    weight *= 1.0 + (scout.determination as f32 / 80.0).min(0.25);
                    if scout.is_chief {
                        weight *= 1.5;
                        if matches!(
                            choice,
                            ScoutVoteChoice::Approve | ScoutVoteChoice::StrongApprove
                        ) {
                            chief_scout_support = true;
                        }
                    }
                    let score = (weight as f32).clamp(-3.0, 3.0);
                    consensus_score += score;
                    total_weight += weight.abs();
                    let vote = ScoutVote {
                        scout_staff_id: scout.staff_id,
                        player_id: *player_id,
                        vote: choice,
                        score,
                        confidence: m.confidence,
                        reason,
                        date,
                    };
                    player_votes.push(vote);
                    // Surface a small staff event for the casting scout.
                    let event = match choice {
                        ScoutVoteChoice::StrongApprove | ScoutVoteChoice::Approve => {
                            StaffEventType::TargetRecommended
                        }
                        ScoutVoteChoice::Reject => StaffEventType::TargetRejected,
                        _ => StaffEventType::RecruitmentMeeting,
                    };
                    staff_events.push((scout.staff_id, event));
                }

                // Penalise heavy split — if half the votes are positive
                // and half negative, knock the consensus down.
                if !player_votes.is_empty() {
                    let pos: f32 = player_votes
                        .iter()
                        .filter(|v| v.score > 0.0)
                        .map(|v| v.score)
                        .sum();
                    let neg: f32 = player_votes
                        .iter()
                        .filter(|v| v.score < 0.0)
                        .map(|v| v.score.abs())
                        .sum();
                    if pos > 0.0 && neg > 0.0 {
                        let split_penalty = pos.min(neg) * 0.6;
                        consensus_score -= split_penalty;
                    }
                }

                // Data analyst soft signal — a flat boost if the data
                // shop's ability score is high and the player profile
                // numerically supports it. Doesn't override scout votes
                // but lifts close-call cases.
                let mut data_support = false;
                if let Some(da_skill) = data_analyst_skill {
                    let skill_floor = 11; // ~mid-tier analytics shop
                    if da_skill >= skill_floor && assessed_ability >= 100 {
                        data_support = true;
                        consensus_score += 0.4 + (da_skill as f32 - skill_floor as f32) * 0.05;
                    }
                }

                // Board risk score — used by the dossier later. Higher
                // means more friction at the board level.
                let mut board_risk: f32 = 0.0;
                board_risk += (risk_flag_count as f32) * 0.15;
                if budget_fit > 1.2 {
                    board_risk += (budget_fit - 1.2) * 0.6;
                }
                if !chief_scout_support {
                    board_risk += 0.1;
                }
                if total_weight < 1.5 {
                    // Thin discussion — board will be cautious.
                    board_risk += 0.1;
                }
                let board_risk_score = board_risk.clamp(0.0, 1.5);

                // Decision logic — driven by consensus and risk.
                let active_request =
                    request_id.and_then(|id| plan.transfer_requests.iter().find(|r| r.id == id));
                let priority_critical = active_request
                    .map(|r| {
                        matches!(
                            r.priority,
                            crate::transfers::pipeline::TransferNeedPriority::Critical
                        )
                    })
                    .unwrap_or(false);

                let decision_type: RecruitmentDecisionType;
                let reason_text: &'static str;
                if consensus_score >= 1.8 && budget_fit <= 1.4 && risk_flag_count <= 2 {
                    if priority_critical && chief_scout_support && consensus_score >= 2.5 {
                        decision_type = RecruitmentDecisionType::StartNegotiation;
                        reason_text = "critical need + strong consensus";
                    } else {
                        decision_type = RecruitmentDecisionType::PromoteToShortlist;
                        reason_text = "consensus signing";
                    }
                } else if consensus_score >= 1.0 && (budget_fit > 1.4 || risk_flag_count >= 3) {
                    decision_type = RecruitmentDecisionType::AskBoardApproval;
                    reason_text = "elevated risk — board to weigh in";
                } else if consensus_score <= -1.5 || budget_fit > 2.0 {
                    decision_type = RecruitmentDecisionType::Reject;
                    reason_text = "votes negative or budget out of reach";
                } else {
                    decision_type = RecruitmentDecisionType::KeepMonitoring;
                    reason_text = "split or insufficient confidence";
                }

                // For Reject decisions, push onto the rejection blocklist
                // for the standard window (so re-scouting doesn't re-flag).
                if matches!(decision_type, RecruitmentDecisionType::Reject) {
                    rejections.push((*player_id, 6));
                }

                // For shortlist promotions / board asks / direct
                // negotiations, queue the candidate up for the
                // pass-2 mutation.
                if matches!(
                    decision_type,
                    RecruitmentDecisionType::PromoteToShortlist
                        | RecruitmentDecisionType::AskBoardApproval
                        | RecruitmentDecisionType::StartNegotiation
                ) {
                    if let Some(req_id) = request_id {
                        let lead_scout = monitorings
                            .iter()
                            .max_by(|a, b| {
                                a.confidence
                                    .partial_cmp(&b.confidence)
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            })
                            .map(|m| m.scout_staff_id);
                        promotions.push(PromotionPlan {
                            player_id: *player_id,
                            request_id: req_id,
                            consensus_score,
                            estimated_fee,
                            assessed_ability,
                            role_fit,
                            risk_flag_count,
                            chief_scout_support,
                            lead_scout_staff_id: lead_scout,
                        });
                    }
                }

                let decision = RecruitmentDecision {
                    player_id: *player_id,
                    transfer_request_id: request_id,
                    decision: decision_type,
                    consensus_score,
                    chief_scout_support,
                    data_support,
                    board_risk_score,
                    budget_fit,
                    reason: reason_text,
                };

                debug!(
                    "Recruitment meeting (club {}): player {} -> {:?} (consensus {:.2}, votes {})",
                    club.id,
                    *player_id,
                    decision.decision,
                    consensus_score,
                    player_votes.len()
                );

                votes.extend(player_votes);
                decisions.push(decision);
            }

            meeting.player_votes = votes;
            meeting.decisions = decisions;
            meeting.agenda_request_ids = agenda_request_ids;

            pending.push(PendingMeeting {
                club_id: club.id,
                meeting,
                staff_events,
                rejections,
                promotions,
            });
        }

        // Pass 2: write decisions back into the country.
        for p in pending {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == p.club_id) {
                let plan = &mut club.transfer_plan;
                plan.next_meeting_id = p.meeting.id + 1;

                // Apply decisions: promote / monitor / reject the
                // monitoring rows so downstream consumers can read
                // status without re-running the meeting.
                for d in &p.meeting.decisions {
                    match d.decision {
                        RecruitmentDecisionType::PromoteToShortlist
                        | RecruitmentDecisionType::AskBoardApproval
                        | RecruitmentDecisionType::StartNegotiation => {
                            plan.set_monitoring_status_for_player(
                                d.player_id,
                                ScoutMonitoringStatus::PromotedToShortlist,
                            );
                        }
                        RecruitmentDecisionType::Reject => {
                            plan.set_monitoring_status_for_player(
                                d.player_id,
                                ScoutMonitoringStatus::Rejected,
                            );
                        }
                        RecruitmentDecisionType::KeepMonitoring => {
                            // Demote ReportReady -> Active so further
                            // observations are required before re-vote.
                            for m in plan.scout_monitoring.iter_mut() {
                                if m.player_id == d.player_id
                                    && matches!(m.status, ScoutMonitoringStatus::ReportReady)
                                {
                                    m.status = ScoutMonitoringStatus::Active;
                                }
                            }
                        }
                    }
                }

                // Push promotions onto the relevant shortlists.
                for promo in p.promotions {
                    apply_promotion(plan, promo);
                }

                // Rejection blocklist.
                for (player_id, months) in p.rejections {
                    plan.reject_player(player_id, date, months);
                }

                plan.push_recruitment_meeting(p.meeting);
            }

            // Emit staff events on each meeting participant.
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == p.club_id) {
                for team in &mut club.teams.teams {
                    for (staff_id, event) in &p.staff_events {
                        if let Some(staff) = team.staffs.find_mut(*staff_id) {
                            staff.add_event(event.clone());
                        }
                    }
                }
            }
        }
    }

    /// Build a transient board dossier for a candidate sitting on a
    /// shortlist. Stays alongside the rest of the recruitment helpers
    /// so callers don't need to know the meeting internals.
    pub fn build_board_dossier(
        plan: &crate::transfers::pipeline::ClubTransferPlan,
        player_id: u32,
        request_id: u32,
    ) -> BoardRecruitmentDossier {
        let monitorings: Vec<&ScoutPlayerMonitoring> = plan
            .scout_monitoring
            .iter()
            .filter(|m| m.player_id == player_id)
            .collect();

        let scout_votes = monitorings.len() as u8;
        let avg_confidence = if monitorings.is_empty() {
            0.0
        } else {
            monitorings.iter().map(|m| m.confidence).sum::<f32>() / monitorings.len() as f32
        };
        let avg_role_fit = if monitorings.is_empty() {
            1.0
        } else {
            monitorings.iter().map(|m| m.role_fit).sum::<f32>() / monitorings.len() as f32
        };
        let risk_flag_count = monitorings
            .iter()
            .map(|m| m.risk_flags.len() as u8)
            .max()
            .unwrap_or(0);
        let matches_watched = monitorings.iter().map(|m| m.matches_watched).sum::<u16>();

        // Pull the most recent decision against this player out of the
        // history. Most recent meetings have the freshest signal.
        let latest = plan
            .recruitment_meetings
            .iter()
            .rev()
            .flat_map(|m| m.decisions.iter())
            .find(|d| d.player_id == player_id);
        let consensus_score = latest.map(|d| d.consensus_score).unwrap_or(0.0);
        let chief_scout_support = latest.map(|d| d.chief_scout_support).unwrap_or(false);
        let data_support = latest.map(|d| d.data_support).unwrap_or(false);

        let allocation = plan
            .transfer_requests
            .iter()
            .find(|r| r.id == request_id)
            .map(|r| r.budget_allocation.max(1.0))
            .unwrap_or(plan.total_budget.max(1.0));
        let estimated_fee = monitorings
            .iter()
            .map(|m| m.estimated_value)
            .next()
            .unwrap_or(0.0);
        let budget_fit = (estimated_fee / allocation) as f32;

        BoardRecruitmentDossier {
            player_id,
            scout_votes,
            chief_scout_support,
            avg_confidence,
            avg_role_fit,
            risk_flag_count,
            consensus_score,
            budget_fit,
            data_support,
            matches_watched,
        }
    }
}

/// Promotion handler — adds the player to the relevant shortlist if
/// not already present. If the shortlist exists, the candidate is
/// inserted with a meeting-stamped score multiplier reflecting the
/// scout consensus. If no shortlist exists yet we create one (mirrors
/// the staff-recommendation path).
fn apply_promotion(plan: &mut crate::transfers::pipeline::ClubTransferPlan, promo: PromotionPlan) {
    let request_exists = plan
        .transfer_requests
        .iter()
        .any(|r| r.id == promo.request_id);
    if !request_exists {
        return;
    }
    // Score: weighted by consensus and role fit; risk flags reduce score.
    let consensus_factor = (promo.consensus_score / 4.0).clamp(0.0, 1.5);
    let role_factor = promo.role_fit.clamp(0.5, 1.25);
    let risk_factor = (1.0 - (promo.risk_flag_count as f32) * 0.06).max(0.5);
    let chief_factor = if promo.chief_scout_support { 1.05 } else { 1.0 };
    let score = ((promo.assessed_ability as f32 / 200.0) + 0.25 + consensus_factor)
        * role_factor
        * risk_factor
        * chief_factor;
    let candidate = ShortlistCandidate {
        player_id: promo.player_id,
        score,
        estimated_fee: promo.estimated_fee,
        status: ShortlistCandidateStatus::Available,
    };
    let _ = promo.lead_scout_staff_id; // reserved for future shortlist surface

    if let Some(shortlist) = plan
        .shortlists
        .iter_mut()
        .find(|s| s.transfer_request_id == promo.request_id)
    {
        if !shortlist
            .candidates
            .iter()
            .any(|c| c.player_id == promo.player_id)
        {
            shortlist.candidates.push(candidate);
            shortlist.candidates.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
    } else {
        let allocation = plan
            .transfer_requests
            .iter()
            .find(|r| r.id == promo.request_id)
            .map(|r| r.budget_allocation)
            .unwrap_or(promo.estimated_fee);
        let mut shortlist = TransferShortlist::new(promo.request_id, allocation);
        shortlist.candidates.push(candidate);
        plan.shortlists.push(shortlist);
        // Move the request status forward so downstream sees a shortlist.
        if let Some(req) = plan
            .transfer_requests
            .iter_mut()
            .find(|r| r.id == promo.request_id)
        {
            if matches!(
                req.status,
                TransferRequestStatus::Pending | TransferRequestStatus::ScoutingActive
            ) {
                req.status = TransferRequestStatus::Shortlisted;
            }
        }
    }
}

/// Translate a single scout's monitoring row into a vote choice + reason.
/// Pure function — no side effects, easy to unit test independently.
fn vote_from_monitoring(
    m: &ScoutPlayerMonitoring,
    scout: &ScoutSnapshot,
    budget_fit: f32,
) -> (ScoutVoteChoice, ScoutVoteReason) {
    if m.confidence < 0.45 && m.times_watched < 3 {
        return (
            ScoutVoteChoice::NeedsMoreInfo,
            ScoutVoteReason::InsufficientConfidence,
        );
    }
    if m.risk_flags.contains(&ReportRiskFlag::CurrentlyInjured) {
        return (ScoutVoteChoice::Monitor, ScoutVoteReason::InjuryConcern);
    }
    if m.risk_flags.contains(&ReportRiskFlag::WageDemands) && budget_fit > 1.4 {
        return (ScoutVoteChoice::Reject, ScoutVoteReason::WageConcern);
    }
    if budget_fit > 2.0 {
        return (ScoutVoteChoice::Reject, ScoutVoteReason::TooExpensive);
    }
    if m.risk_flags.contains(&ReportRiskFlag::PoorAttitude) && scout.discipline >= 12 {
        return (ScoutVoteChoice::Reject, ScoutVoteReason::PoorAttitude);
    }

    // Role fit drives the headline judgement. Tactical knowledge
    // sharpens the read so a high-tactical scout is more confident
    // about a marginal fit.
    let tactical_lift = scout.tactical_knowledge as f32 / 30.0; // ~0.0..0.67
    let effective_fit = m.role_fit + tactical_lift * 0.05;

    if effective_fit < 0.85 && scout.adaptability < 12 {
        return (ScoutVoteChoice::Reject, ScoutVoteReason::PoorRoleFit);
    }

    // Strong-approval gates: high effective fit, healthy assessed
    // ability, no significant risks. Strict scouts (high discipline)
    // still vote Approve rather than StrongApprove unless the gap
    // between need and player is generous.
    if effective_fit >= 1.05 && m.confidence >= 0.7 && m.risk_flags.is_empty() {
        // High-assessed / high-potential player → StrongApprove.
        if m.current_assessed_potential > m.current_assessed_ability + 8 {
            return (
                ScoutVoteChoice::StrongApprove,
                ScoutVoteReason::HighPotential,
            );
        }
        if m.current_assessed_ability >= 130 {
            return (ScoutVoteChoice::StrongApprove, ScoutVoteReason::ReadyNow);
        }
        if budget_fit < 0.7 {
            return (
                ScoutVoteChoice::StrongApprove,
                ScoutVoteReason::ValueOpportunity,
            );
        }
    }

    if effective_fit >= 1.0 && m.confidence >= 0.55 {
        return (ScoutVoteChoice::Approve, ScoutVoteReason::RoleFit);
    }

    // Moderate signals — keep monitoring.
    if m.current_assessed_potential > m.current_assessed_ability + 12 && m.confidence >= 0.5 {
        return (ScoutVoteChoice::Monitor, ScoutVoteReason::HighPotential);
    }
    let _ = scout.judging_ability;
    let _ = scout.judging_potential;
    let _ = IntegerUtils::random(0, 1); // reserved for future stochastic tilt

    (
        ScoutVoteChoice::Monitor,
        ScoutVoteReason::InsufficientConfidence,
    )
}

#[allow(dead_code)]
fn _unused_for_link(_s: &Staff, _r: &TransferRequest) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfers::pipeline::recruitment::ScoutMonitoringSource;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn baseline_scout(staff_id: u32) -> ScoutSnapshot {
        ScoutSnapshot {
            staff_id,
            is_chief: false,
            judging_ability: 14,
            judging_potential: 14,
            discipline: 12,
            adaptability: 14,
            determination: 14,
            tactical_knowledge: 13,
        }
    }

    fn fresh_monitoring(scout_id: u32, player_id: u32) -> ScoutPlayerMonitoring {
        let mut m = ScoutPlayerMonitoring::new(
            1,
            scout_id,
            player_id,
            ScoutMonitoringSource::TransferRequest,
            d(2026, 6, 1),
        );
        // Five observations - strong confidence
        for _ in 0..5 {
            m.record_observation(
                140,
                150,
                0.9,
                1.1,
                5_000_000.0,
                vec![],
                d(2026, 6, 5),
                false,
            );
        }
        m
    }

    #[test]
    fn strong_fit_high_confidence_yields_strong_approve_or_approve() {
        let scout = baseline_scout(1);
        let m = fresh_monitoring(1, 99);
        let (choice, _) = vote_from_monitoring(&m, &scout, 0.6);
        assert!(matches!(
            choice,
            ScoutVoteChoice::StrongApprove | ScoutVoteChoice::Approve
        ));
    }

    #[test]
    fn injury_flag_yields_monitor_not_reject() {
        let scout = baseline_scout(1);
        let mut m = fresh_monitoring(1, 99);
        m.risk_flags = vec![ReportRiskFlag::CurrentlyInjured];
        let (choice, _) = vote_from_monitoring(&m, &scout, 0.6);
        assert_eq!(choice, ScoutVoteChoice::Monitor);
    }

    #[test]
    fn poor_role_fit_with_low_adaptability_rejects() {
        let mut scout = baseline_scout(1);
        scout.adaptability = 8;
        let mut m = fresh_monitoring(1, 99);
        m.role_fit = 0.7;
        let (choice, reason) = vote_from_monitoring(&m, &scout, 0.6);
        assert_eq!(choice, ScoutVoteChoice::Reject);
        assert_eq!(reason, ScoutVoteReason::PoorRoleFit);
    }

    #[test]
    fn over_budget_rejects() {
        let scout = baseline_scout(1);
        let m = fresh_monitoring(1, 99);
        let (choice, reason) = vote_from_monitoring(&m, &scout, 2.5);
        assert_eq!(choice, ScoutVoteChoice::Reject);
        assert_eq!(reason, ScoutVoteReason::TooExpensive);
    }

    #[test]
    fn low_observation_count_yields_needs_more_info() {
        let scout = baseline_scout(1);
        let mut m = ScoutPlayerMonitoring::new(
            1,
            1,
            99,
            ScoutMonitoringSource::TransferRequest,
            d(2026, 6, 1),
        );
        m.record_observation(
            120,
            130,
            0.4,
            1.0,
            1_000_000.0,
            vec![],
            d(2026, 6, 1),
            false,
        );
        let (choice, _) = vote_from_monitoring(&m, &scout, 0.6);
        assert_eq!(choice, ScoutVoteChoice::NeedsMoreInfo);
    }
}

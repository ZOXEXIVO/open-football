//! Recruitment department types — the shared dossier that scouts,
//! the chief scout, and the recruitment meeting all read and write.
//!
//! Lives in its own module rather than `mod.rs` to keep that file's
//! domain inventory small. The flow is:
//!
//! 1. A `ScoutingAssignment` triggers the creation/update of a
//!    `ScoutPlayerMonitoring` row tying a specific scout to a player.
//! 2. Each subsequent observation refreshes the row's confidence,
//!    role-fit, and risk flags.
//! 3. Once weekly the recruitment meeting walks all `ReportReady`
//!    monitorings, collects scout votes, and produces
//!    `RecruitmentMeeting` records that gate downstream shortlist /
//!    board behaviour.
//!
//! Nothing here is hidden CA/PA — assessment fields are the same
//! visible-skill estimates the rest of the scouting pipeline uses.
//!
//! All types are `Clone` because the simulation occasionally clones
//! club transfer plans for diagnostics, and `Debug` for log output.

use chrono::NaiveDate;

use crate::transfers::ScoutingRegion;
use crate::transfers::pipeline::ReportRiskFlag;

// ============================================================
// Lifecycle status & origin
// ============================================================

/// Where a monitoring record sits in the recruitment lifecycle.
/// Distinct from the candidate-on-shortlist `ShortlistCandidateStatus`
/// because monitoring tracks scout interest, not pursuit progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoutMonitoringStatus {
    /// Scout is actively observing — confidence still building.
    Active,
    /// Scout temporarily set aside (player unavailable, scout busy
    /// elsewhere). Distinct from Rejected — paused targets can be
    /// resumed without going through the meeting again.
    Paused,
    /// Confidence threshold reached — meeting agenda will pick this up.
    ReportReady,
    /// Meeting voted Reject. Player goes on the rejection blocklist.
    Rejected,
    /// Meeting voted PromoteToShortlist — handed off to shortlist build.
    PromotedToShortlist,
    /// Negotiation has been opened with the selling club.
    Negotiating,
    /// Player has been signed by this club.
    Signed,
    /// Player went elsewhere — meeting decisions tracked but inactive.
    Lost,
}

/// What surfaced this player to the scouting department.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoutMonitoringSource {
    /// Player observed in service of an explicit `TransferRequest`.
    TransferRequest,
    /// Player came up through `staff_recommendations` (scout network /
    /// DoF / head coach proactive signal).
    StaffRecommendation,
    /// Player stood out in a youth/reserve match the scout attended.
    MatchStandout,
    /// Player was already in the persistent shadow squad and is being
    /// re-evaluated this window.
    ShadowReport,
    /// Player came in through `known_players` — historical knowledge
    /// from a past loan / friendly / observed match.
    KnownPlayerRefresh,
    /// Manager / staff manually flagged this player to the scouting
    /// department. Currently unused by the pipeline but reserved for
    /// future UI hooks.
    ManualFollowUp,
}

// ============================================================
// Scout-player monitoring row
// ============================================================

/// A scout's persistent file on a single player. One row per
/// (scout_staff_id, player_id) pair in the active state — the same
/// player observed by two different scouts produces two separate
/// monitoring rows so each scout's view is independent.
///
/// Kept inside `ClubTransferPlan` rather than on the staff struct
/// because the recruitment department, not the individual scout, owns
/// the shared dossier. A scout leaving the club doesn't erase what
/// they saw — successor scouts can pick the file back up.
#[derive(Debug, Clone)]
pub struct ScoutPlayerMonitoring {
    pub id: u32,
    pub scout_staff_id: u32,
    pub player_id: u32,
    /// The scouting assignment that originally surfaced this monitoring,
    /// if any. May be `None` when monitoring originates from a staff
    /// recommendation or a known-player refresh.
    pub origin_assignment_id: Option<u32>,
    /// Linked transfer-request id — drives meeting agenda priority.
    pub transfer_request_id: Option<u32>,
    pub source: ScoutMonitoringSource,
    pub status: ScoutMonitoringStatus,
    pub started_on: NaiveDate,
    pub last_observed: NaiveDate,
    /// Total live or pool observations recorded against this player.
    pub times_watched: u16,
    /// Subset of `times_watched` that came from match-day observations.
    pub matches_watched: u16,
    /// Reserved for future training/data-only checks. The pipeline
    /// reads but doesn't yet write this.
    #[allow(dead_code)]
    pub training_or_data_checks: u16,
    pub current_assessed_ability: u8,
    pub current_assessed_potential: u8,
    /// Confidence in the assessment, 0..1 — drives meeting eligibility
    /// and vote strength.
    pub confidence: f32,
    pub role_fit: f32,
    pub estimated_value: f64,
    pub risk_flags: Vec<ReportRiskFlag>,
    /// Region the scout primarily watched the player in. Drives the
    /// staff workload UI and per-region familiarity bookkeeping.
    pub region: Option<ScoutingRegion>,
}

impl ScoutPlayerMonitoring {
    /// Confidence threshold at which a monitoring row becomes meeting-eligible.
    pub const MEETING_READY_CONFIDENCE: f32 = 0.6;

    /// Convenience constructor for the canonical "first observation" row.
    pub fn new(
        id: u32,
        scout_staff_id: u32,
        player_id: u32,
        source: ScoutMonitoringSource,
        date: NaiveDate,
    ) -> Self {
        ScoutPlayerMonitoring {
            id,
            scout_staff_id,
            player_id,
            origin_assignment_id: None,
            transfer_request_id: None,
            source,
            status: ScoutMonitoringStatus::Active,
            started_on: date,
            last_observed: date,
            times_watched: 0,
            matches_watched: 0,
            training_or_data_checks: 0,
            current_assessed_ability: 0,
            current_assessed_potential: 0,
            confidence: 0.0,
            role_fit: 1.0,
            estimated_value: 0.0,
            risk_flags: Vec::new(),
            region: None,
        }
    }

    /// `true` if the row is in any state where the player still counts
    /// as actively monitored — `Active`, `Paused`, or `ReportReady`.
    /// `Negotiating`/`PromotedToShortlist` count as monitored too:
    /// scouts haven't stopped tracking the player just because the
    /// pursuit pipeline is in flight.
    pub fn is_active_interest(&self) -> bool {
        matches!(
            self.status,
            ScoutMonitoringStatus::Active
                | ScoutMonitoringStatus::Paused
                | ScoutMonitoringStatus::ReportReady
                | ScoutMonitoringStatus::PromotedToShortlist
                | ScoutMonitoringStatus::Negotiating
        )
    }

    /// `true` once a scout has accrued enough confidence for the row
    /// to enter the meeting agenda.
    pub fn is_ready_for_meeting(&self) -> bool {
        matches!(
            self.status,
            ScoutMonitoringStatus::Active | ScoutMonitoringStatus::ReportReady
        ) && self.confidence >= Self::MEETING_READY_CONFIDENCE
    }

    /// Apply a fresh observation snapshot. Mutates the row in place;
    /// pure logic so unit-testable independently of `process_scouting`.
    pub fn record_observation(
        &mut self,
        assessed_ability: u8,
        assessed_potential: u8,
        confidence: f32,
        role_fit: f32,
        estimated_value: f64,
        risk_flags: Vec<ReportRiskFlag>,
        date: NaiveDate,
        is_match: bool,
    ) {
        self.times_watched = self.times_watched.saturating_add(1);
        if is_match {
            self.matches_watched = self.matches_watched.saturating_add(1);
        }
        self.current_assessed_ability = assessed_ability;
        self.current_assessed_potential = assessed_potential;
        self.confidence = self.confidence.max(confidence);
        self.role_fit = role_fit;
        self.estimated_value = estimated_value;
        self.risk_flags = risk_flags;
        self.last_observed = date;

        if self.confidence >= Self::MEETING_READY_CONFIDENCE
            && matches!(self.status, ScoutMonitoringStatus::Active)
        {
            self.status = ScoutMonitoringStatus::ReportReady;
        }
    }
}

// ============================================================
// Scout votes
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoutVoteChoice {
    StrongApprove,
    Approve,
    Monitor,
    Reject,
    NeedsMoreInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoutVoteReason {
    /// Player is ready to slot in immediately.
    ReadyNow,
    /// Young, big assessed potential gap.
    HighPotential,
    /// Estimated value comfortably below budget allocation.
    ValueOpportunity,
    /// Strong technical/mental/physical role fit.
    RoleFit,
    /// Role-fit shortfall — would not slot into the system as-is.
    PoorRoleFit,
    InjuryConcern,
    WageConcern,
    /// Generally aged out for the role.
    AgeConcern,
    /// Determination / work-rate flagged.
    PoorAttitude,
    /// Fee far above allocation.
    TooExpensive,
    /// Confidence too low to commit a recommendation.
    InsufficientConfidence,
    /// Rep gap or financial-stance clash with the board's vision.
    BoardRisk,
}

#[derive(Debug, Clone)]
pub struct ScoutVote {
    pub scout_staff_id: u32,
    pub player_id: u32,
    pub vote: ScoutVoteChoice,
    /// Numeric weight in [-1.5, 1.5] — chief scout votes get amplified
    /// in `RecruitmentMeeting` aggregation.
    pub score: f32,
    /// Confidence the scout had at vote time. Drives whether the
    /// meeting trusts this vote at face value.
    pub confidence: f32,
    pub reason: ScoutVoteReason,
    pub date: NaiveDate,
}

impl ScoutVoteChoice {
    /// Numeric mapping used by the meeting to compute consensus.
    /// Positive = signing pressure, negative = pushback,
    /// 0 = abstain-like (Monitor / NeedsMoreInfo).
    pub fn weight(self) -> f32 {
        match self {
            ScoutVoteChoice::StrongApprove => 1.5,
            ScoutVoteChoice::Approve => 1.0,
            ScoutVoteChoice::Monitor => 0.0,
            ScoutVoteChoice::NeedsMoreInfo => -0.25,
            ScoutVoteChoice::Reject => -1.25,
        }
    }
}

// ============================================================
// Recruitment meeting & decisions
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecruitmentDecisionType {
    PromoteToShortlist,
    KeepMonitoring,
    Reject,
    /// High consensus but elevated risk / budget — board needs to weigh in.
    AskBoardApproval,
    /// Bypass shortlist scoring and start a negotiation directly. Used
    /// for high-consensus + critical-need targets.
    StartNegotiation,
}

#[derive(Debug, Clone)]
pub struct RecruitmentDecision {
    pub player_id: u32,
    pub transfer_request_id: Option<u32>,
    pub decision: RecruitmentDecisionType,
    /// Sum of weighted scout votes minus a small penalty for split votes.
    pub consensus_score: f32,
    /// `true` if the chief scout backed the player.
    pub chief_scout_support: bool,
    /// `true` if data analysis flagged the player as a strong signal.
    pub data_support: bool,
    /// Heuristic 0..1 — how risky the board should consider this on the
    /// dossier basis. Higher = more risk; the board itself adds its own
    /// financial / vision filtering on top.
    pub board_risk_score: f32,
    /// Estimated fee / budget allocation. Above 1.0 means the player's
    /// expected fee exceeds the allocated budget.
    pub budget_fit: f32,
    /// Short reason shown in the UI / staff event log.
    pub reason: &'static str,
}

#[derive(Debug, Clone)]
pub struct RecruitmentMeeting {
    pub id: u32,
    pub date: NaiveDate,
    /// Staff ids that participated — scouts, chief scout, DoF, manager,
    /// data analyst (if present).
    pub participants: Vec<u32>,
    /// Transfer request ids the meeting reviewed.
    pub agenda_request_ids: Vec<u32>,
    pub player_votes: Vec<ScoutVote>,
    pub decisions: Vec<RecruitmentDecision>,
}

impl RecruitmentMeeting {
    pub fn new(id: u32, date: NaiveDate) -> Self {
        RecruitmentMeeting {
            id,
            date,
            participants: Vec::new(),
            agenda_request_ids: Vec::new(),
            player_votes: Vec::new(),
            decisions: Vec::new(),
        }
    }

    /// Hard cap applied when archiving meetings — we keep at most this
    /// many historical meetings per club to bound memory.
    pub const HISTORY_CAP: usize = 16;
}

// ============================================================
// Board recruitment dossier — transient, never persisted
// ============================================================

/// Read-only summary the board uses when reviewing a top shortlist
/// candidate. Built per-call from the latest monitoring/meeting state.
/// Not stored on `ClubTransferPlan` — recreated each board review.
#[derive(Debug, Clone)]
pub struct BoardRecruitmentDossier {
    pub player_id: u32,
    pub scout_votes: u8,
    pub chief_scout_support: bool,
    pub avg_confidence: f32,
    pub avg_role_fit: f32,
    pub risk_flag_count: u8,
    pub consensus_score: f32,
    pub budget_fit: f32,
    pub data_support: bool,
    pub matches_watched: u16,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfers::pipeline::ClubTransferPlan;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn plan_helpers_round_trip_monitoring() {
        let mut plan = ClubTransferPlan::new();
        let id = plan.next_monitoring_id();
        let mut row = ScoutPlayerMonitoring::new(
            id,
            7,
            42,
            ScoutMonitoringSource::TransferRequest,
            d(2026, 6, 1),
        );
        row.record_observation(
            120,
            130,
            0.7,
            1.0,
            1_000_000.0,
            vec![],
            d(2026, 6, 1),
            false,
        );
        plan.scout_monitoring.push(row);

        // Active monitoring shows up in lookups.
        assert_eq!(plan.monitorings_for_player(42).len(), 1);
        assert!(plan.find_monitoring_mut(7, 42).is_some());

        // Promote and verify lookup still finds it (PromotedToShortlist
        // is still considered active interest).
        plan.set_monitoring_status_for_player(42, ScoutMonitoringStatus::PromotedToShortlist);
        assert!(
            plan.monitorings_for_player(42)
                .iter()
                .all(|m| matches!(m.status, ScoutMonitoringStatus::PromotedToShortlist))
        );

        // Mark as Lost — archive should drop it.
        plan.set_monitoring_status_for_player(42, ScoutMonitoringStatus::Lost);
        plan.archive_completed_monitoring();
        assert!(plan.monitorings_for_player(42).is_empty());
    }

    #[test]
    fn monitoring_promotes_to_report_ready_at_threshold() {
        let mut m = ScoutPlayerMonitoring::new(
            1,
            42,
            7,
            ScoutMonitoringSource::TransferRequest,
            d(2026, 6, 1),
        );
        // Below threshold — stays Active
        m.record_observation(
            120,
            130,
            0.4,
            1.0,
            1_000_000.0,
            vec![],
            d(2026, 6, 2),
            false,
        );
        assert_eq!(m.status, ScoutMonitoringStatus::Active);
        // Above threshold — promotes
        m.record_observation(
            124,
            132,
            0.7,
            1.0,
            1_000_000.0,
            vec![],
            d(2026, 6, 5),
            false,
        );
        assert_eq!(m.status, ScoutMonitoringStatus::ReportReady);
        assert!(m.is_ready_for_meeting());
    }

    #[test]
    fn monitoring_match_observation_increments_match_counter() {
        let mut m = ScoutPlayerMonitoring::new(
            1,
            42,
            7,
            ScoutMonitoringSource::MatchStandout,
            d(2026, 6, 1),
        );
        m.record_observation(120, 130, 0.5, 1.0, 1_000_000.0, vec![], d(2026, 6, 2), true);
        m.record_observation(
            122,
            132,
            0.55,
            1.0,
            1_000_000.0,
            vec![],
            d(2026, 6, 5),
            false,
        );
        assert_eq!(m.times_watched, 2);
        assert_eq!(m.matches_watched, 1);
    }

    #[test]
    fn vote_choice_weights_are_signed_consistently() {
        assert!(ScoutVoteChoice::StrongApprove.weight() > ScoutVoteChoice::Approve.weight());
        assert!(ScoutVoteChoice::Approve.weight() > ScoutVoteChoice::Monitor.weight());
        assert!(ScoutVoteChoice::Reject.weight() < 0.0);
    }
}

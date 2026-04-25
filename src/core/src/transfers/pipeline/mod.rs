mod evaluation;
mod helpers;
mod loan_market;
mod negotiations;
mod recommendations;
mod scouting;
mod shortlists;

use crate::{PlayerFieldPositionGroup, PlayerPositionType};
use chrono::NaiveDate;

// Re-export PipelineProcessor and PlayerSummary for external use
pub use self::processor::PipelineProcessor;
pub use self::processor::PlayerSummary;

mod processor {
    use crate::{PlayerFieldPositionGroup, PlayerPositionType};
    use std::collections::HashMap;

    /// PipelineProcessor handles all daily transfer pipeline logic.
    /// Uses a two-pass borrow pattern: immutable read -> collect mutations -> mutable write.
    pub struct PipelineProcessor;

    /// Info about a player in the squad for formation-based analysis.
    pub(in crate::transfers::pipeline) struct SquadPlayerInfo {
        pub player_id: u32,
        pub primary_position: PlayerPositionType,
        pub current_ability: u8,
        pub potential_ability: u8,
        pub age: u8,
        pub position_levels: HashMap<PlayerPositionType, u8>,
        pub appearances: u16,
        pub is_injured: bool,
        pub recovery_days: u16,
        #[allow(dead_code)]
        pub injury_days: u16,
    }

    #[allow(dead_code)]
    pub struct PlayerSummary {
        pub player_id: u32,
        pub club_id: u32,
        pub country_id: u32,
        pub continent_id: u32,
        pub country_code: String,
        pub player_name: String,
        pub club_name: String,
        pub position: PlayerPositionType,
        pub position_group: PlayerFieldPositionGroup,
        pub age: u8,
        pub estimated_value: f64,
        pub is_listed: bool,
        pub is_loan_listed: bool,
        pub skill_ability: u8,
        pub average_rating: f32,
        pub goals: u16,
        pub assists: u16,
        pub appearances: u16,
        pub determination: f32,
        pub work_rate: f32,
        pub composure: f32,
        pub anticipation: f32,
        pub technical_avg: f32,
        pub mental_avg: f32,
        pub physical_avg: f32,
        pub current_reputation: i16,
        pub home_reputation: i16,
        pub world_reputation: i16,
        pub country_reputation: u16,
        /// True if the player is currently injured.
        pub is_injured: bool,
        /// Months left on contract; 0 if no contract (free agent).
        pub contract_months_remaining: i16,
        pub salary: u32,
    }
}

// ============================================================
// Transfer Need Priority & Reason
// ============================================================

#[derive(Debug, Clone, PartialEq)]
pub enum TransferNeedPriority {
    Critical,
    Important,
    Optional,
}

/// Why the coach is requesting this position - derived from tactical analysis.
#[derive(Debug, Clone, PartialEq)]
pub enum TransferNeedReason {
    /// Formation requires this position and we have no one (e.g. 4-2-3-1 needs AMC, we have none)
    FormationGap,
    /// We have a player here but they're not good enough for our level
    QualityUpgrade,
    /// Only one player for a critical position - need backup
    DepthCover,
    /// Key player is aging, need successor within 1-2 seasons
    SuccessionPlanning,
    /// Young prospect with high potential to develop
    DevelopmentSigning,
    /// Staff (scout/DoF) proactively recommended this player
    StaffRecommendation,
    /// Small club needs a loan player to fill first-team spot they can't afford to buy for
    LoanToFillSquad,
    /// Need experienced player on loan to lead dressing room / mentor youth
    ExperiencedHead,
    /// Squad too small to compete — need bodies regardless of position specifics
    SquadPadding,
    /// Cheap short-term reinforcement (free agent, loan, minimal fee)
    CheapReinforcement,
    /// Loan-in to cover for long-term injury in the squad
    InjuryCoverLoan,
    /// Player available on loan who is clearly better than current options
    OpportunisticLoanUpgrade,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransferRequestStatus {
    Pending,
    ScoutingActive,
    Shortlisted,
    Negotiating,
    Fulfilled,
    Abandoned,
}

// ============================================================
// TransferRequest - Coach tells DoF what the squad needs
// The coach says WHAT position and WHY; the DoF decides HOW (buy/loan)
// ============================================================

#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub id: u32,
    pub position: PlayerPositionType,
    pub priority: TransferNeedPriority,
    pub reason: TransferNeedReason,
    pub min_ability: u8,
    pub ideal_ability: u8,
    pub preferred_age_min: u8,
    pub preferred_age_max: u8,
    pub budget_allocation: f64,
    pub status: TransferRequestStatus,
    /// Coach-specified named target — skip scouting and go straight at
    /// this player. Set by `generate_named_target_requests`. The board
    /// may still veto before scouting runs.
    pub named_target: Option<u32>,
    /// Tracks whether the board has rubber-stamped a named target. `None`
    /// for generic requests. `Some(true)` = approved; `Some(false)` =
    /// vetoed (also sets status to Abandoned).
    pub board_approved: Option<bool>,
}

impl TransferRequest {
    pub fn new(
        id: u32,
        position: PlayerPositionType,
        priority: TransferNeedPriority,
        reason: TransferNeedReason,
        min_ability: u8,
        ideal_ability: u8,
        budget_allocation: f64,
    ) -> Self {
        // Age ranges based on the reason for the request - mirrors real-world logic
        let (age_min, age_max) = match reason {
            TransferNeedReason::FormationGap | TransferNeedReason::QualityUpgrade => {
                // Need someone ready now
                match priority {
                    TransferNeedPriority::Critical => (23, 30),
                    TransferNeedPriority::Important => (21, 29),
                    TransferNeedPriority::Optional => (20, 28),
                }
            }
            TransferNeedReason::DepthCover => (20, 32),
            TransferNeedReason::SuccessionPlanning => (19, 24),
            TransferNeedReason::DevelopmentSigning => (16, 21),
            TransferNeedReason::StaffRecommendation => (18, 32),
            TransferNeedReason::LoanToFillSquad => (19, 33),
            TransferNeedReason::ExperiencedHead => (27, 36),
            TransferNeedReason::SquadPadding => (18, 35),
            TransferNeedReason::CheapReinforcement => (19, 34),
            TransferNeedReason::InjuryCoverLoan => (20, 33),
            TransferNeedReason::OpportunisticLoanUpgrade => (19, 32),
        };

        TransferRequest {
            id,
            position,
            priority,
            reason,
            min_ability,
            ideal_ability,
            preferred_age_min: age_min,
            preferred_age_max: age_max,
            budget_allocation,
            status: TransferRequestStatus::Pending,
            named_target: None,
            board_approved: None,
        }
    }
}

// ============================================================
// PlayerObservation - Tracks multi-day observations per player
// ============================================================

#[derive(Debug, Clone)]
pub struct PlayerObservation {
    pub player_id: u32,
    pub observation_count: u32,
    pub assessed_ability: u8,
    pub assessed_potential: u8,
    pub confidence: f32,
    pub last_observed: NaiveDate,
}

impl PlayerObservation {
    pub fn new(
        player_id: u32,
        assessed_ability: u8,
        assessed_potential: u8,
        date: NaiveDate,
    ) -> Self {
        PlayerObservation {
            player_id,
            observation_count: 1,
            assessed_ability,
            assessed_potential,
            confidence: 0.3,
            last_observed: date,
        }
    }

    pub fn add_observation(
        &mut self,
        assessed_ability: u8,
        assessed_potential: u8,
        date: NaiveDate,
    ) {
        self.observation_count += 1;
        let weight = 1.0 / self.observation_count as f32;
        let old_weight = 1.0 - weight;
        self.assessed_ability =
            (old_weight * self.assessed_ability as f32 + weight * assessed_ability as f32) as u8;
        self.assessed_potential = (old_weight * self.assessed_potential as f32
            + weight * assessed_potential as f32) as u8;
        self.confidence = 1.0 - (1.0 / (self.observation_count as f32 + 1.0));
        self.last_observed = date;
    }

    pub fn add_match_observation(
        &mut self,
        assessed_ability: u8,
        assessed_potential: u8,
        match_rating: f32,
        date: NaiveDate,
    ) {
        self.observation_count += 1;
        let weight = 1.0 / self.observation_count as f32;
        let old_weight = 1.0 - weight;
        self.assessed_ability =
            (old_weight * self.assessed_ability as f32 + weight * assessed_ability as f32) as u8;
        self.assessed_potential = (old_weight * self.assessed_potential as f32
            + weight * assessed_potential as f32) as u8;
        let match_rating_bonus = if match_rating > 7.0 {
            0.05
        } else if match_rating > 6.0 {
            0.02
        } else {
            0.0
        };
        self.confidence =
            (1.0 - (0.5 / (self.observation_count as f32 + 1.0)) + match_rating_bonus).min(1.0);
        self.last_observed = date;
    }
}

// ============================================================
// ScoutingAssignment - DoF assigns scouts to find candidates
// ============================================================

#[derive(Debug, Clone)]
pub struct ScoutingAssignment {
    pub id: u32,
    pub transfer_request_id: u32,
    pub scout_staff_id: Option<u32>,
    pub target_position: PlayerPositionType,
    pub min_ability: u8,
    pub preferred_age_min: u8,
    pub preferred_age_max: u8,
    pub max_budget: f64,
    pub role_profile: RoleProfile,
    pub observations: Vec<PlayerObservation>,
    pub reports_produced: u32,
    pub completed: bool,
}

/// What the club is actually looking for at the target position —
/// minimum attribute averages the scout uses to triage candidates.
/// Drives both scouting focus and shortlist scoring: a player who meets
/// the ability bar but fails the role profile scores below a slightly
/// lower-ability candidate who matches the profile.
#[derive(Debug, Clone)]
pub struct RoleProfile {
    pub min_technical_avg: f32,
    pub min_mental_avg: f32,
    pub min_physical_avg: f32,
}

impl RoleProfile {
    /// Default profile by position group, scaled with the requested ability bar.
    /// Higher min_ability requests stricter profiles.
    pub fn for_position(position: PlayerPositionType, min_ability: u8) -> Self {
        let scale = (min_ability as f32 / 20.0).clamp(0.2, 1.0);
        let (t, m, p) = match position.position_group() {
            PlayerFieldPositionGroup::Goalkeeper => (8.0, 12.0, 10.0),
            PlayerFieldPositionGroup::Defender => (9.0, 11.0, 12.0),
            PlayerFieldPositionGroup::Midfielder => (12.0, 12.0, 10.0),
            PlayerFieldPositionGroup::Forward => (13.0, 10.0, 11.0),
        };
        RoleProfile {
            min_technical_avg: t * scale,
            min_mental_avg: m * scale,
            min_physical_avg: p * scale,
        }
    }

    /// Fit score in [0.0, 1.25] — 1.0 means meets all minimums exactly,
    /// 1.25 means comfortably above, <1.0 means below in one or more buckets.
    pub fn fit(&self, technical_avg: f32, mental_avg: f32, physical_avg: f32) -> f32 {
        let t = (technical_avg / self.min_technical_avg.max(1.0)).min(1.25);
        let m = (mental_avg / self.min_mental_avg.max(1.0)).min(1.25);
        let p = (physical_avg / self.min_physical_avg.max(1.0)).min(1.25);
        // Geometric mean — a deep shortfall in one bucket drags the score down
        // more than if penalties were simply averaged.
        (t * m * p).powf(1.0 / 3.0)
    }
}

impl ScoutingAssignment {
    pub fn new(
        id: u32,
        transfer_request_id: u32,
        scout_staff_id: Option<u32>,
        target_position: PlayerPositionType,
        min_ability: u8,
        preferred_age_min: u8,
        preferred_age_max: u8,
        max_budget: f64,
    ) -> Self {
        let role_profile = RoleProfile::for_position(target_position, min_ability);
        ScoutingAssignment {
            id,
            transfer_request_id,
            scout_staff_id,
            target_position,
            min_ability,
            preferred_age_min,
            preferred_age_max,
            max_budget,
            role_profile,
            observations: Vec::new(),
            reports_produced: 0,
            completed: false,
        }
    }

    pub fn find_observation_mut(&mut self, player_id: u32) -> Option<&mut PlayerObservation> {
        self.observations
            .iter_mut()
            .find(|o| o.player_id == player_id)
    }

    pub fn has_observation_for(&self, player_id: u32) -> bool {
        self.observations.iter().any(|o| o.player_id == player_id)
    }
}

// ============================================================
// DetailedScoutingReport - Scout's final assessment (3+ obs)
// ============================================================

#[derive(Debug, Clone)]
pub struct DetailedScoutingReport {
    pub player_id: u32,
    pub assignment_id: u32,
    pub assessed_ability: u8,
    pub assessed_potential: u8,
    pub confidence: f32,
    pub estimated_value: f64,
    pub recommendation: ScoutingRecommendation,
    /// How well the player fits the assignment's role profile. Computed at
    /// report time from the scout's read of technical/mental/physical averages.
    /// ~1.0 = meets profile, <1.0 = short in key buckets, >1.0 = above.
    pub role_fit: f32,
    /// Non-fatal concerns the scout flagged — fed into shortlist scoring
    /// and negotiation acceptance without hard-blocking the report.
    pub risk_flags: Vec<ReportRiskFlag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportRiskFlag {
    /// Currently injured — bid timing risk
    CurrentlyInjured,
    /// Low determination/work_rate — character concern
    PoorAttitude,
    /// Player's reputation is far above the club's budget tier — wage risk
    WageDemands,
    /// Contract running out soon — bargain opportunity (informational)
    ContractExpiring,
    /// Player is over 30 — age risk for long-term contracts
    AgeRisk,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScoutingRecommendation {
    StrongBuy,
    Buy,
    Consider,
    Pass,
}

// ============================================================
// TransferShortlist - DoF's ranked candidate list per position
// ============================================================

#[derive(Debug, Clone, PartialEq)]
pub enum ShortlistCandidateStatus {
    Available,
    CurrentlyPursuing,
    NegotiationFailed,
    Signed,
    Unavailable,
}

/// How the DoF decided to pursue this candidate - determined at negotiation time.
#[derive(Debug, Clone, PartialEq)]
pub enum TransferApproach {
    /// Permanent transfer - club buys the player outright
    PermanentTransfer,
    /// Loan with option to buy
    LoanWithOption,
    /// Pure loan (temporary)
    Loan,
}

#[derive(Debug, Clone)]
pub struct ShortlistCandidate {
    pub player_id: u32,
    pub score: f32,
    pub estimated_fee: f64,
    pub status: ShortlistCandidateStatus,
}

#[derive(Debug, Clone)]
pub struct TransferShortlist {
    pub transfer_request_id: u32,
    pub candidates: Vec<ShortlistCandidate>,
    pub allocated_budget: f64,
    pub current_pursuit_index: usize,
}

impl TransferShortlist {
    pub fn new(transfer_request_id: u32, allocated_budget: f64) -> Self {
        TransferShortlist {
            transfer_request_id,
            candidates: Vec::new(),
            allocated_budget,
            current_pursuit_index: 0,
        }
    }

    pub fn current_candidate(&self) -> Option<&ShortlistCandidate> {
        self.candidates.get(self.current_pursuit_index)
    }

    pub fn current_candidate_mut(&mut self) -> Option<&mut ShortlistCandidate> {
        self.candidates.get_mut(self.current_pursuit_index)
    }

    pub fn advance_to_next(&mut self) -> bool {
        self.current_pursuit_index += 1;
        self.current_pursuit_index < self.candidates.len()
    }

    pub fn all_exhausted(&self) -> bool {
        self.current_pursuit_index >= self.candidates.len()
    }

    pub fn has_pursuing_candidate(&self) -> bool {
        self.candidates
            .iter()
            .any(|c| c.status == ShortlistCandidateStatus::CurrentlyPursuing)
    }
}

// ============================================================
// LoanOutCandidate - Players identified for loan out
// ============================================================

#[derive(Debug, Clone, PartialEq)]
pub enum LoanOutReason {
    /// Young player needs regular first-team football to develop (elite/continental clubs)
    NeedsGameTime,
    /// Good player but blocked by better players in same position
    BlockedByBetterPlayer,
    /// Player surplus to squad requirements
    Surplus,
    /// Club needs to reduce wage bill
    FinancialRelief,
    /// Good player not getting minutes — data-driven (appearances vs expected)
    LackOfPlayingTime,
    /// Returning from long injury, needs match fitness via loan
    PostInjuryFitness,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoanOutStatus {
    Identified,
    Listed,
    Negotiating,
    Completed,
}

#[derive(Debug, Clone)]
pub struct LoanOutCandidate {
    pub player_id: u32,
    pub reason: LoanOutReason,
    pub status: LoanOutStatus,
    pub loan_fee: f64,
}

// ============================================================
// Staff Recommendations - Proactive player identification
// ============================================================

#[derive(Debug, Clone, PartialEq)]
pub enum RecommendationSource {
    ScoutNetwork,
    ChiefScoutReport,
    DirectorOfFootball,
    /// Head coach identifies a player they want
    HeadCoach,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RecommendationType {
    /// Contract <= 6 months
    ExpiringContract,
    /// Club in debt
    FinancialDistress,
    /// Good player at lower-rep club
    ReadyForStepUp,
    /// Young + high potential gap
    HiddenGem,
    /// Loan-listed and fits squad
    LoanOpportunity,
    /// Cheap/free loan available — perfect for small clubs
    CheapLoanAvailable,
    /// Player completely out of contract, can sign for free
    FreeAgentBargain,
    /// Experienced player on loan who could mentor younger squad members
    ExperiencedLoanMentor,
    /// Player from bigger club's surplus — quality above what small club normally gets
    BigClubSurplus,
    /// Player who wants first-team football and would accept lower-level club for game time
    GameTimeSeeker,
    /// Affordable player who would improve the weakest position in the squad
    WeakSpotFix,
    /// Player stood out in a youth/reserve match observed by a scout
    YouthMatchStandout,
}

#[derive(Debug, Clone)]
pub struct StaffRecommendation {
    pub player_id: u32,
    pub recommender_staff_id: u32,
    pub source: RecommendationSource,
    pub recommendation_type: RecommendationType,
    pub assessed_ability: u8,
    pub assessed_potential: u8,
    pub confidence: f32,
    pub estimated_fee: f64,
    pub date_recommended: NaiveDate,
}

/// Persistent club-level knowledge of a player. Unlike active scouting
/// assignments, this survives transfers and loan returns, so a club can
/// remember a foreign player who spent a few months in its league.
#[derive(Debug, Clone)]
pub struct KnownPlayerMemory {
    pub player_id: u32,
    pub last_known_club_id: u32,
    pub last_known_country_id: u32,
    pub position: PlayerPositionType,
    pub position_group: PlayerFieldPositionGroup,
    pub assessed_ability: u8,
    pub assessed_potential: u8,
    pub confidence: f32,
    pub estimated_fee: f64,
    pub last_seen: NaiveDate,
    pub official_appearances_seen: u16,
    pub friendly_appearances_seen: u16,
}

// ============================================================
// ScoutMatchAssignment - Scout assigned to watch a youth/reserve match
// ============================================================

#[derive(Debug, Clone)]
pub struct ScoutMatchAssignment {
    pub scout_staff_id: u32,
    pub target_team_id: u32,
    pub target_club_id: u32,
    pub linked_assignment_ids: Vec<u32>,
    pub last_attended: Option<NaiveDate>,
}

// ============================================================
// ClubTransferPlan - Top-level state per club
// ============================================================

#[derive(Debug, Clone)]
pub struct ClubTransferPlan {
    pub total_budget: f64,
    pub spent: f64,
    pub reserved: f64,

    pub transfer_requests: Vec<TransferRequest>,
    pub scouting_assignments: Vec<ScoutingAssignment>,
    pub scouting_reports: Vec<DetailedScoutingReport>,
    pub shortlists: Vec<TransferShortlist>,

    pub loan_out_candidates: Vec<LoanOutCandidate>,

    pub staff_recommendations: Vec<StaffRecommendation>,

    pub scout_match_assignments: Vec<ScoutMatchAssignment>,

    pub max_concurrent_negotiations: u32,
    pub active_negotiation_count: u32,

    pub next_request_id: u32,
    pub next_assignment_id: u32,

    pub last_evaluation_date: Option<NaiveDate>,
    pub initialized: bool,

    /// Players recently rejected by scouts — (player_id, until_date).
    /// Skipped during future scouting observations until `until_date`.
    /// Prevents re-scouting the same dud repeatedly in the same window.
    pub rejected_players: Vec<(u32, NaiveDate)>,

    /// Reports carried over between transfer windows — a persistent shadow
    /// squad built up over time. On window start these seed new shortlists
    /// instead of forcing a cold-start scouting pass each cycle.
    pub shadow_reports: Vec<ShadowReport>,

    /// Persistent knowledge gathered from scouting and match exposure.
    pub known_players: Vec<KnownPlayerMemory>,
}

/// A scouting report preserved past its originating assignment, used to
/// bootstrap future shortlists without discarding long-tracked targets.
#[derive(Debug, Clone)]
pub struct ShadowReport {
    pub report: DetailedScoutingReport,
    pub position_group: PlayerFieldPositionGroup,
    pub observed_ability: u8,
    pub recorded_on: NaiveDate,
}

impl ClubTransferPlan {
    pub fn new() -> Self {
        ClubTransferPlan {
            total_budget: 0.0,
            spent: 0.0,
            reserved: 0.0,
            transfer_requests: Vec::new(),
            scouting_assignments: Vec::new(),
            scouting_reports: Vec::new(),
            shortlists: Vec::new(),
            loan_out_candidates: Vec::new(),
            staff_recommendations: Vec::new(),
            scout_match_assignments: Vec::new(),
            max_concurrent_negotiations: 2,
            active_negotiation_count: 0,
            next_request_id: 1,
            next_assignment_id: 1,
            last_evaluation_date: None,
            initialized: false,
            rejected_players: Vec::new(),
            shadow_reports: Vec::new(),
            known_players: Vec::new(),
        }
    }

    /// True if a player is on the blocklist for the given date.
    pub fn is_rejected(&self, player_id: u32, date: NaiveDate) -> bool {
        self.rejected_players
            .iter()
            .any(|(id, until)| *id == player_id && *until > date)
    }

    /// Mark a player as rejected for the next `months` calendar months.
    pub fn reject_player(&mut self, player_id: u32, date: NaiveDate, months: i64) {
        let until = date + chrono::Duration::days(months * 30);
        if let Some(existing) = self
            .rejected_players
            .iter_mut()
            .find(|(id, _)| *id == player_id)
        {
            existing.1 = until.max(existing.1);
        } else {
            self.rejected_players.push((player_id, until));
        }
    }

    /// Purge expired entries.
    pub fn prune_rejected(&mut self, date: NaiveDate) {
        self.rejected_players.retain(|(_, until)| *until > date);
    }

    pub fn available_budget(&self) -> f64 {
        (self.total_budget - self.spent - self.reserved).max(0.0)
    }

    pub fn next_request_id(&mut self) -> u32 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }

    pub fn next_assignment_id(&mut self) -> u32 {
        let id = self.next_assignment_id;
        self.next_assignment_id += 1;
        id
    }

    pub fn can_start_negotiation(&self) -> bool {
        self.active_negotiation_count < self.max_concurrent_negotiations
    }

    pub fn has_pending_requests(&self) -> bool {
        self.transfer_requests
            .iter()
            .any(|r| r.status == TransferRequestStatus::Pending)
    }

    pub fn reset_for_window(&mut self) {
        // Archive reports from the closing window so year-over-year tracking
        // isn't lost — scouts don't forget every player every summer.
        self.archive_reports_to_shadow();

        self.transfer_requests.clear();
        self.scouting_assignments.clear();
        self.scouting_reports.clear();
        self.shortlists.clear();
        self.loan_out_candidates.clear();
        self.staff_recommendations.clear();
        self.scout_match_assignments.clear();
        self.active_negotiation_count = 0;
        self.spent = 0.0;
        self.reserved = 0.0;
        self.initialized = false;
        self.last_evaluation_date = None;
    }

    /// Move the current window's scouting reports into the persistent shadow
    /// squad. Keeps only the strongest N per position group to bound growth.
    pub fn archive_reports_to_shadow(&mut self) {
        use std::collections::HashMap;
        const SHADOW_CAP_PER_GROUP: usize = 15;

        if self.scouting_reports.is_empty() {
            return;
        }

        let assign_lookup: HashMap<u32, &ScoutingAssignment> = self
            .scouting_assignments
            .iter()
            .map(|a| (a.id, a))
            .collect();
        let today = self
            .last_evaluation_date
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());

        for report in &self.scouting_reports {
            // Skip reports we've already shadowed (e.g. in-window archive calls).
            if self
                .shadow_reports
                .iter()
                .any(|s| s.report.player_id == report.player_id)
            {
                continue;
            }
            // Only keep reports for non-Pass recommendations — Pass-flagged
            // players are already on the rejection blocklist.
            if matches!(report.recommendation, ScoutingRecommendation::Pass) {
                continue;
            }
            let group = match assign_lookup.get(&report.assignment_id) {
                Some(a) => a.target_position.position_group(),
                None => continue,
            };
            self.shadow_reports.push(ShadowReport {
                report: report.clone(),
                position_group: group,
                observed_ability: report.assessed_ability,
                recorded_on: today,
            });
        }

        // Cap per position group: keep best by assessed_ability × confidence
        for group in [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ] {
            let mut indices: Vec<usize> = self
                .shadow_reports
                .iter()
                .enumerate()
                .filter(|(_, s)| s.position_group == group)
                .map(|(i, _)| i)
                .collect();
            if indices.len() <= SHADOW_CAP_PER_GROUP {
                continue;
            }
            indices.sort_by(|a, b| {
                let sa = &self.shadow_reports[*a];
                let sb = &self.shadow_reports[*b];
                let score_a = sa.report.assessed_ability as f32 * sa.report.confidence;
                let score_b = sb.report.assessed_ability as f32 * sb.report.confidence;
                score_b
                    .partial_cmp(&score_a)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let to_drop: Vec<usize> = indices.into_iter().skip(SHADOW_CAP_PER_GROUP).collect();
            // Drop in reverse to preserve indices
            let mut sorted = to_drop;
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            for idx in sorted {
                self.shadow_reports.swap_remove(idx);
            }
        }
    }

    /// Rehydrate shadow reports into the active window's report pool for any
    /// ScoutingAssignment whose position group matches. Gives newly opened
    /// windows a warm start instead of rescouting from scratch.
    pub fn seed_active_reports_from_shadow(&mut self) {
        if self.shadow_reports.is_empty() || self.scouting_assignments.is_empty() {
            return;
        }
        let assignments: Vec<(u32, PlayerFieldPositionGroup)> = self
            .scouting_assignments
            .iter()
            .map(|a| (a.id, a.target_position.position_group()))
            .collect();

        for shadow in &self.shadow_reports {
            // Bind this shadow report to the first matching active assignment.
            // Dedupe against existing active reports for the same player.
            if let Some((assign_id, _)) = assignments
                .iter()
                .find(|(_, g)| *g == shadow.position_group)
            {
                let already_active = self
                    .scouting_reports
                    .iter()
                    .any(|r| r.player_id == shadow.report.player_id);
                if already_active {
                    continue;
                }
                let mut seeded = shadow.report.clone();
                seeded.assignment_id = *assign_id;
                // Shadow confidence decays with age — a 12-month-old report is
                // meaningfully less sharp than a fresh one.
                seeded.confidence = (seeded.confidence * 0.7).clamp(0.2, 1.0);
                self.scouting_reports.push(seeded);
            }
        }
    }

    pub fn remember_known_player(&mut self, memory: KnownPlayerMemory) {
        const KNOWN_CAP: usize = 120;

        if let Some(existing) = self
            .known_players
            .iter_mut()
            .find(|m| m.player_id == memory.player_id)
        {
            let old_weight = existing.confidence.max(0.1);
            let new_weight = memory.confidence.max(0.1);
            let total = old_weight + new_weight;

            existing.assessed_ability = ((existing.assessed_ability as f32 * old_weight
                + memory.assessed_ability as f32 * new_weight)
                / total)
                .round()
                .clamp(1.0, 200.0) as u8;
            existing.assessed_potential =
                existing.assessed_potential.max(memory.assessed_potential);
            existing.confidence = (existing.confidence + memory.confidence * 0.35).min(0.95);
            existing.estimated_fee = memory.estimated_fee;
            existing.last_known_club_id = memory.last_known_club_id;
            existing.last_known_country_id = memory.last_known_country_id;
            existing.position = memory.position;
            existing.position_group = memory.position_group;
            existing.last_seen = memory.last_seen;
            existing.official_appearances_seen = existing
                .official_appearances_seen
                .saturating_add(memory.official_appearances_seen);
            existing.friendly_appearances_seen = existing
                .friendly_appearances_seen
                .saturating_add(memory.friendly_appearances_seen);
        } else {
            self.known_players.push(memory);
        }

        if self.known_players.len() > KNOWN_CAP {
            self.known_players.sort_by(|a, b| {
                let score_a = a.assessed_ability as f32 * a.confidence;
                let score_b = b.assessed_ability as f32 * b.confidence;
                score_b
                    .partial_cmp(&score_a)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            self.known_players.truncate(KNOWN_CAP);
        }
    }

    pub fn known_player(&self, player_id: u32) -> Option<&KnownPlayerMemory> {
        self.known_players.iter().find(|m| m.player_id == player_id)
    }
}

impl Default for ClubTransferPlan {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod known_player_memory_tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn memory(player_id: u32, ability: u8, confidence: f32, date: NaiveDate) -> KnownPlayerMemory {
        KnownPlayerMemory {
            player_id,
            last_known_club_id: 10,
            last_known_country_id: 1,
            position: PlayerPositionType::ForwardCenter,
            position_group: PlayerFieldPositionGroup::Forward,
            assessed_ability: ability,
            assessed_potential: ability.saturating_add(10),
            confidence,
            estimated_fee: 1_000_000.0,
            last_seen: date,
            official_appearances_seen: 1,
            friendly_appearances_seen: 0,
        }
    }

    #[test]
    fn known_player_memory_updates_existing_record() {
        let mut plan = ClubTransferPlan::new();
        plan.remember_known_player(memory(99, 90, 0.4, d(2026, 7, 1)));
        plan.remember_known_player(memory(99, 110, 0.5, d(2026, 7, 8)));

        let known = plan.known_player(99).unwrap();
        assert_eq!(known.player_id, 99);
        assert!(known.assessed_ability > 90);
        assert!(known.confidence > 0.4);
        assert_eq!(known.official_appearances_seen, 2);
        assert_eq!(known.last_seen, d(2026, 7, 8));
    }
}

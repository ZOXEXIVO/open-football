pub mod ai;
pub mod club;
pub mod competitions;
pub mod config;
pub mod context;
pub mod continent;
pub mod country;
pub mod league;
pub mod r#match;
pub mod performance;
pub mod shared;
pub mod simulator;
pub mod transfers;
pub mod utils;

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub use ai::*;
pub use competitions::*;
pub use config::SimulatorConfig;
pub use continent::national::world::EmergencyCallupMetrics;
pub use continent::national::{
    CompetitionPhase, CompetitionScope, FixtureResult, GroupFixture, GroupStanding,
    KnockoutBracket, KnockoutFixture, KnockoutResult, KnockoutRound, NationalCompetitionConfig,
    NationalCompetitionFixture, NationalCompetitionPhase, NationalTeamCompetition,
    NationalTeamCompetitions, QualifyingConfig, QualifyingGroup, QualifyingPosition,
    QualifyingZoneConfig, ScheduleConfig, ScheduleDate, TournamentConfig,
};
// Namespace conflicting CompetitionType enums
// Country's CompetitionType is for continental competitions (ChampionsLeague, etc.)
pub use country::CompetitionType as ContinentalCompetitionType;
pub use country::{
    CallUpReason, Country, CountryContext, CountryEconomicFactors, CountryGeneratorData,
    CountryPricing, CountryRegulations, CountryResult, CountrySettings, InternationalCompetition,
    MediaCoverage, MediaStory, NationalSelectionPolicy, NationalSquadPlayer, NationalTeam,
    NationalTeamFixture, NationalTeamLevel, NationalTeamMatchResult, NationalTeamStaffMember,
    NationalTeamStaffRole, PeopleNameGeneratorData, SkinColorDistribution, SquadPick, StoryType,
};
pub use nalgebra::*;
pub use performance::{PerfCounters, PerfPhase, PerfSnapshot, PhaseScope, TickEndContext};
pub use simulator::*;
pub use utils::*;

// Re-export club items
pub use club::{
    AcademyGenerationContext,
    AcademyIntakeState,
    AcceptContractHandler,
    Achievement,
    AchievementType,
    AgePreference,
    AwardReputationInput,
    AwardReputationKind,
    AwardTimelineEntry,
    BoardResponsibility,
    CONDITION_MAX_VALUE,
    CareerDesireEventContext,
    CareerDesireEvidence,
    CareerDesireKind,
    ChangeType,
    ChemistryFactors,
    // Club itself
    Club,
    ClubBoard,
    ClubColors,
    ClubContext,
    ClubFacilities,
    ClubFinanceContext,
    ClubFinanceResult,
    // Finance exports
    ClubFinances,
    ClubFinancialBalance,
    ClubFinancialBalanceHistory,
    ClubMood,
    ClubPhilosophy,
    ClubResult,
    ClubSponsorship,
    ClubSponsorshipContract,
    // Status & mood
    ClubStatus,
    // Transfers exports
    ClubTransferStrategy,
    CoachFocus,
    CoachingPhilosophy,
    CoachingStyle,
    CompetitionStatistics,
    ConflictInfo,
    ConflictLocation,
    ConflictSeverity,
    ConflictType,
    ContractBonus,
    ContractBonusType,
    ContractClause,
    ContractClauseType,
    ContractEventContext,
    ContractEventEvidence,
    ContractEventKind,
    ContractRenewalResponsibility,
    ContractType,
    DomesticCupOverride,
    FacilityLevel,
    FacilityQuality,
    FormationChange,
    Goalkeeping,
    HappinessEvent,
    HappinessEventCause,
    HappinessEventChangeKind,
    HappinessEventContext,
    HappinessEventEvidence,
    HappinessEventFollowUp,
    HappinessEventScope,
    HappinessEventSeverity,
    HappinessEventType,
    HappinessFactors,
    HealthIssue,
    IncomingTransfersResponsibility,
    IndividualTrainingPlan,
    InfluenceLevel,
    InjuryRecoveryEventContext,
    InjuryRecoveryEvidence,
    InjuryRecoveryStage,
    InjurySeverity,
    InjuryType,
    Language,
    LeadershipEventContext,
    LeadershipEventKind,
    LifeSimulationDesireContext,
    LifeSimulationDesireKind,
    LifeSimulationSeverity,
    LifeSimulationTrigger,
    LiveCupSlice,
    LoanEventContext,
    LoanEventKind,
    ManagerCriticismReason,
    ManagerInteractionEventContext,
    ManagerInteractionTone,
    ManagerInteractionTopic,
    ManagerTalkResult,
    ManagerTalkType,
    MatchHistory,
    MatchHistoryItem,
    MatchOutcome,
    MatchPerformanceEventContext,
    MatchPerformanceEvidence,
    MatchPerformanceKind,
    MatchResultInfo,
    MatchSelectionContext,
    MatchTacticType,
    MediaFanEventContext,
    MediaFanEventKind,
    MediaFanSource,
    Mental,
    MentalFocusType,
    MentalGains,
    MentorshipType,
    NationalTeamEventContext,
    NationalTeamEventKind,
    NegativeHappiness,
    NegotiationPolicy,
    OutgoingTransfersResponsibility,
    PeriodizationPhase,
    // Person exports
    Person,
    PersonAttributes,
    PersonBehaviour,
    PersonBehaviourState,
    PersonalAdaptationEventContext,
    PersonalAdaptationKind,
    Physical,
    PhysicalFocusType,
    PhysicalGains,
    // Player exports
    Player,
    PlayerAcceptance,
    PlayerAttributes,
    PlayerAwardsCount,
    PlayerBehaviourResult,
    PlayerBuilder,
    PlayerClubContract,
    PlayerCollection,
    PlayerCollectionResult,
    PlayerCompetitionStatsRow,
    PlayerContext,
    PlayerContractProposal,
    PlayerContractResult,
    PlayerDecision,
    PlayerDecisionHistory,
    PlayerFieldPositionGroup,
    PlayerGenerator,
    PlayerHappiness,
    PlayerHistoryRow,
    PlayerHistoryRowBreakdown,
    PlayerLanguage,
    PlayerLiveStatsInput,
    PlayerMailbox,
    PlayerMailboxResult,
    PlayerMessage,
    PlayerMessageType,
    PlayerPlan,
    PlayerPlanRole,
    PlayerPosition,
    PlayerPositionType,
    PlayerPositions,
    PlayerPreferredFoot,
    PlayerRelation,
    PlayerRelationshipChangeResult,
    PlayerResult,
    PlayerSkills,
    PlayerSquadStatus,
    PlayerStatCompetitionKind,
    PlayerStatLedgerEntry,
    PlayerStatistics,
    PlayerStatisticsHistory,
    PlayerStatisticsHistoryItem,
    PlayerStatisticsProjection,
    PlayerStatus,
    PlayerStatusType,
    PlayerTraining,
    PlayerTrainingHistory,
    PlayerTrainingLoad,
    PlayerTrainingResult,
    PlayerTransferStatus,
    PlayerUtils,
    PlayerValueCalculator,
    PositionWeights,
    PositiveHappiness,
    ProcessContractHandler,
    PromiseKind,
    RecognitionEventContext,
    RecognitionEventKind,
    RecommendationCategory,
    RecommendationPriority,
    RecruitmentPolicy,
    RecruitmentResponsibility,
    RegionFamiliarity,
    RegulationEventContext,
    RegulationOutcomeKind,
    RegulationSlotKind,
    // Relations exports
    Relations,
    RelationshipChange,
    RelationshipEvent,
    ReputationLevel,
    ReputationRequirements,
    ReputationTrend,
    ResignationReason,
    RoleStatusEventContext,
    RoleStatusKind,
    RotationPreference,
    ScoutRecommendation,
    ScoutingReport,
    ScoutingResponsibility,
    SeasonOutcomeContext,
    SeasonOutcomeKind,
    SelectionComparison,
    SelectionDecisionScope,
    SelectionOmissionReason,
    SelectionRole,
    SelectionScoreFactor,
    SellOnObligation,
    SellingDecision,
    SellingPolicy,
    SkillType,
    SpecialInstruction,
    SquadAnalysis,
    SquadBuildingPolicy,
    SquadPhase,
    // Staff exports
    Staff,
    StaffAttributes,
    StaffClubContract,
    StaffCoaching,
    StaffCollection,
    StaffCollectionResult,
    StaffContext,
    StaffContractResult,
    StaffDataAnalysis,
    StaffEvent,
    StaffEventType,
    StaffGoalkeeperCoaching,
    StaffKnowledge,
    StaffLicenseType,
    StaffMedical,
    StaffMental,
    StaffMoraleEvent,
    StaffPerformance,
    StaffPosition,
    StaffRelation,
    StaffResponsibility,
    StaffResult,
    StaffStatus,
    StaffStub,
    StaffTrainingResult,
    StaffTrainingSession,
    StaffWarning,
    StatusData,
    SupportEventContext,
    SupportMatchPhase,
    SupportSetting,
    SupportSource,
    SupportTone,
    SupportTrigger,
    TACTICS_POSITIONS,
    TacticSelectionReason,
    TacticalDecisionEngine,
    TacticalDecisionResult,
    TacticalFocus,
    TacticalRecommendation,
    TacticalStyle,
    Tactics,
    TacticsSelector,
    // Team exports
    Team,
    TeamBehaviour,
    TeamBehaviourResult,
    TeamBuilder,
    TeamCollection,
    TeamCompetitionType,
    TeamContext,
    TeamInfo,
    TeamReputation,
    TeamResult,
    TeamTraining,
    TeamTrainingResult,
    TeamType,
    TeammateConflictContext,
    TeammateConflictReason,
    Technical,
    TechnicalFocusType,
    TechnicalGains,
    TrainingEffects,
    // Phase 1-11: structured event-context payloads
    TrainingEventContext,
    TrainingEventEvidence,
    TrainingEventReason,
    TrainingFacilities,
    TrainingFocus,
    TrainingIntensity,
    TrainingIntensityPreference,
    TrainingLoadManager,
    TrainingRecord,
    TrainingResponsibility,
    TrainingSchedule,
    TrainingSession,
    TrainingType,
    TransferInterestContext,
    TransferInterestDecision,
    TransferInterestEvidence,
    TransferInterestKind,
    TransferInterestReaction,
    TransferInterestReason,
    TransferInterestRisk,
    TransferInterestScore,
    TransferInterestSource,
    TransferInterestStage,
    TransferItem,
    TransferSportingFit,
    TransferStrategyContext,
    Transfers,
    TrophyEventContext,
    TrophyKind,
    WageCalculator,
    WeeklyTrainingPlan,
    // Modules
    academy,
    behaviour,
    board,
    collection,
    handlers,
    matches,
    mood,
    next_player_id,

    player_attributes_mod,
    player_builder_mod,
    player_context,
    player_contract_mod,
    reputation,
    seed_player_id_sequence as seed_core_player_id_sequence,
    staff_attributes_mod,
    staff_context,
    staff_contract_mod,
    tactics,
    team_builder_mod,
    team_context,
    team_training_mod,
    team_transfers_mod,
    transfers as club_transfers,
};

// Re-export shot-gate diagnostic counters for the dev stats harness.
// Only compiled with the `match-logs` feature.
#[cfg(feature = "match-logs")]
pub use crate::r#match::engine::player::events::players::save_accounting_stats;
#[cfg(feature = "match-logs")]
pub use crate::r#match::engine::player::strategies::forwarders::states::running::shot_gate_stats;
#[cfg(feature = "match-logs")]
pub use crate::r#match::engine::player::strategies::forwarders::states::running::tackle_stats;
#[cfg(feature = "match-logs")]
pub use crate::r#match::player::strategies::players::ops::forward_shot_decision::helper_diag;
#[cfg(feature = "match-logs")]
pub use crate::r#match::player::strategies::players::ops::forward_shot_decision::mid_run_diag;

static STORE_MATCH_EVENTS_MODE: AtomicBool = AtomicBool::new(false);
static MATCH_RECORDINGS_MODE: AtomicBool = AtomicBool::new(false);
static MATCH_STORE_MAX_THREADS: AtomicUsize = AtomicUsize::new(4);
static MATCH_ENGINE_POOL: OnceLock<r#match::MatchPlayEnginePool> = OnceLock::new();

/// Process-global match-engine runtime configuration and the shared engine
/// pool. The web crate flips these flags at startup (see `settings.rs`) and
/// the engine/orchestrator read them back; grouping them behind one facade
/// keeps the otherwise-scattered statics discoverable.
pub struct MatchRuntime;

impl MatchRuntime {
    pub fn set_events_mode(enabled: bool) {
        STORE_MATCH_EVENTS_MODE.store(enabled, Ordering::SeqCst);
    }

    pub fn events_mode() -> bool {
        STORE_MATCH_EVENTS_MODE.load(Ordering::SeqCst)
    }

    pub fn set_recordings_mode(enabled: bool) {
        MATCH_RECORDINGS_MODE.store(enabled, Ordering::SeqCst);
    }

    pub fn recordings_mode() -> bool {
        MATCH_RECORDINGS_MODE.load(Ordering::SeqCst)
    }

    pub fn set_store_max_threads(n: usize) {
        MATCH_STORE_MAX_THREADS.store(n, Ordering::SeqCst);
    }

    pub fn store_max_threads() -> usize {
        MATCH_STORE_MAX_THREADS.load(Ordering::SeqCst)
    }

    /// Eagerly build the shared engine pool with a fixed worker count.
    /// No-op if the pool was already initialised.
    pub fn init_engine_pool(num_threads: usize) {
        MATCH_ENGINE_POOL.get_or_init(|| r#match::MatchPlayEnginePool::new(num_threads));
    }

    /// Borrow the shared engine pool, lazily initialising it sized to the
    /// available parallelism if `init_engine_pool` was never called.
    pub fn engine_pool() -> &'static r#match::MatchPlayEnginePool {
        MATCH_ENGINE_POOL.get_or_init(|| {
            let cpus = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4);
            r#match::MatchPlayEnginePool::new(cpus)
        })
    }
}

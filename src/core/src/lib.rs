use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static STORE_MATCH_EVENTS_MODE: AtomicBool = AtomicBool::new(false);
static MATCH_RECORDINGS_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_match_events_mode(enabled: bool) {
    STORE_MATCH_EVENTS_MODE.store(enabled, Ordering::SeqCst);
}

pub fn is_match_events_mode() -> bool {
    STORE_MATCH_EVENTS_MODE.load(Ordering::SeqCst)
}

pub fn set_match_recordings_mode(enabled: bool) {
    MATCH_RECORDINGS_MODE.store(enabled, Ordering::SeqCst);
}

pub fn is_match_recordings_mode() -> bool {
    MATCH_RECORDINGS_MODE.load(Ordering::SeqCst)
}

static MATCH_STORE_MAX_THREADS: AtomicUsize = AtomicUsize::new(4);

pub fn set_match_store_max_threads(n: usize) {
    MATCH_STORE_MAX_THREADS.store(n, Ordering::SeqCst);
}

pub fn match_store_max_threads() -> usize {
    MATCH_STORE_MAX_THREADS.load(Ordering::SeqCst)
}

static MATCH_ENGINE_POOL: OnceLock<r#match::MatchPlayEnginePool> = OnceLock::new();

pub fn init_match_engine_pool(num_threads: usize) {
    MATCH_ENGINE_POOL.get_or_init(|| r#match::MatchPlayEnginePool::new(num_threads));
}

pub fn match_engine_pool() -> &'static r#match::MatchPlayEnginePool {
    MATCH_ENGINE_POOL.get_or_init(|| {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        r#match::MatchPlayEnginePool::new(cpus)
    })
}

// Re-export shot-gate diagnostic counters for the dev stats harness.
// Only compiled with the `match-logs` feature.
#[cfg(feature = "match-logs")]
pub use crate::r#match::engine::player::strategies::forwarders::states::running::shot_gate_stats;
#[cfg(feature = "match-logs")]
pub use crate::r#match::engine::player::strategies::forwarders::states::running::tackle_stats;
#[cfg(feature = "match-logs")]
pub use crate::r#match::engine::player::events::players::save_accounting_stats;

pub mod config;
pub mod simulator;
pub use config::SimulatorConfig;
pub use continent::national::world::emergency_callups_total;
pub use simulator::*;

pub mod club;
pub mod competitions;
pub mod context;
pub mod continent;
pub mod country;
pub mod league;
pub mod r#match;
pub mod transfers;

pub mod ai;
pub mod shared;
pub mod utils;

// Re-export club items
pub use club::{
    AcademyGenerationContext,
    AcademyIntakeState,
    AcceptContractHandler,
    Achievement,
    AchievementType,
    BoardResponsibility,
    CONDITION_MAX_VALUE,
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
    ConflictInfo,
    ConflictSeverity,
    ConflictType,
    ContractBonus,
    ContractBonusType,
    ContractClause,
    ContractClauseType,
    ContractRenewalResponsibility,
    ContractType,
    FacilityLevel,
    FacilityQuality,
    FormationChange,
    Goalkeeping,
    HappinessEvent,
    HappinessEventType,
    HappinessFactors,
    HealthIssue,
    IncomingTransfersResponsibility,
    IndividualTrainingPlan,
    InfluenceLevel,
    InjurySeverity,
    InjuryType,
    Language,
    ManagerTalkResult,
    ManagerTalkType,
    MatchHistory,
    MatchHistoryItem,
    MatchOutcome,
    MatchResultInfo,
    MatchTacticType,
    Mental,
    MentalFocusType,
    MentalGains,
    MentorshipType,
    NegativeHappiness,
    OutgoingTransfersResponsibility,
    PeriodizationPhase,
    // Person exports
    Person,
    PersonAttributes,
    PersonBehaviour,
    PersonBehaviourState,
    Physical,
    PhysicalFocusType,
    PhysicalGains,
    // Player exports
    Player,
    PlayerAttributes,
    PlayerBehaviourResult,
    PlayerBuilder,
    PlayerClubContract,
    PlayerCollection,
    PlayerCollectionResult,
    PlayerContext,
    PlayerContractProposal,
    PlayerContractResult,
    PlayerDecision,
    PlayerDecisionHistory,
    PlayerFieldPositionGroup,
    PlayerGenerator,
    PlayerHappiness,
    PlayerLanguage,
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
    PlayerStatistics,
    PlayerStatisticsHistory,
    PlayerStatisticsHistoryItem,
    PlayerCareerSpell,
    CareerSpellKind,
    CareerEventKind,
    RootKind,
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
    RecommendationCategory,
    RecommendationPriority,
    RecruitmentResponsibility,
    RegionFamiliarity,
    // Relations exports
    Relations,
    RelationshipChange,
    RelationshipEvent,
    ReputationLevel,
    ReputationRequirements,
    ReputationTrend,
    ResignationReason,
    RotationPreference,
    ScoutRecommendation,
    ScoutingReport,
    ScoutingResponsibility,
    SellOnObligation,
    SkillType,
    SpecialInstruction,
    SquadAnalysis,
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
    Technical,
    TechnicalFocusType,
    TechnicalGains,
    TrainingEffects,
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
    TransferItem,
    Transfers,
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

// Re-export country items
pub use country::{
    CallUpReason, Country, CountryContext, CountryEconomicFactors, CountryGeneratorData,
    CountryPricing, CountryRegulations, CountryResult, CountrySettings, InternationalCompetition,
    MediaCoverage, MediaStory, NationalSquadPlayer, NationalTeam, NationalTeamFixture,
    NationalTeamMatchResult, NationalTeamStaffMember, NationalTeamStaffRole,
    PeopleNameGeneratorData, SkinColorDistribution, SquadPick, StoryType,
};

pub use continent::national::{
    CompetitionPhase, CompetitionScope, FixtureResult, GroupFixture, GroupStanding,
    KnockoutBracket, KnockoutFixture, KnockoutResult, KnockoutRound, NationalCompetitionConfig,
    NationalCompetitionFixture, NationalCompetitionPhase, NationalTeamCompetition,
    NationalTeamCompetitions, QualifyingConfig, QualifyingGroup, QualifyingPosition,
    QualifyingZoneConfig, ScheduleConfig, ScheduleDate, TournamentConfig,
};

pub use competitions::*;

// Namespace conflicting CompetitionType enums
// Country's CompetitionType is for continental competitions (ChampionsLeague, etc.)
pub use country::CompetitionType as ContinentalCompetitionType;

pub use ai::*;
pub use nalgebra::*;
pub use utils::*;

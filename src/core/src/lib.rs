use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::OnceLock;

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

#[macro_use]
pub mod match_logs;

// Re-export shot-gate diagnostic counters for the dev stats harness.
// Only compiled with the `match-logs` feature.
#[cfg(feature = "match-logs")]
pub use crate::r#match::engine::player::strategies::forwarders::states::running::shot_gate_stats;
#[cfg(feature = "match-logs")]
pub use crate::r#match::engine::player::strategies::forwarders::states::running::tackle_stats;

pub mod simulator;
pub mod config;
pub use simulator::*;
pub use config::SimulatorConfig;
pub use continent::national::world::emergency_callups_total;

pub mod club;
pub mod context;
pub mod continent;
pub mod country;
pub mod competitions;
pub mod league;
pub mod r#match;
pub mod transfers;

pub mod shared;
pub mod utils;
pub mod ai;

// Re-export club items
pub use club::{
    // Modules
    academy, board, mood, transfers as club_transfers,
    // Person exports
    Person, PersonAttributes, PersonBehaviour, PersonBehaviourState,
    // Club itself
    Club, ClubBoard, ClubColors, ClubFacilities, ClubPhilosophy, ClubResult,
    ClubContext, FacilityLevel,
    // Finance exports
    ClubFinances, ClubFinancialBalance, ClubFinancialBalanceHistory,
    ClubSponsorship, ClubSponsorshipContract,
    ClubFinanceContext, ClubFinanceResult,
    // Relations exports
    Relations, PlayerRelation, StaffRelation, ChemistryFactors,
    ChangeType, MentorshipType, InfluenceLevel, ConflictType, ConflictSeverity,
    RelationshipChange, ConflictInfo,
    // Transfers exports
    ClubTransferStrategy,
    // Player exports
    Player, PlayerCollection, PlayerBuilder, SellOnObligation,
    PlayerAttributes, PlayerContext,
    PlayerPreferredFoot, PlayerPositionType, PlayerFieldPositionGroup, PlayerStatusType,
    PlayerSkills, Technical, Mental, Physical, Goalkeeping,
    PlayerPositions, PlayerPosition, PlayerStatus, StatusData,
    PlayerStatistics, PlayerStatisticsHistory, PlayerStatisticsHistoryItem, TeamInfo,
    PlayerHappiness, PositiveHappiness, NegativeHappiness,
    HappinessFactors, HappinessEvent, HappinessEventType,
    PlayerClubContract, ContractType, PlayerSquadStatus, PlayerTransferStatus,
    ContractBonusType, ContractBonus, ContractClauseType, ContractClause,
    PlayerMailbox, PlayerMessage, PlayerMessageType, PlayerContractProposal, PlayerMailboxResult,
    AcceptContractHandler, ProcessContractHandler, handlers,
    PlayerDecisionHistory, PlayerDecision,
    PlayerTraining, PlayerTrainingHistory, TrainingRecord, PlayerTrainingResult,
    PlayerResult, PlayerCollectionResult, PlayerContractResult,
    PlayerValueCalculator, WageCalculator, PlayerGenerator, PlayerUtils,
    seed_player_id_sequence as seed_core_player_id_sequence, next_player_id,

    PlayerPlan, PlayerPlanRole,
    Language, PlayerLanguage,
    InjuryType, InjurySeverity,
    player_context, player_attributes_mod, player_contract_mod, player_builder_mod,
    CONDITION_MAX_VALUE,
    // Staff exports
    Staff, StaffCollection, StaffStub,
    StaffAttributes, StaffContext,
    StaffCoaching, StaffGoalkeeperCoaching, StaffMental,
    StaffKnowledge, StaffDataAnalysis, StaffMedical, RegionFamiliarity,
    StaffClubContract, StaffPosition, StaffStatus,
    CoachFocus, TechnicalFocusType, MentalFocusType, PhysicalFocusType,
    StaffResponsibility, BoardResponsibility, RecruitmentResponsibility,
    IncomingTransfersResponsibility, OutgoingTransfersResponsibility,
    ContractRenewalResponsibility, ScoutingResponsibility, TrainingResponsibility,
    StaffPerformance, CoachingStyle,
    StaffResult, StaffCollectionResult, StaffContractResult, StaffTrainingResult,
    StaffWarning, StaffMoraleEvent, ResignationReason, HealthIssue,
    RelationshipEvent, StaffLicenseType, StaffTrainingSession,
    StaffEvent, StaffEventType,
    ScoutingReport, ScoutRecommendation,
    staff_context, staff_attributes_mod, staff_contract_mod,
    // Team exports
    Team, TeamCollection, TeamType, TeamBuilder, TeamContext,
    TeamResult,
    TeamBehaviour, TeamBehaviourResult, PlayerBehaviourResult, PlayerRelationshipChangeResult,
    ManagerTalkResult, ManagerTalkType,
    Tactics, TacticalStyle, MatchTacticType, TacticSelectionReason, TacticsSelector,
    TacticalDecisionEngine, TacticalDecisionResult, FormationChange, SquadAnalysis,
    TacticalRecommendation, RecommendationPriority, RecommendationCategory,
    TeamTraining, TeamTrainingResult, TrainingSchedule, TrainingType,
    TrainingSession, TrainingIntensity, WeeklyTrainingPlan, PeriodizationPhase,
    TrainingEffects, PhysicalGains, TechnicalGains, MentalGains,
    IndividualTrainingPlan, TrainingFocus, SkillType, SpecialInstruction,
    CoachingPhilosophy, TacticalFocus, TrainingIntensityPreference, RotationPreference,
    TrainingFacilities, FacilityQuality, TrainingLoadManager, PlayerTrainingLoad,
    Transfers, TransferItem,
    MatchHistory, MatchHistoryItem,
    TeamReputation, ReputationLevel, ReputationTrend, Achievement, AchievementType,
    MatchResultInfo, MatchOutcome, ReputationRequirements,
    team_context, team_builder_mod, team_training_mod, team_transfers_mod,
    behaviour, collection, matches, reputation, tactics,
    TACTICS_POSITIONS,
    TeamCompetitionType,
    // Status & mood
    ClubStatus, ClubMood,
};

// Re-export country items
pub use country::{
    Country, CountryResult, CountryContext,
    CountryEconomicFactors, InternationalCompetition, MediaCoverage,
    MediaStory, StoryType, CountryRegulations, CountryGeneratorData,
    PeopleNameGeneratorData, CountrySettings, CountryPricing, SkinColorDistribution,
    NationalTeam, NationalTeamStaffMember, NationalTeamStaffRole,
    NationalSquadPlayer, NationalTeamFixture, NationalTeamMatchResult,
    CallUpReason, SquadPick,
};

pub use continent::national::{
    NationalTeamCompetitions, NationalCompetitionFixture, NationalCompetitionPhase,
    NationalTeamCompetition, NationalCompetitionConfig, CompetitionScope,
    QualifyingPosition, QualifyingConfig, QualifyingZoneConfig,
    TournamentConfig, ScheduleConfig, ScheduleDate,
    CompetitionPhase, QualifyingGroup, GroupStanding, GroupFixture, FixtureResult,
    KnockoutBracket, KnockoutRound, KnockoutFixture, KnockoutResult,
};

pub use competitions::*;

// Namespace conflicting CompetitionType enums
// Country's CompetitionType is for continental competitions (ChampionsLeague, etc.)
pub use country::CompetitionType as ContinentalCompetitionType;

pub use nalgebra::*;
pub use utils::*;
pub use ai::*;
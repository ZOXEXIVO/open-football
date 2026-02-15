pub mod academy;
pub mod board;
pub mod club;
pub mod context;
pub mod finance;
pub mod mood;
pub mod person;
pub mod player;
pub mod relations;
pub mod result;
pub mod staff;
pub mod status;
pub mod team;
pub mod transfers;

// Re-export all simple modules
pub use board::*;
pub use club::*;
pub use context::*;
pub use mood::*;
pub use person::*;
pub use result::*;
pub use status::*;

// Finance exports
pub use finance::{
    ClubFinances, ClubFinancialBalance, ClubFinancialBalanceHistory,
    ClubSponsorship, ClubSponsorshipContract,
    ClubFinanceContext, ClubFinanceResult,
};

// Relations exports
pub use relations::{
    Relations, PlayerRelation, StaffRelation, ChemistryFactors,
    ChangeType, MentorshipType, InfluenceLevel, ConflictType, ConflictSeverity,
    RelationshipChange, ConflictInfo,
};

// Transfers exports
pub use transfers::{
    ClubTransferStrategy,
};

// Player exports (except conflicting modules)
pub use player::{
    Player, PlayerCollection, PlayerBuilder,
    PlayerAttributes, PlayerContext,
    PlayerPreferredFoot, PlayerPositionType, PlayerFieldPositionGroup, PlayerStatusType,
    PlayerSkills, Technical, Mental, Physical,
    PlayerPositions, PlayerPosition, PlayerStatus, StatusData,
    PlayerStatistics, PlayerStatisticsHistory, PlayerStatisticsHistoryItem,
    PlayerHappiness, PositiveHappiness, NegativeHappiness,
    PlayerClubContract, ContractType, PlayerSquadStatus, PlayerTransferStatus,
    ContractBonusType, ContractBonus, ContractClauseType, ContractClause,
    PlayerMailbox, PlayerMessage, PlayerMessageType, PlayerContractProposal, PlayerMailboxResult,
    PlayerTraining, PlayerTrainingHistory, TrainingRecord,
    PlayerResult, PlayerCollectionResult, PlayerContractResult,
    PlayerValueCalculator, PlayerGenerator, PlayerUtils,
    InjuryType, InjurySeverity,
    CONDITION_MAX_VALUE,
};
// Also export the missing types
pub use player::training::result::PlayerTrainingResult;
pub use player::mailbox::handlers::{AcceptContractHandler, ProcessContractHandler};
// Also export context module for those who want to import from it
pub use player::context as player_context;
// Also keep module aliases for those who want to import from the module
pub use player::attributes as player_attributes_mod;
pub use player::contract as player_contract_mod;
pub use player::builder as player_builder_mod;
pub use player::mailbox::handlers;

// Staff exports (except conflicting modules)
pub use staff::{
    Staff, StaffCollection, StaffStub,
    StaffAttributes, StaffContext,
    StaffCoaching, StaffGoalkeeperCoaching, StaffMental,
    StaffKnowledge, StaffDataAnalysis, StaffMedical,
    StaffClubContract, StaffPosition, StaffStatus,
    CoachFocus, TechnicalFocusType, MentalFocusType, PhysicalFocusType,
    StaffResponsibility, BoardResponsibility, RecruitmentResponsibility,
    IncomingTransfersResponsibility, OutgoingTransfersResponsibility,
    ContractRenewalResponsibility, ScoutingResponsibility, TrainingResponsibility,
    StaffPerformance, CoachingStyle,
    StaffResult, StaffCollectionResult, StaffContractResult, StaffTrainingResult,
    StaffWarning, StaffMoraleEvent, ResignationReason, HealthIssue,
    RelationshipEvent, StaffLicenseType, StaffTrainingSession,
    ScoutingReport, ScoutRecommendation,
};
// Also export context module for those who want to import from it
pub use staff::context as staff_context;
pub use staff::focus;
pub use staff::responsibility;
pub use staff::staff_stub;
// Also keep module aliases for those who want to import from the module
pub use staff::attributes as staff_attributes_mod;
pub use staff::contract as staff_contract_mod;

// Team exports (except conflicting modules)
pub use team::{
    Team, TeamCollection, TeamType, TeamBuilder, TeamContext,
    TeamResult,
    TeamBehaviour, TeamBehaviourResult, PlayerBehaviourResult, PlayerRelationshipChangeResult,
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
    TACTICS_POSITIONS,
};
// Also export context module for those who want to import from it
pub use team::context as team_context;
pub use team::behaviour;
pub use team::collection;
pub use team::matches;
pub use team::reputation;
pub use team::tactics;
pub use team::training as team_training_mod;
pub use team::transfers as team_transfers_mod;
// Also keep module aliases for those who want to import from the module
pub use team::builder as team_builder_mod;
// Note: team's CompetitionType is exported but will conflict in lib.rs
pub use team::reputation::CompetitionType as TeamCompetitionType;

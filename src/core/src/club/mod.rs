pub mod academy;
pub mod ai;
pub mod board;
pub mod club;
pub mod context;
pub mod facilities;
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
pub use facilities::*;
pub use mood::*;
pub use person::*;
pub use result::*;
pub use status::*;

// Finance exports
pub use finance::{
    ClubFinanceContext, ClubFinanceResult, ClubFinances, ClubFinancialBalance,
    ClubFinancialBalanceHistory, ClubSponsorship, ClubSponsorshipContract, DistressLevel,
    SponsorPerformance, SponsorRenewalContext, TransferObligation, classify_distress,
};

// Relations exports
pub use relations::{
    ChangeType, ChemistryFactors, ConflictInfo, ConflictSeverity, ConflictType, InfluenceLevel,
    MentorshipType, PlayerRelation, Relations, RelationshipChange, StaffRelation,
};

// Transfers exports
pub use transfers::ClubTransferStrategy;

// Player exports (except conflicting modules)
pub use player::{
    AcademyGenerationContext, AcademyIntakeState, CONDITION_MAX_VALUE, ContractBonus,
    ContractBonusType, ContractClause, ContractClauseType, ContractType, Goalkeeping,
    HappinessEvent, HappinessEventType, HappinessFactors, InjurySeverity, InjuryType, Language,
    Mental, NegativeHappiness, Physical, Player, PlayerAttributes, PlayerBuilder,
    PlayerClubContract, PlayerCollection, PlayerCollectionResult, PlayerContext,
    PlayerContractProposal, PlayerContractResult, PlayerDecision, PlayerDecisionHistory,
    PlayerFieldPositionGroup, PlayerGenerator, PlayerHappiness, PlayerLanguage, PlayerMailbox,
    PlayerMailboxResult, PlayerMessage, PlayerMessageType, PlayerPlan, PlayerPlanRole,
    PlayerPosition, PlayerPositionType, PlayerPositions, PlayerPreferredFoot, PlayerResult,
    PlayerSkills, PlayerSquadStatus, PlayerStatistics, PlayerStatisticsHistory,
    PlayerStatisticsHistoryItem, PlayerStatus, PlayerStatusType, PlayerTraining,
    PlayerTrainingHistory, PlayerTransferStatus, PlayerUtils, PlayerValueCalculator,
    PositionWeights, PositiveHappiness, SellOnObligation, StatusData, TeamInfo, Technical,
    TrainingRecord, WageCalculator, next_player_id, seed_player_id_sequence,
};
// Also export the missing types
pub use player::mailbox::handlers::{AcceptContractHandler, ProcessContractHandler};
pub use player::training::result::PlayerTrainingResult;
// Also export context module for those who want to import from it
pub use player::context as player_context;
// Also keep module aliases for those who want to import from the module
pub use player::attributes as player_attributes_mod;
pub use player::builder as player_builder_mod;
pub use player::contract as player_contract_mod;
pub use player::mailbox::handlers;

// Staff exports (except conflicting modules)
pub use staff::{
    BoardResponsibility, CoachFocus, CoachingStyle, ContractRenewalResponsibility, HealthIssue,
    IncomingTransfersResponsibility, MentalFocusType, OutgoingTransfersResponsibility,
    PhysicalFocusType, RecruitmentResponsibility, RegionFamiliarity, RelationshipEvent,
    ResignationReason, ScoutRecommendation, ScoutingReport, ScoutingResponsibility, Staff,
    StaffAttributes, StaffClubContract, StaffCoaching, StaffCollection, StaffCollectionResult,
    StaffContext, StaffContractResult, StaffDataAnalysis, StaffEvent, StaffEventType,
    StaffGoalkeeperCoaching, StaffKnowledge, StaffLicenseType, StaffMedical, StaffMental,
    StaffMoraleEvent, StaffPerformance, StaffPosition, StaffResponsibility, StaffResult,
    StaffStatus, StaffStub, StaffTrainingResult, StaffTrainingSession, StaffWarning,
    TechnicalFocusType, TrainingResponsibility,
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
    Achievement, AchievementType, CoachingPhilosophy, FacilityQuality, FormationChange,
    IndividualTrainingPlan, ManagerTalkResult, ManagerTalkType, MatchHistory, MatchHistoryItem,
    MatchOutcome, MatchResultInfo, MatchTacticType, MentalGains, PeriodizationPhase, PhysicalGains,
    PlayerBehaviourResult, PlayerRelationshipChangeResult, PlayerTrainingLoad,
    RecommendationCategory, RecommendationPriority, ReputationLevel, ReputationRequirements,
    ReputationTrend, RotationPreference, SkillType, SpecialInstruction, SquadAnalysis,
    TACTICS_POSITIONS, TacticSelectionReason, TacticalDecisionEngine, TacticalDecisionResult,
    TacticalFocus, TacticalRecommendation, TacticalStyle, Tactics, TacticsSelector, Team,
    TeamBehaviour, TeamBehaviourResult, TeamBuilder, TeamCollection, TeamContext, TeamReputation,
    TeamResult, TeamTraining, TeamTrainingResult, TeamType, TechnicalGains, TrainingEffects,
    TrainingFacilities, TrainingFocus, TrainingIntensity, TrainingIntensityPreference,
    TrainingLoadManager, TrainingSchedule, TrainingSession, TrainingType, TransferItem, Transfers,
    WeeklyTrainingPlan,
};
// Also export context module for those who want to import from it
pub use team::behaviour;
pub use team::collection;
pub use team::context as team_context;
pub use team::matches;
pub use team::reputation;
pub use team::tactics;
pub use team::training as team_training_mod;
pub use team::transfers as team_transfers_mod;
// Also keep module aliases for those who want to import from the module
pub use team::builder as team_builder_mod;
// Note: team's CompetitionType is exported but will conflict in lib.rs
pub use team::reputation::CompetitionType as TeamCompetitionType;

use std::sync::atomic::{AtomicBool, Ordering};

static STORE_MATCH_EVENTS_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_match_events_mode(enabled: bool) {
    STORE_MATCH_EVENTS_MODE.store(enabled, Ordering::SeqCst);
}

pub fn is_match_events_mode() -> bool {
    STORE_MATCH_EVENTS_MODE.load(Ordering::SeqCst)
}

pub mod simulator;
pub use simulator::*;

pub mod club;
pub mod context;
pub mod continent;
pub mod country;
pub mod league;
pub mod r#match;
pub mod transfers;

pub mod shared;
pub mod utils;

// Re-export club items
pub use club::{
    // Modules
    academy, board, mood, transfers as club_transfers,
    // Person exports
    Person, PersonAttributes, PersonBehaviour, PersonBehaviourState,
    // Club itself
    Club, ClubBoard, ClubColors, ClubResult,
    ClubContext,
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
    AcceptContractHandler, ProcessContractHandler, handlers,
    PlayerTraining, PlayerTrainingHistory, TrainingRecord, PlayerTrainingResult,
    PlayerResult, PlayerCollectionResult, PlayerContractResult,
    PlayerValueCalculator, PlayerGenerator, PlayerUtils,
    InjuryType, InjurySeverity,
    player_context, player_attributes_mod, player_contract_mod, player_builder_mod,
    CONDITION_MAX_VALUE,
    // Staff exports
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
    staff_context, staff_attributes_mod, staff_contract_mod,
    // Team exports
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
    PeopleNameGeneratorData, CountrySettings, CountryPricing,
};

// Namespace conflicting CompetitionType enums
// Country's CompetitionType is for continental competitions (ChampionsLeague, etc.)
pub use country::CompetitionType as ContinentalCompetitionType;

pub use nalgebra::*;
pub use utils::*;

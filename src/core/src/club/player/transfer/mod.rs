//! The player's own side of the transfer market.
//!
//! [`availability`] is what the market can see of him, [`free`] is his life
//! once his contract lapses, [`stage`] is his appetite for a bigger one, and
//! [`processing`] is where a desire to leave is detected in the first place.

pub mod availability;
pub mod free;
pub mod processing;
pub mod stage;

pub use availability::{AvailabilityBlockReason, AvailabilityMarketState, MarketResignation};
pub use free::{
    FreeAgentBlockReason, FreeAgentMarketState, FreeAgentStatusCategory,
    FreeAgentStatusExplanation, MarketStage, PreContractAgreement, ReleaseContext,
};
pub use processing::{
    ContinentalAccessContext, ContinentalCompetitionTier, ContinentalPathHeuristic,
    EuropeanAmbitionConfig, TransferDesireContext,
};
pub use stage::{BigStagePull, BigStagePullConfig, BigStagePullContext};

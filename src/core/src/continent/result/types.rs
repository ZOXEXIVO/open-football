use crate::continent::{CompetitionTier, ContinentalMatchResult, ContinentalRankings};
use crate::r#match::MatchResult;
use crate::transfers::CompletedTransfer;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ContinentalCompetitionResults {
    pub champions_league_results: Option<Vec<ContinentalMatchResult>>,
    pub europa_league_results: Option<Vec<ContinentalMatchResult>>,
    pub conference_league_results: Option<Vec<ContinentalMatchResult>>,
    /// Real match results from continental competitions for stat processing
    pub match_results: Vec<MatchResult>,
}

impl ContinentalCompetitionResults {
    pub fn new() -> Self {
        ContinentalCompetitionResults {
            champions_league_results: None,
            europa_league_results: None,
            conference_league_results: None,
            match_results: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContinentalRankingsUpdate {
    pub country_updates: Vec<(u32, f32)>, // country_id, new coefficient
    pub club_updates: Vec<(u32, f32)>,    // club_id, new points
    pub qualification_changes: Vec<QualificationChange>,
}

impl ContinentalRankingsUpdate {
    pub fn from_rankings(rankings: ContinentalRankings) -> Self {
        ContinentalRankingsUpdate {
            country_updates: rankings.country_rankings,
            club_updates: rankings.club_rankings,
            qualification_changes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct QualificationChange {
    pub country_id: u32,
    pub competition: CompetitionTier,
    pub old_spots: u8,
    pub new_spots: u8,
}

#[derive(Debug, Clone)]
pub struct CrossBorderTransferSummary {
    pub completed_transfers: Vec<CompletedTransfer>,
    pub total_value: f64,
    pub most_expensive: Option<CompletedTransfer>,
    pub by_country_flow: HashMap<u32, TransferFlow>, // country_id -> flow stats
}

#[derive(Debug, Clone)]
pub struct TransferFlow {
    pub incoming_transfers: u32,
    pub outgoing_transfers: u32,
    pub net_spend: f64,
}

#[derive(Debug, Clone)]
pub struct EconomicZoneImpact {
    pub economic_multiplier: f32,
    pub tv_rights_change: f64,
    pub sponsorship_change: f64,
    pub overall_health_change: f32,
}

use crate::{Club, Country, PlayerPositionType};
use crate::transfers::negotiation::{NegotiationPhase};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct TransferActivitySummary {
    pub(crate) total_listings: u32,
    pub(crate) active_negotiations: u32,
    pub(crate) completed_transfers: u32,
    pub(crate) total_fees_exchanged: f64,
}

impl TransferActivitySummary {
    pub(crate) fn new() -> Self {
        TransferActivitySummary {
            total_listings: 0,
            active_negotiations: 0,
            completed_transfers: 0,
            total_fees_exchanged: 0.0,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn get_market_heat_index(&self) -> f32 {
        let activity = (self.active_negotiations as f32 + self.completed_transfers as f32) / 100.0;
        activity.min(1.0)
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct SquadAnalysis {
    pub(crate) surplus_positions: Vec<PlayerPositionType>,
    pub(crate) needed_positions: Vec<PlayerPositionType>,
    pub(crate) average_age: f32,
    pub(crate) quality_level: u8,
}

/// Internal data extracted from a negotiation for phase resolution
pub(crate) struct NegotiationData {
    pub(crate) player_id: u32,
    pub(crate) selling_club_id: u32,
    pub(crate) buying_club_id: u32,
    pub(crate) offer_amount: f64,
    pub(crate) is_loan: bool,
    pub(crate) is_unsolicited: bool,
    pub(crate) phase: NegotiationPhase,
    pub(crate) selling_rep: f32,
    pub(crate) buying_rep: f32,
    pub(crate) player_age: u8,
    pub(crate) player_ambition: f32,
    pub(crate) asking_price: f64,
    pub(crate) is_listed: bool,
    /// Country the player is sold from (None = same as buying country)
    pub(crate) selling_country_id: Option<u32>,
    /// Cached names for cross-country (player not accessible from buying country)
    pub(crate) player_name: String,
    pub(crate) selling_club_name: String,
}

/// A completed negotiation that needs execution at SimulatorData level.
/// Used for ALL transfers — both domestic and cross-country.
pub(crate) struct DeferredTransfer {
    pub(crate) player_id: u32,
    pub(crate) selling_country_id: u32,
    pub(crate) selling_club_id: u32,
    pub(crate) buying_country_id: u32,
    pub(crate) buying_club_id: u32,
    pub(crate) fee: f64,
    pub(crate) is_loan: bool,
}

pub(crate) fn can_club_accept_player(club: &Club) -> bool {
    let max_squad = club.board.season_targets
        .as_ref()
        .map(|t| t.max_squad_size as usize)
        .unwrap_or(50);
    // Only count main team players for squad cap — youth/reserve teams are separate
    let main_squad = club.teams.teams.first()
        .map(|t| t.players.players.len())
        .unwrap_or(0);
    main_squad < max_squad
}

pub(crate) fn find_player_in_country(country: &Country, player_id: u32) -> Option<&crate::Player> {
    for club in &country.clubs {
        for team in &club.teams.teams {
            if let Some(player) = team.players.players.iter().find(|p| p.id == player_id) {
                return Some(player);
            }
        }
    }
    None
}

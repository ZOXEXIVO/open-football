use crate::transfers::market::TransferListingOrigin;
use crate::transfers::negotiation::NegotiationPhase;
use crate::transfers::offer::{PersonalTermsOffer, TransferClause};
use crate::{Club, Country, Player, PlayerPositionType};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct TransferActivitySummary {
    pub(crate) total_listings: u32,
    pub(crate) active_negotiations: u32,
    pub(crate) completed_transfers: u32,
    pub(crate) total_fees_exchanged: f64,
    /// Pre-contract (Bosman) free moves executed in this country's local
    /// pass — a subset of the in-country free-agent signings. Carried up
    /// through `DeferredTransferOps` so Phase C can split the monthly
    /// "signed pre-contract" vs "signed off a domestic expiry" diagnostics.
    pub(crate) signed_pre_contract: u32,
}

impl TransferActivitySummary {
    pub(crate) fn new() -> Self {
        TransferActivitySummary {
            total_listings: 0,
            active_negotiations: 0,
            completed_transfers: 0,
            total_fees_exchanged: 0.0,
            signed_pre_contract: 0,
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
    pub(crate) has_option_to_buy: bool,
    pub(crate) is_unsolicited: bool,
    pub(crate) phase: NegotiationPhase,
    pub(crate) selling_rep: f32,
    pub(crate) buying_rep: f32,
    pub(crate) player_age: u8,
    pub(crate) player_ambition: f32,
    pub(crate) asking_price: f64,
    /// True when ANY listing exists for the player at the selling club.
    /// Includes synthetic listings created by the pipeline to back an
    /// unsolicited approach — useful for plumbing, not for acceptance
    /// scoring (use [`player_is_available`] for that).
    #[allow(dead_code)]
    pub(crate) has_market_listing: bool,
    /// True when the player is genuinely available — covers explicit
    /// status flags (Lst/Loa/Req/Unh), `NotNeeded` squad status, and
    /// seller-advertised listings (Lst/Loa/EndOfContract). Synthetic
    /// listings backing unsolicited approaches do NOT set this to true.
    pub(crate) player_is_available: bool,
    /// Why the active listing exists (if any). None when there is no
    /// listing at all. Resolver uses this to distinguish seller-listed
    /// players from those carried only by a synthetic stub.
    #[allow(dead_code)]
    pub(crate) listing_origin: Option<TransferListingOrigin>,
    /// Country the player is sold from (None = same as buying country)
    pub(crate) selling_country_id: Option<u32>,
    /// Selling country's continent_id (for geographic preference)
    pub(crate) selling_continent_id: Option<u32>,
    /// Selling country's code (for geographic preference)
    pub(crate) selling_country_code: String,
    /// If buying club previously sold this player: (club_id, fee)
    pub(crate) player_sold_from: Option<(u32, f64)>,
    /// Cached names for cross-country (player not accessible from buying country)
    pub(crate) player_name: String,
    pub(crate) selling_club_name: String,
    /// Annual wage the buying club has staged for PersonalTerms.
    pub(crate) offered_annual_wage: Option<u32>,
    /// Buying club's league reputation (0–10000). Used to anchor the
    /// player's reservation wage and for wage installation fallback.
    pub(crate) buying_league_reputation: u16,
    /// Sell-on percentage pledged in the buyer's offer, owed to the current
    /// seller on the player's next sale.
    pub(crate) sell_on_percentage: Option<f32>,
    /// Loan option/obligation fee and whether it is mandatory.
    pub(crate) loan_future_fee: Option<(u32, bool)>,
    /// Structured personal-terms package the buyer has staged on the
    /// current offer. Carried through resolution so PersonalTerms can
    /// score against signing bonus / agent fee / role promise, and so
    /// `resolve_medical` can include the full package in the deferred
    /// transfer for execution to install.
    pub(crate) personal_terms: Option<PersonalTermsOffer>,
    /// Foreign moves only: the player would refuse on willingness grounds
    /// (clear step down, no availability signal), captured from the full
    /// cross-border assessment at negotiation creation. Applied as the
    /// foreign personal-terms hard floor — the buyer's country can't
    /// recompute it (the seller-side data lives abroad). `false` for
    /// domestic moves, whose floor is recomputed live.
    pub(crate) foreign_terms_floor_blocked: bool,
    /// Foreign moves only: seller-side player importance captured at
    /// creation (same 0..1 scale as the domestic importance computation).
    /// `None` for domestic moves, whose importance is recomputed live in
    /// the resolver.
    pub(crate) foreign_seller_importance: Option<f32>,
}

/// A completed negotiation that needs execution at SimulatorData level.
/// Used for ALL transfers — both domestic and cross-country.
pub struct DeferredTransfer {
    pub(crate) player_id: u32,
    pub(crate) selling_country_id: u32,
    pub(crate) selling_club_id: u32,
    pub(crate) buying_country_id: u32,
    pub(crate) buying_club_id: u32,
    pub(crate) fee: f64,
    pub(crate) is_loan: bool,
    /// If true, record a contractual option-to-buy clause on the loan so the
    /// buyer can trigger a permanent purchase at window close / loan end.
    pub(crate) has_option_to_buy: bool,
    /// Annual wage agreed in PersonalTerms — drives installed salary at execution.
    pub(crate) agreed_annual_wage: Option<u32>,
    /// Buying club's league reputation, for wage fallback at execution time.
    pub(crate) buying_league_reputation: u16,
    /// If the buyer's offer carried a sell-on clause, this is the percentage
    /// owed to the *current seller* on the player's next permanent sale.
    pub(crate) sell_on_percentage: Option<f32>,
    /// Loan option/obligation fee and whether it is mandatory.
    pub(crate) loan_future_fee: Option<(u32, bool)>,
    /// Structured personal terms agreed during the negotiation —
    /// signing bonus, agent fee, release clause, squad-status promise.
    /// Execution honours these instead of inventing defaults so the
    /// player ends up with the deal the AI actually agreed to.
    pub(crate) personal_terms: Option<PersonalTermsOffer>,
    /// Snapshot of the buyer's offer clauses — drives the scheduling
    /// of installments, performance add-ons (appearances/goals), and
    /// promotion bonuses at execution time. `sell_on_percentage` and
    /// `loan_future_fee` above are already extracted; this carries the
    /// rest (installments, appearance fees, goal bonuses, promotion
    /// bonus) so the execution layer can write them into the buying
    /// market's `pending_clauses` queue without re-reading the
    /// negotiation (which is about to be dropped).
    pub(crate) offer_clauses: Vec<TransferClause>,
}

pub(crate) fn can_club_accept_player(club: &Club) -> bool {
    let max_squad = club
        .board
        .season_targets
        .as_ref()
        .map(|t| t.max_squad_size as usize)
        .unwrap_or(50);
    // Only count main team players for squad cap — youth/reserve teams are
    // separate. Resolve the Main team by type, NOT teams[0]: the main team
    // is not guaranteed to be first in the collection, and counting the
    // wrong squad would gate the cap against a reserve/B roster.
    let main_squad = club.teams.main().map(|t| t.players.len()).unwrap_or(0);
    main_squad < max_squad
}

pub(crate) fn find_player_in_country(country: &Country, player_id: u32) -> Option<&Player> {
    for club in &country.clubs {
        for team in &club.teams.teams {
            if let Some(player) = team.players.find(player_id) {
                return Some(player);
            }
        }
    }
    None
}

pub(crate) fn find_player_in_country_mut(
    country: &mut Country,
    player_id: u32,
) -> Option<&mut Player> {
    for club in &mut country.clubs {
        for team in club.teams.iter_mut() {
            if let Some(player) = team.players.find_mut(player_id) {
                return Some(player);
            }
        }
    }
    None
}

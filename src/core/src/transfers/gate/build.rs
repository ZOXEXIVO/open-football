//! The world as the plausibility model needs it.
//!
//! [`super`] is a pure model: it names no `Country`, no `Club` and no
//! `Player`, and decides everything from the numbers it is handed. This file
//! is where those numbers come from — assembling [`TransferPlausibilityInputs`]
//! out of the shapes the pipeline actually carries, whether that is a live
//! roster or a pre-built [`PlayerSummary`] from another country snapshot.
//!
//! The two used to sit in one file, separated by a banner comment and a
//! mid-file `use` block. That is the whole reason the layer kept collapsing:
//! nothing stopped a gate from reaching for a club, because the club was
//! already in scope. [`stance`] and [`fit`] are the same shape — hydration
//! beside the model it serves.
//!
//! Keeping the builder methods inside a single struct means call sites stay
//! one-liners and the wage policy / importance heuristics never silently
//! drift between callers.
//!
//! [`stance`]: crate::transfers::gate::stance
//! [`fit`]: crate::transfers::gate::fit

use crate::club::mind::organs::memory::{ActorRef, FactClaim};
use crate::club::staff::DossierTuning;
use crate::transfers::view::player::PlayerView;
use chrono::NaiveDate;

use super::{
    SquadEvidenceSource, TransferMoveAssessment, TransferMovePlausibility,
    TransferPlausibilityEvaluator, TransferPlausibilityInputs, TransferPlausibilityVerdict,
};
use crate::club::player::calculators::WageCalculator;
use crate::club::team::squad::SquadEvidenceContext;
use crate::transfers::market::route::TransferRoutePolicy;
use crate::transfers::pipeline::PlayerSummary;
use crate::transfers::{
    ClubMarketKnowledge, MarketAffinity, MarketAffinityInputs, MarketMap, MoveKind,
};
use crate::{
    Club, Country, Person, Player, PlayerFieldPositionGroup, PlayerSquadStatus, PlayerStatusType,
    TeamType,
};

/// Per-club buyer snapshot reused across plausibility lookups so the
/// builder doesn't re-walk reputation/wage data for every candidate.
#[derive(Debug, Clone)]
pub(crate) struct BuyerPlausibilityContext {
    pub buyer_rep: f32,
    pub buyer_world_rep: i16,
    pub buyer_league_rep: u16,
    pub buyer_transfer_budget: f64,
    pub buyer_wage_budget: u32,
    pub buyer_total_wages: u32,
    pub buyer_country_id: u32,
    pub buyer_country_code: String,
    pub buyer_league_id: Option<u32>,
    /// Annualised trailing income of the BUYING club, and its best-paid
    /// player. The two numbers a loan is priced against (see
    /// [`crate::transfers::loan::guard::LoanAssetGuard`]): a club whose whole year is
    /// worth less than the asset does not borrow it, and one already at
    /// its wage ceiling cannot carry the wage that comes with him.
    /// Permanent moves read neither — a purchase is priced by the fee gate.
    pub buyer_annual_income: i64,
    pub buyer_top_earner: u32,
}

impl BuyerPlausibilityContext {
    pub(crate) fn build(country: &Country, club: &Club, date: NaiveDate) -> Self {
        let main_team = club
            .teams
            .iter()
            .find(|t| matches!(t.team_type, TeamType::Main));
        let buyer_rep = main_team
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.3);
        let buyer_world_rep = main_team.map(|t| t.reputation.world as i16).unwrap_or(0);
        let buyer_league_id = main_team.and_then(|t| t.league_id);
        let buyer_league_rep = buyer_league_id
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);
        let buyer_total_wages: u32 = club.teams.iter().map(|t| t.get_annual_salary()).sum();
        let buyer_wage_budget = club
            .finance
            .wage_budget
            .as_ref()
            .map(|b| b.amount.max(0.0) as u32)
            .unwrap_or(buyer_total_wages.saturating_mul(11) / 10);
        let buyer_transfer_budget = club
            .finance
            .transfer_budget
            .as_ref()
            .map(|b| b.amount)
            .unwrap_or(club.transfer_plan.total_budget);
        BuyerPlausibilityContext {
            buyer_rep,
            buyer_world_rep,
            buyer_league_rep,
            buyer_transfer_budget,
            buyer_wage_budget,
            buyer_total_wages,
            buyer_country_id: country.id,
            buyer_country_code: country.code.clone(),
            buyer_league_id,
            buyer_annual_income: club.finance.estimated_annual_income(date),
            buyer_top_earner: club
                .teams
                .iter()
                .flat_map(|t| t.players.iter())
                .filter_map(|p| p.contract.as_ref().map(|c| c.salary))
                .max()
                .unwrap_or(0),
        }
    }
}

/// Stateless namespace for the pipeline-facing input builders. Wrapped
/// in a struct so call sites read like a discoverable API
/// (`TransferPlausibilityBuilder::from_summary(...)`) instead of a
/// loose function grab-bag.
pub(crate) struct TransferPlausibilityBuilder;

/// The selling club's standing, read off its main team: what the club is
/// worth, what its league is worth, and which league that is.
#[derive(Clone, Copy)]
struct SellerStanding {
    seller_rep: f32,
    seller_world_rep: i16,
    seller_league_id: Option<u32>,
    seller_league_rep: u16,
}

impl TransferPlausibilityBuilder {
    /// Build plausibility inputs from a `PlayerSummary` (the unit used by
    /// process_scouting / shortlists / build_shortlists). Reads the
    /// seller-side context carried on the summary
    /// ([`crate::transfers::pipeline::SellerPlausibilityContext`]) instead of
    /// re-resolving the selling club, so a **foreign** target assesses with
    /// the same rigour as a domestic one. Previously this looked the seller
    /// club up in the *buyer's* country and returned `None` for every
    /// cross-country target — which callers read as "not rejected", letting
    /// a lower-league club publicly chase a first-team player abroad.
    ///
    /// Returns `Option` for signature stability with the staged callers; the
    /// seller context is always present on a pool-built summary, so the only
    /// `None` would come from a hand-built summary with no seller data.
    pub(crate) fn from_summary(
        buyer_ctx: &BuyerPlausibilityContext,
        target: &PlayerSummary,
        is_loan: bool,
        is_unsolicited: bool,
        date: NaiveDate,
        market_reach: Option<f32>,
    ) -> Option<TransferPlausibilityInputs> {
        let seller = &target.seller_ctx;
        let player_ca = target.skill_ability;
        let position_group = target.position_group;
        let best_group_ca = target.club_best_in_group.max(player_ca);

        let same_country = target.country_id == buyer_ctx.buyer_country_id;
        let same_league_or_division = same_country
            && match (buyer_ctx.buyer_league_id, seller.league_id) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            };

        let country_pair_blocked = TransferRoutePolicy::is_blocked(
            &target.country_code,
            &buyer_ctx.buyer_country_code,
            date,
        );

        let expected_annual_wage = WageCalculator::expected_annual_wage_raw(
            player_ca,
            target.current_reputation,
            matches!(position_group, PlayerFieldPositionGroup::Forward),
            matches!(position_group, PlayerFieldPositionGroup::Goalkeeper),
            target.age,
            buyer_ctx.buyer_rep,
            buyer_ctx.buyer_league_rep,
        );

        Some(TransferPlausibilityInputs {
            // A summary carries no memory, so this path cannot know
            // whether the two of them have history. Zero is the honest
            // answer and the overwhelmingly common one.
            manager_affinity: 0.0,
            buyer_rep: buyer_ctx.buyer_rep,
            seller_rep: seller.club_reputation_score,
            buyer_league_rep: buyer_ctx.buyer_league_rep,
            seller_league_rep: seller.league_reputation,
            buyer_world_rep: buyer_ctx.buyer_world_rep,
            seller_world_rep: target.club_world_reputation,
            player_world_rep: target.world_reputation,
            player_current_rep: target.current_reputation,
            player_home_rep: target.home_reputation,
            player_age: target.age,
            position_group,
            is_listed: target.is_listed,
            is_loan_listed: target.is_loan_listed,
            is_transfer_requested: seller.is_transfer_requested,
            is_unhappy: seller.is_unhappy,
            squad_status: seller.squad_status.clone(),
            contract_months_remaining: target.contract_months_remaining,
            current_salary: target.salary,
            estimated_value: target.estimated_value,
            player_appearances: target.appearances,
            seller_club_matches: seller.club_matches_played,
            seller_position_rank: seller.position_group_rank,
            player_ca,
            best_group_ca_at_seller: best_group_ca,
            is_loan,
            is_unsolicited,
            seller_in_debt: seller.in_debt,
            release_clause_triggered: false,
            listing_resignation: seller.market_resignation,
            same_country,
            same_league_or_division,
            country_pair_blocked,
            buyer_transfer_budget: buyer_ctx.buyer_transfer_budget,
            buyer_wage_budget: buyer_ctx.buyer_wage_budget,
            buyer_total_wages: buyer_ctx.buyer_total_wages,
            expected_annual_wage,
            player_stage_inclination: seller.big_stage_inclination,
            // The pool builder cannot read the world map itself: it is
            // called from inside per-country borrows that cannot reach
            // `SimulatorData`. What it CAN do is take the number from a
            // caller that already computed it — the scouting pass memoises
            // exactly this per (passport, league) — so a club cannot show
            // PUBLIC interest in a market it does not work.
            //
            // `None` stays neutral, and neutral is right for the callers
            // that have no map in hand: a missing world fails OPEN here
            // exactly as it does everywhere else in the geography.
            //
            // Folded onto the affinity term with knowledge left at 1.0
            // because the value handed in is already the PRODUCT of the two
            // (`ScoutingProcessor::market_reach_for`); splitting it back
            // apart would invent a decomposition the caller never had.
            market_affinity: market_reach.unwrap_or(1.0).clamp(0.0, 1.0),
            buyer_market_knowledge: 1.0,
            seller_marketed: seller.is_marketed,
            buyer_annual_income: buyer_ctx.buyer_annual_income,
            buyer_top_earner: buyer_ctx.buyer_top_earner,
        })
    }

    /// Convenience wrapper for callers who already have a `PlayerSummary`
    /// and a buyer context: returns the verdict directly. Works for both
    /// domestic and foreign targets — the seller context rides on the
    /// summary, so no selling-country reference is needed.
    pub(crate) fn evaluate_summary(
        buyer_ctx: &BuyerPlausibilityContext,
        target: &PlayerSummary,
        is_loan: bool,
        is_unsolicited: bool,
        date: NaiveDate,
        market_reach: Option<f32>,
    ) -> Option<TransferPlausibilityVerdict> {
        Self::from_summary(
            buyer_ctx,
            target,
            is_loan,
            is_unsolicited,
            date,
            market_reach,
        )
        .map(|i| TransferPlausibilityEvaluator::evaluate(&i))
    }

    /// Staged-model counterpart of [`Self::evaluate_summary`] — returns the
    /// full [`TransferMoveAssessment`] (furthest reachable stage +
    /// diagnostics) so a caller can gate on a specific stage (e.g. scouting
    /// sets public interest only at `CanShowPublicInterest`).
    pub(crate) fn assess_summary(
        buyer_ctx: &BuyerPlausibilityContext,
        target: &PlayerSummary,
        is_loan: bool,
        is_unsolicited: bool,
        date: NaiveDate,
        market_reach: Option<f32>,
    ) -> Option<TransferMoveAssessment> {
        Self::from_summary(
            buyer_ctx,
            target,
            is_loan,
            is_unsolicited,
            date,
            market_reach,
        )
        .map(|i| TransferMovePlausibility::assess(&i))
    }

    /// Build plausibility inputs for a **cross-country** move with the live
    /// `Club` + `Player` references on both sides. The single source of
    /// truth for the full-reference build: [`Self::from_clubs`] (same
    /// country) delegates here with the country passed twice. Used by every
    /// foreign path — `initiate_foreign_negotiations`,
    /// `scan_foreign_loan_market`, and `clubs_interested_in_player` — so a
    /// buyer abroad is held to the same level / fee / wage / willingness
    /// realism as a domestic suitor.
    ///
    /// Seller reputation, league reputation, rank, and best-CA-in-group are
    /// read from the **selling** country/club (not the buyer's), and
    /// `same_country` / `same_league_or_division` compare the two countries
    /// directly. The country-pair route block is evaluated on the real
    /// (seller → buyer) route. When the seller context genuinely cannot be
    /// resolved the caller — not this builder — decides the fallback; this
    /// builder always returns a populated input set from the refs given.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_global(
        buying_country: &Country,
        buying_club: &Club,
        selling_country: &Country,
        selling_club: &Club,
        player: &Player,
        estimated_value: f64,
        is_loan: bool,
        is_unsolicited: bool,
        date: NaiveDate,
        market_map: &MarketMap,
    ) -> TransferPlausibilityInputs {
        let buyer_ctx = BuyerPlausibilityContext::build(buying_country, buying_club, date);

        let standing = Self::seller_standing(selling_country, selling_club);
        let seller_rep = standing.seller_rep;
        let seller_world_rep = standing.seller_world_rep;
        let seller_league_id = standing.seller_league_id;
        let seller_league_rep = standing.seller_league_rep;

        let position = player.position();
        let position_group = position.position_group();
        let player_ca = player.player_attributes.current_ability;
        let rank = PlayerView::position_group_rank(selling_club, player.id, position_group);
        let rank = if rank == u8::MAX { 1 } else { rank };
        let best_group_ca =
            PlayerView::best_ca_in_group(selling_club, position_group).max(player_ca);

        let is_listed = player.statuses.has(PlayerStatusType::Lst);
        let is_loan_listed = player.statuses.has(PlayerStatusType::Loa);
        let is_transfer_requested = player.statuses.has(PlayerStatusType::Req);
        let is_unhappy = player.statuses.has(PlayerStatusType::Unh);

        let (squad_status, contract_months_remaining, current_salary) = player
            .contract
            .as_ref()
            .map(|c| {
                let months =
                    ((c.expiration - date).num_days().max(0) / 30).min(i16::MAX as i64) as i16;
                (c.squad_status.clone(), months, c.salary)
            })
            .unwrap_or((PlayerSquadStatus::NotYetSet, 0, 0));

        let release_clause_triggered = player
            .contract
            .as_ref()
            .map(|c| c.release_clause_triggered(0.0, false).is_some())
            .unwrap_or(false);

        let same_country = buying_country.id == selling_country.id;
        let same_league_or_division = same_country
            && match (buyer_ctx.buyer_league_id, seller_league_id) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            };

        let expected_annual_wage = WageCalculator::expected_annual_wage(
            player,
            player.age(date),
            buyer_ctx.buyer_rep,
            buyer_ctx.buyer_league_rep,
        );

        // Real (seller → buyer) route friction. For a same-country move the
        // pair is (X, X), which is never on the block list, so `from_clubs`
        // still yields `false` here.
        let country_pair_blocked =
            TransferRoutePolicy::is_blocked(&selling_country.code, &buying_country.code, date);

        let (market_affinity, buyer_market_knowledge) = Self::market_geography(
            buying_country,
            buying_club,
            selling_country,
            player,
            market_map,
            date,
        );

        TransferPlausibilityInputs {
            manager_affinity: PlayerManagerAffinity::of(player, buying_club, date),
            buyer_rep: buyer_ctx.buyer_rep,
            seller_rep,
            buyer_league_rep: buyer_ctx.buyer_league_rep,
            seller_league_rep,
            buyer_world_rep: buyer_ctx.buyer_world_rep,
            seller_world_rep,
            player_world_rep: player.player_attributes.world_reputation,
            player_current_rep: player.player_attributes.current_reputation,
            player_home_rep: player.player_attributes.home_reputation,
            player_age: player.age(date),
            position_group,
            is_listed,
            is_loan_listed,
            is_transfer_requested,
            is_unhappy,
            squad_status,
            contract_months_remaining,
            current_salary,
            estimated_value,
            player_appearances: player.statistics.total_games(),
            seller_club_matches: {
                let squad = selling_club
                    .teams
                    .teams
                    .iter()
                    .find(|t| t.players.players.iter().any(|p| p.id == player.id));
                SquadEvidenceSource::club_matches(
                    squad.map(|t| t.team_type).unwrap_or(TeamType::Main),
                    squad.map(|t| t.league_id.is_some()).unwrap_or(true),
                    SquadEvidenceContext::current_season_sample(date, selling_club)
                        .club_matches_proxy(),
                )
            },
            seller_position_rank: rank,
            player_ca,
            best_group_ca_at_seller: best_group_ca,
            is_loan,
            is_unsolicited,
            seller_in_debt: selling_club.finance.balance.balance < 0,
            release_clause_triggered,
            listing_resignation: player.market_resignation(date),
            same_country,
            same_league_or_division,
            country_pair_blocked,
            buyer_transfer_budget: buyer_ctx.buyer_transfer_budget,
            buyer_wage_budget: buyer_ctx.buyer_wage_budget,
            buyer_total_wages: buyer_ctx.buyer_total_wages,
            expected_annual_wage,
            player_stage_inclination: player.big_stage_inclination,
            market_affinity,
            buyer_market_knowledge,
            seller_marketed: selling_club.transfer_plan.is_marketed(player.id),
            buyer_annual_income: buyer_ctx.buyer_annual_income,
            buyer_top_earner: buyer_ctx.buyer_top_earner,
        }
    }

    /// Build plausibility inputs at negotiation-start time when buyer and
    /// seller live in the **same** country and the buyer has the live
    /// `Club` + `Player` references. Thin wrapper over [`Self::from_global`]
    /// (same country passed twice) so the single-country and cross-country
    /// builds can never drift apart.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_clubs(
        country: &Country,
        buyer_club: &Club,
        selling_club: &Club,
        player: &Player,
        estimated_value: f64,
        is_loan: bool,
        is_unsolicited: bool,
        date: NaiveDate,
    ) -> TransferPlausibilityInputs {
        Self::from_global(
            country,
            buyer_club,
            country,
            selling_club,
            player,
            estimated_value,
            is_loan,
            is_unsolicited,
            date,
            // A domestic move has no geography to price: both the corridor
            // and the buyer's knowledge of the market are 1.0 by
            // construction, which is exactly what an empty map yields.
            &MarketMap::default(),
        )
    }

    /// League reputation comes from the SELLER's country registry — a foreign
    /// buyer must read the seller league's standing, not look it up (and miss)
    /// in its own country.
    fn seller_standing(selling_country: &Country, selling_club: &Club) -> SellerStanding {
        let main_team = selling_club
            .teams
            .iter()
            .find(|t| matches!(t.team_type, TeamType::Main));
        let seller_rep = main_team
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.3);
        let seller_world_rep = main_team.map(|t| t.reputation.world as i16).unwrap_or(0);
        let seller_league_id = main_team.and_then(|t| t.league_id);
        // League reputation comes from the SELLER's country registry — a
        // foreign buyer must read the seller league's standing, not look it
        // up (and miss) in its own country.
        let seller_league_rep = seller_league_id
            .and_then(|lid| selling_country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);

        SellerStanding {
            seller_rep,
            seller_world_rep,
            seller_league_id,
            seller_league_rep,
        }
    }

    /// Where the move sits on the map: is this a place people like him go,
    /// and does this club work that market?
    fn market_geography(
        buying_country: &Country,
        buying_club: &Club,
        selling_country: &Country,
        player: &Player,
        market_map: &MarketMap,
        date: NaiveDate,
    ) -> (f32, f32) {
        // Where the move sits on the map: is this a place people like him
        // go, and does this club work that market? A world with no geography
        // loaded (a fixture, a database predating the country cards) reads
        // both as neutral, so the gate is silent rather than closed.
        let (market_affinity, buyer_market_knowledge) = if market_map.is_silent() {
            (1.0, 1.0)
        } else {
            let affinity = MarketAffinity::affinity(
                market_map,
                MarketAffinityInputs {
                    buyer_country_id: buying_country.id,
                    nationality_country_id: player.country_id,
                    current_country_id: selling_country.id,
                    // Nothing here declares a move wage-led: the buyer's
                    // owner funding does, continuously, inside the affinity.
                    // A caller that hard-coded `Money` would make every
                    // approach by a rich club a Gulf landing.
                    kind: MoveKind::Talent,
                    benefactor: buying_club.board.ownership.benefactor,
                },
            );
            // Invert the walk. Asking every staff member "what is your level
            // on Colombia?" scans the whole department per question; a scout
            // knows a handful of countries, so one pass over the department
            // collecting the two countries we care about answers both.
            //
            // The MAX over the selling country and the nationality is the
            // right reading and stays: a club with a Brazil man can see a
            // Brazilian at Porto, and a club with a Portugal man can see the
            // same player through the league he plays in.
            let mut best_scout_level = 0u8;
            for staff in buying_club
                .teams
                .teams
                .iter()
                .flat_map(|team| team.staffs.staffs.iter())
            {
                for known in &staff.staff_attributes.knowledge.known_countries {
                    if known.country_id == selling_country.id
                        || known.country_id == player.country_id
                    {
                        best_scout_level = best_scout_level.max(known.level);
                    }
                }
            }
            let knowledge = ClubMarketKnowledge::knowledge(
                market_map,
                buying_country.id,
                &buying_club.market_ledger,
                best_scout_level,
                selling_country.id,
                date,
            )
            .max(ClubMarketKnowledge::knowledge(
                market_map,
                buying_country.id,
                &buying_club.market_ledger,
                best_scout_level,
                player.country_id,
                date,
            ));
            (affinity, knowledge)
        };

        (market_affinity, buyer_market_knowledge)
    }
}

/// How a player feels about the man who would be picking him.
///
/// Read from the *player's* side — his standing with the coach and the
/// convictions he has formed about him — rather than from the coach's
/// dossier. The two are allowed to disagree and frequently should: a
/// manager who remembers moving a squad player on remembers it as routine,
/// and the player remembers being moved on.
pub(in crate::transfers) struct PlayerManagerAffinity;

impl PlayerManagerAffinity {
    /// −1..=1, and exactly zero for the overwhelming majority of moves,
    /// where the two of them have never met.
    pub(in crate::transfers) fn of(player: &Player, buying_club: &Club, date: NaiveDate) -> f32 {
        let Some(coach) = buying_club.teams.main().map(|team| team.staffs.head_coach()) else {
            return 0.0;
        };
        if coach.id == 0 {
            return 0.0;
        }
        let manager = ActorRef::staff(coach.id);
        let ctx = player.mind_context(date, buying_club.id.into());
        let standing = player.mind.standing_with(manager, &ctx);
        if standing == 0.0
            && player.relations.get_staff(coach.id).is_none()
            && player.rapport.score(coach.id) == 0
        {
            return 0.0;
        }

        (standing
            + player.mind.believes(FactClaim::MadeMeAPlayer, manager)
                * DossierTuning::PLAYER_AFFINITY_W_MADE
            + player.mind.believes(FactClaim::HeBackedMe, manager)
                * DossierTuning::PLAYER_AFFINITY_W_BACKED
            + player.mind.believes(FactClaim::WeClashed, manager)
                * DossierTuning::PLAYER_AFFINITY_W_CLASHED
            + player.mind.believes(FactClaim::NeverTrustedMe, manager)
                * DossierTuning::PLAYER_AFFINITY_W_NEVER_TRUSTED
            + player.mind.believes(FactClaim::HisWordIsWorthless, manager)
                * DossierTuning::PLAYER_AFFINITY_W_WORD)
            .clamp(-1.0, 1.0)
    }
}

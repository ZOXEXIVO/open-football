use super::config::TransferConfig;
use super::execution::{
    ArrivalThreatProfile, DevelopmentLoanPathway, SquadReactionPass, TransferExecution,
};
use super::free_agent_depth::{
    DepthNegotiationAction, EmergencyDepthRequestIntent, EmergencyDepthRequestPlanner,
    FreeAgentNegotiationStager,
};
use super::free_agent_market_calc::{
    BuyerRoleFit, FreeAgentMarketCalculator, FreeAgentOfferPricing,
};
use super::types::{TransferActivitySummary, find_player_in_country};
use crate::club::player::contract::RENEWAL_OFFERED_LABEL;
use crate::club::player::mailbox::handlers::contract_proposal::ProcessContractHandler;
use crate::club::player::transfer::{FreeAgentBlockReason, MarketStage};
use crate::club::staff::perception::PotentialEstimator;
use crate::club::team::squad::{ContractRenewalManager, WageStructureSnapshot};
use crate::country::result::CountryResult;
use crate::shared::{Currency, CurrencyValue};
use crate::simulator::SimulatorData;
use crate::transfers::deal::negotiation::{
    NegotiationPhase, NegotiationStatus, TransferNegotiation,
};
use crate::transfers::deal::offer::{PersonalTermsOffer, PromisedSquadStatus, TransferOffer};
use crate::transfers::deal::reason::TransferReason;
use crate::transfers::gate::fit::{ForeignSlotCount, SquadRegistrationLimits};
use crate::transfers::market::region::ScoutingRegion;
use crate::transfers::pipeline::{
    PipelineProcessor, TransferNeedReason, TransferRequest, TransferRequestStatus,
};
use crate::transfers::squad::needs::{
    EmergencyBuyerContext, EmergencyCandidateView, EmergencyGroupSlot, EmergencyProjectedSquad,
    EmergencySlotStrictness, EmergencySquadFillStrategy, EmergencyStrictness, FirstTeamSquadNeeds,
};
use crate::transfers::view::club::ClubView;
use crate::transfers::{
    ClubMarketKnowledge, MarketAffinity, MarketAffinityInputs, MarketLedgerUpdate, MarketMap,
    MoveKind,
};
use crate::transfers::{CompletedTransfer, TransferType};
use crate::utils::FormattingUtils;
use crate::utils::IntegerUtils;
use crate::{
    Country, Person, PlayerContractProposal, PlayerFieldPositionGroup, PlayerResult,
    PlayerSquadStatus, PlayerStatusType, TeamInfo,
};
use chrono::NaiveDate;
use log::{debug, warn};
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

/// Country ids already reported by the unknown-nationality fallback in
/// `snapshot_global_free_agents`. The data hole is permanent for a given
/// save (the id is missing from both the world and `country_info`), so
/// warning once per id per process keeps the signal without re-emitting
/// the same line for every affected country on every daily tick.
static UNKNOWN_NATIONALITY_WARNED: LazyLock<Mutex<HashSet<u32>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Lightweight snapshot of a player in the global `sim.free_agents` pool.
/// Built before the per-country borrow so `handle_free_agents` can match
/// these players against club needs without holding a SimulatorData borrow.
///
/// Reputation and region fields mirror what `PlayerSummary` carries for
/// the regular scouting / loan pipelines. The market-state fields drive
/// the career-pressure model — without them the matcher would only see
/// nationality reputation and a Russian free agent would stay "too good
/// for Malta" forever, even after a year of unemployment.
#[derive(Clone)]
pub struct GlobalFreeAgentSummary {
    pub player_id: u32,
    pub player_name: String,
    pub ability: u8,
    pub potential: u8,
    pub age: u8,
    pub position_group: PlayerFieldPositionGroup,
    /// Reputation (0–10000) of the player's nationality country.
    pub nationality_country_reputation: u16,
    /// Continent of the player's nationality. Together with
    /// `nationality_country_code` resolves a `ScoutingRegion` for the
    /// region-prestige gate (same pattern as `scan_foreign_loan_market`).
    pub nationality_continent_id: u32,
    pub nationality_country_code: String,
    /// Nationality as an id — what the corridor cards are keyed by.
    pub nationality_country_id: u32,
    /// Career-pressure score in [0,1] computed at snapshot time. Read
    /// here rather than from the player at the call site because the
    /// matcher loop is in a per-country borrow that can't see the
    /// SimulatorData-level free-agent pool.
    pub career_pressure: f32,
    /// Days spent in the pool at snapshot time. Drives the market-
    /// clearing eligibility check without re-deriving the player's
    /// state inside the per-country borrow.
    pub days_free: i64,
    /// Player-side reference reputation used to position them on the
    /// rep-drop sliding gate. See `Player::reference_reputation`.
    pub reference_reputation: u16,
    /// Carry-overs from the player's `FreeAgentMarketState`.
    pub last_salary: u32,
    pub last_country_reputation: u16,
    pub last_league_reputation: u16,
    pub world_reputation: i16,
    pub current_reputation: i16,
    /// Professionalism normalised to [0,1] (raw attribute / 20). Feeds
    /// the soft-clearing opportunistic fit score — professional players
    /// settle for sensible squad-role deals.
    pub professionalism_norm: f32,
    /// Anti-RNG pity streak carried from the player's market state, so
    /// the matcher can lift a structurally-signable player's daily
    /// chance without re-reading the pool inside the country borrow.
    pub failed_approach_streak: u8,
    /// The country he last played in, `0` when he has never been under
    /// contract anywhere the world models.
    ///
    /// Where a man PLAYED is half his geography — a Brazilian released by
    /// Porto is a Portugal-market free agent and a Brazilian released by
    /// Flamengo is not — and the free-agent funnel read only his passport
    /// before this, which is why it could not tell the two apart.
    pub last_country_id: u32,
}

/// A free-agent signing decided by `handle_free_agents` for a player who
/// lives in the global pool (not in any country's club roster). Execution
/// is deferred to the caller because removing the player from
/// `sim.free_agents` requires `&mut SimulatorData` access, which the
/// per-country handler doesn't have.
pub struct GlobalFreeAgentSigning {
    pub player_id: u32,
    pub player_name: String,
    pub buying_country_id: u32,
    pub buying_club_id: u32,
    pub reason: TransferReason,
    /// Pre-computed annual wage + contract length + role promise. Set
    /// by the emergency pass (and any future request-driven path that
    /// stages terms upfront). `None` falls back to the calculator
    /// default at execution time, preserving the legacy "no-terms"
    /// behaviour for callers that didn't compute them.
    pub terms: Option<EmergencySignedTerms>,
}

/// Contract terms staged by the emergency pass so execution installs
/// the wage / role / contract length that was implicitly part of the
/// offer the player accepted. Without this struct the executor falls
/// back to the calculator default and the player ends up with a
/// market-rate deal even though we sold them on a short-term pitch.
///
/// Same shape for the in-country no-contract path and the global
/// pool path so both branches install the same kind of deal — keeps
/// the contract policy from drifting between the two flows.
#[derive(Debug, Clone, Copy)]
pub struct EmergencySignedTerms {
    pub annual_wage: u32,
    pub contract_years: u8,
    pub role: BuyerRoleFit,
}

impl EmergencySignedTerms {
    /// Render the staged terms into a `PersonalTermsOffer` so the
    /// in-country execution path and `complete_free_agent_signing`
    /// install the wage + length + role promise. Role maps to a
    /// promised squad status: Starter/KeyPlayer become explicit
    /// promises so the post-arrival role-fit tick can't downgrade
    /// them silently.
    pub fn to_personal_terms(self) -> PersonalTermsOffer {
        let squad_status_promise = match self.role {
            BuyerRoleFit::KeyPlayer => Some(PromisedSquadStatus::KeyPlayer),
            BuyerRoleFit::Starter => Some(PromisedSquadStatus::FirstTeamRegular),
            BuyerRoleFit::Rotation => Some(PromisedSquadStatus::FirstTeamSquadRotation),
            // Backup / Emergency are written without a role promise:
            // the player accepted the short-term offer on its merits,
            // there's no formal first-team commitment.
            BuyerRoleFit::Backup | BuyerRoleFit::Emergency => None,
        };
        PersonalTermsOffer {
            annual_wage: Some(self.annual_wage),
            signing_bonus: None,
            agent_fee: None,
            contract_years: Some(self.contract_years),
            squad_status_promise,
            release_clause_fee: None,
        }
    }
}

/// One free-agent candidate considered by the country-local matcher.
/// Hoisted to module scope so the emergency-fill pass and the legacy
/// request-driven pass share a single candidate type — pass 1 of
/// `handle_free_agents` builds the vec once, both passes consume it.
#[allow(dead_code)]
#[derive(Clone)]
pub(super) struct FreeAgentCandidate {
    pub player_id: u32,
    pub player_name: String,
    pub club_id: u32,
    pub club_name: String,
    pub ability: u8,
    pub potential: u8,
    pub age: u8,
    pub position_group: PlayerFieldPositionGroup,
    pub days_to_expiry: i64,
    /// Reputation of the country whose realism-gate the candidate
    /// is measured against. For in-country expiring contracts that's
    /// the country we're processing (passes the filter trivially).
    /// For global-pool free agents it's the player's nationality
    /// country reputation, captured in the snapshot.
    pub nationality_country_reputation: u16,
    /// Region of the player's nationality. Same gate the loan market
    /// and personal-terms negotiation use to block moves across a
    /// clear prestige drop (e.g. SouthAmerica→WestAfrica).
    pub nationality_region: ScoutingRegion,
    /// True when the candidate's nationality country code matches
    /// the buyer country's code — drives the emergency strategy's
    /// domestic-preference tiebreaker.
    pub nationality_country_code: String,
    /// Continent id of the player's nationality — emergency strategy
    /// uses this as a softer continental fallback when the player
    /// isn't strictly domestic.
    pub nationality_continent_id: u32,
    /// Career-pressure score (0..1). Drives every sliding gate
    /// in the new decay model. In-country expiring contracts
    /// have pressure = 0 — they're not on the market yet.
    pub career_pressure: f32,
    /// Days on the market. Zero for in-country expiring contracts
    /// (they aren't free yet); for global-pool players carried over
    /// from the snapshot. Market-clearing eligibility reads it.
    pub days_free: i64,
    /// Player-side reference reputation. Pegs the buyer's
    /// rep-drop tolerance against the player's last-known
    /// market and nationality.
    pub reference_reputation: u16,
    pub last_salary: u32,
    pub last_country_reputation: u16,
    pub last_league_reputation: u16,
    pub world_reputation: i16,
    pub current_reputation: i16,
    /// Professionalism normalised to [0,1] (raw attribute / 20). Feeds
    /// the soft-clearing opportunistic fit score.
    pub professionalism_norm: f32,
    /// Anti-RNG pity streak carried from the player's market state
    /// (0 for in-country expiring contracts — they aren't on the
    /// market yet).
    pub failed_approach_streak: u8,
    /// True when the candidate sits in `data.free_agents` — the
    /// global pool. The country borrow can't mutate them, so
    /// any state updates land in `global_*_ids` and are applied
    /// outside the borrow.
    pub is_global_pool: bool,
    /// Nationality as an id — what the corridor cards are keyed by.
    pub nationality_country_id: u32,
    /// The country he last played in; `0` when he never held a modelled
    /// contract. Half his geography: a Brazilian released by Porto is a
    /// Portugal-market free agent, and one released by Flamengo is not.
    pub last_country_id: u32,
}

/// How visible each free agent is to ONE buying market, computed once per
/// country per tick.
///
/// Visibility is a property of the (market, player) pair, not of the club:
/// whether Turkish football has heard of a released Russian does not depend
/// on which Turkish club is asking. So it is built once at the top of
/// [`CountryResult::handle_free_agents`] and read by every gate below —
/// which also keeps the corridor arithmetic out of a loop that runs
/// clubs × requests × candidates.
pub(super) struct FreeAgentMarketVisibility {
    by_player: HashMap<u32, MarketView>,
    import_capacity: f32,
}

/// One candidate as one market sees him.
#[derive(Debug, Clone, Copy)]
struct MarketView {
    /// Corridor plausibility of the place for this player, 0..1.
    affinity: f32,
    /// Whether anyone here is looking at him at all, 0..~2 — affinity
    /// narrowed by the market's familiarity and widened by his time on the
    /// market and by his name where names are bought.
    visibility: f32,
}

impl FreeAgentMarketVisibility {
    /// Floor under the market's familiarity with any source country. An
    /// agent can always get a CV in front of somebody; what he cannot do is
    /// make a league that has never signed anyone like his client treat him
    /// as a normal target.
    const REACH_FLOOR: f32 = 0.15;
    /// A league that buys names knows about names. Scales the destination's
    /// import capacity into the familiarity term so the Gulf and MLS are
    /// reachable without a corridor and Cameroon is not.
    const CAPACITY_REACH: f32 = 0.6;

    pub(super) fn build(
        buyer_country_id: u32,
        map: &MarketMap,
        candidates: &[FreeAgentCandidate],
    ) -> Self {
        // No map at all — a fixture, or a database built before the country
        // cards existed. A missing PAIR fails to the derived prior; a missing
        // WORLD fails open, because a market with no geography loaded must
        // behave exactly as it did before geography existed rather than
        // quietly refusing every foreign signing.
        if map.is_silent() {
            return FreeAgentMarketVisibility {
                by_player: HashMap::new(),
                import_capacity: 1.0,
            };
        }
        let import_capacity = map.import_capacity(buyer_country_id);
        let profile = map.profile(buyer_country_id);
        // The geography of a candidate depends only on WHERE he is from and
        // WHERE he last played, and the pool is thousands of players sharing
        // a few hundred such pairs. Computing it per player per country per
        // tick is the difference between a run that finishes and one that
        // does not — it is four linear scans of a forty-entry corridor list,
        // times the whole free-agent pool, times every country, every day.
        let mut geography: HashMap<(u32, u32), (f32, f32)> = HashMap::new();
        let mut by_player = HashMap::with_capacity(candidates.len());
        for candidate in candidates {
            let key = (candidate.nationality_country_id, candidate.last_country_id);
            let (affinity, reach) = *geography.entry(key).or_insert_with(|| {
                let affinity = MarketAffinity::affinity(
                    map,
                    MarketAffinityInputs {
                        buyer_country_id,
                        nationality_country_id: key.0,
                        current_country_id: key.1,
                        kind: MoveKind::Talent,
                        // The pool is priced per COUNTRY, not per club: this
                        // visibility is a market property every club in the
                        // league shares, and the country's own import
                        // capacity already carries the wage axis. The
                        // buyer-specific owner money enters on the request
                        // matcher's ranking instead.
                        benefactor: 0.0,
                    },
                );
                // A corridor only ONE card names is still a corridor here.
                // `MarketAffinity` discounts a one-sided pair, which is
                // right when the question is how plausible a MOVE is; the
                // question this layer asks is whether anyone has heard of
                // him, and a nationality whose own card names this
                // destination has been sending men there for years however
                // thin the destination's derived card happens to be.
                let export_side = 0.5
                    * map
                        .profile(key.0)
                        .export_weight(buyer_country_id)
                        .unwrap_or(0.0);
                let affinity = affinity.max(export_side);
                let is_local = key.0 == buyer_country_id || key.1 == buyer_country_id;
                let reach = if is_local {
                    1.0
                } else {
                    let from_nationality = 0.5 * profile.import_weight(key.0).unwrap_or(0.0);
                    let from_last_league = 0.5 * profile.import_weight(key.1).unwrap_or(0.0);
                    from_nationality
                        .max(from_last_league)
                        .max(Self::CAPACITY_REACH * import_capacity)
                        .max(Self::REACH_FLOOR)
                };
                (affinity, reach)
            });
            let visibility = FreeAgentMarketCalculator::visibility(
                affinity,
                reach,
                candidate.days_free,
                FreeAgentMarketCalculator::name_reach(candidate.reference_reputation),
                import_capacity,
            );
            by_player.insert(
                candidate.player_id,
                MarketView {
                    affinity,
                    visibility,
                },
            );
        }
        FreeAgentMarketVisibility {
            by_player,
            import_capacity,
        }
    }

    /// Visibility of one candidate. `1.0` for anyone this build never saw —
    /// a caller working from a candidate list the visibility was not built
    /// from must not be silently blocked by a missing entry.
    pub(super) fn of(&self, player_id: u32) -> f32 {
        self.by_player
            .get(&player_id)
            .map(|view| view.visibility)
            .unwrap_or(1.0)
    }

    /// Corridor plausibility of this market for one candidate, 0..1. Read
    /// by rankings that want the geography without the time and name terms.
    pub(super) fn affinity_of(&self, player_id: u32) -> f32 {
        self.by_player
            .get(&player_id)
            .map(|view| view.affinity)
            .unwrap_or(1.0)
    }

    /// The buying country's capacity to import names, 0..1. Conditions the
    /// cross-continent gate's standing relief.
    pub(super) fn import_capacity(&self) -> f32 {
        self.import_capacity
    }

    /// Does this candidate clear the bar for how long he has been available?
    pub(super) fn is_visible(&self, candidate: &FreeAgentCandidate) -> bool {
        self.is_visible_with_relief(candidate, 0)
    }

    /// The same read, with the bar loosened by `stage_relief` rungs of the
    /// market ladder.
    ///
    /// The emergency pass needs this: a club that cannot field a side looks
    /// harder than one filling a rotation slot, and "harder" in this model
    /// is a stage, not a multiplier. The relief NEVER removes the bar —
    /// `LastChance` is the loosest reading there is — so the two moves the
    /// gate exists for (a Russian journeyman to a Brazilian depth slot, a
    /// Russian keeper to a Cameroonian one at routine pressure) stay closed
    /// however short the buyer's squad is.
    pub(super) fn is_visible_with_relief(
        &self,
        candidate: &FreeAgentCandidate,
        stage_relief: u8,
    ) -> bool {
        // A club can always see the players in its own league. The gate is
        // about markets hearing of each other, and a domestic expiring
        // contract is not a market question.
        if !candidate.is_global_pool {
            return true;
        }
        let stage = MarketStage::from_days_free(candidate.days_free).loosened(stage_relief);
        self.of(candidate.player_id) >= FreeAgentMarketCalculator::visibility_bar(stage)
    }
}

/// One signing decided by the country-local matcher. Drained at the
/// end of `handle_free_agents` into either the in-country execution
/// path or the deferred global-signing return vector.
pub(super) struct FreeAgentSigning {
    pub player_id: u32,
    pub player_name: String,
    pub from_club_id: u32,
    pub from_club_name: String,
    pub to_club_id: u32,
    pub reason: TransferReason,
    /// Optional pre-computed contract terms. Emergency pass populates
    /// this so execution installs the agreed short-deal wage / role;
    /// the legacy request-driven pass leaves it `None` and the
    /// installer falls back to the calculator default.
    pub terms: Option<EmergencySignedTerms>,
    /// Position group the signing fills — used to mark matching
    /// transfer requests as fulfilled after the signing executes so
    /// the weekly re-evaluation doesn't re-emit them.
    pub fills_group: Option<PlayerFieldPositionGroup>,
}

impl CountryResult {
    /// Handle expiring contracts and free agent signings.
    ///
    /// Signing probability depends on player quality:
    ///   - Elite players (ability 140+): ~25% daily chance → signed within days
    ///   - Good players (100-140):       ~5-10% daily → signed within weeks
    ///   - Average players (60-100):     ~1-3% daily  → may take months
    ///   - Low quality (<60):            ~0.2-0.5%    → can sit 1-2 seasons
    ///
    /// This creates realistic free agent markets where low-quality players
    /// linger while stars get snapped up immediately.
    pub(crate) fn handle_free_agents(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
        global_pool: &[GlobalFreeAgentSummary],
        market_map: &MarketMap,
        config: &TransferConfig,
        domestic_signed_ids: &mut Vec<u32>,
        global_offered_ids: &mut Vec<u32>,
        global_rejected_ids: &mut Vec<u32>,
        global_blocked: &mut Vec<(u32, FreeAgentBlockReason)>,
    ) -> Vec<GlobalFreeAgentSigning> {
        // Pass 1: Find players with expiring contracts (< 90 days) or already expired
        let mut candidates: Vec<FreeAgentCandidate> = Vec::new();
        let mut expired_player_ids: Vec<u32> = Vec::new();

        for club in &country.clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    // Loaned-in players belong to their parent club regardless
                    // of whether the local record has a `contract` field set.
                    // Check in both branches so a stale None-contract on a loan
                    // can't accidentally mark the player as free.
                    if player.is_on_loan() {
                        continue;
                    }

                    let days_left = match &player.contract {
                        Some(c) => (c.expiration - date).num_days(),
                        None => 0, // already a free agent
                    };

                    // Contract already expired — release player
                    if days_left <= 0 && player.contract.is_some() {
                        expired_player_ids.push(player.id);
                        // Still add as candidate (will be available after release below)
                    }

                    // Available for free agent signing: contract expired or
                    // the player has no contract at all. A player with a
                    // running contract — even one expiring next week —
                    // stays at his current club until it actually ends;
                    // otherwise we fabricate "free transfers" of players
                    // who were still under contract, which is the exact
                    // move real leagues prohibit. Pre-contract agreements
                    // (signed now, effective at contract end) would need
                    // their own deferred-execution flow, not this path.
                    if days_left <= 0 {
                        // Skip if already mid-negotiation — but a staged
                        // pre-contract also wears the `Trn` badge, and
                        // for him THIS scan is the execution path (the
                        // expiry routing honours the agreed club), so he
                        // must pass through.
                        if (player.statuses.has(PlayerStatusType::Trn)
                            || player.statuses.has(PlayerStatusType::Bid))
                            && player.pending_pre_contract().is_none()
                        {
                            continue;
                        }

                        let last_salary = player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
                        candidates.push(FreeAgentCandidate {
                            player_id: player.id,
                            player_name: player.full_name.to_string(),
                            club_id: club.id,
                            club_name: club.name.clone(),
                            ability: player.player_attributes.current_ability,
                            // Signing decisions read the observable
                            // ceiling, never hidden biological PA.
                            potential: PotentialEstimator::observable_ceiling(player, date),
                            age: player.age(date),
                            position_group: player.position().position_group(),
                            days_to_expiry: days_left,
                            // In-country candidates are by definition at a
                            // club in this country, so the country-rep gate
                            // always passes — record `country.reputation`
                            // directly. Same for the region gate: the
                            // candidate sits in `country`, so the buyer's
                            // own region is its own reference point.
                            nationality_country_reputation: country.reputation,
                            nationality_region: ScoutingRegion::from_country(
                                country.continent_id,
                                &country.code,
                            ),
                            // For in-country expiring contracts we don't
                            // hydrate the player's true nationality here
                            // (would need a SimulatorData lookup we don't
                            // have). They're treated as domestic for
                            // emergency-fill purposes — which is the
                            // common case anyway and skews preference
                            // mildly toward local journeymen.
                            nationality_country_code: country.code.clone(),
                            nationality_continent_id: country.continent_id,
                            // Treated as domestic for the same reason: the
                            // true passport is not reachable from inside
                            // this borrow, and the affinity of a man
                            // already playing here is 1.0 either way.
                            nationality_country_id: country.id,
                            // Expiring contracts haven't entered the
                            // market yet — pressure is zero, the player
                            // is just transitioning. The new gates fall
                            // back to the original behaviour for them
                            // because `reference_reputation` matches
                            // the buyer's country rep exactly.
                            career_pressure: 0.0,
                            days_free: 0,
                            reference_reputation: country.reputation,
                            last_salary,
                            last_country_reputation: country.reputation,
                            last_league_reputation: country.reputation,
                            world_reputation: player.player_attributes.world_reputation,
                            current_reputation: player.player_attributes.current_reputation,
                            professionalism_norm: (player.attributes.professionalism / 20.0)
                                .clamp(0.0, 1.0),
                            // Expiring-contract candidates aren't on the
                            // open market yet — no accumulated pity.
                            failed_approach_streak: 0,
                            is_global_pool: false,
                            // An expiring domestic contract is, by
                            // construction, a man playing right here.
                            last_country_id: country.id,
                        });
                    }
                }
            }
        }

        // Final-chance renewal: before the release sweep clears expired
        // contracts, the owning club makes one synchronous renewal attempt
        // (real clubs don't watch a player they want walk out on expiry day
        // without a last offer). Accepted players carry a fresh contract and
        // leave the free-agent flow entirely; rejected ones continue into
        // the release sweep unchanged.
        let renewed_player_ids = Self::run_expiry_day_renewals(country, date, &expired_player_ids);
        candidates.retain(|c| !renewed_player_ids.contains(&c.player_id));

        // Pass 1b: Include the global "Move on Free" pool — players who live
        // outside any country's roster in `sim.free_agents`. Without this
        // step, manually-released players are invisible to club AI: only
        // contract-expiry candidates above would ever get signed. Use
        // club_id=0 / club_name="Free Agent" as the synthetic "from" so the
        // matching filter in Pass 2 (`c.club_id != club.id`) and the Pass 3
        // splitter (`from_club_id == 0` → defer to caller) both work.
        for fa in global_pool {
            candidates.push(FreeAgentCandidate {
                player_id: fa.player_id,
                player_name: fa.player_name.clone(),
                club_id: 0,
                club_name: "Free Agent".to_string(),
                ability: fa.ability,
                potential: fa.potential,
                age: fa.age,
                position_group: fa.position_group,
                days_to_expiry: 0,
                nationality_country_reputation: fa.nationality_country_reputation,
                nationality_region: ScoutingRegion::from_country(
                    fa.nationality_continent_id,
                    &fa.nationality_country_code,
                ),
                nationality_country_code: fa.nationality_country_code.clone(),
                nationality_continent_id: fa.nationality_continent_id,
                nationality_country_id: fa.nationality_country_id,
                career_pressure: fa.career_pressure,
                days_free: fa.days_free,
                reference_reputation: fa.reference_reputation,
                last_salary: fa.last_salary,
                last_country_reputation: fa.last_country_reputation,
                last_league_reputation: fa.last_league_reputation,
                world_reputation: fa.world_reputation,
                current_reputation: fa.current_reputation,
                professionalism_norm: fa.professionalism_norm,
                failed_approach_streak: fa.failed_approach_streak,
                is_global_pool: true,
                last_country_id: fa.last_country_id,
            });
        }

        // Release players with expired contracts. Players who accepted the
        // expiry-day renewal above are no longer expired — skip them, and
        // keep their shortlist/scouting interest intact (they're still
        // legitimate transfer targets under contract).
        for player_id in expired_player_ids {
            if renewed_player_ids.contains(&player_id) {
                continue;
            }
            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    if let Some(player) =
                        team.players.players.iter_mut().find(|p| p.id == player_id)
                    {
                        debug!(
                            "Contract expired: player {} ({}) released from {}",
                            player.full_name, player_id, club.name
                        );
                        player.contract = None;
                        break;
                    }
                }
            }
            // A freshly-released player is no longer a transfer target at his
            // old club, and he cannot be on any other club's loan-out list —
            // drop shortlist, scouting, and loan-out entries everywhere.
            PipelineProcessor::clear_player_interest(country, player_id);
        }

        if candidates.is_empty() {
            return Vec::new();
        }

        // How visible each candidate is to THIS market — the corridor from
        // his passport and his last league, the market's own familiarity
        // with both, how long he has been available, and whether this is a
        // league that buys names. Computed once here because the answer is a
        // property of the market, not of the club doing the asking.
        let visibility = FreeAgentMarketVisibility::build(country.id, market_map, &candidates);

        // The league's foreigner quota, resolved once for the whole pass.
        // Every free-agent door — request matcher, emergency fill, both
        // clearing tiers — asks it before it signs anybody, which is what
        // the paid paths have always done.
        let registration = SquadRegistrationLimits::new(country.id, &country.regulations);

        // Why each global-pool candidate was skipped today, highest-rank
        // reason per player. Drained into `global_blocked` at the end of
        // the tick; Phase C stamps it onto the player's market state.
        // Created BEFORE the emergency pass so that pass can explain its
        // own rejections through the same channel — it is the door the
        // geography complaint was actually about, and it was the one that
        // recorded nothing.
        let mut recorder = BlockReasonRecorder::new();

        // Pass 2: Match candidates to clubs with needs, using probability-based signing
        let mut signings: Vec<FreeAgentSigning> = Vec::new();

        // ── Pass 2-pre: honour staged pre-contracts ─────────────────
        // A player who agreed a pre-contract while running his deal down
        // now has an expired contract (cleared by the release sweep
        // above). Route the agreed free transfer to his future club
        // FIRST — pushed ahead of the emergency / request / clearing
        // passes so their `signings.iter().any(...)` dedup leaves him be.
        // Pass 3 executes it through the ordinary in-country path.
        Self::collect_pre_contract_signings(country, &mut signings);

        // ── Pass 2a (NEW): Emergency squad fill ─────────────────────
        // Runs BEFORE the request-driven matcher so clubs sitting
        // under MIN_FIRST_TEAM_SQUAD don't have to wait for the
        // scouting/shortlist pipeline. Pushes into the same
        // `signings` vec so Pass 3 executes them through the existing
        // path and the normal matcher's `signings.iter().any(...)`
        // dedup naturally skips already-claimed candidates.
        //
        // Depth shortfalls are NOT signed here — the pass returns them
        // as intents and they become DepthCover pipeline requests
        // below, serviced through the staged-negotiation flow like any
        // other recruitment need. Only the "cannot field a side /
        // group below minimum" rescue slots keep the direct path.
        let depth_intents = Self::handle_free_agents_emergency_pass(
            country,
            &candidates,
            config,
            &visibility,
            &mut signings,
            global_offered_ids,
            global_rejected_ids,
            &mut recorder,
        );
        EmergencyDepthRequestPlanner::stage_requests(country, &depth_intents);

        // Peak post-season window (Jun–Aug) lifts the request-driven cap
        // so summer free-agent business isn't throttled to the off-season
        // trickle.
        let max_signings_per_day = config.max_free_agent_signings_for(date);
        let ability_slack = config.free_agent_ability_slack;
        let buyer_country_reputation = country.reputation;
        let buyer_continent_id = country.continent_id;
        // Mirrors `scan_foreign_loan_market`: same region the country sits
        // in, used as the prestige anchor for cross-region gating.
        let buyer_region = ScoutingRegion::from_country(country.continent_id, &country.code);
        let buyer_region_prestige = buyer_region.league_prestige();
        // (club, player) pairs already approached this tick — a player
        // who turned this club down under one request must not be
        // re-asked the same day under another.
        let mut approached_today: HashSet<(u32, u32)> = HashSet::new();
        // Depth-type requests (DepthCover / SquadPadding) never sign
        // instantly — they collect staged offers here and the stager
        // below turns each one into a real Pending negotiation that
        // resolves over the following days via
        // `resolve_pending_negotiations` (personal terms → medical).
        let mut depth_offers: Vec<DepthNegotiationAction> = Vec::new();
        // Snapshot the emergency-pass headcount so the normal cap
        // measures only ITS own signings — otherwise an emergency
        // pass that already added 5 picks would starve every
        // request-driven match for the rest of the tick.
        let emergency_signing_count = signings.len();

        for club in &country.clubs {
            if signings.len() - emergency_signing_count >= max_signings_per_day {
                break;
            }

            if club.teams.teams.is_empty() {
                continue;
            }

            // Skip clubs that have reached their squad cap
            if !ClubView::can_accept_player(club) {
                continue;
            }

            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            // Check unfulfilled transfer requests
            let unfulfilled: Vec<&TransferRequest> = plan
                .transfer_requests
                .iter()
                .filter(|r| {
                    r.status != TransferRequestStatus::Fulfilled
                        && r.status != TransferRequestStatus::Abandoned
                })
                .collect();

            // Pre-compute the buyer's tier anchors. Used for role
            // inference and the quality-fit band — the same numbers
            // every rolling-CA gate in the project relies on. The tier
            // anchor curves are calibrated for `overall_score()` (home /
            // national / world blend); reading raw `world` — usually the
            // lowest of the three — understated every buyer's band and
            // biased the whole pool toward `AboveMaximumAbility`.
            let main_team = club.teams.main().or_else(|| club.teams.teams.first());
            let buyer_club_score = main_team
                .map(|t| t.reputation.overall_score().clamp(0.0, 1.0))
                .unwrap_or(0.0);
            let buyer_league_reputation = main_team
                .and_then(|t| t.league_id)
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);
            // Man-management is the closest analogue to "negotiator
            // skill" in the staff attribute schema. Staff skills run
            // 0..20, scaled to 0..100 here so it slots into the
            // calculator's percentage-style negotiation factor.
            let buyer_negotiator_skill = main_team
                .and_then(|t| t.staffs.find_negotiator())
                .map(|s| (s.staff_attributes.mental.man_management as u32 * 5).min(100) as u8)
                .unwrap_or(50);
            // The quota is a squad fact, so it is counted once per club and
            // not once per open request.
            let buyer_foreign_slots = registration.count(club);
            let buyer_benefactor = club.board.ownership.benefactor;

            for request in &unfulfilled {
                if signings.len() - emergency_signing_count >= max_signings_per_day {
                    break;
                }

                let group = request.position.position_group();

                // Emergency-planner depth requests route through the
                // staged-negotiation flow below instead of instant
                // signing. Explicitly marker-driven: a normal evaluated
                // DepthCover / SquadPadding request keeps the legacy
                // instant path.
                let is_depth_request = request.is_emergency_free_agent_depth();
                // One pursuit in flight per request, for EVERY source:
                // Negotiating means a live paid negotiation (or staged
                // FA pursuit) already owns this need. Instant-signing on
                // top of it delivered two players for one hole — the FA
                // journeyman landed, `mark_group_fulfilled` stamped the
                // request Fulfilled, and the paid negotiation still
                // completed for the same position.
                if request.status == TransferRequestStatus::Negotiating {
                    continue;
                }

                let buyer_ctx = RequestBuyerContext {
                    club_score: buyer_club_score,
                    league_reputation: buyer_league_reputation,
                    negotiator_skill: buyer_negotiator_skill,
                    country_reputation: buyer_country_reputation,
                    continent_id: buyer_continent_id,
                    region_prestige: buyer_region_prestige,
                    visibility: &visibility,
                    foreign_slots: buyer_foreign_slots,
                    benefactor: buyer_benefactor,
                };
                let nominal_floor = request.min_ability.saturating_sub(ability_slack);

                // Gate pass — the same sliding career-pressure
                // tolerances as before (quality band, country rep,
                // cross-continent, region prestige), but every
                // passing candidate is collected instead of only the
                // single best, and each gate failure is recorded so
                // the diagnosis layer can explain long sits.
                let mut ranked: Vec<(&FreeAgentCandidate, f32)> = Vec::new();
                for c in candidates.iter() {
                    if c.club_id == club.id {
                        continue;
                    }
                    if c.position_group != group {
                        continue;
                    }
                    if signings.iter().any(|s| s.player_id == c.player_id)
                        || depth_offers.iter().any(|d| d.player_id == c.player_id)
                    {
                        continue;
                    }
                    match RequestCandidateGates::evaluate(
                        c,
                        &buyer_ctx,
                        group,
                        is_depth_request,
                        nominal_floor,
                    ) {
                        Ok(()) => {
                            let priority = RequestCandidateOrdering::priority(c, &buyer_ctx, group);
                            ranked.push((c, priority));
                        }
                        Err(reason) => {
                            if c.is_global_pool {
                                recorder.record(c.player_id, reason);
                            }
                        }
                    }
                }
                if ranked.is_empty() {
                    continue;
                }
                // Combined score replaces the legacy raw-quality
                // `max_by_key`: quality fit, locality, rep closeness,
                // career pressure, and wage affordability together
                // decide the order, so a realistic willing journeyman
                // outranks a stronger player who will never accept.
                ranked.sort_by(RequestCandidateOrdering::cmp);

                // Daily probability of this club making an offer today.
                // Urgency reflects how badly the request matters; for
                // free agents the unfulfilled-request reason maps to a
                // urgency bonus.
                let urgency_bonus = match request.reason {
                    TransferNeedReason::SquadPadding => 10.0,
                    TransferNeedReason::FormationGap => 7.0,
                    TransferNeedReason::DepthCover => 5.0,
                    TransferNeedReason::CheapReinforcement => 4.0,
                    TransferNeedReason::QualityUpgrade => 3.0,
                    _ => 2.0,
                };

                // Fallback attempts: walk the ranked list until a
                // candidate signs (or stages), or the per-request
                // attempt cap runs out. The legacy single-candidate
                // behaviour skipped the whole request when the one
                // pick failed a roll, which let an unrealistic strong
                // candidate starve every signable player behind them.
                let mut attempts = 0usize;
                for (best, _priority) in ranked {
                    if attempts >= config.free_agent_attempts_per_request {
                        break;
                    }
                    if signings.len() - emergency_signing_count >= max_signings_per_day {
                        break;
                    }
                    // One pursuit in flight per (club, player) pair.
                    if is_depth_request
                        && country
                            .transfer_market
                            .has_active_negotiation_for(best.player_id, club.id)
                    {
                        continue;
                    }
                    // One approach per (club, player) per tick — a
                    // player this club already tried today under
                    // another request must not be re-asked.
                    if !approached_today.insert((club.id, best.player_id)) {
                        continue;
                    }
                    attempts += 1;

                    let daily_chance = if best.is_global_pool {
                        // Pity bonus lifts the daily chance for a
                        // structurally-signable player who keeps losing
                        // the approach roll, so a real squad need isn't
                        // left unfilled for months purely on dice. The
                        // fresh-high-ability bonus makes a good player who
                        // just came free move quickly when a club already
                        // has a matching open request for him.
                        FreeAgentMarketCalculator::daily_signing_chance(
                            best.career_pressure,
                            best.ability,
                            urgency_bonus
                                + FreeAgentMarketCalculator::pity_bonus(
                                    best.failed_approach_streak,
                                )
                                + config.fresh_high_ability_bonus(best.days_free, best.ability),
                        )
                    } else {
                        // In-country expiring-contract candidates keep the
                        // tuned tier-table behaviour — they're not on the
                        // open market yet, just transitioning. Falling back
                        // to the pressure curve here would cut elite-player
                        // signings (CA 160 + pressure 0 = ~7%, vs the 25%
                        // the existing balance assumes).
                        config.daily_signing_chance(best.ability, best.potential, best.age)
                    };

                    // Roll the dice — a miss moves on to the next-ranked
                    // candidate instead of abandoning the request.
                    let roll = IntegerUtils::random(1, 1000) as f32 / 10.0; // 0.1 to 100.0
                    if roll > daily_chance {
                        if best.is_global_pool {
                            recorder.record(
                                best.player_id,
                                FreeAgentBlockReason::DailyChanceRollFailed,
                            );
                        }
                        continue;
                    }

                    // Depth-type request: stage a real negotiation instead
                    // of an instant signing. The player's acceptance is NOT
                    // rolled here — `resolve_personal_terms` owns it when
                    // the PersonalTerms phase matures, exactly like any
                    // pipeline pursuit. Wage / role / contract length are
                    // staged now so the offer the player evaluates is the
                    // offer that gets installed on completion.
                    if is_depth_request {
                        let pricing = FreeAgentOfferPricing::compute(
                            best,
                            group,
                            buyer_club_score,
                            buyer_league_reputation,
                            buyer_negotiator_skill,
                            buyer_country_reputation,
                        );
                        let terms = pricing.signed_terms(best);
                        // Player-side anchor for the rep-diff logic in
                        // `resolve_personal_terms`: in-country candidates
                        // use their current club's standing, pool players
                        // their own reference reputation — a big name at a
                        // tiny buyer reads as a downward move and resists.
                        let selling_rep = if best.is_global_pool {
                            (best.reference_reputation as f32 / 10_000.0).clamp(0.0, 1.0)
                        } else {
                            country
                                .clubs
                                .iter()
                                .find(|c| c.id == best.club_id)
                                .and_then(|c| c.teams.teams.first())
                                .map(|t| t.reputation.overall_score().clamp(0.0, 1.0))
                                .unwrap_or(0.3)
                        };
                        let player_ambition = if best.is_global_pool {
                            0.5
                        } else {
                            find_player_in_country(country, best.player_id)
                                .map(|p| p.attributes.ambition)
                                .unwrap_or(0.5)
                        };
                        let negotiator_staff_id =
                            main_team.and_then(|t| t.staffs.find_negotiator().map(|s| s.id));

                        depth_offers.push(DepthNegotiationAction {
                            player_id: best.player_id,
                            player_name: best.player_name.clone(),
                            from_club_id: best.club_id,
                            from_club_name: best.club_name.clone(),
                            to_club_id: club.id,
                            request_id: request.id,
                            terms,
                            selling_rep,
                            buying_rep: buyer_club_score,
                            buying_league_reputation: buyer_league_reputation,
                            negotiator_staff_id,
                            player_age: best.age,
                            player_ambition,
                            is_global_pool: best.is_global_pool,
                            reason: TransferReason::key(request.reason.as_signing_reason_key()),
                        });
                        // One staged pursuit per request — the resolver
                        // owns it from here.
                        break;
                    }

                    // Acceptance: would the player actually sign this
                    // particular offer? Wage / role / prestige / quality
                    // fit weighted into a single score, sigmoid against a
                    // pressure-decayed threshold. Skipped for in-country
                    // expiring contracts (no career pressure; pre-decay
                    // behaviour keeps the existing balance).
                    if best.is_global_pool {
                        let pricing = FreeAgentOfferPricing::compute(
                            best,
                            group,
                            buyer_club_score,
                            buyer_league_reputation,
                            buyer_negotiator_skill,
                            buyer_country_reputation,
                        );
                        let rep_drop = FreeAgentMarketCalculator::rep_drop_allowed(
                            best.career_pressure,
                            best.age,
                            best.ability,
                        );
                        let min_ca = FreeAgentMarketCalculator::min_acceptable_ca(
                            buyer_club_score,
                            group,
                            best.career_pressure,
                        );
                        let max_ca = FreeAgentMarketCalculator::max_acceptable_ca(
                            buyer_club_score,
                            group,
                            best.career_pressure,
                        );
                        let wage_fit = FreeAgentMarketCalculator::wage_score(
                            pricing.offer_wage,
                            pricing.reservation_wage,
                        );
                        let score = FreeAgentMarketCalculator::acceptance_score(
                            wage_fit,
                            FreeAgentMarketCalculator::role_score(pricing.role),
                            FreeAgentMarketCalculator::prestige_score(
                                buyer_country_reputation,
                                best.reference_reputation,
                                rep_drop,
                            ),
                            FreeAgentMarketCalculator::quality_fit_score(
                                best.ability,
                                min_ca,
                                max_ca,
                            ),
                            best.career_pressure,
                        );
                        let threshold =
                            FreeAgentMarketCalculator::acceptance_threshold(best.career_pressure);
                        let prob =
                            FreeAgentMarketCalculator::acceptance_probability(score, threshold);
                        let acceptance_roll = IntegerUtils::random(1, 1000) as f32 / 1000.0;
                        // Every roll is an "offer received" — the player
                        // got a concrete approach today. Track separately
                        // whether they accepted so the pool-side state can
                        // bump `offers_rejected_total` only on declines.
                        global_offered_ids.push(best.player_id);
                        if acceptance_roll > prob {
                            global_rejected_ids.push(best.player_id);
                            // A clearly-underwater wage is the most
                            // informative cause; otherwise it was the
                            // overall composition.
                            recorder.record(
                                best.player_id,
                                if wage_fit < 0.35 {
                                    FreeAgentBlockReason::WageReservationMismatch
                                } else {
                                    FreeAgentBlockReason::AcceptanceRollFailed
                                },
                            );
                            continue;
                        }
                    }

                    let reason = TransferReason::key(request.reason.as_signing_reason_key());

                    // Stage stage-aware contract terms so the installed deal
                    // matches the free agent's market stage (a long-unemployed
                    // or older player signs a short trial, not a multi-year
                    // deal off the generic age-band default) and carries the
                    // role / pressure-decayed wage. Mirrors the depth and
                    // global-pool pricing so the contract-length policy can't
                    // drift between the free-agent entry points.
                    let terms = FreeAgentOfferPricing::compute(
                        best,
                        group,
                        buyer_club_score,
                        buyer_league_reputation,
                        buyer_negotiator_skill,
                        buyer_country_reputation,
                    )
                    .signed_terms(best);

                    signings.push(FreeAgentSigning {
                        player_id: best.player_id,
                        player_name: best.player_name.clone(),
                        from_club_id: best.club_id,
                        from_club_name: best.club_name.clone(),
                        to_club_id: club.id,
                        reason,
                        terms: Some(terms),
                        fills_group: Some(group),
                    });
                    break;
                }
            }
        }

        // Pass 2b: turn the staged depth offers into real Pending
        // negotiations (PersonalTerms phase). Runs after the matcher
        // loop because creating a negotiation needs the mutable
        // country borrow the loop's club iteration holds immutably.
        let staged_depth_ids: HashSet<u32> = depth_offers.iter().map(|d| d.player_id).collect();
        FreeAgentNegotiationStager::stage(country, depth_offers, date, global_offered_ids);

        // Pass 2c: long-term market clearing. Free agents past the
        // pressure / days-free thresholds stop waiting for an explicit
        // transfer request — they take a modest squad-role deal at a
        // lower-tier club with open roster room. Runs last so it only
        // touches the long tail the emergency and request-driven
        // passes left behind.
        Self::handle_free_agents_market_clearing_pass(
            country,
            &candidates,
            config,
            date,
            &visibility,
            &staged_depth_ids,
            &mut signings,
            global_offered_ids,
            global_rejected_ids,
            &mut recorder,
            market_map,
        );

        // Surface the tick's skip reasons; Phase C stamps them onto
        // the pool players' market state outside the country borrow.
        recorder.drain_into(global_blocked);

        // Split signings: in-country (player still has a from-club row)
        // versus global pool (player lives in `sim.free_agents`, signaled
        // by `from_club_id == 0`). The global ones can't be executed here
        // because removing the player from the global pool needs
        // `&mut SimulatorData`; collect and return them to the caller.
        let mut global_signings: Vec<GlobalFreeAgentSigning> = Vec::new();
        let country_id = country.id;

        // Pass 3: Execute signings as free transfers with negotiation records
        for signing in &signings {
            if signing.from_club_id == 0 {
                continue;
            }
            let negotiator_staff_id = country
                .clubs
                .iter()
                .find(|c| c.id == signing.to_club_id)
                .and_then(|c| c.teams.teams.first())
                .and_then(|t| t.staffs.find_negotiator().map(|s| s.id));

            let neg_id = country.transfer_market.next_negotiation_id;
            country.transfer_market.next_negotiation_id += 1;

            let offer = TransferOffer::new(
                CurrencyValue::new(0.0, Currency::Usd),
                signing.to_club_id,
                date,
            );

            let mut negotiation = TransferNegotiation::new(
                neg_id,
                signing.player_id,
                0,
                signing.from_club_id,
                signing.to_club_id,
                offer,
                date,
                0.0,
                0.0,
                0,
                0.0,
            );
            negotiation.negotiator_staff_id = negotiator_staff_id;
            negotiation.reason = signing.reason.clone();
            negotiation.status = NegotiationStatus::Accepted;
            negotiation.phase = NegotiationPhase::MedicalAndFinalization { started: date };
            country
                .transfer_market
                .negotiations
                .insert(neg_id, negotiation);
        }

        for signing in signings {
            if signing.from_club_id == 0 {
                // Global pool signing — the caller must execute against
                // `sim.free_agents`. We surface intent only; first-come-
                // first-served dedup happens at execution time when the
                // player may have already been claimed by another country.
                let to_club_id = signing.to_club_id;
                let fills_group = signing.fills_group;
                global_signings.push(GlobalFreeAgentSigning {
                    player_id: signing.player_id,
                    player_name: signing.player_name,
                    buying_country_id: country_id,
                    buying_club_id: to_club_id,
                    reason: signing.reason,
                    terms: signing.terms,
                });
                // Even though execution is deferred, the buying club's
                // open request for the same group is conceptually
                // serviced — mark fulfilled now so weekly re-evaluation
                // doesn't re-emit it. The actual roster mutation may
                // still fail at Phase C (player taken by another
                // country first); the request mark is conservative —
                // worst case a later tick re-emits it.
                if let Some(group) = fills_group {
                    TransferPlanSync::mark_group_fulfilled(country, to_club_id, group);
                }
                continue;
            }

            let to_club_name = country
                .clubs
                .iter()
                .find(|c| c.id == signing.to_club_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            // Captured before `signing.reason` is moved into the history
            // row below, so the monthly diagnostics can split pre-contract
            // moves from ordinary domestic-expiry signings.
            let is_pre_contract = signing.reason.key == "pre_contract";

            // Execute first — a failed move (squad full, player not found
            // at claimed origin) must NOT leave a phantom transfer-history
            // row. The club-transfers page reads this list directly, so
            // any entry written here is visible whether or not the player
            // actually moved.
            let buying_league_reputation = country
                .clubs
                .iter()
                .find(|c| c.id == signing.to_club_id)
                .and_then(|c| c.teams.teams.first())
                .and_then(|t| t.league_id)
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);
            // Translate staged emergency terms (if any) into the
            // executor's wage + personal-terms inputs. Without this
            // the in-country free-agent path silently falls back to
            // the calculator default and any short-deal pitch made
            // during the emergency offer evaporates.
            let agreed_annual_wage = signing.terms.map(|t| t.annual_wage);
            let personal_terms = signing.terms.map(|t| t.to_personal_terms());
            let deferred = super::types::DeferredTransfer {
                player_id: signing.player_id,
                selling_country_id: country.id,
                selling_club_id: signing.from_club_id,
                buying_country_id: country.id,
                buying_club_id: signing.to_club_id,
                fee: 0.0,
                is_loan: false,
                has_option_to_buy: false,
                agreed_annual_wage,
                buying_league_reputation,
                sell_on_percentage: None,
                loan_future_fee: None,
                personal_terms,
                // Free-agent signings carry no transfer-fee clauses
                // (no fee, no sell-on, no installments).
                offer_clauses: Vec::new(),
            };
            // Free signings carry no fee, hence no sell-on payouts — the
            // foreign-credit out-param stays empty by construction.
            let mut no_foreign_credits: Vec<(u32, f64)> = Vec::new();
            let executed = super::execution::execute_transfer_within_country(
                country,
                &deferred,
                date,
                &mut no_foreign_credits,
            );

            if !executed {
                debug!(
                    "Free agent signing rejected: player {} from club {} to club {}",
                    signing.player_id, signing.from_club_id, signing.to_club_id
                );
                continue;
            }

            // A young free signing at a big club is development material
            // too — same pathway as paid prospect purchases. Foreign
            // loanee count is unavailable from a single-country borrow;
            // the domestic count still enforces the cap.
            DevelopmentLoanPathway::stage_after_purchase(
                country,
                signing.to_club_id,
                signing.player_id,
                None,
                date,
                0,
            );

            country.transfer_market.transfer_history.push(
                CompletedTransfer::new(
                    signing.player_id,
                    signing.player_name,
                    signing.from_club_id,
                    0,
                    signing.from_club_name,
                    signing.to_club_id,
                    to_club_name,
                    date,
                    CurrencyValue::new(0.0, Currency::Usd),
                    TransferType::Free,
                )
                .with_reason(signing.reason)
                // A domestic expiry never leaves the country, and the row
                // has to say so: without an origin the corridor census
                // cannot tell it apart from a pool signing out of nowhere.
                .with_origin_country(country.id),
            );

            PipelineProcessor::clear_player_interest(country, signing.player_id);
            // Mirror the global-pool branch above: once a signing
            // actually lands, mark the matching group's open request as
            // fulfilled so the weekly re-evaluation doesn't generate a
            // duplicate. Done after execution so a failed move (squad
            // cap, lookup miss) doesn't silently fulfill a still-open
            // need.
            if let Some(group) = signing.fills_group {
                TransferPlanSync::mark_group_fulfilled(country, signing.to_club_id, group);
            }
            // Surface the signed id so the caller (which holds the full
            // simulator) can run the cross-country interest sweep once
            // the country mutable borrow ends.
            domestic_signed_ids.push(signing.player_id);
            summary.completed_transfers += 1;
            // Pre-contract moves are a subset of the in-country signings;
            // count them separately so Phase C can split the monthly
            // "pre-contract" vs "domestic-expiry" diagnostics.
            if is_pre_contract {
                summary.signed_pre_contract += 1;
            }

            debug!(
                "Free agent signing: player {} from club {} to club {}",
                signing.player_id, signing.from_club_id, signing.to_club_id
            );
        }

        global_signings
    }

    /// Turn staged pre-contracts into priority free-agent signings. A
    /// player whose contract has just lapsed (cleared by the release sweep
    /// — so `contract.is_none()` and he's still on his old roster) and who
    /// agreed a pre-contract with a domestic club moves there directly,
    /// instead of entering the open free-agent market.
    ///
    /// Read-only on `country`; pushes a [`FreeAgentSigning`] per honoured
    /// agreement so the existing Pass 3 executor performs the in-country
    /// move and `reset_on_club_change` clears the consumed agreement. A
    /// pre-contract whose buyer no longer exists / has no room is silently
    /// dropped — the player falls through to the pool and the sweep clears
    /// the stale agreement.
    fn collect_pre_contract_signings(country: &Country, signings: &mut Vec<FreeAgentSigning>) {
        for club in &country.clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    // Only just-expired players (contract cleared this
                    // tick, still on the roster) are eligible. A live
                    // contract or a loanee's parent contract is not.
                    if player.contract.is_some() || player.is_on_loan() {
                        continue;
                    }
                    let Some(agreement) = player.pending_pre_contract() else {
                        continue;
                    };
                    // Domestic only, and never a no-op self-move.
                    if agreement.to_country_id != country.id || agreement.to_club_id == club.id {
                        continue;
                    }
                    // Claimed already this tick (defensive — the pre pass
                    // runs first, so this is normally empty).
                    if signings.iter().any(|s| s.player_id == player.id) {
                        continue;
                    }
                    // Buyer must still exist with roster room.
                    let Some(buyer) = country.clubs.iter().find(|c| c.id == agreement.to_club_id)
                    else {
                        continue;
                    };
                    if buyer.teams.teams.is_empty() || !ClubView::can_accept_player(buyer) {
                        continue;
                    }

                    let role = match agreement.promised_status {
                        Some(PlayerSquadStatus::KeyPlayer) => BuyerRoleFit::KeyPlayer,
                        Some(PlayerSquadStatus::FirstTeamRegular) => BuyerRoleFit::Starter,
                        Some(PlayerSquadStatus::FirstTeamSquadRotation) => BuyerRoleFit::Rotation,
                        _ => BuyerRoleFit::Backup,
                    };
                    signings.push(FreeAgentSigning {
                        player_id: player.id,
                        player_name: player.full_name.to_string(),
                        from_club_id: club.id,
                        from_club_name: club.name.clone(),
                        to_club_id: agreement.to_club_id,
                        reason: TransferReason::key("pre_contract"),
                        terms: Some(EmergencySignedTerms {
                            annual_wage: agreement.annual_wage,
                            contract_years: agreement.contract_years,
                            role,
                        }),
                        fills_group: Some(player.position().position_group()),
                    });
                }
            }
        }
    }

    /// One synchronous last-chance renewal attempt for every player whose
    /// contract has expired today, run BEFORE the release sweep clears the
    /// contract. Returns the ids of players who accepted — the caller
    /// excludes them from both the release sweep and the free-agent
    /// candidate pool for this tick.
    ///
    /// Two-phase to satisfy the borrow checker: Phase A scans immutably
    /// and builds proposals with the owning club's wage context; Phase B
    /// applies them mutably, recording the offer in decision history and
    /// running `ProcessContractHandler::process` in place. The mailbox is
    /// deliberately bypassed — its drain runs after the release sweep,
    /// which would clear the contract before the offer is ever read.
    fn run_expiry_day_renewals(
        country: &mut Country,
        date: NaiveDate,
        expired_player_ids: &[u32],
    ) -> HashSet<u32> {
        let mut renewed: HashSet<u32> = HashSet::new();
        if expired_player_ids.is_empty() {
            return renewed;
        }
        let expired_set: HashSet<u32> = expired_player_ids.iter().copied().collect();

        struct ExpiryRenewalOffer {
            player_id: u32,
            proposal: PlayerContractProposal,
            coach_name: String,
        }
        let mut offers: Vec<ExpiryRenewalOffer> = Vec::new();

        // Phase A (immutable): build proposals. The main team anchors the
        // wage structure / staff context — same convention as the proactive
        // monthly pass and the parent-loanee pass.
        for club in &country.clubs {
            let Some(main_team) = club.teams.main().or_else(|| club.teams.teams.first()) else {
                continue;
            };
            // Cheap pre-check before snapshotting the wage structure.
            let club_has_expired = club.teams.teams.iter().any(|t| {
                t.players
                    .players
                    .iter()
                    .any(|p| expired_set.contains(&p.id))
            });
            if !club_has_expired {
                continue;
            }

            let wage_budget = club
                .finance
                .wage_budget
                .as_ref()
                .map(|b| b.amount.max(0.0) as u32);
            let league_reputation = main_team
                .league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);
            // Caps anchor on the main team's hierarchy; the budget gate
            // compares against the CLUB-wide bill (the wage budget is a
            // club-level pot, not a per-team one).
            let mut structure = WageStructureSnapshot::from_team(main_team);
            structure.current_bill = WageStructureSnapshot::club_wide_bill(&club.teams.teams);

            for team in &club.teams.teams {
                for player in &team.players.players {
                    if !expired_set.contains(&player.id) {
                        continue;
                    }
                    if let Some((proposal, coach_name)) =
                        ContractRenewalManager::try_build_expiry_day_offer(
                            main_team,
                            player,
                            date,
                            wage_budget,
                            league_reputation,
                            &structure,
                        )
                    {
                        offers.push(ExpiryRenewalOffer {
                            player_id: player.id,
                            proposal,
                            coach_name,
                        });
                    }
                }
            }
        }

        // Phase B (mutable): record the offer and run acceptance in place.
        for offer in offers {
            'apply: for club in country.clubs.iter_mut() {
                for team in club.teams.teams.iter_mut() {
                    if let Some(player) = team
                        .players
                        .players
                        .iter_mut()
                        .find(|p| p.id == offer.player_id)
                    {
                        let movement = format!(
                            "{}y · ${}/y",
                            offer.proposal.years,
                            FormattingUtils::format_money(offer.proposal.salary as f64)
                        );
                        player.decision_history.add(
                            date,
                            movement,
                            RENEWAL_OFFERED_LABEL.to_string(),
                            offer.coach_name.clone(),
                        );

                        let mut result = PlayerResult::new(player.id);
                        ProcessContractHandler::process(player, offer.proposal, date, &mut result);

                        // Accepted iff a live contract is now installed —
                        // rejection leaves the lapsed one in place.
                        let renewed_now = player
                            .contract
                            .as_ref()
                            .map(|c| c.expiration > date)
                            .unwrap_or(false);
                        if renewed_now {
                            renewed.insert(player.id);
                            // He's staying — void any pre-contract he had
                            // agreed with a rival so it can't fire later.
                            player.clear_pre_contract();
                            debug!(
                                "Expiry-day renewal accepted: player {} ({}) stays at {}",
                                player.full_name, player.id, club.name
                            );
                        }
                        break 'apply;
                    }
                }
            }
        }

        renewed
    }

    /// Emergency squad-fill pass. Walks the country's clubs, finds any
    /// whose main team is under `MIN_FIRST_TEAM_SQUAD` (or short in
    /// any specific position group), and immediately stages free-agent
    /// signings — bypassing the request-driven matcher so an
    /// underfilled side can field a team within a tick or two instead
    /// of waiting weeks for scouting / shortlists.
    ///
    /// Pushes into the shared `signings` vec so the existing Pass 3
    /// (execution) handles the actual move. Both the in-country
    /// no-contract path and the global-pool deferred-signing path are
    /// reused — no new execution code paths.
    ///
    /// Hard caps:
    ///   - per-country: `config.emergency_max_signings_per_country_per_day`
    ///   - per-club:    `config.emergency_max_signings_per_club_per_day`
    ///     (lifted to `emergency_urgent_per_club_cap_floor` when the
    ///     projected squad sits below the playable size)
    ///
    /// Above the configured squad-size threshold the pass exits early
    /// for that club so the normal scouting / shortlist pipeline gets
    /// to fill the final slots through proper recruitment.
    ///
    /// `global_offered_ids` / `global_rejected_ids` mirror the regular
    /// matcher's side-channels: every emergency offer to a global-pool
    /// candidate pushes to `offered`, and failed acceptance rolls also
    /// push to `rejected`. Phase C consumes these to bump the player's
    /// `FreeAgentMarketState` counters.
    ///
    /// Returns the depth shortfalls the pass refused to fill directly:
    /// `emergency_squad_fill_depth` slots are routine recruitment, not
    /// rescue, so they become DepthCover pipeline requests (staged by
    /// the caller via [`EmergencyDepthRequestPlanner`]) and resolve
    /// through normal negotiations instead of instant signings.
    pub(super) fn handle_free_agents_emergency_pass(
        country: &Country,
        candidates: &[FreeAgentCandidate],
        config: &TransferConfig,
        visibility: &FreeAgentMarketVisibility,
        signings: &mut Vec<FreeAgentSigning>,
        global_offered_ids: &mut Vec<u32>,
        global_rejected_ids: &mut Vec<u32>,
        recorder: &mut BlockReasonRecorder,
    ) -> Vec<EmergencyDepthRequestIntent> {
        let mut depth_intents: Vec<EmergencyDepthRequestIntent> = Vec::new();
        if candidates.is_empty() {
            return depth_intents;
        }
        let country_cap = config.emergency_max_signings_per_country_per_day;
        let base_per_club_cap = config.emergency_max_signings_per_club_per_day;
        if country_cap == 0 || base_per_club_cap == 0 {
            return depth_intents;
        }
        let mut country_signed = 0usize;
        let buyer_country_code = country.code.clone();
        let buyer_country_id = country.id;
        let buyer_continent_id = country.continent_id;
        let buyer_rep = country.reputation;
        let registration = SquadRegistrationLimits::new(country.id, &country.regulations);
        // Same anchor every realism gate in the project uses for the
        // buyer side: continent + country code → scouting region →
        // prestige score. Pre-computed once per country so the per-slot
        // buyer context build is a couple of field assignments.
        let buyer_region_prestige =
            ScoutingRegion::from_country(country.continent_id, &country.code).league_prestige();

        for club in &country.clubs {
            if country_signed >= country_cap {
                break;
            }
            if club.teams.teams.is_empty() {
                continue;
            }
            // Reuse the same squad-cap guard the normal matcher uses
            // — emergency fill cannot push past a club's max squad
            // size. `ClubView::can_accept_player` covers that.
            if !ClubView::can_accept_player(club) {
                continue;
            }

            let needs = FirstTeamSquadNeeds::for_club(club);
            if !needs.needs_emergency_fill() {
                continue;
            }
            // Once the squad is at or above the configured threshold
            // the normal scouting pipeline takes over.
            if needs.main_team_size >= config.emergency_squad_size_threshold
                && needs.group_shortfall() == 0
            {
                continue;
            }

            // Adaptive per-club cap: a club below 11 players gets a
            // higher cap so it can become playable in this tick. Country
            // cap still applies as a final ceiling so multiple unplayable
            // clubs don't all drain the market.
            let mut projected = EmergencyProjectedSquad::from_needs(&needs);
            let mut per_club_cap = base_per_club_cap;
            if projected.total < config.emergency_min_playable_size {
                let gap = config
                    .emergency_min_playable_size
                    .saturating_sub(projected.total);
                // Lift the cap up to the urgent floor (or the gap, if
                // larger). Don't compound `base + gap` because the
                // floor already encodes the playable-size target.
                per_club_cap = per_club_cap
                    .max(config.emergency_urgent_per_club_cap_floor)
                    .max(gap);
            }

            // Tier anchors for wage / role inference — match the
            // request-driven path so emergency deals fit on the same
            // market scale as the rest of the pipeline (overall_score,
            // the unit the tier anchor curves are calibrated for).
            let main_team = club.teams.main().or_else(|| club.teams.teams.first());
            let buyer_club_score = main_team
                .map(|t| t.reputation.overall_score().clamp(0.0, 1.0))
                .unwrap_or(0.0);
            let buyer_league_reputation = main_team
                .and_then(|t| t.league_id)
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);
            let buyer_negotiator_skill = main_team
                .and_then(|t| t.staffs.find_negotiator())
                .map(|s| (s.staff_attributes.mental.man_management as u32 * 5).min(100) as u8)
                .unwrap_or(50);
            let buyer_foreign_slots = registration.count(club);

            let mut club_signed = 0usize;
            // Player ids that already rejected an emergency offer
            // from this club in this tick — used to dedup retries
            // so a single rejection doesn't lock the slot for the
            // whole pass.
            let mut rejected_locally: HashSet<u32> = HashSet::new();
            // Groups the picker has reported empty for this tick —
            // the planner skips them on subsequent iterations so we
            // don't spin re-selecting the same dead slot. Without
            // this a club short of GKs with no GK candidates would
            // never sign anyone, because the planner keeps emitting
            // the GK slot and the picker keeps returning None.
            let mut empty_groups: HashSet<PlayerFieldPositionGroup> = HashSet::new();

            // Sign up to per_club_cap times; each iteration recomputes
            // the buyer urgency flag and the next-best slot from the
            // current projection. The loop bound (per_club_cap) caps
            // staged signings; the inner `pick` may return None when no
            // candidate clears the score gate, in which case we mark
            // the group as empty for this tick and try the next one.
            while club_signed < per_club_cap && country_signed < country_cap {
                // Stop emergency fill once the projected squad meets
                // the threshold AND every group minimum is satisfied.
                if !projected.needs_more_signings(config.emergency_squad_size_threshold) {
                    break;
                }

                // Pick the next slot dynamically — once the urgent
                // groups are filled, the depth tail rotates into the
                // currently thinnest group instead of always being a
                // midfielder. Groups whose pool is empty this tick
                // are excluded so the planner can move on.
                let slot = EmergencySlotPlanner::next_slot(&projected, &empty_groups);
                let Some(slot) = slot else { break };

                // Depth slots never sign directly. The shortfall turns
                // into a DepthCover pipeline request and the staged-
                // negotiation flow takes it from there. Depth is the
                // planner's terminal state for this club (group
                // minimums met or unfillable this tick), so stop here.
                if slot.reason == "emergency_squad_fill_depth" {
                    depth_intents.push(EmergencyDepthRequestIntent {
                        club_id: club.id,
                        group: slot.group,
                    });
                    break;
                }

                // Strictness is derived per-slot from the reason tag
                // so the depth slot can fire the realism gates at full
                // strength while a no-keeper GK fill stays permissive.
                let urgent = projected.is_urgent();
                let strictness = EmergencySlotStrictness::from_reason(slot.reason, urgent);
                let buyer_ctx = EmergencyBuyerContext {
                    country_reputation: buyer_rep,
                    country_code: buyer_country_code.clone(),
                    continent_id: buyer_continent_id,
                    region_prestige: buyer_region_prestige,
                    club_reputation_score: buyer_club_score,
                    league_reputation: buyer_league_reputation,
                    negotiator_skill: buyer_negotiator_skill,
                    urgent,
                    strictness,
                    import_capacity: visibility.import_capacity(),
                    country_id: buyer_country_id,
                    foreign_slots_free: buyer_foreign_slots.free(),
                };

                let pick = EmergencyCandidatePicker::pick(
                    candidates,
                    signings,
                    &rejected_locally,
                    slot,
                    &buyer_ctx,
                    club.id,
                    visibility,
                    recorder,
                );
                let Some(best) = pick else {
                    // No viable candidate for this slot this tick —
                    // mark the group as empty so the planner moves to
                    // the next-most-needed group on the next iteration.
                    empty_groups.insert(slot.group);
                    continue;
                };

                // Stage wage / role / terms — emergency offers are
                // realistic short deals, priced through the same
                // shared wage chain as the regular matcher and the
                // staged depth flow, then run through the acceptance
                // roll lifted by the emergency multiplier.
                let pricing = FreeAgentOfferPricing::compute(
                    best,
                    slot.group,
                    buyer_club_score,
                    buyer_league_reputation,
                    buyer_negotiator_skill,
                    buyer_rep,
                );

                // Acceptance: same composition as the regular matcher
                // (wage / role / prestige / quality_fit / pressure),
                // multiplied by the emergency uplift so the short-deal
                // pitch translates into a higher acceptance chance.
                // Crucially the multiplier is applied on the probability
                // — not by lowering the threshold — so an implausible
                // offer still gets a low probability, just slightly less
                // low.
                let rep_drop = FreeAgentMarketCalculator::rep_drop_allowed(
                    best.career_pressure,
                    best.age,
                    best.ability,
                );
                let min_ca = FreeAgentMarketCalculator::min_acceptable_ca(
                    buyer_club_score,
                    slot.group,
                    best.career_pressure,
                );
                let max_ca = FreeAgentMarketCalculator::max_acceptable_ca(
                    buyer_club_score,
                    slot.group,
                    best.career_pressure,
                );
                let score = FreeAgentMarketCalculator::acceptance_score(
                    FreeAgentMarketCalculator::wage_score(
                        pricing.offer_wage,
                        pricing.reservation_wage,
                    ),
                    FreeAgentMarketCalculator::role_score(pricing.role),
                    FreeAgentMarketCalculator::prestige_score(
                        buyer_rep,
                        best.reference_reputation,
                        rep_drop,
                    ),
                    FreeAgentMarketCalculator::quality_fit_score(best.ability, min_ca, max_ca),
                    best.career_pressure,
                );
                let threshold =
                    FreeAgentMarketCalculator::acceptance_threshold(best.career_pressure);
                let base_prob = FreeAgentMarketCalculator::acceptance_probability(score, threshold);
                let prob = (base_prob
                    * EmergencySquadFillStrategy::EMERGENCY_ACCEPTANCE_MULTIPLIER)
                    .clamp(0.0, 1.0);

                if best.is_global_pool {
                    // Global-pool offer: bump the player's `offered`
                    // counter regardless of acceptance, so the 30-day
                    // window stays consistent with normal matching.
                    global_offered_ids.push(best.player_id);
                }

                let acceptance_roll = IntegerUtils::random(1, 1000) as f32 / 1000.0;
                if acceptance_roll > prob {
                    if best.is_global_pool {
                        global_rejected_ids.push(best.player_id);
                    }
                    debug!(
                        "Emergency offer rejected: club {} → player {} ({:?}, prob={:.2})",
                        club.id, best.player_id, slot.group, prob
                    );
                    // Skip this candidate for the rest of the pass at
                    // this club — they declined once and shouldn't be
                    // re-asked this tick — and try another candidate
                    // for the same slot. The country / per-club caps
                    // still bound the loop so a stream of rejections
                    // can't run forever; once the picker exhausts the
                    // pool it returns None and the outer break fires.
                    rejected_locally.insert(best.player_id);
                    continue;
                }

                signings.push(FreeAgentSigning {
                    player_id: best.player_id,
                    player_name: best.player_name.clone(),
                    from_club_id: best.club_id,
                    from_club_name: best.club_name.clone(),
                    to_club_id: club.id,
                    reason: TransferReason::key(slot.reason),
                    terms: Some(pricing.signed_terms(best)),
                    fills_group: Some(slot.group),
                });
                projected.apply_signing(slot.group);
                club_signed += 1;
                country_signed += 1;
                debug!(
                    "Emergency squad fill: club {} → player {} ({:?}, {}, wage={})",
                    club.id, best.player_id, slot.group, slot.reason, pricing.offer_wage
                );
            }
        }

        depth_intents
    }

    /// Market-clearing pass. Free agents past the pressure / days-free
    /// thresholds stop waiting for an explicit transfer request: each one
    /// (most desperate first) is matched against the lowest-tier club
    /// with open roster room whose quality band fits, and offered a short
    /// Backup/Emergency squad-role deal through the same wage chain and
    /// acceptance model as every other entry point.
    ///
    /// Two tiers run back to back over a single shared buyer set:
    ///   - **Soft** (≈3 months / 0.45 pressure): restricted to DOMESTIC /
    ///     same-continent candidates and gated by the opportunistic
    ///     squad-fit score, so most normal free agents resolve early
    ///     through a realistic local fit rather than waiting a full year.
    ///   - **Hard** (≈1 year / 0.75 pressure): the broad long-tail
    ///     backstop with the wider region / reputation tolerance.
    ///
    /// Hard realism gates stay on in both tiers; `MarketStage::LastChance`
    /// players (a year or more on the market) get a wider allowance: rep
    /// drop +600, region drop +0.10, cross-continent pressure floor 0.75
    /// instead of 0.85. Per-day caps (soft 1, hard 2) keep clearing
    /// gradual and the daily approach roll keeps it organic.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn handle_free_agents_market_clearing_pass(
        country: &Country,
        candidates: &[FreeAgentCandidate],
        config: &TransferConfig,
        date: NaiveDate,
        visibility: &FreeAgentMarketVisibility,
        staged_ids: &HashSet<u32>,
        signings: &mut Vec<FreeAgentSigning>,
        global_offered_ids: &mut Vec<u32>,
        global_rejected_ids: &mut Vec<u32>,
        recorder: &mut BlockReasonRecorder,
        market_map: &MarketMap,
    ) {
        // Build the open-capacity buyer set once (lowest tier first) and
        // share it across both clearing tiers.
        let buyers = MarketClearingBuyer::rows_for_country(country);
        if buyers.is_empty() {
            return;
        }

        // Per-day caps scale with the country's club count so a large
        // league clears proportionally more of its pool, and lift by one
        // in the peak window. The daily-chance bonus accelerates clearing
        // during the summer too. Small countries keep the base caps.
        let club_count = country.clubs.len();
        let peak_chance_bonus = config.peak_clearing_chance_bonus(date);

        // Soft tier first: early, domestic / same-continent only, gated
        // by the opportunistic fit score — the "a local club takes a punt
        // on a useful free body" outcome that resolves most normal free
        // agents well before the hard backstop.
        Self::run_market_clearing_tier(
            country,
            candidates,
            &buyers,
            MarketClearingTier {
                min_pressure: config.soft_market_clearing_min_pressure,
                min_days_free: config.soft_market_clearing_min_days_free,
                cap: config.soft_clearing_cap(club_count, date),
                locality_restricted: true,
                opportunistic_gate: true,
                peak_chance_bonus,
            },
            visibility,
            staged_ids,
            signings,
            global_offered_ids,
            global_rejected_ids,
            recorder,
            market_map,
            date,
        );

        // Hard tier: the long-tail backstop with broad tolerance, for
        // players the soft tier's locality restriction never reached.
        Self::run_market_clearing_tier(
            country,
            candidates,
            &buyers,
            MarketClearingTier {
                min_pressure: config.hard_market_clearing_min_pressure,
                min_days_free: config.hard_market_clearing_min_days_free,
                cap: config.hard_clearing_cap(club_count, date),
                locality_restricted: false,
                opportunistic_gate: false,
                peak_chance_bonus,
            },
            visibility,
            staged_ids,
            signings,
            global_offered_ids,
            global_rejected_ids,
            recorder,
            market_map,
            date,
        );
    }

    /// Pick which of the clubs whose quality band fits actually signs him.
    ///
    /// The soft tier hands in a one-element list (it stops at the first fit
    /// by design — it is the local outlet, and a local club taking a punt is
    /// the first club with room). The hard tier hands in every fitting club,
    /// and the choice is weighted by how badly each one needs the position
    /// AND by whether it works the market he comes out of. That is what
    /// replaces "the first lowest-tier club on the list", which is the line
    /// that produced the random team that urgently needs a player: it always
    /// picked the smallest club in the country, whatever its actual shape.
    ///
    /// The knowledge term is a MULTIPLIER on a floor, never a gate: the
    /// visibility layer above has already decided the market can see him at
    /// all, and this only orders the clubs inside it. A club that has never
    /// touched his market still weighs 0.3 of one that has, so an
    /// unfashionable corridor is unlikely rather than impossible — which is
    /// how a save opens a corridor the shipped data never named.
    fn sample_clearing_buyer<'a>(
        fitting: &[(&'a MarketClearingBuyer, u8, u8)],
        group: PlayerFieldPositionGroup,
        knowledge: &ClearingMarketKnowledge<'_>,
    ) -> Option<(&'a MarketClearingBuyer, u8, u8)> {
        if fitting.len() <= 1 {
            return fitting.first().copied();
        }
        /// Weight a club with no knowledge of the market keeps.
        const UNFAMILIAR_SHARE: f32 = 0.3;
        let weights: Vec<f32> = fitting
            .iter()
            .map(|(buyer, _, _)| {
                let familiarity = UNFAMILIAR_SHARE
                    + (1.0 - UNFAMILIAR_SHARE) * knowledge.of(buyer.club_id).clamp(0.0, 1.0);
                buyer.position_depth_need(group).max(0.05) * familiarity
            })
            .collect();
        let total: f32 = weights.iter().sum();
        if total <= 0.0 {
            return fitting.first().copied();
        }
        let roll = IntegerUtils::random(0, 10_000) as f32 / 10_000.0 * total;
        let mut acc = 0.0;
        for (entry, weight) in fitting.iter().zip(weights.iter()) {
            acc += *weight;
            if roll < acc {
                return Some(*entry);
            }
        }
        fitting.last().copied()
    }

    /// Run one market-clearing tier over the long-tail pool. Soft and
    /// hard tiers share this body; [`MarketClearingTier`] sets the
    /// eligibility thresholds, the per-day cap, whether candidates are
    /// restricted to the buyer country's domestic / continental market,
    /// and whether the opportunistic squad-fit gate fires before an
    /// offer is made.
    #[allow(clippy::too_many_arguments)]
    fn run_market_clearing_tier(
        country: &Country,
        candidates: &[FreeAgentCandidate],
        buyers: &[MarketClearingBuyer],
        tier: MarketClearingTier,
        visibility: &FreeAgentMarketVisibility,
        staged_ids: &HashSet<u32>,
        signings: &mut Vec<FreeAgentSigning>,
        global_offered_ids: &mut Vec<u32>,
        global_rejected_ids: &mut Vec<u32>,
        recorder: &mut BlockReasonRecorder,
        market_map: &MarketMap,
        date: NaiveDate,
    ) {
        if tier.cap == 0 {
            return;
        }
        let buyer_country_reputation = country.reputation;
        let buyer_continent_id = country.continent_id;
        let buyer_country_code = country.code.as_str();
        let buyer_region_prestige =
            ScoutingRegion::from_country(country.continent_id, &country.code).league_prestige();

        // Long-tail candidates eligible for this tier, most desperate
        // first. The soft tier additionally restricts to domestic /
        // same-continent nationalities — its whole purpose is the local
        // market outlet; a cross-continent punt is the hard tier's job.
        let mut eligible: Vec<&FreeAgentCandidate> = candidates
            .iter()
            .filter(|c| c.is_global_pool)
            .filter(|c| {
                // The days-free floor shrinks with quality: a strong free
                // body reaches the opportunistic outlet in weeks, not
                // months — clubs don't wait a quarter to notice a CA-140
                // player sitting on the market.
                c.career_pressure >= tier.min_pressure
                    || c.days_free
                        >= FreeAgentMarketCalculator::quality_scaled_min_days(
                            tier.min_days_free,
                            c.ability,
                        )
            })
            .filter(|c| {
                !tier.locality_restricted
                    || c.nationality_country_code
                        .eq_ignore_ascii_case(buyer_country_code)
                    || c.nationality_continent_id == buyer_continent_id
            })
            .filter(|c| !signings.iter().any(|s| s.player_id == c.player_id))
            .filter(|c| !staged_ids.contains(&c.player_id))
            .collect();
        // Pressure-led, quality-spotlit ordering — see
        // `clearing_queue_score` for why raw pressure alone starves
        // good players behind the per-day cap.
        eligible.sort_by(|a, b| {
            let score_a = FreeAgentMarketCalculator::clearing_queue_score(
                a.career_pressure,
                a.ability,
                a.days_free,
            );
            let score_b = FreeAgentMarketCalculator::clearing_queue_score(
                b.career_pressure,
                b.ability,
                b.days_free,
            );
            score_b
                .partial_cmp(&score_a)
                .unwrap_or(Ordering::Equal)
                .then_with(|| b.days_free.cmp(&a.days_free))
        });

        let mut cleared = 0usize;
        for candidate in eligible {
            if cleared >= tier.cap {
                break;
            }

            // Organic pacing, lifted by the anti-RNG pity bonus so a
            // structurally-signable long-tail player isn't left waiting
            // on dice alone, and by the peak-window bonus so summer
            // clearing runs faster.
            let daily = FreeAgentMarketCalculator::daily_signing_chance(
                candidate.career_pressure,
                candidate.ability,
                4.0 + FreeAgentMarketCalculator::pity_bonus(candidate.failed_approach_streak)
                    + tier.peak_chance_bonus,
            );
            let roll = IntegerUtils::random(1, 1000) as f32 / 10.0;
            if roll > daily {
                continue;
            }

            // A year-plus on the market unlocks the widest allowance,
            // independent of which tier is running.
            let last_chance = candidate.days_free >= 365;

            // Country-level realism gates, widened for LastChance.
            let rep_drop = FreeAgentMarketCalculator::rep_drop_allowed(
                candidate.career_pressure,
                candidate.age,
                candidate.ability,
            ) + if last_chance { 600 } else { 0 };
            if (buyer_country_reputation as i32 + rep_drop) < candidate.reference_reputation as i32
            {
                recorder.record(
                    candidate.player_id,
                    FreeAgentBlockReason::CountryReputationGap,
                );
                continue;
            }
            let cross_floor = if last_chance { 0.75 } else { 0.85 };
            if FreeAgentMarketCalculator::cross_continent_blocked(
                candidate.nationality_continent_id == buyer_continent_id,
                candidate.nationality_region.league_prestige(),
                buyer_region_prestige,
                candidate.career_pressure,
                cross_floor,
                candidate.reference_reputation,
                visibility.import_capacity(),
            ) {
                recorder.record(
                    candidate.player_id,
                    FreeAgentBlockReason::CrossContinentPressureTooLow,
                );
                continue;
            }
            let region_drop = FreeAgentMarketCalculator::region_drop_allowed(
                candidate.career_pressure,
                candidate.reference_reputation,
            ) + if last_chance { 0.10 } else { 0.0 };
            if candidate.nationality_region.league_prestige() > buyer_region_prestige + region_drop
            {
                recorder.record(candidate.player_id, FreeAgentBlockReason::RegionPrestigeGap);
                continue;
            }
            // The clearing tiers are the backstop, not a bypass: a market
            // that has never seen anyone like him does not sign him just
            // because his contract ran out somewhere else.
            if !visibility.is_visible(candidate) {
                recorder.record(candidate.player_id, FreeAgentBlockReason::MarketUnfamiliar);
                continue;
            }

            // How well each fitting club knows where he comes from —
            // resolved per candidate, read per club below.
            let candidate_market = ClearingMarketKnowledge {
                country,
                map: market_map,
                date,
                nationality_country_id: candidate.nationality_country_id,
                last_country_id: candidate.last_country_id,
            };

            // Which buyer takes him. The soft tier keeps the lowest-tier
            // first-fit — it IS the local outlet, and a local club taking a
            // punt is exactly the first club with room. The hard tier picks
            // among the clubs whose band fits by visibility-weighted
            // sampling, so "the random team that urgently needs a player"
            // stops being the mechanism: a fitting club that knows his
            // market gets him ahead of one that does not.
            let mut too_good_everywhere = true;
            let mut fitting: Vec<(&MarketClearingBuyer, u8, u8)> = Vec::new();
            // Clubs whose band fits but whose foreigner quota is full — the
            // difference between "nobody wants him" and "nobody can
            // register him", which is a different answer for the player.
            let mut unregistrable_fits = 0usize;
            for buyer in buyers {
                let min_ca = FreeAgentMarketCalculator::min_acceptable_ca(
                    buyer.club_score,
                    candidate.position_group,
                    candidate.career_pressure,
                );
                let max_ca = FreeAgentMarketCalculator::max_acceptable_ca(
                    buyer.club_score,
                    candidate.position_group,
                    candidate.career_pressure,
                );
                if candidate.ability <= max_ca {
                    too_good_everywhere = false;
                }
                if candidate.ability >= min_ca && candidate.ability <= max_ca {
                    // A club with no registration slot left for a foreigner
                    // is not a landing spot, however well he fits: signing
                    // him produces an omitted registration, not a squad
                    // member, and the surplus machinery lists him for it.
                    if buyer
                        .foreign_slots
                        .would_block(candidate.nationality_country_id)
                    {
                        unregistrable_fits += 1;
                        continue;
                    }
                    fitting.push((buyer, min_ca, max_ca));
                    if tier.locality_restricted {
                        break;
                    }
                }
            }
            if fitting.is_empty() && unregistrable_fits > 0 {
                recorder.record(
                    candidate.player_id,
                    FreeAgentBlockReason::NoRegistrationSlot,
                );
            }
            let chosen =
                Self::sample_clearing_buyer(&fitting, candidate.position_group, &candidate_market);
            let Some((buyer, min_ca, max_ca)) = chosen else {
                recorder.record(
                    candidate.player_id,
                    if too_good_everywhere {
                        FreeAgentBlockReason::AboveMaximumAbility
                    } else {
                        FreeAgentBlockReason::BelowMinimumAbility
                    },
                );
                continue;
            };

            // Market clearing never pitches a starter's role — the
            // deal is "join the squad on a short, modest contract",
            // priced as Backup (or Emergency when even that overstates
            // the fit).
            let inferred = FreeAgentMarketCalculator::infer_buyer_role(
                candidate.ability,
                buyer.club_score,
                candidate.position_group,
            );
            let role = match inferred {
                BuyerRoleFit::Emergency => BuyerRoleFit::Emergency,
                _ => BuyerRoleFit::Backup,
            };
            let pricing = FreeAgentOfferPricing::compute_with_role(
                candidate,
                candidate.position_group,
                role,
                buyer.club_score,
                buyer.league_reputation,
                buyer.negotiator_skill,
                buyer_country_reputation,
            );

            let wage_fit =
                FreeAgentMarketCalculator::wage_score(pricing.offer_wage, pricing.reservation_wage);
            let quality_fit =
                FreeAgentMarketCalculator::quality_fit_score(candidate.ability, min_ca, max_ca);

            // Soft tier only: opportunistic squad-fit gate. A club takes
            // a punt on a free body only when the overall fit — depth
            // need, affordability, locality, quality, pressure,
            // professionalism — clears the stage-scaled threshold. This
            // is what makes the early domestic layer a *selective*
            // outlet rather than an indiscriminate sweep.
            if tier.opportunistic_gate {
                let stage = MarketStage::from_days_free(candidate.days_free);
                let locality = if candidate
                    .nationality_country_code
                    .eq_ignore_ascii_case(buyer_country_code)
                {
                    1.0
                } else if candidate.nationality_continent_id == buyer_continent_id {
                    0.6
                } else {
                    0.25
                };
                let fit = FreeAgentMarketCalculator::opportunistic_fit_score(
                    buyer.position_depth_need(candidate.position_group),
                    wage_fit,
                    locality,
                    quality_fit,
                    candidate.career_pressure,
                    candidate.professionalism_norm,
                );
                if fit < FreeAgentMarketCalculator::opportunistic_fit_threshold(stage) {
                    recorder.record(candidate.player_id, FreeAgentBlockReason::NoMatchingRequest);
                    continue;
                }
            }

            let score = FreeAgentMarketCalculator::acceptance_score(
                wage_fit,
                FreeAgentMarketCalculator::role_score(pricing.role),
                FreeAgentMarketCalculator::prestige_score(
                    buyer_country_reputation,
                    candidate.reference_reputation,
                    rep_drop,
                ),
                quality_fit,
                candidate.career_pressure,
            );
            let threshold =
                FreeAgentMarketCalculator::acceptance_threshold(candidate.career_pressure);
            let prob = FreeAgentMarketCalculator::acceptance_probability(score, threshold);
            global_offered_ids.push(candidate.player_id);
            let acceptance_roll = IntegerUtils::random(1, 1000) as f32 / 1000.0;
            if acceptance_roll > prob {
                global_rejected_ids.push(candidate.player_id);
                recorder.record(
                    candidate.player_id,
                    if wage_fit < 0.35 {
                        FreeAgentBlockReason::WageReservationMismatch
                    } else {
                        FreeAgentBlockReason::AcceptanceRollFailed
                    },
                );
                continue;
            }

            signings.push(FreeAgentSigning {
                player_id: candidate.player_id,
                player_name: candidate.player_name.clone(),
                from_club_id: candidate.club_id,
                from_club_name: candidate.club_name.clone(),
                to_club_id: buyer.club_id,
                reason: TransferReason::key("free_agent_market_clearing"),
                terms: Some(pricing.signed_terms(candidate)),
                // No transfer request is being serviced — leave the
                // request bookkeeping untouched.
                fills_group: None,
            });
            cleared += 1;
            debug!(
                "Market clearing ({} tier): club {} signs long-term free agent {} (cp={:.2}, days_free={})",
                if tier.opportunistic_gate {
                    "soft"
                } else {
                    "hard"
                },
                buyer.club_id,
                candidate.player_id,
                candidate.career_pressure,
                candidate.days_free
            );
        }
    }
}

/// Parameters for one market-clearing tier ([`CountryResult::run_market_clearing_tier`]).
/// The soft tier (early, local, opportunistic) and the hard tier
/// (long-tail, broad) differ only in these knobs.
struct MarketClearingTier {
    /// Career-pressure floor for tier eligibility.
    min_pressure: f32,
    /// Days-free floor for tier eligibility (either criterion qualifies).
    min_days_free: i64,
    /// Per-country per-day signing cap for this tier.
    cap: usize,
    /// Soft tier: restrict candidates to the buyer country's domestic /
    /// same-continent market.
    locality_restricted: bool,
    /// Soft tier: require the opportunistic squad-fit score to clear the
    /// stage threshold before an offer is made.
    opportunistic_gate: bool,
    /// Extra percentage points on the daily signing chance during the
    /// peak post-season window; zero off-season.
    peak_chance_bonus: f32,
}

/// Picks the next emergency slot from the running projected squad.
/// The plan always starts at GK > DEF > FWD > MID (the legacy order)
/// while any per-group minimum is unmet, then rotates depth into the
/// currently thinnest group. Returns `None` when the projection is
/// fully satisfied so the caller can break out of the loop.
struct EmergencySlotPlanner;

impl EmergencySlotPlanner {
    fn next_slot(
        projected: &EmergencyProjectedSquad,
        empty_groups: &HashSet<PlayerFieldPositionGroup>,
    ) -> Option<EmergencyGroupSlot> {
        use crate::transfers::squad::needs::{
            MIN_GROUP_DEFENDER, MIN_GROUP_FORWARD, MIN_GROUP_GOALKEEPER, MIN_GROUP_MIDFIELDER,
        };
        if MIN_GROUP_GOALKEEPER > projected.gk
            && !empty_groups.contains(&PlayerFieldPositionGroup::Goalkeeper)
        {
            return Some(EmergencyGroupSlot {
                group: PlayerFieldPositionGroup::Goalkeeper,
                missing: MIN_GROUP_GOALKEEPER - projected.gk,
                reason: "emergency_squad_fill_gk",
            });
        }
        if MIN_GROUP_DEFENDER > projected.def
            && !empty_groups.contains(&PlayerFieldPositionGroup::Defender)
        {
            return Some(EmergencyGroupSlot {
                group: PlayerFieldPositionGroup::Defender,
                missing: MIN_GROUP_DEFENDER - projected.def,
                reason: "emergency_squad_fill_def",
            });
        }
        if MIN_GROUP_FORWARD > projected.fwd
            && !empty_groups.contains(&PlayerFieldPositionGroup::Forward)
        {
            return Some(EmergencyGroupSlot {
                group: PlayerFieldPositionGroup::Forward,
                missing: MIN_GROUP_FORWARD - projected.fwd,
                reason: "emergency_squad_fill_fwd",
            });
        }
        if MIN_GROUP_MIDFIELDER > projected.mid
            && !empty_groups.contains(&PlayerFieldPositionGroup::Midfielder)
        {
            return Some(EmergencyGroupSlot {
                group: PlayerFieldPositionGroup::Midfielder,
                missing: MIN_GROUP_MIDFIELDER - projected.mid,
                reason: "emergency_squad_fill_mid",
            });
        }
        // Group minimums all met (or exhausted) — pick the thinnest
        // group for depth, skipping any that have no candidates left
        // this tick. The caller's outer check on `needs_more_signings`
        // already gates whether we get here at all.
        let depth_group = Self::depth_group(projected, empty_groups)?;
        Some(EmergencyGroupSlot {
            group: depth_group,
            missing: 1,
            reason: "emergency_squad_fill_depth",
        })
    }

    /// Same logic as `EmergencyProjectedSquad::thinnest_group` but
    /// honours the empty-groups set so a dead pool doesn't deadlock
    /// the depth tail.
    fn depth_group(
        projected: &EmergencyProjectedSquad,
        empty_groups: &HashSet<PlayerFieldPositionGroup>,
    ) -> Option<PlayerFieldPositionGroup> {
        let fallback = projected.thinnest_group();
        if !empty_groups.contains(&fallback) {
            return Some(fallback);
        }
        // The thinnest group is dead — try the others in order of
        // shortfall, then by tie-breaker.
        let mut candidates = [
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
            PlayerFieldPositionGroup::Goalkeeper,
        ];
        candidates.sort_by_key(|g| -Self::gap_for(projected, *g));
        candidates.into_iter().find(|g| !empty_groups.contains(g))
    }

    fn gap_for(projected: &EmergencyProjectedSquad, group: PlayerFieldPositionGroup) -> i32 {
        use crate::transfers::squad::needs::{
            MIN_GROUP_DEFENDER, MIN_GROUP_FORWARD, MIN_GROUP_GOALKEEPER, MIN_GROUP_MIDFIELDER,
        };
        match group {
            PlayerFieldPositionGroup::Goalkeeper => {
                (MIN_GROUP_GOALKEEPER as i32) - (projected.gk as i32)
            }
            PlayerFieldPositionGroup::Defender => {
                (MIN_GROUP_DEFENDER as i32) - (projected.def as i32)
            }
            PlayerFieldPositionGroup::Midfielder => {
                (MIN_GROUP_MIDFIELDER as i32) - (projected.mid as i32)
            }
            PlayerFieldPositionGroup::Forward => {
                (MIN_GROUP_FORWARD as i32) - (projected.fwd as i32)
            }
        }
    }
}

/// Hard realism filters shared with the normal free-agent matcher.
/// Wraps the gate family (quality / reputation / region) on a unit
/// struct so the picker can call `EmergencyRealismGates::passes(...)`
/// once and every check stays in lockstep with the rest of the
/// transfer pipeline. Strictness from
/// [`EmergencyBuyerContext::strictness`] decides how much slack each
/// gate gets — depth slots run at full strength, urgent group fills
/// widen the band slightly, a no-keeper GK fill widens it the most.
struct EmergencyRealismGates;

impl EmergencyRealismGates {
    /// Every gate must pass for the candidate to enter scoring. Returns the
    /// reason the candidate failed so the emergency pass can explain a long
    /// sit the same way the request matcher does.
    fn evaluate(
        candidate: &FreeAgentCandidate,
        buyer: &EmergencyBuyerContext,
        group: PlayerFieldPositionGroup,
        visibility: &FreeAgentMarketVisibility,
    ) -> Result<(), FreeAgentBlockReason> {
        if !Self::passes_quality(candidate, buyer, group) {
            // Which SIDE of the band he fell off, read off the band itself
            // rather than guessed from a threshold — the diagnosis layer
            // shows this to the player, and "too good for everyone" and
            // "not good enough for anyone" are opposite answers.
            let floor = FreeAgentMarketCalculator::min_acceptable_ca(
                buyer.club_reputation_score,
                group,
                candidate.career_pressure,
            );
            return Err(if candidate.ability < floor {
                FreeAgentBlockReason::BelowMinimumAbility
            } else {
                FreeAgentBlockReason::AboveMaximumAbility
            });
        }
        if !Self::passes_reputation(candidate, buyer) {
            return Err(FreeAgentBlockReason::CountryReputationGap);
        }
        if !Self::passes_region(candidate, buyer) {
            return Err(FreeAgentBlockReason::RegionPrestigeGap);
        }
        if !Self::passes_market(candidate, buyer, visibility) {
            return Err(FreeAgentBlockReason::MarketUnfamiliar);
        }
        if buyer.would_block_registration(candidate.nationality_country_id) {
            return Err(FreeAgentBlockReason::NoRegistrationSlot);
        }
        Ok(())
    }

    /// Has this market heard of anybody like him?
    ///
    /// The emergency pass is the "random team that urgently needs a player"
    /// of the original complaint, and it was the one door with no visibility
    /// gate at all — the corridor entered only as six points of score, which
    /// a thin pool erases. A club that cannot field a side looks HARDER, and
    /// harder here is measured in rungs of the market ladder rather than in
    /// a slackened multiplier: `Strict` (depth cover) reads the bar as
    /// written, `Standard` (an urgent group fill) reads it one stage
    /// looser, and `Flexible` (the no-keeper carve-out) two. None of the
    /// three turns it off.
    fn passes_market(
        candidate: &FreeAgentCandidate,
        buyer: &EmergencyBuyerContext,
        visibility: &FreeAgentMarketVisibility,
    ) -> bool {
        let relief = match buyer.strictness {
            EmergencyStrictness::Strict => 0,
            EmergencyStrictness::Standard => 1,
            EmergencyStrictness::Flexible => 2,
        };
        visibility.is_visible_with_relief(candidate, relief)
    }

    /// Same CA band the normal global matcher uses, tuned per slot:
    /// `Flexible` (no-keeper GK) widens the floor so any registered
    /// goalkeeper qualifies; `Strict` (depth) tightens the ceiling so
    /// a buyer can't sign a star slumming under the "we needed a
    /// body" banner. Maps onto the existing
    /// `FreeAgentMarketCalculator::min_acceptable_ca` /
    /// `max_acceptable_ca` curves so the emergency band reads off the
    /// same tier-anchored math as everywhere else.
    fn passes_quality(
        candidate: &FreeAgentCandidate,
        buyer: &EmergencyBuyerContext,
        group: PlayerFieldPositionGroup,
    ) -> bool {
        let base_min = FreeAgentMarketCalculator::min_acceptable_ca(
            buyer.club_reputation_score,
            group,
            candidate.career_pressure,
        );
        let base_max = FreeAgentMarketCalculator::max_acceptable_ca(
            buyer.club_reputation_score,
            group,
            candidate.career_pressure,
        );
        let (eff_min, eff_max) = match buyer.strictness {
            EmergencyStrictness::Flexible => {
                (base_min.saturating_sub(15), base_max.saturating_add(5))
            }
            EmergencyStrictness::Standard => (base_min, base_max),
            EmergencyStrictness::Strict => {
                // Depth slots don't get the overreach band — a
                // 4500-rep buyer cannot credibly sign a CA-180 free
                // agent for emergency depth, even if pressure is high.
                (base_min, base_max.saturating_sub(5))
            }
        };
        candidate.ability >= eff_min && candidate.ability <= eff_max
    }

    /// Sliding country-rep gate, shared with the normal matcher.
    /// `Flexible` slots add an 800-point emergency bonus on top of
    /// the player-side allowance; `Standard` adds 400; `Strict`
    /// adds nothing — depth fills never get the urgent uplift.
    fn passes_reputation(candidate: &FreeAgentCandidate, buyer: &EmergencyBuyerContext) -> bool {
        let base = FreeAgentMarketCalculator::rep_drop_allowed(
            candidate.career_pressure,
            candidate.age,
            candidate.ability,
        );
        let bonus = match buyer.strictness {
            EmergencyStrictness::Flexible => 800,
            EmergencyStrictness::Standard => 400,
            EmergencyStrictness::Strict => 0,
        };
        let allowed = base + bonus;
        (buyer.country_reputation as i32 + allowed) >= candidate.reference_reputation as i32
    }

    /// Region-prestige gate, shared with the normal matcher. Same
    /// country always passes — domestic candidates skip the gate.
    /// Every strictness level fires the hard cross-continent guard,
    /// only the pressure floor differs: `Strict` 0.85, `Standard` 0.75,
    /// `Flexible` 0.65. The Flexible (no-keeper GK fill) floor is the
    /// softest carve-out we allow — a Russian veteran can land at a
    /// Cameroonian club when he is well past his peak and the team
    /// has no other keeper, but a routine mid-career Russian moving
    /// to West Africa for an emergency GK slot stays blocked. The
    /// previous "empty net beats any keeper" carve-out let routine
    /// step-downs through and was reported as unrealistic.
    fn passes_region(candidate: &FreeAgentCandidate, buyer: &EmergencyBuyerContext) -> bool {
        if candidate
            .nationality_country_code
            .eq_ignore_ascii_case(&buyer.country_code)
        {
            return true;
        }
        let same_continent = candidate.nationality_continent_id == buyer.continent_id;
        let cross_continent_min_pressure = match buyer.strictness {
            EmergencyStrictness::Strict => Some(0.85),
            EmergencyStrictness::Standard => Some(0.75),
            EmergencyStrictness::Flexible => Some(0.65),
        };
        if let Some(min_pressure) = cross_continent_min_pressure
            && FreeAgentMarketCalculator::cross_continent_blocked(
                same_continent,
                candidate.nationality_region.league_prestige(),
                buyer.region_prestige,
                candidate.career_pressure,
                min_pressure,
                candidate.reference_reputation,
                buyer.import_capacity,
            )
        {
            return false;
        }
        let base = FreeAgentMarketCalculator::region_drop_allowed(
            candidate.career_pressure,
            candidate.reference_reputation,
        );
        let strictness_extra = match buyer.strictness {
            EmergencyStrictness::Flexible => 0.20,
            EmergencyStrictness::Standard => 0.08,
            EmergencyStrictness::Strict => 0.0,
        };
        let continent_bonus = if same_continent { 0.05 } else { 0.0 };
        let allowed = base + strictness_extra + continent_bonus;
        candidate.nationality_region.league_prestige() <= buyer.region_prestige + allowed
    }
}

/// Pick the highest-scoring free-agent candidate for one emergency
/// slot. Returns `None` when no candidate clears the realism gates or
/// the strategy's minimum score. Sorting is delegated to
/// [`EmergencyCandidateOrdering`] so locality (domestic / in-country /
/// same-continent / pressure / rep-mismatch / ability fit) outranks
/// raw ability — the depth signing should be the realistic local
/// pick, not the strongest cross-region option.
struct EmergencyCandidatePicker;

impl EmergencyCandidatePicker {
    #[allow(clippy::too_many_arguments)]
    fn pick<'a>(
        candidates: &'a [FreeAgentCandidate],
        signings: &[FreeAgentSigning],
        rejected_locally: &HashSet<u32>,
        slot: EmergencyGroupSlot,
        buyer_ctx: &EmergencyBuyerContext,
        buying_club_id: u32,
        visibility: &FreeAgentMarketVisibility,
        recorder: &mut BlockReasonRecorder,
    ) -> Option<&'a FreeAgentCandidate> {
        let mut scored: Vec<(&FreeAgentCandidate, f32)> = candidates
            .iter()
            .filter(|c| c.club_id != buying_club_id)
            .filter(|c| c.position_group == slot.group)
            .filter(|c| !signings.iter().any(|s| s.player_id == c.player_id))
            .filter(|c| !rejected_locally.contains(&c.player_id))
            .filter(|c| {
                match EmergencyRealismGates::evaluate(c, buyer_ctx, slot.group, visibility) {
                    Ok(()) => true,
                    Err(reason) => {
                        if c.is_global_pool {
                            recorder.record(c.player_id, reason);
                        }
                        false
                    }
                }
            })
            .filter_map(|c| {
                let view = EmergencyCandidateView {
                    ability: c.ability,
                    age: c.age,
                    same_country_nationality: c
                        .nationality_country_code
                        .eq_ignore_ascii_case(&buyer_ctx.country_code),
                    same_continent: c.nationality_continent_id == buyer_ctx.continent_id,
                    reference_reputation: c.reference_reputation,
                    career_pressure: c.career_pressure,
                    region_prestige: c.nationality_region.league_prestige(),
                    is_global_pool: c.is_global_pool,
                    market_affinity: visibility.affinity_of(c.player_id),
                };
                EmergencySquadFillStrategy::score(&view, buyer_ctx).and_then(|score| {
                    if score < EmergencySquadFillStrategy::MIN_ACCEPTABLE_SCORE {
                        None
                    } else {
                        Some((c, score))
                    }
                })
            })
            .collect();

        scored.sort_by(|a, b| EmergencyCandidateOrdering::cmp(a, b, buyer_ctx, slot.group));
        // Don't hard-lock onto the single deterministic top: among the
        // genuinely interchangeable head of the ranking (same locality,
        // score within an epsilon) make a weighted random pick so the
        // same free agent doesn't funnel to the same club every tick.
        EmergencyTopClusterSelector::choose(&scored, buyer_ctx, slot.group)
    }
}

/// Picks one candidate from the interchangeable head of an already-sorted
/// emergency candidate list. The deterministic ordering in
/// [`EmergencyCandidateOrdering`] leaves a cluster of near-equal
/// candidates (same locality tier, score within
/// [`Self::SCORE_EPSILON`]) separated only by the continuous
/// `career_pressure` tiebreak — which always crowned the same player,
/// so the same club re-signed the same free agent season after season.
///
/// This selector keeps the full calibrated preference order (anyone who
/// beats the leader on a locality key sorts ahead and is the leader; the
/// cluster never reaches below it) but, *within* that already-near-equal
/// tier, draws a weighted random pick. The weight still favours the most
/// pressured / best-fitting candidate (the signals the deterministic
/// tiebreak used) so the previous favourite stays the favourite — just
/// not a certainty. A single-member cluster returns deterministically
/// with no RNG draw, so unambiguous picks (and their tests) are
/// unchanged.
struct EmergencyTopClusterSelector;

impl EmergencyTopClusterSelector {
    /// Score window (in raw score points) within which two candidates
    /// count as interchangeable. Sized below the gap between adjacent
    /// ability/locality buckets in [`EmergencySquadFillStrategy::score`]
    /// so the cluster never merges a meaningfully weaker fit with a
    /// stronger one — only candidates the ordering already treats as a
    /// near-tie are grouped.
    const SCORE_EPSILON: f32 = 6.0;
    /// Weight gain for a fully-pressured candidate. Keeps the most
    /// desperate free agent the favourite (~4x the base weight at full
    /// pressure) without the old 100% lock.
    const W_PRESSURE: f32 = 3.0;
    /// Weight gain for a perfect ability-fit candidate. Smaller than the
    /// pressure term — fit already shaped the score that defined the
    /// cluster; this just nudges ties.
    const W_FIT: f32 = 1.5;

    /// Length of the interchangeable prefix of `scored`. The list is
    /// pre-sorted by score(desc) then the locality keys, so the cluster
    /// is a contiguous run from index 0: every member is within
    /// [`Self::SCORE_EPSILON`] of the leader's score and shares the
    /// leader's three locality keys (domestic / in-country / continent).
    fn cluster_len(scored: &[(&FreeAgentCandidate, f32)], buyer: &EmergencyBuyerContext) -> usize {
        let Some((leader, leader_score)) = scored.first() else {
            return 0;
        };
        let leader_keys = Self::locality_keys(leader, buyer);
        let mut len = 1;
        for (candidate, score) in scored.iter().skip(1) {
            if (score - leader_score).abs() > Self::SCORE_EPSILON {
                break;
            }
            if Self::locality_keys(candidate, buyer) != leader_keys {
                break;
            }
            len += 1;
        }
        len
    }

    /// The three categorical locality flags the ordering keys on, packed
    /// so two candidates compare equal only when they sit in the same
    /// locality tier relative to the buyer.
    fn locality_keys(
        candidate: &FreeAgentCandidate,
        buyer: &EmergencyBuyerContext,
    ) -> (bool, bool, bool) {
        let domestic = candidate
            .nationality_country_code
            .eq_ignore_ascii_case(&buyer.country_code);
        let in_country = !candidate.is_global_pool;
        let same_continent = candidate.nationality_continent_id == buyer.continent_id;
        (domestic, in_country, same_continent)
    }

    /// Soft version of the deterministic `career_pressure` / `ability_fit`
    /// tiebreak: every cluster member keeps a nonzero base weight so it
    /// genuinely competes, with the most pressured / best-fitting members
    /// favoured.
    fn weight(
        candidate: &FreeAgentCandidate,
        buyer: &EmergencyBuyerContext,
        group: PlayerFieldPositionGroup,
    ) -> f32 {
        let min_ca = FreeAgentMarketCalculator::min_acceptable_ca(
            buyer.club_reputation_score,
            group,
            candidate.career_pressure,
        );
        let max_ca = FreeAgentMarketCalculator::max_acceptable_ca(
            buyer.club_reputation_score,
            group,
            candidate.career_pressure,
        );
        let ability_fit =
            FreeAgentMarketCalculator::quality_fit_score(candidate.ability, min_ca, max_ca);
        1.0 + candidate.career_pressure.clamp(0.0, 1.0) * Self::W_PRESSURE
            + ability_fit.clamp(0.0, 1.0) * Self::W_FIT
    }

    /// Return the chosen candidate. Empty list → `None`; single-member
    /// cluster → the leader with no RNG draw; otherwise a weighted
    /// roulette pick over the cluster.
    fn choose<'a>(
        scored: &[(&'a FreeAgentCandidate, f32)],
        buyer: &EmergencyBuyerContext,
        group: PlayerFieldPositionGroup,
    ) -> Option<&'a FreeAgentCandidate> {
        let len = Self::cluster_len(scored, buyer);
        if len == 0 {
            return None;
        }
        if len == 1 {
            return Some(scored[0].0);
        }
        let cluster = &scored[..len];
        let total: f32 = cluster
            .iter()
            .map(|(c, _)| Self::weight(c, buyer, group))
            .sum();
        // `total` >= len >= 2 because every weight carries a 1.0 base, so
        // the draw can't divide by zero or land outside the run.
        let roll = IntegerUtils::random(1, 1_000_000) as f32 / 1_000_000.0;
        let target = roll * total;
        let mut acc = 0.0;
        for (candidate, _) in cluster {
            acc += Self::weight(candidate, buyer, group);
            if acc >= target {
                return Some(candidate);
            }
        }
        // Float rounding fallback — return the last cluster member.
        cluster.last().map(|(c, _)| *c)
    }
}

/// Locality-aware ordering for emergency candidates. Score first so
/// genuinely unsuitable picks can't sneak through on a domestic-only
/// tiebreak, then the locality and fit criteria the user spec calls
/// out. Wrapped on a unit struct so the comparator and its key
/// stay together and the picker call site reads as one method call.
struct EmergencyCandidateOrdering;

impl EmergencyCandidateOrdering {
    /// Compare two scored candidates. Returns the ordering such that
    /// the better candidate sorts first (descending on score and the
    /// preference signals, ascending on rep mismatch).
    fn cmp(
        a: &(&FreeAgentCandidate, f32),
        b: &(&FreeAgentCandidate, f32),
        buyer: &EmergencyBuyerContext,
        group: PlayerFieldPositionGroup,
    ) -> Ordering {
        let ka = Self::key(a.0, a.1, buyer, group);
        let kb = Self::key(b.0, b.1, buyer, group);
        // Score (desc) > domestic > in-country > same-continent >
        // career pressure > smallest rep mismatch > best ability fit.
        kb.score
            .partial_cmp(&ka.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| kb.domestic.cmp(&ka.domestic))
            .then_with(|| kb.in_country.cmp(&ka.in_country))
            .then_with(|| kb.same_continent.cmp(&ka.same_continent))
            .then_with(|| {
                kb.career_pressure
                    .partial_cmp(&ka.career_pressure)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| ka.rep_mismatch.cmp(&kb.rep_mismatch))
            .then_with(|| {
                kb.ability_fit
                    .partial_cmp(&ka.ability_fit)
                    .unwrap_or(Ordering::Equal)
            })
    }

    fn key(
        candidate: &FreeAgentCandidate,
        score: f32,
        buyer: &EmergencyBuyerContext,
        group: PlayerFieldPositionGroup,
    ) -> EmergencyOrderingKey {
        let domestic = candidate
            .nationality_country_code
            .eq_ignore_ascii_case(&buyer.country_code);
        let in_country = !candidate.is_global_pool;
        let same_continent = candidate.nationality_continent_id == buyer.continent_id;
        let rep_mismatch =
            (candidate.reference_reputation as i32 - buyer.country_reputation as i32).abs();
        let min_ca = FreeAgentMarketCalculator::min_acceptable_ca(
            buyer.club_reputation_score,
            group,
            candidate.career_pressure,
        );
        let max_ca = FreeAgentMarketCalculator::max_acceptable_ca(
            buyer.club_reputation_score,
            group,
            candidate.career_pressure,
        );
        let ability_fit =
            FreeAgentMarketCalculator::quality_fit_score(candidate.ability, min_ca, max_ca);
        EmergencyOrderingKey {
            score,
            domestic,
            in_country,
            same_continent,
            career_pressure: candidate.career_pressure,
            rep_mismatch,
            ability_fit,
        }
    }
}

/// Packed sort key for the locality-aware ordering. Held by value so
/// the picker's `sort_by` closure can compare two keys without
/// re-running the per-field math twice per comparison.
struct EmergencyOrderingKey {
    score: f32,
    domestic: bool,
    in_country: bool,
    same_continent: bool,
    career_pressure: f32,
    rep_mismatch: i32,
    ability_fit: f32,
}

/// Mark a buying club's open transfer requests as fulfilled once the
/// emergency pass actually staged a signing for the matching group.
/// Without this every weekly re-evaluation would either find a stale
/// "needs this group" request still pending (and try to scout for it
/// again) or generate a duplicate. The dedup in `evaluate_squads`
/// only blocks NEW duplicates — it doesn't tidy fulfilled-but-stale
/// rows.
///
/// Idempotent: marks every matching unfulfilled row in the same group.
/// Multiple emergency signings in the same group during one tick will
/// each call this; the second call simply finds zero remaining matches.
struct TransferPlanSync;

impl TransferPlanSync {
    fn mark_group_fulfilled(country: &mut Country, club_id: u32, group: PlayerFieldPositionGroup) {
        if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
            for request in club.transfer_plan.transfer_requests.iter_mut() {
                if request.position.position_group() != group {
                    continue;
                }
                if request.status == TransferRequestStatus::Fulfilled
                    || request.status == TransferRequestStatus::Abandoned
                {
                    continue;
                }
                // A Negotiating request is owned by a live pursuit — the
                // resolver stamps it at resolution. Stamping it Fulfilled
                // here made the in-flight deal invisible to the plan: the
                // negotiation still completed and the club ended up with
                // two signings for one need.
                if request.status == TransferRequestStatus::Negotiating {
                    continue;
                }
                request.status = TransferRequestStatus::Fulfilled;
            }
        }
    }
}

/// Buyer-side anchors for one club evaluating free-agent candidates
/// against a transfer request. Bundles the tier / locality fields the
/// gates, ordering, and offer pricing all read so the request loop
/// passes one context instead of seven scalars.
struct RequestBuyerContext<'a> {
    club_score: f32,
    league_reputation: u16,
    negotiator_skill: u8,
    country_reputation: u16,
    continent_id: u32,
    region_prestige: f32,
    /// Per-candidate visibility of this market, built once for the whole
    /// country — see [`FreeAgentMarketVisibility`].
    visibility: &'a FreeAgentMarketVisibility,
    /// The club's room under its league's foreigner quota. Counted once per
    /// club, not per request: the quota is a squad fact.
    foreign_slots: ForeignSlotCount,
    /// The buying club's owner funding, 0..1. Ordering only — a rich club
    /// in a mid league hears about a name before its neighbours do, and
    /// that is the whole of what money buys on this path. It never opens a
    /// gate (memory `loan_market_argmax_predictability`: gates read truth,
    /// rankings read belief).
    benefactor: f32,
}

/// Hard-filter classifier for the request-driven matcher. The same
/// sliding career-pressure gates the legacy filter closure applied
/// (quality band, country rep, cross-continent, region prestige), but
/// returning the specific block reason instead of a bare `false` so
/// skipped global-pool candidates stay explainable in diagnosis.
struct RequestCandidateGates;

impl RequestCandidateGates {
    fn evaluate(
        candidate: &FreeAgentCandidate,
        buyer: &RequestBuyerContext<'_>,
        group: PlayerFieldPositionGroup,
        is_depth_request: bool,
        nominal_floor: u8,
    ) -> Result<(), FreeAgentBlockReason> {
        // Quality fit: tier-anchored band, slackened by pressure. The
        // request's own min-ability floor (minus the configured slack)
        // still applies — whichever is lower wins, because a free
        // agent below the nominal target is acceptable at zero fee.
        let min_ca = FreeAgentMarketCalculator::min_acceptable_ca(
            buyer.club_score,
            group,
            candidate.career_pressure,
        );
        let max_ca = FreeAgentMarketCalculator::max_acceptable_ca(
            buyer.club_score,
            group,
            candidate.career_pressure,
        );
        // Depth fills run the strict band: no star-overreach above the
        // buyer's tier ceiling — same trim the Strict emergency gate
        // applies.
        let max_ca = if is_depth_request {
            max_ca.saturating_sub(5)
        } else {
            max_ca
        };
        if candidate.ability < min_ca.min(nominal_floor) {
            return Err(FreeAgentBlockReason::BelowMinimumAbility);
        }
        if candidate.ability > max_ca {
            return Err(FreeAgentBlockReason::AboveMaximumAbility);
        }
        // Sliding country-rep gate.
        let rep_drop = FreeAgentMarketCalculator::rep_drop_allowed(
            candidate.career_pressure,
            candidate.age,
            candidate.ability,
        );
        if (buyer.country_reputation as i32 + rep_drop) < candidate.reference_reputation as i32 {
            return Err(FreeAgentBlockReason::CountryReputationGap);
        }
        // Hard cross-continent gate — mirrors the Strict emergency-
        // depth cut-off so the request-driven path can't bypass it.
        if FreeAgentMarketCalculator::cross_continent_blocked(
            candidate.nationality_continent_id == buyer.continent_id,
            candidate.nationality_region.league_prestige(),
            buyer.region_prestige,
            candidate.career_pressure,
            0.85,
            candidate.reference_reputation,
            buyer.visibility.import_capacity(),
        ) {
            return Err(FreeAgentBlockReason::CrossContinentPressureTooLow);
        }
        // Sliding region-prestige gate. At pressure 0 this collapses
        // to the legacy 0.20 threshold; at pressure 1.0 it widens to
        // 0.65.
        let region_drop = FreeAgentMarketCalculator::region_drop_allowed(
            candidate.career_pressure,
            candidate.reference_reputation,
        );
        if candidate.nationality_region.league_prestige() > buyer.region_prestige + region_drop {
            return Err(FreeAgentBlockReason::RegionPrestigeGap);
        }
        // Does this market have any reason to be looking at him at all? The
        // three gates above ask whether the move is a plausible STEP; this
        // one asks whether the two sides have ever heard of each other.
        // Widens with time on the market, along the corridors.
        if !buyer.visibility.is_visible(candidate) {
            return Err(FreeAgentBlockReason::MarketUnfamiliar);
        }
        // And could he be registered if he signed? The paid paths count
        // their foreigner slots before bidding; this door did not.
        if buyer
            .foreign_slots
            .would_block(candidate.nationality_country_id)
        {
            return Err(FreeAgentBlockReason::NoRegistrationSlot);
        }
        Ok(())
    }
}

/// Combined-score ordering for the request-driven matcher. Wraps the
/// priority computation and the comparator so the matcher call site
/// reads as two method calls (`priority`, then `sort_by(cmp)`).
struct RequestCandidateOrdering;

impl RequestCandidateOrdering {
    /// Priority score in [0,1] — see
    /// `FreeAgentMarketCalculator::candidate_priority_score`.
    fn priority(
        candidate: &FreeAgentCandidate,
        buyer: &RequestBuyerContext<'_>,
        group: PlayerFieldPositionGroup,
    ) -> f32 {
        let min_ca = FreeAgentMarketCalculator::min_acceptable_ca(
            buyer.club_score,
            group,
            candidate.career_pressure,
        );
        let max_ca = FreeAgentMarketCalculator::max_acceptable_ca(
            buyer.club_score,
            group,
            candidate.career_pressure,
        );
        let quality_fit =
            FreeAgentMarketCalculator::quality_fit_score(candidate.ability, min_ca, max_ca);
        let rep_mismatch = candidate.reference_reputation as i32 - buyer.country_reputation as i32;
        let pricing = FreeAgentOfferPricing::compute(
            candidate,
            group,
            buyer.club_score,
            buyer.league_reputation,
            buyer.negotiator_skill,
            buyer.country_reputation,
        );
        let wage_affordability =
            FreeAgentMarketCalculator::wage_score(pricing.offer_wage, pricing.reservation_wage);
        let base = FreeAgentMarketCalculator::candidate_priority_score(
            quality_fit,
            buyer.visibility.of(candidate.player_id),
            rep_mismatch,
            candidate.career_pressure,
            wage_affordability,
        );
        // Recently-released players get a short ordering bump so clubs
        // notice a fresh name before he fades into the long tail. Pool
        // players only — an in-country expiring contract (days_free 0)
        // isn't a "newly released into the market" signal. Ordering
        // only; it never relaxes the acceptance / realism gates.
        let fresh_release = if candidate.is_global_pool {
            FreeAgentMarketCalculator::recent_release_visibility_boost(candidate.days_free)
        } else {
            0.0
        };
        base + fresh_release + Self::owner_money_nudge(buyer, candidate)
    }

    /// Ordering weight for the buying club's owner money, 0..
    /// [`Self::OWNER_MONEY_WEIGHT`].
    ///
    /// The country-grain visibility layer prices a market, and every club in
    /// a league shares it. What it cannot say is that one of those clubs is
    /// funded by an owner and its neighbours are not — so the state-backed
    /// side of a mid league got to a released name in exactly the same
    /// order as the side living on gate receipts. This is the whole of the
    /// per-club money term on this path: it reorders, and reordering is
    /// what being first to the phone actually is.
    ///
    /// Domestic candidates are excluded: owner money is what reaches ACROSS
    /// a market, and a club needs no cheque to hear about a man in its own
    /// league.
    fn owner_money_nudge(buyer: &RequestBuyerContext<'_>, candidate: &FreeAgentCandidate) -> f32 {
        if candidate.nationality_country_id == 0 || !candidate.is_global_pool {
            return 0.0;
        }
        Self::OWNER_MONEY_WEIGHT * buyer.benefactor.clamp(0.0, 1.0)
    }

    /// Small on purpose. The priority score runs 0..1 and this sits an
    /// order of magnitude under the quality-fit term, so a fully funded
    /// club moves up the queue among candidates it already rates and never
    /// past one it does not.
    const OWNER_MONEY_WEIGHT: f32 = 0.03;

    /// Descending on priority; raw quality as the tiebreak so equal-
    /// priority candidates keep the legacy strongest-first order.
    fn cmp(a: &(&FreeAgentCandidate, f32), b: &(&FreeAgentCandidate, f32)) -> Ordering {
        b.1.partial_cmp(&a.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| {
                let qa = a.0.ability as u16 + a.0.potential as u16;
                let qb = b.0.ability as u16 + b.0.potential as u16;
                qb.cmp(&qa)
            })
    }
}

/// Per-tick collector of skip reasons for global-pool candidates.
/// Keeps the highest-ranked (closest-to-signing) reason per player;
/// drained into the `global_blocked` side-channel so Phase C can stamp
/// `FreeAgentMarketState::last_block` outside the country borrow.
pub(super) struct BlockReasonRecorder {
    reasons: HashMap<u32, FreeAgentBlockReason>,
}

impl BlockReasonRecorder {
    pub(super) fn new() -> Self {
        BlockReasonRecorder {
            reasons: HashMap::new(),
        }
    }

    pub(super) fn record(&mut self, player_id: u32, reason: FreeAgentBlockReason) {
        self.reasons
            .entry(player_id)
            .and_modify(|existing| {
                if reason.rank() > existing.rank() {
                    *existing = reason;
                }
            })
            .or_insert(reason);
    }

    pub(super) fn drain_into(self, out: &mut Vec<(u32, FreeAgentBlockReason)>) {
        out.extend(self.reasons);
    }
}

/// One open-capacity buyer row for the market-clearing pass. Cached
/// per club so the per-candidate buyer scan is field reads, not
/// repeated league / staff lookups. Group head-counts let the soft
/// (opportunistic) tier weigh how badly the club needs the candidate's
/// position before taking a punt on a free body.
struct MarketClearingBuyer {
    club_id: u32,
    club_score: f32,
    league_reputation: u16,
    negotiator_skill: u8,
    gk: u8,
    def: u8,
    mid: u8,
    fwd: u8,
    /// Room left under the league's foreigner quota. The clearing tiers
    /// are the market's backstop, not an exemption from registration.
    foreign_slots: ForeignSlotCount,
}

/// How well the clubs of one country know the market a single clearing
/// candidate comes out of.
///
/// Built per candidate and read per fitting club, so the ledger and the
/// scouting department are walked only for the handful of clubs whose
/// quality band actually fits him — which is what keeps a per-club,
/// per-candidate question off the hot path.
struct ClearingMarketKnowledge<'a> {
    country: &'a Country,
    map: &'a MarketMap,
    date: NaiveDate,
    nationality_country_id: u32,
    /// The league he last played in, `0` when he never held a modelled
    /// contract.
    last_country_id: u32,
}

impl ClearingMarketKnowledge<'_> {
    /// 0..1 — the better of what this club knows of his passport and of
    /// the league that released him. The maximum rather than a blend, for
    /// the same reason [`ClubMarketKnowledge::knowledge`] takes one: they
    /// are two ways of knowing the same man, not two halves of knowing him.
    fn of(&self, club_id: u32) -> f32 {
        if self.map.is_silent() {
            return 1.0;
        }
        let Some(club) = self.country.clubs.iter().find(|c| c.id == club_id) else {
            return 0.0;
        };
        // One pass over the department, not one per source country — a
        // scout knows a handful of countries, so inverting the walk turns a
        // scan per question into a scan per club.
        let mut coverage: HashMap<u32, u8> = HashMap::new();
        for staff in club.teams.iter().flat_map(|t| t.staffs.iter()) {
            for known in &staff.staff_attributes.knowledge.known_countries {
                let slot = coverage.entry(known.country_id).or_insert(0);
                *slot = (*slot).max(known.level);
            }
        }
        let mut best = 0.0f32;
        for source in [self.nationality_country_id, self.last_country_id] {
            if source == 0 {
                continue;
            }
            let level = coverage.get(&source).copied().unwrap_or(0);
            best = best.max(ClubMarketKnowledge::knowledge(
                self.map,
                self.country.id,
                &club.market_ledger,
                level,
                source,
                self.date,
            ));
        }
        best
    }
}

impl MarketClearingBuyer {
    /// Build the buyer rows for every club in `country` with open
    /// roster room, sorted lowest-tier first — the realistic landing
    /// spot for a long-unemployed journeyman is the small club that can
    /// use a cheap body, not the strongest club that happens to have
    /// space.
    fn rows_for_country(country: &Country) -> Vec<MarketClearingBuyer> {
        let registration = SquadRegistrationLimits::new(country.id, &country.regulations);
        let mut buyers: Vec<MarketClearingBuyer> = country
            .clubs
            .iter()
            .filter(|club| !club.teams.teams.is_empty() && ClubView::can_accept_player(club))
            .map(|club| {
                let main_team = club.teams.main().or_else(|| club.teams.teams.first());
                // overall_score — the unit the tier anchor curves expect.
                let club_score = main_team
                    .map(|t| t.reputation.overall_score().clamp(0.0, 1.0))
                    .unwrap_or(0.0);
                let league_reputation = main_team
                    .and_then(|t| t.league_id)
                    .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                    .map(|l| l.reputation)
                    .unwrap_or(0);
                let negotiator_skill = main_team
                    .and_then(|t| t.staffs.find_negotiator())
                    .map(|s| (s.staff_attributes.mental.man_management as u32 * 5).min(100) as u8)
                    .unwrap_or(50);
                let (mut gk, mut def, mut mid, mut fwd) = (0u8, 0u8, 0u8, 0u8);
                if let Some(team) = main_team {
                    for player in &team.players.players {
                        match player.position().position_group() {
                            PlayerFieldPositionGroup::Goalkeeper => gk = gk.saturating_add(1),
                            PlayerFieldPositionGroup::Defender => def = def.saturating_add(1),
                            PlayerFieldPositionGroup::Midfielder => mid = mid.saturating_add(1),
                            PlayerFieldPositionGroup::Forward => fwd = fwd.saturating_add(1),
                        }
                    }
                }
                MarketClearingBuyer {
                    club_id: club.id,
                    club_score,
                    league_reputation,
                    negotiator_skill,
                    gk,
                    def,
                    mid,
                    fwd,
                    foreign_slots: registration.count(club),
                }
            })
            .collect();
        buyers.sort_by(|a, b| {
            a.club_score
                .partial_cmp(&b.club_score)
                .unwrap_or(Ordering::Equal)
        });
        buyers
    }

    /// Current head-count in `group` for this buyer.
    fn group_count(&self, group: PlayerFieldPositionGroup) -> u8 {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => self.gk,
            PlayerFieldPositionGroup::Defender => self.def,
            PlayerFieldPositionGroup::Midfielder => self.mid,
            PlayerFieldPositionGroup::Forward => self.fwd,
        }
    }

    /// How badly the buyer needs another body in `group`, in [0,1].
    /// Thin groups score high; well-stocked ones score low. Measured
    /// against the group's real minimum viable depth (2 keepers, 7
    /// defenders, 7 midfielders, 4 forwards — the same floors the
    /// squad-needs analysis uses), not absolute head-counts: with the
    /// old 0-3 buckets every outfield group at a normal club read
    /// "fully stocked" (0.15) because real squads carry 7+ per line, so
    /// the largest term of the opportunistic fit score was a constant
    /// and the soft clearing tier could hardly ever fire. Feeds the
    /// opportunistic fit score's depth term.
    fn position_depth_need(&self, group: PlayerFieldPositionGroup) -> f32 {
        let min_viable: i16 = match group {
            PlayerFieldPositionGroup::Goalkeeper => 2,
            PlayerFieldPositionGroup::Defender => 7,
            PlayerFieldPositionGroup::Midfielder => 7,
            PlayerFieldPositionGroup::Forward => 4,
        };
        let have = self.group_count(group) as i16;
        match have - min_viable {
            i16::MIN..=-1 => 1.0, // below the viable floor — a real hole
            0 => 0.6,             // at the bare floor — one injury from a hole
            1 => 0.35,            // floor + 1 — useful cover still welcome
            _ => 0.15,            // genuinely stocked
        }
    }
}

/// Build a snapshot of `sim.free_agents` so per-country handlers can match
/// these players against club needs. Mutating: each free agent gets
/// `ensure_free_agent_state` called so the career-pressure score we
/// surface here is read from the player's own durable state. The
/// snapshot itself holds no Player reference — the simulator can
/// continue to mutate the pool while signings are being decided.
pub(crate) fn snapshot_global_free_agents(
    data: &mut SimulatorData,
    date: NaiveDate,
) -> Vec<GlobalFreeAgentSummary> {
    // Pass 1 (immutable): resolve nationality info per unique country
    // in parallel. Two-stage resolve mirrors the rest of the transfer
    // pipeline: an active country (full `Country`) first, then the
    // lighter `country_info` map. Without the second stage, the gates
    // fall back to permissive defaults and an Argentinian free agent
    // slips through to a Mali buyer. Build a cache keyed by country_id
    // so the mutable pass below doesn't need a SimulatorData borrow.
    let unique_country_ids: HashSet<u32> = data.free_agents.iter().map(|p| p.country_id).collect();
    let nationality_cache: HashMap<u32, (u16, u32, String)> = {
        let data_ref: &SimulatorData = data;
        unique_country_ids
            .into_par_iter()
            .map(|cid| {
                let resolved = data_ref
                    .country(cid)
                    .map(|c| (c.reputation, c.continent_id, c.code.clone()))
                    .or_else(|| {
                        data_ref
                            .country_info
                            .get(&cid)
                            .map(|c| (c.reputation, c.continent_id, c.code.clone()))
                    })
                    // Truly unknown nationality: fail-closed on the rep
                    // gate (`u16::MAX` blocks every buyer) and pin the
                    // region to the most prestigious one so the
                    // prestige gate also rejects, instead of opening
                    // every door. Loudly: a player carrying this
                    // fallback can NEVER be signed, so silent data
                    // holes would read as "the market ignores him".
                    .unwrap_or_else(|| {
                        let first_report = UNKNOWN_NATIONALITY_WARNED
                            .lock()
                            .map(|mut seen| seen.insert(cid))
                            .unwrap_or(true);
                        if first_report {
                            warn!(
                                "free-agent snapshot: unknown nationality country {cid} — \
                                 affected players are blocked from every buyer until \
                                 retirement resolves them"
                            );
                        } else {
                            debug!("free-agent snapshot: unknown nationality country {cid}");
                        }
                        (u16::MAX, 1, "gb".to_string())
                    });
                (cid, resolved)
            })
            .collect()
    };

    // Pass 2 (mutable on the pool only): seed market state for any
    // free agent who arrived without it (database-only entries that
    // never came through `on_release`), then build the snapshot row.
    // Each iteration mutates only its own `Player`, so this runs in
    // parallel safely.
    data.free_agents
        .par_iter_mut()
        .map(|player| {
            let (nationality_rep, nationality_continent_id, nationality_country_code) =
                nationality_cache
                    .get(&player.country_id)
                    .cloned()
                    .unwrap_or_else(|| (u16::MAX, 1, "gb".to_string()));
            player.ensure_free_agent_state(date, nationality_rep);
            // Unknown-nationality fallback can never pass the rep gate
            // — stamp the diagnosis reason so the audit layer reports
            // the data hole instead of a mysterious endless sit.
            if nationality_rep == u16::MAX {
                player.on_market_blocked(date, FreeAgentBlockReason::UnknownNationality);
            }

            let career_pressure = player.career_pressure(date);
            let (
                last_salary,
                last_country_reputation,
                last_league_reputation,
                days_free,
                last_country_id,
            ) = player
                .free_agent_state()
                .map(|s| {
                    (
                        s.last_salary,
                        s.last_country_reputation,
                        s.last_league_reputation,
                        (date - s.free_since).num_days().max(0),
                        // A database free agent who never held a modelled
                        // contract has no last league; his own country is
                        // the only market he is known in, which is the
                        // truth about him rather than a fallback.
                        s.last_country_id.unwrap_or(player.country_id),
                    )
                })
                .unwrap_or((
                    0,
                    nationality_rep,
                    ((nationality_rep as f32) * 0.75) as u16,
                    0,
                    player.country_id,
                ));
            // Time on the market erodes the old-league prestige the
            // country/region gates key on — see
            // `decayed_reference_reputation`. (The unknown-nationality
            // u16::MAX sentinel survives the decay: 55% of MAX still
            // out-reps every buyer, so the fail-closed block holds.)
            let reference_reputation = FreeAgentMarketCalculator::decayed_reference_reputation(
                player.reference_reputation(nationality_rep),
                days_free,
            );
            let failed_approach_streak = player
                .free_agent_state()
                .map(|s| s.failed_approach_streak)
                .unwrap_or(0);

            GlobalFreeAgentSummary {
                player_id: player.id,
                player_name: player.full_name.to_string(),
                ability: player.player_attributes.current_ability,
                // Observable ceiling — pool matchers are club decisions
                // and must not see hidden biological PA.
                potential: PotentialEstimator::observable_ceiling(player, date),
                age: player.age(date),
                position_group: player.position().position_group(),
                nationality_country_reputation: nationality_rep,
                nationality_continent_id,
                nationality_country_code,
                nationality_country_id: player.country_id,
                career_pressure,
                days_free,
                reference_reputation,
                last_salary,
                last_country_reputation,
                last_league_reputation,
                world_reputation: player.player_attributes.world_reputation,
                current_reputation: player.player_attributes.current_reputation,
                professionalism_norm: (player.attributes.professionalism / 20.0).clamp(0.0, 1.0),
                failed_approach_streak,
                last_country_id,
            }
        })
        .collect()
}

/// Snapshot of the buying side captured *before* we take a mutable borrow
/// on `SimulatorData` to remove the player from the global pool. Holds
/// everything `Player::complete_free_agent_signing` needs to install the
/// contract, seed the signing plan, and push the destination career row.
struct BuyingClubSnapshot {
    to_info: TeamInfo,
    league_reputation: u16,
}

/// Resolve the buying club's `TeamInfo` and league reputation from a
/// read-only borrow. Returns `None` if the country/club/main team chain
/// is incomplete or if the club is at squad capacity.
fn snapshot_buying_club(
    data: &SimulatorData,
    buying_country_id: u32,
    buying_club_id: u32,
) -> Option<BuyingClubSnapshot> {
    let country = data.country(buying_country_id)?;
    let club = country.clubs.iter().find(|c| c.id == buying_club_id)?;
    if club.teams.teams.is_empty() || !ClubView::can_accept_player(club) {
        return None;
    }
    let main_team = club.teams.main().or_else(|| club.teams.teams.first())?;
    let (league_name, league_slug, league_reputation) = main_team
        .league_id
        .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
        .map(|l| (l.name.clone(), l.slug.clone(), l.reputation))
        .unwrap_or_default();
    Some(BuyingClubSnapshot {
        to_info: TeamInfo {
            name: club.name.clone(),
            slug: main_team.slug.clone(),
            reputation: main_team.reputation.world,
            league_name,
            league_slug,
        },
        league_reputation,
    })
}

/// Execute a deferred global free-agent signing produced by
/// `handle_free_agents`. Returns true if the player was placed at the
/// buying club. First-come-first-served deduplication: if another country
/// already claimed the player earlier in the same tick, the lookup misses
/// and we return false silently.
///
/// The signing flows through `Player::complete_free_agent_signing` — the
/// no-source-club mirror of `complete_transfer`. Career history goes
/// through `record_free_agent_signing`, which only pushes the destination
/// row, so games the player accumulated at their previous club stay
/// attributed to that club rather than to a synthetic "Free Agent" entry.
/// The "Free Agent" string survives only on the country-level
/// `CompletedTransfer` log written below, where it is the correct label.
pub(crate) fn execute_global_free_agent_signing(
    data: &mut SimulatorData,
    signing: &GlobalFreeAgentSigning,
    date: NaiveDate,
    _config: &TransferConfig,
) -> bool {
    // Pre-check 1: is the player still in the global pool?
    let player_idx = match data
        .free_agents
        .iter()
        .position(|p| p.id == signing.player_id)
    {
        Some(i) => i,
        None => return false,
    };

    // Pre-check 2: buying club exists, has a team to place into, and can
    // still accept a player. Capture the destination snapshot now while
    // we hold the read borrow; we'll need it after we mutate the pool.
    let snapshot =
        match snapshot_buying_club(data, signing.buying_country_id, signing.buying_club_id) {
            Some(s) => s,
            None => return false,
        };

    // All pre-checks passed — take the player out of the pool.
    let mut player = data.free_agents.swap_remove(player_idx);

    // Where he came FROM, read before the signing clears his market state.
    // A pool signing writes `from_club_id: 0`, so this is the only record
    // the world keeps of which league released him — and the free-agent
    // corridor is a real corridor: a released man signs where he is known.
    let origin_country_id = player
        .free_agent_state()
        .and_then(|state| state.last_country_id)
        .unwrap_or(0);

    // Use the no-source-club completion path: contract install, signing
    // plan, and pending-signing run identically to a paid transfer, but
    // career history goes through `on_free_agent_signing` so we don't
    // fabricate a "Free Agent" career row for games that were actually
    // played at the player's previous club.
    let agreed_wage = signing.terms.map(|t| t.annual_wage);
    player.complete_free_agent_signing(
        &snapshot.to_info,
        date,
        signing.buying_club_id,
        snapshot.league_reputation,
        agreed_wage,
    );
    // Honour staged emergency contract terms (length, role promise).
    // `complete_free_agent_signing` installs the wage above via
    // `install_permanent_contract`; rewriting the contract here with
    // the term-aware installer makes the contract length, role
    // promise, and signing bonus stick. Without this the global-pool
    // path silently gives every emergency signing a 4–5 year
    // calculator-default deal and the in-country / global flows
    // drift apart.
    if let Some(terms) = signing.terms {
        let personal_terms = terms.to_personal_terms();
        player.install_permanent_contract_with_terms(
            date,
            snapshot.to_info.reputation,
            snapshot.league_reputation,
            Some(terms.annual_wage),
            Some(&personal_terms),
        );
    }

    // Now place the player at the buying club and write the country-level
    // market history entry. Re-borrow mutably; pre-checks above guarantee
    // the country/club lookup will succeed, but we still bail safely if
    // they don't (and restore the player to the pool).
    let buying_country = match data.country_mut(signing.buying_country_id) {
        Some(c) => c,
        None => {
            data.free_agents.push(player);
            return false;
        }
    };

    let buying_club_idx = match buying_country
        .clubs
        .iter()
        .position(|c| c.id == signing.buying_club_id)
    {
        Some(i) => i,
        None => {
            let _ = buying_country;
            data.free_agents.push(player);
            return false;
        }
    };

    let buying_club_name = buying_country.clubs[buying_club_idx].name.clone();

    // Reception ingredients — captured before `player` moves into the
    // roster and before the club goes mutably borrowed.
    let arrival_country_id = player.country_id;
    let club_country_id = buying_country.id;
    let club_country_code = buying_country.code.clone();
    let arrival_threat = ArrivalThreatProfile::from_player(&player, date);

    // Main team by TYPE — `teams[0]` is not guaranteed to be the Main
    // squad, and the contract/history identity and squad-cap check above
    // were already keyed to it. The historical first-team insert rostered
    // pool signings on whatever squad happened to sit first.
    TransferExecution::add_to_main_team(&mut buying_country.clubs[buying_club_idx], player);

    // A pool signing is business in a market, exactly like a paid one: the
    // four paid executors write the ledger and this door did not, so a club
    // that signed three released Colombians learned nothing about Colombia
    // and its knowledge of the market never moved.
    MarketLedgerUpdate::on_signing(
        &mut buying_country.clubs[buying_club_idx],
        club_country_id,
        origin_country_id,
        arrival_country_id,
        date,
    );

    // A pool signing walks into a dressing room like any other arrival —
    // this path used to install him with no compatriot / competition /
    // investment reaction at all.
    SquadReactionPass::arrival_reception(
        &mut buying_country.clubs[buying_club_idx],
        signing.player_id,
        arrival_country_id,
        club_country_id,
        &club_country_code,
        &arrival_threat,
        0.0,
        date,
    );

    // Country-level market log (separate from the player's career history
    // populated above by `complete_free_agent_signing`).
    buying_country.transfer_market.transfer_history.push(
        CompletedTransfer::new(
            signing.player_id,
            signing.player_name.clone(),
            0,
            0,
            "Free Agent".to_string(),
            signing.buying_club_id,
            buying_club_name,
            date,
            CurrencyValue::new(0.0, Currency::Usd),
            TransferType::Free,
        )
        .with_reason(signing.reason.clone())
        .with_origin_country(origin_country_id),
    );

    PipelineProcessor::clear_player_interest(buying_country, signing.player_id);

    // Stale interest in OTHER countries — monitoring or shortlist rows
    // that survived the local clear — is swept by the caller. That sweep
    // walks the whole world and costs the same whether it strips one
    // player or a hundred, so firing it per signing made the drain
    // O(signings x world); the caller batches every id it just placed and
    // sweeps once, before it initiates any foreign negotiation.

    // Monthly diagnostics flow counter — a player just left the global
    // pool for a club, which a later point-in-time scan can't recover.
    data.free_agent_flow.signed_from_global_pool = data
        .free_agent_flow
        .signed_from_global_pool
        .saturating_add(1);

    debug!(
        "Free agent signing (global pool): player {} → club {} in country {}",
        signing.player_id, signing.buying_club_id, signing.buying_country_id
    );

    true
}

#[cfg(test)]
mod emergency;
#[cfg(test)]
mod expiry;

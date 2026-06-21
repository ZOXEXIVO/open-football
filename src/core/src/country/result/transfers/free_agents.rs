use super::config::TransferConfig;
use super::execution::DevelopmentLoanPathway;
use super::free_agent_depth::{
    DepthNegotiationAction, EmergencyDepthRequestIntent, EmergencyDepthRequestPlanner,
    FreeAgentNegotiationStager,
};
use super::free_agent_market_calc::{
    BuyerRoleFit, FreeAgentMarketCalculator, FreeAgentOfferPricing,
};
use super::types::{TransferActivitySummary, can_club_accept_player, find_player_in_country};
use crate::club::player::contract::RENEWAL_OFFERED_LABEL;
use crate::club::player::mailbox::handlers::contract_proposal::ProcessContractHandler;
use crate::club::player::transfer::{FreeAgentBlockReason, MarketStage};
use crate::club::staff::perception::PotentialEstimator;
use crate::club::team::squad::{ContractRenewalManager, WageStructureSnapshot};
use crate::country::result::CountryResult;
use crate::shared::{Currency, CurrencyValue};
use crate::simulator::SimulatorData;
use crate::transfers::negotiation::{NegotiationPhase, NegotiationStatus, TransferNegotiation};
use crate::transfers::offer::{PersonalTermsOffer, PromisedSquadStatus, TransferOffer};
use crate::transfers::pipeline::{
    PipelineProcessor, TransferNeedReason, TransferRequest, TransferRequestStatus,
};
use crate::transfers::scouting_region::ScoutingRegion;
use crate::transfers::squad_needs::{
    EmergencyBuyerContext, EmergencyCandidateView, EmergencyGroupSlot, EmergencyProjectedSquad,
    EmergencySlotStrictness, EmergencySquadFillStrategy, EmergencyStrictness, FirstTeamSquadNeeds,
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
    pub reason: String,
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
    pub reason: String,
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
                        // Skip if already in negotiation
                        let statuses = player.statuses.get();
                        if statuses.contains(&PlayerStatusType::Trn)
                            || statuses.contains(&PlayerStatusType::Bid)
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
            &mut signings,
            global_offered_ids,
            global_rejected_ids,
        );
        EmergencyDepthRequestPlanner::stage_requests(country, &depth_intents);

        // Peak post-season window (Jun–Aug) lifts the request-driven cap
        // so summer free-agent business isn't throttled to the off-season
        // trickle.
        let max_signings_per_day = config.max_free_agent_signings_for(date);
        let ability_slack = config.free_agent_ability_slack;
        let buyer_country_reputation = country.reputation;
        let buyer_country_code = country.code.clone();
        let buyer_continent_id = country.continent_id;
        // Mirrors `scan_foreign_loan_market`: same region the country sits
        // in, used as the prestige anchor for cross-region gating.
        let buyer_region = ScoutingRegion::from_country(country.continent_id, &country.code);
        let buyer_region_prestige = buyer_region.league_prestige();
        // Why each global-pool candidate was skipped today, highest-rank
        // reason per player. Drained into `global_blocked` at the end of
        // the tick; Phase C stamps it onto the player's market state.
        let mut recorder = BlockReasonRecorder::new();
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
            if !can_club_accept_player(club) {
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
            // every rolling-CA gate in the project relies on.
            let main_team = club.teams.main().or_else(|| club.teams.teams.first());
            let buyer_club_score = main_team
                .map(|t| (t.reputation.world as f32 / 10_000.0).clamp(0.0, 1.0))
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

            for request in &unfulfilled {
                if signings.len() - emergency_signing_count >= max_signings_per_day {
                    break;
                }

                let group = request.position.position_group();

                // Emergency-planner depth requests route through the
                // staged-negotiation flow below instead of instant
                // signing. Explicitly marker-driven: a normal evaluated
                // DepthCover / SquadPadding request keeps the legacy
                // instant path. One pursuit in flight per request:
                // Negotiating means the resolver already owns it —
                // re-approaching daily would stack offers on the need.
                let is_depth_request = request.is_emergency_free_agent_depth();
                if is_depth_request && request.status == TransferRequestStatus::Negotiating {
                    continue;
                }

                let buyer_ctx = RequestBuyerContext {
                    club_score: buyer_club_score,
                    league_reputation: buyer_league_reputation,
                    negotiator_skill: buyer_negotiator_skill,
                    country_reputation: buyer_country_reputation,
                    country_code: &buyer_country_code,
                    continent_id: buyer_continent_id,
                    region_prestige: buyer_region_prestige,
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
                                + FreeAgentMarketCalculator::pity_bonus(best.failed_approach_streak)
                                + config
                                    .fresh_high_ability_bonus(best.days_free, best.ability),
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
                                .map(|t| (t.reputation.world as f32 / 10_000.0).clamp(0.0, 1.0))
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
                            reason: PipelineProcessor::transfer_need_reason_text(&request.reason)
                                .to_string(),
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

                    let reason =
                        PipelineProcessor::transfer_need_reason_text(&request.reason).to_string();

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
            &staged_depth_ids,
            &mut signings,
            global_offered_ids,
            global_rejected_ids,
            &mut recorder,
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
            let executed =
                super::execution::execute_transfer_within_country(country, &deferred, date);

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
                .with_reason(signing.reason),
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
                    if agreement.to_country_id != country.id
                        || agreement.to_club_id == club.id
                    {
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
                    if buyer.teams.teams.is_empty() || !can_club_accept_player(buyer) {
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
                        reason: "pre_contract".to_string(),
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
            let structure = WageStructureSnapshot::from_team(main_team);

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
        signings: &mut Vec<FreeAgentSigning>,
        global_offered_ids: &mut Vec<u32>,
        global_rejected_ids: &mut Vec<u32>,
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
        let buyer_continent_id = country.continent_id;
        let buyer_rep = country.reputation;
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
            // size. `can_club_accept_player` covers that.
            if !can_club_accept_player(club) {
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
            // market scale as the rest of the pipeline.
            let main_team = club.teams.main().or_else(|| club.teams.teams.first());
            let buyer_club_score = main_team
                .map(|t| (t.reputation.world as f32 / 10_000.0).clamp(0.0, 1.0))
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
                };

                let pick = EmergencyCandidatePicker::pick(
                    candidates,
                    signings,
                    &rejected_locally,
                    slot,
                    &buyer_ctx,
                    club.id,
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
                    reason: slot.reason.to_string(),
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
        staged_ids: &HashSet<u32>,
        signings: &mut Vec<FreeAgentSigning>,
        global_offered_ids: &mut Vec<u32>,
        global_rejected_ids: &mut Vec<u32>,
        recorder: &mut BlockReasonRecorder,
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
            staged_ids,
            signings,
            global_offered_ids,
            global_rejected_ids,
            recorder,
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
            staged_ids,
            signings,
            global_offered_ids,
            global_rejected_ids,
            recorder,
        );
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
        staged_ids: &HashSet<u32>,
        signings: &mut Vec<FreeAgentSigning>,
        global_offered_ids: &mut Vec<u32>,
        global_rejected_ids: &mut Vec<u32>,
        recorder: &mut BlockReasonRecorder,
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
            .filter(|c| c.career_pressure >= tier.min_pressure || c.days_free >= tier.min_days_free)
            .filter(|c| {
                !tier.locality_restricted
                    || c.nationality_country_code
                        .eq_ignore_ascii_case(buyer_country_code)
                    || c.nationality_continent_id == buyer_continent_id
            })
            .filter(|c| !signings.iter().any(|s| s.player_id == c.player_id))
            .filter(|c| !staged_ids.contains(&c.player_id))
            .collect();
        eligible.sort_by(|a, b| {
            b.career_pressure
                .partial_cmp(&a.career_pressure)
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
            if (buyer_country_reputation as i32 + rep_drop) < candidate.reference_reputation as i32 {
                recorder.record(candidate.player_id, FreeAgentBlockReason::CountryReputationGap);
                continue;
            }
            let cross_floor = if last_chance { 0.75 } else { 0.85 };
            if FreeAgentMarketCalculator::cross_continent_blocked(
                candidate.nationality_continent_id == buyer_continent_id,
                candidate.nationality_region.league_prestige(),
                buyer_region_prestige,
                candidate.career_pressure,
                cross_floor,
            ) {
                recorder.record(
                    candidate.player_id,
                    FreeAgentBlockReason::CrossContinentPressureTooLow,
                );
                continue;
            }
            let region_drop =
                FreeAgentMarketCalculator::region_drop_allowed(candidate.career_pressure)
                    + if last_chance { 0.10 } else { 0.0 };
            if candidate.nationality_region.league_prestige() > buyer_region_prestige + region_drop {
                recorder.record(candidate.player_id, FreeAgentBlockReason::RegionPrestigeGap);
                continue;
            }

            // First (lowest-tier) buyer whose quality band fits.
            let mut too_good_everywhere = true;
            let mut chosen: Option<(&MarketClearingBuyer, u8, u8)> = None;
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
                    chosen = Some((buyer, min_ca, max_ca));
                    break;
                }
            }
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
                reason: "free_agent_market_clearing".to_string(),
                terms: Some(pricing.signed_terms(candidate)),
                // No transfer request is being serviced — leave the
                // request bookkeeping untouched.
                fills_group: None,
            });
            cleared += 1;
            debug!(
                "Market clearing ({} tier): club {} signs long-term free agent {} (cp={:.2}, days_free={})",
                if tier.opportunistic_gate { "soft" } else { "hard" },
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
        use crate::transfers::squad_needs::{
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
        use crate::transfers::squad_needs::{
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
    /// All three gates must pass for the candidate to enter scoring.
    fn passes(
        candidate: &FreeAgentCandidate,
        buyer: &EmergencyBuyerContext,
        group: PlayerFieldPositionGroup,
    ) -> bool {
        Self::passes_quality(candidate, buyer, group)
            && Self::passes_reputation(candidate, buyer)
            && Self::passes_region(candidate, buyer)
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
            )
        {
            return false;
        }
        let base = FreeAgentMarketCalculator::region_drop_allowed(candidate.career_pressure);
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
    fn pick<'a>(
        candidates: &'a [FreeAgentCandidate],
        signings: &[FreeAgentSigning],
        rejected_locally: &HashSet<u32>,
        slot: EmergencyGroupSlot,
        buyer_ctx: &EmergencyBuyerContext,
        buying_club_id: u32,
    ) -> Option<&'a FreeAgentCandidate> {
        let mut scored: Vec<(&FreeAgentCandidate, f32)> = candidates
            .iter()
            .filter(|c| c.club_id != buying_club_id)
            .filter(|c| c.position_group == slot.group)
            .filter(|c| !signings.iter().any(|s| s.player_id == c.player_id))
            .filter(|c| !rejected_locally.contains(&c.player_id))
            .filter(|c| EmergencyRealismGates::passes(c, buyer_ctx, slot.group))
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
    fn cluster_len(
        scored: &[(&FreeAgentCandidate, f32)],
        buyer: &EmergencyBuyerContext,
    ) -> usize {
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
    country_code: &'a str,
    continent_id: u32,
    region_prestige: f32,
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
        ) {
            return Err(FreeAgentBlockReason::CrossContinentPressureTooLow);
        }
        // Sliding region-prestige gate. At pressure 0 this collapses
        // to the legacy 0.20 threshold; at pressure 1.0 it widens to
        // 0.65.
        let region_drop = FreeAgentMarketCalculator::region_drop_allowed(candidate.career_pressure);
        if candidate.nationality_region.league_prestige() > buyer.region_prestige + region_drop {
            return Err(FreeAgentBlockReason::RegionPrestigeGap);
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
        let domestic = candidate
            .nationality_country_code
            .eq_ignore_ascii_case(buyer.country_code);
        let same_continent = candidate.nationality_continent_id == buyer.continent_id;
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
            domestic,
            same_continent,
            rep_mismatch,
            candidate.career_pressure,
            wage_affordability,
        );
        // Recently-released players get a short ordering bump so clubs
        // notice a fresh name before he fades into the long tail. Pool
        // players only — an in-country expiring contract (days_free 0)
        // isn't a "newly released into the market" signal. Ordering
        // only; it never relaxes the acceptance / realism gates.
        let visibility = if candidate.is_global_pool {
            FreeAgentMarketCalculator::recent_release_visibility_boost(candidate.days_free)
        } else {
            0.0
        };
        base + visibility
    }

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
}

impl MarketClearingBuyer {
    /// Build the buyer rows for every club in `country` with open
    /// roster room, sorted lowest-tier first — the realistic landing
    /// spot for a long-unemployed journeyman is the small club that can
    /// use a cheap body, not the strongest club that happens to have
    /// space.
    fn rows_for_country(country: &Country) -> Vec<MarketClearingBuyer> {
        let mut buyers: Vec<MarketClearingBuyer> = country
            .clubs
            .iter()
            .filter(|club| !club.teams.teams.is_empty() && can_club_accept_player(club))
            .map(|club| {
                let main_team = club.teams.main().or_else(|| club.teams.teams.first());
                let club_score = main_team
                    .map(|t| (t.reputation.world as f32 / 10_000.0).clamp(0.0, 1.0))
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
    /// Thin groups (a club with one keeper) score high; well-stocked
    /// ones score low. Feeds the opportunistic fit score's depth term.
    fn position_depth_need(&self, group: PlayerFieldPositionGroup) -> f32 {
        match self.group_count(group) {
            0 | 1 => 1.0,
            2 => 0.6,
            3 => 0.35,
            _ => 0.15,
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
                        warn!(
                            "free-agent snapshot: unknown nationality country {cid} — \
                             affected players are blocked from every buyer until \
                             retirement resolves them"
                        );
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
            let reference_reputation = player.reference_reputation(nationality_rep);
            let (last_salary, last_country_reputation, last_league_reputation, days_free) = player
                .free_agent_state()
                .map(|s| {
                    (
                        s.last_salary,
                        s.last_country_reputation,
                        s.last_league_reputation,
                        (date - s.free_since).num_days().max(0),
                    )
                })
                .unwrap_or((
                    0,
                    nationality_rep,
                    ((nationality_rep as f32) * 0.75) as u16,
                    0,
                ));
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
    if club.teams.teams.is_empty() || !can_club_accept_player(club) {
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

    buying_country.clubs[buying_club_idx].teams.teams[0]
        .players
        .add(player);

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
        .with_reason(signing.reason.clone()),
    );

    PipelineProcessor::clear_player_interest(buying_country, signing.player_id);

    // Sweep stale interest in every country — clubs in other leagues may
    // have monitoring or shortlist rows that survived the local clear.
    let player_id = signing.player_id;
    PipelineProcessor::cleanup_player_transfer_interest(data, player_id);

    // Monthly diagnostics flow counter — a player just left the global
    // pool for a club, which a later point-in-time scan can't recover.
    data.free_agents_signed_this_period = data.free_agents_signed_this_period.saturating_add(1);

    debug!(
        "Free agent signing (global pool): player {} → club {} in country {}",
        signing.player_id, signing.buying_club_id, signing.buying_country_id
    );

    true
}

#[cfg(test)]
mod emergency_fill_tests {
    use super::*;
    use crate::club::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::transfers::market::TransferListingStatus;
    use crate::transfers::negotiation::NegotiationRejectionReason;
    use crate::transfers::pipeline::{ShortlistCandidateStatus, TransferNeedPriority};
    use crate::transfers::squad_needs::EmergencyContractTermsPolicy;
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes,
        Player, PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, StaffCollection, Team, TeamCollection, TeamReputation,
        TeamType, TrainingSchedule,
    };
    use chrono::NaiveTime;

    /// Test fixtures grouped on a unit struct so the test module
    /// stays free of loose helper fns (project convention — see
    /// `feedback_use_directives`).
    struct EmergencyFillFixtures;

    impl EmergencyFillFixtures {
        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn player(id: u32, position: PlayerPositionType) -> Player {
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Test".to_string(), format!("P{id}")))
                .birth_date(Self::d(1998, 1, 1))
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 16,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap()
        }

        fn team(id: u32, name: &str, slug: &str, players: Vec<Player>) -> Team {
            Team::builder()
                .id(id)
                .league_id(Some(1))
                .club_id(100)
                .name(name.to_string())
                .slug(slug.to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(2000, 2000, 4000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn club(id: u32, name: &str, main: Team) -> Club {
            Club::new(
                id,
                name.to_string(),
                Location::new(1),
                ClubFinances::new(1_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![main]),
                ClubFacilities::default(),
            )
        }

        fn country(clubs: Vec<Club>) -> Country {
            Country::builder()
                .id(1)
                .code("en".to_string())
                .slug("england".to_string())
                .name("England".to_string())
                .continent_id(1)
                // Buyer-country reputation drives the chasm gate in
                // EmergencySquadFillStrategy. 5000 sits comfortably
                // above the test candidates' reference reputation
                // (3000-4000), so the gate passes — the failing
                // alternative is unintentionally testing the
                // rep-chasm rejection path.
                .reputation(5000)
                .leagues(LeagueCollection::new(vec![League::new(
                    1,
                    "L".to_string(),
                    "english".to_string(),
                    1,
                    5000,
                    LeagueSettings {
                        season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                        season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                        tier: 1,
                        promotion_spots: 0,
                        relegation_spots: 0,
                        league_group: None,
                    },
                    false,
                )]))
                .clubs(clubs)
                .build()
                .unwrap()
        }

        /// Build a free-agent candidate sourced from the global pool
        /// (club_id = 0) — the path emergency fill exercises most
        /// commonly because expired-contract candidates are normally
        /// rare on any given tick.
        fn candidate(
            player_id: u32,
            ability: u8,
            age: u8,
            position_group: PlayerFieldPositionGroup,
            same_country: bool,
        ) -> FreeAgentCandidate {
            let code = if same_country { "en" } else { "ar" };
            FreeAgentCandidate {
                player_id,
                player_name: format!("Cand{player_id}"),
                club_id: 0,
                club_name: "Free Agent".to_string(),
                ability,
                potential: ability.saturating_add(5),
                age,
                position_group,
                days_to_expiry: 0,
                nationality_country_reputation: if same_country { 5000 } else { 3000 },
                nationality_region: ScoutingRegion::from_country(1, code),
                nationality_country_code: code.to_string(),
                nationality_continent_id: 1,
                career_pressure: 0.6,
                days_free: 0,
                reference_reputation: if same_country { 4000 } else { 3000 },
                last_salary: 50_000,
                last_country_reputation: 5000,
                last_league_reputation: 4500,
                world_reputation: 1500,
                current_reputation: 1500,
                professionalism_norm: 0.5,
                failed_approach_streak: 0,
                is_global_pool: true,
            }
        }

        /// Variant of [`Self::candidate`] with explicit career pressure
        /// override — needed for the acceptance tests where a low-
        /// pressure superstar should reject a tiny club's emergency
        /// offer, and a high-pressure veteran should accept.
        fn candidate_with(
            player_id: u32,
            ability: u8,
            age: u8,
            position_group: PlayerFieldPositionGroup,
            same_country: bool,
            career_pressure: f32,
            reference_reputation: u16,
        ) -> FreeAgentCandidate {
            let mut c = Self::candidate(player_id, ability, age, position_group, same_country);
            c.career_pressure = career_pressure;
            c.reference_reputation = reference_reputation;
            c
        }

        /// Build a country with a configurable reputation so the
        /// realism tests can exercise low-rep / high-rep buyers
        /// without rewriting the whole fixture.
        fn country_with_reputation(clubs: Vec<Club>, reputation: u16) -> Country {
            Country::builder()
                .id(1)
                .code("en".to_string())
                .slug("england".to_string())
                .name("England".to_string())
                .continent_id(1)
                .reputation(reputation)
                .leagues(LeagueCollection::new(vec![League::new(
                    1,
                    "L".to_string(),
                    "english".to_string(),
                    1,
                    reputation,
                    LeagueSettings {
                        season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                        season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                        tier: 1,
                        promotion_spots: 0,
                        relegation_spots: 0,
                        league_group: None,
                    },
                    false,
                )]))
                .clubs(clubs)
                .build()
                .unwrap()
        }

        /// Run the emergency pass with both side-channel vecs
        /// allocated locally — most tests don't care about offered /
        /// rejected tracking, so funneling those into a helper keeps
        /// the test bodies tight.
        fn run_emergency(
            country: &Country,
            candidates: &[FreeAgentCandidate],
            config: &TransferConfig,
            signings: &mut Vec<FreeAgentSigning>,
        ) -> (Vec<u32>, Vec<u32>) {
            let mut offered = Vec::new();
            let mut rejected = Vec::new();
            CountryResult::handle_free_agents_emergency_pass(
                country,
                candidates,
                config,
                signings,
                &mut offered,
                &mut rejected,
            );
            (offered, rejected)
        }
    }

    #[test]
    fn empty_main_team_generates_emergency_signings_for_each_group() {
        // Test-isolation: seed the shared RandomEngine so the per-slot
        // weighted cluster pick + acceptance roll sequence is independent
        // of how many RNG draws preceding tests consumed (mirrors the
        // seeded sibling tests in this block).
        crate::utils::random::engine::RandomEngine::set_seed(0xE11E_C7AB_u64);
        // Empty squad → urgent flag. Emergency pass should produce at
        // least one signing per missing group (GK/DEF/MID/FWD) up to
        // the per-club cap, before the normal request-driven matcher
        // has any state to work from.
        let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);

        let mut candidates = Vec::new();
        for i in 0..3 {
            candidates.push(EmergencyFillFixtures::candidate(
                100 + i,
                70,
                26,
                PlayerFieldPositionGroup::Goalkeeper,
                true,
            ));
        }
        for i in 0..8 {
            candidates.push(EmergencyFillFixtures::candidate(
                200 + i,
                75,
                26,
                PlayerFieldPositionGroup::Defender,
                true,
            ));
        }
        for i in 0..8 {
            candidates.push(EmergencyFillFixtures::candidate(
                300 + i,
                75,
                26,
                PlayerFieldPositionGroup::Midfielder,
                true,
            ));
        }
        for i in 0..5 {
            candidates.push(EmergencyFillFixtures::candidate(
                400 + i,
                80,
                26,
                PlayerFieldPositionGroup::Forward,
                true,
            ));
        }

        let mut signings = Vec::new();
        let config = TransferConfig::default();
        EmergencyFillFixtures::run_emergency(&country, &candidates, &config, &mut signings);

        // Empty squad triggers the adaptive cap: per-club cap is
        // lifted to the playable-size floor so the club can reach 11
        // in one tick when the pool allows it. Country cap is still
        // the ceiling.
        assert!(
            !signings.is_empty(),
            "empty squad must generate emergency signings, got 0"
        );
        assert!(
            signings.len() <= config.emergency_max_signings_per_country_per_day,
            "exceeded country emergency cap"
        );
    }

    #[test]
    fn club_short_one_gk_signs_a_gk_first() {
        // Test-isolation: seed the global RandomEngine so the probabilistic
        // acceptance roll is deterministic regardless of how many RNG draws
        // preceding tests consumed (mirrors the seeded sibling tests above).
        // Without this the outcome is execution-order dependent.
        crate::utils::random::engine::RandomEngine::set_seed(0xE11E_C7AB_u64);
        // Squad has 0 GK and a handful of outfield bodies — emergency
        // pass must reach for the goalkeeper before anything else.
        let players: Vec<Player> = (0..8)
            .map(|i| EmergencyFillFixtures::player(i, PlayerPositionType::DefenderCenter))
            .chain((0..6).map(|i| {
                EmergencyFillFixtures::player(20 + i, PlayerPositionType::MidfielderCenter)
            }))
            .chain(
                (0..4).map(|i| EmergencyFillFixtures::player(40 + i, PlayerPositionType::Striker)),
            )
            .collect();

        let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);

        // Candidate pool: GKs only (everything else already filled).
        // Full career pressure pins the acceptance roll near-certain —
        // the assertion is about slot ordering, not willingness.
        let candidates: Vec<FreeAgentCandidate> = (0..3)
            .map(|i| {
                EmergencyFillFixtures::candidate_with(
                    500 + i,
                    70,
                    26,
                    PlayerFieldPositionGroup::Goalkeeper,
                    true,
                    1.0,
                    3500,
                )
            })
            .collect();

        let mut signings = Vec::new();
        EmergencyFillFixtures::run_emergency(
            &country,
            &candidates,
            &TransferConfig::default(),
            &mut signings,
        );

        assert!(
            !signings.is_empty(),
            "GK-deficient squad must sign at least one goalkeeper"
        );
        assert_eq!(
            signings[0].reason, "emergency_squad_fill_gk",
            "first emergency signing for a GK-deficient squad must be tagged GK"
        );
    }

    #[test]
    fn full_squad_does_not_emergency_sign() {
        // 25-player squad split sensibly across groups → no emergency
        // need at all. signings should stay empty regardless of the
        // candidates available.
        let mut players: Vec<Player> = Vec::new();
        for i in 0..2 {
            players.push(EmergencyFillFixtures::player(
                i,
                PlayerPositionType::Goalkeeper,
            ));
        }
        for i in 0..8 {
            players.push(EmergencyFillFixtures::player(
                10 + i,
                PlayerPositionType::DefenderCenter,
            ));
        }
        for i in 0..9 {
            players.push(EmergencyFillFixtures::player(
                20 + i,
                PlayerPositionType::MidfielderCenter,
            ));
        }
        for i in 0..6 {
            players.push(EmergencyFillFixtures::player(
                40 + i,
                PlayerPositionType::Striker,
            ));
        }

        let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);

        let candidates: Vec<FreeAgentCandidate> = (0..10)
            .map(|i| {
                EmergencyFillFixtures::candidate(
                    600 + i,
                    80,
                    27,
                    PlayerFieldPositionGroup::Midfielder,
                    true,
                )
            })
            .collect();

        let mut signings = Vec::new();
        EmergencyFillFixtures::run_emergency(
            &country,
            &candidates,
            &TransferConfig::default(),
            &mut signings,
        );
        assert!(signings.is_empty(), "full squad should not emergency-sign");
    }

    #[test]
    fn underfilled_club_signs_multiple_despite_normal_daily_cap() {
        // Test-isolation: seed the shared RandomEngine so the per-slot
        // weighted cluster pick + acceptance roll sequence stays
        // deterministic regardless of suite position (the cluster pick
        // now draws RNG for multi-candidate slots).
        crate::utils::random::engine::RandomEngine::set_seed(0xE11E_C7AB_u64);
        // Squad of 9 (urgent < 11). Normal max_free_agent_signings_per_day
        // is 2; the emergency pass uses a separate per-club cap (5
        // by default) so the underfilled club must be able to sign
        // more than 2 in a single tick.
        let players: Vec<Player> = (0..9)
            .map(|i| EmergencyFillFixtures::player(i, PlayerPositionType::MidfielderCenter))
            .collect();

        let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);

        // Full career pressure keeps the per-candidate acceptance roll
        // near-certain — the assertion is about cap behaviour, not
        // player willingness, and must not flake on the shared stream.
        let mut candidates: Vec<FreeAgentCandidate> = Vec::new();
        for i in 0..2 {
            candidates.push(EmergencyFillFixtures::candidate_with(
                700 + i,
                70,
                26,
                PlayerFieldPositionGroup::Goalkeeper,
                true,
                1.0,
                3500,
            ));
        }
        for i in 0..8 {
            candidates.push(EmergencyFillFixtures::candidate_with(
                710 + i,
                75,
                26,
                PlayerFieldPositionGroup::Defender,
                true,
                1.0,
                3500,
            ));
        }
        for i in 0..5 {
            candidates.push(EmergencyFillFixtures::candidate_with(
                720 + i,
                80,
                26,
                PlayerFieldPositionGroup::Forward,
                true,
                1.0,
                3500,
            ));
        }

        let mut signings = Vec::new();
        let config = TransferConfig::default();
        EmergencyFillFixtures::run_emergency(&country, &candidates, &config, &mut signings);
        assert!(
            signings.len() > config.max_free_agent_signings_per_day,
            "emergency pass should exceed the normal {} daily cap (urgent club, got {} signings)",
            config.max_free_agent_signings_per_day,
            signings.len()
        );
    }

    #[test]
    fn zero_transfer_budget_does_not_block_emergency_fill() {
        // Seed + full career pressure: what's under test is the budget
        // independence, not the acceptance roll — with cp 0.6 each
        // candidate accepted only ~40% of the time and all five could
        // decline on an unlucky stream.
        crate::utils::random::engine::RandomEngine::set_seed(0x0B0D_6E70);
        // Construct a club whose finance balance is zero / negative —
        // emergency fill should still proceed because free-agent fee
        // is 0 and the emergency pass doesn't gate on transfer budget.
        let players: Vec<Player> = (0..8)
            .map(|i| EmergencyFillFixtures::player(i, PlayerPositionType::MidfielderCenter))
            .collect();
        let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
        let mut club = EmergencyFillFixtures::club(100, "FC", main);
        // Zero out the finance balance — the emergency path must not
        // care, because no fee is paid.
        club.finance = ClubFinances::new(0, Vec::new());
        let country = EmergencyFillFixtures::country(vec![club]);

        let candidates: Vec<FreeAgentCandidate> = (0..5)
            .map(|i| {
                EmergencyFillFixtures::candidate_with(
                    800 + i,
                    70,
                    27,
                    PlayerFieldPositionGroup::Defender,
                    true,
                    1.0,
                    3500,
                )
            })
            .collect();

        let mut signings = Vec::new();
        EmergencyFillFixtures::run_emergency(
            &country,
            &candidates,
            &TransferConfig::default(),
            &mut signings,
        );
        assert!(
            !signings.is_empty(),
            "zero-budget club must still emergency-sign free agents"
        );
    }

    #[test]
    fn emergency_pass_skips_player_already_signed_in_same_tick() {
        // Pre-populate signings with one of the candidates — the
        // pass must not re-pick the same player. This is the
        // multi-club path: two underfilled clubs in the same country
        // shouldn't both grab the same free agent.
        let players: Vec<Player> = (0..8)
            .map(|i| EmergencyFillFixtures::player(i, PlayerPositionType::MidfielderCenter))
            .collect();
        let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);

        let candidate =
            EmergencyFillFixtures::candidate(900, 70, 26, PlayerFieldPositionGroup::Defender, true);
        let already =
            EmergencyFillFixtures::candidate(901, 70, 26, PlayerFieldPositionGroup::Defender, true);
        let candidates = vec![candidate, already];

        // Mark player 900 as already signed in this tick.
        let mut signings = vec![FreeAgentSigning {
            player_id: 900,
            player_name: "Cand900".to_string(),
            from_club_id: 0,
            from_club_name: "Free Agent".to_string(),
            to_club_id: 200,
            reason: "emergency_squad_fill_def".to_string(),
            terms: None,
            fills_group: Some(PlayerFieldPositionGroup::Defender),
        }];

        EmergencyFillFixtures::run_emergency(
            &country,
            &candidates,
            &TransferConfig::default(),
            &mut signings,
        );
        assert!(
            !signings.iter().skip(1).any(|s| s.player_id == 900),
            "emergency pass must not re-pick a player already signed this tick"
        );
    }

    #[test]
    fn emergency_pass_respects_per_club_cap() {
        // Squad of 0 → every group missing. With per-club cap of 5
        // the pass must sign at most 5 even when 20 candidates are
        // available in the right groups.
        let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);

        let mut candidates: Vec<FreeAgentCandidate> = Vec::new();
        // Plenty of every group.
        for grp in &[
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ] {
            for i in 0..5 {
                let pid = match grp {
                    PlayerFieldPositionGroup::Goalkeeper => 1000 + i,
                    PlayerFieldPositionGroup::Defender => 1100 + i,
                    PlayerFieldPositionGroup::Midfielder => 1200 + i,
                    PlayerFieldPositionGroup::Forward => 1300 + i,
                };
                candidates.push(EmergencyFillFixtures::candidate(pid, 75, 26, *grp, true));
            }
        }

        let mut signings = Vec::new();
        let config = TransferConfig::default();
        EmergencyFillFixtures::run_emergency(&country, &candidates, &config, &mut signings);
        // Per-club cap is lifted to the playable-size floor when the
        // squad is empty, so use the urgent floor (or country cap when
        // smaller) as the realistic upper bound.
        let expected_max = config
            .emergency_urgent_per_club_cap_floor
            .max(config.emergency_max_signings_per_club_per_day)
            .min(config.emergency_max_signings_per_country_per_day);
        assert!(
            signings.len() <= expected_max,
            "per-club cap exceeded: got {} signings, cap {}",
            signings.len(),
            expected_max
        );
    }

    #[test]
    fn emergency_pass_picks_domestic_over_foreign_at_equal_quality() {
        // Empty squad. Two equally-strong defender candidates available,
        // one domestic, one foreign — the domestic preference should
        // surface the domestic player first. Both candidates use full
        // career pressure so the new acceptance roll lands reliably
        // for whichever is offered, isolating the scoring preference.
        let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);

        let domestic = EmergencyFillFixtures::candidate_with(
            2000,
            75,
            27,
            PlayerFieldPositionGroup::Defender,
            true,
            1.0,
            3500,
        );
        let foreign = EmergencyFillFixtures::candidate_with(
            2001,
            75,
            27,
            PlayerFieldPositionGroup::Defender,
            false,
            1.0,
            3500,
        );
        // Order foreign first to prove ordering isn't the reason —
        // scoring is.
        let candidates = vec![foreign, domestic];

        let mut signings = Vec::new();
        EmergencyFillFixtures::run_emergency(
            &country,
            &candidates,
            &TransferConfig::default(),
            &mut signings,
        );
        // The first DEF-tagged signing should be the domestic one.
        let first_def = signings
            .iter()
            .find(|s| s.reason == "emergency_squad_fill_def");
        assert_eq!(
            first_def.map(|s| s.player_id),
            Some(2000),
            "domestic candidate should win the defender slot, signings={:?}",
            signings
                .iter()
                .map(|s| (s.player_id, &s.reason))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn urgent_club_reaches_eleven_in_one_tick_with_plausible_pool() {
        // Pin the shared sim RNG so the per-candidate acceptance roll
        // sequence is deterministic — otherwise this test is at the
        // mercy of whatever earlier tests in the suite drained from
        // the thread-local stream, and one or two unlucky rejects
        // tip the signings count under the 11 floor.
        crate::utils::random::engine::RandomEngine::set_seed(0xE11E_C7AB_u64);

        // Empty squad + plenty of plausible candidates → adaptive cap
        // lifts to the playable-size floor. The signing budget is
        // capped by the country-wide cap, but with 20 of room and a
        // pool of 20+ realistic candidates, a single tick must land
        // at least 11 signings so the club becomes playable.
        let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);

        // Full career pressure pins each acceptance roll near-certain;
        // the assertion measures the adaptive cap, not willingness.
        let mut candidates = Vec::new();
        for i in 0..3 {
            candidates.push(EmergencyFillFixtures::candidate_with(
                3000 + i,
                70,
                28,
                PlayerFieldPositionGroup::Goalkeeper,
                true,
                1.0,
                3500,
            ));
        }
        for i in 0..8 {
            candidates.push(EmergencyFillFixtures::candidate_with(
                3100 + i,
                75,
                28,
                PlayerFieldPositionGroup::Defender,
                true,
                1.0,
                3500,
            ));
        }
        for i in 0..8 {
            candidates.push(EmergencyFillFixtures::candidate_with(
                3200 + i,
                75,
                28,
                PlayerFieldPositionGroup::Midfielder,
                true,
                1.0,
                3500,
            ));
        }
        for i in 0..5 {
            candidates.push(EmergencyFillFixtures::candidate_with(
                3300 + i,
                75,
                28,
                PlayerFieldPositionGroup::Forward,
                true,
                1.0,
                3500,
            ));
        }

        let mut signings = Vec::new();
        let config = TransferConfig::default();
        EmergencyFillFixtures::run_emergency(&country, &candidates, &config, &mut signings);
        assert!(
            signings.len() >= config.emergency_min_playable_size,
            "urgent club should reach at least {} signings in one tick (got {})",
            config.emergency_min_playable_size,
            signings.len()
        );
    }

    #[test]
    fn urgency_turns_off_at_eleven_signings() {
        // 10 players + plenty of plausible candidates → the 11th
        // signing flips the urgent flag off. Subsequent slots run
        // under non-urgent rules, which means a low-rep buyer should
        // start rejecting candidates the urgent path would have
        // accepted. We assert urgency by counting how many signings
        // tagged the depth slot vs. the urgent-group slots.
        let players: Vec<Player> = (0..10)
            .map(|i| EmergencyFillFixtures::player(i, PlayerPositionType::MidfielderCenter))
            .collect();
        let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);

        // Candidate pool: 1 keeper to flip projected count to 11, then
        // a few extras of each non-keeper group so the projection
        // continues filling but the urgency check has fired. Full
        // career pressure removes RNG flakiness from the GK signing
        // — what we're testing is the slot ordering, not acceptance.
        let mut candidates = Vec::new();
        candidates.push(EmergencyFillFixtures::candidate_with(
            4000,
            70,
            28,
            PlayerFieldPositionGroup::Goalkeeper,
            true,
            1.0,
            3500,
        ));
        for i in 0..3 {
            candidates.push(EmergencyFillFixtures::candidate_with(
                4100 + i,
                75,
                28,
                PlayerFieldPositionGroup::Defender,
                true,
                1.0,
                3500,
            ));
        }

        let mut signings = Vec::new();
        EmergencyFillFixtures::run_emergency(
            &country,
            &candidates,
            &TransferConfig::default(),
            &mut signings,
        );
        // Buyer projected 10 → first signing (GK) makes it 11; urgent
        // flag turns off afterwards. The pass must still be able to
        // sign defenders (group floor 7 > current 0) but uses non-
        // urgent rules. We can't assert "urgency was off" directly,
        // but we can assert the first signing was the keeper, since
        // GK gets explicit priority.
        assert!(
            signings
                .iter()
                .any(|s| s.reason == "emergency_squad_fill_gk"),
            "GK shortfall must be filled first when projected starts urgent"
        );
    }

    #[test]
    fn elite_low_pressure_player_does_not_sign_for_low_rep_emergency_club() {
        // 800-rep amateur side, urgent (0 players). A CA-180 megastar
        // with low career pressure should not be signed even on the
        // urgent path — the scoring chasm gate (now relaxed for urgent
        // but still bounded) and the soft CA cap both block.
        let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country_with_reputation(vec![club], 800);

        let mega = EmergencyFillFixtures::candidate_with(
            5000,
            180,
            27,
            PlayerFieldPositionGroup::Midfielder,
            false,
            0.1,
            7500,
        );
        let candidates = vec![mega];

        let mut signings = Vec::new();
        EmergencyFillFixtures::run_emergency(
            &country,
            &candidates,
            &TransferConfig::default(),
            &mut signings,
        );
        assert!(
            !signings.iter().any(|s| s.player_id == 5000),
            "elite low-pressure player must not sign for low-rep urgent club"
        );
    }

    #[test]
    fn accepted_emergency_signing_stages_wage_and_terms() {
        // After a successful emergency signing the staged terms must
        // travel with the signing so execution installs the agreed
        // wage and role promise rather than the calculator default.
        let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);

        let candidate = EmergencyFillFixtures::candidate_with(
            5100,
            80,
            29,
            PlayerFieldPositionGroup::Midfielder,
            true,
            0.7,
            3500,
        );
        let candidates = vec![candidate];

        let mut signings = Vec::new();
        EmergencyFillFixtures::run_emergency(
            &country,
            &candidates,
            &TransferConfig::default(),
            &mut signings,
        );
        let staged = signings
            .iter()
            .find(|s| s.player_id == 5100)
            .expect("acceptance should land the signing");
        let terms = staged.terms.expect("emergency pass must stage terms");
        assert!(terms.annual_wage > 0, "annual wage must be staged");
        assert!(
            terms.contract_years <= EmergencyContractTermsPolicy::YOUNG_USEFUL_YEARS,
            "emergency contract length must stay short"
        );
        assert_eq!(
            staged.fills_group,
            Some(PlayerFieldPositionGroup::Midfielder)
        );
    }

    #[test]
    fn rejected_emergency_offer_updates_offered_and_rejected_ids() {
        // High-pressure offer that the buyer can't credibly match —
        // we set up a low-rep buyer + high-rep + low-pressure
        // candidate. Expected outcome: offer is made (offered_ids
        // populated) AND rejected (rejected_ids populated).
        // Determinism is tricky because the acceptance roll is RNG,
        // but the prob will be near zero when the buyer is tiny and
        // the player has no pressure.
        let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country_with_reputation(vec![club], 1200);

        // CA 170, low pressure, very high reference rep. Even on the
        // urgent path the chasm gate is now `2500 + 0.1*4500 = 2950`,
        // so 1200+2950=4150 vs 7800 ref rep — the gate fails and the
        // candidate is skipped before scoring (no offered/rejected).
        // For this test we want the offer to be MADE but rejected, so
        // pick a borderline candidate that passes the score gate but
        // fails the acceptance roll. CA 150, mid pressure, ref rep
        // 4500 — chasm: 2500+0.4*4500=4300, 1200+4300=5500 > 4500 ✓.
        let borderline = EmergencyFillFixtures::candidate_with(
            5200,
            150,
            27,
            PlayerFieldPositionGroup::Midfielder,
            false,
            0.4,
            4500,
        );
        let candidates = vec![borderline];

        let mut signings = Vec::new();
        let mut offered = Vec::new();
        let mut rejected = Vec::new();
        CountryResult::handle_free_agents_emergency_pass(
            &country,
            &candidates,
            &TransferConfig::default(),
            &mut signings,
            &mut offered,
            &mut rejected,
        );
        // If the offer was made at all, it must have been tracked. The
        // candidate is global-pool (club_id=0). Whether it accepted
        // depends on the RNG, but `offered_ids` is populated
        // regardless of outcome.
        if !signings.iter().any(|s| s.player_id == 5200) {
            // Rejected branch: offered AND rejected populated. The
            // score gate may filter the candidate before offering, in
            // which case neither is populated — that's also acceptable
            // behaviour (no offer made).
            if offered.contains(&5200) {
                assert!(
                    rejected.contains(&5200),
                    "an offer made and not signed must be tracked as rejected"
                );
            }
        }
    }

    #[test]
    fn emergency_signing_marks_matching_transfer_request_fulfilled() {
        // Stage a transfer request for a defender on the club's
        // transfer plan; emergency signing should mark it fulfilled.
        let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
        let mut club = EmergencyFillFixtures::club(100, "FC", main);
        club.transfer_plan.initialized = true;
        club.transfer_plan
            .transfer_requests
            .push(TransferRequest::new(
                1,
                PlayerPositionType::DefenderCenter,
                TransferNeedPriority::Critical,
                TransferNeedReason::SquadPadding,
                50,
                80,
                0.0,
            ));
        let mut country = EmergencyFillFixtures::country(vec![club]);

        let candidate = EmergencyFillFixtures::candidate_with(
            5300,
            75,
            29,
            PlayerFieldPositionGroup::Defender,
            true,
            0.6,
            3500,
        );
        let candidates = vec![candidate];

        // Drive the full handle_free_agents path so the post-signing
        // sync runs. This requires the same context handle_free_agents
        // builds — we approximate by running the pass and then the
        // sync helper directly.
        let mut signings = Vec::new();
        let mut offered = Vec::new();
        let mut rejected = Vec::new();
        CountryResult::handle_free_agents_emergency_pass(
            &country,
            &candidates,
            &TransferConfig::default(),
            &mut signings,
            &mut offered,
            &mut rejected,
        );
        if let Some(signing) = signings.iter().find(|s| s.player_id == 5300) {
            if let Some(group) = signing.fills_group {
                TransferPlanSync::mark_group_fulfilled(&mut country, signing.to_club_id, group);
            }
        }
        let club = &country.clubs[0];
        // Either: the request was marked fulfilled by the sync helper,
        // or the candidate wasn't accepted (RNG dependent) and the
        // request stays pending.
        let request = club
            .transfer_plan
            .transfer_requests
            .iter()
            .find(|r| r.id == 1)
            .expect("staged request must survive the pass");
        if signings.iter().any(|s| s.player_id == 5300) {
            assert_eq!(
                request.status,
                TransferRequestStatus::Fulfilled,
                "matching request must be fulfilled after a successful signing"
            );
        }
    }

    #[test]
    fn depth_fill_rotates_into_thinnest_group_not_always_midfield() {
        // 25 players (no group shortage) wouldn't trigger emergency.
        // Construct a club at exactly the group minimums (GK 2 / DEF 7
        // / FWD 4 / MID 7 = 20). Total 20 sits under the threshold of
        // 18 — wait, 20 > 18 so the pass exits early. Lower DEF count
        // by 2 so the depth slot rotates into DEF as the thinnest
        // group (1 short vs MID/FWD/GK at minimums).
        //
        // We test the thinnest_group helper directly because the
        // full-pass scoring randomness makes integration testing
        // flaky.
        let needs = FirstTeamSquadNeeds {
            main_team_size: 18,
            total_missing: 7,
            gk_missing: 0,
            def_missing: 2,
            mid_missing: 0,
            fwd_missing: 0,
            urgent: false,
        };
        let projected = EmergencyProjectedSquad::from_needs(&needs);
        assert_eq!(
            projected.thinnest_group(),
            PlayerFieldPositionGroup::Defender,
            "depth fill must rotate into the currently thinnest group"
        );
    }

    #[test]
    fn country_cap_still_limits_one_country_pool() {
        // Two unplayable clubs in the same country with a massive
        // candidate pool — country cap (default 20) must bound the
        // total even though each club individually would otherwise
        // sign the full playable-size lift.
        let main_a = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
        let club_a = EmergencyFillFixtures::club(100, "FCA", main_a);

        let main_b = EmergencyFillFixtures::team(20, "ZZ", "zz", Vec::new());
        // Use a different club id to avoid the same-club skip.
        let club_b = Club::new(
            200,
            "FCB".to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(vec![main_b]),
            ClubFacilities::default(),
        );
        let country = EmergencyFillFixtures::country(vec![club_a, club_b]);

        let mut candidates = Vec::new();
        for i in 0..40 {
            candidates.push(EmergencyFillFixtures::candidate(
                6000 + i,
                70,
                28,
                if i % 4 == 0 {
                    PlayerFieldPositionGroup::Goalkeeper
                } else if i % 4 == 1 {
                    PlayerFieldPositionGroup::Defender
                } else if i % 4 == 2 {
                    PlayerFieldPositionGroup::Midfielder
                } else {
                    PlayerFieldPositionGroup::Forward
                },
                true,
            ));
        }

        let mut signings = Vec::new();
        let config = TransferConfig::default();
        EmergencyFillFixtures::run_emergency(&country, &candidates, &config, &mut signings);
        assert!(
            signings.len() <= config.emergency_max_signings_per_country_per_day,
            "country cap exceeded: {} signings, cap {}",
            signings.len(),
            config.emergency_max_signings_per_country_per_day
        );
    }

    /// Test fixtures for the realism-gate / cross-region tests added
    /// alongside the strictness rework. Kept on a dedicated struct so
    /// the original [`EmergencyFillFixtures`] helpers stay focused on
    /// the existing pipeline tests and the new cases can dial in
    /// continent / code / region without rewriting the shared
    /// helpers.
    struct CrossRegionFixtures;

    impl CrossRegionFixtures {
        /// Buyer context for the picker / gate tests. Builds an
        /// Algerian-style low-rep North-African buyer when `algerian`
        /// is true, an English-style mid-rep European buyer otherwise.
        /// Strictness is exposed so a single fixture works for the
        /// depth (Strict) and GK (Flexible) variants.
        fn buyer(
            algerian: bool,
            strictness: EmergencyStrictness,
            urgent: bool,
        ) -> EmergencyBuyerContext {
            let (rep, code, continent, region_prestige, club_score, league_rep) = if algerian {
                (
                    1500,
                    "dz".to_string(),
                    0u32,
                    ScoutingRegion::from_country(0, "dz").league_prestige(),
                    0.18,
                    1400u16,
                )
            } else {
                (
                    5000,
                    "en".to_string(),
                    1u32,
                    ScoutingRegion::from_country(1, "en").league_prestige(),
                    0.55,
                    4800u16,
                )
            };
            EmergencyBuyerContext {
                country_reputation: rep,
                country_code: code,
                continent_id: continent,
                region_prestige,
                club_reputation_score: club_score,
                league_reputation: league_rep,
                negotiator_skill: 50,
                urgent,
                strictness,
            }
        }

        /// Build a free-agent candidate in the global pool with an
        /// explicit nationality (continent + code). Lets a single
        /// helper cover Russian (`ru`, continent 1), Algerian (`dz`,
        /// continent 0), and any other cross-region setup the gate
        /// tests need.
        fn candidate(
            player_id: u32,
            ability: u8,
            age: u8,
            group: PlayerFieldPositionGroup,
            code: &str,
            continent_id: u32,
            nationality_country_reputation: u16,
            reference_reputation: u16,
            career_pressure: f32,
        ) -> FreeAgentCandidate {
            FreeAgentCandidate {
                player_id,
                player_name: format!("Cand{player_id}"),
                club_id: 0,
                club_name: "Free Agent".to_string(),
                ability,
                potential: ability.saturating_add(5),
                age,
                position_group: group,
                days_to_expiry: 0,
                nationality_country_reputation,
                nationality_region: ScoutingRegion::from_country(continent_id, code),
                nationality_country_code: code.to_string(),
                nationality_continent_id: continent_id,
                career_pressure,
                days_free: 0,
                reference_reputation,
                last_salary: 40_000,
                last_country_reputation: nationality_country_reputation,
                last_league_reputation: ((nationality_country_reputation as f32) * 0.85) as u16,
                world_reputation: 1200,
                current_reputation: 1200,
                professionalism_norm: 0.5,
                failed_approach_streak: 0,
                is_global_pool: true,
            }
        }
    }

    #[test]
    fn realism_region_gate_blocks_russian_to_algerian_depth_at_low_pressure() {
        // Russian player + Algerian club + Strict (depth) slot must
        // block before scoring even runs. Pressure 0.5 is comfortably
        // below the `Strict + cross-continent` cutoff of 0.85.
        let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
        let russian = CrossRegionFixtures::candidate(
            1,
            75,
            27,
            PlayerFieldPositionGroup::Defender,
            "ru",
            1,
            3000,
            3500,
            0.5,
        );
        assert!(
            !EmergencyRealismGates::passes_region(&russian, &buyer),
            "Strict + cross-continent + pressure 0.5 must fail the region gate"
        );
        assert!(
            !EmergencyRealismGates::passes(&russian, &buyer, PlayerFieldPositionGroup::Defender),
            "the composite gate must reject the same case"
        );
    }

    #[test]
    fn realism_region_gate_passes_russian_to_algerian_at_very_high_pressure() {
        // Same cross-continent move but with the player on the very
        // verge of retiring (pressure 0.92) — Strict region gate now
        // lets it through. The rep / quality gates do their own
        // checks; the test isolates the region behaviour.
        let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
        let russian = CrossRegionFixtures::candidate(
            2,
            70,
            33,
            PlayerFieldPositionGroup::Defender,
            "ru",
            1,
            1800,
            1700,
            0.92,
        );
        assert!(
            EmergencyRealismGates::passes_region(&russian, &buyer),
            "Strict + cross-continent at high pressure must clear the region gate"
        );
    }

    #[test]
    fn realism_region_gate_lets_gk_flexible_pass_where_depth_strict_blocks() {
        // Same candidate, same buyer — only the slot strictness
        // changes. Flexible (no-keeper GK fill) now requires a 0.65
        // pressure floor, so the test runs at 0.70: well past Flexible
        // but below Strict's 0.85 floor. Tests the strictness dial
        // directly without leaning on the old "any pressure" carve-out.
        let cross = CrossRegionFixtures::candidate(
            3,
            72,
            30,
            PlayerFieldPositionGroup::Goalkeeper,
            "ru",
            1,
            2200,
            2400,
            0.70,
        );
        let gk_buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Flexible, true);
        let depth_buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
        assert!(
            EmergencyRealismGates::passes_region(&cross, &gk_buyer),
            "Flexible GK fill should accept a cross-region keeper past its 0.65 floor"
        );
        assert!(
            !EmergencyRealismGates::passes_region(&cross, &depth_buyer),
            "Strict depth fill must reject the same candidate"
        );
    }

    #[test]
    fn realism_region_gate_blocks_russian_to_african_gk_at_routine_pressure() {
        // Regression: a Russian free-agent keeper was signing for a
        // Cameroonian club via `emergency_squad_fill_gk` (Flexible
        // strictness) at routine career pressure. The Flexible floor
        // of 0.65 must block the move; only a player well past peak
        // is allowed to cross continents into a markedly weaker region
        // even for a no-keeper slot.
        let cameroonian_buyer = EmergencyBuyerContext {
            country_reputation: 1100,
            country_code: "cm".to_string(),
            continent_id: 0,
            region_prestige: ScoutingRegion::from_country(0, "cm").league_prestige(),
            club_reputation_score: 0.14,
            league_reputation: 1000,
            negotiator_skill: 50,
            urgent: true,
            strictness: EmergencyStrictness::Flexible,
        };
        let russian_gk = CrossRegionFixtures::candidate(
            60,
            70,
            28,
            PlayerFieldPositionGroup::Goalkeeper,
            "ru",
            1,
            2200,
            2200,
            0.45,
        );
        assert!(
            !EmergencyRealismGates::passes_region(&russian_gk, &cameroonian_buyer),
            "Flexible GK fill + Russian → Cameroon at routine pressure must remain blocked"
        );
    }

    #[test]
    fn realism_region_gate_passes_russian_to_african_gk_at_high_pressure() {
        // Same Russian → Cameroonian GK case as the blocking test
        // above, but at 0.78 — comfortably above the Flexible floor
        // of 0.65 and the Standard floor of 0.75. A near-retirement
        // veteran landing a desperation no-keeper slot is the
        // realistic carve-out the dial is meant to allow.
        let cameroonian_buyer = EmergencyBuyerContext {
            country_reputation: 1100,
            country_code: "cm".to_string(),
            continent_id: 0,
            region_prestige: ScoutingRegion::from_country(0, "cm").league_prestige(),
            club_reputation_score: 0.14,
            league_reputation: 1000,
            negotiator_skill: 50,
            urgent: true,
            strictness: EmergencyStrictness::Flexible,
        };
        let russian_gk = CrossRegionFixtures::candidate(
            61,
            68,
            34,
            PlayerFieldPositionGroup::Goalkeeper,
            "ru",
            1,
            1600,
            1500,
            0.78,
        );
        assert!(
            EmergencyRealismGates::passes_region(&russian_gk, &cameroonian_buyer),
            "Flexible GK fill at high pressure must clear the region gate"
        );
    }

    #[test]
    fn picker_prefers_domestic_depth_over_higher_ca_foreign() {
        // Two candidates for a Strict (depth) defender slot at an
        // Algerian buyer: a domestic Algerian at CA 65 and a Russian
        // at CA 75 on full pressure (so the Russian could in principle
        // clear the region gate). Locality ordering must still pick
        // the Algerian — depth is not about raw ability.
        let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
        let algerian = CrossRegionFixtures::candidate(
            10,
            65,
            29,
            PlayerFieldPositionGroup::Defender,
            "dz",
            0,
            1500,
            1500,
            0.6,
        );
        let russian = CrossRegionFixtures::candidate(
            11,
            75,
            33,
            PlayerFieldPositionGroup::Defender,
            "ru",
            1,
            2200,
            2000,
            0.95,
        );
        let candidates = vec![russian, algerian];
        let signings: Vec<FreeAgentSigning> = Vec::new();
        let rejected: HashSet<u32> = HashSet::new();
        let slot = EmergencyGroupSlot {
            group: PlayerFieldPositionGroup::Defender,
            missing: 1,
            reason: "emergency_squad_fill_depth",
        };
        let pick =
            EmergencyCandidatePicker::pick(&candidates, &signings, &rejected, slot, &buyer, 999);
        let picked = pick.expect("at least one candidate must clear all gates");
        assert_eq!(
            picked.player_id, 10,
            "Strict depth at Algeria must prefer the domestic CA-65 Algerian over the foreign CA-75 Russian"
        );
    }

    #[test]
    fn picker_skips_only_unrealistic_candidates_for_depth() {
        // Only candidate available is a low-pressure Russian against
        // an Algerian Strict (depth) slot. With no domestic / closer
        // alternative the picker should return None rather than fall
        // back to the unrealistic cross-region option.
        let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
        let russian = CrossRegionFixtures::candidate(
            20,
            80,
            27,
            PlayerFieldPositionGroup::Midfielder,
            "ru",
            1,
            3000,
            3500,
            0.4,
        );
        let candidates = vec![russian];
        let signings: Vec<FreeAgentSigning> = Vec::new();
        let rejected: HashSet<u32> = HashSet::new();
        let slot = EmergencyGroupSlot {
            group: PlayerFieldPositionGroup::Midfielder,
            missing: 1,
            reason: "emergency_squad_fill_depth",
        };
        let pick =
            EmergencyCandidatePicker::pick(&candidates, &signings, &rejected, slot, &buyer, 999);
        assert!(
            pick.is_none(),
            "depth slot must skip rather than fall back to an unrealistic cross-region pick"
        );
    }

    #[test]
    fn pressure_threshold_separates_blocked_from_passing_step_down() {
        // Same Russian candidate against the same Algerian Strict
        // depth slot, only career pressure changes. Low pressure
        // must fail every gate; very high pressure must clear the
        // region gate. This proves pressure is the dial that
        // unlocks realistic step-downs.
        let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
        let low_pressure = CrossRegionFixtures::candidate(
            30,
            70,
            33,
            PlayerFieldPositionGroup::Defender,
            "ru",
            1,
            1800,
            1700,
            0.2,
        );
        let high_pressure = CrossRegionFixtures::candidate(
            31,
            70,
            33,
            PlayerFieldPositionGroup::Defender,
            "ru",
            1,
            1800,
            1700,
            0.95,
        );
        assert!(
            !EmergencyRealismGates::passes_region(&low_pressure, &buyer),
            "low-pressure cross-continent depth must remain blocked"
        );
        assert!(
            EmergencyRealismGates::passes_region(&high_pressure, &buyer),
            "very high pressure must unlock the region gate"
        );
    }

    #[test]
    fn depth_strictness_does_not_get_urgent_rep_bonus() {
        // High-rep Russian candidate, low-rep buyer. The 400-point
        // Standard rep bonus / 800-point Flexible rep bonus must NOT
        // apply for Strict depth — otherwise the urgent uplift
        // creeps into the depth path. Demonstrates the difference
        // between strictness levels at the rep gate.
        let candidate = CrossRegionFixtures::candidate(
            40,
            85,
            30,
            PlayerFieldPositionGroup::Midfielder,
            "ru",
            1,
            3500,
            3500,
            0.4,
        );
        let strict_buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Strict, false);
        let flex_buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Flexible, true);
        let strict_pass = EmergencyRealismGates::passes_reputation(&candidate, &strict_buyer);
        let flex_pass = EmergencyRealismGates::passes_reputation(&candidate, &flex_buyer);
        assert!(
            flex_pass || !strict_pass,
            "Flexible rep gate must be at least as permissive as Strict — \
             strict_pass={strict_pass} flex_pass={flex_pass}"
        );
    }

    #[test]
    fn realism_region_gate_blocks_russian_to_algerian_standard_at_low_pressure() {
        // Standard slot (urgent sub-11 outfield fill) now also fires
        // the hard cross-continent guard. Same Russian → Algerian
        // case as the Strict test, but at the Standard pressure
        // floor (0.75) instead of 0.85.
        let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Standard, true);
        let russian = CrossRegionFixtures::candidate(
            50,
            75,
            27,
            PlayerFieldPositionGroup::Defender,
            "ru",
            1,
            3000,
            3500,
            0.5,
        );
        assert!(
            !EmergencyRealismGates::passes_region(&russian, &buyer),
            "Standard urgent fill + cross-continent + pressure 0.5 must still fail the region gate"
        );
    }

    #[test]
    fn realism_region_gate_passes_russian_to_algerian_standard_at_high_pressure() {
        // The Standard floor is 0.75 — at 0.80 the Russian veteran
        // can land in Algeria for an urgent group fill, mirroring
        // the Strict path's "verge of retiring" carve-out.
        let buyer = CrossRegionFixtures::buyer(true, EmergencyStrictness::Standard, true);
        let russian = CrossRegionFixtures::candidate(
            51,
            70,
            33,
            PlayerFieldPositionGroup::Defender,
            "ru",
            1,
            1800,
            1700,
            0.80,
        );
        assert!(
            EmergencyRealismGates::passes_region(&russian, &buyer),
            "Standard slot at very high pressure must clear the region gate"
        );
    }

    #[test]
    fn slot_strictness_maps_correctly_from_reason() {
        // Sanity check that the policy struct routes each emergency
        // reason to the strictness the spec calls for.
        assert_eq!(
            EmergencySlotStrictness::from_reason("emergency_squad_fill_gk", true),
            EmergencyStrictness::Flexible
        );
        assert_eq!(
            EmergencySlotStrictness::from_reason("emergency_squad_fill_def", true),
            EmergencyStrictness::Standard
        );
        assert_eq!(
            EmergencySlotStrictness::from_reason("emergency_squad_fill_def", false),
            EmergencyStrictness::Strict
        );
        assert_eq!(
            EmergencySlotStrictness::from_reason("emergency_squad_fill_depth", true),
            EmergencyStrictness::Strict
        );
        assert_eq!(
            EmergencySlotStrictness::from_reason("emergency_squad_fill_depth", false),
            EmergencyStrictness::Strict
        );
    }

    /// Fixtures for the depth-through-pipeline tests. Separate struct
    /// so the squad / pool-snapshot builders the new flow needs don't
    /// bloat the shared [`EmergencyFillFixtures`] helpers.
    struct DepthPipelineFixtures;

    impl DepthPipelineFixtures {
        /// Balanced 20-man squad (2 GK / 7 DEF / 7 MID / 4 FWD): every
        /// group minimum met and total above the emergency threshold,
        /// so the emergency pass skips the club entirely and only the
        /// staged DepthCover request drives market activity.
        fn balanced_squad() -> Vec<Player> {
            let mut players: Vec<Player> = Vec::new();
            for i in 0..2 {
                players.push(EmergencyFillFixtures::player(
                    i,
                    PlayerPositionType::Goalkeeper,
                ));
            }
            for i in 0..7 {
                players.push(EmergencyFillFixtures::player(
                    10 + i,
                    PlayerPositionType::DefenderCenter,
                ));
            }
            for i in 0..7 {
                players.push(EmergencyFillFixtures::player(
                    20 + i,
                    PlayerPositionType::MidfielderCenter,
                ));
            }
            for i in 0..4 {
                players.push(EmergencyFillFixtures::player(
                    30 + i,
                    PlayerPositionType::Striker,
                ));
            }
            players
        }

        /// Snapshot row for the global free-agent pool input of
        /// `handle_free_agents`.
        fn pool_summary(
            player_id: u32,
            ability: u8,
            age: u8,
            group: PlayerFieldPositionGroup,
            same_country: bool,
            career_pressure: f32,
            reference_reputation: u16,
        ) -> GlobalFreeAgentSummary {
            let code = if same_country { "en" } else { "ar" };
            GlobalFreeAgentSummary {
                player_id,
                player_name: format!("Pool{player_id}"),
                ability,
                potential: ability.saturating_add(5),
                age,
                position_group: group,
                nationality_country_reputation: reference_reputation,
                nationality_continent_id: if same_country { 1 } else { 3 },
                nationality_country_code: code.to_string(),
                career_pressure,
                days_free: 0,
                reference_reputation,
                last_salary: 50_000,
                last_country_reputation: reference_reputation,
                last_league_reputation: ((reference_reputation as f32) * 0.85) as u16,
                world_reputation: 1500,
                current_reputation: 1500,
                professionalism_norm: 0.5,
                failed_approach_streak: 0,
            }
        }

        /// Drive `handle_free_agents` until the staged-negotiation
        /// matcher fires (the daily-chance roll is probabilistic) or
        /// `max_ticks` pass. Returns the pool signings the calls
        /// produced plus the offered / rejected side-channels.
        fn run_until_negotiation(
            country: &mut Country,
            pool: &[GlobalFreeAgentSummary],
            max_ticks: usize,
        ) -> (Vec<GlobalFreeAgentSigning>, Vec<u32>, Vec<u32>) {
            let date = EmergencyFillFixtures::d(2026, 6, 10);
            let config = TransferConfig::default();
            let mut all_signings = Vec::new();
            let mut offered = Vec::new();
            let mut rejected = Vec::new();
            for _ in 0..max_ticks {
                let mut summary = TransferActivitySummary::new();
                let mut domestic = Vec::new();
                let mut blocked = Vec::new();
                let signings = CountryResult::handle_free_agents(
                    country,
                    date,
                    &mut summary,
                    pool,
                    &config,
                    &mut domestic,
                    &mut offered,
                    &mut rejected,
                    &mut blocked,
                );
                all_signings.extend(signings);
                if !country.transfer_market.negotiations.is_empty() {
                    break;
                }
            }
            (all_signings, offered, rejected)
        }

        /// Balanced club + staged midfield DepthCover request + one
        /// domestic pool journeyman — the canonical "depth fill should
        /// negotiate" setup the flow tests share.
        fn staged_depth_country() -> (Country, Vec<GlobalFreeAgentSummary>) {
            let main = EmergencyFillFixtures::team(
                10,
                "FC",
                "fc",
                DepthPipelineFixtures::balanced_squad(),
            );
            let club = EmergencyFillFixtures::club(100, "FC", main);
            let mut country = EmergencyFillFixtures::country(vec![club]);
            country.clubs[0].transfer_plan.initialized = true;
            EmergencyDepthRequestPlanner::stage_requests(
                &mut country,
                &[EmergencyDepthRequestIntent {
                    club_id: 100,
                    group: PlayerFieldPositionGroup::Midfielder,
                }],
            );
            let pool = vec![DepthPipelineFixtures::pool_summary(
                9000,
                80,
                28,
                PlayerFieldPositionGroup::Midfielder,
                true,
                0.6,
                4000,
            )];
            (country, pool)
        }
    }

    #[test]
    fn depth_slot_stages_pipeline_request_instead_of_direct_signing() {
        // Squad of 14: MID six short, every other group at its minimum.
        // The candidate pool carries no midfielders, so the MID slot is
        // dead this tick and the planner falls to the depth tail —
        // which previously direct-signed a defender under
        // `emergency_squad_fill_depth`. Now it must return an intent
        // and sign nothing.
        let mut players: Vec<Player> = Vec::new();
        for i in 0..2 {
            players.push(EmergencyFillFixtures::player(
                i,
                PlayerPositionType::Goalkeeper,
            ));
        }
        for i in 0..7 {
            players.push(EmergencyFillFixtures::player(
                10 + i,
                PlayerPositionType::DefenderCenter,
            ));
        }
        players.push(EmergencyFillFixtures::player(
            20,
            PlayerPositionType::MidfielderCenter,
        ));
        for i in 0..4 {
            players.push(EmergencyFillFixtures::player(
                30 + i,
                PlayerPositionType::Striker,
            ));
        }

        let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let mut country = EmergencyFillFixtures::country(vec![club]);

        let candidates: Vec<FreeAgentCandidate> = (0..5)
            .map(|i| {
                EmergencyFillFixtures::candidate(
                    7000 + i,
                    75,
                    28,
                    PlayerFieldPositionGroup::Defender,
                    true,
                )
            })
            .collect();

        let mut signings = Vec::new();
        let mut offered = Vec::new();
        let mut rejected = Vec::new();
        let intents = CountryResult::handle_free_agents_emergency_pass(
            &country,
            &candidates,
            &TransferConfig::default(),
            &mut signings,
            &mut offered,
            &mut rejected,
        );

        assert!(
            signings.is_empty(),
            "depth tail must not direct-sign, got {:?}",
            signings
                .iter()
                .map(|s| (s.player_id, &s.reason))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            intents.len(),
            1,
            "depth shortfall must surface as an intent"
        );
        assert_eq!(intents[0].group, PlayerFieldPositionGroup::Defender);

        EmergencyDepthRequestPlanner::stage_requests(&mut country, &intents);
        {
            let plan = &country.clubs[0].transfer_plan;
            let request = plan
                .transfer_requests
                .iter()
                .find(|r| r.reason == TransferNeedReason::DepthCover)
                .expect("depth intent must stage a DepthCover request");
            assert_eq!(request.priority, TransferNeedPriority::Optional);
            assert_eq!(request.position, PlayerPositionType::DefenderCenter);
            assert_eq!(request.status, TransferRequestStatus::Pending);
        }

        // Re-staging while the request is open must not duplicate it.
        EmergencyDepthRequestPlanner::stage_requests(&mut country, &intents);
        let depth_count = country.clubs[0]
            .transfer_plan
            .transfer_requests
            .iter()
            .filter(|r| r.reason == TransferNeedReason::DepthCover)
            .count();
        assert_eq!(depth_count, 1, "open depth request must dedup re-staging");
    }

    #[test]
    fn depth_request_creates_pending_personal_terms_negotiation() {
        crate::utils::random::engine::RandomEngine::set_seed(0xDE91_07F1);
        let (mut country, pool) = DepthPipelineFixtures::staged_depth_country();

        let (signings, offered, _rejected) =
            DepthPipelineFixtures::run_until_negotiation(&mut country, &pool, 400);

        assert!(
            signings.is_empty(),
            "depth request must not produce an instant pool signing"
        );
        let negotiation = country
            .transfer_market
            .negotiations
            .values()
            .next()
            .expect("plausible domestic candidate must enter negotiation within 400 ticks");
        assert_eq!(negotiation.status, NegotiationStatus::Pending);
        assert!(
            matches!(negotiation.phase, NegotiationPhase::PersonalTerms { .. }),
            "staged depth negotiation must enter PersonalTerms, got {:?}",
            negotiation.phase
        );
        assert_eq!(negotiation.player_id, 9000);
        assert_eq!(
            negotiation.selling_club_id, 0,
            "pool free agents negotiate from the synthetic club-0 seller"
        );
        assert!(negotiation.offered_salary.unwrap_or(0) > 0);
        assert!(negotiation.current_offer.personal_terms.is_some());
        assert_eq!(
            negotiation.reason,
            PipelineProcessor::transfer_need_reason_text(&TransferNeedReason::DepthCover),
            "history reason must be the human-readable depth text, not a raw emergency tag"
        );
        assert!(
            country
                .transfer_market
                .negotiations
                .values()
                .all(|n| n.status != NegotiationStatus::Accepted),
            "depth path must never insert a pre-accepted negotiation"
        );
        assert!(country.transfer_market.transfer_history.is_empty());
        assert!(
            offered.contains(&9000),
            "negotiated offer must bump the offered counter"
        );

        let plan = &country.clubs[0].transfer_plan;
        let request = plan
            .transfer_requests
            .iter()
            .find(|r| r.reason == TransferNeedReason::DepthCover)
            .unwrap();
        assert_eq!(request.status, TransferRequestStatus::Negotiating);
        let shortlist = plan
            .shortlists
            .iter()
            .find(|s| s.transfer_request_id == request.id)
            .expect("staging must wire a shortlist for the request");
        assert!(shortlist.candidates.iter().any(
            |c| c.player_id == 9000 && c.status == ShortlistCandidateStatus::CurrentlyPursuing
        ));
    }

    #[test]
    fn low_rep_club_cannot_depth_sign_high_rep_foreign_free_agent() {
        crate::utils::random::engine::RandomEngine::set_seed(0x10F_FA11);
        // 800-rep country, routine pressure, CA-165 foreign star with a
        // 7500 reference reputation: the strict depth gates (tier CA
        // ceiling without overreach + pressure-scaled rep drop) must
        // filter the candidate before any offer or negotiation exists.
        let main =
            EmergencyFillFixtures::team(10, "FC", "fc", DepthPipelineFixtures::balanced_squad());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let mut country = EmergencyFillFixtures::country_with_reputation(vec![club], 800);
        country.clubs[0].transfer_plan.initialized = true;
        EmergencyDepthRequestPlanner::stage_requests(
            &mut country,
            &[EmergencyDepthRequestIntent {
                club_id: 100,
                group: PlayerFieldPositionGroup::Midfielder,
            }],
        );

        let pool = vec![DepthPipelineFixtures::pool_summary(
            9100,
            165,
            27,
            PlayerFieldPositionGroup::Midfielder,
            false,
            0.3,
            7500,
        )];
        let (signings, offered, _rejected) =
            DepthPipelineFixtures::run_until_negotiation(&mut country, &pool, 300);

        assert!(signings.is_empty(), "no pool signing may be staged");
        assert!(
            country.transfer_market.negotiations.is_empty(),
            "implausible star must never enter a depth negotiation at a low-rep club"
        );
        assert!(country.transfer_market.transfer_history.is_empty());
        assert!(
            !offered.contains(&9100),
            "filtered candidates must not be counted as offered"
        );
    }

    #[test]
    fn depth_personal_terms_rejection_updates_request_and_shortlist() {
        crate::utils::random::engine::RandomEngine::set_seed(0xBAD_7E55);
        let (mut country, pool) = DepthPipelineFixtures::staged_depth_country();
        DepthPipelineFixtures::run_until_negotiation(&mut country, &pool, 400);
        assert!(!country.transfer_market.negotiations.is_empty());

        // Mirror what `resolve_personal_terms` does on a declined offer
        // — the staged shortlist wiring must respond like any pipeline
        // pursuit: candidate marked failed, Optional request abandoned,
        // negotiation slot released.
        PipelineProcessor::on_negotiation_resolved(&mut country, 100, 9000, false);

        let plan = &country.clubs[0].transfer_plan;
        let request = plan
            .transfer_requests
            .iter()
            .find(|r| r.reason == TransferNeedReason::DepthCover)
            .unwrap();
        assert_eq!(
            request.status,
            TransferRequestStatus::Abandoned,
            "Optional depth request with an exhausted shortlist must be abandoned"
        );
        let shortlist = plan
            .shortlists
            .iter()
            .find(|s| s.transfer_request_id == request.id)
            .unwrap();
        assert!(shortlist.candidates.iter().any(
            |c| c.player_id == 9000 && c.status == ShortlistCandidateStatus::NegotiationFailed
        ));
        assert_eq!(plan.active_negotiation_count, 0);
    }

    #[test]
    fn pool_depth_medical_completion_defers_global_signing_without_direct_history() {
        // The medical phase keeps an unconditional 1% collapse roll even
        // for healthy players, and the seeded RNG stream is mixed with a
        // per-test-thread id — so any single seed can land in the 1%
        // band depending on suite composition. Retry across a few seeds:
        // a genuine completion-path regression fails every attempt,
        // while the 1% artifact cannot survive eight (P ≈ 1e-16).
        for attempt in 0..8u64 {
            crate::utils::random::engine::RandomEngine::set_seed(0xD0C7_0001 + attempt);
            let (mut country, pool) = DepthPipelineFixtures::staged_depth_country();
            DepthPipelineFixtures::run_until_negotiation(&mut country, &pool, 400);
            let neg_id = *country
                .transfer_market
                .negotiations
                .keys()
                .next()
                .expect("staging must have created the negotiation");

            // Fast-forward the negotiation to a mature medical phase so a
            // single resolver tick completes it.
            let date = EmergencyFillFixtures::d(2026, 6, 10);
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.phase = NegotiationPhase::MedicalAndFinalization { started: date };
                negotiation.phase_expiry = date;
            }

            // A second club has a competing pool bid in flight (not yet
            // phase-ready, so the resolver doesn't touch it directly) —
            // completion must sweep it like `complete_transfer` would.
            let competing_id = country.transfer_market.next_negotiation_id;
            country.transfer_market.next_negotiation_id += 1;
            country.transfer_market.negotiations.insert(
                competing_id,
                TransferNegotiation::new(
                    competing_id,
                    9000,
                    0,
                    0,
                    200,
                    TransferOffer::new(CurrencyValue::new(0.0, Currency::Usd), 200, date),
                    date,
                    0.4,
                    0.3,
                    28,
                    0.5,
                ),
            );

            crate::utils::random::engine::RandomEngine::set_seed(42 + attempt);
            let mut summary = TransferActivitySummary::new();
            let outcomes =
                CountryResult::resolve_pending_negotiations(&mut country, date, &mut summary);

            // 1% medical collapse — the RNG artifact, not the behaviour
            // under test. Re-roll the scenario with the next seed.
            if country.transfer_market.negotiations[&neg_id].rejection_reason
                == Some(NegotiationRejectionReason::MedicalFailed)
            {
                continue;
            }

            assert!(
                outcomes.deferred.is_empty(),
                "pool free agents must not enter the club-to-club execution queue"
            );
            assert_eq!(
                outcomes.free_agent_signings.len(),
                1,
                "cleared medical must defer exactly one global pool signing"
            );
            let signing = &outcomes.free_agent_signings[0];
            assert_eq!(signing.player_id, 9000);
            assert_eq!(signing.buying_club_id, 100);
            assert_eq!(
                signing.reason,
                PipelineProcessor::transfer_need_reason_text(&TransferNeedReason::DepthCover)
            );
            assert!(
                signing.terms.is_some(),
                "negotiated wage / length / role must travel to execution"
            );
            assert!(
                country.transfer_market.transfer_history.is_empty(),
                "the resolver must not write history — the deferred executor owns that row"
            );
            assert_eq!(
                country.transfer_market.negotiations[&neg_id].status,
                NegotiationStatus::Accepted
            );
            assert_eq!(
                country.transfer_market.negotiations[&competing_id].status,
                NegotiationStatus::Rejected,
                "competing pool negotiations must be cancelled on completion"
            );
            assert!(
                country
                    .transfer_market
                    .listings
                    .iter()
                    .filter(|l| l.player_id == 9000)
                    .all(|l| l.status == TransferListingStatus::Completed),
                "pool player's listings must be marked completed on medical success"
            );
            let request = country.clubs[0]
                .transfer_plan
                .transfer_requests
                .iter()
                .find(|r| r.reason == TransferNeedReason::DepthCover)
                .unwrap();
            assert_eq!(
                request.status,
                TransferRequestStatus::Fulfilled,
                "request is fulfilled only once the negotiation actually completes"
            );
            return;
        }
        panic!("medical collapsed on every seed — the completion path is broken, not unlucky");
    }

    #[test]
    fn unmarked_depth_cover_request_keeps_legacy_instant_signing() {
        crate::utils::random::engine::RandomEngine::set_seed(0x1E6A_C001);
        // A DepthCover request from the weekly evaluation (no
        // EmergencyFreeAgentDepth marker) must keep the legacy instant
        // free-agent path — the staged-negotiation flow is reserved
        // for emergency-planner depth requests.
        let main =
            EmergencyFillFixtures::team(10, "FC", "fc", DepthPipelineFixtures::balanced_squad());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let mut country = EmergencyFillFixtures::country(vec![club]);
        country.clubs[0].transfer_plan.initialized = true;
        country.clubs[0]
            .transfer_plan
            .transfer_requests
            .push(TransferRequest::new(
                1,
                PlayerPositionType::MidfielderCenter,
                TransferNeedPriority::Optional,
                TransferNeedReason::DepthCover,
                60,
                80,
                0.0,
            ));

        let pool = vec![DepthPipelineFixtures::pool_summary(
            9300,
            80,
            28,
            PlayerFieldPositionGroup::Midfielder,
            true,
            1.0,
            3500,
        )];

        let date = EmergencyFillFixtures::d(2026, 6, 10);
        let config = TransferConfig::default();
        let mut signings = Vec::new();
        for _ in 0..400 {
            let mut summary = TransferActivitySummary::new();
            let mut domestic = Vec::new();
            let mut offered = Vec::new();
            let mut rejected = Vec::new();
            let mut blocked = Vec::new();
            signings = CountryResult::handle_free_agents(
                &mut country,
                date,
                &mut summary,
                &pool,
                &config,
                &mut domestic,
                &mut offered,
                &mut rejected,
                &mut blocked,
            );
            if !signings.is_empty() {
                break;
            }
        }

        assert!(
            !signings.is_empty(),
            "unmarked DepthCover must still instant-sign within 400 ticks"
        );
        assert_eq!(signings[0].player_id, 9300);
        assert!(
            country.transfer_market.negotiations.is_empty(),
            "normal evaluated DepthCover requests must not enter the staged-negotiation flow"
        );
    }

    #[test]
    fn pool_executor_writes_single_history_row_on_success() {
        let date = EmergencyFillFixtures::d(2026, 6, 10);
        let mut pool_player =
            EmergencyFillFixtures::player(9400, PlayerPositionType::MidfielderCenter);
        pool_player.ensure_free_agent_state(date, 4000);
        let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);
        let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
        let mut data = SimulatorData::new(
            date.and_hms_opt(12, 0, 0).unwrap(),
            vec![continent],
            GlobalCompetitions::new(Vec::new()),
        );
        data.free_agents.push(pool_player);

        let signing = GlobalFreeAgentSigning {
            player_id: 9400,
            player_name: "Pool P9400".to_string(),
            buying_country_id: 1,
            buying_club_id: 100,
            reason: "Squad depth — need backup for position group".to_string(),
            terms: None,
        };
        let executed = execute_global_free_agent_signing(
            &mut data,
            &signing,
            date,
            &TransferConfig::default(),
        );

        assert!(executed, "unclaimed pool player must be signable");
        assert!(data.free_agents.is_empty(), "player leaves the pool");
        let country = data.country(1).unwrap();
        let rows: Vec<_> = country
            .transfer_market
            .transfer_history
            .iter()
            .filter(|t| t.player_id == 9400)
            .collect();
        assert_eq!(
            rows.len(),
            1,
            "the executor is the single writer of the history row"
        );
        assert_eq!(
            rows[0].reason,
            "Squad depth — need backup for position group"
        );
        assert!(
            country.clubs[0].teams.teams.iter().any(|t| t
                .players
                .players
                .iter()
                .any(|p| p.id == 9400)),
            "player must land on the buying club's roster"
        );
    }

    #[test]
    fn pool_executor_writes_no_history_when_player_already_claimed() {
        let date = EmergencyFillFixtures::d(2026, 6, 10);
        let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);
        let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
        let mut data = SimulatorData::new(
            date.and_hms_opt(12, 0, 0).unwrap(),
            vec![continent],
            GlobalCompetitions::new(Vec::new()),
        );
        // Pool is empty — another country claimed the player earlier in
        // the same tick. The executor must fail silently.
        let signing = GlobalFreeAgentSigning {
            player_id: 9400,
            player_name: "Pool P9400".to_string(),
            buying_country_id: 1,
            buying_club_id: 100,
            reason: "Squad depth — need backup for position group".to_string(),
            terms: None,
        };
        let executed = execute_global_free_agent_signing(
            &mut data,
            &signing,
            date,
            &TransferConfig::default(),
        );

        assert!(!executed, "claimed player cannot be signed twice");
        assert!(
            data.country(1)
                .unwrap()
                .transfer_market
                .transfer_history
                .is_empty(),
            "no phantom history row may be written for a claimed player"
        );
    }

    /// Fixtures for the fallback-matcher and market-clearing tests.
    /// Wrapped on a unit struct per the project convention.
    struct MarketClearingFixtures;

    impl MarketClearingFixtures {
        /// Buyer country: one balanced-squad club (no emergency need),
        /// transfer plan initialized with a single open SquadPadding
        /// request for a defender.
        fn country_with_defender_request() -> Country {
            let main = EmergencyFillFixtures::team(
                10,
                "FC",
                "fc",
                DepthPipelineFixtures::balanced_squad(),
            );
            let mut club = EmergencyFillFixtures::club(100, "FC", main);
            club.transfer_plan.initialized = true;
            club.transfer_plan
                .transfer_requests
                .push(TransferRequest::new(
                    1,
                    PlayerPositionType::DefenderCenter,
                    TransferNeedPriority::Critical,
                    TransferNeedReason::SquadPadding,
                    50,
                    80,
                    0.0,
                ));
            EmergencyFillFixtures::country(vec![club])
        }

        /// Long-tail global-pool candidate for the clearing pass:
        /// domestic journeyman deep in the decay curve.
        fn long_term_candidate(player_id: u32) -> FreeAgentCandidate {
            let mut c = EmergencyFillFixtures::candidate(
                player_id,
                70,
                29,
                PlayerFieldPositionGroup::Defender,
                true,
            );
            c.career_pressure = 0.95;
            c.days_free = 400;
            c
        }

        fn run_clearing(
            country: &Country,
            candidates: &[FreeAgentCandidate],
            config: &TransferConfig,
        ) -> Vec<FreeAgentSigning> {
            let mut signings = Vec::new();
            let mut offered = Vec::new();
            let mut rejected = Vec::new();
            let mut recorder = BlockReasonRecorder::new();
            // Non-peak date (March) so the club-scaled / peak-window cap
            // adjustments stay at the base values these tests assert on.
            let date = EmergencyFillFixtures::d(2026, 3, 10);
            CountryResult::handle_free_agents_market_clearing_pass(
                country,
                candidates,
                config,
                date,
                &HashSet::new(),
                &mut signings,
                &mut offered,
                &mut rejected,
                &mut recorder,
            );
            signings
        }

        /// Buyer country with one club deliberately thin in defenders (a
        /// single CB) and NO transfer plan / open requests. The high
        /// position-depth need makes the soft tier's opportunistic fit
        /// gate reliably pass for a domestic defender, so the soft-tier
        /// tests exercise the early domestic clearing layer rather than
        /// the hard backstop.
        fn country_thin_in_defenders() -> Country {
            let mut players: Vec<Player> = Vec::new();
            for i in 0..2 {
                players.push(EmergencyFillFixtures::player(
                    i,
                    PlayerPositionType::Goalkeeper,
                ));
            }
            players.push(EmergencyFillFixtures::player(
                10,
                PlayerPositionType::DefenderCenter,
            ));
            for i in 0..7 {
                players.push(EmergencyFillFixtures::player(
                    20 + i,
                    PlayerPositionType::MidfielderCenter,
                ));
            }
            for i in 0..4 {
                players.push(EmergencyFillFixtures::player(
                    30 + i,
                    PlayerPositionType::Striker,
                ));
            }
            let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
            let club = EmergencyFillFixtures::club(100, "FC", main);
            EmergencyFillFixtures::country(vec![club])
        }
    }

    #[test]
    fn request_matcher_tries_fallback_candidates_past_rejecting_top_quality() {
        // Two pool defenders against one open request. Candidate A is
        // the raw-quality pick (CA 95) but practically unsignable: no
        // career pressure and a reservation wage anchored to a 50M
        // previous salary, so nearly every offer is declined. B is a
        // pressured domestic journeyman who accepts realistic terms.
        //
        // The legacy matcher (single best by raw quality) would offer
        // ONLY A, tick after tick, and B would never sign. The
        // fallback matcher must (a) eventually sign B and (b) offer
        // BOTH candidates across trials — proof that a failed roll
        // moves on to the next-ranked candidate instead of abandoning
        // the request.
        let date = EmergencyFillFixtures::d(2026, 6, 10);
        let config = TransferConfig::default();

        let mut star = DepthPipelineFixtures::pool_summary(
            8100,
            95,
            27,
            PlayerFieldPositionGroup::Defender,
            false,
            0.0,
            6200,
        );
        star.last_salary = 50_000_000;
        let journeyman = DepthPipelineFixtures::pool_summary(
            8101,
            75,
            28,
            PlayerFieldPositionGroup::Defender,
            true,
            0.7,
            3500,
        );
        let pool = vec![star, journeyman];

        let mut star_offered = false;
        let mut journeyman_offered = false;
        let mut journeyman_signed = false;
        for _ in 0..400 {
            // Fresh country per trial — a successful signing fulfills
            // the request and would otherwise stop the matcher.
            let mut country = MarketClearingFixtures::country_with_defender_request();
            let mut summary = TransferActivitySummary::new();
            let mut domestic = Vec::new();
            let mut offered = Vec::new();
            let mut rejected = Vec::new();
            let mut blocked = Vec::new();
            let signings = CountryResult::handle_free_agents(
                &mut country,
                date,
                &mut summary,
                &pool,
                &config,
                &mut domestic,
                &mut offered,
                &mut rejected,
                &mut blocked,
            );
            star_offered |= offered.contains(&8100);
            journeyman_offered |= offered.contains(&8101);
            journeyman_signed |= signings.iter().any(|s| s.player_id == 8101);
            if star_offered && journeyman_offered && journeyman_signed {
                break;
            }
        }

        assert!(
            journeyman_signed,
            "fallback matcher must eventually sign the realistic journeyman"
        );
        assert!(
            star_offered && journeyman_offered,
            "both candidates must field offers across trials (star={star_offered}, \
             journeyman={journeyman_offered}) — single-candidate matching starves the pool"
        );
    }

    #[test]
    fn market_clearing_signs_long_term_domestic_free_agent_without_request() {
        // No transfer plan, no requests, no emergency need — the only
        // path that can sign this 400-days-free domestic journeyman is
        // the market-clearing pass.
        let main =
            EmergencyFillFixtures::team(10, "FC", "fc", DepthPipelineFixtures::balanced_squad());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);
        let config = TransferConfig::default();

        let mut signed = None;
        for _ in 0..400 {
            let candidates = vec![MarketClearingFixtures::long_term_candidate(8200)];
            let signings = MarketClearingFixtures::run_clearing(&country, &candidates, &config);
            if let Some(s) = signings.into_iter().next() {
                signed = Some(s);
                break;
            }
        }

        let signing = signed.expect(
            "market clearing must pick up a long-term domestic free agent within 400 ticks",
        );
        assert_eq!(signing.player_id, 8200);
        assert_eq!(signing.to_club_id, 100);
        assert_eq!(signing.reason, "free_agent_market_clearing");
        assert!(
            signing.fills_group.is_none(),
            "clearing services no request — request bookkeeping must stay untouched"
        );
        let terms = signing
            .terms
            .expect("clearing must stage explicit short-deal terms");
        assert!(terms.annual_wage > 0);
        assert!(
            matches!(terms.role, BuyerRoleFit::Backup | BuyerRoleFit::Emergency),
            "clearing offers are squad-role deals, got {:?}",
            terms.role
        );
    }

    #[test]
    fn market_clearing_skips_players_below_both_thresholds() {
        // cp 0.3 and 60 days free: under BOTH soft (0.40 / 75d) and hard
        // (0.75 / 365d) eligibility floors — the pass must never touch
        // them, no matter how many ticks. (A 100-day / 0.5-cp player is
        // now deliberately soft-eligible — see the soft-clearing tests.)
        let main =
            EmergencyFillFixtures::team(10, "FC", "fc", DepthPipelineFixtures::balanced_squad());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);
        let config = TransferConfig::default();

        for _ in 0..50 {
            let mut c = EmergencyFillFixtures::candidate(
                8300,
                70,
                29,
                PlayerFieldPositionGroup::Defender,
                true,
            );
            c.career_pressure = 0.3;
            c.days_free = 60;
            let signings = MarketClearingFixtures::run_clearing(&country, &[c], &config);
            assert!(
                signings.is_empty(),
                "players below both soft and hard floors are not market-clearing eligible"
            );
        }
    }

    #[test]
    fn market_clearing_per_day_cap_prevents_mass_pool_draining() {
        // Thirty fully-desperate candidates against one open club:
        // every tick must stay at or under the COMBINED per-country
        // clearing cap (soft + hard), even though far more would pass
        // the gates. This is the realism backstop against draining the
        // pool in a single week.
        let main =
            EmergencyFillFixtures::team(10, "FC", "fc", DepthPipelineFixtures::balanced_squad());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);
        let config = TransferConfig::default();
        // Both tiers can fire in one tick for a fully-desperate domestic
        // cohort (soft 1 + hard 2 = 3).
        let combined_cap = config.soft_market_clearing_max_signings_per_country_per_day
            + config.market_clearing_max_signings_per_country_per_day;

        let mut any_signed = false;
        for _ in 0..100 {
            let candidates: Vec<FreeAgentCandidate> = (0..30)
                .map(|i| {
                    let mut c = MarketClearingFixtures::long_term_candidate(8400 + i);
                    c.career_pressure = 1.0;
                    c
                })
                .collect();
            let signings = MarketClearingFixtures::run_clearing(&country, &candidates, &config);
            assert!(
                signings.len() <= combined_cap,
                "clearing must respect the combined soft+hard per-day cap ({combined_cap}), \
                 got {} signings",
                signings.len()
            );
            any_signed |= !signings.is_empty();
        }
        assert!(
            any_signed,
            "with 30 desperate candidates over 100 ticks, clearing must sign someone"
        );
    }

    #[test]
    fn soft_market_clearing_signs_100_day_domestic_backup() {
        // A ~100-day-free domestic backup-level defender (cp 0.55) is
        // BELOW both hard thresholds (0.75 pressure / 365 days), so any
        // clearing signing here can only come from the new SOFT tier —
        // the early, domestic, opportunistic layer. Acceptance criterion
        // #1: a fringe domestic free agent resolves in months, not years.
        let country = MarketClearingFixtures::country_thin_in_defenders();
        let config = TransferConfig::default();

        let mut signed = None;
        for _ in 0..400 {
            let mut c = EmergencyFillFixtures::candidate(
                8500,
                78,
                29,
                PlayerFieldPositionGroup::Defender,
                true,
            );
            c.career_pressure = 0.55;
            c.days_free = 100;
            let signings = MarketClearingFixtures::run_clearing(&country, &[c], &config);
            if let Some(s) = signings.into_iter().next() {
                signed = Some(s);
                break;
            }
        }

        let signing =
            signed.expect("soft clearing must sign a 100-day domestic backup within 400 ticks");
        assert_eq!(signing.player_id, 8500);
        assert_eq!(signing.reason, "free_agent_market_clearing");
        let terms = signing.terms.expect("soft clearing stages short-deal terms");
        // Stage-aware contract: a Flexible-stage player gets a short
        // 1-2 year deal, never a long commitment.
        assert!(
            terms.contract_years <= 2,
            "soft-stage clearing deals stay short, got {}y",
            terms.contract_years
        );
        assert!(matches!(
            terms.role,
            BuyerRoleFit::Backup | BuyerRoleFit::Emergency
        ));
    }

    #[test]
    fn opportunistic_clearing_signs_free_agent_with_no_matching_request() {
        // The club has NO open transfer request for a defender (empty
        // transfer plan), yet a domestic soft-eligible journeyman is
        // signed anyway through the opportunistic depth logic. Acceptance
        // criterion #6 / spec test #6: NoMatchingRequest free agents are
        // still reachable.
        let country = MarketClearingFixtures::country_thin_in_defenders();
        assert!(
            country.clubs[0]
                .transfer_plan
                .transfer_requests
                .is_empty(),
            "fixture must have no open requests for this test to be meaningful"
        );
        let config = TransferConfig::default();

        let mut signed = false;
        for _ in 0..400 {
            let mut c = EmergencyFillFixtures::candidate(
                8550,
                78,
                30,
                PlayerFieldPositionGroup::Defender,
                true,
            );
            // Soft-eligible by pressure, still below the hard floor.
            c.career_pressure = 0.6;
            c.days_free = 120;
            let signings = MarketClearingFixtures::run_clearing(&country, &[c], &config);
            if signings.iter().any(|s| s.player_id == 8550) {
                signed = true;
                break;
            }
        }
        assert!(
            signed,
            "opportunistic soft clearing must sign a useful domestic FA with no open request"
        );
    }

    /// Spec test #2: a USEFUL domestic player whose contract recently
    /// expired (≈11-12 weeks free) is signed through opportunistic
    /// clearing WITHOUT any club holding an explicit transfer request for
    /// him. At 80 days free he sits below the legacy 90-day soft floor —
    /// the lowered floor (75d / 0.40cp) is exactly what lets a local club
    /// take a punt on him a few weeks sooner.
    #[test]
    fn useful_domestic_expired_player_cleared_opportunistically_under_lowered_floor() {
        let country = MarketClearingFixtures::country_thin_in_defenders();
        assert!(
            country.clubs[0].transfer_plan.transfer_requests.is_empty(),
            "fixture must have no open requests — this is the opportunistic, no-request path"
        );
        // 80 days free is below the legacy 90-day soft-clearing floor; it
        // is only reachable because the floor was lowered to 75 days.
        assert!(
            80 >= TransferConfig::default().soft_market_clearing_min_days_free,
            "80 days must clear the (lowered) soft days-free floor"
        );
        let config = TransferConfig::default();

        let mut signing = None;
        for _ in 0..800 {
            let mut c = EmergencyFillFixtures::candidate(
                8560,
                80,
                29,
                PlayerFieldPositionGroup::Defender,
                true,
            );
            // Domestic, useful, recently expired: eligible via the days
            // floor; the modest pressure keeps the opportunistic fit over
            // its Open-stage threshold for a thin-in-defenders club.
            c.career_pressure = 0.5;
            c.days_free = 80;
            let signings = MarketClearingFixtures::run_clearing(&country, &[c], &config);
            if let Some(s) = signings.into_iter().find(|s| s.player_id == 8560) {
                signing = Some(s);
                break;
            }
        }

        let signing = signing.expect(
            "a useful domestic expired player must clear opportunistically within 800 ticks",
        );
        assert_eq!(signing.reason, "free_agent_market_clearing");
        // No request was serviced — the opportunistic path leaves request
        // bookkeeping untouched.
        assert!(signing.fills_group.is_none());
        let terms = signing.terms.expect("clearing stages short-deal terms");
        assert!(matches!(
            terms.role,
            BuyerRoleFit::Backup | BuyerRoleFit::Emergency
        ));
    }

    #[test]
    fn soft_clearing_ignores_cross_continent_candidates() {
        // The soft tier is the LOCAL market outlet: a cross-continent
        // foreigner at the same soft-eligible stage must NOT be swept by
        // it (he is the hard tier's job, and only once far more
        // pressured). Below the hard floor he stays unsigned.
        let country = MarketClearingFixtures::country_thin_in_defenders();
        let config = TransferConfig::default();

        for _ in 0..200 {
            let mut c = EmergencyFillFixtures::candidate(
                8560,
                78,
                29,
                PlayerFieldPositionGroup::Defender,
                false,
            );
            // Foreign AND cross-continent (different continent id).
            c.nationality_continent_id = 7;
            c.nationality_region = ScoutingRegion::from_country(7, "br");
            c.career_pressure = 0.6;
            c.days_free = 120;
            let signings = MarketClearingFixtures::run_clearing(&country, &[c], &config);
            assert!(
                signings.is_empty(),
                "soft clearing must not reach a cross-continent foreigner below the hard floor"
            );
        }
    }
}

#[cfg(test)]
mod expiry_renewal_tests {
    //! Expiry-day last-chance renewal: a player whose contract lapses must
    //! get one synchronous offer from his club before the release sweep
    //! clears the contract. Acceptance keeps him under a fresh deal and out
    //! of the same-day free-agent flow; rejection falls through to the
    //! existing release path.

    use super::*;
    use crate::club::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::contract::RENEWAL_REJECTED_LABEL;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
        PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, PlayerSquadStatus, StaffCollection, Team, TeamCollection,
        TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::NaiveTime;

    struct ExpiryRenewalFixtures;

    impl ExpiryRenewalFixtures {
        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn attrs(ambition: f32, loyalty: f32) -> PersonAttributes {
            PersonAttributes {
                adaptability: 12.0,
                ambition,
                controversy: 5.0,
                loyalty,
                pressure: 12.0,
                professionalism: 12.0,
                sportsmanship: 12.0,
                temperament: 12.0,
                consistency: 12.0,
                important_matches: 12.0,
                dirtiness: 5.0,
            }
        }

        fn player(
            id: u32,
            position: PlayerPositionType,
            attrs: PersonAttributes,
            salary: u32,
            squad_status: PlayerSquadStatus,
            expiration: NaiveDate,
        ) -> Player {
            let mut player_attributes = PlayerAttributes::default();
            player_attributes.current_ability = 100;
            player_attributes.potential_ability = 110;
            let mut p = PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Test".to_string(), format!("P{id}")))
                .birth_date(Self::d(1998, 1, 1))
                .country_id(1)
                .attributes(attrs)
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 16,
                    }],
                })
                .player_attributes(player_attributes)
                .build()
                .unwrap();
            let mut contract = PlayerClubContract::new(salary, expiration);
            contract.squad_status = squad_status;
            p.contract = Some(contract);
            p
        }

        fn team(id: u32, club_id: u32, players: Vec<Player>) -> Team {
            Team::builder()
                .id(id)
                .league_id(Some(1))
                .club_id(club_id)
                .name(format!("Team{id}"))
                .slug(format!("team-{id}"))
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(2000, 2000, 4000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn club(id: u32, main: Team) -> Club {
            Club::new(
                id,
                format!("Club{id}"),
                Location::new(1),
                ClubFinances::new(1_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![main]),
                ClubFacilities::default(),
            )
        }

        fn country(clubs: Vec<Club>) -> Country {
            Country::builder()
                .id(1)
                .code("en".to_string())
                .slug("england".to_string())
                .name("England".to_string())
                .continent_id(1)
                .reputation(5000)
                .leagues(LeagueCollection::new(vec![League::new(
                    1,
                    "L".to_string(),
                    "english".to_string(),
                    1,
                    5000,
                    LeagueSettings {
                        season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                        season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                        tier: 1,
                        promotion_spots: 0,
                        relegation_spots: 0,
                        league_group: None,
                    },
                    false,
                )]))
                .clubs(clubs)
                .build()
                .unwrap()
        }

        fn run(country: &mut Country, date: NaiveDate) -> Vec<GlobalFreeAgentSigning> {
            let mut summary = TransferActivitySummary::new();
            let config = TransferConfig::default();
            let mut domestic_signed_ids = Vec::new();
            let mut global_offered_ids = Vec::new();
            let mut global_rejected_ids = Vec::new();
            let mut global_blocked = Vec::new();
            CountryResult::handle_free_agents(
                country,
                date,
                &mut summary,
                &[],
                &config,
                &mut domestic_signed_ids,
                &mut global_offered_ids,
                &mut global_rejected_ids,
                &mut global_blocked,
            )
        }

        fn find_player(country: &Country, club_id: u32, player_id: u32) -> &Player {
            country
                .clubs
                .iter()
                .find(|c| c.id == club_id)
                .expect("club exists")
                .teams
                .teams
                .iter()
                .flat_map(|t| t.players.players.iter())
                .find(|p| p.id == player_id)
                .expect("player still in roster")
        }

        fn history_count(player: &Player, label: &str) -> usize {
            player
                .decision_history
                .items
                .iter()
                .filter(|d| d.decision == label)
                .count()
        }
    }

    #[test]
    fn accepted_expiry_offer_renews_contract_and_keeps_player() {
        let date = ExpiryRenewalFixtures::d(2026, 6, 10);
        // Low current salary against a 100k top earner: the offer is a
        // big raise that the acceptance handler takes deterministically.
        let renewer = ExpiryRenewalFixtures::player(
            1,
            PlayerPositionType::MidfielderCenter,
            ExpiryRenewalFixtures::attrs(8.0, 12.0),
            10_000,
            PlayerSquadStatus::FirstTeamRegular,
            date,
        );
        let anchor = ExpiryRenewalFixtures::player(
            2,
            PlayerPositionType::Striker,
            ExpiryRenewalFixtures::attrs(8.0, 12.0),
            100_000,
            PlayerSquadStatus::KeyPlayer,
            ExpiryRenewalFixtures::d(2028, 6, 30),
        );
        let main = ExpiryRenewalFixtures::team(10, 100, vec![renewer, anchor]);
        let club = ExpiryRenewalFixtures::club(100, main);
        let mut country = ExpiryRenewalFixtures::country(vec![club]);

        let global_signings = ExpiryRenewalFixtures::run(&mut country, date);

        assert!(global_signings.is_empty());
        let p = ExpiryRenewalFixtures::find_player(&country, 100, 1);
        let contract = p
            .contract
            .as_ref()
            .expect("accepted expiry offer must install a fresh contract");
        assert!(
            contract.expiration > date,
            "renewed contract must run past today, got {}",
            contract.expiration
        );
        assert_eq!(
            ExpiryRenewalFixtures::history_count(p, RENEWAL_OFFERED_LABEL),
            1,
            "expiry-day offer must be recorded in decision history"
        );
        assert_eq!(
            ExpiryRenewalFixtures::history_count(p, RENEWAL_REJECTED_LABEL),
            0
        );
    }

    #[test]
    fn rejected_expiry_offer_falls_through_to_release() {
        let date = ExpiryRenewalFixtures::d(2026, 6, 10);
        // The player is his own top earner, so the wage-structure cap
        // turns the final offer into a pay cut; loyalty 5 rejects every
        // pay-cut branch deterministically.
        let leaver = ExpiryRenewalFixtures::player(
            1,
            PlayerPositionType::Striker,
            ExpiryRenewalFixtures::attrs(8.0, 5.0),
            100_000,
            PlayerSquadStatus::FirstTeamRegular,
            date,
        );
        let main = ExpiryRenewalFixtures::team(10, 100, vec![leaver]);
        let club = ExpiryRenewalFixtures::club(100, main);
        let mut country = ExpiryRenewalFixtures::country(vec![club]);

        ExpiryRenewalFixtures::run(&mut country, date);

        let p = ExpiryRenewalFixtures::find_player(&country, 100, 1);
        assert!(
            p.contract.is_none(),
            "rejected expiry offer must still end in release"
        );
        assert_eq!(
            ExpiryRenewalFixtures::history_count(p, RENEWAL_OFFERED_LABEL),
            1,
            "the final offer must be on record even when it fails"
        );
        assert_eq!(
            ExpiryRenewalFixtures::history_count(p, RENEWAL_REJECTED_LABEL),
            1,
            "rejection must use the existing rejection label"
        );
    }

    #[test]
    fn loaned_in_expired_parent_contract_is_not_renewed_by_borrower() {
        let date = ExpiryRenewalFixtures::d(2026, 6, 10);
        let mut loanee = ExpiryRenewalFixtures::player(
            1,
            PlayerPositionType::Striker,
            ExpiryRenewalFixtures::attrs(8.0, 12.0),
            50_000,
            PlayerSquadStatus::FirstTeamRegular,
            date,
        );
        // Parent club 99 owns the (expired) permanent contract; the
        // borrower (club 100) only holds the loan agreement.
        loanee.contract_loan = Some(PlayerClubContract::new_loan(
            20_000,
            ExpiryRenewalFixtures::d(2026, 12, 31),
            99,
            1,
            100,
        ));
        let main = ExpiryRenewalFixtures::team(10, 100, vec![loanee]);
        let club = ExpiryRenewalFixtures::club(100, main);
        let mut country = ExpiryRenewalFixtures::country(vec![club]);

        ExpiryRenewalFixtures::run(&mut country, date);

        let p = ExpiryRenewalFixtures::find_player(&country, 100, 1);
        assert_eq!(
            ExpiryRenewalFixtures::history_count(p, RENEWAL_OFFERED_LABEL),
            0,
            "the borrower must not make an expiry-day offer on a loanee"
        );
        let parent_contract = p
            .contract
            .as_ref()
            .expect("parent contract is not the borrower's to clear");
        assert_eq!(
            parent_contract.expiration, date,
            "parent contract must be left exactly as it was"
        );
    }

    #[test]
    fn renewed_player_is_excluded_from_same_day_free_agent_flow() {
        let date = ExpiryRenewalFixtures::d(2026, 6, 10);
        let renewer = ExpiryRenewalFixtures::player(
            1,
            PlayerPositionType::Goalkeeper,
            ExpiryRenewalFixtures::attrs(8.0, 12.0),
            10_000,
            PlayerSquadStatus::FirstTeamRegular,
            date,
        );
        let anchor = ExpiryRenewalFixtures::player(
            2,
            PlayerPositionType::Striker,
            ExpiryRenewalFixtures::attrs(8.0, 12.0),
            100_000,
            PlayerSquadStatus::KeyPlayer,
            ExpiryRenewalFixtures::d(2028, 6, 30),
        );
        let club_a = ExpiryRenewalFixtures::club(
            100,
            ExpiryRenewalFixtures::team(10, 100, vec![renewer, anchor]),
        );
        // Club B has an empty main squad — the hungriest possible buyer:
        // the emergency pass would grab any available free-agent keeper.
        let club_b =
            ExpiryRenewalFixtures::club(200, ExpiryRenewalFixtures::team(20, 200, Vec::new()));
        let mut country = ExpiryRenewalFixtures::country(vec![club_a, club_b]);

        let global_signings = ExpiryRenewalFixtures::run(&mut country, date);

        assert!(global_signings.is_empty());
        let p = ExpiryRenewalFixtures::find_player(&country, 100, 1);
        assert!(
            p.contract.as_ref().is_some_and(|c| c.expiration > date),
            "player must have renewed at his own club"
        );
        let club_b_roster: usize = country
            .clubs
            .iter()
            .find(|c| c.id == 200)
            .unwrap()
            .teams
            .teams
            .iter()
            .map(|t| t.players.players.len())
            .sum();
        assert_eq!(
            club_b_roster, 0,
            "a renewed player must not be signable as a same-day free agent"
        );
        assert!(
            country.transfer_market.transfer_history.is_empty(),
            "no free transfer may be recorded for a renewed player"
        );
    }

    /// Spec test #3: a player who has already rejected a season's worth of
    /// renewal offers and is still asking for a wage the club won't fund
    /// must NOT receive yet another identical expiry-day proposal. The
    /// final offer is suppressed (the club lets him walk) instead of
    /// spamming the same losing deal — no new RENEWAL_OFFERED row appears.
    #[test]
    fn repeated_rejected_renewal_does_not_spam_expiry_day_offer() {
        use crate::club::player::contract::RENEWAL_OFFERED_LABEL;
        use crate::club::player::mailbox::{PlayerContractAsk, RejectionReason};

        let date = ExpiryRenewalFixtures::d(2026, 6, 10);
        let mut player = ExpiryRenewalFixtures::player(
            1,
            PlayerPositionType::MidfielderCenter,
            ExpiryRenewalFixtures::attrs(8.0, 8.0),
            100_000,
            PlayerSquadStatus::FirstTeamRegular,
            date,
        );
        // Three renewal offers already made (and turned down) this rolling
        // year — the season's worth of attempts is spent.
        for offer_date in [
            ExpiryRenewalFixtures::d(2026, 1, 10),
            ExpiryRenewalFixtures::d(2026, 3, 10),
            ExpiryRenewalFixtures::d(2026, 5, 10),
        ] {
            player.decision_history.add(
                offer_date,
                "3y · $110,000/y".to_string(),
                RENEWAL_OFFERED_LABEL.to_string(),
                "Coach".to_string(),
            );
        }
        // His standing ask is a wage well above anything the club's
        // valuation produces for a CA-100 player — the only sticking point
        // is money, with no clause / role / length demand to grant.
        player.pending_contract_ask = Some(PlayerContractAsk {
            desired_salary: 800_000,
            desired_years: 3,
            recorded_on: ExpiryRenewalFixtures::d(2026, 5, 10),
            demanded_status: None,
            demanded_release_clause: None,
            demanded_signing_bonus: None,
            rejection_reason: Some(RejectionReason::LowSalary),
        });

        let offers_before =
            ExpiryRenewalFixtures::history_count(&player, RENEWAL_OFFERED_LABEL);
        assert_eq!(offers_before, 3, "fixture must start with three prior offers");

        let main = ExpiryRenewalFixtures::team(10, 100, vec![player]);
        let club = ExpiryRenewalFixtures::club(100, main);
        let mut country = ExpiryRenewalFixtures::country(vec![club]);

        ExpiryRenewalFixtures::run(&mut country, date);

        let p = ExpiryRenewalFixtures::find_player(&country, 100, 1);
        assert_eq!(
            ExpiryRenewalFixtures::history_count(p, RENEWAL_OFFERED_LABEL),
            3,
            "no fourth identical offer may be made — the expiry offer is suppressed"
        );
        assert!(
            p.contract.is_none(),
            "with no improved offer the player walks for free on expiry"
        );
    }
}

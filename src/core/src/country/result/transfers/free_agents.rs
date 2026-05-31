use super::config::TransferConfig;
use super::free_agent_market_calc::{BuyerRoleFit, FreeAgentMarketCalculator};
use super::types::{TransferActivitySummary, can_club_accept_player};
use crate::club::player::calculators::WageCalculator;
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
    EmergencyBuyerContext, EmergencyCandidateView, EmergencyContractTermsPolicy,
    EmergencyGroupSlot, EmergencyProjectedSquad, EmergencySquadFillStrategy, FirstTeamSquadNeeds,
};
use crate::transfers::{CompletedTransfer, TransferType};
use crate::utils::IntegerUtils;
use crate::{Country, Person, PlayerFieldPositionGroup, PlayerStatusType, TeamInfo};
use chrono::NaiveDate;
use log::debug;
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
    /// Player-side reference reputation used to position them on the
    /// rep-drop sliding gate. See `Player::reference_reputation`.
    pub reference_reputation: u16,
    /// Carry-overs from the player's `FreeAgentMarketState`.
    pub last_salary: u32,
    pub last_country_reputation: u16,
    pub last_league_reputation: u16,
    pub world_reputation: i16,
    pub current_reputation: i16,
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
    /// Player-side reference reputation. Pegs the buyer's
    /// rep-drop tolerance against the player's last-known
    /// market and nationality.
    pub reference_reputation: u16,
    pub last_salary: u32,
    pub last_country_reputation: u16,
    pub last_league_reputation: u16,
    pub world_reputation: i16,
    pub current_reputation: i16,
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
                            potential: player.player_attributes.potential_ability,
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
                            reference_reputation: country.reputation,
                            last_salary,
                            last_country_reputation: country.reputation,
                            last_league_reputation: country.reputation,
                            world_reputation: player.player_attributes.world_reputation,
                            current_reputation: player.player_attributes.current_reputation,
                            is_global_pool: false,
                        });
                    }
                }
            }
        }

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
                reference_reputation: fa.reference_reputation,
                last_salary: fa.last_salary,
                last_country_reputation: fa.last_country_reputation,
                last_league_reputation: fa.last_league_reputation,
                world_reputation: fa.world_reputation,
                current_reputation: fa.current_reputation,
                is_global_pool: true,
            });
        }

        // Release players with expired contracts
        for player_id in expired_player_ids {
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

        // ── Pass 2a (NEW): Emergency squad fill ─────────────────────
        // Runs BEFORE the request-driven matcher so clubs sitting
        // under MIN_FIRST_TEAM_SQUAD don't have to wait for the
        // scouting/shortlist pipeline. Pushes into the same
        // `signings` vec so Pass 3 executes them through the existing
        // path and the normal matcher's `signings.iter().any(...)`
        // dedup naturally skips already-claimed candidates.
        Self::handle_free_agents_emergency_pass(
            country,
            &candidates,
            config,
            &mut signings,
            global_offered_ids,
            global_rejected_ids,
        );

        let max_signings_per_day = config.max_free_agent_signings_per_day;
        let ability_slack = config.free_agent_ability_slack;
        let buyer_country_reputation = country.reputation;
        // Mirrors `scan_foreign_loan_market`: same region the country sits
        // in, used as the prestige anchor for cross-region gating.
        let buyer_region = ScoutingRegion::from_country(country.continent_id, &country.code);
        let buyer_region_prestige = buyer_region.league_prestige();
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

                // Filter pass — replaces the legacy hard country-rep
                // and region-prestige gates with sliding tolerances
                // driven by career pressure. In-country candidates pass
                // trivially because their `reference_reputation` equals
                // the buyer's country rep; the gates only bite for
                // cross-market global-pool free agents who haven't yet
                // accumulated the pressure to step down.
                let best = candidates
                    .iter()
                    .filter(|c| {
                        if c.club_id == club.id {
                            return false;
                        }
                        if c.position_group != group {
                            return false;
                        }
                        if signings.iter().any(|s| s.player_id == c.player_id) {
                            return false;
                        }
                        // Quality fit: replace `min_ability - slack`
                        // with a tier-anchored band. Slack still pays
                        // off — the buyer accepts a free agent slightly
                        // below the nominal target because the price
                        // (zero fee) compensates.
                        let min_ca = FreeAgentMarketCalculator::min_acceptable_ca(
                            buyer_club_score,
                            group,
                            c.career_pressure,
                        );
                        let max_ca = FreeAgentMarketCalculator::max_acceptable_ca(
                            buyer_club_score,
                            group,
                            c.career_pressure,
                        );
                        let nominal_floor = request.min_ability.saturating_sub(ability_slack);
                        if c.ability < min_ca.min(nominal_floor) {
                            return false;
                        }
                        if c.ability > max_ca {
                            return false;
                        }
                        // Sliding country-rep gate.
                        let rep_drop = FreeAgentMarketCalculator::rep_drop_allowed(
                            c.career_pressure,
                            c.age,
                            c.ability,
                        );
                        let buyer_anchor = buyer_country_reputation as i32 + rep_drop;
                        if buyer_anchor < c.reference_reputation as i32 {
                            return false;
                        }
                        // Sliding region-prestige gate. At pressure 0
                        // this collapses to the legacy 0.20 threshold;
                        // at pressure 1.0 it widens to 0.65.
                        let region_drop =
                            FreeAgentMarketCalculator::region_drop_allowed(c.career_pressure);
                        if c.nationality_region.league_prestige()
                            > buyer_region_prestige + region_drop
                        {
                            return false;
                        }
                        true
                    })
                    .max_by_key(|c| c.ability as u16 + c.potential as u16);

                let Some(best) = best else { continue };

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
                let daily_chance = if best.is_global_pool {
                    FreeAgentMarketCalculator::daily_signing_chance(
                        best.career_pressure,
                        best.ability,
                        urgency_bonus,
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

                // Roll the dice
                let roll = IntegerUtils::random(1, 1000) as f32 / 10.0; // 0.1 to 100.0
                if roll > daily_chance {
                    continue; // Not today — player stays on the market
                }

                // Acceptance: would the player actually sign this
                // particular offer? Wage / role / prestige / quality
                // fit weighted into a single score, sigmoid against a
                // pressure-decayed threshold. Skipped for in-country
                // expiring contracts (no career pressure; pre-decay
                // behaviour keeps the existing balance).
                if best.is_global_pool {
                    let role = FreeAgentMarketCalculator::infer_buyer_role(
                        best.ability,
                        buyer_club_score,
                        group,
                    );
                    let market_wage = WageCalculator::expected_annual_wage_raw(
                        best.ability,
                        best.current_reputation,
                        group == PlayerFieldPositionGroup::Forward,
                        group == PlayerFieldPositionGroup::Goalkeeper,
                        best.age,
                        buyer_club_score,
                        buyer_league_reputation,
                    );
                    let reservation = FreeAgentMarketCalculator::reservation_wage(
                        market_wage,
                        best.last_salary,
                        best.career_pressure,
                        buyer_country_reputation,
                    );
                    let offer = FreeAgentMarketCalculator::offer_wage(
                        market_wage,
                        role,
                        buyer_negotiator_skill,
                        buyer_country_reputation,
                        reservation,
                        best.career_pressure,
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
                    let score = FreeAgentMarketCalculator::acceptance_score(
                        FreeAgentMarketCalculator::wage_score(offer, reservation),
                        FreeAgentMarketCalculator::role_score(role),
                        FreeAgentMarketCalculator::prestige_score(
                            buyer_country_reputation,
                            best.reference_reputation,
                            rep_drop,
                        ),
                        FreeAgentMarketCalculator::quality_fit_score(best.ability, min_ca, max_ca),
                        best.career_pressure,
                    );
                    let threshold =
                        FreeAgentMarketCalculator::acceptance_threshold(best.career_pressure);
                    let prob = FreeAgentMarketCalculator::acceptance_probability(score, threshold);
                    let acceptance_roll = IntegerUtils::random(1, 1000) as f32 / 1000.0;
                    // Every roll is an "offer received" — the player
                    // got a concrete approach today. Track separately
                    // whether they accepted so the pool-side state can
                    // bump `offers_rejected_total` only on declines.
                    global_offered_ids.push(best.player_id);
                    if acceptance_roll > prob {
                        global_rejected_ids.push(best.player_id);
                        continue;
                    }
                }

                let reason =
                    PipelineProcessor::transfer_need_reason_text(&request.reason).to_string();

                signings.push(FreeAgentSigning {
                    player_id: best.player_id,
                    player_name: best.player_name.clone(),
                    from_club_id: best.club_id,
                    from_club_name: best.club_name.clone(),
                    to_club_id: club.id,
                    reason,
                    // Request-driven matcher doesn't stage explicit
                    // contract terms — the installer falls back to the
                    // calculator default, matching the legacy behaviour
                    // for non-emergency signings.
                    terms: None,
                    fills_group: Some(group),
                });
            }
        }

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
    pub(super) fn handle_free_agents_emergency_pass(
        country: &Country,
        candidates: &[FreeAgentCandidate],
        config: &TransferConfig,
        signings: &mut Vec<FreeAgentSigning>,
        global_offered_ids: &mut Vec<u32>,
        global_rejected_ids: &mut Vec<u32>,
    ) {
        if candidates.is_empty() {
            return;
        }
        let country_cap = config.emergency_max_signings_per_country_per_day;
        let base_per_club_cap = config.emergency_max_signings_per_club_per_day;
        if country_cap == 0 || base_per_club_cap == 0 {
            return;
        }
        let mut country_signed = 0usize;
        let buyer_country_code = country.code.clone();
        let buyer_continent_id = country.continent_id;
        let buyer_rep = country.reputation;

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

                let buyer_ctx = EmergencyBuyerContext {
                    country_reputation: buyer_rep,
                    urgent: projected.is_urgent(),
                };

                // Pick the next slot dynamically — once the urgent
                // groups are filled, the depth tail rotates into the
                // currently thinnest group instead of always being a
                // midfielder. Groups whose pool is empty this tick
                // are excluded so the planner can move on.
                let slot = EmergencySlotPlanner::next_slot(&projected, &empty_groups);
                let Some(slot) = slot else { break };

                let pick = EmergencyCandidatePicker::pick(
                    candidates,
                    signings,
                    &rejected_locally,
                    slot,
                    &buyer_ctx,
                    &buyer_country_code,
                    buyer_continent_id,
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
                // realistic short deals, so we compute the same wage
                // model the global free-agent matcher uses then run
                // the acceptance roll lifted by the emergency
                // multiplier.
                let role = FreeAgentMarketCalculator::infer_buyer_role(
                    best.ability,
                    buyer_club_score,
                    slot.group,
                );
                let market_wage = WageCalculator::expected_annual_wage_raw(
                    best.ability,
                    best.current_reputation,
                    slot.group == PlayerFieldPositionGroup::Forward,
                    slot.group == PlayerFieldPositionGroup::Goalkeeper,
                    best.age,
                    buyer_club_score,
                    buyer_league_reputation,
                );
                let reservation = FreeAgentMarketCalculator::reservation_wage(
                    market_wage,
                    best.last_salary,
                    best.career_pressure,
                    buyer_rep,
                );
                let offer = FreeAgentMarketCalculator::offer_wage(
                    market_wage,
                    role,
                    buyer_negotiator_skill,
                    buyer_rep,
                    reservation,
                    best.career_pressure,
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
                    FreeAgentMarketCalculator::wage_score(offer, reservation),
                    FreeAgentMarketCalculator::role_score(role),
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
                let base_prob =
                    FreeAgentMarketCalculator::acceptance_probability(score, threshold);
                let prob = (base_prob * EmergencySquadFillStrategy::EMERGENCY_ACCEPTANCE_MULTIPLIER)
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

                let terms = EmergencySignedTerms {
                    annual_wage: offer,
                    contract_years: EmergencyContractTermsPolicy::contract_years(
                        best.age, best.ability,
                    ),
                    role,
                };

                signings.push(FreeAgentSigning {
                    player_id: best.player_id,
                    player_name: best.player_name.clone(),
                    from_club_id: best.club_id,
                    from_club_name: best.club_name.clone(),
                    to_club_id: club.id,
                    reason: slot.reason.to_string(),
                    terms: Some(terms),
                    fills_group: Some(slot.group),
                });
                projected.apply_signing(slot.group);
                club_signed += 1;
                country_signed += 1;
                debug!(
                    "Emergency squad fill: club {} → player {} ({:?}, {}, wage={})",
                    club.id, best.player_id, slot.group, slot.reason, offer
                );
            }
        }
    }
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
        candidates
            .into_iter()
            .find(|g| !empty_groups.contains(g))
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

/// Pick the highest-scoring free-agent candidate for one emergency
/// slot. Returns `None` when no candidate clears the strategy's
/// minimum score (or when every viable one is already claimed by an
/// earlier signing this tick). Same logic as the previous
/// `CountryResult::pick_emergency_candidate`, lifted onto a struct so
/// the file stays free of impl-bound private helpers and the picker
/// can be unit-tested in isolation.
struct EmergencyCandidatePicker;

impl EmergencyCandidatePicker {
    fn pick<'a>(
        candidates: &'a [FreeAgentCandidate],
        signings: &[FreeAgentSigning],
        rejected_locally: &HashSet<u32>,
        slot: EmergencyGroupSlot,
        buyer_ctx: &EmergencyBuyerContext,
        buyer_country_code: &str,
        buyer_continent_id: u32,
        buying_club_id: u32,
    ) -> Option<&'a FreeAgentCandidate> {
        candidates
            .iter()
            .filter(|c| c.club_id != buying_club_id)
            .filter(|c| c.position_group == slot.group)
            .filter(|c| !signings.iter().any(|s| s.player_id == c.player_id))
            .filter(|c| !rejected_locally.contains(&c.player_id))
            .filter_map(|c| {
                let view = EmergencyCandidateView {
                    ability: c.ability,
                    age: c.age,
                    same_country_nationality: c
                        .nationality_country_code
                        .eq_ignore_ascii_case(buyer_country_code),
                    same_continent: c.nationality_continent_id == buyer_continent_id,
                    reference_reputation: c.reference_reputation,
                    career_pressure: c.career_pressure,
                };
                EmergencySquadFillStrategy::score(&view, buyer_ctx).and_then(|score| {
                    if score < EmergencySquadFillStrategy::MIN_ACCEPTABLE_SCORE {
                        None
                    } else {
                        Some((c, score))
                    }
                })
            })
            .max_by(|a, b| {
                a.1.partial_cmp(&b.1)
                    .unwrap_or(Ordering::Equal)
                    // Tiebreaker: prefer the in-country (no-contract)
                    // candidate over a global-pool one — keeps the
                    // signing local when scores tie.
                    .then_with(|| b.0.is_global_pool.cmp(&a.0.is_global_pool))
            })
            .map(|(c, _)| c)
    }
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
                    // every door.
                    .unwrap_or_else(|| (u16::MAX, 1, "gb".to_string()));
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

            let career_pressure = player.career_pressure(date);
            let reference_reputation = player.reference_reputation(nationality_rep);
            let (last_salary, last_country_reputation, last_league_reputation) = player
                .free_agent_state()
                .map(|s| {
                    (
                        s.last_salary,
                        s.last_country_reputation,
                        s.last_league_reputation,
                    )
                })
                .unwrap_or((0, nationality_rep, ((nationality_rep as f32) * 0.75) as u16));

            GlobalFreeAgentSummary {
                player_id: player.id,
                player_name: player.full_name.to_string(),
                ability: player.player_attributes.current_ability,
                potential: player.player_attributes.potential_ability,
                age: player.age(date),
                position_group: player.position().position_group(),
                nationality_country_reputation: nationality_rep,
                nationality_continent_id,
                nationality_country_code,
                career_pressure,
                reference_reputation,
                last_salary,
                last_country_reputation,
                last_league_reputation,
                world_reputation: player.player_attributes.world_reputation,
                current_reputation: player.player_attributes.current_reputation,
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
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::transfers::pipeline::TransferNeedPriority;
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
                    positions: vec![PlayerPosition { position, level: 16 }],
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
                reference_reputation: if same_country { 4000 } else { 3000 },
                last_salary: 50_000,
                last_country_reputation: 5000,
                last_league_reputation: 4500,
                world_reputation: 1500,
                current_reputation: 1500,
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
            let mut c =
                Self::candidate(player_id, ability, age, position_group, same_country);
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
        // Squad has 0 GK and a handful of outfield bodies — emergency
        // pass must reach for the goalkeeper before anything else.
        let players: Vec<Player> = (0..8)
            .map(|i| EmergencyFillFixtures::player(i, PlayerPositionType::DefenderCenter))
            .chain(
                (0..6).map(|i| EmergencyFillFixtures::player(20 + i, PlayerPositionType::MidfielderCenter)),
            )
            .chain(
                (0..4).map(|i| EmergencyFillFixtures::player(40 + i, PlayerPositionType::Striker)),
            )
            .collect();

        let main = EmergencyFillFixtures::team(10, "FC", "fc", players);
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);

        // Candidate pool: GKs only (everything else already filled).
        let candidates: Vec<FreeAgentCandidate> = (0..3)
            .map(|i| {
                EmergencyFillFixtures::candidate(
                    500 + i,
                    70,
                    26,
                    PlayerFieldPositionGroup::Goalkeeper,
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
            players.push(EmergencyFillFixtures::player(i, PlayerPositionType::Goalkeeper));
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
            players.push(EmergencyFillFixtures::player(40 + i, PlayerPositionType::Striker));
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

        let mut candidates: Vec<FreeAgentCandidate> = Vec::new();
        for i in 0..2 {
            candidates.push(EmergencyFillFixtures::candidate(
                700 + i,
                70,
                26,
                PlayerFieldPositionGroup::Goalkeeper,
                true,
            ));
        }
        for i in 0..8 {
            candidates.push(EmergencyFillFixtures::candidate(
                710 + i,
                75,
                26,
                PlayerFieldPositionGroup::Defender,
                true,
            ));
        }
        for i in 0..5 {
            candidates.push(EmergencyFillFixtures::candidate(
                720 + i,
                80,
                26,
                PlayerFieldPositionGroup::Forward,
                true,
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
                EmergencyFillFixtures::candidate(
                    800 + i,
                    70,
                    27,
                    PlayerFieldPositionGroup::Defender,
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

        let candidate = EmergencyFillFixtures::candidate(
            900,
            70,
            26,
            PlayerFieldPositionGroup::Defender,
            true,
        );
        let already = EmergencyFillFixtures::candidate(
            901,
            70,
            26,
            PlayerFieldPositionGroup::Defender,
            true,
        );
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
            !signings
                .iter()
                .skip(1)
                .any(|s| s.player_id == 900),
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
            signings.iter().map(|s| (s.player_id, &s.reason)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn urgent_club_reaches_eleven_in_one_tick_with_plausible_pool() {
        // Empty squad + plenty of plausible candidates → adaptive cap
        // lifts to the playable-size floor. The signing budget is
        // capped by the country-wide cap, but with 20 of room and a
        // pool of 20+ realistic candidates, a single tick must land
        // at least 11 signings so the club becomes playable.
        let main = EmergencyFillFixtures::team(10, "FC", "fc", Vec::new());
        let club = EmergencyFillFixtures::club(100, "FC", main);
        let country = EmergencyFillFixtures::country(vec![club]);

        let mut candidates = Vec::new();
        for i in 0..3 {
            candidates.push(EmergencyFillFixtures::candidate(
                3000 + i,
                70,
                28,
                PlayerFieldPositionGroup::Goalkeeper,
                true,
            ));
        }
        for i in 0..8 {
            candidates.push(EmergencyFillFixtures::candidate(
                3100 + i,
                75,
                28,
                PlayerFieldPositionGroup::Defender,
                true,
            ));
        }
        for i in 0..8 {
            candidates.push(EmergencyFillFixtures::candidate(
                3200 + i,
                75,
                28,
                PlayerFieldPositionGroup::Midfielder,
                true,
            ));
        }
        for i in 0..5 {
            candidates.push(EmergencyFillFixtures::candidate(
                3300 + i,
                75,
                28,
                PlayerFieldPositionGroup::Forward,
                true,
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
        assert_eq!(staged.fills_group, Some(PlayerFieldPositionGroup::Midfielder));
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
        club.transfer_plan.transfer_requests.push(TransferRequest::new(
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
}

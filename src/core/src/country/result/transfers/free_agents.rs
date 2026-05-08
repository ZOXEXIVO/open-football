use super::config::TransferConfig;
use super::free_agent_market_calc::FreeAgentMarketCalculator;
use super::types::{TransferActivitySummary, can_club_accept_player};
use crate::club::player::calculators::WageCalculator;
use crate::country::result::CountryResult;
use crate::shared::{Currency, CurrencyValue};
use crate::simulator::SimulatorData;
use crate::transfers::negotiation::{NegotiationPhase, NegotiationStatus, TransferNegotiation};
use crate::transfers::offer::TransferOffer;
use crate::transfers::pipeline::{
    PipelineProcessor, TransferNeedReason, TransferRequest, TransferRequestStatus,
};
use crate::transfers::scouting_region::ScoutingRegion;
use crate::transfers::{CompletedTransfer, TransferType};
use crate::utils::IntegerUtils;
use crate::{Country, Person, PlayerFieldPositionGroup, PlayerStatusType, TeamInfo};
use chrono::NaiveDate;
use log::debug;
use std::collections::HashMap;

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
pub(crate) struct GlobalFreeAgentSummary {
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
pub(crate) struct GlobalFreeAgentSigning {
    pub player_id: u32,
    pub player_name: String,
    pub buying_country_id: u32,
    pub buying_club_id: u32,
    pub reason: String,
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
        #[allow(dead_code)]
        struct FreeAgentCandidate {
            player_id: u32,
            player_name: String,
            club_id: u32,
            club_name: String,
            ability: u8,
            potential: u8,
            age: u8,
            position_group: PlayerFieldPositionGroup,
            days_to_expiry: i64,
            /// Reputation of the country whose realism-gate the candidate
            /// is measured against. For in-country expiring contracts that's
            /// the country we're processing (passes the filter trivially).
            /// For global-pool free agents it's the player's nationality
            /// country reputation, captured in the snapshot.
            nationality_country_reputation: u16,
            /// Region of the player's nationality. Same gate the loan
            /// market and personal-terms negotiation use to block moves
            /// across a clear prestige drop (e.g. SouthAmerica→WestAfrica).
            nationality_region: ScoutingRegion,
            /// Career-pressure score (0..1). Drives every sliding gate
            /// in the new decay model. In-country expiring contracts
            /// have pressure = 0 — they're not on the market yet.
            career_pressure: f32,
            /// Player-side reference reputation. Pegs the buyer's
            /// rep-drop tolerance against the player's last-known
            /// market and nationality.
            reference_reputation: u16,
            last_salary: u32,
            last_country_reputation: u16,
            last_league_reputation: u16,
            world_reputation: i16,
            current_reputation: i16,
            /// True when the candidate sits in `data.free_agents` — the
            /// global pool. The country borrow can't mutate them, so
            /// any state updates land in `global_*_ids` and are applied
            /// outside the borrow.
            is_global_pool: bool,
        }

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

                        let last_salary =
                            player.contract.as_ref().map(|c| c.salary).unwrap_or(0);
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
        struct FreeAgentSigning {
            player_id: u32,
            player_name: String,
            from_club_id: u32,
            from_club_name: String,
            to_club_id: u32,
            reason: String,
        }

        let mut signings: Vec<FreeAgentSigning> = Vec::new();
        let max_signings_per_day = config.max_free_agent_signings_per_day;
        let ability_slack = config.free_agent_ability_slack;
        let buyer_country_reputation = country.reputation;
        // Mirrors `scan_foreign_loan_market`: same region the country sits
        // in, used as the prestige anchor for cross-region gating.
        let buyer_region = ScoutingRegion::from_country(country.continent_id, &country.code);
        let buyer_region_prestige = buyer_region.league_prestige();

        for club in &country.clubs {
            if signings.len() >= max_signings_per_day {
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
                if signings.len() >= max_signings_per_day {
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
                        let buyer_anchor =
                            buyer_country_reputation as i32 + rep_drop;
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
                global_signings.push(GlobalFreeAgentSigning {
                    player_id: signing.player_id,
                    player_name: signing.player_name,
                    buying_country_id: country_id,
                    buying_club_id: signing.to_club_id,
                    reason: signing.reason,
                });
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
            let deferred = super::types::DeferredTransfer {
                player_id: signing.player_id,
                selling_country_id: country.id,
                selling_club_id: signing.from_club_id,
                buying_country_id: country.id,
                buying_club_id: signing.to_club_id,
                fee: 0.0,
                is_loan: false,
                has_option_to_buy: false,
                agreed_annual_wage: None,
                buying_league_reputation,
                sell_on_percentage: None,
                loan_future_fee: None,
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
    // Pass 1 (immutable): resolve nationality info per unique country.
    // Two-stage resolve mirrors the rest of the transfer pipeline: an
    // active country (full `Country`) first, then the lighter
    // `country_info` map. Without the second stage, the gates fall
    // back to permissive defaults and an Argentinian free agent slips
    // through to a Mali buyer. Build a cache keyed by country_id so
    // the mutable pass below doesn't need a SimulatorData borrow.
    let mut nationality_cache: HashMap<u32, (u16, u32, String)> = HashMap::new();
    for player in &data.free_agents {
        if nationality_cache.contains_key(&player.country_id) {
            continue;
        }
        let resolved = data
            .country(player.country_id)
            .map(|c| (c.reputation, c.continent_id, c.code.clone()))
            .or_else(|| {
                data.country_info
                    .get(&player.country_id)
                    .map(|c| (c.reputation, c.continent_id, c.code.clone()))
            })
            // Truly unknown nationality: fail-closed on the rep gate
            // (`u16::MAX` blocks every buyer) and pin the region to
            // the most prestigious one so the prestige gate also
            // rejects, instead of opening every door.
            .unwrap_or_else(|| (u16::MAX, 1, "gb".to_string()));
        nationality_cache.insert(player.country_id, resolved);
    }

    // Pass 2 (mutable on the pool only): seed market state for any
    // free agent who arrived without it (database-only entries that
    // never came through `on_release`), then build the snapshot row.
    let mut summaries: Vec<GlobalFreeAgentSummary> = Vec::with_capacity(data.free_agents.len());
    for player in data.free_agents.iter_mut() {
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
            .map(|s| (s.last_salary, s.last_country_reputation, s.last_league_reputation))
            .unwrap_or((0, nationality_rep, ((nationality_rep as f32) * 0.75) as u16));

        summaries.push(GlobalFreeAgentSummary {
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
        });
    }
    summaries
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
    player.complete_free_agent_signing(
        &snapshot.to_info,
        date,
        signing.buying_club_id,
        snapshot.league_reputation,
        None,
    );

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

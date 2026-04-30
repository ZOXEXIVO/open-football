use super::config::TransferConfig;
use super::types::{TransferActivitySummary, can_club_accept_player};
use crate::country::result::CountryResult;
use crate::shared::{Currency, CurrencyValue};
use crate::simulator::SimulatorData;
use crate::transfers::negotiation::{NegotiationPhase, NegotiationStatus, TransferNegotiation};
use crate::transfers::offer::TransferOffer;
use crate::transfers::pipeline::{PipelineProcessor, TransferRequest, TransferRequestStatus};
use crate::transfers::scouting_region::ScoutingRegion;
use crate::transfers::{CompletedTransfer, TransferType};
use crate::utils::IntegerUtils;
use crate::{Country, Person, PlayerFieldPositionGroup, PlayerStatusType, TeamInfo};
use chrono::NaiveDate;
use log::debug;

/// Lightweight snapshot of a player in the global `sim.free_agents` pool.
/// Built before the per-country borrow so `handle_free_agents` can match
/// these players against club needs without holding a SimulatorData borrow.
///
/// Reputation and region fields mirror what `PlayerSummary` carries for
/// the regular scouting / loan pipelines, so the same realism gates
/// (country-rep + region-prestige) work uniformly here.
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

            for request in &unfulfilled {
                if signings.len() >= max_signings_per_day {
                    break;
                }

                // Find a matching free agent candidate. Realism gates
                // mirror the foreign-loan and personal-terms paths so all
                // three player-decision flows agree on what counts as a
                // plausible cross-country move:
                //  1. Country-reputation gate — buying country must be at
                //     or above the player's nationality country reputation.
                //  2. Region-prestige gate — player's home region can be at
                //     most `+0.20` more prestigious than the buyer's.
                //     Stops e.g. an Argentinian (SouthAmerica, 0.45) landing
                //     at a Mali club (WestAfrica, 0.20). Same threshold as
                //     `scan_foreign_loan_market`.
                if let Some(best) = candidates
                    .iter()
                    .filter(|c| {
                        c.club_id != club.id
                            && c.position_group == request.position.position_group()
                            && c.ability >= request.min_ability.saturating_sub(ability_slack)
                            && c.nationality_country_reputation <= buyer_country_reputation
                            && c.nationality_region.league_prestige()
                                <= buyer_region_prestige + 0.20
                            && !signings.iter().any(|s| s.player_id == c.player_id)
                    })
                    .max_by_key(|c| c.ability as u16 + c.potential as u16)
                {
                    // Probability tables, age penalties, and the young-potential
                    // boost all live in `TransferConfig` — see config.rs.
                    let daily_chance =
                        config.daily_signing_chance(best.ability, best.potential, best.age);

                    // Roll the dice
                    let roll = IntegerUtils::random(1, 1000) as f32 / 10.0; // 0.1 to 100.0
                    if roll > daily_chance {
                        continue; // Not today — player stays on the market
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
/// these players against club needs. Cheap clones (id/name/ability/etc.) —
/// no Player reference is held, so the simulator can mutate the pool while
/// signings are being decided.
pub(crate) fn snapshot_global_free_agents(
    data: &SimulatorData,
    date: NaiveDate,
) -> Vec<GlobalFreeAgentSummary> {
    data.free_agents
        .iter()
        .map(|p| {
            // Resolve nationality info in two stages: an active country
            // (full `Country`) first, then the lighter `country_info` map
            // that covers *every* country — including ones whose leagues
            // aren't simulated this save. Without the second stage, the
            // gates fall back to permissive defaults and an Argentinian
            // free agent slips through to a Mali buyer.
            let (nationality_rep, nationality_continent_id, nationality_country_code) = data
                .country(p.country_id)
                .map(|c| (c.reputation, c.continent_id, c.code.clone()))
                .or_else(|| {
                    data.country_info
                        .get(&p.country_id)
                        .map(|c| (c.reputation, c.continent_id, c.code.clone()))
                })
                // Truly unknown nationality: fail-closed on the rep gate
                // (`u16::MAX` blocks every buyer) and pin the region to
                // the most prestigious one so the prestige gate also
                // rejects, instead of opening every door.
                .unwrap_or_else(|| (u16::MAX, 1, "gb".to_string()));
            GlobalFreeAgentSummary {
                player_id: p.id,
                player_name: p.full_name.to_string(),
                ability: p.player_attributes.current_ability,
                potential: p.player_attributes.potential_ability,
                age: p.age(date),
                position_group: p.position().position_group(),
                nationality_country_reputation: nationality_rep,
                nationality_continent_id,
                nationality_country_code,
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

    debug!(
        "Free agent signing (global pool): player {} → club {} in country {}",
        signing.player_id, signing.buying_club_id, signing.buying_country_id
    );

    true
}

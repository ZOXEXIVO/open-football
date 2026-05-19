mod end_of_period;
mod preseason;
mod regulations;
mod reputation;
mod statistics;
pub mod transfers;

use crate::ai::PendingAiRequest;
use crate::country::result::transfers::DeferredTransferOps;
use crate::league::LeagueResult;
use crate::league::result::DeferredGlobalOps;
use crate::r#match::MatchResult;
use crate::simulator::SimulatorData;
use crate::{ClubResult, SimulationResult};

pub struct CountryResult {
    pub country_id: u32,
    pub leagues: Vec<LeagueResult>,
    pub clubs: Vec<ClubResult>,
    /// Batched AI requests collected from every club in this country
    /// during the parallel tick. Drained by the simulator before
    /// Phase B (`AiBatchProcessor::execute`).
    pub pending_ai_requests: Vec<PendingAiRequest>,
    /// MatchResults already fanned through `LeagueResult::process_local`
    /// during Phase A. Phase C just pushes them into the world
    /// `SimulationResult.match_results` — no further per-match work
    /// remains. The tuple keys are league_id, kept so callers can
    /// inspect provenance without re-resolving.
    pub processed_match_results: Vec<(u32, Vec<MatchResult>)>,
    /// Global mutations the parallel pass couldn't apply in place
    /// (sacked-staff free-agent admittances, cross-country club
    /// updates, …). Phase C drains them on `&mut SimulatorData`.
    pub deferred_global_ops: DeferredGlobalOps,
    /// Transfer-market cross-country tail. Phase A runs the per-country
    /// scouting / negotiation / shadow-report pipeline; this carries
    /// the deferred global writes (sweeps, free-agent state, transfer
    /// execution, foreign-negotiation kickoff) to Phase C.
    pub deferred_transfer_ops: Option<DeferredTransferOps>,
}

impl CountryResult {
    pub fn new(country_id: u32, leagues: Vec<LeagueResult>, mut clubs: Vec<ClubResult>) -> Self {
        let mut pending_ai_requests: Vec<PendingAiRequest> = Vec::new();
        for club in &mut clubs {
            if !club.pending_ai_requests.is_empty() {
                pending_ai_requests.append(&mut club.pending_ai_requests);
            }
        }
        CountryResult {
            country_id,
            leagues,
            clubs,
            pending_ai_requests,
            processed_match_results: Vec::new(),
            deferred_global_ops: DeferredGlobalOps::new(),
            deferred_transfer_ops: None,
        }
    }

    pub fn process(self, data: &mut SimulatorData, result: &mut SimulationResult) {
        let current_date = data.date.date();
        let country_id = self.get_country_id(data);

        // Country-local subphases (media coverage, country reputation,
        // end-of-period, international competitions, economic factors,
        // preseason) all moved into `Country::simulate` (Phase A) so
        // they parallelize across countries via the existing
        // `countries.par_iter_mut()`.

        // Phase 1: Drain match results that `LeagueResult::process_local`
        // already fanned through in Phase A. The per-match stat /
        // morale / discipline pipeline has run inside the parallel
        // pass; here we just publish the results into the world
        // SimulationResult so callers downstream of the simulator
        // (UI, match storage) see them.
        let any_new_season = self.leagues.iter().any(|l| l.new_season_started);

        for (_lid, match_results) in self.processed_match_results {
            for mr in match_results {
                result.match_results.push(mr);
            }
        }
        // Apply deferred global ops emitted by Phase A. Currently
        // these are sacked staff awaiting admittance to the global
        // free-agent pool and pending manager appointments —
        // populated by ClubResult's board path. Until that path is
        // also routed through CountryProcessCtx, this drain is a
        // no-op; left in place so the Phase-A ↔ Phase-C handshake
        // is wired end-to-end.
        for staff in self.deferred_global_ops.free_agent_staff {
            crate::club::staff::free_pool::admit_to_pool(
                &mut data.free_agent_staff,
                staff,
                current_date,
            );
        }
        for club_id in self.deferred_global_ops.pending_appointments {
            crate::club::board::manager_market::ManagerMarketTick::execute_appointment(
                data,
                club_id,
                current_date,
            );
        }
        if !self.deferred_global_ops.free_agent_players.is_empty() {
            // Academy-aged-out releases. The players already have
            // `contract = None` and `Frt` stamped — extend straight
            // onto the global pool and mark the player index dirty
            // so subsequent reads find them.
            data.free_agents
                .extend(self.deferred_global_ops.free_agent_players);
            data.dirty_player_index = true;
        }

        // Snapshot BEFORE loan returns: this ensures cross-country loan players
        // get their season stats recorded while they're still at the borrowing club.
        // The snapshot correctly handles is_loan flag from the player's contract.
        // Loan returns then move the player back — if both clubs are in the same
        // country, the snapshot already captured the loan entry correctly.
        //
        // The snapshot itself catches up on any seasons whose gate was
        // missed previously (per-country watermark
        // `Country::last_snapshotted_season_year`), so a year that
        // failed the `new_season_started` check is recovered the next
        // time any league does flip the gate.
        if any_new_season {
            Self::snapshot_player_season_statistics(data, self.country_id);
            Self::process_loan_returns(data, country_id, current_date);
            // Retirement already runs inside process_end_of_period above,
            // via Self::process_player_retirements when the season ends.

            // Apply per-country squad-registration rules — foreign-player
            // limits drop the weakest non-domestic surplus, and the
            // omitted players receive the SquadRegistrationOmitted
            // happiness event. Runs once on the season-start tick so
            // the registration is stable for the rest of the campaign.
            Self::enforce_squad_registration(data, self.country_id, current_date);
        }

        // Phase 2: Club result processing was driven from
        // `Country::simulate` (Phase A) via CountryProcessCtx. Nothing
        // to do here for the per-club mutations; placeholder shells
        // sit in self.clubs only to preserve the type signature.
        let _ = &self.clubs;

        // Regular loan return check for non-season-end days
        if !any_new_season {
            Self::process_loan_returns(data, country_id, current_date);
        }

        // Pre-season, international competitions, economic factors all
        // moved into `Country::simulate` (Phase A) so they parallelize
        // across countries — kept the local-only ones; transfer market
        // is the only "country-driven" phase still here because it
        // mutates `data.free_agents` and routes cross-country
        // negotiations.

        // Phase 4: Transfer Market — Phase A ran the per-country
        // pipeline (scouting, negotiations, listings, shadow reports).
        // Here we just apply the cross-country tail collected into
        // DeferredTransferOps: sweep shortlists for signed players,
        // mutate `data.free_agents`, execute deferred transfers,
        // kickoff foreign negotiations.
        if let Some(ops) = self.deferred_transfer_ops {
            Self::apply_deferred_transfer_ops(data, ops, current_date);
        }
    }

    fn get_country_id(&self, _data: &SimulatorData) -> u32 {
        self.country_id
    }
}

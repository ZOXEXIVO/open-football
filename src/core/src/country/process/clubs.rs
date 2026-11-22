use crate::club::academy::result::ClubAcademyResult;
use crate::club::{BoardResult, ClubFinanceResult, PlayerCollectionResult};
use crate::country::country::Country;
use crate::league::DeferredContractInteraction;
use crate::league::result::{
    ClubProcessCtx, CountryProcessCtx, DeferredGlobalOps, StagedClubOps, WorldSnapshot,
};
use crate::league::{League, LeagueCollection};
use crate::shared::CountryLookupIndex;
use crate::transfers::CompletedTransfer;
use crate::transfers::pipeline::approach::ApproachPass;
use crate::utils::PerformanceProfiler;
use crate::{Club, ClubResult, PlayerResult, SimulationResult};
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};
use rayon::prelude::IntoParallelRefMutIterator;

/// The per-club result fan-out: reconcile the lookup once, run every club
/// worker in parallel, then merge the staged operations serially in club
/// order.
pub struct CountryClubFanout;

impl CountryClubFanout {
    pub fn reconcile(country: &mut Country) -> CountryLookupIndex {
        let mut lookup = std::mem::take(&mut country.lookup);
        PerformanceProfiler::stage("country_lookup_reconcile", 3, || lookup.reconcile(country));
        lookup
    }

    pub fn restore(country: &mut Country, lookup: CountryLookupIndex) {
        country.lookup = lookup;
    }

    pub fn run(
        country: &mut Country,
        world: WorldSnapshot<'_>,
        mut clubs_results: Vec<ClubResult>,
        lookup: &CountryLookupIndex,
        deferred: &mut DeferredGlobalOps,
    ) -> Vec<ClubResult> {
        let mut academy_transfers = Vec::new();
        Self::collect_academy_transfers(&clubs_results, &mut academy_transfers);
        Self::release_academy_players(&mut clubs_results, deferred);
        let club_results_after = Self::placeholders(&clubs_results);

        let staged_parts = Self::process_parallel(country, world, clubs_results, lookup);
        Self::merge_staged(
            country,
            world,
            staged_parts,
            deferred,
            academy_transfers,
            lookup,
        );

        club_results_after
    }

    fn collect_academy_transfers(
        clubs_results: &[ClubResult],
        academy_transfers: &mut Vec<CompletedTransfer>,
    ) {
        for result in clubs_results {
            if !result.academy_transfers.is_empty() {
                academy_transfers.extend(result.academy_transfers.iter().cloned());
            }
        }
    }

    fn release_academy_players(clubs_results: &mut [ClubResult], deferred: &mut DeferredGlobalOps) {
        for result in clubs_results {
            if !result.academy_released_players.is_empty() {
                deferred
                    .free_agent_players
                    .extend(std::mem::take(&mut result.academy_released_players));
            }
        }
    }

    fn placeholders(clubs_results: &[ClubResult]) -> Vec<ClubResult> {
        clubs_results
            .iter()
            .map(|result| {
                ClubResult::new(
                    result.club_id,
                    ClubFinanceResult::new(),
                    Vec::new(),
                    BoardResult::new(),
                    ClubAcademyResult::new(PlayerCollectionResult::new(Vec::new())),
                )
            })
            .collect()
    }

    fn process_parallel(
        country: &mut Country,
        world: WorldSnapshot<'_>,
        clubs_results: Vec<ClubResult>,
        lookup: &CountryLookupIndex,
    ) -> Vec<(DeferredGlobalOps, StagedClubOps)> {
        let country_id = country.id;
        let market_strength = country.economic_factors.sponsorship_market_strength;
        let leagues_ref = &country.leagues;
        let cup_league_ref = country.domestic_cup.as_ref().map(|cup| &cup.league);
        let stage = PerformanceProfiler::stage_scope("country_club_results", 2)
            .labelled(|| country.name.clone());

        let staged_parts = country
            .clubs
            .par_iter_mut()
            .zip(clubs_results.into_par_iter())
            .enumerate()
            .map(|(club_idx, (club, result))| {
                assert_eq!(
                    club.id, result.club_id,
                    "Phase 1c: clubs/results order diverged"
                );
                CountryClubWorker {
                    country_id,
                    world,
                    market_strength,
                    leagues: leagues_ref,
                    cup_league: cup_league_ref,
                    lookup,
                }
                .process(club, club_idx as u16, result)
            })
            .collect();
        drop(stage);

        staged_parts
    }

    fn merge_staged(
        country: &mut Country,
        world: WorldSnapshot<'_>,
        staged_parts: Vec<(DeferredGlobalOps, StagedClubOps)>,
        deferred: &mut DeferredGlobalOps,
        academy_transfers: Vec<CompletedTransfer>,
        lookup: &CountryLookupIndex,
    ) {
        let merge = PerformanceProfiler::stage_scope("country_club_staged_merge", 3);
        let mut contract_interactions: Vec<DeferredContractInteraction> = Vec::new();
        for (worker_deferred, worker_staged) in staged_parts {
            deferred.merge(worker_deferred);
            for listing in worker_staged.market_listings {
                country.transfer_market.add_listing(listing);
            }
            for player_id in worker_staged.interest_clears {
                ApproachPass::clear_player_interest(country, player_id);
            }
            contract_interactions.extend(worker_staged.contract_interactions);
        }
        drop(merge);

        Self::replay_contract_interactions(country, world, contract_interactions, deferred, lookup);
        for transfer in academy_transfers {
            country.transfer_market.transfer_history.push(transfer);
        }
    }

    fn replay_contract_interactions(
        country: &mut Country,
        world: WorldSnapshot<'_>,
        interactions: Vec<DeferredContractInteraction>,
        deferred: &mut DeferredGlobalOps,
        lookup: &CountryLookupIndex,
    ) {
        for interaction in interactions {
            let mut player_result = PlayerResult::new(interaction.player_id);
            player_result.contract.no_contract = interaction.no_contract;
            player_result.contract.want_improve_contract = interaction.want_improve_contract;
            player_result.contract.want_extend_contract = interaction.want_extend_contract;
            let mut ctx_local = CountryProcessCtx {
                country,
                date: world.date,
                country_info_ref: world.country_info,
                indexes_ref: world.indexes,
                deferred,
                lookup: Some(lookup),
            };
            ClubResult::process_player_contract_interaction(
                &player_result,
                &mut ctx_local,
                interaction.employing_club_id,
            );
        }
    }
}

struct CountryClubWorker<'a, 'w> {
    country_id: u32,
    world: WorldSnapshot<'w>,
    market_strength: f32,
    leagues: &'a LeagueCollection,
    cup_league: Option<&'a League>,
    lookup: &'a CountryLookupIndex,
}

impl<'a, 'w> CountryClubWorker<'a, 'w> {
    fn process(
        self,
        club: &mut Club,
        club_idx: u16,
        result: ClubResult,
    ) -> (DeferredGlobalOps, StagedClubOps) {
        let mut worker_deferred = DeferredGlobalOps::new();
        let mut worker_staged = StagedClubOps::default();
        let mut ctx_local = ClubProcessCtx {
            club,
            club_idx,
            country_id: self.country_id,
            date: self.world.date,
            country_info_ref: self.world.country_info,
            indexes_ref: self.world.indexes,
            leagues_ref: self.leagues,
            cup_league_ref: self.cup_league,
            sponsorship_market_strength: self.market_strength,
            lookup: self.lookup,
            deferred: &mut worker_deferred,
            staged: &mut worker_staged,
        };
        result.process(&mut ctx_local, &mut SimulationResult::new());
        (worker_deferred, worker_staged)
    }
}

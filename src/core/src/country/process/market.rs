use crate::country::country::Country;
use crate::country::result::transfers::{DeferredTransferOps, TransferTick};
use crate::league::result::WorldSnapshot;
use crate::utils::PerformanceProfiler;
use chrono::NaiveDate;

/// The country-local transfer market pass and the cross-country operations
/// it defers to the serial drain.
pub struct CountryTransferMarketTick;

impl CountryTransferMarketTick {
    pub fn run(
        country: &mut Country,
        current_date: NaiveDate,
        world: WorldSnapshot<'_>,
    ) -> DeferredTransferOps {
        let country_name = country.name.clone();
        let stage = PerformanceProfiler::stage_scope("country_transfer_market", 2)
            .labelled(|| country_name);
        let ops = TransferTick::simulate_transfer_market_local(
            country,
            current_date,
            world.world_pool,
            world.global_free_agents,
            world.market_map,
            world.placement_reach,
        );
        drop(stage);
        ops
    }
}

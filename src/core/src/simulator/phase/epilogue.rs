use crate::config::SimulatorConfig;
use crate::utils::PerformanceProfiler;
use crate::world::SimulatorData;

/// Settling the world back down once every match this tick has been
/// played: release the international flags, move the newly contractless
/// into the pool, and refresh the derived structures that the tick's
/// movements invalidated.
pub struct Epilogue;

impl Epilogue {
    pub fn run(data: &mut SimulatorData, config: &SimulatorConfig) {
        let today = data.date.date();

        // AFTER all matches, continent and global alike — a tournament
        // final on the release date should be played before the squad's
        // flags are cleared.
        let phase = PerformanceProfiler::phase_scope("E1_national_release", 0);
        data.process_world_national_team_release();
        drop(phase);

        // Move any player whose contract was cleared this tick (positional
        // surplus, free-transfer release, contract expiry) off their old
        // team's roster and into the global free-agent pool, so the player
        // page header and contract panel agree.
        //
        // This tick's cross-country matching already read the free-agent
        // snapshot built before the matchday, so a player swept here first
        // becomes visible to OTHER countries' clubs next tick — a
        // deliberate one-tick latency, not same-tick global matching. His
        // own country released and could re-sign him within this tick's
        // matchday pass, which clears expired contracts inline.
        let phase = PerformanceProfiler::phase_scope("E2_sweep_released", 0);
        data.sweep_released_to_free_agents();
        drop(phase);

        // Only if a transfer actually moved a player between clubs today.
        // Walking the world every day is wasteful.
        let phase = PerformanceProfiler::phase_scope("E3_rebuild_indexes", 0);
        data.rebuild_indexes_if_dirty();
        drop(phase);

        // Catches players created today — youth intake, regens, new clubs
        // — within one tick.
        let phase = PerformanceProfiler::phase_scope("E4_seed_histories", 0);
        data.seed_missing_player_histories();
        drop(phase);

        // Cadence lives on the config (default: first of every month).
        // Cheap — a BTreeMap range walk over evicted dates only.
        let _phase = PerformanceProfiler::phase_scope("E5_match_store_trim", 0);
        if config.is_trim_day(today) {
            data.match_store.trim(today);
        }
    }
}

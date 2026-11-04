use crate::GameAppData;
use crate::I18nManager;
use crate::r#match::stores::MatchStore;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use core::FootballSimulator;
use core::MatchRuntime;
use core::SimulationResult;
use core::SimulatorData;
use log::{debug, error};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::runtime::Handle;
use tokio::sync::{RwLock, Semaphore};
use tokio::task::{JoinHandle, JoinSet, spawn_blocking};

#[derive(Deserialize)]
pub struct ProcessQuery {
    pub days: Option<u32>,
}

pub async fn game_process_action(
    State(state): State<GameAppData>,
    Query(query): Query<ProcessQuery>,
) -> impl IntoResponse {
    let days = query.days.unwrap_or(1);

    // If already processing, return immediately
    let process_guard = match Arc::clone(&state.process_lock).try_lock_owned() {
        Ok(guard) => guard,
        Err(_) => return StatusCode::OK,
    };

    // Reset cancel flag at start
    state.cancel_flag.store(false, Ordering::SeqCst);

    // Clone data under read lock (cheap Arc clone), then release lock immediately
    let data_arc = {
        let guard = state.data.read().await;
        Arc::clone(guard.as_ref().unwrap())
    };

    let run = ProcessingRun {
        handle: Handle::current(),
        data: Arc::clone(&state.data),
        i18n: Arc::clone(&state.i18n),
        cancel_flag: Arc::clone(&state.cancel_flag),
    };

    // Run CPU-bound simulation on the blocking thread pool so tokio worker
    // threads stay free to serve HTTP requests while processing runs.
    // Await completion so the client knows when processing is done.
    let join_result = spawn_blocking(move || {
        let _guard = process_guard;

        // Deep clone outside any lock (the shared slot still holds a
        // reference, so unwrap_or_clone always clones here).
        run.execute(Arc::unwrap_or_clone(data_arc), days);
    })
    .await;

    if let Err(err) = join_result {
        error!("game process task failed: {err}");
        return StatusCode::INTERNAL_SERVER_ERROR;
    }

    StatusCode::OK
}

/// A day's replays, baking while the next day simulates. At most one day is
/// in flight, so a long holiday never piles up recordings, and `finish`
/// comes before every publish and before the request returns, so nothing is
/// shown ahead of the files its matches point at.
#[derive(Default)]
struct ReplayBaking {
    in_flight: Option<JoinHandle<()>>,
}

impl ReplayBaking {
    fn hand_over(&mut self, handle: &Handle, bake: impl Future<Output = ()> + Send + 'static) {
        self.finish(handle);
        self.in_flight = Some(handle.spawn(bake));
    }

    fn finish(&mut self, handle: &Handle) {
        if let Some(bake) = self.in_flight.take()
            && let Err(err) = handle.block_on(bake)
        {
            error!("baking a day's replays failed: {err}");
        }
    }
}

/// One processing run behind `POST /api/game/process`: simulates an owned
/// deep copy of the world on a blocking thread and publishes snapshots
/// into the shared slot without stalling readers.
struct ProcessingRun {
    handle: Handle,
    data: Arc<RwLock<Option<Arc<SimulatorData>>>>,
    i18n: Arc<I18nManager>,
    cancel_flag: Arc<AtomicBool>,
}

impl ProcessingRun {
    /// Simulate `days` daily ticks, publishing progress once per simulated
    /// week and once at the end.
    fn execute(self, mut simulator_data: SimulatorData, days: u32) {
        let mut days_since_sync: u32 = 0;
        let mut baking = ReplayBaking::default();

        for _ in 0..days {
            // Check cancellation before each day
            if self.cancel_flag.load(Ordering::SeqCst) {
                break;
            }

            let result = self
                .handle
                .block_on(FootballSimulator::simulate(&mut simulator_data));
            if result.has_match_results() && MatchRuntime::recordings_mode() {
                baking.hand_over(&self.handle, Self::write_match_results(result));
            }

            // During multi-day runs (e.g. holiday), publish progress every
            // simulated week so the UI and readers observe intermediate state
            // instead of waiting for the whole range to finish.
            days_since_sync += 1;
            if days_since_sync >= 7 {
                days_since_sync = 0;
                baking.finish(&self.handle);
                simulator_data = self.publish_progress(simulator_data);
            }
        }

        baking.finish(&self.handle);
        self.cancel_flag.store(false, Ordering::SeqCst);
        self.publish_final(simulator_data);
    }

    /// Publish an intermediate snapshot and hand back an owned working copy.
    ///
    /// The working copy is swapped in first and the outgoing world freed
    /// before re-cloning, so at most two world copies exist at any moment:
    /// readers see fresh data right after the swap, the old world drops
    /// off-lock, then the published snapshot is deep-cloned back into an
    /// owned working copy for the remaining days.
    fn publish_progress(&self, world: SimulatorData) -> SimulatorData {
        self.i18n.set_date(world.date);
        let published = Arc::new(world);
        let previous = self.swap(Arc::clone(&published));
        Self::release(previous);
        Arc::unwrap_or_clone(published)
    }

    /// Publish the finished world; the outgoing world is freed here on the
    /// blocking thread, after the write lock is released.
    fn publish_final(&self, world: SimulatorData) {
        self.i18n.set_date(world.date);
        let previous = self.swap(Arc::new(world));
        Self::release(previous);
    }

    /// Free an outgoing world across the pool when this run holds the last
    /// handle to it. A reader still holding one frees it later, as before.
    fn release(previous: Option<Arc<SimulatorData>>) {
        if let Some(world) = previous.and_then(Arc::into_inner) {
            world.release();
        }
    }

    /// Swap `next` into the shared slot and hand the previous world back to
    /// the caller. The write-lock critical section is a pointer swap only:
    /// dropping the outgoing world deallocates the entire object graph
    /// (seconds on a full save) and must happen after the guard is released,
    /// otherwise every `data.read()` page handler blocks for the whole
    /// deallocation and the web app appears frozen.
    fn swap(&self, next: Arc<SimulatorData>) -> Option<Arc<SimulatorData>> {
        self.handle.block_on(async {
            let mut guard = self.data.write().await;
            guard.replace(next)
        })
    }

    async fn write_match_results(result: SimulationResult) {
        let now = Instant::now();

        let max_concurrent = MatchRuntime::store_max_threads();
        let semaphore = Arc::new(Semaphore::new(max_concurrent));

        let mut tasks = JoinSet::new();

        // Friendlies are stored too — see `Match::play` for why the exclusion
        // that used to sit here is gone. `MatchRuntime::recordings_mode()`,
        // checked by the caller, is now the only thing that decides whether a
        // match is written, and it decides it the same way for every match.
        for match_result in result.match_results {
            let permit = Arc::clone(&semaphore);
            tasks.spawn(async move {
                let _permit = permit.acquire().await.unwrap();
                MatchStore::store(match_result).await;
            });
        }

        tasks.join_all().await;

        let elapsed = now.elapsed();
        debug!("match results stored in {} ms", elapsed.as_millis());
    }
}

#[derive(Serialize)]
pub struct ProcessingStatus {
    pub processing: bool,
}

pub async fn game_processing_status_action(
    State(state): State<GameAppData>,
) -> Json<ProcessingStatus> {
    let processing = state.process_lock.try_lock().is_err();
    Json(ProcessingStatus { processing })
}

pub async fn game_cancel_action(State(state): State<GameAppData>) -> StatusCode {
    state.cancel_flag.store(true, Ordering::SeqCst);
    StatusCode::OK
}

#[cfg(test)]
mod tests {
    use super::ReplayBaking;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;
    use tokio::runtime::Runtime;

    fn day_of_baking(written: &Arc<AtomicU32>) -> impl Future<Output = ()> + Send + 'static {
        let written = Arc::clone(written);
        async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            written.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn a_new_day_waits_for_the_previous_days_replays() {
        let runtime = Runtime::new().unwrap();
        let written = Arc::new(AtomicU32::new(0));
        let mut baking = ReplayBaking::default();

        baking.hand_over(runtime.handle(), day_of_baking(&written));
        baking.hand_over(runtime.handle(), day_of_baking(&written));

        assert_eq!(written.load(Ordering::SeqCst), 1, "only one day may be in flight");
        baking.finish(runtime.handle());
        assert_eq!(written.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn finishing_leaves_nothing_unwritten() {
        let runtime = Runtime::new().unwrap();
        let written = Arc::new(AtomicU32::new(0));
        let mut baking = ReplayBaking::default();

        for _ in 0..3 {
            baking.hand_over(runtime.handle(), day_of_baking(&written));
        }
        baking.finish(runtime.handle());

        assert_eq!(written.load(Ordering::SeqCst), 3);
        baking.finish(runtime.handle());
    }
}

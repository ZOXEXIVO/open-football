use crate::GameAppData;
use crate::r#match::stores::MatchStore;
use crate::world::{ProcessOutcome, WorldSession};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use core::FootballSimulator;
use core::MatchRuntime;
use core::SimulationResult;
use log::{debug, error};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::sync::Arc;
use std::time::Instant;
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use tokio::task::{JoinHandle, JoinSet};

#[derive(Deserialize)]
pub struct ProcessQuery {
    pub days: Option<u32>,
}

pub struct GameProcess;

impl GameProcess {
    /// Answers at once when a run is already in progress; otherwise once the
    /// run's last world is published.
    pub async fn process(
        State(state): State<GameAppData>,
        Query(query): Query<ProcessQuery>,
    ) -> impl IntoResponse {
        let run = ProcessingRun {
            handle: Handle::current(),
            days: query.days.unwrap_or(1),
        };
        match state
            .world
            .process(move |session| run.execute(session))
            .await
        {
            ProcessOutcome::AlreadyRunning | ProcessOutcome::Finished => StatusCode::OK,
            ProcessOutcome::Failed => {
                error!("game process run failed");
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }
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

/// One processing run behind `POST /api/game/process`, executed by the world
/// writer against its session. It publishes every simulated week, and takes
/// the editor actions queued behind it at every day boundary.
struct ProcessingRun {
    handle: Handle,
    days: u32,
}

impl ProcessingRun {
    fn execute(self, session: &mut WorldSession<'_>) {
        let mut days_since_sync: u32 = 0;
        let mut baking = ReplayBaking::default();

        for _ in 0..self.days {
            // Edits come before the cancellation check, so a cancelled run
            // still applies the ones waiting for it.
            session.catch_up(|| baking.finish(&self.handle));
            if session.cancelled() {
                break;
            }

            let result = self
                .handle
                .block_on(FootballSimulator::simulate(session.world()));
            if result.has_match_results() && MatchRuntime::recordings_mode() {
                baking.hand_over(&self.handle, Self::write_match_results(result));
            }

            days_since_sync += 1;
            if days_since_sync >= 7 {
                days_since_sync = 0;
                baking.finish(&self.handle);
                session.publish();
            }
        }

        session.catch_up(|| baking.finish(&self.handle));
        baking.finish(&self.handle);
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

impl GameProcess {
    pub async fn status(State(state): State<GameAppData>) -> Json<ProcessingStatus> {
        Json(ProcessingStatus {
            processing: state.world.processing(),
        })
    }

    pub async fn cancel(State(state): State<GameAppData>) -> StatusCode {
        state.world.cancel();
        StatusCode::OK
    }
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

        assert_eq!(
            written.load(Ordering::SeqCst),
            1,
            "only one day may be in flight"
        );
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

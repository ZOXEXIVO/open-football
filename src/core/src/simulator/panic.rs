use std::any::Any;
use std::sync::atomic::{AtomicU64, Ordering};

/// Cumulative count of continent panics swallowed by the simulator. The
/// tick catches a panicking continent and substitutes an empty result so
/// the rest of the world keeps ticking — this counter exposes that silent
/// failure to operators and tests.
static PANICKED_CONTINENTS: AtomicU64 = AtomicU64::new(0);

/// Process-global accessor for the swallowed-continent-panic counter.
pub struct ContinentPanicMetrics;

impl ContinentPanicMetrics {
    /// Total continent panics swallowed since process start.
    pub fn total() -> u64 {
        PANICKED_CONTINENTS.load(Ordering::Relaxed)
    }

    /// Record one swallowed continent panic.
    pub fn record() {
        PANICKED_CONTINENTS.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a swallowed panic and log it against the continent it came
    /// from. `stage` names the half of the matchday that failed, so a
    /// build-side panic and a fan-out-side panic stay distinguishable in
    /// the log.
    pub fn swallow(payload: &(dyn Any + Send), stage: &str, continent_id: u32, name: &str) {
        Self::record();
        log::error!(
            "event=continent_{}_panic continent_id={} continent_name={:?} message={:?} tick_action=continue_with_empty_result",
            stage,
            continent_id,
            name,
            Self::message(payload)
        );
    }

    fn message(payload: &(dyn Any + Send)) -> &'static str {
        if let Some(s) = payload.downcast_ref::<&'static str>() {
            s
        } else if payload.downcast_ref::<String>().is_some() {
            "<String panic>"
        } else {
            "<non-string panic>"
        }
    }
}

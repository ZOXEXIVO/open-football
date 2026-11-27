use super::WorldSession;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::oneshot;

/// A processing run queued for the writer, executed against one session.
pub(super) struct WorldRun {
    pub(super) run: Box<dyn FnOnce(&mut WorldSession<'_>) + Send>,
    pub(super) claim: ProcessingClaim,
    pub(super) finished: oneshot::Sender<()>,
}

/// Holds the processing flag for one run. It is released when the run's
/// last world is published, or when the run is dropped for any other reason,
/// so the flag cannot outlive the run even if the requesting client leaves.
pub(super) struct ProcessingClaim {
    flag: Arc<AtomicBool>,
}

impl ProcessingClaim {
    pub(super) fn take(flag: &Arc<AtomicBool>) -> Option<Self> {
        flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| ProcessingClaim {
                flag: Arc::clone(flag),
            })
    }
}

impl Drop for ProcessingClaim {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}

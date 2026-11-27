mod edit;
mod run;
mod session;
mod store;
mod writer;

#[cfg(test)]
mod tests;

pub(crate) use session::WorldSession;

use self::edit::WorldEdit;
use self::run::{ProcessingClaim, WorldRun};
use self::store::WorldStore;
use self::writer::{WorldCommand, WorldWriter};
use crate::I18nManager;
use core::SimulatorData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use tokio::sync::oneshot;

/// The world the web serves. Requests read lock-free snapshots of it, and
/// every change goes through one writer thread, in the order it was asked for.
#[derive(Clone)]
pub struct World {
    store: Arc<WorldStore>,
    commands: Sender<WorldCommand>,
    processing: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

/// An editor action that panicked. The published world was left as it was.
#[derive(Debug)]
pub struct EditFailed;

pub(crate) enum ProcessOutcome {
    AlreadyRunning,
    Finished,
    Failed,
}

impl World {
    pub fn start(world: Option<SimulatorData>, i18n: Arc<I18nManager>) -> Self {
        let store = Arc::new(WorldStore::new(world));
        let cancel = Arc::new(AtomicBool::new(false));
        let (commands, inbox) = mpsc::channel();
        WorldWriter::new(Arc::clone(&store), i18n, Arc::clone(&cancel), inbox).spawn();
        World {
            store,
            commands,
            processing: Arc::new(AtomicBool::new(false)),
            cancel,
        }
    }

    /// The world a request reads for its whole duration, or `None` before
    /// one is loaded.
    pub fn snapshot(&self) -> Option<Arc<SimulatorData>> {
        self.store.snapshot()
    }

    /// Queues `edit` when called rather than when awaited, so edits apply in
    /// the order they are made. It resolves once a world containing the edit
    /// is published; `None` means no world is loaded.
    pub fn edit<R: Send + 'static>(
        &self,
        edit: impl FnOnce(&mut SimulatorData) -> R + Send + 'static,
    ) -> impl Future<Output = Result<Option<R>, EditFailed>> + Send + 'static {
        let (reply, answer) = oneshot::channel();
        // Only a writer that is gone refuses a command, and dropping it with
        // its reply makes `answer` resolve as failed.
        let _ = self
            .commands
            .send(WorldCommand::Edit(WorldEdit::new(edit, reply)));
        async move { answer.await.map_err(|_| EditFailed) }
    }

    /// Queues a processing run unless one is already in progress, and
    /// resolves once the run's last world is published.
    pub(crate) fn process(
        &self,
        run: impl FnOnce(&mut WorldSession<'_>) + Send + 'static,
    ) -> impl Future<Output = ProcessOutcome> + Send + 'static {
        let queued = ProcessingClaim::take(&self.processing).map(|claim| {
            self.cancel.store(false, Ordering::SeqCst);
            let (finished, done) = oneshot::channel();
            let _ = self.commands.send(WorldCommand::Run(WorldRun {
                run: Box::new(run),
                claim,
                finished,
            }));
            done
        });
        async move {
            match queued {
                None => ProcessOutcome::AlreadyRunning,
                Some(done) => match done.await {
                    Ok(()) => ProcessOutcome::Finished,
                    Err(_) => ProcessOutcome::Failed,
                },
            }
        }
    }

    pub fn processing(&self) -> bool {
        self.processing.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

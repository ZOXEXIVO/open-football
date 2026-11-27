use super::edit::WorldEdit;
use super::run::WorldRun;
use super::session::WorldSession;
use super::store::WorldStore;
use crate::I18nManager;
use core::SimulatorData;
use log::error;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

pub(super) enum WorldCommand {
    Edit(WorldEdit),
    Run(WorldRun),
}

/// The one thread that changes the world. It takes commands in the order
/// they were sent, and frees the worlds it supersedes once no request still
/// reads them, so no request ever pays for a deallocation.
pub(super) struct WorldWriter {
    store: Arc<WorldStore>,
    i18n: Arc<I18nManager>,
    cancel: Arc<AtomicBool>,
    inbox: Receiver<WorldCommand>,
    deferred: Option<WorldCommand>,
    retired: Vec<Arc<SimulatorData>>,
}

impl WorldWriter {
    /// A superseded world can be let go by its last reader at any moment,
    /// with no command to wake the writer; this is how often it looks.
    const RECHECK: Duration = Duration::from_millis(100);

    pub(super) fn new(
        store: Arc<WorldStore>,
        i18n: Arc<I18nManager>,
        cancel: Arc<AtomicBool>,
        inbox: Receiver<WorldCommand>,
    ) -> Self {
        WorldWriter {
            store,
            i18n,
            cancel,
            inbox,
            deferred: None,
            retired: Vec::new(),
        }
    }

    /// The writer runs until every handle to the world is gone.
    pub(super) fn spawn(self) {
        std::thread::Builder::new()
            .name("world-writer".to_string())
            .spawn(move || self.serve())
            .expect("the world writer thread starts");
    }

    fn serve(mut self) {
        loop {
            match self.next_command() {
                Ok(command) => self.execute(command),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.reclaim();
        }
    }

    fn next_command(&mut self) -> Result<WorldCommand, RecvTimeoutError> {
        if let Some(command) = self.deferred.take() {
            return Ok(command);
        }
        if self.retired.is_empty() {
            self.inbox
                .recv()
                .map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            self.inbox.recv_timeout(Self::RECHECK)
        }
    }

    /// The next queued command, if it is an edit. Anything else waits for the
    /// main loop, so no edit is ever applied ahead of a command sent before it.
    pub(super) fn next_edit(&mut self) -> Option<WorldEdit> {
        if self.deferred.is_some() {
            return None;
        }
        match self.inbox.try_recv() {
            Ok(WorldCommand::Edit(edit)) => Some(edit),
            Ok(command) => {
                self.deferred = Some(command);
                None
            }
            Err(_) => None,
        }
    }

    /// A command that panics loses its own working world and the answers it
    /// owed. Whatever it already published stays, and the writer carries on.
    fn execute(&mut self, command: WorldCommand) {
        let outcome = catch_unwind(AssertUnwindSafe(|| match command {
            WorldCommand::Edit(edit) => self.edit(edit),
            WorldCommand::Run(run) => self.run(run),
        }));
        if outcome.is_err() {
            error!("a change to the world panicked; it was not published");
        }
    }

    /// An idle edit shares its session, and the copy that opens it, with
    /// every edit already queued behind it.
    fn edit(&mut self, edit: WorldEdit) {
        let Some(mut session) = WorldSession::open(self) else {
            edit.apply(None).send();
            return;
        };
        session.apply(edit);
        session.apply_queued();
        session.close();
    }

    fn run(&mut self, run: WorldRun) {
        let WorldRun {
            run,
            claim,
            finished,
        } = run;
        let Some(mut session) = WorldSession::open(self) else {
            return;
        };
        run(&mut session);
        session.close();
        drop(claim);
        let _ = finished.send(());
    }

    pub(super) fn snapshot(&self) -> Option<Arc<SimulatorData>> {
        self.store.snapshot()
    }

    pub(super) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    pub(super) fn publish(&mut self, next: Arc<SimulatorData>) {
        self.i18n.set_date(next.date);
        self.retired.extend(self.store.publish(next));
        self.reclaim();
    }

    /// Frees every superseded world no request still reads. The release runs
    /// on rayon, so the writer goes straight on to the next day or edit.
    pub(super) fn reclaim(&mut self) {
        for world in std::mem::take(&mut self.retired) {
            match Arc::try_unwrap(world) {
                Ok(world) => rayon::spawn(move || world.release()),
                Err(world) => self.retired.push(world),
            }
        }
    }
}

use core::SimulatorData;
use tokio::sync::oneshot;

type ApplyEdit = Box<dyn FnOnce(Option<&mut SimulatorData>) -> EditAnswer + Send>;

/// An editor action queued for the writer. Applying it yields its answer,
/// which the writer sends only once a world containing the edit is published.
pub(super) struct WorldEdit {
    apply: ApplyEdit,
}

/// The reply to an applied edit, held back until its world is published.
pub(super) struct EditAnswer {
    send: Box<dyn FnOnce() + Send>,
}

impl WorldEdit {
    pub(super) fn new<R: Send + 'static>(
        edit: impl FnOnce(&mut SimulatorData) -> R + Send + 'static,
        reply: oneshot::Sender<Option<R>>,
    ) -> Self {
        WorldEdit {
            apply: Box::new(move |world: Option<&mut SimulatorData>| {
                let outcome = world.map(edit);
                EditAnswer {
                    // A handler that has gone away leaves the edit standing.
                    send: Box::new(move || {
                        let _ = reply.send(outcome);
                    }),
                }
            }),
        }
    }

    /// With no world loaded the edit is skipped, and it answers `None`.
    pub(super) fn apply(self, world: Option<&mut SimulatorData>) -> EditAnswer {
        (self.apply)(world)
    }
}

impl EditAnswer {
    pub(super) fn send(self) {
        (self.send)();
    }
}

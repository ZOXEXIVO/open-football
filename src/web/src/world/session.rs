use super::edit::{EditAnswer, WorldEdit};
use super::writer::WorldWriter;
use core::SimulatorData;
use std::sync::Arc;

/// One run of changes on the writer: a private working world copied from the
/// published one, and the answers owed to the edits applied to it.
pub(crate) struct WorldSession<'w> {
    world: SimulatorData,
    writer: &'w mut WorldWriter,
    answers: Vec<EditAnswer>,
}

impl<'w> WorldSession<'w> {
    /// `None` when no world has been published yet.
    pub(super) fn open(writer: &'w mut WorldWriter) -> Option<Self> {
        let published = writer.snapshot()?;
        let world = SimulatorData::clone(&published);
        Some(WorldSession {
            world,
            writer,
            answers: Vec::new(),
        })
    }

    pub(crate) fn world(&mut self) -> &mut SimulatorData {
        &mut self.world
    }

    pub(crate) fn cancelled(&self) -> bool {
        self.writer.cancelled()
    }

    /// Publishes a copy of the working world and answers the edits it holds.
    pub(crate) fn publish(&mut self) {
        self.writer.publish(Arc::new(self.world.clone()));
        for answer in self.answers.drain(..) {
            answer.send();
        }
    }

    /// Applies the edits queued behind a run and publishes them at once, and
    /// frees superseded worlds no request reads any more. `before_publish`
    /// runs first, and only when there is an edit to publish.
    pub(crate) fn catch_up(&mut self, before_publish: impl FnOnce()) {
        self.writer.reclaim();
        let Some(first) = self.writer.next_edit() else {
            return;
        };
        before_publish();
        self.apply(first);
        self.apply_queued();
        self.publish();
    }

    pub(super) fn apply(&mut self, edit: WorldEdit) {
        self.answers.push(edit.apply(Some(&mut self.world)));
    }

    pub(super) fn apply_queued(&mut self) {
        while let Some(edit) = self.writer.next_edit() {
            self.apply(edit);
        }
    }

    /// Publishes the working world by move and answers the edits it holds.
    pub(super) fn close(self) {
        self.writer.publish(Arc::new(self.world));
        for answer in self.answers {
            answer.send();
        }
    }
}

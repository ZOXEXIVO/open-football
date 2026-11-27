use arc_swap::ArcSwapOption;
use core::SimulatorData;
use std::sync::Arc;

/// The published world. Reading it takes no lock and hands out an owned
/// `Arc`, so a request keeps its world across awaits and a long render.
pub(super) struct WorldStore {
    published: ArcSwapOption<SimulatorData>,
}

impl WorldStore {
    pub(super) fn new(world: Option<SimulatorData>) -> Self {
        WorldStore {
            published: ArcSwapOption::new(world.map(Arc::new)),
        }
    }

    pub(super) fn snapshot(&self) -> Option<Arc<SimulatorData>> {
        self.published.load_full()
    }

    /// Swaps `next` in and returns the world it replaced, for the writer to
    /// free once no request still reads it.
    pub(super) fn publish(&self, next: Arc<SimulatorData>) -> Option<Arc<SimulatorData>> {
        self.published.swap(Some(next))
    }
}

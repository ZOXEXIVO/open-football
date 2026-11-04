use crate::world::SimulatorData;
use std::sync::OnceLock;

static OBSERVER: OnceLock<fn(&'static str, &SimulatorData)> = OnceLock::new();

/// A look at the world between the steps of the tick. The fingerprint
/// harness installs one to digest the world after each step, so a run that
/// stops reproducing names the step it diverged in.
pub struct PhaseObserver;

impl PhaseObserver {
    /// First call wins.
    pub fn install(observer: fn(&'static str, &SimulatorData)) {
        let _ = OBSERVER.set(observer);
    }

    pub(crate) fn notify(step: &'static str, data: &SimulatorData) {
        if let Some(observe) = OBSERVER.get() {
            observe(step, data);
        }
    }
}

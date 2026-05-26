//! Stoppage-time accounting: how many milliseconds each match incident
//! adds to the natural added time.
//!
//! Pure helpers; no engine state mutation.

/// Stoppage-time additions for various match incidents. All in
/// milliseconds; spec reference values:
///   goal: 30–55s
///   substitution: 25–40s
///   injury: 45–120s
#[derive(Debug, Clone, Copy)]
pub enum StoppageEvent {
    Goal,
    Substitution,
    InjuryShort,
    InjuryLong,
    TimeWastingFoul,
}

/// Stoppage-time accounting, grouped as associated functions.
pub struct StoppageTime;

impl StoppageTime {
    /// Milliseconds of stoppage time added by a single incident.
    pub fn for_event(event: StoppageEvent) -> u64 {
        match event {
            StoppageEvent::Goal => 42_000,
            StoppageEvent::Substitution => 32_000,
            StoppageEvent::InjuryShort => 60_000,
            StoppageEvent::InjuryLong => 105_000,
            StoppageEvent::TimeWastingFoul => 15_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stoppage_event_mapping() {
        assert!(
            StoppageTime::for_event(StoppageEvent::InjuryLong)
                > StoppageTime::for_event(StoppageEvent::InjuryShort)
        );
        assert!(
            StoppageTime::for_event(StoppageEvent::Substitution)
                < StoppageTime::for_event(StoppageEvent::InjuryShort)
        );
    }
}

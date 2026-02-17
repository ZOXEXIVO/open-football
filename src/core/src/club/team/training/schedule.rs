use chrono::{NaiveDateTime, NaiveTime};

#[derive(Debug)]
pub struct TrainingSchedule {
    pub morning_time: NaiveTime,
    pub evening_time: NaiveTime,
    pub is_default: bool,
}

impl TrainingSchedule {
    pub fn new(morning_time: NaiveTime, evening_time: NaiveTime) -> Self {
        TrainingSchedule {
            morning_time,
            evening_time,
            is_default: true,
        }
    }

    /// Check if training should happen on this day.
    /// The simulation advances in whole-day steps (time is always 00:00),
    /// so we simply return true — the weekly plan determines what sessions
    /// actually run on each weekday (rest days return empty sessions).
    pub fn is_time(&self, _date: NaiveDateTime) -> bool {
        true
    }
}

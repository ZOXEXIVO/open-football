//! **What the coach watches.** A moving read of the run of play: one
//! bucket per minute, the most recent kept, so every read covers the same
//! trailing span once it has been played and none restarts from nothing.
//!
//! The match loop hands [`CoachRead::note`] the side's running counters
//! on the coach's cadence; [`CoachRead::read`] sums the window into the
//! [`RollingTeamMetrics`] the coach and the bench reason from.

/// The read the coach and the bench reason from.
#[derive(Debug, Clone, Copy, Default)]
pub struct RollingTeamMetrics {
    pub xg_for_last_15: f32,
    pub xg_against_last_15: f32,
    pub shots_for_last_15: u16,
    pub deep_entries_for_last_15: u16,
    /// Share of the last ten minutes the ball spent in the opponent's third.
    pub field_tilt_last_10: f32,
    /// Share of the last ten minutes the side held the ball.
    pub possession_last_10: f32,
    pub dangerous_turnovers_last_10: u16,
    /// Successful pressures over pressures in the last ten minutes.
    pub press_success_rate_last_10: f32,
    /// Times the opposition passed or carried the ball past the side's last
    /// line, a minute, over the last ten.
    pub line_breaks_per_minute: f32,
}

/// A side's running counters, as the match loop reads them.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ReadCounters {
    pub xg_for: f32,
    pub xg_against: f32,
    pub shots_for: u32,
    pub pressures: u32,
    pub successful_pressures: u32,
    pub deep_entries: u32,
    pub dangerous_turnovers: u32,
    pub line_breaks_against: u32,
    pub possession_ticks: u32,
    pub tilt_ticks: u32,
    pub ticks: u32,
}

impl ReadCounters {
    fn since(&self, earlier: &ReadCounters) -> ReadCounters {
        ReadCounters {
            xg_for: (self.xg_for - earlier.xg_for).max(0.0),
            xg_against: (self.xg_against - earlier.xg_against).max(0.0),
            shots_for: self.shots_for.saturating_sub(earlier.shots_for),
            pressures: self.pressures.saturating_sub(earlier.pressures),
            successful_pressures: self
                .successful_pressures
                .saturating_sub(earlier.successful_pressures),
            deep_entries: self.deep_entries.saturating_sub(earlier.deep_entries),
            dangerous_turnovers: self
                .dangerous_turnovers
                .saturating_sub(earlier.dangerous_turnovers),
            line_breaks_against: self
                .line_breaks_against
                .saturating_sub(earlier.line_breaks_against),
            possession_ticks: self.possession_ticks.saturating_sub(earlier.possession_ticks),
            tilt_ticks: self.tilt_ticks.saturating_sub(earlier.tilt_ticks),
            ticks: self.ticks.saturating_sub(earlier.ticks),
        }
    }

    fn add(&mut self, other: &ReadCounters) {
        self.xg_for += other.xg_for;
        self.xg_against += other.xg_against;
        self.shots_for += other.shots_for;
        self.pressures += other.pressures;
        self.successful_pressures += other.successful_pressures;
        self.deep_entries += other.deep_entries;
        self.dangerous_turnovers += other.dangerous_turnovers;
        self.line_breaks_against += other.line_breaks_against;
        self.possession_ticks += other.possession_ticks;
        self.tilt_ticks += other.tilt_ticks;
        self.ticks += other.ticks;
    }
}

/// One bucket per minute of play, the newest the minute in progress.
#[derive(Debug, Clone, Default)]
pub struct CoachRead {
    buckets: [ReadCounters; Self::SLOTS],
    minute: u32,
    last: ReadCounters,
}

impl CoachRead {
    /// The minutes the chances, shots and entries are read over, and the
    /// minutes possession, territory, turnovers and the press are.
    pub const WINDOW: u32 = 15;
    pub const SHORT_WINDOW: u32 = 10;
    /// The window and the minute in progress.
    const SLOTS: usize = Self::WINDOW as usize + 1;

    /// Fold the counters as they stand at `minute` into that minute.
    pub fn note(&mut self, minute: u32, now: ReadCounters) {
        while self.minute < minute {
            self.minute += 1;
            self.buckets[self.minute as usize % Self::SLOTS] = ReadCounters::default();
        }
        let delta = now.since(&self.last);
        self.buckets[self.minute as usize % Self::SLOTS].add(&delta);
        self.last = now;
    }

    /// The read: the window before the minute in progress and the part of
    /// it played, or the play so far before the window has been played.
    pub fn read(&self) -> RollingTeamMetrics {
        let span = |minutes: u32| {
            let mut total = ReadCounters::default();
            for back in 0..=minutes.min(self.minute) {
                total.add(&self.buckets[(self.minute - back) as usize % Self::SLOTS]);
            }
            total
        };
        let long = span(Self::WINDOW);
        let short = span(Self::SHORT_WINDOW);
        let share = |part: u32| {
            if short.ticks == 0 {
                0.0
            } else {
                (part as f32 / short.ticks as f32).clamp(0.0, 1.0)
            }
        };
        let minutes = (short.ticks as f32 / 6_000.0).max(1.0);
        RollingTeamMetrics {
            xg_for_last_15: long.xg_for,
            xg_against_last_15: long.xg_against,
            shots_for_last_15: long.shots_for.min(u16::MAX as u32) as u16,
            deep_entries_for_last_15: long.deep_entries.min(u16::MAX as u32) as u16,
            field_tilt_last_10: share(short.tilt_ticks),
            possession_last_10: share(short.possession_ticks),
            dangerous_turnovers_last_10: short.dangerous_turnovers.min(u16::MAX as u32) as u16,
            press_success_rate_last_10: if short.pressures == 0 {
                0.5
            } else {
                short.successful_pressures as f32 / short.pressures as f32
            },
            line_breaks_per_minute: short.line_breaks_against as f32 / minutes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One xG a minute and half the ball, noted twice a minute.
    fn played_to(minutes: u32) -> CoachRead {
        let mut read = CoachRead::default();
        for half in 0..=(minutes * 2) {
            let minute = half / 2;
            let at = half as f32 / 2.0;
            read.note(
                minute,
                ReadCounters {
                    xg_for: at,
                    possession_ticks: (at * 3_000.0) as u32,
                    ticks: (at * 6_000.0) as u32,
                    ..ReadCounters::default()
                },
            );
        }
        read
    }

    #[test]
    fn each_read_covers_the_fifteen_minutes_before_it() {
        for minute in [16, 31] {
            let read = played_to(minute).read();
            assert!(
                (read.xg_for_last_15 - 15.0).abs() < 0.6,
                "{minute}': {}",
                read.xg_for_last_15
            );
        }
    }

    #[test]
    fn a_half_held_window_reads_one_half_at_every_moment() {
        for minute in [1, 4, 9, 16, 31, 70] {
            let read = played_to(minute).read();
            assert!(
                (read.possession_last_10 - 0.5).abs() < 1e-3,
                "{minute}': {}",
                read.possession_last_10
            );
        }
    }

    #[test]
    fn with_no_pressures_the_press_reads_neutral() {
        let mut read = CoachRead::default();
        read.note(3, ReadCounters::default());
        assert_eq!(read.read().press_success_rate_last_10, 0.5);
    }

    #[test]
    fn before_the_window_is_played_the_read_is_the_play_so_far() {
        let read = played_to(5).read();
        assert!((read.xg_for_last_15 - 5.0).abs() < 0.6, "{}", read.xg_for_last_15);
    }
}

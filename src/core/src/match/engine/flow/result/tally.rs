//! **The lines of a match report the per-player stat lines cannot add up
//! to**: restarts taken, offences and the cards they drew, injuries, and
//! how much of the match was football. Kept by the match context as it
//! happens and handed back on the result, so a batch of matches played in
//! parallel can be summed without any process-global counter.

use crate::r#match::engine::ball::ball::PassOriginRestart;
use crate::r#match::engine::player::events::players::FoulSource;
use crate::r#match::engine::player::injury::{InjuryCause, InjuryGrade};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeriodKind {
    FirstHalf,
    SecondHalf,
    ExtraTime,
}

impl PeriodKind {
    pub const COUNT: usize = 3;

    pub fn index(self) -> usize {
        match self {
            PeriodKind::FirstHalf => 0,
            PeriodKind::SecondHalf => 1,
            PeriodKind::ExtraTime => 2,
        }
    }
}

/// Why the ball is dead. The referee adds some of these back as stoppage
/// time and not others — see `RefereeProfile::add_back`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadTime {
    /// Out of play, waiting for the restart to be taken.
    Restart,
    Celebration,
    Substitution,
    /// A player down and being treated.
    Treatment,
    /// The referee taking a name.
    Booking,
    /// A side deliberately slowing its own restart.
    Delay,
}

impl DeadTime {
    pub const COUNT: usize = 6;
    pub const NAMES: [&'static str; Self::COUNT] = [
        "restart",
        "celebration",
        "substitution",
        "treatment",
        "booking",
        "delay",
    ];

    pub fn index(self) -> usize {
        match self {
            DeadTime::Restart => 0,
            DeadTime::Celebration => 1,
            DeadTime::Substitution => 2,
            DeadTime::Treatment => 3,
            DeadTime::Booking => 4,
            DeadTime::Delay => 5,
        }
    }
}

/// Where a tick of a timed period went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayingTime {
    Live,
    Dead(DeadTime),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OffenceTally {
    pub whistled: u16,
    pub yellows: u16,
    pub reds: u16,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchTally {
    /// Restarts actually taken, by [`PassOriginRestart::index`].
    pub restarts: [u16; PassOriginRestart::COUNT],
    /// By [`FoulSource::index`].
    pub offences: [OffenceTally; FoulSource::COUNT],
    /// `[cause][severity]`.
    pub injuries: [[u8; InjuryGrade::COUNT]; InjuryCause::COUNT],
    pub ball_in_play_ms: u64,
    /// By [`DeadTime::index`].
    pub dead_ms: [u64; DeadTime::COUNT],
    /// From the ball going dead to the restart being played, by
    /// [`PassOriginRestart::index`].
    #[serde(default)]
    pub restart_dead_ms: [u64; PassOriginRestart::COUNT],
    /// Time played past each period's length, by [`PeriodKind::index`];
    /// the two extra-time periods add into one slot.
    pub added_time_ms: [u64; PeriodKind::COUNT],
    /// Balls played to the runner a line had stepped up past: `[sprung,
    /// beaten]` — left offside, or played on by a man who was late.
    #[serde(default)]
    pub offside_traps: [u16; 2],
}

impl MatchTally {
    pub fn note_restart(&mut self, origin: PassOriginRestart) {
        self.restarts[origin.index()] += 1;
    }

    pub fn note_restart_wait(&mut self, origin: PassOriginRestart, ms: u64) {
        self.restart_dead_ms[origin.index()] += ms;
    }

    pub fn note_whistled(&mut self, source: FoulSource) {
        self.offences[source.index()].whistled += 1;
    }

    pub fn note_card(&mut self, source: FoulSource, red: bool) {
        let offence = &mut self.offences[source.index()];
        if red {
            offence.reds += 1;
        } else {
            offence.yellows += 1;
        }
    }

    pub fn note_injury(&mut self, cause: InjuryCause, severity: InjuryGrade) {
        self.injuries[cause.index()][severity.index()] += 1;
    }

    pub fn note_tick(&mut self, time: PlayingTime, ms: u64) {
        match time {
            PlayingTime::Live => self.ball_in_play_ms += ms,
            PlayingTime::Dead(why) => self.dead_ms[why.index()] += ms,
        }
    }

    pub fn note_offside_trap(&mut self, sprung: bool) {
        self.offside_traps[usize::from(!sprung)] += 1;
    }

    pub fn note_added_time(&mut self, period: PeriodKind, ms: u64) {
        self.added_time_ms[period.index()] += ms;
    }

    pub fn restarts_of(&self, origin: PassOriginRestart) -> u16 {
        self.restarts[origin.index()]
    }

    pub fn dead_total_ms(&self) -> u64 {
        self.dead_ms.iter().sum()
    }

    pub fn injury_count(&self) -> u32 {
        self.injuries.iter().flatten().map(|&n| n as u32).sum()
    }

    /// Injuries at least as bad as `grade`.
    pub fn injuries_from(&self, grade: InjuryGrade) -> u32 {
        self.injuries
            .iter()
            .flat_map(|by_grade| by_grade[grade.index()..].iter())
            .map(|&n| n as u32)
            .sum()
    }

    /// Fold another match's tally into this one — for batch totals.
    pub fn absorb(&mut self, other: &MatchTally) {
        for (mine, theirs) in self.restarts.iter_mut().zip(other.restarts.iter()) {
            *mine += theirs;
        }
        for (mine, theirs) in self.offences.iter_mut().zip(other.offences.iter()) {
            mine.whistled += theirs.whistled;
            mine.yellows += theirs.yellows;
            mine.reds += theirs.reds;
        }
        for (mine, theirs) in self.injuries.iter_mut().zip(other.injuries.iter()) {
            for (m, t) in mine.iter_mut().zip(theirs.iter()) {
                *m += t;
            }
        }
        self.ball_in_play_ms += other.ball_in_play_ms;
        for (mine, theirs) in self.dead_ms.iter_mut().zip(other.dead_ms.iter()) {
            *mine += theirs;
        }
        for (mine, theirs) in self
            .restart_dead_ms
            .iter_mut()
            .zip(other.restart_dead_ms.iter())
        {
            *mine += theirs;
        }
        for (mine, theirs) in self
            .added_time_ms
            .iter_mut()
            .zip(other.added_time_ms.iter())
        {
            *mine += theirs;
        }
        for (mine, theirs) in self
            .offside_traps
            .iter_mut()
            .zip(other.offside_traps.iter())
        {
            *mine += theirs;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offences_and_cards_are_kept_per_source() {
        let mut tally = MatchTally::default();
        tally.note_whistled(FoulSource::Tackle);
        tally.note_whistled(FoulSource::Tackle);
        tally.note_card(FoulSource::Tackle, false);
        tally.note_whistled(FoulSource::Holding);
        tally.note_card(FoulSource::Holding, true);

        let tackle = tally.offences[FoulSource::Tackle.index()];
        assert_eq!((tackle.whistled, tackle.yellows, tackle.reds), (2, 1, 0));
        let holding = tally.offences[FoulSource::Holding.index()];
        assert_eq!((holding.whistled, holding.yellows, holding.reds), (1, 0, 1));
    }

    #[test]
    fn absorbing_a_match_adds_every_line() {
        let mut one = MatchTally::default();
        one.note_restart(PassOriginRestart::Corner);
        one.note_tick(PlayingTime::Live, 10);
        one.note_tick(PlayingTime::Dead(DeadTime::Booking), 10);
        one.note_added_time(PeriodKind::SecondHalf, 240_000);
        one.note_injury(InjuryCause::Contact, InjuryGrade::Hurt);

        let mut total = MatchTally::default();
        total.absorb(&one);
        total.absorb(&one);

        assert_eq!(total.restarts_of(PassOriginRestart::Corner), 2);
        assert_eq!(total.ball_in_play_ms, 20);
        assert_eq!(total.dead_ms[DeadTime::Booking.index()], 20);
        assert_eq!(total.added_time_ms[PeriodKind::SecondHalf.index()], 480_000);
        assert_eq!(total.injury_count(), 2);
    }
}

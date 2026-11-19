//! **The lines of a match report the per-player stat lines cannot add up
//! to**: restarts taken, offences and the cards they drew, injuries, and
//! how much of the match was football. Kept by the match context as it
//! happens and handed back on the result, so a batch of matches played in
//! parallel can be summed without any process-global counter.

use crate::r#match::engine::ball::ball::PassOriginRestart;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::delivery_diag::DeliveryTally;
use crate::r#match::engine::player::events::players::FoulSource;
use crate::r#match::engine::player::injury::{InjuryCause, InjuryGrade};
#[cfg(feature = "match-logs")]
use crate::r#match::engine::result::team_census::TeamCensusTally;
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
    /// Summed over the outfield players of the match.
    #[cfg(feature = "match-logs")]
    #[serde(default)]
    pub outfield_motion: OutfieldMotionTally,
    /// Both keepers' time on the ground after a dive that left them
    /// without the ball.
    #[cfg(feature = "match-logs")]
    #[serde(default)]
    pub keeper_ground: KeeperGroundTally,
    /// Summed over the players who run the defender state machine.
    #[cfg(feature = "match-logs")]
    #[serde(default)]
    pub defender_motion: RoleMotionTally,
    /// …and over the players who run the midfielder one.
    #[cfg(feature = "match-logs")]
    #[serde(default)]
    pub midfielder_motion: RoleMotionTally,
    /// …and over the players who run the forward one.
    #[cfg(feature = "match-logs")]
    #[serde(default)]
    pub forward_motion: RoleMotionTally,
    /// How each side played, by game state. See `TeamCensus`.
    #[cfg(feature = "match-logs")]
    #[serde(default)]
    pub team_census: TeamCensusTally,
    /// Every cross from its strike to its end, and where corners,
    /// throw-ins and own goals came from. See `DeliveryCensus`.
    #[cfg(feature = "match-logs")]
    #[serde(default)]
    pub delivery: DeliveryTally,
}

/// A keeper's spells on the ground without the ball: how many, how many
/// engine ticks they lasted in all, and how far he travelled during them.
#[cfg(feature = "match-logs")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeeperGroundTally {
    pub spells: u64,
    pub ticks: u64,
    pub path_mm: u64,
}

/// How much the outfielders of a match moved and changed their minds, in
/// the whole-number units the tally's `Eq` needs. Only the dev harness reads
/// it, and only its build has the per-player trace it comes from.
#[cfg(feature = "match-logs")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutfieldMotionTally {
    pub state_changes: u64,
    pub path_mm: u64,
    pub ticks: u64,
}

/// One role's share of [`OutfieldMotionTally`], and how much of the live ball
/// its players spent at or below a walk (2 m/s over a quarter second).
#[cfg(feature = "match-logs")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleMotionTally {
    pub state_changes: u64,
    pub path_mm: u64,
    pub ticks: u64,
    pub live_ticks: u64,
    pub slow_live_ticks: u64,
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
        #[cfg(feature = "match-logs")]
        {
            self.outfield_motion.state_changes += other.outfield_motion.state_changes;
            self.outfield_motion.path_mm += other.outfield_motion.path_mm;
            self.outfield_motion.ticks += other.outfield_motion.ticks;
            self.keeper_ground.spells += other.keeper_ground.spells;
            self.keeper_ground.ticks += other.keeper_ground.ticks;
            self.keeper_ground.path_mm += other.keeper_ground.path_mm;
            self.defender_motion.state_changes += other.defender_motion.state_changes;
            self.defender_motion.path_mm += other.defender_motion.path_mm;
            self.defender_motion.ticks += other.defender_motion.ticks;
            self.defender_motion.live_ticks += other.defender_motion.live_ticks;
            self.defender_motion.slow_live_ticks += other.defender_motion.slow_live_ticks;
            self.midfielder_motion.state_changes += other.midfielder_motion.state_changes;
            self.midfielder_motion.path_mm += other.midfielder_motion.path_mm;
            self.midfielder_motion.ticks += other.midfielder_motion.ticks;
            self.midfielder_motion.live_ticks += other.midfielder_motion.live_ticks;
            self.midfielder_motion.slow_live_ticks += other.midfielder_motion.slow_live_ticks;
            self.forward_motion.state_changes += other.forward_motion.state_changes;
            self.forward_motion.path_mm += other.forward_motion.path_mm;
            self.forward_motion.ticks += other.forward_motion.ticks;
            self.forward_motion.live_ticks += other.forward_motion.live_ticks;
            self.forward_motion.slow_live_ticks += other.forward_motion.slow_live_ticks;
            self.team_census.absorb(&other.team_census);
            self.delivery.absorb(&other.delivery);
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

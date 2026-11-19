//! **How each side played, as the `match-logs` team census reads it**: the
//! game-state ledger, the style fingerprint and the possessions, per side,
//! in the whole-number units the tally's `Eq` needs. Filled by
//! [`TeamCensus`](crate::r#match::engine::teamplay::census::TeamCensus) and
//! summed by [`MatchTally::absorb`](super::tally::MatchTally::absorb).

use serde::{Deserialize, Serialize};

/// The score from one side's view, as the ledger keys it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerState {
    TrailingByTwo,
    TrailingByOne,
    Level,
    LeadingByOne,
    LeadingByTwo,
}

impl LedgerState {
    pub const COUNT: usize = 5;
    pub const ALL: [LedgerState; Self::COUNT] = [
        LedgerState::TrailingByTwo,
        LedgerState::TrailingByOne,
        LedgerState::Level,
        LedgerState::LeadingByOne,
        LedgerState::LeadingByTwo,
    ];

    pub fn from_goal_difference(difference: i32) -> Self {
        match difference {
            d if d <= -2 => LedgerState::TrailingByTwo,
            -1 => LedgerState::TrailingByOne,
            0 => LedgerState::Level,
            1 => LedgerState::LeadingByOne,
            _ => LedgerState::LeadingByTwo,
        }
    }

    pub fn index(self) -> usize {
        match self {
            LedgerState::TrailingByTwo => 0,
            LedgerState::TrailingByOne => 1,
            LedgerState::Level => 2,
            LedgerState::LeadingByOne => 3,
            LedgerState::LeadingByTwo => 4,
        }
    }

    /// Leading, level or trailing: 0, 1, 2.
    pub fn broad(self) -> usize {
        match self {
            LedgerState::LeadingByOne | LedgerState::LeadingByTwo => 0,
            LedgerState::Level => 1,
            LedgerState::TrailingByOne | LedgerState::TrailingByTwo => 2,
        }
    }
}

/// One side's play in one game state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateLedger {
    pub ticks: u64,
    /// Ticks either side held the ball.
    pub held_ticks: u64,
    pub possession_ticks: u64,
    /// Ticks the ball was in the opponent's half.
    pub territory_ticks: u64,
    pub shots: u32,
    pub xg_milli: u64,
    /// Opposition passes completed, and this side's tackles, interceptions
    /// and fouls, both counted while the ball was in the opponent's own 60%
    /// of the pitch.
    pub opposition_passes: u32,
    pub defensive_actions: u32,
    /// Samples taken without the ball, and over them the back line's height
    /// from its own goal and the block's length and width.
    pub block_samples: u32,
    pub line_height_mm: u64,
    pub block_length_mm: u64,
    pub block_width_mm: u64,
}

impl StateLedger {
    pub fn absorb(&mut self, other: &StateLedger) {
        self.ticks += other.ticks;
        self.held_ticks += other.held_ticks;
        self.possession_ticks += other.possession_ticks;
        self.territory_ticks += other.territory_ticks;
        self.shots += other.shots;
        self.xg_milli += other.xg_milli;
        self.opposition_passes += other.opposition_passes;
        self.defensive_actions += other.defensive_actions;
        self.block_samples += other.block_samples;
        self.line_height_mm += other.line_height_mm;
        self.block_length_mm += other.block_length_mm;
        self.block_width_mm += other.block_width_mm;
    }
}

/// How a possession ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PossessionEnd {
    Goal,
    Shot,
    LostInPlay,
    DeadBall,
    PeriodEnd,
}

impl PossessionEnd {
    pub const COUNT: usize = 5;
    pub const NAMES: [&'static str; Self::COUNT] =
        ["goal", "shot", "lost in play", "dead ball", "period end"];

    pub fn index(self) -> usize {
        match self {
            PossessionEnd::Goal => 0,
            PossessionEnd::Shot => 1,
            PossessionEnd::LostInPlay => 2,
            PossessionEnd::DeadBall => 3,
            PossessionEnd::PeriodEnd => 4,
        }
    }
}

/// One side's possessions over a match.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PossessionTally {
    pub possessions: u32,
    pub passes: u32,
    pub ticks: u64,
    /// Toward the opponent's goal, from where each possession started to
    /// where it ended.
    pub progress_mm: i64,
    /// By [`PossessionEnd::index`].
    pub ended: [u32; PossessionEnd::COUNT],
    /// Possessions of 0-1, 2-3, 4-6, 7-10 and 11+ passes.
    pub by_passes: [u32; 5],
}

impl PossessionTally {
    pub fn pass_bucket(passes: u32) -> usize {
        match passes {
            0..=1 => 0,
            2..=3 => 1,
            4..=6 => 2,
            7..=10 => 3,
            _ => 4,
        }
    }

    pub fn absorb(&mut self, other: &PossessionTally) {
        self.possessions += other.possessions;
        self.passes += other.passes;
        self.ticks += other.ticks;
        self.progress_mm += other.progress_mm;
        for (mine, theirs) in self.ended.iter_mut().zip(other.ended.iter()) {
            *mine += theirs;
        }
        for (mine, theirs) in self.by_passes.iter_mut().zip(other.by_passes.iter()) {
            *mine += theirs;
        }
    }
}

/// The parts of a side's style the ledger does not already hold.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StyleTally {
    /// The ball won in open play, and how far from its own goal each time.
    pub regains: u32,
    pub regain_height_mm: u64,
    /// Regains that reached a shot within 15 s.
    pub counter_attacks: u32,
    /// Samples taken with the ball, and the outfielders' width over them.
    pub width_samples: u32,
    pub width_in_possession_mm: u64,
}

impl StyleTally {
    pub fn absorb(&mut self, other: &StyleTally) {
        self.regains += other.regains;
        self.regain_height_mm += other.regain_height_mm;
        self.counter_attacks += other.counter_attacks;
        self.width_samples += other.width_samples;
        self.width_in_possession_mm += other.width_in_possession_mm;
    }
}

/// The team census of a match, home side first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamCensusTally {
    /// `[side][LedgerState::index]`.
    pub ledger: [[StateLedger; LedgerState::COUNT]; 2],
    pub style: [StyleTally; 2],
    pub possessions: [PossessionTally; 2],
}

impl TeamCensusTally {
    pub fn absorb(&mut self, other: &TeamCensusTally) {
        for (mine, theirs) in self.ledger.iter_mut().zip(other.ledger.iter()) {
            for (m, t) in mine.iter_mut().zip(theirs.iter()) {
                m.absorb(t);
            }
        }
        for (mine, theirs) in self.style.iter_mut().zip(other.style.iter()) {
            mine.absorb(theirs);
        }
        for (mine, theirs) in self.possessions.iter_mut().zip(other.possessions.iter()) {
            mine.absorb(theirs);
        }
    }

    /// Both sides' ledgers for one state, added together.
    pub fn pooled(&self, state: LedgerState) -> StateLedger {
        let mut total = StateLedger::default();
        for side in &self.ledger {
            total.absorb(&side[state.index()]);
        }
        total
    }
}

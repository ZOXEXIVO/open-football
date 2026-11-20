//! A training session reaches a player's attributes.

use super::ledger::GrowthSource;
use super::skills_array::SKILL_COUNT;
use super::work::SessionWork;
use crate::club::player::player::Player;
use chrono::NaiveDate;

impl Player {
    /// Progress one repetition of quality 1 gives an attribute, before his
    /// plasticity, absorption and headroom.
    const SESSION_YIELD: f32 = 0.55;

    /// A session he took part in. Training states what was practised and how
    /// well; what it does to his attributes is decided here.
    pub fn on_training_session(&mut self, work: &SessionWork, date: NaiveDate) {
        self.development.ledger.roll(date);
        self.development.layoff.on_work();
        let mut progress = [0.0f32; SKILL_COUNT];
        for (i, p) in progress.iter_mut().enumerate() {
            *p = work.reps_at(i) * work.quality() * Self::SESSION_YIELD;
        }
        self.grow(&progress, date, GrowthSource::Sessions);
    }
}

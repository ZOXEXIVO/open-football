use crate::PlayerSkills;
use chrono::{NaiveDate, NaiveDateTime};

#[derive(Debug)]
pub struct PlayerTrainingHistory {
    records: Vec<TrainingRecord>,
}

impl PlayerTrainingHistory {
    pub fn new() -> Self {
        PlayerTrainingHistory {
            records: Vec::new(),
        }
    }

    pub fn add_record(&mut self, record: TrainingRecord) {
        self.records.push(record);
    }

    pub fn get_latest_record(&self) -> Option<&TrainingRecord> {
        self.records.last()
    }

    pub fn weeks_since_last_training(&self, now: NaiveDate) -> u32 {
        let mut weeks_since_last_training: u32 = 0;

        if let Some(last_training_record) = self.records.last() {
            let duration = now.signed_duration_since(last_training_record.date.date());
            weeks_since_last_training = duration.num_weeks() as u32;
        }

        weeks_since_last_training
    }
}

#[derive(Debug)]
pub struct TrainingRecord {
    date: NaiveDateTime,
    _skills: PlayerSkills,
}
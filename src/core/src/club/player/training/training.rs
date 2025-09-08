use crate::training::result::PlayerTrainingResult;
use crate::{Player, Staff};
use chrono::NaiveDateTime;

#[derive(Debug)]
pub struct PlayerTraining {}

impl PlayerTraining {
    pub fn new() -> Self {
        PlayerTraining {}
    }

    pub fn train(player: &Player, coach: &Staff, now: NaiveDateTime) -> PlayerTrainingResult {
        let now = now.date();

        let mut result = PlayerTrainingResult::new(player.id);



        result
    }
}

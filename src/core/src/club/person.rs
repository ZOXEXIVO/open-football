use crate::shared::FullName;
use crate::utils::DateUtils;
use crate::Relations;
use chrono::NaiveDate;

pub trait Person {
    fn id(&self) -> u32;
    fn fullname(&self) -> &FullName;
    fn birthday(&self) -> NaiveDate;

    fn age(&self, now: NaiveDate) -> u8 {
        DateUtils::age(self.birthday(), now)
    }

    fn behaviour(&self) -> &PersonBehaviour;
    fn attributes(&self) -> &PersonAttributes;

    fn relations(&self) -> &Relations;
}

#[derive(Debug, Copy, Clone, Default)]
pub struct PersonAttributes {
    pub adaptability: f32,
    pub ambition: f32,
    pub controversy: f32,
    pub loyalty: f32,
    pub pressure: f32,
    pub professionalism: f32,
    pub sportsmanship: f32,
    pub temperament: f32,
}

#[derive(Debug)]
pub struct PersonBehaviour {
    pub state: PersonBehaviourState,
}

impl PersonBehaviour {
    pub fn default() -> Self {
        PersonBehaviour {
            state: PersonBehaviourState::Normal,
        }
    }

    pub fn try_increase(&mut self) {
        match self.state {
            PersonBehaviourState::Poor => {
                self.state = PersonBehaviourState::Normal;
            }
            PersonBehaviourState::Normal => {
                self.state = PersonBehaviourState::Good;
            }
            _ => {}
        }
    }

    pub fn is_poor(&self) -> bool {
        self.state == PersonBehaviourState::Poor
    }

    pub fn as_str(&self) -> &'static str {
        self.state.as_str()
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PersonBehaviourState {
    Poor,
    Normal,
    Good,
}

impl PersonBehaviourState {
    pub fn as_str(&self) -> &'static str {
        match self {
            PersonBehaviourState::Poor => "Poor",
            PersonBehaviourState::Normal => "Normal",
            PersonBehaviourState::Good => "Good",
        }
    }
}

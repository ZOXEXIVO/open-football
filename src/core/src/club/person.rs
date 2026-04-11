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

/// Hidden personality attributes, FM-style, all on a 0.0–20.0 scale.
/// These drive contract renewal, transfer acceptance, training progression,
/// big-match performance, discipline, and performance variance. They are
/// not shown in match events directly but modulate nearly every player-side
/// decision point in the simulator.
#[derive(Debug, Copy, Clone, Default)]
pub struct PersonAttributes {
    /// How quickly a player settles in a new club/country.
    pub adaptability: f32,
    /// Drives upward moves; resists downward moves.
    pub ambition: f32,
    /// Off-pitch flashpoints with teammates / media.
    pub controversy: f32,
    /// Resistance to leaving the club.
    pub loyalty: f32,
    /// How the player copes with big moments.
    pub pressure: f32,
    /// How hard the player trains — the #1 development driver.
    pub professionalism: f32,
    /// Fair-play bias — low = dives, shirt pulls, dark arts.
    pub sportsmanship: f32,
    /// Short fuse → more cards, fouls, red-card risk.
    pub temperament: f32,

    /// Match-to-match performance stability (1–20).
    /// High consistency = narrow rating variance; low = flaky.
    pub consistency: f32,
    /// Steps up (or shrinks) in cup finals, derbies, CL nights.
    pub important_matches: f32,
    /// Aggression on tackles / late challenges. Not the same as
    /// Temperament: a dirty player can be calm about it.
    pub dirtiness: f32,
}

#[derive(Debug, Clone, Default)]
pub struct PersonBehaviour {
    pub state: PersonBehaviourState,
}

impl PersonBehaviour {
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

    pub fn is_good(&self) -> bool {
        self.state == PersonBehaviourState::Good
    }

    pub fn as_str(&self) -> &'static str {
        self.state.as_str()
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Default)]
pub enum PersonBehaviourState {
    Poor,
    #[default]
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

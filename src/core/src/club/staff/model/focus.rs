#[derive(Debug, Clone)]
pub struct CoachFocus {
    pub technical_focus: Vec<TechnicalFocusType>,
    pub mental_focus: Vec<MentalFocusType>,
    pub physical_focus: Vec<PhysicalFocusType>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum TechnicalFocusType {
    Corners,
    Crossing,
    Dribbling,
    Finishing,
    FirstTouch,
    FreeKicks,
    Heading,
    LongShots,
    LongThrows,
    Marking,
    Passing,
    PenaltyTaking,
    Tackling,
    Technique,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum MentalFocusType {
    Aggression,
    Anticipation,
    Bravery,
    Composure,
    Concentration,
    Decisions,
    Determination,
    Flair,
    Leadership,
    OffTheBall,
    Positioning,
    Teamwork,
    Vision,
    WorkRate,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum PhysicalFocusType {
    Acceleration,
    Agility,
    Balance,
    Jumping,
    NaturalFitness,
    Pace,
    Stamina,
    Strength,
    MatchReadiness,
}

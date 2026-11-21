//! What the manager carries into a matchday from the goalkeeping room.
//!
//! The plan itself is a monthly document about four squads. The team sheet
//! needs four names and one number: who is the number one, who is the
//! deputy, who is third, who has the department asked him to play — and how
//! much notice he takes of the man asking.
//!
//! Keeping the matchday read this small matters. It is copied per fixture,
//! it must not borrow the club while the selector runs, and every magnitude
//! that can move a team sheet lives here in one place rather than scattered
//! through the scoring engine.

use chrono::NaiveDate;

use super::plan::KeeperRoomPlan;

/// The goalkeeping department's word, as the selector hears it.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeeperSelectionBrief {
    /// The declared number one.
    pub number_one: Option<u32>,
    /// The deputy — the man who goes in without the team dropping.
    pub deputy: Option<u32>,
    /// Third choice.
    pub third: Option<u32>,
    /// The keeper the department has asked the manager to start, if that
    /// request still stands today. Deliberately allowed to be a keeper who
    /// is not the best at the club.
    pub nominated: Option<u32>,
    /// 0..1 — how far the manager acts on the department's word.
    pub authority: f32,
    /// How settled the nominee was read as when the department asked.
    pub nominee_assurance: Option<f32>,
    /// The standard of football this fixture is expected to be played at.
    /// `None` until the matchday stamps it.
    pub fixture_standard: Option<f32>,
    /// The keeper the department wants kept out of the big nights.
    pub rested: Option<u32>,
    /// How far into the domestic cup the deputy keeps the gloves, when the
    /// club has declared the cup his: 1.0 is the final.
    pub deputy_cup_reach: Option<f32>,
}

/// What a fixture is, as far as the gloves are concerned.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KeeperFixture {
    /// What the fixture is worth to the side. Never the week: a crowded
    /// calendar does not rest a goalkeeper.
    pub stakes: f32,
    /// For a domestic cup tie, how far through the bracket it sits: 0 the
    /// first round, 1 the final.
    pub domestic_cup_progress: Option<f32>,
}

impl KeeperSelectionBrief {
    /// How much of the manager's ear and the fixture's room a request needs
    /// before he acts on it. A department he half-listens to gets its boy a
    /// game only in a fixture with nothing on it.
    const ACTS_ON: f32 = 0.5;

    /// Fixture importance at or below which the club can afford to hand a
    /// prospect the gloves, and the importance by which it can no longer
    /// afford it at all.
    const STAKES_FREE: f32 = 0.35;
    const STAKES_CLOSED: f32 = 0.62;

    /// A perceived step up of this much or more from the football the
    /// nominee is used to, and the fixture is no longer his: a debut is
    /// made where the step is small, not where the stakes merely are.
    const STEP_LIMIT: f32 = 0.08;

    /// Build the matchday read from the standing plan.
    pub fn from_plan(plan: &KeeperRoomPlan, today: NaiveDate) -> Self {
        KeeperSelectionBrief {
            number_one: plan.number_one(),
            deputy: plan.deputy(),
            third: plan.third(),
            nominated: plan.nominated(today),
            authority: plan.authority(),
            nominee_assurance: plan
                .nomination()
                .filter(|n| n.is_live(today))
                .and_then(|n| n.perceived_assurance),
            fixture_standard: None,
            rested: plan.rested(),
            deputy_cup_reach: plan.deputy_competition().map(|declared| declared.reach),
        }
    }

    /// The same word, for a fixture expected at `standard`.
    pub fn with_fixture_standard(mut self, standard: f32) -> Self {
        self.fixture_standard = Some(standard);
        self
    }

    /// How far this fixture sits within the nominee's reach: whole when
    /// the step up from the football he is used to is small, nothing once
    /// it is `STEP_LIMIT`. Whole when either side of the comparison is
    /// unknown.
    fn step_room(&self) -> f32 {
        match (self.fixture_standard, self.nominee_assurance) {
            (Some(fixture), Some(assured)) => {
                1.0 - ((fixture - assured) / Self::STEP_LIMIT).clamp(0.0, 1.0)
            }
            _ => 1.0,
        }
    }

    /// True when the department has nothing to say — the brief then adds
    /// nothing anywhere and selection behaves exactly as it did before the
    /// department existed.
    pub fn is_silent(&self) -> bool {
        self.number_one.is_none() && self.nominated.is_none()
    }

    /// How much room the fixture leaves for developing a keeper. Full at a
    /// dead rubber or an early cup tie, nothing once the result matters.
    fn stakes_room(match_importance: f32) -> f32 {
        if match_importance <= Self::STAKES_FREE {
            return 1.0;
        }
        if match_importance >= Self::STAKES_CLOSED {
            return 0.0;
        }
        1.0 - (match_importance - Self::STAKES_FREE) / (Self::STAKES_CLOSED - Self::STAKES_FREE)
    }

    /// The keeper the declared order names for a fixture, among those
    /// `eligible` for it. `None` when nobody in the order can play.
    ///
    /// The number one whenever he is eligible, except on a night that is
    /// the deputy's by declaration, a night the manager acts on the
    /// department's nomination, or a big night it wants a shaken number one
    /// kept out of. Nothing here weighs form, freshness or football owed.
    pub fn keeper_for(&self, fixture: &KeeperFixture, eligible: &[u32]) -> Option<u32> {
        let can = |id: Option<u32>| id.filter(|id| eligible.contains(id));
        let authority = self.authority.clamp(0.0, 1.0);

        if let Some(nominee) = can(self.nominated)
            && authority * Self::stakes_room(fixture.stakes) * self.step_room() >= Self::ACTS_ON
        {
            return Some(nominee);
        }

        let deputys_night = fixture
            .domestic_cup_progress
            .zip(self.deputy_cup_reach)
            .is_some_and(|(progress, reach)| progress <= reach + f32::EPSILON);
        let rested = self.rested.is_some()
            && self.rested == self.number_one
            && fixture.stakes >= Self::STAKES_CLOSED
            && authority >= Self::ACTS_ON;

        let order = if deputys_night || rested {
            [self.deputy, self.number_one, self.third]
        } else {
            [self.number_one, self.deputy, self.third]
        };
        order.into_iter().find_map(can)
    }

    /// The next keeper in the declared order among those `eligible`: the
    /// man who sits on the bench behind whoever was asked for.
    pub fn next_in_order(&self, eligible: &[u32]) -> Option<u32> {
        [self.number_one, self.deputy, self.third]
            .into_iter()
            .flatten()
            .find(|id| eligible.contains(id))
    }
}

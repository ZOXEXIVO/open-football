use super::model::CoachSelectionTendencies;
use crate::club::team::ClubWeek;

/// How far the football a plan owes may move a shirt in one fixture.
pub struct FixtureRoom;

impl FixtureRoom {
    /// A coach of ordinary rotation discipline uses the week's room whole;
    /// the most disciplined uses this much less of it, the least this much
    /// more.
    const DISCIPLINE_SPAN: f32 = 0.5;

    /// Nothing at a must-win, the most at a fixture with nothing on it, and
    /// more when a fixture that matters more to the club is close.
    pub fn of(stakes: f32, week: &ClubWeek, coach: &CoachSelectionTendencies) -> f32 {
        let stakes = stakes.clamp(0.0, 1.0);
        let ahead = week.nearness_of_next() * (week.next_importance - stakes).max(0.0);
        ((1.0 - stakes) + Self::use_of(coach) * ahead).clamp(0.0, 1.0)
    }

    /// The same fixture leaves a side that exists to develop players more
    /// room, and a decider none all the same.
    pub fn for_side(room: f32, development_weight: f32) -> f32 {
        room.clamp(0.0, 1.0)
            .powf(1.0 / (1.0 + development_weight.clamp(0.0, 1.0)))
    }

    fn use_of(coach: &CoachSelectionTendencies) -> f32 {
        1.0 - Self::DISCIPLINE_SPAN * (coach.rotation_discipline - 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn week(to_next: i64, next_importance: f32) -> ClubWeek {
        ClubWeek {
            since_last: Some(7),
            to_next: Some(to_next),
            next_importance,
        }
    }

    fn room(stakes: f32, week: &ClubWeek) -> f32 {
        FixtureRoom::of(stakes, week, &CoachSelectionTendencies::neutral())
    }

    #[test]
    fn a_development_side_is_left_more_room_by_an_ordinary_fixture_and_none_by_a_decider() {
        let ordinary = room(0.58, &ClubWeek::free());
        assert_eq!(FixtureRoom::for_side(ordinary, 0.0), ordinary);
        let b_team = FixtureRoom::for_side(ordinary, 0.45);
        let under_18 = FixtureRoom::for_side(ordinary, 1.0);
        assert!(ordinary < b_team && b_team < under_18 && under_18 < 1.0);
        assert_eq!(FixtureRoom::for_side(0.0, 1.0), 0.0);
        assert_eq!(FixtureRoom::for_side(1.0, 1.0), 1.0);
    }

    #[test]
    fn a_must_win_leaves_no_room_whatever_the_week() {
        assert_eq!(room(1.0, &ClubWeek::free()), 0.0);
        assert_eq!(room(1.0, &week(2, 1.0)), 0.0);
        assert_eq!(room(1.0, &week(3, 0.4)), 0.0);
    }

    #[test]
    fn room_falls_smoothly_as_stakes_rise() {
        let mut last = room(0.0, &ClubWeek::free());
        assert_eq!(last, 1.0);
        for step in 1..=100 {
            let now = room(step as f32 / 100.0, &ClubWeek::free());
            assert!(now < last && last - now < 0.011, "step {step}");
            last = now;
        }
    }

    #[test]
    fn room_rises_only_when_the_next_fixture_is_close_and_matters_more() {
        let alone = room(0.6, &ClubWeek::free());
        assert!(
            room(0.6, &week(3, 1.0)) > alone + 0.25,
            "a decider in three days"
        );
        assert_eq!(room(0.6, &week(7, 1.0)), alone, "a decider a week off");
        assert_eq!(
            room(0.6, &week(3, 0.6)),
            alone,
            "an equal fixture in three days"
        );
        assert_eq!(
            room(0.9, &week(3, 0.5)),
            room(0.9, &ClubWeek::free()),
            "a routine match after a decider"
        );
    }

    #[test]
    fn room_is_continuous_in_the_days_and_in_the_next_fixture() {
        for days in 2..7 {
            let step = room(0.6, &week(days, 1.0)) - room(0.6, &week(days + 1, 1.0));
            assert!((0.0..0.1).contains(&step), "day {days}: {step}");
        }
        for step in 0..40 {
            let next = 0.6 + step as f32 / 100.0;
            let moved = room(0.6, &week(3, next + 0.01)) - room(0.6, &week(3, next));
            assert!((0.0..0.011).contains(&moved));
        }
    }

    #[test]
    fn a_disciplined_coach_uses_less_of_the_week() {
        let mut steady = CoachSelectionTendencies::neutral();
        steady.rotation_discipline = 0.9;
        let mut restless = CoachSelectionTendencies::neutral();
        restless.rotation_discipline = 0.2;
        let ahead = week(3, 1.0);
        assert!(FixtureRoom::of(0.6, &ahead, &steady) < FixtureRoom::of(0.6, &ahead, &restless));
        assert_eq!(
            FixtureRoom::of(0.6, &ClubWeek::free(), &steady),
            FixtureRoom::of(0.6, &ClubWeek::free(), &restless),
            "nothing ahead, nothing to use"
        );
    }
}

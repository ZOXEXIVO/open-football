/// The calendar years a fixture list has football in, and where the one on
/// screen sits among them. Only years with matches are reachable, so
/// stepping never lands on an empty table.
pub struct YearStep {
    pub selected: i32,
    pub previous: Option<i32>,
    pub next: Option<i32>,
}

impl YearStep {
    /// `None` for a list with no matches at all — there is no year to name,
    /// so the page shows its empty state instead of a stepper.
    ///
    /// Landing view: the year asked for, else the season under way, else the
    /// last year with football in it. `current` is what keeps a fixture list
    /// that runs into next summer from opening on its own tail.
    pub fn resolve(
        years: impl IntoIterator<Item = i32>,
        requested: Option<i32>,
        current: i32,
    ) -> Option<Self> {
        let mut years: Vec<i32> = years.into_iter().collect();
        years.sort_unstable();
        years.dedup();

        let at = match requested
            .into_iter()
            .chain(std::iter::once(current))
            .find_map(|year| years.iter().position(|y| *y == year))
        {
            Some(at) => at,
            None => years.len().checked_sub(1)?,
        };

        Some(Self {
            selected: years[at],
            previous: at.checked_sub(1).map(|i| years[i]),
            next: years.get(at + 1).copied(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stepper_lands_on_the_season_under_way() {
        let step = YearStep::resolve([2024, 2026, 2026], None, 2026).unwrap();
        assert_eq!(step.selected, 2026);
        assert_eq!(step.previous, Some(2024));
        assert_eq!(step.next, None);
    }

    #[test]
    fn a_fixture_list_running_into_next_summer_does_not_open_on_its_tail() {
        // September 2026: the 2026/27 programme reaches May, so the newest
        // year in the list is 2027 — but the football being played is 2026's.
        let step = YearStep::resolve([2026, 2027], None, 2026).unwrap();
        assert_eq!(step.selected, 2026);
        assert_eq!(step.next, Some(2027));
    }

    #[test]
    fn stepping_skips_the_years_with_no_matches() {
        let step = YearStep::resolve([2022, 2026], Some(2022), 2026).unwrap();
        assert_eq!(step.previous, None);
        // 2023..2025 are silent, so one step forward is 2026 rather than a
        // walk through four empty tables.
        assert_eq!(step.next, Some(2026));
    }

    #[test]
    fn a_year_without_football_falls_back_to_the_newest_one() {
        // Retired in 2026, sim now in 2030: neither the asked-for year nor
        // the current one has football, so the last year played wins.
        let step = YearStep::resolve([2025, 2026], Some(1999), 2030).unwrap();
        assert_eq!(step.selected, 2026);
    }

    #[test]
    fn an_empty_list_has_no_year_to_name() {
        assert!(YearStep::resolve([], None, 2026).is_none());
    }
}

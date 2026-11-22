use core::league::season::LeagueSeason;

/// The seasons a fixture list has football in, and where the one on
/// screen sits among them. Only seasons with matches are reachable, so
/// stepping never lands on an empty table.
pub struct SeasonStep {
    pub selected: i32,
    pub label: String,
    pub previous: Option<i32>,
    pub next: Option<i32>,
}

impl SeasonStep {
    /// `None` for a list with no matches at all — there is no season to
    /// name, so the page shows its empty state instead of a stepper.
    ///
    /// Landing view: the season asked for, else the season under way when
    /// the caller knows one, else the last season with football in it.
    pub fn resolve(
        seasons: impl IntoIterator<Item = LeagueSeason>,
        requested: Option<i32>,
        current: Option<i32>,
    ) -> Option<Self> {
        let seasons: Vec<LeagueSeason> = seasons.into_iter().collect();
        let mut years: Vec<i32> = seasons.iter().map(|s| s.opening_year).collect();
        years.sort_unstable();
        years.dedup();

        let at = match requested
            .into_iter()
            .chain(current)
            .find_map(|year| years.iter().position(|y| *y == year))
        {
            Some(at) => at,
            None => years.len().checked_sub(1)?,
        };
        let selected = years[at];

        // A player who moved from a calendar-year league into one that
        // crosses the new year has both in one stop; the stop spans the
        // longer campaign, so it takes that campaign's name.
        let label = LeagueSeason {
            opening_year: selected,
            crosses_new_year: seasons
                .iter()
                .any(|s| s.opening_year == selected && s.crosses_new_year),
        }
        .label();

        Some(Self {
            selected,
            label,
            previous: at.checked_sub(1).map(|i| years[i]),
            next: years.get(at + 1).copied(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crossing(opening_year: i32) -> LeagueSeason {
        LeagueSeason {
            opening_year,
            crosses_new_year: true,
        }
    }

    fn calendar_year(opening_year: i32) -> LeagueSeason {
        LeagueSeason {
            opening_year,
            crosses_new_year: false,
        }
    }

    #[test]
    fn the_stepper_lands_on_the_season_under_way() {
        let step =
            SeasonStep::resolve([crossing(2024), crossing(2026), crossing(2026)], None, Some(2026))
                .unwrap();
        assert_eq!(step.selected, 2026);
        assert_eq!(step.label, "2026/27");
        assert_eq!(step.previous, Some(2024));
        assert_eq!(step.next, None);
    }

    #[test]
    fn the_season_under_way_wins_over_a_later_one_with_fixtures() {
        let step =
            SeasonStep::resolve([crossing(2026), crossing(2027)], None, Some(2026)).unwrap();
        assert_eq!(step.selected, 2026);
        assert_eq!(step.next, Some(2027));
    }

    #[test]
    fn stepping_skips_the_seasons_with_no_matches() {
        let step =
            SeasonStep::resolve([crossing(2022), crossing(2026)], Some(2022), Some(2026)).unwrap();
        assert_eq!(step.previous, None);
        // 2023/24..2025/26 are silent, so one step forward is 2026/27 rather
        // than a walk through three empty tables.
        assert_eq!(step.next, Some(2026));
    }

    #[test]
    fn a_season_without_football_falls_back_to_the_newest_one() {
        // Retired in 2026/27, sim now in 2030: neither the asked-for season
        // nor the current one has football, so the last season played wins.
        let step =
            SeasonStep::resolve([crossing(2025), crossing(2026)], Some(1999), Some(2030)).unwrap();
        assert_eq!(step.selected, 2026);
    }

    #[test]
    fn without_a_season_under_way_the_newest_season_is_shown() {
        let step = SeasonStep::resolve([crossing(2024), crossing(2025)], None, None).unwrap();
        assert_eq!(step.selected, 2025);
        assert_eq!(step.previous, Some(2024));
    }

    #[test]
    fn a_stop_mixing_calendars_is_named_after_the_campaign_across_the_new_year() {
        let seasons = [calendar_year(2025), calendar_year(2026), crossing(2026)];
        let newest = SeasonStep::resolve(seasons, None, None).unwrap();
        assert_eq!(newest.label, "2026/27");
        let earlier = SeasonStep::resolve(seasons, Some(2025), None).unwrap();
        assert_eq!(earlier.label, "2025");
    }

    #[test]
    fn a_calendar_year_stop_is_named_by_its_year() {
        let step = SeasonStep::resolve([calendar_year(2026)], None, None).unwrap();
        assert_eq!(step.label, "2026");
    }

    #[test]
    fn an_empty_list_has_no_season_to_name() {
        assert!(SeasonStep::resolve([], None, Some(2026)).is_none());
    }
}

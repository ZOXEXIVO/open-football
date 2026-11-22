use crate::league::{LeagueSettings, LeagueTable};

/// A boundary between two places of a table that a season turns on: the
/// title, a continental band's last place, promotion, a play-off place,
/// relegation, a group's qualifying place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableLine {
    /// The line runs below this many places.
    pub places_above: usize,
    /// How much crossing it is worth, 0..1.
    pub weight: f32,
}

impl TableLine {
    pub const TITLE: f32 = 1.0;
    pub const PROMOTION: f32 = 1.0;
    pub const RELEGATION: f32 = 1.0;
    pub const PLAY_OFF: f32 = 0.8;
    /// A lower division's title is worth what the trophy is; going up is
    /// its promotion line.
    pub const LOWER_TITLE: f32 = 0.5;

    pub fn below(places_above: usize, weight: f32) -> Self {
        TableLine {
            places_above,
            weight,
        }
    }

    /// The lines a league's own rules draw across a table of `teams`.
    pub fn of_league(settings: &LeagueSettings, teams: usize) -> Vec<TableLine> {
        let mut lines = Vec::with_capacity(4);
        lines.push(Self::below(
            1,
            if settings.tier == 1 {
                Self::TITLE
            } else {
                Self::LOWER_TITLE
            },
        ));
        if settings.promotion_spots > 0 {
            lines.push(Self::below(
                settings.promotion_spots as usize,
                Self::PROMOTION,
            ));
        }
        let play_off = settings
            .league_group
            .as_ref()
            .and_then(|group| group.playoff.as_ref())
            .map(|playoff| playoff.qualifiers_per_group as usize);
        if let Some(places) = play_off {
            lines.push(Self::below(places, Self::PLAY_OFF));
        }
        if settings.relegation_spots > 0 {
            lines.push(Self::below(
                teams.saturating_sub(settings.relegation_spots as usize),
                Self::RELEGATION,
            ));
        }
        lines
    }
}

/// One side's row among the others, in table order.
#[derive(Debug, Clone, Copy)]
pub struct TableStanding {
    pub team_id: u32,
    pub points: i32,
}

impl TableStanding {
    pub fn of_league(table: &LeagueTable) -> Vec<TableStanding> {
        table
            .rows
            .iter()
            .map(|row| TableStanding {
                team_id: row.team_id,
                points: i32::from(row.effective_points()),
            })
            .collect()
    }
}

/// How much one result can still change for a side in a competition with a
/// table.
pub struct TableStakes;

impl TableStakes {
    /// A competitive fixture with nothing on it: the side rotates, and does
    /// not field a development team.
    pub const PROFESSIONAL_FLOOR: f32 = 0.45;
    /// A fixture with every line still in reach and the season young.
    pub const ROUTINE: f32 = 0.66;
    const POINTS_FOR_A_WIN: f32 = 3.0;

    /// `standings` in table order; `left` and `played` are the side's own
    /// matches in the competition.
    pub fn of(
        standings: &[TableStanding],
        team_id: u32,
        played: u16,
        left: u16,
        lines: &[TableLine],
    ) -> f32 {
        let Some(place) = standings.iter().position(|s| s.team_id == team_id) else {
            return Self::ROUTINE;
        };
        if left == 0 {
            return Self::PROFESSIONAL_FLOOR;
        }
        let reach = f32::from(left) * Self::POINTS_FOR_A_WIN;
        let own = standings[place].points;
        let contention = lines
            .iter()
            .filter(|line| line.places_above > 0 && line.places_above < standings.len())
            .map(|line| {
                // The nearest side across the line: the first below it for a
                // side above, the last above it for a side below.
                let across = if place < line.places_above {
                    own - standings[line.places_above].points
                } else {
                    standings[line.places_above - 1].points - own
                };
                line.weight * (1.0 - across.max(0) as f32 / reach).clamp(0.0, 1.0)
            })
            .fold(0.0_f32, f32::max);
        let late = Self::decisiveness(played, left);
        Self::PROFESSIONAL_FLOOR
            + (Self::ROUTINE - Self::PROFESSIONAL_FLOOR) * contention
            + (1.0 - Self::ROUTINE) * contention * late
    }

    /// How decisive one match is at this point: little while most of the
    /// programme is ahead, everything in the last match.
    fn decisiveness(played: u16, left: u16) -> f32 {
        let progress = f32::from(played) / f32::from(played + left).max(1.0);
        (progress * progress).max(1.0 / f32::from(left.max(1)))
    }

    /// A derby raises whatever the table says.
    pub fn with_rivalry(stakes: f32) -> f32 {
        const DERBY_LIFT: f32 = 0.35;
        stakes + DERBY_LIFT * (1.0 - stakes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::league::DayMonthPeriod;

    fn settings(tier: u8, promotion: u8, relegation: u8) -> LeagueSettings {
        LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
            season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
            tier,
            promotion_spots: promotion,
            relegation_spots: relegation,
            league_group: None,
            split_season: false,
        }
    }

    /// A top division of twenty with four Champions League places, three
    /// more continental ones and three relegated.
    fn top_flight_lines() -> Vec<TableLine> {
        let mut lines = TableLine::of_league(&settings(1, 0, 3), 20);
        lines.push(TableLine::below(4, 0.95));
        lines.push(TableLine::below(7, 0.8));
        lines
    }

    fn standings(points: &[i32]) -> Vec<TableStanding> {
        points
            .iter()
            .enumerate()
            .map(|(place, &points)| TableStanding {
                team_id: place as u32 + 1,
                points,
            })
            .collect()
    }

    /// After 34 of 38: leaders clear, a mid-table that can reach nothing
    /// and be caught by nobody, and a scrap at the bottom.
    fn late_table() -> Vec<TableStanding> {
        standings(&[
            82, 80, 78, 76, 74, 72, 68, 50, 49, 48, 47, 46, 45, 44, 33, 32, 31, 30, 29, 28,
        ])
    }

    fn stakes_at(table: &[TableStanding], place: usize, played: u16, left: u16) -> f32 {
        TableStakes::of(table, place as u32, played, left, &top_flight_lines())
    }

    #[test]
    fn visitors_fighting_relegation_and_a_settled_home_side_read_differently() {
        let table = late_table();
        let threatened = stakes_at(&table, 17, 34, 4);
        let settled = stakes_at(&table, 10, 34, 4);
        assert!(threatened >= 0.85, "one point above the drop: {threatened}");
        assert_eq!(settled, TableStakes::PROFESSIONAL_FLOOR);
    }

    #[test]
    fn a_chase_for_a_european_place_matters() {
        // Seventh holds the last continental place; eighth is two points
        // behind with six to play and cannot reach the top four.
        let table = standings(&[
            75, 72, 70, 68, 60, 58, 52, 50, 44, 43, 42, 41, 40, 39, 38, 37, 36, 20, 18, 15,
        ]);
        let chasing = stakes_at(&table, 8, 32, 6);
        assert!(
            chasing >= 0.72,
            "two points off Europe with six left: {chasing}"
        );
        assert!(chasing > stakes_at(&table, 12, 32, 6) + 0.1);
    }

    #[test]
    fn a_lower_division_reads_its_own_promotion_places() {
        let lines = TableLine::of_league(&settings(2, 3, 3), 24);
        let mut points = vec![80, 78, 70, 70];
        points.extend((0..20).map(|i| 60 - i * 2));
        let table = standings(&points);
        let level_with_third = TableStakes::of(&table, 4, 42, 4, &lines);
        assert!(
            level_with_third >= 0.85,
            "level on points with promotion: {level_with_third}"
        );

        let no_promotion = TableLine::of_league(&settings(2, 0, 3), 24);
        assert!(TableStakes::of(&table, 4, 42, 4, &no_promotion) < level_with_third - 0.2);
    }

    #[test]
    fn a_champion_already_crowned_has_nothing_left() {
        let table = standings(&[
            90, 78, 76, 74, 70, 58, 56, 50, 49, 48, 47, 46, 45, 44, 43, 42, 41, 30, 29, 28,
        ]);
        assert_eq!(stakes_at(&table, 1, 35, 3), TableStakes::PROFESSIONAL_FLOOR);
        assert!(
            stakes_at(&table, 4, 35, 3) > TableStakes::PROFESSIONAL_FLOOR + 0.2,
            "fourth still has a Champions League place to defend"
        );
    }

    #[test]
    fn one_point_never_makes_a_cliff() {
        for left in [30u16, 12, 4, 1] {
            let played = 38 - left;
            for gap in 0..40 {
                let mut near = late_table();
                let mut far = late_table();
                near[16].points = near[17].points + gap;
                far[16].points = far[17].points + gap + 1;
                let step =
                    (stakes_at(&near, 17, played, left) - stakes_at(&far, 17, played, left)).abs();
                let limit = 0.56 / (f32::from(left) * 3.0) + 1e-4;
                assert!(
                    step <= limit,
                    "gap {gap}, {left} left: a point moved it by {step}"
                );
            }
        }
    }

    #[test]
    fn an_open_season_is_a_routine_match_and_a_derby_is_more() {
        let level = standings(&[0; 20]);
        let opener = stakes_at(&level, 10, 0, 38);
        assert!((opener - TableStakes::ROUTINE).abs() < 0.02, "{opener}");
        assert!(TableStakes::with_rivalry(opener) > opener + 0.1);
        assert!(TableStakes::with_rivalry(1.0) <= 1.0);
    }

    #[test]
    fn a_line_out_of_reach_either_way_counts_for_nothing() {
        let lines = [TableLine::below(2, 1.0)];
        let table = standings(&[30, 28, 10, 9]);
        assert_eq!(
            TableStakes::of(&table, 1, 5, 1, &lines),
            TableStakes::PROFESSIONAL_FLOOR,
            "eighteen clear with one to play"
        );
        assert_eq!(
            TableStakes::of(&table, 4, 5, 1, &lines),
            TableStakes::PROFESSIONAL_FLOOR,
            "nineteen adrift with one to play"
        );
        assert!(
            TableStakes::of(&table, 2, 5, 1, &lines) < 0.5,
            "second is safe by eighteen"
        );
    }
}

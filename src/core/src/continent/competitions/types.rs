use crate::league::Season;
use chrono::{Duration, NaiveDate, Weekday};

/// Reserved league_id values for continental competitions.
/// Used in match result processing to identify competition type.
pub const CHAMPIONS_LEAGUE_ID: u32 = 900_000_001;
pub const EUROPA_LEAGUE_ID: u32 = 900_000_002;
pub const CONFERENCE_LEAGUE_ID: u32 = 900_000_003;
pub const COPA_LIBERTADORES_ID: u32 = 900_000_004;

#[derive(Debug, Clone)]
pub enum CompetitionStage {
    NotStarted,
    Qualifying,
    GroupStage,
    RoundOf32,
    RoundOf16,
    QuarterFinals,
    SemiFinals,
    Final,
}

impl CompetitionStage {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            CompetitionStage::NotStarted => "stage_not_started",
            CompetitionStage::Qualifying => "stage_qualifying",
            CompetitionStage::GroupStage => "stage_group_stage",
            CompetitionStage::RoundOf32 => "round_of_32",
            CompetitionStage::RoundOf16 => "round_of_16",
            CompetitionStage::QuarterFinals => "quarter_finals",
            CompetitionStage::SemiFinals => "semi_finals",
            CompetitionStage::Final => "final",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContinentalMatch {
    pub home_team: u32,
    pub away_team: u32,
    pub date: NaiveDate,
    pub stage: CompetitionStage,
    pub match_id: String,
    pub result: Option<(u8, u8)>,
}

#[derive(Debug, Clone)]
pub struct ContinentalMatchResult {
    pub home_team: u32,
    pub away_team: u32,
    pub home_score: u8,
    pub away_score: u8,
    pub competition: CompetitionTier,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CompetitionTier {
    ChampionsLeague,
    EuropaLeague,
    ConferenceLeague,
    CopaLibertadores,
}

// ─── Shared group / knockout types for all continental competitions ──

#[derive(Debug, Clone, Default)]
pub struct GroupTable {
    pub rows: Vec<GroupRow>,
}

#[derive(Debug, Clone)]
pub struct GroupRow {
    pub team_id: u32,
    pub played: u8,
    pub won: u8,
    pub drawn: u8,
    pub lost: u8,
    pub gf: u8,
    pub ga: u8,
    pub points: u8,
}

impl GroupTable {
    pub fn new(teams: &[u32]) -> Self {
        GroupTable {
            rows: teams
                .iter()
                .map(|&id| GroupRow {
                    team_id: id,
                    played: 0,
                    won: 0,
                    drawn: 0,
                    lost: 0,
                    gf: 0,
                    ga: 0,
                    points: 0,
                })
                .collect(),
        }
    }

    pub fn update(&mut self, home_id: u32, away_id: u32, home_goals: u8, away_goals: u8) {
        use std::cmp::Ordering;
        match home_goals.cmp(&away_goals) {
            Ordering::Greater => {
                self.record(home_id, home_goals, away_goals, 3, true, false, false);
                self.record(away_id, away_goals, home_goals, 0, false, false, true);
            }
            Ordering::Less => {
                self.record(home_id, home_goals, away_goals, 0, false, false, true);
                self.record(away_id, away_goals, home_goals, 3, true, false, false);
            }
            Ordering::Equal => {
                self.record(home_id, home_goals, away_goals, 1, false, true, false);
                self.record(away_id, away_goals, home_goals, 1, false, true, false);
            }
        }
        self.sort();
    }

    fn record(
        &mut self,
        team_id: u32,
        gf: u8,
        ga: u8,
        pts: u8,
        won: bool,
        drawn: bool,
        lost: bool,
    ) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.team_id == team_id) {
            row.played += 1;
            row.gf += gf;
            row.ga += ga;
            row.points += pts;
            if won {
                row.won += 1;
            }
            if drawn {
                row.drawn += 1;
            }
            if lost {
                row.lost += 1;
            }
        }
    }

    fn sort(&mut self) {
        self.rows.sort_by(|a, b| {
            b.points
                .cmp(&a.points)
                .then_with(|| (b.gf as i16 - b.ga as i16).cmp(&(a.gf as i16 - a.ga as i16)))
                .then_with(|| b.gf.cmp(&a.gf))
        });
    }

    /// Top 2 teams qualify for knockout
    pub fn qualifiers(&self) -> (u32, u32) {
        (self.rows[0].team_id, self.rows[1].team_id)
    }
}

#[derive(Debug, Clone)]
pub struct KnockoutTie {
    pub home_team: u32,
    pub away_team: u32,
    pub leg1_score: Option<(u8, u8)>,
    pub leg2_score: Option<(u8, u8)>,
    /// Optional second-leg shootout result (home_kicks, away_kicks).
    /// Set by the caller when the leg was played as a knockout fixture
    /// and the aggregate ended level. `record_leg2_with_shootout` is
    /// the canonical entry point.
    pub shootout: Option<(u8, u8)>,
    pub winner: Option<u32>,
}

impl KnockoutTie {
    pub fn new(home: u32, away: u32) -> Self {
        KnockoutTie {
            home_team: home,
            away_team: away,
            leg1_score: None,
            leg2_score: None,
            shootout: None,
            winner: None,
        }
    }

    pub fn record_leg1(&mut self, home_goals: u8, away_goals: u8) {
        self.leg1_score = Some((home_goals, away_goals));
    }

    /// Record the second leg without an explicit shootout. If aggregate
    /// is level the tie has no winner yet — the caller is expected to
    /// either replay extra time / penalties externally or call
    /// `record_leg2_with_shootout` instead. Away-goals rule is NOT
    /// applied: UEFA dropped it in 2021 and we follow modern rules.
    pub fn record_leg2(&mut self, home_goals: u8, away_goals: u8) {
        self.record_leg2_with_shootout(home_goals, away_goals, None);
    }

    /// Canonical second-leg recorder. Aggregate decides the tie when
    /// it is level after both legs. If the aggregate is tied, the
    /// caller-provided shootout result (home_kicks, away_kicks)
    /// determines the winner. When neither aggregate nor shootout
    /// breaks the tie, `winner` stays `None` so the caller can detect
    /// the missing decisive event.
    pub fn record_leg2_with_shootout(
        &mut self,
        home_goals: u8,
        away_goals: u8,
        shootout: Option<(u8, u8)>,
    ) {
        self.leg2_score = Some((home_goals, away_goals));
        self.shootout = shootout;
        if let (Some((h1, a1)), Some((h2, a2))) = (self.leg1_score, self.leg2_score) {
            // Two-leg tie: home of leg1 hosts both fixtures of legs.
            // For aggregate purposes:
            //   leg1: home goals = h1, away goals = a1
            //   leg2: home goals = h2, away goals = a2 (sides reversed)
            // Aggregate from `self.home_team`'s perspective is
            // (h1 + a2): goals scored at home in leg 1 + goals scored
            // away in leg 2.
            let agg_home = h1 as u16 + a2 as u16;
            let agg_away = a1 as u16 + h2 as u16;
            self.winner = if agg_home > agg_away {
                Some(self.home_team)
            } else if agg_away > agg_home {
                Some(self.away_team)
            } else if let Some((sh, sa)) = shootout {
                // Aggregate level → shootout decides.
                if sh > sa {
                    Some(self.home_team)
                } else if sa > sh {
                    Some(self.away_team)
                } else {
                    None
                }
            } else {
                // Aggregate level, no shootout supplied — leave winner
                // undecided. Callers should either run extra time +
                // penalties externally or call this method again with
                // the resulting shootout score.
                None
            };
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransferInterest {
    pub player_id: u32,
    pub source_country: u32,
    pub interest_level: f32,
}

#[derive(Debug, Clone)]
pub struct TransferNegotiation {
    pub player_id: u32,
    pub selling_club: u32,
    pub buying_club: u32,
    pub current_offer: f64,
}

/// Where a continental fixture falls. A competition's calendar names the
/// season WEEK of each matchday or leg, on the same grid as the
/// international windows so no continental night lands inside one; the
/// competition's own weekday inside that week is the day it is played.
/// `index` (a group or tie) spreads a round across the competition's
/// weekdays.
pub struct ContinentalMatchweek;

impl ContinentalMatchweek {
    /// League-phase matchdays 1-6, shared by every competition so the
    /// three UEFA nights of one matchday fall in the same week.
    pub const LEAGUE_PHASE: [u32; 6] = [2, 4, 7, 9, 12, 14];
    pub const ROUND_OF_16_FIRST_LEGS: [u32; 2] = [24, 25];
    pub const ROUND_OF_16_SECOND_LEGS: [u32; 2] = [27, 28];
    pub const QUARTER_FINAL_LEGS: ([u32; 1], [u32; 1]) = ([31], [32]);
    pub const SEMI_FINAL_LEGS: ([u32; 1], [u32; 1]) = ([35], [36]);
    pub const FINAL: u32 = 38;

    /// True when the Monday-to-Sunday week holding `date` carries
    /// continental club football.
    pub fn is_matchweek(date: NaiveDate) -> bool {
        let season = Season::from_date(date);
        let start = season.week(0);
        if date < start {
            return false;
        }
        let week = ((date - start).num_days() / 7) as u32;
        Self::weeks().any(|matchweek| matchweek == week)
    }

    fn weeks() -> impl Iterator<Item = u32> {
        Self::LEAGUE_PHASE
            .into_iter()
            .chain(Self::ROUND_OF_16_FIRST_LEGS)
            .chain(Self::ROUND_OF_16_SECOND_LEGS)
            .chain(Self::QUARTER_FINAL_LEGS.0)
            .chain(Self::QUARTER_FINAL_LEGS.1)
            .chain(Self::SEMI_FINAL_LEGS.0)
            .chain(Self::SEMI_FINAL_LEGS.1)
            .chain([Self::FINAL])
    }

    pub fn day(season: &Season, week: u32, weekdays: &[Weekday], index: usize) -> NaiveDate {
        let weekday = weekdays[index % weekdays.len()];
        season.week(week) + Duration::days(weekday.num_days_from_monday() as i64)
    }

    /// A knockout leg spread over several weeks: ties fill one week's
    /// weekdays before the next week is used.
    pub fn leg(season: &Season, weeks: &[u32], weekdays: &[Weekday], index: usize) -> NaiveDate {
        let week = weeks[(index / weekdays.len()) % weeks.len()];
        Self::day(season, week, weekdays, index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn the_first_europa_league_matchday_of_2026_is_thursday_17_september() {
        let season = Season::new(2026);
        assert_eq!(
            ContinentalMatchweek::day(&season, ContinentalMatchweek::LEAGUE_PHASE[0], &[Weekday::Thu], 0),
            d(2026, 9, 17)
        );
    }

    #[test]
    fn the_index_spreads_a_round_across_the_weekdays() {
        let season = Season::new(2026);
        let weekdays = [Weekday::Tue, Weekday::Wed];
        assert_eq!(ContinentalMatchweek::day(&season, 2, &weekdays, 0), d(2026, 9, 15));
        assert_eq!(ContinentalMatchweek::day(&season, 2, &weekdays, 1), d(2026, 9, 16));
        assert_eq!(ContinentalMatchweek::day(&season, 2, &weekdays, 2), d(2026, 9, 15));
    }

    #[test]
    fn knockout_ties_fill_a_weeks_nights_before_the_next_week() {
        let season = Season::new(2026);
        let weekdays = [Weekday::Tue, Weekday::Wed];
        let weeks = ContinentalMatchweek::ROUND_OF_16_FIRST_LEGS;
        let days: Vec<NaiveDate> = (0..4)
            .map(|i| ContinentalMatchweek::leg(&season, &weeks, &weekdays, i))
            .collect();
        assert_eq!(
            days,
            vec![d(2027, 2, 16), d(2027, 2, 17), d(2027, 2, 23), d(2027, 2, 24)]
        );
    }

    #[test]
    fn no_continental_night_falls_inside_an_international_window() {
        use crate::InternationalCalendar;
        for year in 2026..=2060 {
            let season = Season::new(year);
            for week in ContinentalMatchweek::weeks() {
                for weekday in [Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Sat] {
                    let day = ContinentalMatchweek::day(&season, week, &[weekday], 0);
                    assert!(
                        InternationalCalendar::window_on(day).is_none(),
                        "{year}: week {week} {weekday:?} ({day}) inside a window"
                    );
                }
            }
        }
    }

    #[test]
    fn a_free_saturday_follows_every_window() {
        use crate::InternationalCalendar;
        for year in 2026..=2060 {
            let season = Season::new(year);
            let first_nights: Vec<NaiveDate> = ContinentalMatchweek::weeks()
                .map(|week| ContinentalMatchweek::day(&season, week, &[Weekday::Tue], 0))
                .collect();
            for window in InternationalCalendar::windows(&season) {
                let Some(next) = first_nights.iter().filter(|n| **n > window.closes).min() else {
                    continue;
                };
                let saturday = (1..(*next - window.closes).num_days())
                    .map(|k| window.closes + Duration::days(k))
                    .any(|day| day.weekday() == Weekday::Sat);
                assert!(saturday, "{year}: no Saturday between {} and {next}", window.closes);
            }
        }
    }

    #[test]
    fn a_matchweek_is_the_whole_monday_to_sunday_week() {
        // Week 2 of 2026 runs Monday 14 to Sunday 20 September.
        assert!(!ContinentalMatchweek::is_matchweek(d(2026, 9, 13)));
        assert!(ContinentalMatchweek::is_matchweek(d(2026, 9, 14)));
        assert!(ContinentalMatchweek::is_matchweek(d(2026, 9, 20)));
        assert!(!ContinentalMatchweek::is_matchweek(d(2026, 9, 21)));
        assert!(!ContinentalMatchweek::is_matchweek(d(2026, 8, 20)), "before week 0");
    }

    #[test]
    fn the_three_uefa_competitions_share_their_weeks() {
        use crate::continent::{
            ChampionsLeague, ConferenceLeague, ContinentalMatch, ContinentalRankings, EuropaLeague,
        };
        use std::collections::BTreeSet;

        let weeks = |matches: &[ContinentalMatch], knockout: bool| -> BTreeSet<NaiveDate> {
            matches
                .iter()
                .filter(|m| matches!(m.stage, CompetitionStage::RoundOf16) == knockout)
                .map(|m| m.date - Duration::days(m.date.weekday().num_days_from_monday() as i64))
                .collect()
        };
        let clubs: Vec<u32> = (1..=32).collect();
        let rankings = ContinentalRankings::new();
        for year in [2026, 2027, 2033] {
            let draw = NaiveDate::from_ymd_opt(year, 8, 15).unwrap();
            let mut cl = ChampionsLeague::new();
            cl.conduct_draw(&clubs, &rankings, draw);
            cl.generate_knockout_fixtures();
            let mut el = EuropaLeague::new();
            el.conduct_draw(&clubs, &rankings, draw);
            el.generate_knockout_fixtures();
            let mut uecl = ConferenceLeague::new();
            uecl.conduct_draw(&clubs, &rankings, draw);
            uecl.generate_knockout_fixtures();

            for knockout in [false, true] {
                let cl_weeks = weeks(&cl.matches, knockout);
                assert_eq!(cl_weeks, weeks(&el.matches, knockout), "{year} CL v EL");
                assert_eq!(cl_weeks, weeks(&uecl.matches, knockout), "{year} CL v UECL");
            }
            assert_eq!(weeks(&cl.matches, false).len(), 6, "{year}: six league-phase weeks");
        }
    }

    #[test]
    fn continental_reserved_ids_are_distinct() {
        let ids = [
            CHAMPIONS_LEAGUE_ID,
            EUROPA_LEAGUE_ID,
            CONFERENCE_LEAGUE_ID,
            COPA_LIBERTADORES_ID,
        ];
        // Copa Libertadores must claim its own reserved slot so match-event
        // routing can recognise it as a continental cup.
        assert_eq!(COPA_LIBERTADORES_ID, 900_000_004);
        for (i, a) in ids.iter().enumerate() {
            for b in ids.iter().skip(i + 1) {
                assert_ne!(a, b, "reserved continental ids must be unique");
            }
        }
    }

    #[test]
    fn knockout_aggregate_winner_decides_when_unequal() {
        // Home wins 2-1 at home, draws 1-1 away → aggregate 3-2 home.
        let mut tie = KnockoutTie::new(1, 2);
        tie.record_leg1(2, 1);
        tie.record_leg2(1, 1);
        assert_eq!(tie.winner, Some(1));
    }

    #[test]
    fn knockout_aggregate_winner_handles_road_advantage_without_away_goals_rule() {
        // Leg 1 (team 1 hosts): team 1 wins 1-0 → h1=1, a1=0.
        // Leg 2 (team 2 hosts): team 2 wins 1-0 → h2=1 (team 2 at home),
        //                                          a2=0 (team 1 away).
        // Aggregate: team 1 = h1 + a2 = 1, team 2 = a1 + h2 = 1 → tied.
        // Without away-goals rule the tie is undecided → None.
        let mut tie = KnockoutTie::new(1, 2);
        tie.record_leg1(1, 0);
        tie.record_leg2(1, 0);
        assert_eq!(tie.winner, None);
    }

    #[test]
    fn knockout_tied_aggregate_resolves_via_shootout() {
        // Build a tied aggregate (each side wins their home leg 1-0)
        // and feed a home-favouring shootout.
        let mut tie = KnockoutTie::new(1, 2);
        tie.record_leg1(1, 0);
        tie.record_leg2_with_shootout(1, 0, Some((4, 3)));
        // Home shootout score 4 > away 3 → home wins.
        assert_eq!(tie.winner, Some(1));
    }

    #[test]
    fn knockout_tied_aggregate_resolves_via_shootout_for_visitor() {
        let mut tie = KnockoutTie::new(1, 2);
        tie.record_leg1(1, 0);
        tie.record_leg2_with_shootout(1, 0, Some((3, 5)));
        assert_eq!(tie.winner, Some(2));
    }

    #[test]
    fn knockout_no_winner_when_aggregate_level_and_no_shootout() {
        let mut tie = KnockoutTie::new(1, 2);
        tie.record_leg1(0, 0);
        tie.record_leg2(2, 2);
        assert_eq!(tie.winner, None);
        // Caller can supply a shootout later (e.g. after running ET +
        // pens externally).
        tie.record_leg2_with_shootout(2, 2, Some((5, 4)));
        assert_eq!(tie.winner, Some(1));
    }
}

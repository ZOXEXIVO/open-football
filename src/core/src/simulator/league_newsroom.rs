use crate::Country;
use crate::club::news::{
    NewsEditor, NewsStory, NewspaperIssue, PressMood, RecentEvents, RumourDesk,
};
use crate::league::{ChartsDesk, League};
use crate::simulator::SimulatorData;
use chrono::{Datelike, NaiveDate};
use rayon::prelude::*;
use std::collections::HashMap;

/// The monthly press run for the divisions themselves.
///
/// A club paper is written from inside a dressing room: it knows how the
/// week went and it takes a side. A league paper is written from above
/// the whole division and has no side to take, so it prints the two
/// things that only make sense at that altitude — who scored the most
/// goals in the month, and who the division's players are being linked
/// with. Everything else a supporter wants is already on his own club's
/// front page.
///
/// Runs on the 1st, immediately after `MonthlyAwardsTick`, so the month
/// it is reporting has already been frozen into the league's awards
/// shelf and the charts can be read rather than recomputed.
///
/// Laid out gather-then-write like the weekly run: all the reading is
/// done in parallel over an immutable world and only the finished
/// editions are applied under `&mut`.
pub(crate) struct LeagueNewsroomTick;

/// One league's month, ready to be filed.
struct PendingIssue {
    league_id: u32,
    /// The month covered, as the first of that month. Recorded even
    /// when `issue` is `None` so an empty month is never re-walked.
    month_start: NaiveDate,
    issue: Option<NewspaperIssue>,
}

impl LeagueNewsroomTick {
    pub(crate) fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        if today.day() != 1 {
            return;
        }
        let Some(month_start) = Self::previous_month(today) else {
            return;
        };

        let pending = Self::collect(data, today, month_start);

        for entry in pending {
            if let Some(league) = data.league_mut(entry.league_id) {
                if let Some(issue) = entry.issue {
                    league.newsroom.publish(issue);
                }
                league.newsroom.close_month(entry.month_start);
            }
        }
    }

    /// First-of-month → the first of the month just ended.
    fn previous_month(today: NaiveDate) -> Option<NaiveDate> {
        if today.month() == 1 {
            NaiveDate::from_ymd_opt(today.year() - 1, 12, 1)
        } else {
            NaiveDate::from_ymd_opt(today.year(), today.month() - 1, 1)
        }
    }

    fn collect(
        data: &SimulatorData,
        today: NaiveDate,
        month_start: NaiveDate,
    ) -> Vec<PendingIssue> {
        data.continents
            .par_iter()
            .flat_map(|continent| continent.countries.par_iter())
            .flat_map_iter(|country| {
                country
                    .leagues
                    .leagues
                    .iter()
                    .filter_map(move |league| Self::compile(league, country, today, month_start))
            })
            .collect()
    }

    /// Put one division's month to bed. `None` means this league does
    /// not print at all — not that it had a quiet month.
    fn compile(
        league: &League,
        country: &Country,
        today: NaiveDate,
        month_start: NaiveDate,
    ) -> Option<PendingIssue> {
        // Cups, playoffs and friendly competitions have no league page
        // to read a paper on, and a knockout has no scoring chart worth
        // the name — a two-goal night in a one-legged tie is not a
        // month's work.
        if league.friendly || league.is_cup {
            return None;
        }
        if league.newsroom.has_covered(month_start) {
            return None;
        }

        let mut candidates: Vec<NewsStory> = Vec::new();

        // The charts, read off the month `MonthlyAwardsTick` has just
        // frozen. A month with no football leaves no snapshot, and the
        // guard on the date is what stops last month's chart being
        // reprinted under this month's date during a winter break.
        if let Some(snapshot) = league
            .awards
            .latest_monthly_snapshot()
            .filter(|snapshot| snapshot.month_start_date == month_start)
        {
            ChartsDesk::file(&mut candidates, snapshot, today);
        }

        let sides = Self::file_rumours(&mut candidates, league, country, today);

        // Every line on a division's page names somebody's club, and the
        // page is read for a year afterwards. The credit is therefore
        // settled here, against the division as it stood on press day,
        // rather than looked up when a reader opens the archive — a
        // player sold in September would otherwise drag August's "asks
        // to leave" onto the club he had just joined.
        sides.credit(&mut candidates);

        let stories = NewsEditor::compile(candidates, &league.newsroom.issues);

        // A month in which the division neither scored nor was talked
        // about prints nothing. The month is still closed by the caller
        // so it is never walked again.
        let issue = (!stories.is_empty()).then(|| NewspaperIssue {
            number: league.newsroom.next_number,
            date: today,
            // A division has no form and nobody to be pleased or
            // furious for. The mood the club papers swing on is a
            // club's mood; a league review is written level.
            mood: PressMood::Steady,
            stories,
            // The rail's results column is a club's own fixtures. A
            // division played every fixture in it, so there is no such
            // list to print, and the sheet sets the page full-measure.
            results: Vec::new(),
        });

        Some(PendingIssue {
            league_id: league.id,
            month_start,
            issue,
        })
    }

    /// The rumour mill, division-wide.
    ///
    /// The club desk is reused whole rather than reimplemented: it
    /// already knows the running order a transfer column ranks its
    /// stories in, and two rumour desks would disagree the first time
    /// one of them changed. It is run over the month rather than the
    /// club paper's fortnight — see `RumourDesk::file_player_over`.
    ///
    /// Only sides actually playing in this division are walked. A
    /// club's B team and its under-18s are in other competitions (or
    /// none), and their players belong to those papers.
    ///
    /// Returns the team sheet it read on the way through. The charts
    /// desk needs the same answer for its own credits, and walking a
    /// country's clubs twice to ask the same question would be two
    /// readings of one roster that a mid-walk sale could disagree on.
    fn file_rumours(
        out: &mut Vec<NewsStory>,
        league: &League,
        country: &Country,
        today: NaiveDate,
    ) -> DivisionSides {
        let mut sides = DivisionSides::default();

        for club in &country.clubs {
            for team in &club.teams.teams {
                if team.league_id != Some(league.id) {
                    continue;
                }
                for player in &team.players.players {
                    sides.side_of.insert(player.id, team.id);
                    RumourDesk::file_player_over(out, player, today, RecentEvents::MONTH);
                }
            }
        }

        sides
    }
}

/// Who played for whom in this division on press day.
///
/// A club's paper has one credit and it is on the nameplate. A
/// division's has one per story, and unlike the nameplate it can go out
/// of date between the presses running and a reader opening the page —
/// which is the whole reason it is written down here rather than looked
/// up then.
#[derive(Default)]
struct DivisionSides {
    side_of: HashMap<u32, u32>,
}

impl DivisionSides {
    /// Stamp each story with the side its subject was at.
    ///
    /// A subject the walk never saw — a scorer sold between the last
    /// fixture of the month and the first of the next — keeps the `0`
    /// he was filed with, and the page falls back to naming his current
    /// club. That is the old behaviour, and for a man who has left the
    /// division it is the only club there is left to name.
    fn credit(&self, stories: &mut [NewsStory]) {
        for story in stories {
            if let Some(team_id) = self.side_of.get(&story.player_id) {
                story.credited_team_id = *team_id;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DivisionSides, LeagueNewsroomTick};
    use crate::PlayerFieldPositionGroup;
    use crate::club::news::{NewsEditor, NewsStory, NewsStoryKind, NewspaperIssue, PressMood};
    use crate::league::awards::{MonthlyAwardsSnapshot, MonthlyStatLeader};
    use crate::league::{ChartsDesk, LeagueNewsroom};
    use chrono::NaiveDate;
    use std::collections::VecDeque;

    /// Builds the two halves of a division's month the way `compile`
    /// does, so the assembly can be checked without standing up a world.
    struct Month;

    impl Month {
        fn press_day() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
        }

        fn snapshot(places: usize) -> MonthlyAwardsSnapshot {
            let top_scorers = (0..places)
                .map(|place| MonthlyStatLeader {
                    player_id: 100 + place as u32,
                    player_name: format!("Scorer {}", place),
                    player_slug: format!("{}-scorer", 100 + place),
                    club_id: 200 + place as u32,
                    club_name: "Club".to_string(),
                    club_slug: "club".to_string(),
                    position_group: PlayerFieldPositionGroup::Forward,
                    matches_played: 4,
                    goals: (9 - place) as u8,
                    assists: 1,
                    average_rating: 7.5,
                })
                .collect();

            MonthlyAwardsSnapshot {
                month_start_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
                month_end_date: NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
                matches_count: 40,
                player_of_month: None,
                young_player_of_month: None,
                team_of_month: Vec::new(),
                young_team_of_month: Vec::new(),
                top_scorers,
                top_assists: Vec::new(),
                best_ratings: Vec::new(),
            }
        }

        /// A month's transfer talk, one line each about different
        /// players — the shape `RumourDesk` produces across a division.
        fn rumours() -> Vec<NewsStory> {
            let day = Self::press_day();
            vec![
                NewsStory::new(NewsStoryKind::BidRejected, day)
                    .about(300)
                    .against(400),
                NewsStory::new(NewsStoryKind::RumourRival, day)
                    .about(301)
                    .against(401),
                NewsStory::new(NewsStoryKind::ScoutsWatching, day)
                    .about(302)
                    .against(402),
                NewsStory::new(NewsStoryKind::TransferSpeculation, day).about(303),
                NewsStory::new(NewsStoryKind::HomecomingLink, day)
                    .about(304)
                    .against(404),
                NewsStory::new(NewsStoryKind::AgentTouting, day)
                    .about(305)
                    .against(405),
            ]
        }

        /// Both halves through the editor, exactly as `compile` runs it.
        fn edition() -> Vec<NewsStory> {
            let snapshot = Self::snapshot(5);
            let mut candidates = Vec::new();
            ChartsDesk::file(&mut candidates, &snapshot, Self::press_day());
            candidates.extend(Self::rumours());

            NewsEditor::compile(candidates, &VecDeque::new())
        }
    }

    /// The paper is the charts and the mill, and the editor has to let
    /// both onto the page. A run that printed five scorers and no
    /// transfer talk — or the reverse — is not the paper that was asked
    /// for, and the desk allowances are the only thing deciding it.
    #[test]
    fn a_months_edition_carries_both_the_charts_and_the_mill() {
        let edition = Month::edition();

        let charts = edition
            .iter()
            .filter(|story| story.desk() == crate::club::news::NewsDesk::Charts)
            .count();
        let market = edition
            .iter()
            .filter(|story| story.desk() == crate::club::news::NewsDesk::Market)
            .count();

        assert_eq!(charts, 5, "the whole five-man chart prints, not four of it");
        assert!(market >= 3, "the transfer mill got {} lines", market);
    }

    /// The month's leading scorer is what a division's paper is for, so
    /// it leads — ahead of every live transfer link on the page.
    #[test]
    fn the_months_leading_scorer_takes_the_front_page() {
        let edition = Month::edition();

        assert_eq!(edition[0].kind, NewsStoryKind::LeagueTopScorer);
        assert_eq!(edition[0].player_id, 100);
        assert_eq!(edition[0].a, 9, "the tally the copy quotes");
    }

    /// The four men behind the leader are four separate stories about
    /// four separate players — the kind has to allow repeats or the
    /// chart collapses to a single line.
    #[test]
    fn the_field_behind_the_leader_is_not_collapsed_to_one_line() {
        let chase: Vec<u32> = Month::edition()
            .iter()
            .filter(|story| story.kind == NewsStoryKind::LeagueScoringChase)
            .map(|story| story.player_id)
            .collect();

        assert_eq!(chase, vec![101, 102, 103, 104]);
    }

    /// The credit on a division's story is settled on press day, not
    /// when a reader opens the archive. A player sold in September must
    /// not drag August's "asks to leave" onto the club he has just
    /// joined — which is exactly what a live lookup printed.
    #[test]
    fn a_months_stories_are_credited_where_they_were_written() {
        let mut sides = DivisionSides::default();
        sides.side_of.insert(300, 41);
        sides.side_of.insert(301, 42);
        // 302–305 were never on one of the division's team sheets.

        let mut stories = Month::rumours();
        sides.credit(&mut stories);

        let credited: Vec<(u32, u32)> = stories
            .iter()
            .map(|story| (story.player_id, story.credited_team_id))
            .collect();

        assert_eq!(credited[0], (300, 41));
        assert_eq!(credited[1], (301, 42));
        assert!(
            credited[2..].iter().all(|(_, team)| *team == 0),
            "a subject the walk never saw keeps no credit, and the page \
             falls back to naming his current club: {:?}",
            credited
        );
    }

    /// The charts come off a frozen snapshot but are filed before the
    /// walk, so they only get their credit if the same stamp covers
    /// them. A scoring chart that renamed its clubs a month later is
    /// the same bug as a transfer request that did.
    #[test]
    fn the_scoring_chart_is_credited_too() {
        let mut sides = DivisionSides::default();
        sides.side_of.insert(100, 41);

        let mut stories = Vec::new();
        ChartsDesk::file(&mut stories, &Month::snapshot(5), Month::press_day());
        sides.credit(&mut stories);

        assert_eq!(stories[0].player_id, 100);
        assert_eq!(stories[0].credited_team_id, 41);
    }

    /// The window is the month just ended, and it has to survive the
    /// year boundary — a paper dated 1 January reviews December.
    #[test]
    fn the_first_of_january_reviews_the_december_before_it() {
        assert_eq!(
            LeagueNewsroomTick::previous_month(NaiveDate::from_ymd_opt(2027, 1, 1).unwrap()),
            NaiveDate::from_ymd_opt(2026, 12, 1)
        );
        assert_eq!(
            LeagueNewsroomTick::previous_month(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()),
            NaiveDate::from_ymd_opt(2026, 3, 1)
        );
    }

    /// A month that has already been put to bed is never walked again,
    /// printed or not. The division's whole player list is behind that
    /// check.
    #[test]
    fn a_month_already_put_to_bed_is_not_reopened() {
        let mut room = LeagueNewsroom::for_league(1);
        let march = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();

        assert!(!room.has_covered(march));

        room.publish(NewspaperIssue {
            number: 1,
            date: NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
            mood: PressMood::Steady,
            stories: vec![
                crate::club::news::NewsStory::new(NewsStoryKind::LeagueTopScorer, march).about(9),
            ],
            results: Vec::new(),
        });
        room.close_month(march);

        assert!(room.has_covered(march));
    }
}

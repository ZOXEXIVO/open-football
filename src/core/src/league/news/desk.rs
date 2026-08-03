use crate::club::news::{NewsStory, NewsStoryKind};
use crate::league::awards::{MonthlyAwardsSnapshot, MonthlyStatLeader};
use chrono::NaiveDate;

/// The scoring charts, read off the month the league has just closed.
///
/// The desk deliberately does not compute anything. `MonthlyAwardsTick`
/// has already frozen the month's leading scorers into the league's own
/// awards shelf — names, clubs, goals and appearances — and re-deriving
/// the same table from the match records a few lines later would be two
/// charts that could disagree with each other.
pub struct ChartsDesk;

impl ChartsDesk {
    /// Places on the chart the paper prints. The snapshot holds five;
    /// the leader gets the front page and the four behind him get the
    /// column, which is exactly what the page has room for once the
    /// rumour mill has taken its share.
    const PLACES: usize = 5;

    /// Names from the team of the month. A column that printed one
    /// name would not be a team of the month, and one that printed
    /// eleven would be the whole page.
    const TEAM_NAMES: usize = 2;

    /// Places on the assists chart. Shorter than the scoring chart on
    /// purpose: the page carries both, and goals are what a division
    /// argues about.
    const ASSIST_PLACES: usize = 3;

    pub fn file(out: &mut Vec<NewsStory>, snapshot: &MonthlyAwardsSnapshot, date: NaiveDate) {
        for (place, leader) in snapshot.top_scorers.iter().take(Self::PLACES).enumerate() {
            // The snapshot only records players who actually scored, but
            // the copy quotes the tally and a chart entry on nought is a
            // sentence the paper cannot stand behind.
            if leader.goals == 0 {
                continue;
            }

            let kind = if place == 0 {
                NewsStoryKind::LeagueTopScorer
            } else {
                NewsStoryKind::LeagueScoringChase
            };

            out.push(Self::entry(kind, leader, date));
        }

        Self::file_awards(out, snapshot, date);
        Self::file_assists(out, snapshot, date);
        Self::file_ratings(out, snapshot, date);
        Self::file_team_of_month(out, snapshot, date);
    }

    /// The month's two individual awards.
    ///
    /// A club's paper reports these as its own player being honoured.
    /// The division's paper is the only page on which they are what
    /// they actually are — a verdict on everybody, settled.
    fn file_awards(out: &mut Vec<NewsStory>, snapshot: &MonthlyAwardsSnapshot, date: NaiveDate) {
        for (award, kind) in [
            (
                snapshot.player_of_month.as_ref(),
                NewsStoryKind::LeaguePlayerOfMonth,
            ),
            (
                snapshot.young_player_of_month.as_ref(),
                NewsStoryKind::LeagueYoungStar,
            ),
        ] {
            let Some(award) = award else {
                continue;
            };
            // The copy quotes his mark, so an award with no rating
            // behind it is one the page cannot print.
            let rating = (award.average_rating * 100.0) as i32;
            if rating <= 0 {
                continue;
            }
            out.push(
                NewsStory::new(kind, date)
                    .about(award.player_id)
                    .with_numbers(award.goals as i32 + award.assists as i32, rating),
            );
        }
    }

    /// The other chart. Goals decide who gets talked about and assists
    /// decide who the scorers should be thanking.
    fn file_assists(out: &mut Vec<NewsStory>, snapshot: &MonthlyAwardsSnapshot, date: NaiveDate) {
        for (place, leader) in snapshot
            .top_assists
            .iter()
            .take(Self::ASSIST_PLACES)
            .enumerate()
        {
            if leader.assists == 0 {
                continue;
            }
            let kind = if place == 0 {
                NewsStoryKind::LeagueAssistKing
            } else {
                NewsStoryKind::LeagueAssistChase
            };
            out.push(
                NewsStory::new(kind, date)
                    .about(leader.player_id)
                    .with_numbers(
                        leader.assists as i32,
                        (leader.average_rating * 100.0) as i32,
                    ),
            );
        }
    }

    /// The chart that catches everybody a goal tally never will — the
    /// centre-half who was immense for four weekends, the holding
    /// midfielder nobody outside his own ground has heard of.
    fn file_ratings(out: &mut Vec<NewsStory>, snapshot: &MonthlyAwardsSnapshot, date: NaiveDate) {
        let Some(leader) = snapshot.best_ratings.first() else {
            return;
        };
        let rating = (leader.average_rating * 100.0) as i32;
        if rating <= 0 {
            return;
        }
        out.push(
            NewsStory::new(NewsStoryKind::LeagueRatingsLeader, date)
                .about(leader.player_id)
                .with_numbers(leader.matches_played as i32, rating),
        );
    }

    fn file_team_of_month(
        out: &mut Vec<NewsStory>,
        snapshot: &MonthlyAwardsSnapshot,
        date: NaiveDate,
    ) {
        for slot in snapshot.team_of_month.iter().take(Self::TEAM_NAMES) {
            let rating = (slot.average_rating * 100.0) as i32;
            if rating <= 0 {
                continue;
            }
            out.push(
                NewsStory::new(NewsStoryKind::LeagueTeamOfMonth, date)
                    .about(slot.player_id)
                    .with_numbers(slot.matches_played as i32, rating),
            );
        }
    }

    /// One line of the chart. `{n}` is the tally and `{m}` the games it
    /// took — the two numbers a scoring chart is actually made of — and
    /// `{club}` is filled by the web layer from the player's own side,
    /// because on a division's page every story belongs to a different
    /// club.
    fn entry(kind: NewsStoryKind, leader: &MonthlyStatLeader, date: NaiveDate) -> NewsStory {
        NewsStory::new(kind, date)
            .about(leader.player_id)
            .with_numbers(leader.goals as i32, leader.matches_played as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::ChartsDesk;
    use crate::PlayerFieldPositionGroup;
    use crate::club::news::{NewsStory, NewsStoryKind};
    use crate::league::awards::{MonthlyAwardsSnapshot, MonthlyStatLeader};
    use chrono::NaiveDate;

    struct Charts;

    impl Charts {
        fn day() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
        }

        fn leader(id: u32, goals: u8) -> MonthlyStatLeader {
            MonthlyStatLeader {
                player_id: id,
                player_name: format!("Player {}", id),
                player_slug: format!("{}-player", id),
                club_id: 100 + id,
                club_name: "Club".to_string(),
                club_slug: "club".to_string(),
                position_group: PlayerFieldPositionGroup::Forward,
                matches_played: 4,
                goals,
                assists: 1,
                average_rating: 7.4,
            }
        }

        fn snapshot(scorers: Vec<MonthlyStatLeader>) -> MonthlyAwardsSnapshot {
            MonthlyAwardsSnapshot {
                month_start_date: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
                month_end_date: NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
                matches_count: 40,
                player_of_month: None,
                young_player_of_month: None,
                team_of_month: Vec::new(),
                young_team_of_month: Vec::new(),
                top_scorers: scorers,
                top_assists: Vec::new(),
                best_ratings: Vec::new(),
            }
        }

        fn file(scorers: Vec<MonthlyStatLeader>) -> Vec<NewsStory> {
            let mut out = Vec::new();
            ChartsDesk::file(&mut out, &Self::snapshot(scorers), Self::day());
            out
        }
    }

    /// The man at the top gets the front page; everybody else on the
    /// chart is the column beside it. One leader, never two.
    #[test]
    fn the_chart_has_one_leader_and_a_field_behind_him() {
        let stories = Charts::file(vec![
            Charts::leader(1, 7),
            Charts::leader(2, 6),
            Charts::leader(3, 5),
        ]);

        assert_eq!(stories.len(), 3);
        assert_eq!(stories[0].kind, NewsStoryKind::LeagueTopScorer);
        assert_eq!(stories[0].player_id, 1);
        assert_eq!(stories[0].a, 7, "the tally fills {{n}}");
        assert_eq!(stories[0].b, 4, "the games fill {{m}}");
        assert!(
            stories[1..]
                .iter()
                .all(|s| s.kind == NewsStoryKind::LeagueScoringChase),
            "only the leader leads"
        );
    }

    /// The copy quotes the tally, so a nought on the chart is a story
    /// the paper cannot print. The snapshot should never contain one —
    /// this is the guard that keeps it that way if it ever does.
    #[test]
    fn a_goalless_entry_never_reaches_the_chart() {
        let stories = Charts::file(vec![Charts::leader(1, 3), Charts::leader(2, 0)]);

        assert_eq!(stories.len(), 1);
        assert_eq!(stories[0].player_id, 1);
    }

    /// A month nobody scored in is a month with no chart. It is not a
    /// front page reading "nobody scored".
    #[test]
    fn a_month_without_a_goal_files_nothing() {
        assert!(Charts::file(Vec::new()).is_empty());
    }

    /// The page carries every column the month produced, not just the
    /// scorers.
    ///
    /// The snapshot has always held an award, a young award, an assists
    /// chart, a ratings chart and a team of the month, frozen and
    /// ready, and the desk read one field of it — so a division's own
    /// paper printed the same two stories every month for the life of a
    /// save. This is the test that keeps the rest of them on the page.
    #[test]
    fn the_monthly_page_reads_the_whole_snapshot() {
        use crate::league::awards::{MonthlyPlayerAward, TeamOfTheWeekSlot};

        let award = |id: u32| MonthlyPlayerAward {
            month_end_date: NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
            player_id: id,
            player_name: format!("Player {}", id),
            player_slug: format!("{}-player", id),
            club_id: 100 + id,
            club_name: "Club".to_string(),
            club_slug: "club".to_string(),
            matches_played: 4,
            goals: 3,
            assists: 2,
            average_rating: 7.6,
            score: 9.0,
        };

        let slot = |id: u32| TeamOfTheWeekSlot {
            player_id: id,
            player_name: format!("Player {}", id),
            player_slug: format!("{}-player", id),
            club_id: 100 + id,
            club_name: "Club".to_string(),
            club_slug: "club".to_string(),
            position_group: PlayerFieldPositionGroup::Midfielder,
            score: 8.0,
            matches_played: 4,
            goals: 1,
            assists: 1,
            average_rating: 7.5,
        };

        let mut snapshot = Charts::snapshot(vec![Charts::leader(1, 7), Charts::leader(2, 5)]);
        snapshot.player_of_month = Some(award(10));
        snapshot.young_player_of_month = Some(award(11));
        snapshot.top_assists = vec![Charts::leader(20, 1), Charts::leader(21, 1)];
        snapshot.best_ratings = vec![Charts::leader(30, 2)];
        snapshot.team_of_month = vec![slot(40), slot(41), slot(42)];

        let mut out = Vec::new();
        ChartsDesk::file(&mut out, &snapshot, Charts::day());
        let kinds: Vec<NewsStoryKind> = out.iter().map(|story| story.kind).collect();

        for expected in [
            NewsStoryKind::LeagueTopScorer,
            NewsStoryKind::LeagueScoringChase,
            NewsStoryKind::LeaguePlayerOfMonth,
            NewsStoryKind::LeagueYoungStar,
            NewsStoryKind::LeagueAssistKing,
            NewsStoryKind::LeagueAssistChase,
            NewsStoryKind::LeagueRatingsLeader,
            NewsStoryKind::LeagueTeamOfMonth,
        ] {
            assert!(
                kinds.contains(&expected),
                "the monthly page dropped {:?}; it has: {:?}",
                expected,
                kinds
            );
        }

        // The team of the month is a list of names, not the whole
        // eleven — a column, not the page.
        assert_eq!(
            kinds
                .iter()
                .filter(|kind| **kind == NewsStoryKind::LeagueTeamOfMonth)
                .count(),
            2
        );

        // Every one of these quotes a mark, so none may reach the
        // editor without one.
        for story in &out {
            if story.kind.quotes_a_rating() {
                assert!(story.b > 0, "{:?} would print a mark of 0.00", story.kind);
            }
        }
    }
}

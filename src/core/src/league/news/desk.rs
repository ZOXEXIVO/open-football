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
}

use super::facts::{StandingSnapshot, WeeklyMatchFacts};
use crate::Team;
use crate::club::news::types::{IssueResult, NewsStory, NewsStoryKind};
use chrono::NaiveDate;
use rustc_hash::FxHashSet;

/// Match reports and the run-of-form pieces that hang off them.
pub struct MatchDesk;

impl MatchDesk {
    pub fn file(
        out: &mut Vec<NewsStory>,
        results: &[IssueResult],
        rival_team_ids: &FxHashSet<u32>,
        facts: &WeeklyMatchFacts,
        team: &Team,
    ) {
        for result in results {
            if result.is_cup() {
                Self::file_cup_tie(out, result, facts, team.id);
                continue;
            }

            let is_derby = rival_team_ids.contains(&result.opponent_team_id);
            let kind = Self::classify(result.goals_for, result.goals_against, is_derby);

            // A seven-goal thriller outranks a 1-0: the bigger the story
            // on the pitch, the higher up the page it goes.
            let goal_bonus = (result.goals_for as i32 + result.goals_against as i32) * 6;

            out.push(
                NewsStory::new(kind, result.date)
                    .against(result.opponent_team_id)
                    // The side's top scorer that afternoon, when the
                    // week's facts recorded one for exactly this result.
                    // With him on the story the composer may reach for
                    // the copy that names him; without him it stays on
                    // the phrasings that only need a scoreline.
                    .about(facts.star_of(team.id, result.opponent_team_id, result.goals_for))
                    .at_home(result.is_home)
                    .with_numbers(result.goals_for as i32, result.goals_against as i32)
                    .weighted(goal_bonus),
            );
        }

        Self::file_runs(out, results, team);
    }

    /// Which report a scoreline earns. A derby is its own story whatever
    /// the margin — losing 1-0 to the neighbours hurts more than losing
    /// 4-0 to anyone else, and the page has to read that way.
    pub fn classify(goals_for: u8, goals_against: u8, is_derby: bool) -> NewsStoryKind {
        let margin = goals_for as i32 - goals_against as i32;

        if is_derby && margin > 0 {
            NewsStoryKind::DerbyWin
        } else if is_derby && margin < 0 {
            NewsStoryKind::DerbyDefeat
        } else if margin >= 3 {
            NewsStoryKind::Rout
        } else if margin <= -3 {
            NewsStoryKind::HeavyDefeat
        } else if margin > 0 {
            NewsStoryKind::LeagueWin
        } else if margin < 0 {
            NewsStoryKind::LeagueDefeat
        } else if goals_for == 0 {
            // A goalless afternoon is a different piece from a 2-2, and
            // every local paper writes it differently.
            NewsStoryKind::GoallessDraw
        } else {
            NewsStoryKind::LeagueDraw
        }
    }

    /// A knockout tie is reported on whether the club is still in the
    /// competition, not on the margin. Going out on penalties after a
    /// draw is the story of the week however the ninety minutes read.
    fn file_cup_tie(
        out: &mut Vec<NewsStory>,
        result: &IssueResult,
        facts: &WeeklyMatchFacts,
        team_id: u32,
    ) {
        let advanced = facts
            .cup_ties
            .get(&team_id)
            .map(|tie| tie.advanced())
            .unwrap_or(result.is_win());

        let kind = if advanced {
            NewsStoryKind::CupWin
        } else {
            NewsStoryKind::CupExit
        };

        out.push(
            NewsStory::new(kind, result.date)
                .against(result.opponent_team_id)
                .about(facts.star_of(team_id, result.opponent_team_id, result.goals_for))
                .at_home(result.is_home)
                .with_numbers(result.goals_for as i32, result.goals_against as i32),
        );
    }

    /// Streaks are read off the senior side's own match log rather than
    /// this week's fixtures — a run is by definition longer than a week.
    fn file_runs(out: &mut Vec<NewsStory>, results: &[IssueResult], team: &Team) {
        let Some(latest) = results.last() else {
            return;
        };

        let mut wins = 0u8;
        let mut unbeaten = 0u8;
        let mut winless = 0u8;
        let mut counting_wins = true;
        let mut counting_unbeaten = true;
        let mut counting_winless = true;

        for item in team.match_history.items().iter().rev() {
            let scored = item.score.0.get();
            let conceded = item.score.1.get();
            let won = scored > conceded;
            let lost = scored < conceded;

            if counting_wins {
                if won {
                    wins = wins.saturating_add(1);
                } else {
                    counting_wins = false;
                }
            }
            if counting_unbeaten {
                if lost {
                    counting_unbeaten = false;
                } else {
                    unbeaten = unbeaten.saturating_add(1);
                }
            }
            if counting_winless {
                if won {
                    counting_winless = false;
                } else {
                    winless = winless.saturating_add(1);
                }
            }
            if !counting_wins && !counting_unbeaten && !counting_winless {
                break;
            }
        }

        if wins >= 3 {
            out.push(
                NewsStory::new(NewsStoryKind::WinningRun, latest.date)
                    .with_numbers(wins as i32, 0)
                    .weighted((wins as i32 - 3) * 25),
            );
        } else if winless >= 4 {
            out.push(
                NewsStory::new(NewsStoryKind::WinlessRun, latest.date)
                    .with_numbers(winless as i32, 0)
                    .weighted((winless as i32 - 4) * 25),
            );
        }

        // An unbeaten run made mostly of draws is its own story, and the
        // only one the club has on a week it drew again. It never runs
        // alongside the winning-run piece, which already says more.
        if wins < 3 && unbeaten >= 6 {
            out.push(
                NewsStory::new(NewsStoryKind::UnbeatenRun, latest.date)
                    .with_numbers(unbeaten as i32, 0)
                    .weighted((unbeaten as i32 - 6) * 20),
            );
        }
    }
}

/// The table story: who is climbing and who is in trouble.
pub struct TableDesk;

impl TableDesk {
    /// Below this share of the season the table is noise, and no serious
    /// paper leads on it.
    const MIN_PROGRESS: f32 = 0.30;

    pub fn file(out: &mut Vec<NewsStory>, standing: Option<StandingSnapshot>, date: NaiveDate) {
        let Some(standing) = standing else {
            return;
        };
        if standing.teams == 0 || standing.progress() < Self::MIN_PROGRESS {
            return;
        }

        let position = standing.position as i32;
        let points = standing.points as i32;

        if standing.position <= 3 {
            out.push(
                NewsStory::new(NewsStoryKind::TitleCharge, date)
                    .with_numbers(position, points)
                    // Leading the table is a bigger story than third.
                    .weighted((4 - position) * 30),
            );
            return;
        }

        let drop_edge = standing.teams.saturating_sub(3);
        if standing.position > drop_edge {
            out.push(
                NewsStory::new(NewsStoryKind::RelegationFight, date)
                    .with_numbers(position, points)
                    // Deeper in the mire, and later in the season, hurts more.
                    .weighted(
                        (position - drop_edge as i32) * 20 + (standing.progress() * 60.0) as i32,
                    ),
            );
        }
    }
}

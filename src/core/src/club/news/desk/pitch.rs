use super::facts::{MatchDramaFacts, StandingSnapshot, WeeklyMatchFacts};
use crate::Team;
use crate::club::news::types::{IssueResult, NewsStory, NewsStoryKind, ResultCompetition};
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
            Self::file_drama(out, result, facts, team.id);

            // A European night is its own kind of evening and outranks
            // every domestic framing bar a derby. Reported before the
            // cup branch because the two stores are separate: a club
            // can play in both inside one week, and only one of them
            // is Wednesday.
            if result.competition == ResultCompetition::Continental {
                Self::file_continental(out, result, facts, team.id);
                continue;
            }

            if result.competition == ResultCompetition::Playoff {
                Self::file_playoff(out, result, facts, team.id);
                continue;
            }

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

    /// A margin this size in Europe is not a result, it is an evening
    /// somebody will still be describing in a decade.
    const CONTINENTAL_ROUT: i32 = 3;

    /// A European night, told as one.
    ///
    /// Deliberately simpler than the domestic report: no derby framing
    /// (a continental opponent is by definition not the neighbours) and
    /// no table context, because the group standings are not the club's
    /// own league. What is left is the thing a supporter actually
    /// remembers — whether the night was good, bad or a hiding.
    fn file_continental(
        out: &mut Vec<NewsStory>,
        result: &IssueResult,
        facts: &WeeklyMatchFacts,
        team_id: u32,
    ) {
        let margin = result.goals_for as i32 - result.goals_against as i32;

        let kind = if margin >= Self::CONTINENTAL_ROUT {
            NewsStoryKind::ContinentalRout
        } else if margin <= -Self::CONTINENTAL_ROUT {
            NewsStoryKind::ContinentalHiding
        } else if margin >= 0 {
            // A draw away from home in Europe is a decent night and a
            // draw at home is a poor one, but neither is a defeat —
            // both belong on the positive side of this split.
            NewsStoryKind::ContinentalNightWin
        } else {
            NewsStoryKind::ContinentalDefeat
        };

        out.push(
            NewsStory::new(kind, result.date)
                .against(result.opponent_team_id)
                .about(facts.star_of(team_id, result.opponent_team_id, result.goals_for))
                .at_home(result.is_home)
                .with_numbers(result.goals_for as i32, result.goals_against as i32),
        );
    }

    /// A playoff game, reported against the series rather than against
    /// the scoreline.
    ///
    /// The question a playoff piece has to answer is not "did they win
    /// today" but "are they still in it", and only the bracket knows
    /// that. A side can lose game two of a best-of-three and be
    /// perfectly fine; a side can win one and be out. So the series
    /// verdict leads whenever there is one, and the game report is
    /// what runs while the tie is still open.
    fn file_playoff(
        out: &mut Vec<NewsStory>,
        result: &IssueResult,
        facts: &WeeklyMatchFacts,
        team_id: u32,
    ) {
        let tie = facts.playoff.get(&team_id).copied();

        let kind = match tie {
            // One game from everything. A larger morning than simply
            // going through, and the only round distinction a
            // supporter actually needs.
            Some(tie) if tie.advanced && tie.decides_a_finalist => {
                NewsStoryKind::PlayoffFinalReached
            }
            Some(tie) if tie.advanced => NewsStoryKind::PlayoffTieWon,
            Some(tie) if tie.eliminated => NewsStoryKind::PlayoffTieLost,
            // The series is still open, or the bracket had nothing to
            // say about this fixture — either way today's game is the
            // story and tomorrow's is somebody else's problem.
            _ if result.goals_for >= result.goals_against => NewsStoryKind::PlayoffGameWin,
            _ => NewsStoryKind::PlayoffGameDefeat,
        };

        out.push(
            NewsStory::new(kind, result.date)
                .against(result.opponent_team_id)
                .about(facts.star_of(team_id, result.opponent_team_id, result.goals_for))
                .at_home(result.is_home)
                .with_numbers(result.goals_for as i32, result.goals_against as i32),
        );
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

    /// The sidebar: how one afternoon actually went, when how it went
    /// was worth a piece of its own.
    ///
    /// At most one runs per match, and it is always the biggest angle
    /// available. A 4-3 won from two down in stoppage time with ten men
    /// is four true stories, and a paper that printed all four would
    /// read like a machine — it prints the one a supporter would lead
    /// with in the pub, which is the comeback.
    fn file_drama(
        out: &mut Vec<NewsStory>,
        result: &IssueResult,
        facts: &WeeklyMatchFacts,
        team_id: u32,
    ) {
        let Some(drama) = facts.drama_of(team_id, result.opponent_team_id, result.goals_for) else {
            return;
        };

        // Ordered by how a town would rank the afternoon, not by how
        // rare the flag is.
        let (kind, figure) = if drama.won && drama.max_deficit >= 2 {
            (NewsStoryKind::ComebackWin, drama.max_deficit as i32)
        } else if !drama.won && drama.max_lead >= 2 {
            (NewsStoryKind::LeadThrownAway, drama.max_lead as i32)
        } else if drama.winner_minute >= MatchDramaFacts::STOPPAGE_MINUTE {
            (NewsStoryKind::StoppageTimeDrama, drama.winner_minute as i32)
        } else if drama.winner_minute >= MatchDramaFacts::LATE_MINUTE {
            (NewsStoryKind::LateWinner, drama.winner_minute as i32)
        } else if drama.won && drama.red_card {
            (NewsStoryKind::TenManWin, 10)
        } else if drama.total_goals >= 6 {
            (NewsStoryKind::GoalFest, drama.total_goals as i32)
        } else if drama.early_goals >= 2 {
            (NewsStoryKind::EarlyBlitz, drama.early_goals as i32)
        } else if drama.reply_minutes > 0 {
            (NewsStoryKind::InstantReply, drama.reply_minutes as i32)
        } else {
            return;
        };

        // `a` carries the figure the piece is actually about — the
        // minute, the deficit, the goal count — rather than the goals
        // scored, which is what the report alongside it is for. So none
        // of this desk's drama copy may print `{score}`: it would set
        // "89-1" under a stoppage-time winner. The run-of-form and
        // table pieces have used `a` this way since they were written.
        out.push(
            NewsStory::new(kind, result.date)
                .against(result.opponent_team_id)
                .about(facts.star_of(team_id, result.opponent_team_id, result.goals_for))
                .at_home(result.is_home)
                .with_numbers(figure, result.goals_against as i32),
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

        Self::file_scoring_runs(out, team, latest.date);
        Self::file_ground_runs(out, team, latest.date);
    }

    /// The two runs a phone-in argues about separately. A side in a bad
    /// month is either not scoring or not defending, and which of the
    /// two it is decides who gets blamed for it.
    fn file_scoring_runs(out: &mut Vec<NewsStory>, team: &Team, date: NaiveDate) {
        let mut goalless = 0u8;
        let mut leaky = 0u8;
        let mut counting_goalless = true;
        let mut counting_leaky = true;

        for item in team.match_history.items().iter().rev() {
            if counting_goalless {
                if item.score.0.get() == 0 {
                    goalless = goalless.saturating_add(1);
                } else {
                    counting_goalless = false;
                }
            }
            if counting_leaky {
                if item.score.1.get() >= 2 {
                    leaky = leaky.saturating_add(1);
                } else {
                    counting_leaky = false;
                }
            }
            if !counting_goalless && !counting_leaky {
                break;
            }
        }

        if goalless >= 3 {
            out.push(
                NewsStory::new(NewsStoryKind::GoalsDriedUp, date)
                    .with_numbers(goalless as i32, 0)
                    .weighted((goalless as i32 - 3) * 25),
            );
        }

        if leaky >= 4 {
            out.push(
                NewsStory::new(NewsStoryKind::DefensiveCrisis, date)
                    .with_numbers(leaky as i32, 0)
                    .weighted((leaky as i32 - 4) * 25),
            );
        }
    }

    /// Form with an address on it. Home and away are different games and
    /// a town talks about them as different games — "nobody wins here"
    /// and "we cannot win away" are both said about the same season.
    fn file_ground_runs(out: &mut Vec<NewsStory>, team: &Team, date: NaiveDate) {
        let mut home_unbeaten = 0u8;
        let mut away_wins = 0u8;
        let mut counting_home = true;
        let mut counting_away = true;

        for item in team.match_history.items().iter().rev() {
            let scored = item.score.0.get();
            let conceded = item.score.1.get();

            if item.is_home {
                if counting_home {
                    if scored >= conceded {
                        home_unbeaten = home_unbeaten.saturating_add(1);
                    } else {
                        counting_home = false;
                    }
                }
            } else if counting_away {
                if scored > conceded {
                    away_wins = away_wins.saturating_add(1);
                } else {
                    counting_away = false;
                }
            }

            if !counting_home && !counting_away {
                break;
            }
        }

        if home_unbeaten >= 8 {
            out.push(
                NewsStory::new(NewsStoryKind::FortressHome, date)
                    .with_numbers(home_unbeaten as i32, 0)
                    .at_home(true)
                    .weighted((home_unbeaten as i32 - 8) * 20),
            );
        }

        if away_wins >= 3 {
            out.push(
                NewsStory::new(NewsStoryKind::AwayDayForm, date)
                    .with_numbers(away_wins as i32, 0)
                    .weighted((away_wins as i32 - 3) * 25),
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

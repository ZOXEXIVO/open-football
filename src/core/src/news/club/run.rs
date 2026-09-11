use super::edition::{TeamPressRun, TransferSplit};
use super::{WeeklyDugout, WeeklyLoanWatch, WeeklyMarket};
use crate::club::news::{
    IssueResult, NewspaperIssue, NextFixture, ResultCompetition, StandingSnapshot, WeeklyMatchFacts,
};
use crate::league::League;
use crate::{Club, Country, Team, TeamType};
use chrono::{Duration, NaiveDate};
use rustc_hash::FxHashSet;

/// One club's week, turned into as many editions as it has branded
/// sides. Squads with no brand of their own (Reserve, U18..U23) are read
/// about in the first team's.
pub(super) struct ClubPressRun;

impl ClubPressRun {
    pub(super) fn compile(
        club: &Club,
        country: &Country,
        facts: &WeeklyMatchFacts,
        market: &WeeklyMarket,
        loans: &WeeklyLoanWatch,
        dugout: &WeeklyDugout,
        week_start: NaiveDate,
        week_end: NaiveDate,
    ) -> Vec<(u32, NewspaperIssue)> {
        let papers: Vec<&Team> = club
            .teams
            .iter()
            .filter(|team| team.team_type.is_own_team())
            .collect();

        // The club's page of record: the first team, or — for the odd
        // club whose data carries no Main side at all — whichever branded
        // side it does have. It carries the club-wide desks on top of its
        // own football.
        let Some(flagship) = club
            .teams
            .iter()
            .find(|team| team.team_type == TeamType::Main)
            .or_else(|| papers.first().copied())
        else {
            return Vec::new();
        };

        let week: Vec<Vec<IssueResult>> = papers
            .iter()
            .map(|team| Self::results(team, facts, week_start, week_end))
            .collect();

        let fixtures: Vec<Option<NextFixture>> = papers
            .iter()
            .map(|team| Self::next_fixture(team, country, week_end))
            .collect();

        // Resolving rivals means walking the country's club list, so it
        // only happens on the weeks one of the sides played or is about
        // to — a derby is recognised from the opponent alone, both ways.
        let rivals = if week.iter().any(|results| !results.is_empty())
            || fixtures.iter().any(Option::is_some)
        {
            Self::rival_team_ids(club, country)
        } else {
            FxHashSet::default()
        };

        let transfers = market
            .for_club(club.id)
            .map(|business| TransferSplit::by_team(club, business, flagship.id))
            .unwrap_or_default();
        let peak_value = Self::squad_peak_value(club);
        // Every side the club owns, branded or not. The ratings page
        // reports football played by one of these and nothing else.
        let sides: FxHashSet<u32> = club.teams.iter().map(|team| team.id).collect();

        papers
            .iter()
            .zip(week)
            .zip(fixtures)
            .filter_map(|((team, results), fixture)| {
                let is_flagship = team.id == flagship.id;

                let edition = TeamPressRun {
                    club,
                    team,
                    rosters: Self::rosters(club, team, is_flagship),
                    sides: &sides,
                    results,
                    standing: Self::standing(team.league_id, country, team.id),
                    rivals: &rivals,
                    facts,
                    transfers: transfers.get(&team.id),
                    peak_value,
                    // The loan column belongs to the page of record: a
                    // loanee's contract is held by the club, and nothing
                    // says which of its sides he would be playing for.
                    loans: if is_flagship {
                        loans.for_club(club.id)
                    } else {
                        None
                    },
                    // The dugout belongs to the club, not to one of its
                    // sides — a B team does not have its own manager
                    // market — so the pursuit column runs in the page of
                    // record alongside the rest of the boardroom.
                    dugout: if is_flagship {
                        dugout.for_club(club.id)
                    } else {
                        None
                    },
                    fixture,
                    // The club's own pursuits and the calendar are the
                    // page of record's, like the rest of the boardroom.
                    targets: if is_flagship {
                        market.for_targets(club.id)
                    } else {
                        None
                    },
                    window: if is_flagship {
                        market.window_for(country.id, club.id)
                    } else {
                        None
                    },
                    week_start,
                    flagship: is_flagship,
                }
                .compile(week_end)?;

                Some((team.id, edition))
            })
            .collect()
    }

    /// The squads one paper speaks for: its own side, plus — for the page
    /// of record — every squad with no brand of its own. Between them a
    /// club's papers cover each of its players exactly once, so nobody's
    /// week is reported twice and nobody's is dropped.
    fn rosters<'a>(club: &'a Club, team: &'a Team, is_flagship: bool) -> Vec<&'a Team> {
        let mut rosters = vec![team];
        if is_flagship {
            rosters.extend(
                club.teams
                    .iter()
                    .filter(|other| !other.team_type.is_own_team()),
            );
        }
        rosters
    }

    /// This side's fixtures inside the window, newest last so the results
    /// panel reads in the order they were played. A tie the knockout
    /// sweep saw is marked as one — the match log itself does not record
    /// which competition a fixture belonged to.
    fn results(
        team: &Team,
        facts: &WeeklyMatchFacts,
        week_start: NaiveDate,
        week_end: NaiveDate,
    ) -> Vec<IssueResult> {
        let cup_opponent = facts.cup_ties.get(&team.id).map(|tie| tie.opponent_team_id);
        // A European night is labelled before a cup tie is considered:
        // the two stores are separate and a club can play in both
        // inside one week, but only one of them is Wednesday.
        let continental_opponent = facts
            .continental
            .get(&team.id)
            .map(|night| night.opponent_team_id);
        let playoff_opponent = facts.playoff.get(&team.id).map(|tie| tie.opponent_team_id);

        team.match_history
            .items()
            .iter()
            .filter(|item| {
                let played = item.date.date();
                played >= week_start && played < week_end
            })
            .map(|item| IssueResult {
                date: item.date.date(),
                opponent_team_id: item.rival_team_id,
                goals_for: item.score.0.get(),
                goals_against: item.score.1.get(),
                competition: if continental_opponent == Some(item.rival_team_id) {
                    ResultCompetition::Continental
                } else if playoff_opponent == Some(item.rival_team_id) {
                    ResultCompetition::Playoff
                } else if cup_opponent == Some(item.rival_team_id) {
                    ResultCompetition::Cup
                } else {
                    ResultCompetition::League
                },
                is_home: item.is_home,
            })
            .collect()
    }

    /// How far ahead the preview desk looks. Eight days: the whole of
    /// next week, and a Monday fixture on the far side of it.
    const PREVIEW_HORIZON_DAYS: i64 = 8;

    /// The side's next fixture, read off every competition schedule in
    /// the country: the nearest unplayed match inside the horizon. A
    /// cup tie in midweek beats the league game after it because it is
    /// played first, and the cup schedule is a separate league object,
    /// so every schedule is asked.
    fn next_fixture(team: &Team, country: &Country, week_end: NaiveDate) -> Option<NextFixture> {
        let horizon = week_end + Duration::days(Self::PREVIEW_HORIZON_DAYS);
        let mut nearest: Option<(chrono::NaiveDateTime, u32, bool, bool)> = None;

        for league in &country.leagues.leagues {
            if league.friendly {
                continue;
            }
            for item in league.schedule.get_matches_for_team(team.id) {
                if item.result.is_some() {
                    continue;
                }
                let day = item.date.date();
                if day < week_end || day >= horizon {
                    continue;
                }
                if nearest.is_none_or(|(when, ..)| item.date < when) {
                    let is_home = item.home_team_id == team.id;
                    let opponent = if is_home {
                        item.away_team_id
                    } else {
                        item.home_team_id
                    };
                    nearest = Some((item.date, opponent, is_home, league.is_cup));
                }
            }
        }

        let (_, opponent_team_id, is_home, is_cup) = nearest?;
        if opponent_team_id == 0 {
            return None;
        }

        // Where they sit in OUR table — nothing when they are not in it.
        let opponent_position = team
            .league_id
            .and_then(|league_id| {
                country
                    .leagues
                    .leagues
                    .iter()
                    .find(|league| league.id == league_id && !league.friendly)
            })
            .and_then(|league| {
                league
                    .table
                    .get()
                    .iter()
                    .position(|row| row.team_id == opponent_team_id)
            })
            .map(|index| (index + 1).min(u8::MAX as usize) as u8)
            .unwrap_or(0);

        // One of ours who used to be one of theirs: the most important
        // such man on the roster, by the paper's own measure.
        let old_boy_player_id = Self::team_slug(country, opponent_team_id)
            .and_then(|slug| {
                team.players
                    .iter()
                    .filter(|player| !player.is_retired())
                    .filter(|player| {
                        player
                            .statistics_history
                            .career_team_slugs()
                            .contains(&slug)
                    })
                    .max_by_key(|player| {
                        (
                            crate::club::news::PlayerStanding::importance(player),
                            player.id,
                        )
                    })
                    .map(|player| player.id)
            })
            .unwrap_or(0);

        Some(NextFixture {
            opponent_team_id,
            is_home,
            is_cup,
            opponent_position,
            old_boy_player_id,
        })
    }

    fn team_slug(country: &Country, team_id: u32) -> Option<&str> {
        country
            .clubs
            .iter()
            .flat_map(|club| club.teams.iter())
            .find(|team| team.id == team_id)
            .map(|team| team.slug.as_str())
    }

    /// Rival clubs resolved down to the team ids a match report actually
    /// carries, so a derby is recognised from the opponent alone.
    fn rival_team_ids(club: &Club, country: &Country) -> FxHashSet<u32> {
        let mut ids = FxHashSet::default();
        for rival_club_id in &club.rivals {
            if let Some(rival) = country.clubs.iter().find(|c| c.id == *rival_club_id) {
                for team in rival.teams.teams.iter() {
                    ids.insert(team.id);
                }
            }
        }
        ids
    }

    fn standing(
        league_id: Option<u32>,
        country: &Country,
        team_id: u32,
    ) -> Option<StandingSnapshot> {
        let league_id = league_id?;
        let league = country
            .leagues
            .leagues
            .iter()
            .find(|league| league.id == league_id && !league.friendly)?;

        let rows = league.table.get();
        let index = rows.iter().position(|row| row.team_id == team_id)?;
        let row = &rows[index];

        Some(StandingSnapshot {
            position: (index + 1) as u8,
            teams: rows.len().min(u8::MAX as usize) as u8,
            points: row.effective_points(),
            played: row.played,
            // A round-robin double programme: every side plays each of
            // the others home and away.
            total_rounds: Self::total_rounds(league, rows.len()),
        })
    }

    /// How long the programme is. The schedule knows exactly; a league
    /// whose schedule has not been drawn yet is assumed to be the usual
    /// double round-robin.
    fn total_rounds(league: &League, teams: usize) -> u8 {
        let rounds = if league.schedule.tours.is_empty() {
            teams.saturating_sub(1) * 2
        } else {
            league.schedule.tours.len()
        };
        rounds.min(u8::MAX as usize) as u8
    }

    /// The most valuable player currently on the books — the yardstick
    /// the market desk measures a fee against.
    fn squad_peak_value(club: &Club) -> i64 {
        club.teams
            .iter()
            .filter(|team| team.team_type.is_own_team())
            .flat_map(|team| team.players.iter())
            .map(|player| player.player_attributes.value as i64)
            .max()
            .unwrap_or(0)
    }
}

use crate::club::news::{
    BoardroomDesk, ClubTransferWeek, IssueResult, MarketDesk, MatchDesk, NewsEditor, NewsStory,
    NewspaperIssue, PressMood, SquadDesk, StandingSnapshot, TableDesk, WeeklyMatchFacts,
};
use crate::r#match::MatchResult;
use crate::r#match::player::statistics::MatchStatisticType;
use crate::simulator::SimulatorData;
use crate::{Club, Country, Team, TeamType};
use chrono::NaiveDate;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

/// Weekly press run. Every club's local paper goes to print on the same
/// Monday morning, covering the seven days just gone.
///
/// The pass is deliberately laid out as gather-then-write: all the
/// reading happens in parallel over an immutable world, and only the
/// finished editions are applied under `&mut`. That keeps it off the
/// critical path of the daily tick and away from the borrow gymnastics
/// the transfer pipeline needs.
pub(crate) struct NewsroomTick;

impl NewsroomTick {
    pub(crate) fn run(data: &mut SimulatorData, week_start: NaiveDate, week_end: NaiveDate) {
        let facts = WeeklyMatchFacts::from_world(data, week_start, week_end);
        let market = WeeklyMarket::from_world(data, week_start, week_end);

        let editions: Vec<(u32, NewspaperIssue)> = data
            .continents
            .par_iter()
            .flat_map(|continent| continent.countries.par_iter())
            .flat_map_iter(|country| {
                country.clubs.iter().filter_map(|club| {
                    ClubPressRun::compile(club, country, &facts, &market, week_start, week_end)
                        .map(|issue| (club.id, issue))
                })
            })
            .collect();

        for (club_id, issue) in editions {
            if let Some(club) = data.club_mut(club_id) {
                club.newsroom.publish(issue);
            }
        }
    }
}

impl WeeklyMatchFacts {
    /// Read the week's completed matches once and keep only the two
    /// things no club can work out for itself: who scored three or more
    /// in a single game, and who was sent off.
    ///
    /// Domestic fixtures live in each competition's own store; only
    /// national and continental ties reach the global one. Both are
    /// walked, or the desk would never see a hat-trick in league
    /// football — which is nearly all of it.
    fn from_world(data: &SimulatorData, week_start: NaiveDate, week_end: NaiveDate) -> Self {
        let mut facts = data
            .continents
            .par_iter()
            .flat_map(|continent| continent.countries.par_iter())
            .map(|country| {
                let mut country_facts = WeeklyMatchFacts::empty();

                let cup = country
                    .domestic_cup
                    .as_ref()
                    .map(|cup| &cup.league)
                    .into_iter();
                let playoffs = country.playoffs.iter().map(|playoff| &playoff.league);

                for league in country
                    .leagues
                    .leagues
                    .iter()
                    .chain(cup)
                    .chain(playoffs)
                    .filter(|league| !league.friendly)
                {
                    country_facts.absorb(league.matches.iter_in_range(week_start, week_end));
                }

                country_facts
            })
            .reduce(WeeklyMatchFacts::empty, WeeklyMatchFacts::merged);

        facts.absorb(data.match_store.iter_in_range(week_start, week_end));
        facts
    }

    /// Fold one competition's week of results into the tally.
    fn absorb<'a>(&mut self, results: impl Iterator<Item = &'a MatchResult>) {
        let mut per_match: FxHashMap<u32, u8> = FxHashMap::default();

        for result in results {
            if result.friendly {
                continue;
            }

            per_match.clear();

            for goal in &result.score.details {
                match goal.stat_type {
                    MatchStatisticType::Goal if !goal.is_auto_goal => {
                        *per_match.entry(goal.player_id).or_insert(0) += 1;
                    }
                    MatchStatisticType::RedCard => {
                        self.red_cards.insert(goal.player_id);
                    }
                    _ => {}
                }
            }

            for (player_id, goals) in per_match.iter() {
                if *goals < 3 {
                    continue;
                }
                let best = self.hat_tricks.entry(*player_id).or_insert(0);
                *best = (*best).max(*goals);
            }
        }
    }

    /// Rayon fold partner: two half-worlds become one.
    fn merged(mut self, other: WeeklyMatchFacts) -> Self {
        for (player_id, goals) in other.hat_tricks {
            let best = self.hat_tricks.entry(player_id).or_insert(0);
            *best = (*best).max(goals);
        }
        self.red_cards.extend(other.red_cards);
        self
    }
}

/// The week's completed transfer business, bucketed by club. Both sides
/// of a deal are recorded, so a club hears about its own departures even
/// though the player has already left the roster.
struct WeeklyMarket {
    by_club: FxHashMap<u32, ClubTransferWeek>,
}

impl WeeklyMarket {
    fn from_world(data: &SimulatorData, week_start: NaiveDate, week_end: NaiveDate) -> Self {
        let mut by_club: FxHashMap<u32, ClubTransferWeek> = FxHashMap::default();

        for continent in &data.continents {
            for country in &continent.countries {
                // The history is append-ordered and never trimmed, so
                // walk back from the newest entry and stop as soon as
                // the window closes rather than scanning a decade of
                // completed deals every Monday.
                for transfer in country
                    .transfer_market
                    .transfer_history
                    .iter()
                    .rev()
                    .take_while(|transfer| transfer.transfer_date >= week_start)
                {
                    if transfer.transfer_date >= week_end {
                        continue;
                    }
                    if transfer.to_club_id != 0 {
                        by_club
                            .entry(transfer.to_club_id)
                            .or_default()
                            .absorb(transfer.to_club_id, transfer);
                    }
                    if transfer.from_club_id != 0 {
                        by_club
                            .entry(transfer.from_club_id)
                            .or_default()
                            .absorb(transfer.from_club_id, transfer);
                    }
                }
            }
        }

        WeeklyMarket { by_club }
    }

    fn for_club(&self, club_id: u32) -> Option<&ClubTransferWeek> {
        self.by_club.get(&club_id)
    }
}

/// Everything that turns one club's week into one printed edition.
struct ClubPressRun;

impl ClubPressRun {
    /// Recent matches the press mood is read from — roughly a month of
    /// football, which is how far back a supporter's memory really runs.
    const FORM_WINDOW: usize = 6;

    fn compile(
        club: &Club,
        country: &Country,
        facts: &WeeklyMatchFacts,
        market: &WeeklyMarket,
        week_start: NaiveDate,
        week_end: NaiveDate,
    ) -> Option<NewspaperIssue> {
        let main = club
            .teams
            .teams
            .iter()
            .find(|team| team.team_type == TeamType::Main)?;

        let results = Self::results(main, week_start, week_end);
        let transfers = market.for_club(club.id);

        let mut candidates: Vec<NewsStory> = Vec::new();

        // Resolving rivals means walking the country's club list, so it
        // only happens on the weeks the club actually played.
        if !results.is_empty() {
            let rivals = Self::rival_team_ids(club, country);
            MatchDesk::file(&mut candidates, &results, &rivals, main);
        }
        TableDesk::file(
            &mut candidates,
            Self::standing(main.league_id, country, main.id),
            week_end,
        );
        SquadDesk::file(&mut candidates, club, facts, !results.is_empty(), week_end);
        if let Some(transfers) = transfers {
            MarketDesk::file(
                &mut candidates,
                transfers,
                Self::squad_peak_value(club),
                week_end,
            );
        }
        BoardroomDesk::file(&mut candidates, club, week_end);

        let stories = NewsEditor::compile(candidates, &club.newsroom.issues);

        // A paper with nothing at all to say does not go to print. That
        // keeps dormant clubs (no fixtures, no squad churn) from filling
        // the shelf with blank sheets.
        if stories.is_empty() && results.is_empty() {
            return None;
        }

        let mood = Self::mood(club, main, &results);

        Some(NewspaperIssue {
            number: club.newsroom.next_number,
            date: week_end,
            mood,
            stories,
            results,
        })
    }

    /// The senior side's fixtures inside the window, newest last so the
    /// results panel reads in the order they were played.
    fn results(main: &Team, week_start: NaiveDate, week_end: NaiveDate) -> Vec<IssueResult> {
        main.match_history
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
            })
            .collect()
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
            total_rounds: (rows.len().saturating_sub(1) * 2).min(u8::MAX as usize) as u8,
        })
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

    fn mood(club: &Club, main: &Team, results: &[IssueResult]) -> PressMood {
        let week = results.iter().fold((0u8, 0u8, 0u8), |mut tally, result| {
            if result.is_win() {
                tally.0 += 1;
            } else if result.is_draw() {
                tally.1 += 1;
            } else {
                tally.2 += 1;
            }
            tally
        });

        let form = main.match_history.recent_wins_ratio(Self::FORM_WINDOW);

        // Board confidence read as pressure: a chairman at 100 applies
        // none, a chairman at 0 is already drafting the statement.
        let pressure = if club.board.manager_on_final_warning {
            1.0
        } else {
            ((100 - club.board.confidence.level.clamp(0, 100)) as f32 / 100.0).clamp(0.0, 1.0)
        };

        PressMood::read(week, form, pressure)
    }
}

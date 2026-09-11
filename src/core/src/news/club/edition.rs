use crate::club::news::{
    BoardroomDesk, ClubDugoutWatch, ClubLoanWatch, ClubTargetsWeek, ClubTransferWeek, DugoutDesk,
    FansDesk, IssueResult, LoanDesk, MarketDesk, MatchDesk, NewsEditor, NewsStory, NewspaperIssue,
    NextFixture, PressMood, PreviewDesk, ResultCompetition, SquadDesk, StandingSnapshot, TableDesk,
    TargetsDesk, TownMood, WeeklyMatchFacts, WindowWeek,
};
use crate::{Club, Team};
use chrono::NaiveDate;
use rustc_hash::{FxHashMap, FxHashSet};

/// Everything that turns one side's week into one printed edition.
pub(super) struct TeamPressRun<'a> {
    pub(super) club: &'a Club,
    pub(super) team: &'a Team,
    /// The squads this paper speaks for.
    pub(super) rosters: Vec<&'a Team>,
    /// Every side the club owns — the football this paper is entitled to
    /// report as its own. Shared across the club's editions because it
    /// is a property of the club, not of one of its papers.
    pub(super) sides: &'a FxHashSet<u32>,
    pub(super) results: Vec<IssueResult>,
    pub(super) standing: Option<StandingSnapshot>,
    pub(super) rivals: &'a FxHashSet<u32>,
    pub(super) facts: &'a WeeklyMatchFacts,
    pub(super) transfers: Option<&'a ClubTransferWeek>,
    pub(super) peak_value: i64,
    pub(super) loans: Option<&'a ClubLoanWatch>,
    pub(super) dugout: Option<&'a ClubDugoutWatch>,
    /// This side's next fixture, when the schedules hold one inside
    /// the preview horizon.
    pub(super) fixture: Option<NextFixture>,
    /// The club's own pursuits in the market. Page of record only.
    pub(super) targets: Option<&'a ClubTargetsWeek>,
    /// What the registration window did this week. Page of record only.
    pub(super) window: Option<WindowWeek>,
    /// First day of the window the edition covers — the cut-off the
    /// club's own diary is read back to.
    pub(super) week_start: NaiveDate,
    /// This side is the club's page of record.
    pub(super) flagship: bool,
}

impl TeamPressRun<'_> {
    /// Recent matches the press mood is read from — roughly a month of
    /// football, which is how far back a supporter's memory really runs.
    const FORM_WINDOW: usize = 6;

    pub(super) fn compile(self, date: NaiveDate) -> Option<NewspaperIssue> {
        let played_this_week = !self.results.is_empty();
        let mut candidates: Vec<NewsStory> = Vec::new();

        if played_this_week {
            MatchDesk::file(
                &mut candidates,
                &self.results,
                self.rivals,
                self.facts,
                self.team,
            );
            // The man in the dugout, on the week's last match. A vacant
            // seat resolves to a stub with no id, and the desk stays
            // quiet rather than quote nobody.
            DugoutDesk::file_verdict(
                &mut candidates,
                &self.results,
                self.team.staffs.head_coach().id,
                date,
            );
        }
        let league_games_this_week = self
            .results
            .iter()
            .filter(|result| result.competition == ResultCompetition::League)
            .count()
            .min(u8::MAX as usize) as u8;
        TableDesk::file(&mut candidates, self.standing, league_games_this_week, date);
        PreviewDesk::file(
            &mut candidates,
            self.fixture,
            self.standing,
            self.rivals,
            date,
        );

        // One walk over this paper's players feeds the squad, dugout,
        // terraces, market-verdict and rumour desks, and comes back with
        // the moods that only exist across a whole dressing room.
        let pulse = SquadDesk::file(
            &mut candidates,
            &self.rosters,
            self.sides,
            self.facts,
            played_this_week,
            date,
        );

        if let Some(transfers) = self.transfers {
            MarketDesk::file(&mut candidates, transfers, self.peak_value, date);
        }
        if let Some(targets) = self.targets {
            TargetsDesk::file(&mut candidates, targets, date);
        }
        if let Some(watch) = self.loans {
            LoanDesk::file(&mut candidates, watch, date);
        }
        DugoutDesk::file_club(&mut candidates, &pulse, date);
        FansDesk::file_club(
            &mut candidates,
            &pulse,
            TownMood {
                supporter_pressure: self.club.board.pressure.supporter_pressure,
                standing: self.standing,
                results: &self.results,
                transfers: self.transfers,
                peak_value: self.peak_value,
            },
            date,
        );

        // The boardroom belongs to the club, so it only ever runs in the
        // page of record — a reserve side's paper reporting the sacking
        // reads as if the reserve side had done the sacking.
        if self.flagship {
            BoardroomDesk::file(
                &mut candidates,
                self.club,
                &pulse,
                self.dugout,
                self.window,
                self.week_start,
                date,
            );
        }

        let stories = NewsEditor::compile(candidates, &self.team.newsroom.issues);

        // A paper with nothing at all to say does not go to print. That
        // keeps dormant sides (no fixtures, no squad churn) from filling
        // the shelf with blank sheets.
        if stories.is_empty() && self.results.is_empty() {
            return None;
        }

        let mood = self.mood();

        Some(NewspaperIssue {
            number: self.team.newsroom.next_number,
            date,
            mood,
            stories,
            results: self.results,
        })
    }

    fn mood(&self) -> PressMood {
        let week = self
            .results
            .iter()
            .fold((0u8, 0u8, 0u8), |mut tally, result| {
                if result.is_win() {
                    tally.0 += 1;
                } else if result.is_draw() {
                    tally.1 += 1;
                } else {
                    tally.2 += 1;
                }
                tally
            });

        let form = self.team.match_history.recent_wins_ratio(Self::FORM_WINDOW);

        // Board confidence read as pressure: a chairman at 100 applies
        // none, a chairman at 0 is already drafting the statement. Only
        // the page of record carries it — the board does not sack the
        // manager of the "2" side over the first team's results.
        let pressure = if !self.flagship {
            0.0
        } else if self.club.board.manager_on_final_warning {
            1.0
        } else {
            ((100 - self.club.board.confidence.level.clamp(0, 100)) as f32 / 100.0).clamp(0.0, 1.0)
        };

        PressMood::read(week, form, pressure)
    }
}

/// Splits a club's completed transfer business between its papers.
///
/// An arrival is news for the side he actually joined: a "{Club} 2"
/// paper reporting its own signing is the whole point of it having a
/// paper. Everything else — departures, and arrivals into a squad with
/// no paper of its own — goes to the page of record, which is where a
/// reader would look for a player who is no longer anywhere on the books.
pub(super) struct TransferSplit;

impl TransferSplit {
    pub(super) fn by_team(
        club: &Club,
        business: &ClubTransferWeek,
        flagship_id: u32,
    ) -> FxHashMap<u32, ClubTransferWeek> {
        let mut split: FxHashMap<u32, ClubTransferWeek> = FxHashMap::default();

        for arrival in &business.arrivals {
            let team_id = Self::squad_of(club, arrival.player_id).unwrap_or(flagship_id);
            split.entry(team_id).or_default().arrivals.push(*arrival);
        }

        if !business.departures.is_empty() {
            split
                .entry(flagship_id)
                .or_default()
                .departures
                .extend(business.departures.iter().copied());
        }

        split
    }

    /// Which of the club's papers holds the player now. `None` when he
    /// is in a squad that prints nothing, or has already left.
    fn squad_of(club: &Club, player_id: u32) -> Option<u32> {
        club.teams
            .iter()
            .filter(|team| team.team_type.is_own_team())
            .find(|team| team.players.iter().any(|player| player.id == player_id))
            .map(|team| team.id)
    }
}

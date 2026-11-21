use crate::club::player::events::discipline::CompetitionFamily;
use crate::club::staff::goalkeeping::KeeperSelectionBrief;
use crate::club::team::ClubWeek;
use crate::league::{League, TableLine, TableStakes, TableStanding};
use crate::r#match::squad::selection::model::{FixtureEligibility, MatchSelectionGameModel};
use crate::r#match::{FixtureRoom, SelectionCompetition, SelectionContext};
use crate::{Club, Team, TeamType};
use chrono::NaiveDate;
use rustc_hash::FxHashMap;

impl League {
    /// The lines this table turns on: its own rules, and the continental
    /// places the continent has named.
    pub fn table_lines(&self) -> Vec<TableLine> {
        let mut lines = TableLine::of_league(&self.settings, self.table.rows.len());
        lines.extend(self.continental_lines.iter().copied());
        lines
    }

    /// What a league fixture is worth today to every side in the table.
    pub(crate) fn table_stakes(&self) -> FxHashMap<u32, f32> {
        let lines = self.table_lines();
        let standings = TableStanding::of_league(&self.table);
        let mut left: FxHashMap<u32, u16> = FxHashMap::default();
        for item in self
            .schedule
            .tours
            .iter()
            .flat_map(|tour| &tour.items)
            .filter(|item| item.result.is_none())
        {
            *left.entry(item.home_team_id).or_default() += 1;
            *left.entry(item.away_team_id).or_default() += 1;
        }
        self.table
            .rows
            .iter()
            .map(|row| {
                let stakes = TableStakes::of(
                    &standings,
                    row.team_id,
                    u16::from(row.played),
                    left.get(&row.team_id).copied().unwrap_or(0),
                    &lines,
                );
                (row.team_id, stakes)
            })
            .collect()
    }
}

/// One side of a fixture as its selection is told of it. Every competition
/// that puts a club on a pitch builds its context here, so a club is read
/// the same way in the league, the cup and on a continental night.
pub(crate) struct MatchdaySide<'a> {
    pub team: &'a Team,
    pub club: Option<&'a Club>,
    pub opponent: &'a Team,
    pub home: bool,
    /// What the fixture is worth to this side, 0..1.
    pub stakes: f32,
    pub competition: SelectionCompetition,
    pub derby: bool,
    pub friendly: bool,
    pub date: NaiveDate,
    pub season_matches_played: Option<f32>,
    /// The cup the fixture belongs to, when it is one.
    pub cup_slug: Option<&'a str>,
    /// Substitutes the competition lets a side name.
    pub bench_size: u8,
}

impl MatchdaySide<'_> {
    /// A fixture nobody is trying to win.
    pub(crate) const FRIENDLY_STAKES: f32 = 0.1;

    pub(crate) fn selection_context(&self) -> SelectionContext {
        let week = if self.friendly {
            ClubWeek::free()
        } else {
            self.team.fixture_window.week(self.date)
        };
        let mut ctx = SelectionContext {
            is_friendly: self.friendly,
            date: self.date,
            match_importance: self.stakes,
            philosophy: self.club.map(|club| club.philosophy),
            competition: self.competition,
            game_model: None,
            development_guest_ids: Vec::new(),
            season_matches_played: self.season_matches_played,
            bench_size: usize::from(self.bench_size),
            keeper_brief: self.keeper_brief(),
            week,
            room: None,
            eligibility: FixtureEligibility {
                opponent_club_id: Some(self.opponent.club_id),
                cup_slug: self.cup_slug.map(str::to_string),
                family: match self.competition {
                    SelectionCompetition::ContinentalCup => CompetitionFamily::Continental,
                    _ => CompetitionFamily::Domestic,
                },
                team_slug: self.team.slug.clone(),
                team_id: self.team.id,
            },
        };
        // Friendlies skip the fixture model: the rotation selector never
        // reads it.
        if !self.friendly {
            let model = MatchSelectionGameModel::build_for_fixture(
                &ctx,
                self.team,
                self.opponent,
                self.home,
                self.derby,
            );
            ctx.room = Some(FixtureRoom::for_side(
                FixtureRoom::of(self.stakes, &week, &model.coach_tendencies),
                self.team.team_type.development_weight(),
            ));
            ctx.game_model = Some(model);
        }
        ctx
    }

    /// The goalkeeping department's word. Only the club's own first team
    /// hears it: an academy side picks its keepers on its own plan.
    fn keeper_brief(&self) -> Option<KeeperSelectionBrief> {
        if self.team.team_type != TeamType::Main {
            return None;
        }
        let standard = (self.team.playing_standard + self.opponent.playing_standard) * 0.5;
        self.club
            .and_then(|club| club.keeper_selection_brief(self.date))
            .map(|brief| brief.with_fixture_standard(standard))
    }
}

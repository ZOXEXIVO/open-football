use crate::I18n;
use chrono::NaiveDateTime;
use core::continent::CompetitionTier;
use core::league::season::LeagueSeason;
use core::{MatchHistoryItem, SimulatorData, Team};

/// A match a team played, read off the team's own match history. A
/// competition's schedule and result store both start over every season;
/// the history does not, so it is what past seasons are listed from.
pub struct PlayedFixture {
    pub kickoff: NaiveDateTime,
    /// On the competition's calendar, else the team's league's; `None` when
    /// neither is known.
    pub season: Option<LeagueSeason>,
    pub date: String,
    pub time: String,
    pub opponent_slug: String,
    pub opponent_name: String,
    pub is_home: bool,
    pub competition_name: String,
    pub home_goals: u8,
    pub away_goals: u8,
    /// Empty once the full record is no longer stored, so the score is not
    /// linked to a match page that would 404.
    pub match_id: String,
}

impl PlayedFixture {
    pub fn read(data: &SimulatorData, i18n: &I18n, team: &Team, item: &MatchHistoryItem) -> Self {
        let competition = data.league(item.league_id);
        let calendar = competition
            .or_else(|| team.league_id.and_then(|id| data.league(id)))
            .map(|league| league.settings.season_calendar());
        let day = item.date.date();

        let competition_name = match competition {
            Some(league) => league.name.clone(),
            None => CompetitionTier::from_league_id(item.league_id)
                .map(|tier| i18n.t(tier.as_i18n_key()).to_string())
                .unwrap_or_default(),
        };

        let match_id: &str = &item.match_id;
        let still_stored = data
            .match_store
            .get(match_id)
            .or_else(|| competition.and_then(|league| league.matches.get(match_id)))
            .is_some();

        let (opponent_name, opponent_slug) = match data.team_data(item.rival_team_id) {
            Some(rival) => (rival.name.clone(), rival.slug.clone()),
            None => data
                .team(item.rival_team_id)
                .map(|rival| (rival.name.clone(), rival.slug.clone()))
                .unwrap_or_else(|| (i18n.t("unknown").to_string(), String::new())),
        };

        let (us, them) = (item.score.0.get(), item.score.1.get());
        let (home_goals, away_goals) = if item.is_home { (us, them) } else { (them, us) };

        PlayedFixture {
            kickoff: item.kickoff.map_or(item.date, |time| day.and_time(time)),
            season: calendar.map(|calendar| calendar.season_of(day)),
            date: day.format("%d.%m.%Y").to_string(),
            time: item
                .kickoff
                .map(|time| time.format("%H:%M").to_string())
                .unwrap_or_default(),
            opponent_slug,
            opponent_name,
            is_home: item.is_home,
            competition_name,
            home_goals,
            away_goals,
            match_id: if still_stored {
                match_id.to_string()
            } else {
                String::new()
            },
        }
    }
}

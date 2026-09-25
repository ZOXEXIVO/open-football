//! Every match a player took the field in, read off the match histories of
//! the teams he has played for rather than from whichever team he happens
//! to be registered with today.
//!
//! A team's history is the one record holding all of its football — the
//! league, the youth sub-league, the cup, the playoffs, continental ties —
//! and keeping it across seasons, while each competition's own schedule
//! and result store start over every season. So the collector works out
//! which clubs the player has belonged to, walks every squad of those
//! clubs, and keeps the matches he started or came on in. That also covers
//! a prospect rostered with the Main squad but fielded for the academy
//! side, everything played before a move, and a player with no club at
//! all. Caps come from his own country's national sides.

use super::{PlayerMatchItem, PlayerMatchResult};
use crate::I18n;
use crate::common::played_fixture::PlayedFixture;
use chrono::NaiveDateTime;
use core::league::season::Season;
use core::r#match::MatchResult;
use core::{Player, SimulatorData, Team};

/// One resolved appearance, kept with its kickoff so the whole list can be
/// ordered before it is handed to the template.
struct DatedItem {
    kickoff: NaiveDateTime,
    /// A row with no club calendar of its own — a cap, or a match whose
    /// competition and team both lack one — carries only a fallback in
    /// `item.season` until [`PlayerMatchCollector::file_borrowed_seasons`]
    /// files it beside the club football around it.
    borrows_season: bool,
    item: PlayerMatchItem,
}

pub struct PlayerMatchCollector;

impl PlayerMatchCollector {
    /// Build the player's match list: every squad of every club he has
    /// belonged to, then internationals, all ordered by kickoff.
    pub fn collect(
        data: &SimulatorData,
        i18n: &I18n,
        player: &Player,
        team: Option<&Team>,
    ) -> Vec<PlayerMatchItem> {
        let clubs = Self::clubs(data, player, team);

        let mut dated: Vec<DatedItem> = Vec::new();
        for club in clubs.iter().filter_map(|club_id| data.club(*club_id)) {
            for squad in &club.teams.teams {
                Self::collect_from_history(data, i18n, player, squad, &mut dated);
            }
        }
        Self::collect_international(data, player, &clubs, &mut dated);

        dated.sort_by_key(|row| row.kickoff);
        Self::file_borrowed_seasons(&mut dated);
        dated.into_iter().map(|d| d.item).collect()
    }

    /// Club ids worth searching, current registration first: the player's
    /// registration plus every team his statistics history names. The
    /// history slugs are what carry a mid-season move — the ledger writes a
    /// row per (season, team) spell, and `current_secondary` covers being
    /// borrowed by a sibling squad without ever changing registration.
    fn clubs(data: &SimulatorData, player: &Player, team: Option<&Team>) -> Vec<u32> {
        let mut clubs: Vec<u32> = Vec::new();

        if let Some(team) = team {
            Self::remember(&mut clubs, team.club_id);
        }

        let history = &player.statistics_history;
        let teams = history
            .season_ledger
            .iter()
            .map(|e| e.team_slug.as_str())
            .chain(history.current.iter().map(|e| e.team_slug.as_str()))
            .chain(
                history
                    .current_secondary
                    .iter()
                    .map(|e| e.team_slug.as_str()),
            )
            .chain(history.items.iter().map(|i| i.team_slug.as_str()));
        for slug in teams {
            if let Some(club_id) = Self::team_by_slug(data, slug).map(|t| t.club_id) {
                Self::remember(&mut clubs, club_id);
            }
        }

        clubs
    }

    fn remember(ids: &mut Vec<u32>, id: u32) {
        if id != 0 && !ids.contains(&id) {
            ids.push(id);
        }
    }

    fn team_by_slug<'d>(data: &'d SimulatorData, slug: &str) -> Option<&'d Team> {
        if slug.is_empty() {
            return None;
        }
        data.indexes
            .as_ref()
            .and_then(|idx| idx.slug_indexes.get_team_by_slug(slug))
            .and_then(|team_id| data.team(team_id))
    }

    fn collect_from_history(
        data: &SimulatorData,
        i18n: &I18n,
        player: &Player,
        team: &Team,
        out: &mut Vec<DatedItem>,
    ) {
        for played in team
            .match_history
            .items()
            .iter()
            .filter(|played| played.took_the_field(player.id))
        {
            let fixture = PlayedFixture::read(data, i18n, team, played);
            out.push(DatedItem {
                kickoff: fixture.kickoff,
                borrows_season: fixture.season.is_none(),
                item: PlayerMatchItem {
                    season: fixture.season.unwrap_or_else(|| {
                        Season::from_date(fixture.kickoff.date()).as_league_season()
                    }),
                    date: fixture.date,
                    time: fixture.time,
                    opponent_slug: fixture.opponent_slug,
                    opponent_name: fixture.opponent_name,
                    is_home: fixture.is_home,
                    competition_name: fixture.competition_name,
                    result: Some(PlayerMatchResult {
                        match_id: fixture.match_id,
                        home_goals: fixture.home_goals,
                        away_goals: fixture.away_goals,
                    }),
                },
            });
        }
    }

    /// Caps. Both squads a country fields are walked, and the country is the
    /// player's OWN rather than his employer's — a foreign player's caps
    /// must not drop off because his club sits in somebody else's league.
    /// The employer's country is a fallback only for a player whose own
    /// country carries no squad at all. The fixture already records which
    /// side was at home, so the stat line alone decides inclusion.
    fn collect_international(
        data: &SimulatorData,
        player: &Player,
        clubs: &[u32],
        out: &mut Vec<DatedItem>,
    ) {
        let country = data.country(player.country_id).or_else(|| {
            clubs
                .iter()
                .find_map(|club_id| data.country_by_club(*club_id))
        });
        let Some(country) = country else {
            return;
        };
        let top_division = country
            .leagues
            .leagues
            .iter()
            .filter(|league| !league.friendly)
            .min_by_key(|league| league.settings.tier)
            .map(|league| league.settings.season_calendar());

        for fixture in [&country.national_team, &country.u21_national_team]
            .iter()
            .flat_map(|squad| squad.schedule.iter())
        {
            let Some(ref result) = fixture.result else {
                continue;
            };
            let appeared = data
                .match_store
                .get(&fixture.match_id)
                .is_some_and(|mr| Self::appeared(mr, player.id));
            if !appeared {
                continue;
            }

            let kickoff = fixture.date.and_hms_opt(20, 0, 0).unwrap_or_default();

            out.push(DatedItem {
                kickoff,
                borrows_season: true,
                item: PlayerMatchItem {
                    season: top_division
                        .map(|calendar| calendar.season_of(fixture.date))
                        .unwrap_or_else(|| Season::from_date(fixture.date).as_league_season()),
                    date: fixture.date.format("%d.%m.%Y").to_string(),
                    time: "20:00".to_string(),
                    opponent_slug: String::new(),
                    opponent_name: fixture.opponent_country_name.clone(),
                    is_home: fixture.is_home,
                    competition_name: fixture.competition_name.clone(),
                    result: Some(PlayerMatchResult {
                        match_id: fixture.match_id.clone(),
                        home_goals: result.home_score,
                        away_goals: result.away_score,
                    }),
                },
            });
        }
    }

    /// File every borrowed row under the season of the nearest club match
    /// before it, or — ahead of the first club match — the one after it. A
    /// cap lines up with the club football the player was playing at the
    /// time; the national team's own league calendar would put a March cap
    /// of a player abroad beside the wrong campaign.
    fn file_borrowed_seasons(dated: &mut [DatedItem]) {
        let mut surrounding = dated
            .iter()
            .find(|row| !row.borrows_season)
            .map(|row| row.item.season);
        for row in dated.iter_mut() {
            if !row.borrows_season {
                surrounding = Some(row.item.season);
            } else if let Some(season) = surrounding {
                row.item.season = season;
            }
        }
    }

    /// Did the player take the field? The engine writes a stat line for
    /// everyone it had on the pitch and for nobody else, so this is the
    /// appearance test — an unused substitute is named in a squad but has
    /// no entry here.
    fn appeared(match_result: &MatchResult, player_id: u32) -> bool {
        match_result
            .details
            .as_ref()
            .is_some_and(|details| details.player_stats.contains_key(&player_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::PlayerFieldPositionGroup;
    use core::league::season::LeagueSeason;
    use core::r#match::{FieldSquad, MatchResultRaw, PlayerMatchEndStats, Score};

    fn stat_line() -> PlayerMatchEndStats {
        PlayerMatchEndStats {
            shots_on_target: 0,
            shots_total: 0,
            passes_attempted: 0,
            passes_completed: 0,
            tackles: 0,
            interceptions: 0,
            saves: 0,
            shots_faced: 0,
            goals: 0,
            assists: 0,
            match_rating: 6.0,
            raw_match_rating: 6.0,
            xg: 0.0,
            position_group: PlayerFieldPositionGroup::Midfielder,
            fouls: 0,
            yellow_cards: 0,
            red_cards: 0,
            minutes_played: 90,
            key_passes: 0,
            progressive_passes: 0,
            progressive_carries: 0,
            successful_dribbles: 0,
            attempted_dribbles: 0,
            successful_pressures: 0,
            pressures: 0,
            blocks: 0,
            clearances: 0,
            passes_into_box: 0,
            crosses_attempted: 0,
            crosses_completed: 0,
            xg_chain: 0.0,
            xg_buildup: 0.0,
            miscontrols: 0,
            heavy_touches: 0,
            carry_distance: 0,
            errors_leading_to_shot: 0,
            errors_leading_to_goal: 0,
            xg_prevented: 0.0,
            xg_faced: 0.0,
            offsides: 0,
            own_goals: 0,
            zone_stats: Default::default(),
        }
    }

    fn squad(team_id: u32, main: &[u32], substitutes: &[u32], used: &[u32]) -> FieldSquad {
        FieldSquad {
            team_id,
            main: main.to_vec(),
            substitutes: substitutes.to_vec(),
            substitutes_used: used.to_vec(),
            selection_omissions: Vec::new(),
            overlooked: Vec::new(),
            starter_slots: Vec::new(),
        }
    }

    fn played(home_team: u32, away_team: u32, on_pitch: &[u32]) -> MatchResult {
        let mut details = MatchResultRaw::with_match_time(90 * 60 * 1000);
        details.left_team_players = squad(home_team, &[10, 11], &[12], &[12]);
        details.right_team_players = squad(away_team, &[20, 21], &[22], &[]);
        for id in on_pitch {
            details.player_stats.insert(*id, stat_line());
        }

        MatchResult {
            id: format!("2026-08-01_{}_{}", home_team, away_team),
            league_id: 1,
            league_slug: "league".to_string(),
            details: Some(details),
            score: Score::new(home_team, away_team),
            home_team_id: home_team,
            away_team_id: away_team,
            friendly: false,
        }
    }

    #[test]
    fn appearance_is_the_stat_line_not_squad_membership() {
        let m = played(1, 2, &[10, 11, 20]);
        // 10 started, 20 started for the other side.
        assert!(PlayerMatchCollector::appeared(&m, 10));
        assert!(PlayerMatchCollector::appeared(&m, 20));
        // 12 is a named substitute who came on but never got a stat line
        // written — treat that as not having played.
        assert!(!PlayerMatchCollector::appeared(&m, 12));
        // 22 is an unused substitute: named, no stat line, no appearance.
        assert!(!PlayerMatchCollector::appeared(&m, 22));
        // 99 was never involved at all.
        assert!(!PlayerMatchCollector::appeared(&m, 99));
    }

    fn row(borrows_season: bool, opening_year: i32, crosses_new_year: bool) -> DatedItem {
        DatedItem {
            kickoff: NaiveDateTime::default(),
            borrows_season,
            item: PlayerMatchItem {
                season: LeagueSeason {
                    opening_year,
                    crosses_new_year,
                },
                date: String::new(),
                time: String::new(),
                opponent_slug: String::new(),
                opponent_name: String::new(),
                is_home: true,
                competition_name: String::new(),
                result: None,
            },
        }
    }

    fn seasons(rows: &[DatedItem]) -> Vec<(i32, bool)> {
        rows.iter()
            .map(|r| (r.item.season.opening_year, r.item.season.crosses_new_year))
            .collect()
    }

    #[test]
    fn a_cap_between_club_matches_joins_the_campaign_before_it() {
        // February club match in 2026/27, a March cap whose national league
        // would call it 2027, then the next campaign's opener.
        let mut rows = [row(false, 2026, true), row(true, 2027, false), row(false, 2027, true)];
        PlayerMatchCollector::file_borrowed_seasons(&mut rows);
        assert_eq!(seasons(&rows), [(2026, true), (2026, true), (2027, true)]);
    }

    #[test]
    fn a_cap_before_any_club_match_joins_the_first_campaign_after_it() {
        let mut rows = [row(true, 2019, false), row(false, 2020, true)];
        PlayerMatchCollector::file_borrowed_seasons(&mut rows);
        assert_eq!(seasons(&rows), [(2020, true), (2020, true)]);
    }

    #[test]
    fn caps_without_any_club_football_keep_their_fallback() {
        let mut rows = [row(true, 2025, false), row(true, 2026, false)];
        PlayerMatchCollector::file_borrowed_seasons(&mut rows);
        assert_eq!(seasons(&rows), [(2025, false), (2026, false)]);
    }

    #[test]
    fn remember_skips_blanks_and_duplicates() {
        let mut ids = Vec::new();
        PlayerMatchCollector::remember(&mut ids, 7);
        PlayerMatchCollector::remember(&mut ids, 7);
        PlayerMatchCollector::remember(&mut ids, 0);
        PlayerMatchCollector::remember(&mut ids, 9);
        assert_eq!(ids, vec![7, 9]);
    }
}

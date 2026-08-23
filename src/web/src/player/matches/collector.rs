//! Every match a player actually appeared in — gathered from the match
//! records themselves rather than from whichever team he happens to be
//! registered with today.
//!
//! The page used to ask exactly one question: "what does my current
//! team's league schedule hold?". That reads the record through a single
//! keyhole and loses football the player really played:
//!
//!   * A youth-league appearance. U18 / U19 / U20 sides compete in
//!     generated `friendly` sub-leagues ("Primera Division Zona B U18")
//!     hanging off the parent competition. A prospect rostered with the
//!     Main squad but fielded for the academy side has his stat line in
//!     that sub-league's store, which the senior schedule never touches
//!     — so History listed the appearances and Matches showed an empty
//!     table.
//!   * Anything played before a move. `League::matches` is per-league, so
//!     the moment a player changes club — or is loaned, or is pulled up
//!     from the "2" side — every match from the earlier spell drops off
//!     the page even though it happened this season.
//!   * A player with no club at all. Free agents and retired players got
//!     an unconditionally empty table because the whole builder sat
//!     behind `if let Some(team)`.
//!
//! So the collector inverts the lookup. It works out which competitions
//! the player could plausibly have appeared in — his clubs' countries,
//! which is the unit that owns the senior programme, the youth
//! sub-leagues, the domestic cup and the playoffs alike — then walks
//! those competitions' stored match records and keeps the ones carrying
//! a stat line for him. The engine writes a stat line only for a player
//! it had on the pitch, so an unused substitute never appears, and the
//! side he played for is read from the squad that names him rather than
//! from his present registration.

use super::{PlayerMatchItem, PlayerMatchResult};
use chrono::{NaiveDate, NaiveDateTime};
use core::league::League;
use core::r#match::{FieldSquad, MatchResult};
use core::{Country, Player, SimulatorData, Team};
use std::collections::{HashMap, HashSet};

/// One resolved appearance, kept with its kickoff so the whole list can be
/// ordered before it is handed to the template.
struct DatedItem {
    kickoff: NaiveDateTime,
    match_id: String,
    item: PlayerMatchItem,
}

/// The clubs and competitions a player's season could have touched.
struct Footprint {
    /// Club ids, current registration first.
    clubs: Vec<u32>,
    /// Competitions named directly by the player's own record — his team's
    /// league, the leagues his history rows were filed under, the youth
    /// league his friendly bucket points at. Scanned after the countries so
    /// a competition whose country lookup came up empty is still read.
    leagues: Vec<u32>,
}

pub struct PlayerMatchCollector;

impl PlayerMatchCollector {
    /// Build the player's match list: club competitions (league, youth
    /// sub-league, domestic cup, playoff), then continental ties, then
    /// internationals, all ordered by kickoff.
    pub fn collect(
        data: &SimulatorData,
        player: &Player,
        team: Option<&Team>,
    ) -> Vec<PlayerMatchItem> {
        let footprint = Self::footprint(data, player, team);

        let mut dated: Vec<DatedItem> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut scanned: HashSet<u32> = HashSet::new();

        for country in Self::footprint_countries(data, &footprint.clubs) {
            for league in Self::country_competitions(country) {
                if scanned.insert(league.id) {
                    Self::collect_from_league(
                        data, player, league, &footprint, &mut dated, &mut seen,
                    );
                }
            }
        }

        for league_id in &footprint.leagues {
            if !scanned.insert(*league_id) {
                continue;
            }
            if let Some(league) = data.league(*league_id) {
                Self::collect_from_league(data, player, league, &footprint, &mut dated, &mut seen);
            }
        }

        for club_id in &footprint.clubs {
            Self::collect_continental(data, player, *club_id, &mut dated, &mut seen);
        }

        Self::collect_international(data, player, &footprint, &mut dated, &mut seen);

        dated.sort_by(|a, b| a.kickoff.cmp(&b.kickoff).then(a.match_id.cmp(&b.match_id)));
        dated.into_iter().map(|d| d.item).collect()
    }

    /// Clubs and competitions worth searching: the player's registration
    /// plus everything his statistics history names. The history slugs are
    /// what carry a mid-season move — the ledger writes a row per (season,
    /// team) spell, and `current_secondary` covers being borrowed by a
    /// sibling squad without ever changing registration.
    fn footprint(data: &SimulatorData, player: &Player, team: Option<&Team>) -> Footprint {
        let mut clubs: Vec<u32> = Vec::new();
        let mut leagues: Vec<u32> = Vec::new();

        if let Some(team) = team {
            Self::remember(&mut clubs, team.club_id);
            if let Some(league_id) = team.league_id {
                Self::remember(&mut leagues, league_id);
            }
        }
        // The youth league the player's friendly appearances were actually
        // played in, recorded on him at match time — the one pointer that
        // survives even when he is rostered somewhere else entirely.
        Self::remember_by_slug(data, &mut leagues, player.friendly_source_slug.as_deref());

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

        let competitions = history
            .season_ledger
            .iter()
            .flat_map(|e| [e.league_slug.as_str(), e.competition_slug.as_str()])
            .chain(history.current.iter().map(|e| e.league_slug.as_str()))
            .chain(
                history
                    .current_secondary
                    .iter()
                    .map(|e| e.league_slug.as_str()),
            )
            .chain(history.items.iter().map(|i| i.league_slug.as_str()));
        for slug in competitions {
            Self::remember_by_slug(data, &mut leagues, Some(slug));
        }

        Footprint { clubs, leagues }
    }

    fn remember(ids: &mut Vec<u32>, id: u32) {
        if id != 0 && !ids.contains(&id) {
            ids.push(id);
        }
    }

    fn remember_by_slug(data: &SimulatorData, leagues: &mut Vec<u32>, slug: Option<&str>) {
        let Some(slug) = slug.filter(|s| !s.is_empty()) else {
            return;
        };
        if let Some(id) = data
            .indexes
            .as_ref()
            .and_then(|idx| idx.slug_indexes.get_league_by_slug(slug))
        {
            Self::remember(leagues, id);
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

    /// The countries those clubs play in, deduplicated. A country is the
    /// right unit to scan: its `leagues` collection holds the senior
    /// programme AND the generated youth sub-leagues, so a prospect's
    /// academy football is found without having to know which squad he was
    /// rostered with on the day.
    fn footprint_countries<'d>(data: &'d SimulatorData, clubs: &[u32]) -> Vec<&'d Country> {
        let mut countries: Vec<&Country> = Vec::new();
        for club_id in clubs {
            if let Some(country) = data
                .country_by_club(*club_id)
                .filter(|country| !countries.iter().any(|c| c.id == country.id))
            {
                countries.push(country);
            }
        }
        countries
    }

    /// Every competition a country runs. The knockout cup and the grouped
    /// playoffs are stored apart from `leagues` so the round-robin
    /// programme stays pure, but for this purpose they are all just
    /// competitions with a match store.
    fn country_competitions(country: &Country) -> impl Iterator<Item = &League> {
        country
            .leagues
            .leagues
            .iter()
            .chain(country.domestic_cup.as_ref().map(|cup| &cup.league))
            .chain(country.playoffs.iter().map(|playoff| &playoff.league))
    }

    /// Walk one competition's stored results. The store is the authority on
    /// who played; the schedule is consulted only for the kickoff, because
    /// `MatchResult` carries no date of its own.
    fn collect_from_league(
        data: &SimulatorData,
        player: &Player,
        league: &League,
        footprint: &Footprint,
        out: &mut Vec<DatedItem>,
        seen: &mut HashSet<String>,
    ) {
        let mut kickoffs: Option<HashMap<&str, NaiveDateTime>> = None;

        for match_result in league.matches.iter() {
            if !Self::appeared(match_result, player.id) {
                continue;
            }
            let Some(side) = Self::side(data, match_result, player.id, footprint) else {
                continue;
            };
            if !seen.insert(match_result.id.clone()) {
                continue;
            }

            let kickoffs = kickoffs.get_or_insert_with(|| {
                league
                    .schedule
                    .tours
                    .iter()
                    .flat_map(|tour| &tour.items)
                    .map(|item| (item.id.as_str(), item.date))
                    .collect()
            });

            // A fixture the schedule no longer holds still has its kickoff
            // date in the match id — ids are minted `{date}_{home}_{away}`
            // — so the row keeps its place in the timeline rather than
            // being dropped or floated to the front.
            let scheduled = kickoffs.get(match_result.id.as_str()).copied();
            let kickoff = scheduled
                .or_else(|| Self::kickoff_from_id(&match_result.id))
                .unwrap_or_default();

            let is_home = side == match_result.home_team_id;
            let opponent_id = if is_home {
                match_result.away_team_id
            } else {
                match_result.home_team_id
            };
            let (opponent_name, opponent_slug) = Self::team_label(data, opponent_id);
            let (home_goals, away_goals) = Self::goals_as_listed(match_result);

            out.push(DatedItem {
                kickoff,
                match_id: match_result.id.clone(),
                item: PlayerMatchItem {
                    date: kickoff.format("%d.%m.%Y").to_string(),
                    // Only a kickoff the schedule vouched for carries a
                    // clock; the id fallback knows the day and nothing more.
                    time: match scheduled {
                        Some(dt) => dt.format("%H:%M").to_string(),
                        None => String::new(),
                    },
                    opponent_slug,
                    opponent_name,
                    is_home,
                    competition_name: league.name.clone(),
                    result: Some(PlayerMatchResult {
                        match_id: match_result.id.clone(),
                        home_goals,
                        away_goals,
                    }),
                },
            });
        }
    }

    fn collect_continental(
        data: &SimulatorData,
        player: &Player,
        club_id: u32,
        out: &mut Vec<DatedItem>,
        seen: &mut HashSet<String>,
    ) {
        for (comp_name, home_club_id, away_club_id, date, match_id, result) in
            data.continental_matches_for_club(club_id)
        {
            let Some((home_goals, away_goals)) = result else {
                continue;
            };
            let appeared = data
                .match_store
                .get(match_id)
                .is_some_and(|mr| Self::appeared(mr, player.id));
            if !appeared {
                continue;
            }
            if !seen.insert(match_id.to_string()) {
                continue;
            }

            // Continental fixtures are keyed by club and the squad by team,
            // so resolve the player's side back to its club. Falls back to
            // the club whose bracket produced this fixture, which is the
            // side he must have been on to have a stat line here.
            let side_club_id = data
                .match_store
                .get(match_id)
                .and_then(|mr| Self::player_side(mr, player.id))
                .and_then(|team_id| data.team(team_id))
                .map(|t| t.club_id)
                .unwrap_or(club_id);
            let is_home = side_club_id == home_club_id;
            let opponent_club_id = if is_home { away_club_id } else { home_club_id };

            let (opponent_name, opponent_slug) = data
                .club(opponent_club_id)
                .and_then(|club| {
                    club.teams
                        .main_team_id()
                        .and_then(|tid| data.team(tid))
                        .map(|t| (t.name.clone(), t.slug.clone()))
                })
                .unwrap_or_else(|| ("Unknown".to_string(), String::new()));

            let kickoff = date.and_hms_opt(20, 0, 0).unwrap_or_default();

            out.push(DatedItem {
                kickoff,
                match_id: match_id.to_string(),
                item: PlayerMatchItem {
                    date: date.format("%d.%m.%Y").to_string(),
                    time: "20:00".to_string(),
                    opponent_slug,
                    opponent_name,
                    is_home,
                    competition_name: comp_name.to_string(),
                    result: Some(PlayerMatchResult {
                        match_id: match_id.to_string(),
                        home_goals,
                        away_goals,
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
        footprint: &Footprint,
        out: &mut Vec<DatedItem>,
        seen: &mut HashSet<String>,
    ) {
        let country = data.country(player.country_id).or_else(|| {
            footprint
                .clubs
                .iter()
                .find_map(|club_id| data.country_by_club(*club_id))
        });
        let Some(country) = country else {
            return;
        };

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
            if !seen.insert(fixture.match_id.clone()) {
                continue;
            }

            let kickoff = fixture.date.and_hms_opt(20, 0, 0).unwrap_or_default();

            out.push(DatedItem {
                kickoff,
                match_id: fixture.match_id.clone(),
                item: PlayerMatchItem {
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

    /// Which team the player turned out for, so the row can say home or
    /// away and name the right opponent. The squad that lists him is the
    /// authority; a result stored without squads falls back to whichever
    /// side belongs to a club in the player's footprint.
    fn side(
        data: &SimulatorData,
        match_result: &MatchResult,
        player_id: u32,
        footprint: &Footprint,
    ) -> Option<u32> {
        if let Some(team_id) = Self::player_side(match_result, player_id) {
            return Some(team_id);
        }
        [match_result.home_team_id, match_result.away_team_id]
            .into_iter()
            .find(|team_id| {
                data.team(*team_id)
                    .is_some_and(|t| footprint.clubs.contains(&t.club_id))
            })
    }

    /// The `team_id` of whichever match squad names this player.
    fn player_side(match_result: &MatchResult, player_id: u32) -> Option<u32> {
        let details = match_result.details.as_ref()?;
        let named = |squad: &FieldSquad| {
            squad.main.contains(&player_id)
                || squad.substitutes.contains(&player_id)
                || squad.substitutes_used.contains(&player_id)
        };
        if named(&details.left_team_players) {
            Some(details.left_team_players.team_id)
        } else if named(&details.right_team_players) {
            Some(details.right_team_players.team_id)
        } else {
            None
        }
    }

    /// Goals ordered to match the fixture's own home/away sides. A knockout
    /// `Score` may carry its two entries in either order relative to the
    /// stored fixture, so map them back through the recorded `team_id`s
    /// instead of trusting the field names.
    fn goals_as_listed(match_result: &MatchResult) -> (u8, u8) {
        let home = &match_result.score.home_team;
        let away = &match_result.score.away_team;
        if away.team_id == match_result.home_team_id && home.team_id != match_result.home_team_id {
            (away.get(), home.get())
        } else {
            (home.get(), away.get())
        }
    }

    /// Display name and URL slug for a team, preferring the index snapshot
    /// and falling back to the team itself when the index hasn't caught up.
    fn team_label(data: &SimulatorData, team_id: u32) -> (String, String) {
        if let Some(team_data) = data.team_data(team_id) {
            return (team_data.name.clone(), team_data.slug.clone());
        }
        data.team(team_id)
            .map(|t| (t.name.clone(), t.slug.clone()))
            .unwrap_or_else(|| ("Unknown".to_string(), String::new()))
    }

    /// Recover a kickoff date from a match id. Ids are minted as
    /// `{yyyy-mm-dd}_{home}_{away}`, so the date prefix is a reliable last
    /// resort when the fixture is no longer in the schedule.
    fn kickoff_from_id(match_id: &str) -> Option<NaiveDateTime> {
        let (date, _) = match_id.split_once('_')?;
        NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::PlayerFieldPositionGroup;
    use core::r#match::{MatchResultRaw, PlayerMatchEndStats, Score, TeamScore};

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
            starter_slots: Vec::new(),
        }
    }

    /// `home_first` mirrors how the fixture's own `Score` is laid out —
    /// knockout results can carry the two sides in either order.
    fn played(
        id: &str,
        home_team: u32,
        away_team: u32,
        home_goals: u8,
        away_goals: u8,
        home_first: bool,
        on_pitch: &[u32],
    ) -> MatchResult {
        let mut details = MatchResultRaw::with_match_time(90 * 60 * 1000);
        details.left_team_players = squad(home_team, &[10, 11], &[12], &[12]);
        details.right_team_players = squad(away_team, &[20, 21], &[22], &[]);
        for id in on_pitch {
            details.player_stats.insert(*id, stat_line());
        }

        let score = if home_first {
            Score {
                home_team: TeamScore::new_with_score(home_team, home_goals),
                away_team: TeamScore::new_with_score(away_team, away_goals),
                details: Vec::new(),
                home_shootout: 0,
                away_shootout: 0,
            }
        } else {
            Score {
                home_team: TeamScore::new_with_score(away_team, away_goals),
                away_team: TeamScore::new_with_score(home_team, home_goals),
                details: Vec::new(),
                home_shootout: 0,
                away_shootout: 0,
            }
        };

        MatchResult {
            id: id.to_string(),
            league_id: 1,
            league_slug: "league".to_string(),
            details: Some(details),
            score,
            home_team_id: home_team,
            away_team_id: away_team,
            friendly: false,
        }
    }

    #[test]
    fn appearance_is_the_stat_line_not_squad_membership() {
        let m = played("2026-08-01_1_2", 1, 2, 1, 0, true, &[10, 11, 20]);
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

    #[test]
    fn side_comes_from_the_squad_that_names_the_player() {
        let m = played("2026-08-01_1_2", 1, 2, 1, 0, true, &[10, 20, 12]);
        assert_eq!(PlayerMatchCollector::player_side(&m, 10), Some(1));
        assert_eq!(PlayerMatchCollector::player_side(&m, 20), Some(2));
        // A used substitute belongs to the side that brought him on.
        assert_eq!(PlayerMatchCollector::player_side(&m, 12), Some(1));
        assert_eq!(PlayerMatchCollector::player_side(&m, 99), None);
    }

    #[test]
    fn goals_follow_the_fixtures_own_sides() {
        // Score laid out home-first: read straight through.
        let straight = played("2026-08-01_1_2", 1, 2, 3, 1, true, &[10]);
        assert_eq!(PlayerMatchCollector::goals_as_listed(&straight), (3, 1));

        // Same fixture, `Score` carrying its two sides the other way round
        // — the row must still read "3 - 1" for the home team.
        let flipped = played("2026-08-01_1_2", 1, 2, 3, 1, false, &[10]);
        assert_eq!(PlayerMatchCollector::goals_as_listed(&flipped), (3, 1));
    }

    #[test]
    fn kickoff_falls_back_to_the_date_encoded_in_the_match_id() {
        let kickoff = PlayerMatchCollector::kickoff_from_id("2026-08-01_1300489_15035999");
        assert_eq!(
            kickoff,
            NaiveDate::from_ymd_opt(2026, 8, 1).and_then(|d| d.and_hms_opt(0, 0, 0))
        );
        assert_eq!(PlayerMatchCollector::kickoff_from_id("nonsense"), None);
        assert_eq!(
            PlayerMatchCollector::kickoff_from_id("not-a-date_1_2"),
            None
        );
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

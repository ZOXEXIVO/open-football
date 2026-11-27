pub mod routes;

use crate::face::skin::CountrySkin;
use crate::leagues::address::LeagueAddress;
use crate::shared::default_handler::{
    COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION, MATCH_VIEWER_AVAILABLE, MATCH_VIEWER_BYTES,
    MATCH_VIEWER_VERSION,
};
use crate::shared::slug::PlayerSlug;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::MatchRuntime;
use core::NationalTeamLevel;
use core::Player;
use core::SimulatorData;
use core::r#match::MatchStatisticType;
use core::r#match::{MatchResult, MatchResultRaw};
use serde::{Deserialize, Serialize};
use shared::Appearance;

#[derive(Deserialize)]
pub struct MatchGetRequest {
    pub lang: String,
    pub match_id: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "match/get/index.html")]
pub struct MatchGetTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    /// The browser tab and the page's hidden heading. The header itself shows
    /// the scoreboard instead of a title, so this template overrides
    /// `header_title` and carries none of the layout's sub-title fields.
    pub title: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: I18n,
    pub lang: String,
    pub competition_name: String,
    /// Empty for a match that belongs to no page of its own.
    pub competition_url: String,
    pub home_team_name: String,
    pub home_team_url: String,
    pub home_goals: u8,
    pub home_goal_events: Vec<GoalEventDisplay>,
    pub home_squad_main: Vec<MatchPlayer>,
    pub home_squad_subs: Vec<MatchPlayer>,
    pub away_team_name: String,
    pub away_team_url: String,
    pub away_goals: u8,
    pub away_goal_events: Vec<GoalEventDisplay>,
    pub away_squad_main: Vec<MatchPlayer>,
    pub away_squad_subs: Vec<MatchPlayer>,
    /// Everything the WebAssembly viewer needs, as a JSON literal the page
    /// hands straight to `MatchViewer.start`.
    pub viewer_config_json: String,
    pub match_viewer_available: bool,
    pub match_viewer_version: &'static str,
    /// What the viewer's wasm inflates to, so the loading bar can be drawn
    /// against a real total. The browser reports `Content-Length` for the
    /// COMPRESSED transfer and hands the script the inflated bytes, so a bar
    /// drawn from the response alone finishes a fifth of the way in.
    pub match_viewer_bytes: u64,
    pub player_of_the_match_id: u32,
    pub player_of_the_match_slug: String,
    pub player_of_the_match_name: String,
    pub match_recordings_enabled: bool,
}

pub struct GoalEventDisplay {
    pub player_slug: String,
    pub player_name: String,
    pub minute: u32,
    pub is_auto_goal: bool,
}

pub struct MatchPlayer {
    pub slug: String,
    pub last_name: String,
    pub position: String,
    pub sub_minute: Option<u32>,
    pub subbed_off_minute: Option<u32>,
    pub is_player_of_the_match: bool,
    pub rating: String,
    pub rating_tier: &'static str,
}

/// Mirror of `match_viewer::app::config::ViewerConfig`. The viewer reads its whole
/// world from this document, so anything the replay needs to know about the
/// fixture is resolved here, on the server, where the simulator data lives.
#[derive(Serialize)]
struct ViewerConfigJson {
    canvas: &'static str,
    api_base: String,
    match_time_ms: u64,
    home: TeamColorsJson,
    away: TeamColorsJson,
    /// What the two sides are called. The replay names them on its score bug
    /// and on the card it ends with; resolved here because an international
    /// fixture carries country IDs rather than team ones and only this side
    /// knows the difference.
    home_name: String,
    away_name: String,
    players: Vec<PlayerJson>,
    goals: Vec<GoalEventJson>,
    chances: Vec<ChanceEventJson>,
    substitutions: Vec<SubstitutionEventJson>,
    labels: ViewerLabelsJson,
    venue: VenueJson,
    /// Walk the two teams out before the replay starts. Always, in the game —
    /// only the `.dev/match` harness ever turns it off.
    lineup: bool,
}

impl ViewerConfigJson {
    /// Serialises for inlining inside a `<script>` element. `</` is escaped so
    /// a player name can never close the element early; `\/` is a legal JSON
    /// escape, so the viewer still parses it as written.
    fn to_script_literal(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|_| "null".to_string())
            .replace("</", "<\\/")
    }
}

#[derive(Serialize)]
struct TeamColorsJson {
    background: String,
    foreground: String,
}

impl TeamColorsJson {
    /// International fixtures carry country IDs rather than club IDs, so there
    /// is no kit to look up — those fall back to the neutral pair.
    fn for_club(data: &SimulatorData, club_id: u32, fallback_background: &str) -> Self {
        let club = if club_id > 0 {
            data.club(club_id)
        } else {
            None
        };
        TeamColorsJson {
            background: club
                .map(|c| c.colors.background.clone())
                .unwrap_or_else(|| fallback_background.to_string()),
            foreground: club
                .map(|c| c.colors.foreground.clone())
                .unwrap_or_else(|| "#ffffff".to_string()),
        }
    }
}

/// **The ground the fixture was played at**, for the viewer to build a stadium
/// out of.
///
/// Facts, not decisions: how many the ground holds, how many usually come, how
/// well thought of the two sides are, and whether this is an age-restricted
/// fixture at all. What that comes to in rows of concrete and
/// coats in the stands is the viewer's, since it is the only end that knows how
/// big a row is — see `match_viewer::scene::crowd::Stature`.
#[derive(Serialize)]
struct VenueJson {
    /// Frozen assignment from the home team's ID; independent of this fixture.
    field_style: u8,
    capacity: u32,
    attendance: u32,
    reputation: u16,
    /// The VISITORS, which is half of what decides whether a ground fills.
    visitor: u16,
    youth: bool,
}

impl VenueJson {
    /// The ground belongs to the HOME side, and so do the first three of
    /// these. The last two are about the fixture rather than the venue.
    ///
    /// That flag is read off both teams because a youth fixture is a youth
    /// fixture whoever hosted it — the two academies of two great clubs meet
    /// on a training pitch, not in either of their stadiums.
    ///
    /// An international tie carries country IDs rather than team ones, so
    /// there is no ground to look up: those are played at a national stadium,
    /// which is what the default already describes.
    fn for_fixture(data: &SimulatorData, home_team_id: u32, away_team_id: u32) -> Self {
        let field_style = shared::field::FieldStyle::for_home_team(home_team_id);
        let Some(home) = data.team(home_team_id) else {
            return VenueJson {
                field_style,
                ..VenueJson::default()
            };
        };
        let ground = data
            .club(home.club_id)
            .map(|club| &club.facilities)
            .map(|f| (f.stadium_capacity, f.average_attendance))
            .unwrap_or_default();

        let away = data.team(away_team_id);
        VenueJson {
            field_style,
            capacity: ground.0,
            attendance: ground.1,
            reputation: home.reputation.world,
            // An unknown visitor is read as a peer, which leaves the gate at
            // whatever the club ordinarily draws.
            visitor: away.map_or(home.reputation.world, |away| away.reputation.world),
            youth: home.team_type.is_youth() || away.is_some_and(|away| away.team_type.is_youth()),
        }
    }
}

impl Default for VenueJson {
    /// A great ground, comfortably full — the stadium the viewer built for
    /// every match before there was a venue to describe, and what an
    /// international tie is played in.
    fn default() -> Self {
        VenueJson {
            field_style: 0,
            capacity: 60_000,
            attendance: 50_000,
            reputation: 10_000,
            visitor: 10_000,
            youth: false,
        }
    }
}

#[derive(Serialize)]
struct ViewerLabelsJson {
    first_half: String,
    second_half: String,
    loading: String,
    /// Shown in place of the loading notice when the recording turns out to
    /// hold nothing — a match that finished goalless has no clips in it.
    no_recording: String,
    /// What the card the replay ends on is headed with. The same string this
    /// page sets over its own result, so the scoreboard above the canvas and
    /// the one the replay finishes on read as one thing.
    full_time: String,
    /// …and what the bench is headed with on the team sheet the replay opens
    /// its walk-out over.
    substitutes: String,
}

#[derive(Serialize)]
struct GoalEventJson {
    player_id: u32,
    time: u64,
    is_auto_goal: bool,
}

/// A goal-scoring moment that stayed out — the engine's shortlist, already cut
/// to two or three a side (`HighlightSelector`), each with a clip of its own in
/// the recording. The timeline marks them so there is something to seek to
/// between the goals; on a goalless match they are the entire reel.
#[derive(Serialize)]
struct ChanceEventJson {
    player_id: u32,
    time: u64,
}

/// A change, with a clip of its own in the recording: the engine stops the
/// match for it and plays the two walks out, so the timeline marks it the way
/// it marks a goal and there is something behind the mark to watch.
#[derive(Serialize)]
struct SubstitutionEventJson {
    player_in_id: u32,
    /// And the man he replaced. **Nothing is ever pointed at him** — the shot
    /// is on the men coming ON — but the replay still needs to know who he is,
    /// because he is one of the two bodies moving while the change is played
    /// out and the camera must not give way to him. See `ChangeoverShot::pan`.
    player_out_id: u32,
    time: u64,
    /// How long the match stopped for while it was made. The replay holds its
    /// pitch-side camera on the change for exactly this long.
    break_ms: u64,
}

#[derive(Serialize)]
struct PlayerJson {
    id: u32,
    shirt_number: u8,
    first_name: String,
    last_name: String,
    position: String,
    is_home: bool,
    /// Whether he was named in the starting eleven rather than on the bench.
    /// Only the eleven walk out before kick-off, and the recording cannot say
    /// which eleven those were — see `match_viewer::broadcast::lineup`.
    starting: bool,
    /// What he looks like, as indices into `shared::Palette`
    /// tables. Resolved here rather than in the viewer for the same reason
    /// the labels above are: the answer needs the country table, which lives
    /// on this side of the WebAssembly boundary — and the portrait on his
    /// profile page is drawn from the very same numbers, so a player is one
    /// man in both places instead of two.
    skin: u8,
    hair: u8,
    eyes: u8,
    /// His photograph, for a real footballer — `None` for a regen, who has
    /// never been photographed by anybody.
    photo: Option<String>,
    /// …and the drawn portrait behind it, which every player has: the same
    /// head his profile page shows, asked for as a cutout so the viewer gets
    /// a head on transparent ground rather than one on a club-coloured card.
    ///
    /// Both are URLs rather than ids because the viewer has no business
    /// knowing where this game keeps its pictures — it fetches what the page
    /// hands it, in the order the page hands it, exactly as the profile page
    /// falls back from one to the other.
    face: String,
}

impl PlayerJson {
    /// Everyone in one squad, in team-sheet order.
    ///
    /// `number` carries on across the two calls a side makes (starters, then
    /// bench), so a squad with no registered shirt numbers still hands out
    /// 1..18 rather than 1..11 twice.
    fn append(
        into: &mut Vec<PlayerJson>,
        data: &SimulatorData,
        player_ids: &[u32],
        is_home: bool,
        starting: bool,
        number: &mut u8,
    ) {
        for player_id in player_ids {
            let Some(player) = data.player(*player_id) else {
                continue;
            };
            into.push(PlayerJson::of(data, player, is_home, starting, *number));
            *number += 1;
        }
    }

    fn of(
        data: &SimulatorData,
        player: &Player,
        is_home: bool,
        starting: bool,
        squad_number: u8,
    ) -> PlayerJson {
        let shirt_number = player.shirt_number();
        let look = Appearance::of(player.id, CountrySkin::for_country(data, player.country_id));
        PlayerJson {
            id: player.id,
            shirt_number: if shirt_number == 0 {
                squad_number
            } else {
                shirt_number
            },
            first_name: player.full_name.display_first_name().to_string(),
            last_name: player.full_name.display_last_name().to_string(),
            position: player.position().get_short_name().to_string(),
            is_home,
            starting,
            skin: look.skin as u8,
            hair: look.hair as u8,
            eyes: look.eyes as u8,
            photo: (!player.is_generated())
                .then(|| format!("{}/{}.png", crate::face::PHOTO_LIBRARY, player.id)),
            face: format!(
                "/api/players/{}/face.svg?cutout=1&v={}",
                player.id,
                crate::face::FACE_VERSION
            ),
        }
    }
}

pub struct MatchGetPage<'a> {
    data: &'a SimulatorData,
    i18n: I18n,
    lang: String,
    match_id: String,
}

impl<'a> MatchGetPage<'a> {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<MatchGetRequest>,
    ) -> ApiResult<impl IntoResponse> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let world = state.world.snapshot();

        let simulator_data = world
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        // Look up match from global store, then fall back to scanning leagues
        let match_result = simulator_data
            .match_store
            .get(&route_params.match_id)
            .or_else(|| {
                // Fall back: scan each country's per-league match stores. The
                // domestic cup and the playoffs live outside the `leagues`
                // collection, so scan their inner leagues too — otherwise ties
                // linked from a bracket or a schedule would 404.
                simulator_data
                    .continents
                    .iter()
                    .flat_map(|c| &c.countries)
                    .find_map(|country| {
                        country
                            .leagues
                            .leagues
                            .iter()
                            .chain(country.domestic_cup.as_ref().map(|cup| &cup.league))
                            .chain(country.playoffs.iter().map(|playoff| &playoff.league))
                            .find_map(|l| l.matches.get(&route_params.match_id))
                    })
            })
            .ok_or_else(|| {
                ApiError::NotFound(format!("Match '{}' not found", route_params.match_id))
            })?;

        let page = MatchGetPage {
            data: simulator_data,
            i18n,
            lang: route_params.lang,
            match_id: route_params.match_id.clone(),
        };
        Ok(page.render(match_result).into_response())
    }

    fn render(self, match_result: &MatchResult) -> ApiResult<MatchGetTemplate> {
        let league = self.data.league(match_result.league_id);
        let is_international = match_result.league_slug == "international";
        let national_team_suffix = match NationalTeamLevel::of_match_id(&match_result.id) {
            NationalTeamLevel::Senior => "",
            NationalTeamLevel::Under21 => "/u21",
        };

        let teams =
            MatchTeams::resolve(&self, match_result, is_international, national_team_suffix)?;

        let result_details = match_result
            .details
            .as_ref()
            .ok_or_else(|| ApiError::NotFound("Match details not available".to_string()))?;

        let score = result_details
            .score
            .as_ref()
            .ok_or_else(|| ApiError::NotFound("Match score not available".to_string()))?;

        let viewer_config = self.viewer_config(
            match_result,
            result_details,
            score,
            &teams,
            is_international,
        );
        let (motm_id, motm_name, motm_slug) = self.motm(result_details);

        let (competition_name, competition_url) = self.competition(league, match_result);
        let squads = MatchSquads::of(self.data, result_details, motm_id);

        Ok(MatchGetTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            cpu_brand: &CPU_BRAND,
            cores_count: *CPU_CORES,
            title: format!("{} - {}", teams.home_name, teams.away_name),
            header_color: String::new(),
            foreground_color: String::new(),
            menu_sections: vec![],
            competition_name,
            competition_url,
            home_team_name: teams.home_name.clone(),
            home_team_url: teams.home_url,
            home_goals: score.home_team.get(),
            home_goal_events: GoalEventDisplay::events(
                self.data,
                &self.i18n,
                result_details,
                score,
                true,
            ),
            home_squad_main: squads.home_main,
            home_squad_subs: squads.home_subs,
            away_team_name: teams.away_name.clone(),
            away_team_url: teams.away_url,
            away_goals: score.away_team.get(),
            away_goal_events: GoalEventDisplay::events(
                self.data,
                &self.i18n,
                result_details,
                score,
                false,
            ),
            away_squad_main: squads.away_main,
            away_squad_subs: squads.away_subs,
            viewer_config_json: viewer_config.to_script_literal(),
            match_viewer_available: MATCH_VIEWER_AVAILABLE,
            match_viewer_version: MATCH_VIEWER_VERSION,
            match_viewer_bytes: MATCH_VIEWER_BYTES,
            player_of_the_match_id: motm_id.unwrap_or(0),
            player_of_the_match_slug: motm_slug,
            player_of_the_match_name: motm_name,
            // The same flag that decided whether to record decides whether to
            // offer the replay — including for a friendly, which is a youth or
            // reserve league fixture here (`League::friendly`). See `Match::play`
            // for why those are no longer a special case.
            match_recordings_enabled: MatchRuntime::recordings_mode(),
            i18n: self.i18n,
            lang: self.lang,
        })
    }

    fn viewer_config(
        &self,
        match_result: &MatchResult,
        result_details: &MatchResultRaw,
        score: &core::r#match::Score,
        teams: &MatchTeams,
        is_international: bool,
    ) -> ViewerConfigJson {
        let players = PlayerJson::squad(self.data, result_details);
        let goals = GoalEventJson::from_score(score);
        let chances = result_details
            .chances
            .iter()
            .map(ChanceEventJson::from_chance)
            .collect();
        let substitutions = result_details
            .substitutions
            .iter()
            .map(SubstitutionEventJson::from_change)
            .collect();
        ViewerConfigJson::of(
            self,
            match_result,
            result_details,
            teams,
            players,
            goals,
            chances,
            substitutions,
            is_international,
        )
    }

    fn motm(&self, result_details: &MatchResultRaw) -> (Option<u32>, String, String) {
        let id = result_details.player_of_the_match_id;
        let name = id
            .and_then(|id| self.data.player(id))
            .map(MatchPlayer::full_name)
            .unwrap_or_default();
        let slug = id
            .map(|id| PlayerSlug::history(self.data, id, &name))
            .unwrap_or_default();
        (id, name, slug)
    }

    fn competition(
        &self,
        league: Option<&core::league::League>,
        match_result: &MatchResult,
    ) -> (String, String) {
        // What the header's left rail names the match after. This used to be the
        // layout's sub-title, and the scoreboard that replaced it inherits the
        // sub-title's resolution rather than the raw league name: the display name
        // is the localised, country-qualified one, and a continental tie is not a
        // league at all — it has its own page, and `/leagues/champions-league`
        // is not it.
        if let Some(l) = league {
            (
                views::Menus::league_display_name(l, &self.i18n, self.data),
                LeagueAddress::of(self.data, l).url(&self.lang),
            )
        } else {
            let name = match match_result.league_slug.as_str() {
                "champions-league" => "champions_league",
                "europa-league" => "europa_league",
                "conference-league" => "conference_league",
                "copa-libertadores" => "copa_libertadores",
                _ => "international",
            };
            let link = match match_result.league_slug.as_str() {
                "champions-league" => format!("/{}/champions-league", self.lang),
                "europa-league" => format!("/{}/europa-league", self.lang),
                "conference-league" => format!("/{}/conference-league", self.lang),
                "copa-libertadores" => format!("/{}/copa-libertadores", self.lang),
                _ => String::new(),
            };
            (self.i18n.t(name).to_string(), link)
        }
    }
}

struct MatchTeams {
    home_name: String,
    home_url: String,
    home_club_id: u32,
    away_name: String,
    away_url: String,
    away_club_id: u32,
}

impl MatchTeams {
    fn resolve(
        page: &MatchGetPage<'_>,
        match_result: &MatchResult,
        is_international: bool,
        national_team_suffix: &str,
    ) -> ApiResult<MatchTeams> {
        let (home_name, home_url, home_club_id) = Self::team_side(
            page,
            match_result.home_team_id,
            is_international,
            national_team_suffix,
            true,
        )?;
        let (away_name, away_url, away_club_id) = Self::team_side(
            page,
            match_result.away_team_id,
            is_international,
            national_team_suffix,
            false,
        )?;
        Ok(MatchTeams {
            home_name,
            home_url,
            home_club_id,
            away_name,
            away_url,
            away_club_id,
        })
    }

    fn team_side(
        page: &MatchGetPage<'_>,
        team_id: u32,
        is_international: bool,
        national_team_suffix: &str,
        home: bool,
    ) -> ApiResult<(String, String, u32)> {
        if is_international {
            let (name, url) = page
                .data
                .country(team_id)
                .map(|c| {
                    let url = format!(
                        "/{}/countries/{}{}",
                        page.lang, c.slug, national_team_suffix
                    );
                    (c.name.clone(), url)
                })
                .unwrap_or_else(|| {
                    let key = if home { "home_team" } else { "away_team" };
                    (page.i18n.t(key).to_string(), String::new())
                });
            return Ok((name, url, 0u32));
        }

        let t = page.data.team(team_id).ok_or_else(|| {
            ApiError::NotFound(if home {
                "Home team not found".to_string()
            } else {
                "Away team not found".to_string()
            })
        })?;
        let url = format!("/{}/teams/{}", page.lang, t.slug);
        Ok((t.name.clone(), url, t.club_id))
    }
}

struct MatchSquads {
    home_main: Vec<MatchPlayer>,
    home_subs: Vec<MatchPlayer>,
    away_main: Vec<MatchPlayer>,
    away_subs: Vec<MatchPlayer>,
}

impl MatchSquads {
    fn of(
        data: &SimulatorData,
        result_details: &MatchResultRaw,
        motm_id: Option<u32>,
    ) -> MatchSquads {
        MatchSquads {
            home_main: Self::main(
                data,
                &result_details.left_team_players.main,
                motm_id,
                result_details,
            ),
            home_subs: Self::subs(
                data,
                &result_details.left_team_players.substitutes,
                motm_id,
                result_details,
            ),
            away_main: Self::main(
                data,
                &result_details.right_team_players.main,
                motm_id,
                result_details,
            ),
            away_subs: Self::subs(
                data,
                &result_details.right_team_players.substitutes,
                motm_id,
                result_details,
            ),
        }
    }

    fn main(
        data: &SimulatorData,
        ids: &[u32],
        motm_id: Option<u32>,
        result_details: &MatchResultRaw,
    ) -> Vec<MatchPlayer> {
        ids.iter()
            .filter_map(|pid| {
                let mut p = MatchPlayer::from_raw(*pid, data, motm_id, result_details)?;
                if let Some(sub) = result_details
                    .substitutions
                    .iter()
                    .find(|s| s.player_out_id == *pid)
                {
                    p.subbed_off_minute = Some(MatchPlayer::minute(
                        sub.match_time_ms,
                        result_details.match_time_ms,
                    ));
                }
                Some(p)
            })
            .collect()
    }

    fn subs(
        data: &SimulatorData,
        ids: &[u32],
        motm_id: Option<u32>,
        result_details: &MatchResultRaw,
    ) -> Vec<MatchPlayer> {
        ids.iter()
            .filter_map(|pid| {
                let mut p = MatchPlayer::from_raw(*pid, data, motm_id, result_details)?;
                if let Some(sub) = result_details
                    .substitutions
                    .iter()
                    .find(|s| s.player_in_id == *pid)
                {
                    p.sub_minute = Some(MatchPlayer::minute(
                        sub.match_time_ms,
                        result_details.match_time_ms,
                    ));
                }
                // Check if this sub was also later subbed off (sub-of-sub)
                if let Some(sub_off) = result_details
                    .substitutions
                    .iter()
                    .find(|s| s.player_out_id == *pid)
                {
                    p.subbed_off_minute = Some(MatchPlayer::minute(
                        sub_off.match_time_ms,
                        result_details.match_time_ms,
                    ));
                }
                Some(p)
            })
            .collect()
    }
}

impl MatchPlayer {
    fn from_raw(
        player_id: u32,
        simulator_data: &SimulatorData,
        motm_id: Option<u32>,
        result_details: &MatchResultRaw,
    ) -> Option<MatchPlayer> {
        let player = simulator_data.player(player_id)?;
        let (rating, rating_tier) = result_details
            .player_stats
            .get(&player_id)
            .map(|s| {
                let r = s.match_rating;
                let tier = if r < 6.0 {
                    "low"
                } else if r < 7.0 {
                    "mid"
                } else if r < 8.0 {
                    "good"
                } else {
                    "great"
                };
                (format!("{:.1}", r), tier)
            })
            .unwrap_or_else(|| (String::new(), ""));
        Some(MatchPlayer {
            slug: player.slug(),
            last_name: player.full_name.display_last_name().to_string(),
            position: player.position().get_short_name().to_string(),
            sub_minute: None,
            subbed_off_minute: None,
            is_player_of_the_match: motm_id == Some(player_id),
            rating,
            rating_tier,
        })
    }

    fn minute(match_time_ms: u64, total_match_time_ms: u64) -> u32 {
        if total_match_time_ms == 0 {
            return 0;
        }
        (match_time_ms * 90 / total_match_time_ms) as u32
    }

    fn full_name(player: &Player) -> String {
        format!(
            "{} {}",
            player.full_name.display_first_name(),
            player.full_name.display_last_name()
        )
    }
}

impl GoalEventDisplay {
    fn events(
        data: &SimulatorData,
        i18n: &I18n,
        result_details: &MatchResultRaw,
        score: &core::r#match::Score,
        home: bool,
    ) -> Vec<GoalEventDisplay> {
        score
            .detail()
            .iter()
            .filter(|g| g.stat_type == MatchStatisticType::Goal)
            .filter(|g| {
                let is_side_player = if home {
                    result_details.left_team_players.main.contains(&g.player_id)
                        || result_details
                            .left_team_players
                            .substitutes
                            .contains(&g.player_id)
                } else {
                    result_details
                        .right_team_players
                        .main
                        .contains(&g.player_id)
                        || result_details
                            .right_team_players
                            .substitutes
                            .contains(&g.player_id)
                };
                if g.is_auto_goal {
                    !is_side_player
                } else {
                    is_side_player
                }
            })
            .map(|g| {
                let player_name = data
                    .player(g.player_id)
                    .map(MatchPlayer::full_name)
                    .unwrap_or_else(|| i18n.t("unknown").to_string());
                let minute = (g.time * 90)
                    .checked_div(result_details.match_time_ms)
                    .unwrap_or(0) as u32;
                GoalEventDisplay {
                    player_slug: PlayerSlug::history(data, g.player_id, &player_name),
                    player_name,
                    minute,
                    is_auto_goal: g.is_auto_goal,
                }
            })
            .collect()
    }
}

impl GoalEventJson {
    fn from_score(score: &core::r#match::Score) -> Vec<GoalEventJson> {
        score
            .detail()
            .iter()
            .filter(|goal| goal.stat_type == MatchStatisticType::Goal)
            .map(|goal| GoalEventJson {
                player_id: goal.player_id,
                time: goal.time,
                is_auto_goal: goal.is_auto_goal,
            })
            .collect()
    }
}

impl ChanceEventJson {
    fn from_chance(chance: &core::r#match::ChanceDetail) -> ChanceEventJson {
        ChanceEventJson {
            player_id: chance.player_id,
            time: chance.time,
        }
    }
}

impl SubstitutionEventJson {
    fn from_change(change: &core::r#match::SubstitutionInfo) -> SubstitutionEventJson {
        SubstitutionEventJson {
            player_in_id: change.player_in_id,
            player_out_id: change.player_out_id,
            time: change.match_time_ms,
            break_ms: change.break_ms,
        }
    }
}

impl PlayerJson {
    fn squad(data: &SimulatorData, result_details: &MatchResultRaw) -> Vec<PlayerJson> {
        let mut viewer_players: Vec<PlayerJson> = Vec::new();

        // Assign squad numbers (1-based) per team when shirt_number is not set
        let mut home_number: u8 = 1;
        PlayerJson::append(
            &mut viewer_players,
            data,
            &result_details.left_team_players.main,
            true,
            true,
            &mut home_number,
        );
        PlayerJson::append(
            &mut viewer_players,
            data,
            &result_details.left_team_players.substitutes,
            true,
            false,
            &mut home_number,
        );

        let mut away_number: u8 = 1;
        PlayerJson::append(
            &mut viewer_players,
            data,
            &result_details.right_team_players.main,
            false,
            true,
            &mut away_number,
        );
        PlayerJson::append(
            &mut viewer_players,
            data,
            &result_details.right_team_players.substitutes,
            false,
            false,
            &mut away_number,
        );
        viewer_players
    }
}

impl ViewerConfigJson {
    fn of(
        page: &MatchGetPage<'_>,
        match_result: &MatchResult,
        result_details: &MatchResultRaw,
        teams: &MatchTeams,
        players: Vec<PlayerJson>,
        goals: Vec<GoalEventJson>,
        chances: Vec<ChanceEventJson>,
        substitutions: Vec<SubstitutionEventJson>,
        is_international: bool,
    ) -> ViewerConfigJson {
        ViewerConfigJson {
            canvas: "#match-canvas",
            api_base: format!("/api/match/{}", page.match_id),
            match_time_ms: result_details.match_time_ms,
            home: TeamColorsJson::for_club(page.data, teams.home_club_id, "#00307d"),
            away: TeamColorsJson::for_club(page.data, teams.away_club_id, "#b33f00"),
            home_name: teams.home_name.clone(),
            away_name: teams.away_name.clone(),
            players,
            goals,
            chances,
            substitutions,
            labels: ViewerLabelsJson::of(&page.i18n),
            venue: if is_international {
                VenueJson {
                    field_style: shared::field::FieldStyle::for_home_team(
                        match_result.home_team_id,
                    ),
                    ..VenueJson::default()
                }
            } else {
                VenueJson::for_fixture(
                    page.data,
                    match_result.home_team_id,
                    match_result.away_team_id,
                )
            },
            lineup: true,
        }
    }
}

impl ViewerLabelsJson {
    fn of(i18n: &I18n) -> ViewerLabelsJson {
        ViewerLabelsJson {
            first_half: i18n.t("first_half").to_string(),
            second_half: i18n.t("second_half").to_string(),
            loading: i18n.t("loading_match").to_string(),
            no_recording: i18n.t("match_no_recording").to_string(),
            full_time: i18n.t("full_time").to_string(),
            substitutes: i18n.t("substitutes").to_string(),
        }
    }
}

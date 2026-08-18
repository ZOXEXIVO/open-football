pub mod routes;

use crate::common::default_handler::{
    COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION, MATCH_VIEWER_AVAILABLE, MATCH_VIEWER_VERSION,
};
use crate::common::slug::player_history_slug;
use crate::face::skin::CountrySkin;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use shared::Appearance;
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::MatchRuntime;
use core::Player;
use core::SimulatorData;
use core::r#match::MatchResultRaw;
use core::r#match::player::statistics::MatchStatisticType;
use serde::{Deserialize, Serialize};

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
    pub home_team_slug: String,
    pub home_goals: u8,
    pub home_goal_events: Vec<GoalEventDisplay>,
    pub home_squad_main: Vec<MatchPlayer>,
    pub home_squad_subs: Vec<MatchPlayer>,
    pub away_team_name: String,
    pub away_team_slug: String,
    pub away_goals: u8,
    pub away_goal_events: Vec<GoalEventDisplay>,
    pub away_squad_main: Vec<MatchPlayer>,
    pub away_squad_subs: Vec<MatchPlayer>,
    /// Everything the WebAssembly viewer needs, as a JSON literal the page
    /// hands straight to `MatchViewer.start`.
    pub viewer_config_json: String,
    pub match_viewer_available: bool,
    pub match_viewer_version: &'static str,
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

/// Mirror of `match_viewer::config::ViewerConfig`. The viewer reads its whole
/// world from this document, so anything the replay needs to know about the
/// fixture is resolved here, on the server, where the simulator data lives.
#[derive(Serialize)]
struct ViewerConfigJson {
    canvas: &'static str,
    api_base: String,
    match_time_ms: u64,
    home: TeamColorsJson,
    away: TeamColorsJson,
    players: Vec<PlayerJson>,
    goals: Vec<GoalEventJson>,
    labels: ViewerLabelsJson,
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

#[derive(Serialize)]
struct ViewerLabelsJson {
    first_half: String,
    second_half: String,
    loading: String,
    /// Shown in place of the loading notice when the recording turns out to
    /// hold nothing — a match that finished goalless has no clips in it.
    no_recording: String,
}

#[derive(Serialize)]
struct GoalEventJson {
    player_id: u32,
    time: u64,
    is_auto_goal: bool,
}

#[derive(Serialize)]
struct PlayerJson {
    id: u32,
    shirt_number: u8,
    last_name: String,
    position: String,
    is_home: bool,
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
        number: &mut u8,
    ) {
        for player_id in player_ids {
            let Some(player) = data.player(*player_id) else {
                continue;
            };
            into.push(PlayerJson::of(data, player, is_home, *number));
            *number += 1;
        }
    }

    fn of(data: &SimulatorData, player: &Player, is_home: bool, squad_number: u8) -> PlayerJson {
        let shirt_number = player.shirt_number();
        let look = Appearance::of(player.id, CountrySkin::for_country(data, player.country_id));
        PlayerJson {
            id: player.id,
            shirt_number: if shirt_number == 0 {
                squad_number
            } else {
                shirt_number
            },
            last_name: player.full_name.display_last_name().to_string(),
            position: player.position().get_short_name().to_string(),
            is_home,
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

pub async fn match_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<MatchGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    // Look up match from global store, then fall back to scanning leagues
    let match_result = simulator_data
        .match_store
        .get(&route_params.match_id)
        .or_else(|| {
            // Fall back: scan each country's per-league match stores. The
            // domestic cup lives on `Country::domestic_cup`, outside the
            // `leagues` collection, so scan its inner league too — otherwise
            // cup ties linked from the bracket would 404.
            simulator_data
                .continents
                .iter()
                .flat_map(|c| &c.countries)
                .find_map(|country| {
                    country
                        .leagues
                        .leagues
                        .iter()
                        .find_map(|l| l.matches.get(&route_params.match_id))
                        .or_else(|| {
                            country
                                .domestic_cup
                                .as_ref()
                                .and_then(|cup| cup.league.matches.get(&route_params.match_id))
                        })
                })
        })
        .ok_or_else(|| {
            ApiError::NotFound(format!("Match '{}' not found", route_params.match_id))
        })?;

    let league = simulator_data.league(match_result.league_id);

    let is_international = match_result.league_slug == "international";

    // For international matches, team IDs are country IDs — resolve names differently
    let (home_team_name, home_team_slug, home_club_id) = if is_international {
        let name = simulator_data
            .country(match_result.home_team_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "Home".to_string());
        let slug = simulator_data
            .country(match_result.home_team_id)
            .map(|c| c.slug.clone())
            .unwrap_or_default();
        (name, slug, 0u32)
    } else {
        let t = simulator_data
            .team(match_result.home_team_id)
            .ok_or_else(|| ApiError::NotFound("Home team not found".to_string()))?;
        (t.name.clone(), t.slug.clone(), t.club_id)
    };

    let (away_team_name, away_team_slug, away_club_id) = if is_international {
        let name = simulator_data
            .country(match_result.away_team_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "Away".to_string());
        let slug = simulator_data
            .country(match_result.away_team_id)
            .map(|c| c.slug.clone())
            .unwrap_or_default();
        (name, slug, 0u32)
    } else {
        let t = simulator_data
            .team(match_result.away_team_id)
            .ok_or_else(|| ApiError::NotFound("Away team not found".to_string()))?;
        (t.name.clone(), t.slug.clone(), t.club_id)
    };

    let result_details = match_result
        .details
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Match details not available".to_string()))?;

    let score = result_details
        .score
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Match score not available".to_string()))?;

    let viewer_goals: Vec<GoalEventJson> = score
        .detail()
        .iter()
        .filter(|goal| goal.stat_type == MatchStatisticType::Goal)
        .map(|goal| GoalEventJson {
            player_id: goal.player_id,
            time: goal.time,
            is_auto_goal: goal.is_auto_goal,
        })
        .collect();

    let mut viewer_players: Vec<PlayerJson> = Vec::new();

    // Assign squad numbers (1-based) per team when shirt_number is not set
    let mut home_number: u8 = 1;
    PlayerJson::append(
        &mut viewer_players,
        simulator_data,
        &result_details.left_team_players.main,
        true,
        &mut home_number,
    );
    PlayerJson::append(
        &mut viewer_players,
        simulator_data,
        &result_details.left_team_players.substitutes,
        true,
        &mut home_number,
    );

    let mut away_number: u8 = 1;
    PlayerJson::append(
        &mut viewer_players,
        simulator_data,
        &result_details.right_team_players.main,
        false,
        &mut away_number,
    );
    PlayerJson::append(
        &mut viewer_players,
        simulator_data,
        &result_details.right_team_players.substitutes,
        false,
        &mut away_number,
    );

    let home_goals = score.home_team.get();
    let away_goals = score.away_team.get();

    let home_goal_events: Vec<GoalEventDisplay> = score
        .detail()
        .iter()
        .filter(|g| g.stat_type == MatchStatisticType::Goal)
        .filter(|g| {
            let is_home_player = result_details.left_team_players.main.contains(&g.player_id)
                || result_details
                    .left_team_players
                    .substitutes
                    .contains(&g.player_id);
            if g.is_auto_goal {
                !is_home_player
            } else {
                is_home_player
            }
        })
        .map(|g| {
            let player_name = simulator_data
                .player(g.player_id)
                .map(|p| {
                    format!(
                        "{} {}",
                        p.full_name.display_first_name(),
                        p.full_name.display_last_name()
                    )
                })
                .unwrap_or_else(|| "Unknown".to_string());
            let minute = if result_details.match_time_ms > 0 {
                (g.time * 90 / result_details.match_time_ms) as u32
            } else {
                0
            };
            GoalEventDisplay {
                player_slug: player_history_slug(simulator_data, g.player_id, &player_name),
                player_name,
                minute,
                is_auto_goal: g.is_auto_goal,
            }
        })
        .collect();

    let away_goal_events: Vec<GoalEventDisplay> = score
        .detail()
        .iter()
        .filter(|g| g.stat_type == MatchStatisticType::Goal)
        .filter(|g| {
            let is_away_player = result_details
                .right_team_players
                .main
                .contains(&g.player_id)
                || result_details
                    .right_team_players
                    .substitutes
                    .contains(&g.player_id);
            if g.is_auto_goal {
                !is_away_player
            } else {
                is_away_player
            }
        })
        .map(|g| {
            let player_name = simulator_data
                .player(g.player_id)
                .map(|p| {
                    format!(
                        "{} {}",
                        p.full_name.display_first_name(),
                        p.full_name.display_last_name()
                    )
                })
                .unwrap_or_else(|| "Unknown".to_string());
            let minute = if result_details.match_time_ms > 0 {
                (g.time * 90 / result_details.match_time_ms) as u32
            } else {
                0
            };
            GoalEventDisplay {
                player_slug: player_history_slug(simulator_data, g.player_id, &player_name),
                player_name,
                minute,
                is_auto_goal: g.is_auto_goal,
            }
        })
        .collect();

    let motm_id = result_details.player_of_the_match_id;
    let motm_name = motm_id
        .and_then(|id| simulator_data.player(id))
        .map(|p| {
            format!(
                "{} {}",
                p.full_name.display_first_name(),
                p.full_name.display_last_name()
            )
        })
        .unwrap_or_default();
    let motm_slug = motm_id
        .map(|id| player_history_slug(simulator_data, id, &motm_name))
        .unwrap_or_default();

    let title = format!("{} - {}", home_team_name, away_team_name);

    // What the header's left rail names the match after. This used to be the
    // layout's sub-title, and the scoreboard that replaced it inherits the
    // sub-title's resolution rather than the raw league name: the display name
    // is the localised, country-qualified one, and a continental tie is not a
    // league at all — it has its own page, and `/leagues/champions-league`
    // is not it.
    let (competition_name, competition_url) = if let Some(l) = league {
        (
            views::league_display_name(l, &i18n, simulator_data),
            format!("/{}/leagues/{}", &route_params.lang, &l.slug),
        )
    } else {
        let name = match match_result.league_slug.as_str() {
            "champions-league" => "Champions League",
            "europa-league" => "Europa League",
            "conference-league" => "Conference League",
            _ => "International",
        };
        let link = match match_result.league_slug.as_str() {
            "champions-league" => format!("/{}/champions-league", &route_params.lang),
            "europa-league" => format!("/{}/europa-league", &route_params.lang),
            "conference-league" => format!("/{}/conference-league", &route_params.lang),
            _ => String::new(),
        };
        (name.to_string(), link)
    };

    let viewer_config = ViewerConfigJson {
        canvas: "#match-canvas",
        api_base: format!("/api/match/{}", route_params.match_id),
        match_time_ms: result_details.match_time_ms,
        home: TeamColorsJson::for_club(simulator_data, home_club_id, "#00307d"),
        away: TeamColorsJson::for_club(simulator_data, away_club_id, "#b33f00"),
        players: viewer_players,
        goals: viewer_goals,
        labels: ViewerLabelsJson {
            first_half: i18n.t("first_half").to_string(),
            second_half: i18n.t("second_half").to_string(),
            loading: i18n.t("loading_match").to_string(),
            no_recording: i18n.t("match_no_recording").to_string(),
        },
    };

    Ok(MatchGetTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        title,
        header_color: String::new(),
        foreground_color: String::new(),
        menu_sections: vec![],
        i18n,
        lang: route_params.lang.clone(),
        competition_name,
        competition_url,
        home_team_name: home_team_name.clone(),
        home_team_slug: home_team_slug.clone(),
        home_goals,
        home_goal_events,
        home_squad_main: result_details
            .left_team_players
            .main
            .iter()
            .filter_map(|pid| {
                let mut p = to_match_player(*pid, simulator_data, motm_id, result_details)?;
                if let Some(sub) = result_details
                    .substitutions
                    .iter()
                    .find(|s| s.player_out_id == *pid)
                {
                    p.subbed_off_minute = Some(sub_time_to_minute(
                        sub.match_time_ms,
                        result_details.match_time_ms,
                    ));
                }
                Some(p)
            })
            .collect(),
        home_squad_subs: result_details
            .left_team_players
            .substitutes
            .iter()
            .filter_map(|pid| {
                let mut p = to_match_player(*pid, simulator_data, motm_id, result_details)?;
                if let Some(sub) = result_details
                    .substitutions
                    .iter()
                    .find(|s| s.player_in_id == *pid)
                {
                    p.sub_minute = Some(sub_time_to_minute(
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
                    p.subbed_off_minute = Some(sub_time_to_minute(
                        sub_off.match_time_ms,
                        result_details.match_time_ms,
                    ));
                }
                Some(p)
            })
            .collect(),
        away_team_name: away_team_name.clone(),
        away_team_slug: away_team_slug.clone(),
        away_goals,
        away_goal_events,
        away_squad_main: result_details
            .right_team_players
            .main
            .iter()
            .filter_map(|pid| {
                let mut p = to_match_player(*pid, simulator_data, motm_id, result_details)?;
                if let Some(sub) = result_details
                    .substitutions
                    .iter()
                    .find(|s| s.player_out_id == *pid)
                {
                    p.subbed_off_minute = Some(sub_time_to_minute(
                        sub.match_time_ms,
                        result_details.match_time_ms,
                    ));
                }
                Some(p)
            })
            .collect(),
        away_squad_subs: result_details
            .right_team_players
            .substitutes
            .iter()
            .filter_map(|pid| {
                let mut p = to_match_player(*pid, simulator_data, motm_id, result_details)?;
                if let Some(sub) = result_details
                    .substitutions
                    .iter()
                    .find(|s| s.player_in_id == *pid)
                {
                    p.sub_minute = Some(sub_time_to_minute(
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
                    p.subbed_off_minute = Some(sub_time_to_minute(
                        sub_off.match_time_ms,
                        result_details.match_time_ms,
                    ));
                }
                Some(p)
            })
            .collect(),
        viewer_config_json: viewer_config.to_script_literal(),
        match_viewer_available: MATCH_VIEWER_AVAILABLE,
        match_viewer_version: MATCH_VIEWER_VERSION,
        player_of_the_match_id: motm_id.unwrap_or(0),
        player_of_the_match_slug: motm_slug,
        player_of_the_match_name: motm_name,
        // The same flag that decided whether to record decides whether to
        // offer the replay — including for a friendly, which is a youth or
        // reserve league fixture here (`League::friendly`). See `Match::play`
        // for why those are no longer a special case.
        match_recordings_enabled: MatchRuntime::recordings_mode(),
    })
}

fn to_match_player(
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

fn sub_time_to_minute(match_time_ms: u64, total_match_time_ms: u64) -> u32 {
    if total_match_time_ms == 0 {
        return 0;
    }
    (match_time_ms * 90 / total_match_time_ms) as u32
}

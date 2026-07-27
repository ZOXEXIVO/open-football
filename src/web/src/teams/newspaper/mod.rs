pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::slug::player_history_slug;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::club::news::{IssueResult, NewsStory, NewsStoryKind, NewspaperIssue};
use core::shared::fullname::FullName;
use core::utils::FormattingUtils;
use core::{Club, SimulatorData, TeamType};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamNewspaperRequest {
    lang: String,
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/newspaper/index.html")]
#[allow(dead_code)]
pub struct TeamNewspaperTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub i18n: I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub active_tab: &'static str,
    pub show_finances_tab: bool,
    pub show_academy_tab: bool,
    /// Newest edition first. Empty until the club's first press run.
    pub issues: Vec<IssueView>,
}

/// One printed edition, fully typeset for the page.
pub struct IssueView {
    pub number: u32,
    pub masthead: String,
    pub date: String,
    pub mood_label: String,
    pub mood_slug: &'static str,
    pub mood_stamped: bool,
    pub lead: Option<StoryView>,
    /// The two pieces set alongside the lead.
    pub secondary: Vec<StoryView>,
    /// Everything else, set as single-line briefs.
    pub briefs: Vec<StoryView>,
    pub results: Vec<ResultView>,
    pub portrait: Option<PortraitView>,
}

pub struct StoryView {
    pub kicker: String,
    pub headline: String,
    pub body: String,
    pub date: String,
    pub player_slug: String,
    pub player_name: String,
}

/// The photograph on the front page, with the caption set under it.
pub struct PortraitView {
    pub player_id: u32,
    pub player_slug: String,
    pub player_name: String,
    pub player_generated: bool,
    pub caption: String,
}

pub struct ResultView {
    /// Compact day-and-month, the form a ruled results column uses.
    pub date: String,
    pub opponent_name: String,
    pub opponent_slug: String,
    pub score: String,
    /// `w` / `d` / `l` — drives the result mark in the ruled column.
    pub outcome: &'static str,
}

pub async fn team_newspaper_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamNewspaperRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let i18n = state.i18n.for_lang(&route_params.lang);

    let team_id = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?
        .slug_indexes
        .get_team_by_slug(&route_params.team_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Team '{}' not found", route_params.team_slug))
        })?;

    let team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    let club = simulator_data.club(team.club_id).ok_or_else(|| {
        ApiError::InternalError(format!("Club with ID {} not found", team.club_id))
    })?;

    let league = team.league_id.and_then(|id| simulator_data.league(id));

    let issues = PressDesk::typeset(simulator_data, club, &i18n);

    let (neighbor_teams, country_leagues) =
        NewspaperPage::menu_sources(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams
        .iter()
        .map(|(name, slug)| (name.as_str(), slug.as_str()))
        .collect();
    let league_refs: Vec<(&str, &str)> = country_leagues
        .iter()
        .map(|(name, slug)| (name.as_str(), slug.as_str()))
        .collect();

    let (country_name, country_slug) = views::club_country_info(simulator_data, team.club_id);
    let current_path = format!("/{}/teams/{}/newspaper", &route_params.lang, &team.slug);
    let menu_params = views::MenuParams {
        i18n: &i18n,
        lang: &route_params.lang,
        current_path: &current_path,
        country_name,
        country_slug,
    };
    let menu_sections = views::team_menu(&menu_params, &neighbor_refs, &league_refs);

    let league_title = league
        .map(|l| views::league_display_name(l, &i18n, simulator_data))
        .unwrap_or_default();

    Ok(TeamNewspaperTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        i18n,
        lang: route_params.lang.clone(),
        title: team.name.clone(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league_title,
        sub_title_link: league
            .map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug))
            .unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: club.colors.background.clone(),
        foreground_color: club.colors.foreground.clone(),
        menu_sections,
        team_slug: team.slug.clone(),
        active_tab: "newspaper",
        show_finances_tab: team.team_type.is_own_team(),
        show_academy_tab: team.team_type == TeamType::Main || team.team_type == TeamType::U18,
        issues,
    })
}

/// Turns the club's stored editions into pages a reader can look at:
/// names resolved, money formatted, prose translated.
struct PressDesk;

impl PressDesk {
    /// Pieces set alongside the lead before the rest drop into briefs.
    const SECONDARY_SLOTS: usize = 2;

    fn typeset(data: &SimulatorData, club: &Club, i18n: &I18n) -> Vec<IssueView> {
        let masthead = i18n
            .t(club.newsroom.masthead_key())
            .replace("{club}", &club.name);

        club.newsroom
            .issues
            .iter()
            .map(|issue| Self::issue(data, club, issue, &masthead, i18n))
            .collect()
    }

    fn issue(
        data: &SimulatorData,
        club: &Club,
        issue: &NewspaperIssue,
        masthead: &str,
        i18n: &I18n,
    ) -> IssueView {
        let mut stories = issue
            .stories
            .iter()
            .map(|story| Self::story(data, club, story, i18n))
            .collect::<Vec<_>>()
            .into_iter();

        let lead = stories.next();
        let secondary: Vec<StoryView> = stories.by_ref().take(Self::SECONDARY_SLOTS).collect();
        let briefs: Vec<StoryView> = stories.collect();

        let portrait = Self::portrait(data, club, issue, i18n);

        IssueView {
            number: issue.number,
            masthead: masthead.to_string(),
            date: i18n.format_date(issue.date),
            mood_label: i18n.t(issue.mood.i18n_key()).to_string(),
            mood_slug: issue.mood.slug(),
            mood_stamped: issue.mood.is_stamped(),
            lead,
            secondary,
            briefs,
            results: issue
                .results
                .iter()
                .map(|result| Self::result(data, result))
                .collect(),
            portrait,
        }
    }

    /// The first story on the page with a face behind it. A paper never
    /// runs a front page of pure text when it has a picture available.
    fn portrait(
        data: &SimulatorData,
        club: &Club,
        issue: &NewspaperIssue,
        i18n: &I18n,
    ) -> Option<PortraitView> {
        let story = issue.stories.iter().find(|story| story.player_id != 0)?;
        let player = data.player(story.player_id)?;

        Some(PortraitView {
            player_id: player.id,
            player_slug: player.slug(),
            player_name: PlayerName::display(&player.full_name),
            player_generated: player.is_generated(),
            caption: StoryComposer::headline(data, story, i18n, &club.name),
        })
    }

    fn story(data: &SimulatorData, club: &Club, story: &NewsStory, i18n: &I18n) -> StoryView {
        let (player_name, player_slug) = PlayerName::resolve(data, story.player_id);

        StoryView {
            kicker: i18n.t(story.kind.desk().i18n_key()).to_string(),
            headline: StoryComposer::headline(data, story, i18n, &club.name),
            body: StoryComposer::body(data, story, i18n, &club.name),
            date: i18n.format_date(story.date),
            player_slug,
            player_name,
        }
    }

    fn result(data: &SimulatorData, result: &IssueResult) -> ResultView {
        let (opponent_name, opponent_slug) = data
            .team_data(result.opponent_team_id)
            .map(|team| (team.name.clone(), team.slug.clone()))
            .unwrap_or_else(|| (String::new(), String::new()));

        ResultView {
            date: result.date.format("%d.%m").to_string(),
            opponent_name,
            opponent_slug,
            score: format!("{}-{}", result.goals_for, result.goals_against),
            outcome: if result.is_win() {
                "w"
            } else if result.is_draw() {
                "d"
            } else {
                "l"
            },
        }
    }
}

/// Fills the blanks in a translated headline or body with the real
/// names, numbers and money of the story it belongs to.
struct StoryComposer;

impl StoryComposer {
    fn headline(data: &SimulatorData, story: &NewsStory, i18n: &I18n, club_name: &str) -> String {
        Self::compose(
            i18n.t(&format!("news_h_{}", story.kind.key_stem())),
            data,
            story,
            i18n,
            club_name,
        )
    }

    fn body(data: &SimulatorData, story: &NewsStory, i18n: &I18n, club_name: &str) -> String {
        Self::compose(
            i18n.t(&format!("news_b_{}", story.kind.key_stem())),
            data,
            story,
            i18n,
            club_name,
        )
    }

    fn compose(
        template: &str,
        data: &SimulatorData,
        story: &NewsStory,
        i18n: &I18n,
        club_name: &str,
    ) -> String {
        let mut text = template.to_string();

        if text.contains("{club}") {
            text = text.replace("{club}", club_name);
        }
        if text.contains("{player}") {
            let (name, _) = PlayerName::resolve(data, story.player_id);
            let name = if name.is_empty() {
                i18n.t("newspaper_unnamed_player").to_string()
            } else {
                name
            };
            text = text.replace("{player}", &name);
        }
        // `{opponent}` names the other team in a match report, `{other}`
        // the other club in a transfer. Both come from the same slot.
        if text.contains("{opponent}") || text.contains("{other}") {
            let mut name = Self::other_party(data, story);
            if name.is_empty() {
                name = i18n.t("newspaper_another_club").to_string();
            }
            text = text.replace("{opponent}", &name).replace("{other}", &name);
        }
        if text.contains("{score}") {
            text = text.replace("{score}", &format!("{}-{}", story.a, story.b));
        }
        if text.contains("{fee}") {
            text = text.replace(
                "{fee}",
                &format!("${}", FormattingUtils::format_money(story.money as f64)),
            );
        }
        if text.contains("{rating}") {
            text = text.replace("{rating}", &format!("{:.2}", story.b as f32 / 100.0));
        }
        if text.contains("{n}") {
            text = text.replace("{n}", &story.a.to_string());
        }
        if text.contains("{pts}") {
            text = text.replace("{pts}", &story.b.to_string());
        }

        text
    }

    /// Match reports name a team; transfer stories name a club. Both
    /// arrive in the same slot, so resolve whichever the kind implies.
    fn other_party(data: &SimulatorData, story: &NewsStory) -> String {
        if story.other_id == 0 {
            return String::new();
        }

        if Self::is_market_story(story.kind) {
            return data
                .club(story.other_id)
                .map(|club| club.name.clone())
                .unwrap_or_default();
        }

        data.team_data(story.other_id)
            .map(|team| team.name.clone())
            .unwrap_or_default()
    }

    fn is_market_story(kind: NewsStoryKind) -> bool {
        matches!(
            kind,
            NewsStoryKind::NewSigning
                | NewsStoryKind::RecordSigning
                | NewsStoryKind::FreeSigning
                | NewsStoryKind::LoanArrival
                | NewsStoryKind::PlayerSold
                | NewsStoryKind::StarSold
                | NewsStoryKind::LoanExit
        )
    }
}

/// How the paper writes a person's name, and where it links to. A
/// player who has already moved on still gets a working byline — the
/// edition that reported the sale is read long after he has gone.
struct PlayerName;

impl PlayerName {
    fn display(full: &FullName) -> String {
        let first = full.display_first_name();
        let last = full.display_last_name();
        if first.is_empty() {
            last.to_string()
        } else {
            format!("{} {}", first, last)
        }
    }

    /// `(display name, url slug)`. Both are empty when the story is
    /// about the club rather than a person.
    fn resolve(data: &SimulatorData, player_id: u32) -> (String, String) {
        if player_id == 0 {
            return (String::new(), String::new());
        }

        if let Some(player) = data.player(player_id) {
            return (Self::display(&player.full_name), player.slug());
        }
        if let Some(player) = data.retired_player(player_id) {
            return (Self::display(&player.full_name), player.slug());
        }

        (String::new(), player_history_slug(data, player_id, ""))
    }
}

/// Left-menu sources shared with every other team tab.
struct NewspaperPage;

impl NewspaperPage {
    fn menu_sources(
        club_id: u32,
        data: &SimulatorData,
        i18n: &I18n,
    ) -> Result<(Vec<(String, String)>, Vec<(String, String)>), ApiError> {
        let club = data.club(club_id).ok_or_else(|| {
            ApiError::InternalError(format!("Club with ID {} not found", club_id))
        })?;

        let teams = views::neighbor_teams(club, i18n);

        let mut country_leagues: Vec<(u32, String, String)> = data
            .country_by_club(club_id)
            .map(|country| {
                country
                    .leagues
                    .leagues
                    .iter()
                    .filter(|league| !league.friendly)
                    .map(|league| (league.id, league.name.clone(), league.slug.clone()))
                    .collect()
            })
            .unwrap_or_default();
        country_leagues.sort_by_key(|(id, _, _)| *id);

        Ok((
            teams,
            country_leagues
                .into_iter()
                .map(|(_, name, slug)| (name, slug))
                .collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use core::club::news::{NewsDesk, NewsStoryKind, PressMood};
    use std::collections::BTreeMap;

    /// Every locale the newspaper has to print in.
    const BUNDLES: &[(&str, &[u8])] = &[
        ("en", include_bytes!("../../../assets/i18n/en.json")),
        ("de", include_bytes!("../../../assets/i18n/de.json")),
        ("es", include_bytes!("../../../assets/i18n/es.json")),
        ("fr", include_bytes!("../../../assets/i18n/fr.json")),
        ("ja", include_bytes!("../../../assets/i18n/ja.json")),
        ("pt", include_bytes!("../../../assets/i18n/pt.json")),
        ("ru", include_bytes!("../../../assets/i18n/ru.json")),
        ("tr", include_bytes!("../../../assets/i18n/tr.json")),
        ("zh", include_bytes!("../../../assets/i18n/zh.json")),
    ];

    /// Collects the keys the page asks for at render time, so a missing
    /// one fails here instead of printing a raw key on a front page.
    struct PressKeys;

    impl PressKeys {
        fn bundle(lang: &str) -> BTreeMap<String, String> {
            let (_, bytes) = BUNDLES.iter().find(|(code, _)| *code == lang).unwrap();
            serde_json::from_slice(bytes)
                .unwrap_or_else(|e| panic!("{}.json is not valid JSON: {}", lang, e))
        }

        fn required() -> Vec<String> {
            let mut keys: Vec<String> = Vec::new();

            for kind in NewsStoryKind::ALL {
                keys.push(format!("news_h_{}", kind.key_stem()));
                keys.push(format!("news_b_{}", kind.key_stem()));
                keys.push(kind.desk().i18n_key().to_string());
            }

            for desk in [
                NewsDesk::Match,
                NewsDesk::Squad,
                NewsDesk::Market,
                NewsDesk::Boardroom,
            ] {
                keys.push(desk.i18n_key().to_string());
            }

            for mood in [
                PressMood::Triumph,
                PressMood::Upbeat,
                PressMood::Steady,
                PressMood::Uneasy,
                PressMood::Crisis,
            ] {
                keys.push(mood.i18n_key().to_string());
            }

            for masthead in [
                "masthead_gazette",
                "masthead_chronicle",
                "masthead_herald",
                "masthead_courier",
                "masthead_post",
                "masthead_sentinel",
            ] {
                keys.push(masthead.to_string());
            }

            for chrome in [
                "newspaper",
                "newspaper_edition",
                "newspaper_results",
                "newspaper_in_brief",
                "newspaper_back_issues",
                "newspaper_our_correspondent",
                "newspaper_presses_idle",
                "newspaper_no_issues",
                "newspaper_unnamed_player",
                "newspaper_another_club",
            ] {
                keys.push(chrome.to_string());
            }

            keys.sort();
            keys.dedup();
            keys
        }
    }

    #[test]
    fn every_story_kind_has_copy_in_every_locale() {
        let required = PressKeys::required();

        for (lang, _) in BUNDLES {
            let bundle = PressKeys::bundle(lang);
            let missing: Vec<&str> = required
                .iter()
                .filter(|key| !bundle.contains_key(*key))
                .map(String::as_str)
                .collect();

            assert!(
                missing.is_empty(),
                "{}.json is missing {} newspaper key(s): {:?}",
                lang,
                missing.len(),
                missing
            );
        }
    }

    /// A masthead that forgot `{club}` would print the same title for
    /// every club in the world.
    #[test]
    fn every_masthead_names_its_club() {
        for (lang, _) in BUNDLES {
            let bundle = PressKeys::bundle(lang);
            for key in bundle.keys().filter(|key| key.starts_with("masthead_")) {
                assert!(
                    bundle[key].contains("{club}"),
                    "{}.json key {} does not name the club",
                    lang,
                    key
                );
            }
        }
    }
}

#[cfg(test)]
mod render_tests {
    use super::{IssueView, PortraitView, ResultView, StoryView, TeamNewspaperTemplate};
    use crate::I18n;
    use askama::Template;
    use std::collections::HashMap;

    /// Builds a page the way the handler would, so the template itself
    /// is exercised — the app can render a front page without anyone
    /// having to start the server to find out.
    struct Page;

    impl Page {
        /// Real English chrome, so a preview screenshot shows the page a
        /// reader gets rather than a grid of raw keys.
        fn copy() -> HashMap<String, String> {
            [
                ("newspaper", "Newspaper"),
                ("newspaper_edition", "Edition"),
                ("newspaper_results", "Results"),
                ("newspaper_in_brief", "In brief"),
                ("newspaper_back_issues", "Back issues"),
                (
                    "newspaper_our_correspondent",
                    "From our football correspondent",
                ),
                ("newspaper_presses_idle", "The presses are idle"),
                (
                    "newspaper_no_issues",
                    "Nothing has been printed about this club yet.",
                ),
                ("site_name", "Open Football"),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
        }

        fn template(issues: Vec<IssueView>) -> TeamNewspaperTemplate {
            TeamNewspaperTemplate {
                css_version: "test",
                computer_name: "test",
                cpu_brand: "test",
                cores_count: 1,
                i18n: I18n::for_test(Self::copy()),
                lang: "en".to_string(),
                title: "Córdoba".to_string(),
                sub_title_prefix: String::new(),
                sub_title_suffix: String::new(),
                sub_title: "Spanish Segunda".to_string(),
                sub_title_link: "/en/leagues/spanish-segunda".to_string(),
                sub_title_country_code: String::new(),
                header_color: "#1e272d".to_string(),
                foreground_color: "#ffffff".to_string(),
                menu_sections: Vec::new(),
                team_slug: "cordoba".to_string(),
                active_tab: "newspaper",
                show_finances_tab: true,
                show_academy_tab: true,
                issues,
            }
        }

        fn story(kicker: &str, headline: &str, body: &str, player: bool) -> StoryView {
            StoryView {
                kicker: kicker.to_string(),
                headline: headline.to_string(),
                body: body.to_string(),
                date: "2 March 2026".to_string(),
                player_slug: if player {
                    "17-diego-mora".to_string()
                } else {
                    String::new()
                },
                player_name: if player {
                    "Diego Mora".to_string()
                } else {
                    String::new()
                },
            }
        }

        fn full_issue() -> IssueView {
            IssueView {
                number: 12,
                masthead: "The Córdoba Chronicle".to_string(),
                date: "2 March 2026".to_string(),
                mood_label: "Triumph".to_string(),
                mood_slug: "triumph",
                mood_stamped: true,
                lead: Some(Self::story(
                    "Match",
                    "Córdoba tear Sevilla apart",
                    "Rarely has this fixture seen a display like it: 4-1 against Sevilla, \
                     and it could have been more.",
                    false,
                )),
                secondary: vec![
                    Self::story(
                        "Squad",
                        "Three goals for Diego Mora",
                        "The match ball belongs to Diego Mora, whose 3 goals turned a \
                         difficult afternoon into a procession.",
                        true,
                    ),
                    Self::story(
                        "Market",
                        "Córdoba break the bank for Iván Salas",
                        "$12.5M. No player has ever cost Córdoba more, and expectation \
                         arrives with him.",
                        true,
                    ),
                ],
                briefs: vec![
                    Self::story("Squad", "Diego Mora back in training", "", true),
                    Self::story("Match", "Córdoba make it 4 in a row", "", false),
                    Self::story("Market", "Rubén Ortega placed on the list", "", true),
                    Self::story("Boardroom", "Córdoba board back the manager", "", false),
                    Self::story("Squad", "180 appearances for Andrés Vidal", "", true),
                    Self::story("Squad", "Nico Reyes out for 42 days", "", true),
                ],
                results: vec![
                    ResultView {
                        date: "25.02".to_string(),
                        opponent_name: "Real Zaragoza".to_string(),
                        opponent_slug: "real-zaragoza".to_string(),
                        score: "1-1".to_string(),
                        outcome: "d",
                    },
                    ResultView {
                        date: "01.03".to_string(),
                        opponent_name: "Sevilla".to_string(),
                        opponent_slug: "sevilla".to_string(),
                        score: "4-1".to_string(),
                        outcome: "w",
                    },
                ],
                portrait: Some(PortraitView {
                    player_id: 17,
                    player_slug: "17-diego-mora".to_string(),
                    player_name: "Diego Mora".to_string(),
                    player_generated: true,
                    caption: "Three goals for Diego Mora".to_string(),
                }),
            }
        }

        /// A quiet week: stories but no fixtures and nothing left over
        /// for the briefs column.
        fn bare_issue() -> IssueView {
            IssueView {
                number: 11,
                masthead: "The Córdoba Chronicle".to_string(),
                date: "23 February 2026".to_string(),
                mood_label: "Steady".to_string(),
                mood_slug: "steady",
                mood_stamped: false,
                lead: Some(Self::story(
                    "Boardroom",
                    "Córdoba board back the manager",
                    "A public word of support from the board and, more usefully, an \
                     understanding about what the next window will look like.",
                    false,
                )),
                secondary: Vec::new(),
                briefs: Vec::new(),
                results: Vec::new(),
                portrait: None,
            }
        }
    }

    #[test]
    fn an_idle_newsroom_renders_its_empty_state() {
        let html = Page::template(Vec::new()).render().unwrap();

        assert!(html.contains("np-empty"));
        assert!(!html.contains("np-sheet"));
    }

    #[test]
    fn a_full_front_page_renders_every_department() {
        let html = Page::template(vec![Page::full_issue(), Page::bare_issue()])
            .render()
            .unwrap();

        assert!(html.contains("The Córdoba Chronicle"));
        assert!(html.contains("Córdoba tear Sevilla apart"));
        assert!(html.contains("np-folio-mood-triumph np-stamp"));
        assert!(html.contains("/api/players/17/face.svg"));
        assert!(html.contains("np-result np-result-w"));
        assert!(html.contains("/en/teams/sevilla"));
        assert!(html.contains("np-brief-link"));
    }

    /// Back issues are set exactly like today's paper — same nameplate,
    /// same folio, same columns — with a labelled rule between them.
    #[test]
    fn a_back_issue_is_printed_as_a_full_edition() {
        let html = Page::template(vec![Page::full_issue(), Page::bare_issue()])
            .render()
            .unwrap();

        assert_eq!(html.matches("np-sheet").count(), 2);
        assert_eq!(html.matches("np-masthead").count(), 2);
        assert_eq!(html.matches("np-folio\"").count(), 2);
        assert_eq!(html.matches("np-archive-rule").count(), 1);
        assert!(html.contains("Córdoba board back the manager"));
        // The rule separates the two, so it cannot precede the front page.
        assert!(html.find("np-sheet").unwrap() < html.find("np-archive-rule").unwrap());
    }

    #[test]
    fn a_week_without_fixtures_runs_the_text_full_measure() {
        let html = Page::template(vec![Page::bare_issue()]).render().unwrap();

        assert!(
            html.contains("np-body np-body-full"),
            "with no results and no briefs the rail must collapse"
        );
        assert!(!html.contains("np-stamp"));
    }

    /// Writes a self-contained copy of the front page so the design can
    /// be looked at in a browser without starting the server:
    ///
    /// ```text
    /// NEWSPAPER_PREVIEW_DIR=<dir> cargo test -p web --lib newspaper_preview -- --ignored
    /// ```
    #[test]
    #[ignore]
    fn newspaper_preview() {
        let Ok(dir) = std::env::var("NEWSPAPER_PREVIEW_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).expect("preview dir");

        let page = Page::template(vec![Page::full_issue(), Page::bare_issue()])
            .render()
            .expect("render");

        // The rendered page links assets by absolute URL, which a
        // `file://` load cannot resolve — inline the two stylesheets the
        // layout actually depends on and drop the rest.
        let bootstrap =
            std::fs::read_to_string("assets/static/css/bootstrap.min.css").expect("bootstrap");
        let style = std::fs::read_to_string("assets/static/css/style.css").expect("stylesheet");

        let mut html = page;
        for link in [
            "<link href=\"/static/css/bootstrap.min.css\" rel=\"stylesheet\">",
            "<link href=\"/static/css/flags.css\" rel=\"stylesheet\">",
            "<link href=\"/static/css/font.min.css\" rel=\"stylesheet\">",
        ] {
            html = html.replace(link, "");
        }
        if let Some(start) = html.find("<link href=\"/static/css/styles.min.css") {
            if let Some(end) = html[start..].find('>') {
                html.replace_range(start..start + end + 1, "");
            }
        }
        html = html.replace(
            "</head>",
            &format!("<style>{bootstrap}</style><style>{style}</style></head>"),
        );

        std::fs::write(std::path::Path::new(&dir).join("newspaper.html"), html)
            .expect("write preview");
    }
}

pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::teams::newspaper::{IssueView, PaperFor, PressDesk, PressFocus};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n, NewsI18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::SimulatorData;
use core::league::League;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct LeagueNewspaperRequest {
    pub lang: String,
    pub league_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "leagues/newspaper/index.html")]
pub struct LeagueNewspaperTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub title: String,
    /// The monthly's own name — "The Serie A Chronicle" — which titles
    /// the browser tab. Same rule as a club's paper: the tab names the
    /// publication, not the section it sits under.
    pub masthead: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: I18n,
    /// Press copy, scoped apart from `i18n`.
    pub news: NewsI18n,
    pub lang: String,
    pub league_slug: String,
    /// Editions on the shelf, for the tabbar badge.
    pub newspaper_count: usize,
    /// Newest month first. Empty until the division's first press run.
    pub issues: Vec<IssueView>,
}

pub async fn league_newspaper_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<LeagueNewspaperRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let news = state.news_i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let indexes = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?;

    let league_id = indexes
        .slug_indexes
        .get_league_by_slug(&route_params.league_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("League '{}' not found", route_params.league_slug))
        })?;

    let league = simulator_data
        .league(league_id)
        .ok_or_else(|| ApiError::NotFound(format!("League with ID {} not found", league_id)))?;

    let country = simulator_data.country(league.country_id).ok_or_else(|| {
        ApiError::NotFound(format!("Country with ID {} not found", league.country_id))
    })?;

    let league_title = views::league_display_name(league, &i18n, simulator_data);
    let masthead = PressDesk::masthead(league.newsroom.masthead_key(), &league.name, &news);
    let issues = LeaguePress::typeset(simulator_data, league, &masthead, &i18n, &news);

    Ok(LeagueNewspaperTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        title: format!("{} — {}", league_title, i18n.t("news")),
        masthead,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: country.name.clone(),
        sub_title_link: format!("/{}/countries/{}", &route_params.lang, &country.slug),
        sub_title_country_code: country.code.clone(),
        header_color: country.background_color.clone(),
        foreground_color: country.foreground_color.clone(),
        menu_sections: {
            let mut cl: Vec<(u32, &str, &str)> = country
                .leagues
                .leagues
                .iter()
                .filter(|l| !l.friendly)
                .map(|l| (l.id, l.name.as_str(), l.slug.as_str()))
                .collect();
            cl.sort_by_key(|(id, _, _)| *id);
            let cl_refs: Vec<(&str, &str)> = cl.iter().map(|(_, n, s)| (*n, *s)).collect();
            let current_path =
                format!("/{}/leagues/{}/newspaper", &route_params.lang, &league.slug);
            let mp = views::MenuParams {
                i18n: &i18n,
                lang: &route_params.lang,
                current_path: &current_path,
                country_name: &country.name,
                country_slug: &country.slug,
            };
            views::league_menu(
                &mp,
                &cl_refs,
                country
                    .domestic_cup
                    .as_ref()
                    .map(|c| (c.league.name.as_str(), c.league.slug.as_str())),
                &country
                    .playoffs
                    .iter()
                    .map(|p| (p.league.name.as_str(), p.league.slug.as_str()))
                    .collect::<Vec<_>>(),
            )
        },
        league_slug: league.slug.clone(),
        newspaper_count: LeagueNewspaperCounter::count(league),
        issues,
        lang: route_params.lang,
        i18n,
        news,
    })
}

/// How many monthly editions the division has on the shelf, for the
/// tabbar badge.
///
/// Editions, not stories — the same contract as the club and player
/// tabs: the badge counts what the reader scrolls past. Bounded by
/// `LeagueNewsroom::MAX_ISSUES`, so it settles at twelve and stays
/// there once the division has been running a year.
pub struct LeagueNewspaperCounter;

impl LeagueNewspaperCounter {
    pub fn count(league: &League) -> usize {
        league.newsroom.issues.len()
    }
}

/// Sets the division's stored months through the club page's own
/// typesetter.
///
/// It generates nothing and it owns no markup. The one thing it has to
/// say that a club page does not is who `{club}` is: on this page every
/// story belongs to a different club, so the credit is resolved per
/// story rather than per edition — see [`PaperFor`].
struct LeaguePress;

impl LeaguePress {
    fn typeset(
        data: &SimulatorData,
        league: &League,
        masthead: &str,
        i18n: &I18n,
        news: &NewsI18n,
    ) -> Vec<IssueView> {
        let paper = PaperFor::Division(&league.name);

        league
            .newsroom
            .issues
            .iter()
            // A division's paper is read by everybody in it, so nothing
            // on it is marked for anybody.
            .map(|issue| {
                PressDesk::issue(data, paper, issue, masthead, i18n, news, PressFocus::none())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::LeagueNewspaperTemplate;
    use crate::teams::newspaper::{IssueView, PortraitView, Prose, Span, StoryView};
    use crate::{I18n, NewsI18n};
    use askama::Template;
    use std::collections::HashMap;

    /// Builds the page the way the handler would, so the template is
    /// really exercised without standing up a world.
    struct Page;

    impl Page {
        fn chrome() -> HashMap<String, String> {
            Self::map(&[
                ("news", "News"),
                ("overview", "Overview"),
                ("transfers", "Transfers"),
                ("awards", "Awards"),
                ("site_name", "Open Football"),
            ])
        }

        fn press() -> HashMap<String, String> {
            Self::map(&[
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
                    "newspaper_no_league_issues",
                    "Nothing has been printed about this division yet.",
                ),
            ])
        }

        fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect()
        }

        /// One story as the composer hands it over: plain type either
        /// side of the proper names the page underscores. `club` is the
        /// side credited in the copy — on a division's page that is a
        /// different club in every story, which is the whole point.
        fn story(kicker: &str, before: &str, player: &str, after: &str, club: &str) -> StoryView {
            let slug = player.to_lowercase().replace(' ', "-");

            let mut spans = vec![Span::text(before.to_string())];
            spans.push(Span::link(
                player.to_string(),
                "players",
                format!("7-{}", slug),
            ));
            spans.push(Span::text(after.to_string()));

            StoryView {
                kicker: kicker.to_string(),
                headline: Prose { spans },
                body: Prose {
                    spans: vec![
                        Span::text("Nobody in the division bettered it: 9 goals for ".to_string()),
                        Span::link(
                            club.to_string(),
                            "teams",
                            club.to_lowercase().replace(' ', "-"),
                        ),
                        Span::text(".".to_string()),
                    ],
                },
                date: "1 April 2026".to_string(),
                player_slug: format!("7-{}", slug),
                player_name: player.to_string(),
                is_quote: false,
                drop_cap: true,
                marked: false,
                subject_marked: false,
            }
        }

        /// A month's edition: the charts lead, the field behind them,
        /// and the rumour mill under the fold. No results column — a
        /// division played every fixture in it, so there is no list of
        /// "our" results to rule off.
        fn issue(number: u32) -> IssueView {
            IssueView {
                number,
                masthead: "The Serie A Chronicle".to_string(),
                paper_slug: String::new(),
                date: "1 April 2026".to_string(),
                mood_label: "Steady".to_string(),
                mood_slug: "steady",
                mood_stamped: false,
                lead: Some(Self::story(
                    "Scoring charts",
                    "",
                    "Dusan Vlahovic",
                    " tops the scoring charts",
                    "Juventus",
                )),
                secondary: vec![
                    Self::story(
                        "Scoring charts",
                        "",
                        "Lautaro Martinez",
                        " keeps the pressure on",
                        "Inter",
                    ),
                    Self::story("Market", "Chelsea watching ", "Rafael Leao", "", "Milan"),
                ],
                // Two whole rows across the measure. A division's paper
                // sets everything it prints as a full story — there is
                // no band under the fold, so `run` is where the rest of
                // the month goes.
                run: vec![
                    Self::story(
                        "Scoring charts",
                        "",
                        "Victor Osimhen",
                        " up among the scorers",
                        "Napoli",
                    ),
                    Self::story(
                        "Scoring charts",
                        "",
                        "Paulo Dybala",
                        " up among the scorers",
                        "Roma",
                    ),
                    Self::story(
                        "Scoring charts",
                        "",
                        "Ademola Lookman",
                        " up among the scorers",
                        "Atalanta",
                    ),
                    Self::story(
                        "Market",
                        "Scouts sent to watch ",
                        "Nicolo Barella",
                        "",
                        "Inter",
                    ),
                    Self::story(
                        "Market",
                        "",
                        "Federico Chiesa",
                        " linked with a move away",
                        "Juventus",
                    ),
                    Self::story(
                        "Market",
                        "Napoli turn down a bid for ",
                        "Kvicha",
                        "",
                        "Napoli",
                    ),
                ],
                briefs: Vec::new(),
                results: Vec::new(),
                portrait: Some(PortraitView {
                    player_id: 7,
                    player_slug: "7-dusan-vlahovic".to_string(),
                    player_name: "Dusan Vlahovic".to_string(),
                    player_generated: true,
                    caption: "Dusan Vlahovic tops the scoring charts".to_string(),
                    marked: false,
                }),
            }
        }

        fn template(issues: Vec<IssueView>) -> LeagueNewspaperTemplate {
            LeagueNewspaperTemplate {
                css_version: "test",
                computer_name: "test",
                cpu_brand: "test",
                cores_count: 1,
                title: "Serie A".to_string(),
                masthead: "The Serie A Chronicle".to_string(),
                sub_title_prefix: String::new(),
                sub_title_suffix: String::new(),
                sub_title: "Italy".to_string(),
                sub_title_link: "/en/countries/italy".to_string(),
                sub_title_country_code: "it".to_string(),
                header_color: "#1e272d".to_string(),
                foreground_color: "#ffffff".to_string(),
                menu_sections: Vec::new(),
                i18n: I18n::for_test(Self::chrome()),
                news: NewsI18n::for_test(Self::press()),
                lang: "en".to_string(),
                league_slug: "italian-serie-a".to_string(),
                newspaper_count: issues.len(),
                issues,
            }
        }
    }

    /// The division's page is the club paper's sheet, unaltered: same
    /// nameplate, same folio, same columns, same photograph.
    #[test]
    fn a_divisions_page_is_set_on_the_club_papers_sheet() {
        let html = Page::template(vec![Page::issue(9)]).render().unwrap();

        assert!(html.contains("np-sheet"));
        assert!(html.contains("The Serie A Chronicle"));
        assert!(html.contains("np-splits"));
        assert!(html.contains("np-run"));
        assert!(
            html.contains("/api/players/7/face.svg"),
            "the photograph block is the club page's, so the face comes from the same route"
        );
    }

    /// No band of one-liners under the fold, and no fold either: a
    /// division's monthly sets every line it prints as a full story.
    #[test]
    fn a_divisions_page_runs_no_briefs_band() {
        let html = Page::template(vec![Page::issue(9)]).render().unwrap();

        assert!(
            !html.contains("np-briefs"),
            "the briefs band reached the page"
        );
        assert!(!html.contains("np-band"), "the fold reached the page");
        assert!(
            !html.contains("In brief"),
            "the band's heading reached the page"
        );
    }

    /// The two things this paper is for, and nothing else: the scoring
    /// charts and the transfer mill, each under its own kicker.
    #[test]
    fn the_page_carries_the_charts_and_the_mill() {
        let html = Page::template(vec![Page::issue(9)]).render().unwrap();

        assert!(html.contains("Scoring charts"));
        assert!(html.contains("Market"));
        assert!(html.contains("Dusan Vlahovic"));
        assert!(html.contains("Rafael Leao"));
    }

    /// Every story on a division's page belongs to a different club, so
    /// the copy has to lead to each of them — not to the paper.
    #[test]
    fn each_story_credits_its_own_club() {
        let html = Page::template(vec![Page::issue(9)]).render().unwrap();

        for club in ["juventus", "inter", "milan", "napoli", "roma"] {
            assert!(
                html.contains(&format!("/en/teams/{}", club)),
                "{} is named in the copy but leads nowhere",
                club
            );
        }
    }

    /// The badge counts editions — what the reader scrolls past — and
    /// the tab renders as the active one.
    #[test]
    fn the_tabbar_badge_counts_the_months_on_the_shelf() {
        let html = Page::template(vec![Page::issue(9), Page::issue(8)])
            .render()
            .unwrap();

        let printed = html.matches("np-sheet").count();

        assert_eq!(printed, 2);
        assert!(html.contains(&format!("<span class=\"fm-tab-badge\">{}</span>", printed)));
        assert!(html.contains("/en/leagues/italian-serie-a/newspaper"));
        assert!(
            html.contains("np-archive-rule"),
            "back issues are ruled off"
        );
    }

    /// A division whose presses have not run yet gets the empty state,
    /// and no badge — a grey zero reads as a bug.
    #[test]
    fn a_division_with_no_editions_gets_the_empty_state() {
        let html = Page::template(Vec::new()).render().unwrap();

        assert!(html.contains("Nothing has been printed about this division yet."));
        assert!(!html.contains("np-sheet"));
        assert!(!html.contains("fm-tab-badge"));
    }

    /// Writes a self-contained copy of the division's press page so it
    /// can be looked at in a browser without starting the server — the
    /// twin of `newspaper_preview` on the club side:
    ///
    /// ```text
    /// NEWSPAPER_PREVIEW_DIR=<dir> cargo test -p web --lib league_newspaper_preview -- --ignored
    /// ```
    #[test]
    #[ignore]
    fn league_newspaper_preview() {
        let Ok(dir) = std::env::var("NEWSPAPER_PREVIEW_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).expect("preview dir");

        let page = Page::template(vec![Page::issue(9), Page::issue(8)])
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

        std::fs::write(
            std::path::Path::new(&dir).join("league-newspaper.html"),
            html,
        )
        .expect("write preview");
    }
}

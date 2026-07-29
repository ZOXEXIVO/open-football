pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::slug::{PlayerPage, resolve_player_page};
use crate::player::decisions::PlayerDecisionsCounter;
use crate::player::events::PlayerEventsCounter;
use crate::teams::newspaper::{IssueView, Paper, PaperFor, PressDesk, PressFocus, PressTeam};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use chrono::NaiveDate;
use core::club::news::NewspaperIssue;
use core::{Player, PlayerStatusType, SimulatorData, Team};
use serde::Deserialize;

/// Which papers write about a player.
///
/// The side he plays for covers his football week to week. A loanee
/// needs a second one: he sits on the borrowing club's roster, so the
/// club that still owns him reports him only through its loan column —
/// and "how is our lad getting on" is the first thing that readership
/// turns to.
///
/// And then every club he has ever played for. A transfer is reported
/// twice — the buying club runs a signing, the selling club runs a
/// departure — and until this walked his career history the second
/// half of his own move was missing from his own page. The same goes
/// for a loan that ends, a recall, a release: the club he is leaving
/// is the one with something to say about it. Old papers keep five
/// editions, so a club he left three seasons ago quietly contributes
/// nothing; the ones he has just walked out of contribute exactly the
/// week a reader came looking for.
///
/// This is also why a free agent or a retired player is no longer an
/// empty page: he belongs to no newsroom now, but the paper that
/// reported his release or his retirement is still on the shelf.
pub struct PlayerPress;

impl PlayerPress {
    pub fn papers<'a>(data: &'a SimulatorData, player: &Player) -> Vec<&'a Team> {
        let mut papers: Vec<&'a Team> = Vec::new();

        if let Some((_, team)) = data.player_with_team(player.id) {
            Self::add(&mut papers, PressTeam::covering(data, team));
        }

        // The club that still holds his contract. Its flagship is where
        // the loan column files, and it is a different newsroom from the
        // one he trains at.
        Self::add(
            &mut papers,
            player
                .parent_club_id()
                .and_then(|club_id| data.club(club_id))
                .and_then(|club| club.teams.main()),
        );

        // Everywhere he has been. The order this arrives in does not
        // matter — every paper on the list is read in full and the
        // editions are then stacked by publication date — but it is
        // stable, so two renders of the same career produce the same
        // page. Squads with no paper of their own (youth, reserves)
        // resolve to their club's flagship and dedupe against it.
        for slug in player.statistics_history.career_team_slugs() {
            let former = data
                .indexes
                .as_ref()
                .and_then(|indexes| indexes.slug_indexes.get_team_by_slug(slug))
                .and_then(|team_id| data.team(team_id));

            Self::add(
                &mut papers,
                former.and_then(|team| PressTeam::covering(data, team)),
            );
        }

        papers
    }

    /// Adds a paper the page has not got already. Every source here can
    /// resolve to the same newsroom — a loanee's parent club is often
    /// also the last club in his history — and an edition printed twice
    /// on one page reads as a bug in the presses.
    fn add<'a>(papers: &mut Vec<&'a Team>, paper: Option<&'a Team>) {
        if let Some(paper) = paper {
            if !papers.iter().any(|held| held.id == paper.id) {
                papers.push(paper);
            }
        }
    }

    /// Whether an edition said anything about this player. An edition
    /// that never names him belongs on his club's page, not on his.
    pub fn mentions(issue: &NewspaperIssue, player_id: u32) -> bool {
        issue
            .stories
            .iter()
            .any(|story| story.player_id == player_id)
    }
}

/// How many papers the tab has to show, for the badge on the tabbar.
///
/// Editions that mention him — exactly what the page prints, so the
/// number on the tab is the number of mastheads a reader scrolls past.
/// The same contract as the team tab's badge, but the shelf here is
/// already filtered to the papers that had something to say about this
/// player, which is what makes it worth a number at all.
pub struct PlayerNewsCounter;

impl PlayerNewsCounter {
    pub fn count(data: &SimulatorData, player: &Player) -> usize {
        PlayerPress::papers(data, player)
            .iter()
            .flat_map(|paper| paper.newsroom.issues.iter())
            .filter(|issue| PlayerPress::mentions(issue, player.id))
            .count()
    }
}

#[derive(Deserialize)]
pub struct PlayerNewspaperRequest {
    pub lang: String,
    pub player_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/newspaper/index.html")]
#[allow(dead_code)]
pub struct PlayerNewspaperTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: I18n,
    pub lang: String,
    pub active_tab: &'static str,
    pub player_id: u32,
    pub player_slug: String,
    pub club_id: u32,
    pub is_on_loan: bool,
    pub is_injured: bool,
    pub is_unhappy: bool,
    pub is_force_match_selection: bool,
    pub is_on_watchlist: bool,
    pub events_count: usize,
    pub decisions_count: usize,
    pub interested_clubs_count: usize,
    pub awards_count: u32,
    pub news_count: usize,
    /// The bureau's slip at the head of the shelf — "Marked for Lloyd
    /// Kelly". One line, above the first edition, because the blue on
    /// the sheets below it is the only thing on the page that was not
    /// printed and a reader is owed the reason for it once.
    pub marked_note: String,
    /// Newest edition first, across every paper that covers him. Whole
    /// editions, set exactly as the club's own page sets them.
    pub issues: Vec<IssueView>,
}

pub async fn player_newspaper_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerNewspaperRequest>,
) -> ApiResult<Response> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let (player, team_opt, canonical) = match resolve_player_page(
        simulator_data,
        &route_params.player_slug,
        &route_params.lang,
        "/newspaper",
    )? {
        PlayerPage::Found {
            player,
            team,
            canonical_slug,
        } => (player, team, canonical_slug),
        PlayerPage::Redirect(r) => return Ok(r),
    };

    let (neighbor_teams, country_leagues) = if let Some(team) = team_opt {
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?
    } else {
        (Vec::new(), Vec::new())
    };
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let league_refs: Vec<(&str, &str)> = country_leagues
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();

    let title = format!(
        "{} {}",
        player.full_name.display_first_name(),
        player.full_name.display_last_name()
    );

    let issues = PressCuttings::collect(simulator_data, player, &i18n);
    let marked_note = i18n.t("newspaper_marked_for").replace("{name}", &title);

    Ok(PlayerNewspaperTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        title,
        sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
        sub_title_suffix: String::new(),
        sub_title: team_opt.map(|t| t.name.clone()).unwrap_or_else(|| {
            if player.is_retired() {
                i18n.t("retired").to_string()
            } else {
                i18n.t("free_agent").to_string()
            }
        }),
        sub_title_link: team_opt
            .map(|t| format!("/{}/teams/{}", &route_params.lang, &t.slug))
            .unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: team_opt
            .and_then(|t| {
                simulator_data
                    .club(t.club_id)
                    .map(|c| c.colors.background.clone())
            })
            .unwrap_or_else(|| "#808080".to_string()),
        foreground_color: team_opt
            .and_then(|t| {
                simulator_data
                    .club(t.club_id)
                    .map(|c| c.colors.foreground.clone())
            })
            .unwrap_or_else(|| "#ffffff".to_string()),
        menu_sections: if let Some(team) = team_opt {
            let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
            let current_path = format!("/{}/teams/{}", &route_params.lang, &team.slug);
            let mp = views::MenuParams {
                i18n: &i18n,
                lang: &route_params.lang,
                current_path: &current_path,
                country_name: cn,
                country_slug: cs,
            };
            views::team_menu(&mp, &neighbor_refs, &league_refs)
        } else {
            Vec::new()
        },
        i18n,
        lang: route_params.lang.clone(),
        active_tab: "newspaper",
        player_id: player.id,
        player_slug: canonical,
        club_id: team_opt.map(|t| t.club_id).unwrap_or(0),
        is_on_loan: player.is_on_loan(),
        is_injured: player.player_attributes.is_injured,
        is_unhappy: player.statuses.get().contains(&PlayerStatusType::Unh),
        is_force_match_selection: player.is_force_match_selection,
        is_on_watchlist: simulator_data.watchlist.contains(&player.id),
        events_count: PlayerEventsCounter::count(player),
        decisions_count: PlayerDecisionsCounter::count_recent(player, simulator_data.date.date()),
        interested_clubs_count: simulator_data.clubs_interested_in_player(player.id).len(),
        awards_count: player.awards_count.total(),
        news_count: PlayerNewsCounter::count(simulator_data, player),
        marked_note,
        issues,
    }
    .into_response())
}

/// Picks the editions a player's page shows.
///
/// Nothing is written or rewritten here — the presses run on Monday in
/// the simulator, and an old paper says what it said on the day. This
/// only chooses which of the editions already on the shelf belong on
/// this page: the ones that mention him. Each is then set through the
/// club page's own typesetter, whole and uncut — same lead, same
/// secondaries, same briefs, same results column and portrait — because
/// what a reader wants from a player's press page is the paper he was
/// in, not a summary of the paragraphs with his name in them.
struct PressCuttings;

impl PressCuttings {
    fn collect(data: &SimulatorData, player: &Player, i18n: &I18n) -> Vec<IssueView> {
        // The reader these editions are being set for. A clippings
        // bureau went through the paper with a blue pencil before it
        // posted it on, and this is the name it was paid to look for.
        let slug = player.slug();
        let focus = PressFocus::player(player.id, &slug);

        // Carries the real publication date alongside the view: the
        // rendered one is a localised string ("2 March 2026"), and
        // ordering a shelf of papers alphabetically would put April
        // before January in half the languages we ship.
        let mut dated: Vec<(NaiveDate, IssueView)> = PlayerPress::papers(data, player)
            .into_iter()
            .flat_map(|paper| {
                let masthead =
                    PressDesk::masthead(paper.newsroom.masthead_key(), &paper.name, i18n);
                let credit = PaperFor::Own(Paper {
                    name: &paper.name,
                    slug: &paper.slug,
                    club_id: paper.club_id,
                    team_id: paper.id,
                });

                paper
                    .newsroom
                    .issues
                    .iter()
                    .filter(|issue| PlayerPress::mentions(issue, player.id))
                    .map(move |issue| {
                        let view = IssueView {
                            // The nameplate opens the paper it belongs
                            // to. On a loanee's page that is how his two
                            // newsrooms are told apart, and it is the
                            // way through to the club's full run.
                            paper_slug: paper.slug.clone(),
                            ..PressDesk::issue(data, credit, issue, &masthead, i18n, focus)
                        };

                        (issue.date, view)
                    })
            })
            .collect();

        // Newest first across both papers. Ties break on the edition
        // number and then the masthead, so a loanee's two papers
        // published on the same Monday always stack the same way round.
        dated.sort_by(|(left_date, left), (right_date, right)| {
            right_date
                .cmp(left_date)
                .then_with(|| right.number.cmp(&left.number))
                .then_with(|| left.masthead.cmp(&right.masthead))
        });

        dated.into_iter().map(|(_, view)| view).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{PlayerNewspaperTemplate, PlayerPress};
    use crate::I18n;
    use crate::teams::newspaper::{IssueView, PortraitView, Prose, ResultView, Span, StoryView};
    use askama::Template;
    use chrono::NaiveDate;
    use core::club::news::{NewsStory, NewsStoryKind, NewspaperIssue, PressMood};
    use std::collections::HashMap;

    /// Builds the page the way the handler would, so the template is
    /// really exercised — a player's press page can be checked without
    /// standing up a world.
    struct Page;

    impl Page {
        fn copy() -> HashMap<String, String> {
            [
                ("newspaper", "Newspaper"),
                ("newspaper_edition", "Edition"),
                ("newspaper_results", "Results"),
                ("newspaper_in_brief", "In brief"),
                ("newspaper_cup_tie", "Cup"),
                ("newspaper_back_issues", "Back issues"),
                ("newspaper_marked_for", "Marked for {name}"),
                (
                    "newspaper_our_correspondent",
                    "From our football correspondent",
                ),
                ("newspaper_presses_idle", "The presses are idle"),
                (
                    "newspaper_no_player_news",
                    "No paper covers this player yet.",
                ),
                ("site_name", "Open Football"),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
        }

        /// One story, with the headline already split into the plain
        /// type and the name the page underscores — the shape the
        /// composer hands the templates.
        ///
        /// `name` is the man in the headline. Diego Mora is the reader
        /// whose page this is, so the typesetter would have marked him;
        /// anybody else is one more name in his club's paper and is
        /// left as the plain cross-reference it was printed as.
        fn story(before: &str, name: &str, after: &str) -> StoryView {
            let his = name == "Diego Mora";
            let slug = match name {
                "" => String::new(),
                "Diego Mora" => "17-diego-mora".to_string(),
                _ => "42-nico-reyes".to_string(),
            };

            let mut spans = Vec::new();
            if !before.is_empty() {
                spans.push(Span::text(before.to_string()));
            }
            if !name.is_empty() {
                let mut span = Span::link(name.to_string(), "players", slug.clone());
                span.marked = his;
                spans.push(span);
            }
            if !after.is_empty() {
                spans.push(Span::text(after.to_string()));
            }

            StoryView {
                kicker: "Squad".to_string(),
                headline: Prose { spans },
                body: Prose {
                    spans: vec![Span::text("Body copy.".to_string())],
                },
                date: "2 March 2026".to_string(),
                player_slug: slug,
                player_name: name.to_string(),
                is_quote: false,
                drop_cap: true,
                marked: his,
                subject_marked: his,
            }
        }

        /// A whole edition, with the furniture the club's page sets:
        /// a lead, the pair below it, the run across the measure and a
        /// briefs band.
        fn issue(number: u32) -> IssueView {
            IssueView {
                number,
                masthead: "The Córdoba Chronicle".to_string(),
                paper_slug: "cordoba".to_string(),
                date: "2 March 2026".to_string(),
                mood_label: "Steady".to_string(),
                mood_slug: "steady",
                mood_stamped: false,
                lead: Some(Self::story("Three goals for ", "Diego Mora", "")),
                secondary: vec![Self::story("Córdoba board back the manager", "", "")],
                run: vec![Self::story("Scouts sent to watch ", "Diego Mora", "")],
                briefs: vec![Self::story("", "Nico Reyes", " out for 42 days")],
                results: Vec::new(),
                portrait: None,
            }
        }

        /// The same edition with the furniture the rail carries — the
        /// face and the ruled results column — so the preview shows the
        /// ring as well as the cuttings.
        fn full_issue(number: u32) -> IssueView {
            IssueView {
                secondary: vec![
                    Self::story("Córdoba board back the manager", "", ""),
                    Self::story("Terms agreed with ", "Diego Mora", ""),
                ],
                run: vec![
                    Self::story("Scouts sent to watch ", "Diego Mora", ""),
                    Self::story("Academy keeper called up", "", ""),
                    Self::story("", "Nico Reyes", " out for six weeks"),
                ],
                results: vec![
                    ResultView {
                        date: "28.02".to_string(),
                        opponent_name: "Sevilla".to_string(),
                        opponent_slug: "sevilla".to_string(),
                        score: "3-1".to_string(),
                        outcome: "w",
                        is_cup: false,
                        match_id: "2026-02-28_301_58".to_string(),
                    },
                    ResultView {
                        date: "22.02".to_string(),
                        opponent_name: "Real Betis".to_string(),
                        opponent_slug: "real-betis".to_string(),
                        score: "1-1".to_string(),
                        outcome: "d",
                        is_cup: true,
                        match_id: String::new(),
                    },
                ],
                portrait: Some(PortraitView {
                    player_id: 17,
                    player_slug: "17-diego-mora".to_string(),
                    player_name: "Diego Mora".to_string(),
                    player_generated: true,
                    caption: "Three goals for Diego Mora".to_string(),
                    marked: true,
                }),
                ..Self::issue(number)
            }
        }

        fn template(issues: Vec<IssueView>) -> PlayerNewspaperTemplate {
            PlayerNewspaperTemplate {
                css_version: "test",
                computer_name: "test",
                cpu_brand: "test",
                cores_count: 1,
                title: "Diego Mora".to_string(),
                sub_title_prefix: "ST".to_string(),
                sub_title_suffix: String::new(),
                sub_title: "Córdoba".to_string(),
                sub_title_link: "/en/teams/cordoba".to_string(),
                sub_title_country_code: String::new(),
                header_color: "#1e272d".to_string(),
                foreground_color: "#ffffff".to_string(),
                menu_sections: Vec::new(),
                i18n: I18n::for_test(Self::copy()),
                lang: "en".to_string(),
                active_tab: "newspaper",
                player_id: 17,
                player_slug: "17-diego-mora".to_string(),
                club_id: 1,
                is_on_loan: false,
                is_injured: false,
                is_unhappy: false,
                is_force_match_selection: false,
                is_on_watchlist: false,
                events_count: 0,
                decisions_count: 0,
                interested_clubs_count: 0,
                awards_count: 0,
                news_count: issues.len(),
                marked_note: "Marked for Diego Mora".to_string(),
                issues,
            }
        }
    }

    /// The whole edition reaches the player's page — lead, secondaries
    /// and briefs. The point of the tab is the paper he was in, so a
    /// page that kept only the paragraphs naming him would be the wrong
    /// page.
    #[test]
    fn a_players_page_prints_the_edition_uncut() {
        let html = Page::template(vec![Page::issue(12)]).render().unwrap();

        assert!(html.contains("The Córdoba Chronicle"));
        assert!(html.contains("Three goals for <a class=\"np-story-link np-mark\""));
        assert!(
            html.contains("Córdoba board back the manager"),
            "a story about somebody else still belongs in the edition he was in"
        );
        assert!(html.contains("np-briefs"));
    }

    /// The tab is his, so the badge counts what the page shows and the
    /// tab renders as active.
    #[test]
    fn the_tabbar_badge_counts_the_papers_on_the_page() {
        let html = Page::template(vec![Page::issue(12), Page::issue(11)])
            .render()
            .unwrap();

        let printed = html.matches("np-sheet").count();

        assert_eq!(printed, 2);
        assert!(html.contains(&format!("<span class=\"fm-tab-badge\">{}</span>", printed)));
        assert!(html.contains("/en/players/17-diego-mora/newspaper"));
    }

    /// The nameplate is the way through to the club's full run — and on
    /// a loanee's page it is how his two newsrooms are told apart. It
    /// must never be set as a story link, because that class draws a
    /// rule under the type and a masthead with a line under it reads as
    /// a printing fault.
    #[test]
    fn the_nameplate_opens_the_clubs_own_paper_without_an_underline() {
        let html = Page::template(vec![Page::issue(12)]).render().unwrap();

        assert!(html.contains(
            "<a class=\"np-masthead-link\" href=\"/en/teams/cordoba/newspaper\">\
             The Córdoba Chronicle</a>"
        ));
        assert!(
            !html.contains("np-story-link\" href=\"/en/teams/cordoba/newspaper\""),
            "the nameplate must not be set as an underlined story link"
        );
    }

    /// Nothing written about him yet is an empty shelf, not a broken
    /// page — and no badge, because a grey zero reads as a bug.
    #[test]
    fn a_player_nobody_has_written_about_gets_the_empty_state() {
        let html = Page::template(Vec::new()).render().unwrap();

        assert!(html.contains("np-empty"));
        assert!(html.contains("No paper covers this player yet."));
        assert!(!html.contains("np-sheet"));
        assert!(!html.contains("fm-tab-badge"));
    }

    /// The passages about him are marked, and nothing else on the sheet
    /// is touched. This is the whole contract of the page: a reader
    /// looking for himself in his club's week should find the paragraphs
    /// without reading the paper twice — and the paper he is reading has
    /// to stay a paper, so the club's other news keeps every bit of the
    /// weight it was printed with.
    #[test]
    fn his_passages_are_marked_and_the_rest_of_the_paper_is_not() {
        let html = Page::template(vec![Page::issue(12)]).render().unwrap();

        assert!(
            html.contains("<section class=\"np-lead np-cut\">"),
            "a lead about him is not marked as a cutting"
        );
        assert!(
            html.contains("<section class=\"np-split np-cut\">"),
            "a story about him further down the page is not marked"
        );
        assert!(
            html.contains("<section class=\"np-split\">"),
            "the club's own news must stay unmarked — the page marks his \
             passages, it does not put the rest of the week in the shade"
        );
        assert!(
            html.contains("<li class=\"np-brief\">"),
            "a brief about somebody else is not his and takes no mark"
        );
    }

    /// The mark is made on his name, not on every name. A brief about a
    /// team-mate's injury names a player and links to him like any other
    /// cross-reference — marking that one would tell the reader he is
    /// looking at himself when he is not.
    #[test]
    fn another_players_name_keeps_the_plain_printed_rule() {
        let html = Page::template(vec![Page::issue(12)]).render().unwrap();

        assert!(
            html.contains("<a class=\"np-brief-link\" href=\"/en/players/42-nico-reyes\""),
            "a team-mate's name was marked as though it were his"
        );
    }

    /// The blue is the one thing on this page that did not come off the
    /// press, so the page says so once, at the top, and never again.
    #[test]
    fn the_bureaus_slip_names_the_reader_once() {
        let html = Page::template(vec![Page::issue(12), Page::issue(11)])
            .render()
            .unwrap();

        assert_eq!(html.matches("np-slip").count(), 1);
        assert!(html.contains("Marked for Diego Mora"));
    }

    /// An empty shelf carries no slip: there is nothing marked, so a
    /// line explaining the marks would be explaining nothing.
    #[test]
    fn an_empty_shelf_carries_no_slip() {
        let html = Page::template(Vec::new()).render().unwrap();

        assert!(!html.contains("np-slip"));
    }

    /// Writes a self-contained copy of a player's press page so the
    /// marking can be looked at in a browser without starting the
    /// server — the twin of `newspaper_preview` on the club side:
    ///
    /// ```text
    /// NEWSPAPER_PREVIEW_DIR=<dir> cargo test -p web --lib player_newspaper_preview -- --ignored
    /// ```
    #[test]
    #[ignore]
    fn player_newspaper_preview() {
        let Ok(dir) = std::env::var("NEWSPAPER_PREVIEW_DIR") else {
            return;
        };
        std::fs::create_dir_all(&dir).expect("preview dir");

        let page = Page::template(vec![Page::full_issue(14), Page::issue(13)])
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
            std::path::Path::new(&dir).join("player-newspaper.html"),
            html,
        )
        .expect("write preview");
    }

    /// The filter that makes this page a player's page: an edition that
    /// never named him is his club's business, not his.
    #[test]
    fn only_editions_that_name_him_are_kept() {
        let day = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
        let edition = |stories: Vec<NewsStory>| NewspaperIssue {
            number: 1,
            date: day,
            mood: PressMood::Steady,
            stories,
            results: Vec::new(),
        };

        let about_him = edition(vec![
            NewsStory::new(NewsStoryKind::HatTrick, day).about(17),
            NewsStory::new(NewsStoryKind::LeagueWin, day),
        ]);
        let about_the_club = edition(vec![
            NewsStory::new(NewsStoryKind::LeagueWin, day),
            NewsStory::new(NewsStoryKind::HatTrick, day).about(99),
        ]);

        assert!(PlayerPress::mentions(&about_him, 17));
        assert!(!PlayerPress::mentions(&about_the_club, 17));
    }
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &I18n,
) -> Result<(Vec<(String, String)>, Vec<(String, String)>), ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let teams = views::neighbor_teams(club, i18n);

    let mut country_leagues: Vec<(u32, String, String)> = data
        .country_by_club(club_id)
        .map(|country| {
            country
                .leagues
                .leagues
                .iter()
                .filter(|l| !l.friendly)
                .map(|l| (l.id, l.name.clone(), l.slug.clone()))
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

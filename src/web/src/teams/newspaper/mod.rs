pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::slug::player_history_slug;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n, NewsI18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::{Datelike, NaiveDate};
use core::club::news::{IssueResult, NewsDesk, NewsStory, NewspaperIssue};
use core::shared::fullname::FullName;
use core::utils::FormattingUtils;
use core::{SimulatorData, Team, TeamType};
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
    /// Press copy, scoped apart from `i18n` — the edition partial resolves
    /// its chrome against this.
    pub news: NewsI18n,
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
    /// Printed items waiting on the newspaper tab, for the tabbar badge.
    pub newspaper_count: usize,
    /// Newest edition first. Empty until the club's first press run.
    pub issues: Vec<IssueView>,
}

/// One printed edition, fully typeset for the page.
pub struct IssueView {
    pub number: u32,
    pub masthead: String,
    /// Slug of the side whose paper this is, when the nameplate should
    /// open that paper. Empty on the club's own page — a masthead that
    /// links to the page you are already reading is a dead end — and
    /// filled on a player's page, where the edition belongs to a
    /// newsroom the reader may want to go to.
    pub paper_slug: String,
    pub date: String,
    pub mood_label: String,
    pub mood_slug: &'static str,
    pub mood_stamped: bool,
    pub lead: Option<StoryView>,
    /// The two pieces set under the lead, in the block the photograph
    /// and the results column sit beside.
    pub secondary: Vec<StoryView>,
    /// The rest of the week's news, set full-width across the measure
    /// below that block. Same treatment as [`Self::secondary`] —
    /// headline, dateline and body — only in narrower columns.
    pub run: Vec<StoryView>,
    /// The week's small change, set as single lines under the fold.
    pub briefs: Vec<StoryView>,
    pub results: Vec<ResultView>,
    pub portrait: Option<PortraitView>,
}

/// One edition's stories, shared out between the runs of type the page
/// sets them in. See [`PressDesk::tiers`].
struct Tiers {
    lead: Option<StoryView>,
    secondary: Vec<StoryView>,
    run: Vec<StoryView>,
    briefs: Vec<StoryView>,
}

pub struct StoryView {
    pub kicker: String,
    pub headline: Prose,
    pub body: Prose,
    pub date: String,
    pub player_slug: String,
    pub player_name: String,
    /// The body is somebody talking rather than the correspondent
    /// writing, and the page sets it as a pull-quote.
    pub is_quote: bool,
    /// The reader is named somewhere in this story — as its subject or
    /// halfway down somebody else's paragraph — and the page sets it as
    /// a marked cutting. Always false on a club's own page.
    pub marked: bool,
    /// The reader is the story's *subject*: the man [`Self::player_slug`]
    /// points at. Held apart from [`Self::marked`] because a headline
    /// that names nobody is underscored whole, and that one link is only
    /// his when the story is his — a story that mentions him in the body
    /// while leading on somebody else must not mark the other man's name.
    pub subject_marked: bool,
    /// Whether this body may take the lead paragraph's drop cap.
    ///
    /// A drop cap is a letter. Set at 54px, a digit or a currency sign
    /// is not a flourish but a printing fault — "4-1 against Sevilla"
    /// opens with a giant `4`, and `{fee}` copy opens with a giant `$`.
    /// Plenty of real copy legitimately starts on a scoreline or a
    /// tally, in every one of the nine bundles, so the page decides
    /// from the composed text rather than asking every writer in every
    /// language to avoid it.
    pub drop_cap: bool,
}

/// One run of composed copy, kept as the alternating plain type and
/// proper names it was written as instead of as one flat string.
///
/// A name in a newspaper is a cross-reference, and the rule belongs
/// under the NAME. Underscoring a whole headline says "this line opens
/// an article", and there is no article to open — the only things behind
/// a story here are the people and the clubs it names. Holding the copy
/// as spans is what lets every one of them be underscored and nothing
/// else, in a headline and in the middle of a paragraph alike, without
/// the composer ever emitting markup a translator could break.
pub struct Prose {
    pub spans: Vec<Span>,
}

impl Prose {
    /// The copy as one string, for the places that set it as plain type
    /// — the photograph's caption above all.
    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    /// Whether anything in this run is a cross-reference. A headline
    /// that names nobody is the one case the page still underscores
    /// whole: see [`StoryView::player_slug`].
    pub fn has_links(&self) -> bool {
        self.spans.iter().any(Span::is_link)
    }
}

/// A stretch of one run of copy: either plain type, or a name the page
/// sets as a link. `section` and `slug` are held apart from the URL so
/// the language the reader is on — which the composer has no business
/// knowing — is put in by the template.
pub struct Span {
    pub text: String,
    /// `players`, `teams` or `staff`; empty for plain type.
    pub section: &'static str,
    /// Empty for plain type, and for a name whose subject could not be
    /// resolved — a dead link is worse than plain type.
    pub slug: String,
    /// This name is the reader's own, and the page marks it by hand.
    /// Always false on a club's page, where nobody in particular is
    /// reading. See [`PressFocus`].
    pub marked: bool,
}

impl Span {
    pub(crate) fn text(text: String) -> Self {
        Span {
            text,
            section: "",
            slug: String::new(),
            marked: false,
        }
    }

    pub(crate) fn link(text: String, section: &'static str, slug: String) -> Self {
        if slug.is_empty() {
            return Self::text(text);
        }
        Span {
            text,
            section,
            slug,
            marked: false,
        }
    }

    pub fn is_link(&self) -> bool {
        !self.slug.is_empty()
    }
}

/// The reader a page is set for.
///
/// A club's paper is set for nobody in particular: every name in it is
/// one cross-reference among many, and none of them is worth more ink
/// than the rest. A player's page is the other thing entirely — it is
/// the envelope a clippings bureau posted on, the same editions with
/// his passages gone through by hand before they were sent.
///
/// The focus is carried into the typesetting rather than searched for
/// afterwards, because the composer is the only place that knows which
/// name in a sentence is whose: by the time the copy is markup, "Kelly"
/// is a word.
#[derive(Clone, Copy)]
pub(crate) struct PressFocus<'a> {
    player_id: u32,
    slug: &'a str,
}

impl<'a> PressFocus<'a> {
    /// A paper read by nobody in particular. Nothing is marked, which
    /// is what a club's own front page should look like.
    pub(crate) fn none() -> Self {
        PressFocus {
            player_id: 0,
            slug: "",
        }
    }

    pub(crate) fn player(player_id: u32, slug: &'a str) -> Self {
        PressFocus { player_id, slug }
    }

    /// Whether a story is *about* the reader — its subject, rather than
    /// somebody it happens to name in passing.
    fn is_subject(&self, player_id: u32) -> bool {
        self.player_id != 0 && self.player_id == player_id
    }

    /// Goes through one run of copy and marks every mention of the
    /// reader, reporting whether it found any.
    ///
    /// Matched on the slug rather than on the text: a bureau marking a
    /// paper knows which Kelly it was paid to look for, and two players
    /// can share a surname inside one sentence.
    fn mark(&self, prose: &mut Prose) -> bool {
        if self.slug.is_empty() {
            return false;
        }

        let mut found = false;
        for span in &mut prose.spans {
            if span.section == "players" && span.slug == self.slug {
                span.marked = true;
                found = true;
            }
        }

        found
    }
}

/// The photograph on the front page, with the caption set under it.
pub struct PortraitView {
    pub player_id: u32,
    pub player_slug: String,
    pub player_name: String,
    pub player_generated: bool,
    pub caption: String,
    /// The face on the front page is the reader's own, and the page
    /// rings it the way a subscriber ringed himself. See [`PressFocus`].
    pub marked: bool,
}

pub struct ResultView {
    /// Compact day-and-month, the form a ruled results column uses.
    pub date: String,
    pub opponent_name: String,
    pub opponent_slug: String,
    pub score: String,
    /// `w` / `d` / `l` — drives the result mark in the ruled column.
    pub outcome: &'static str,
    /// Cup ties carry a competition mark; a 0-1 in a knockout round is
    /// a different result from a 0-1 on a league Saturday.
    pub is_cup: bool,
    /// The match record behind the scoreline (`{date}_{home}_{away}`).
    /// Empty when the paper cannot place the fixture, in which case the
    /// score is set as plain type rather than as a dead link.
    pub match_id: String,
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
    let news = state.news_i18n.for_lang(&route_params.lang);

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

    let issues = PressTeam::covering(simulator_data, team)
        .map(|paper| PressDesk::typeset(simulator_data, paper, &i18n, &news))
        .unwrap_or_default();

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
        news,
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
        newspaper_count: NewspaperCounter::count(simulator_data, team),
        issues,
    })
}

/// Which side's paper a given team's tab shows.
///
/// Every side competing under its own brand — the first team, a B team,
/// a "{Club} 2" side in a real lower division — runs its own presses, so
/// it shows its own editions. A squad with no brand of its own (Reserve,
/// U18..U23) never goes to print and is read about in the first team's
/// paper, which is what its tab shows.
pub struct PressTeam;

impl PressTeam {
    pub fn covering<'a>(data: &'a SimulatorData, team: &'a Team) -> Option<&'a Team> {
        if team.team_type.is_own_team() {
            return Some(team);
        }

        data.club(team.club_id)?.teams.main()
    }
}

/// How many newspapers the tab has to show, for the badge on the
/// tabbar. The badge counts papers because that is what the page is —
/// a stack of editions, each with its own nameplate, folio and date —
/// so the number a reader sees on the tab is the number of mastheads
/// they will scroll past. Bounded by `TeamNewsroom::MAX_ISSUES`.
pub struct NewspaperCounter;

impl NewspaperCounter {
    pub fn count(data: &SimulatorData, team: &Team) -> usize {
        PressTeam::covering(data, team)
            .map(|paper| paper.newsroom.issues.len())
            .unwrap_or(0)
    }
}

/// Turns a side's stored editions into pages a reader can look at:
/// names resolved, money formatted, prose translated.
///
/// Everywhere the copy says `{club}` it is filled with the TEAM's name,
/// not the club's — the placeholder predates papers belonging to teams,
/// and renaming it in nine locale bundles would buy nothing. It is what
/// makes a reserve side's paper read "Spartak Moscow 2 tear Rubin 2
/// apart" instead of crediting the first team with a result it never
/// played.
///
/// `pub(crate)` because the player page prints the same editions, set
/// the same way, filtered to the ones that mention him — a second
/// typesetter would drift from this one the first time the copy
/// changed.
pub(crate) struct PressDesk;

impl PressDesk {
    /// Pieces set beside the photograph and the results column, in the
    /// block the lead opens. Two is what fits next to a rail.
    const SECONDARY_SLOTS: usize = 2;
    /// Most single lines the band under the fold will carry. A paper's
    /// "in brief" column is a handful of one-liners, not a spill tray
    /// for everything that did not lead — so only the week's smallest
    /// items are set that way and the rest stay upstairs as full
    /// stories. The band gives some of them back when the run needs
    /// them; see [`Self::RUN_COLUMNS`].
    const BRIEF_SLOTS: usize = 4;
    /// Columns the run below the front block is set in, and therefore
    /// the number its length has to come out to. Must match the
    /// `.np-run` grid in the stylesheet: this is the count that decides
    /// whether the last row of the page is a full one.
    const RUN_COLUMNS: usize = 3;

    fn typeset(data: &SimulatorData, team: &Team, i18n: &I18n, news: &NewsI18n) -> Vec<IssueView> {
        let masthead = Self::masthead(team.newsroom.masthead_key(), &team.name, news);
        let paper = PaperFor::Own(Paper {
            name: &team.name,
            slug: &team.slug,
            club_id: team.club_id,
            team_id: team.id,
        });

        team.newsroom
            .issues
            .iter()
            // A club's paper is read by the whole town, so nothing on it
            // is marked for anybody.
            .map(|issue| {
                Self::issue(data, paper, issue, &masthead, i18n, news, PressFocus::none())
            })
            .collect()
    }

    /// A nameplate, with whoever the paper belongs to named in it. The
    /// masthead patterns are shared: a division's monthly is "The Serie
    /// A Chronicle" by exactly the same rule that makes a club's weekly
    /// "The Rubin Kazan Chronicle".
    pub(crate) fn masthead(key: &str, name: &str, news: &NewsI18n) -> String {
        news.t(key).replace("{club}", name)
    }

    pub(crate) fn issue<'a>(
        data: &'a SimulatorData,
        paper: PaperFor<'a>,
        issue: &NewspaperIssue,
        masthead: &str,
        i18n: &I18n,
        news: &NewsI18n,
        focus: PressFocus<'_>,
    ) -> IssueView {
        let stories = issue
            .stories
            .iter()
            .map(|story| Self::story(data, paper.for_story(data, story), story, i18n, news, focus))
            .collect::<Vec<_>>();

        let Tiers {
            lead,
            secondary,
            run,
            briefs,
        } = Self::tiers(stories, paper.runs_a_briefs_band());

        let portrait = Self::portrait(data, paper, issue, news, focus);

        // The results column belongs to one side's own paper, so the
        // paper's team is half of every match id in it. A division's
        // monthly carries no results panel; its 0 falls through to a
        // plain, unlinked scoreline if one ever appears.
        let own_team_id = match paper {
            PaperFor::Own(own) => own.team_id,
            PaperFor::Division(_) => 0,
        };

        IssueView {
            number: issue.number,
            masthead: masthead.to_string(),
            // The club's own page never links its own nameplate; the
            // player page fills this in after typesetting.
            paper_slug: String::new(),
            date: i18n.format_date(issue.date),
            mood_label: news.t(issue.mood.i18n_key()).to_string(),
            mood_slug: issue.mood.slug(),
            mood_stamped: issue.mood.is_stamped(),
            lead,
            secondary,
            run,
            briefs,
            results: issue
                .results
                .iter()
                .map(|result| Self::result(data, result, own_team_id))
                .collect(),
            portrait,
        }
    }

    /// Shares one edition's stories out between the page's four runs of
    /// type.
    ///
    /// The stories arrive in the order the editor ranked them, so the
    /// tail of the list is the week's small change — and four of those,
    /// no more, is all the band under the fold takes. Everything above
    /// the tail is set as a full story with its own headline, dateline
    /// and body, so a busy week reads as a page of news rather than as
    /// one article followed by a list of nine.
    ///
    /// The two slots below the lead are held open even on a quiet week:
    /// a paper with three stories in it sets all three properly rather
    /// than demoting two of them to one-liners.
    ///
    /// The run is then rounded **up** to whole rows, and the band pays
    /// for it. A page that ends on one story sitting alone against two
    /// empty columns looks like the printer ran out of copy, so when
    /// the run is short of filling its last row it takes the one or two
    /// it needs off the top of the briefs. That is the only reason the
    /// band ever holds fewer than four: it is never padded, only raided.
    ///
    /// A paper with no band under the fold shares its stories out by
    /// [`Self::tiers_unbanded`] instead.
    fn tiers(stories: Vec<StoryView>, band: bool) -> Tiers {
        if !band {
            return Self::tiers_unbanded(stories);
        }

        let mut stories = stories.into_iter();
        let lead = stories.next();

        let mut rest: Vec<StoryView> = stories.collect();
        let beside_the_rail = Self::SECONDARY_SLOTS.min(rest.len());
        let below = rest.len() - beside_the_rail;

        // Whatever will not fit under the fold has to be set as news, so
        // that count is the floor the run is rounded up from. Rounding
        // up rather than down is what keeps the last row full: the
        // shortfall is taken out of the band, never left on the page.
        let in_the_run = below
            .saturating_sub(Self::BRIEF_SLOTS)
            .div_ceil(Self::RUN_COLUMNS)
            .saturating_mul(Self::RUN_COLUMNS)
            .min(below);

        let briefs = rest.split_off(beside_the_rail + in_the_run);
        let run = rest.split_off(beside_the_rail);

        Tiers {
            lead,
            secondary: rest,
            run,
            briefs,
        }
    }

    /// The share-out for a paper that runs no briefs band — the
    /// divisions' monthly (user, July 2026: "do not show briefs for
    /// Leagues newspapers").
    ///
    /// A band of one-liners is a local weekly's small change: the
    /// fine, the reserve-team result, the thing that is worth a
    /// sentence to the people who already follow the club. A division's
    /// monthly has no small change — every line on it is either a place
    /// on the scoring chart or somebody's future, and there is nothing
    /// on that page a reader would want as a one-liner.
    ///
    /// With the band gone it can no longer pay for the run's last row,
    /// so the run is rounded **down** instead and the tail that cannot
    /// fill a row is left out. The stories arrive in the editor's rank
    /// order, so what goes is always the least newsworthy one or two —
    /// which is what "the page is this size" means on a real paper.
    /// Never more than `RUN_COLUMNS - 1` are dropped, and the common
    /// full edition (a five-place chart plus four from the mill) comes
    /// out at exactly 1 + 2 + 6 and loses nothing.
    fn tiers_unbanded(stories: Vec<StoryView>) -> Tiers {
        let mut stories = stories.into_iter();
        let lead = stories.next();

        let mut rest: Vec<StoryView> = stories.collect();
        let beside_the_rail = Self::SECONDARY_SLOTS.min(rest.len());
        let below = rest.len() - beside_the_rail;

        let in_the_run = (below / Self::RUN_COLUMNS) * Self::RUN_COLUMNS;

        rest.truncate(beside_the_rail + in_the_run);
        let run = rest.split_off(beside_the_rail);

        Tiers {
            lead,
            secondary: rest,
            run,
            briefs: Vec::new(),
        }
    }

    /// The first story on the page with a face behind it. A paper never
    /// runs a front page of pure text when it has a picture available.
    fn portrait<'a>(
        data: &'a SimulatorData,
        paper: PaperFor<'a>,
        issue: &NewspaperIssue,
        news: &NewsI18n,
        focus: PressFocus<'_>,
    ) -> Option<PortraitView> {
        let story = issue.stories.iter().find(|story| story.player_id != 0)?;
        let player = data.player(story.player_id)?;

        Some(PortraitView {
            player_id: player.id,
            player_slug: player.slug(),
            player_name: PlayerName::display(&player.full_name),
            player_generated: player.is_generated(),
            marked: focus.is_subject(player.id),
            caption: StoryComposer::headline(data, story, news, paper.for_story(data, story))
                .text(),
        })
    }

    /// One story, typeset. `paper` is the side whose edition it ran in —
    /// its name fills the `{club}` slot, so a story lifted onto another
    /// page still credits the paper that printed it.
    pub(crate) fn story(
        data: &SimulatorData,
        paper: Paper<'_>,
        story: &NewsStory,
        i18n: &I18n,
        news: &NewsI18n,
        focus: PressFocus<'_>,
    ) -> StoryView {
        let (player_name, player_slug) = PlayerName::resolve(data, story.player_id);
        let mut headline = StoryComposer::headline(data, story, news, paper);
        let mut body = StoryComposer::body(data, story, news, paper);

        // Both runs are gone through, and both are marked: a name in the
        // headline and the same name four lines into the paragraph are
        // the reader finding himself twice, not once. Written as two
        // statements rather than `||` on purpose — the short-circuit
        // would leave the body unmarked whenever the headline named him.
        let in_the_headline = focus.mark(&mut headline);
        let in_the_body = focus.mark(&mut body);
        let subject_marked = focus.is_subject(story.player_id);

        StoryView {
            kicker: news.t(story.kind.desk().i18n_key()).to_string(),
            marked: subject_marked || in_the_headline || in_the_body,
            subject_marked,
            headline,
            // Read off the composed text, after the placeholders are
            // filled: whether a body opens with a letter depends on the
            // scoreline and the club name of this particular story, not
            // on the template it came from.
            drop_cap: body.text().chars().next().is_some_and(char::is_alphabetic),
            body,
            date: i18n.format_date(story.date),
            player_slug,
            player_name,
            is_quote: story.kind.is_quote(),
        }
    }

    fn result(data: &SimulatorData, result: &IssueResult, own_team_id: u32) -> ResultView {
        let (opponent_name, opponent_slug) = data
            .team_data(result.opponent_team_id)
            .map(|team| (team.name.clone(), team.slug.clone()))
            .unwrap_or_else(|| (String::new(), String::new()));

        ResultView {
            date: result.date.format("%d.%m").to_string(),
            opponent_name,
            opponent_slug,
            score: MatchPage::score(result.goals_for as i32, result.goals_against as i32),
            outcome: if result.is_win() {
                "w"
            } else if result.is_draw() {
                "d"
            } else {
                "l"
            },
            is_cup: result.is_cup(),
            match_id: if own_team_id == 0 {
                String::new()
            } else {
                MatchPage::id(
                    result.date,
                    result.is_home,
                    own_team_id,
                    result.opponent_team_id,
                )
            },
        }
    }
}

/// How the paper writes a scoreline, and where it leads.
///
/// A score in newsprint is set solid — "2‑1" broken across a line end
/// reads as a typo — and it is the one figure a reader wants to open:
/// the match record behind it. The record's id is the schedule's own
/// (`{date}_{home}_{away}`), so rebuilding it needs the date, the two
/// sides, and which of them was at home.
struct MatchPage;

impl MatchPage {
    /// The scoreline as type: joined with a non-breaking hyphen so it
    /// can never wrap.
    fn score(goals_for: i32, goals_against: i32) -> String {
        format!("{}\u{2011}{}", goals_for, goals_against)
    }

    /// The match record's id, in the schedule's own format.
    fn id(date: NaiveDate, is_home: bool, team_id: u32, opponent_team_id: u32) -> String {
        let (home, away) = if is_home {
            (team_id, opponent_team_id)
        } else {
            (opponent_team_id, team_id)
        };
        format!("{}_{}_{}", date, home, away)
    }
}

/// The edition a story ran in, as the composer needs it.
///
/// `name` is the side the paper belongs to and fills the `{club}` slot;
/// `slug` is where that name points when the page sets it as a link;
/// `club_id` is the club behind it, which is where a `{manager}` is
/// looked for first; `team_id` is the side itself, which is half of a
/// match record's id — a report's scoreline links to the match through
/// it. Carried together because the four always travel together and a
/// story lifted onto a player's page has to keep crediting the newsroom
/// that printed it.
#[derive(Clone, Copy)]
pub(crate) struct Paper<'a> {
    pub name: &'a str,
    pub slug: &'a str,
    pub club_id: u32,
    pub team_id: u32,
}

/// Who the `{club}` in a piece of copy belongs to, for the edition being
/// set.
///
/// A club's paper is one voice: every story on the page is that club's
/// story, so the slot is the same name from the nameplate to the briefs.
/// A division's paper is not — its front page is one club's striker, its
/// second column is another club's player being watched by a third. The
/// same sentence has to name a different club in each of them, or the
/// copy reads "Serie A turn down Juventus for Vlahović".
///
/// So the paper is resolved per STORY rather than per edition. That is
/// the whole of the difference between the two publications as far as
/// the typesetter is concerned, which is why there is still only one
/// typesetter.
#[derive(Clone, Copy)]
pub(crate) enum PaperFor<'a> {
    /// One side's own paper.
    Own(Paper<'a>),
    /// A division's paper, carrying its own name for the rare story
    /// whose subject cannot be placed at a club.
    Division(&'a str),
}

impl<'a> PaperFor<'a> {
    /// Whether this paper runs a band of one-liners under the fold.
    ///
    /// A club weekly does: the fine, the reserve result, the thing
    /// worth a sentence to people who already follow the club. A
    /// division's monthly does not — every line on it is a place on the
    /// scoring chart or somebody's future, and neither is small change.
    /// See [`PressDesk::tiers_unbanded`].
    fn runs_a_briefs_band(self) -> bool {
        matches!(self, PaperFor::Own(_))
    }

    fn for_story(self, data: &'a SimulatorData, story: &NewsStory) -> Paper<'a> {
        match self {
            PaperFor::Own(paper) => paper,
            PaperFor::Division(division) => data
                .player_with_team(story.player_id)
                .map(|(_, team)| Paper {
                    name: &team.name,
                    slug: &team.slug,
                    club_id: team.club_id,
                    team_id: team.id,
                })
                // A retired or released subject has no club to credit.
                // The division's own name stands in, with no slug — an
                // empty slug degrades to plain type rather than to a
                // link pointing at a team page that does not exist.
                .unwrap_or(Paper {
                    name: division,
                    slug: "",
                    club_id: 0,
                    team_id: 0,
                }),
        }
    }
}

/// What a `{slot}` in a piece of copy turns into: a figure the page sets
/// as type, a proper name the page sets as a cross-reference, or a token
/// no desk knows about.
enum Slot {
    Text(String),
    Name {
        text: String,
        section: &'static str,
        /// Empty when the subject could not be resolved. The name is
        /// still printed — a story that says "the manager" is worse than
        /// one that says his name without linking it, and both beat a
        /// link to nowhere.
        slug: String,
    },
    Unknown,
}

/// Fills the blanks in a translated headline or body with the real
/// names, numbers and money of the story it belongs to.
struct StoryComposer;

impl StoryComposer {
    /// Highest variant suffix the composer probes for. Copy can grow to
    /// `_v6` for any stem without touching code.
    const MAX_VARIANTS: usize = 6;

    fn headline(
        data: &SimulatorData,
        story: &NewsStory,
        news: &NewsI18n,
        paper: Paper<'_>,
    ) -> Prose {
        Self::compose(
            news.t(&Self::phrasing_key("h", story, news)),
            data,
            story,
            news,
            paper,
        )
    }

    fn body(data: &SimulatorData, story: &NewsStory, news: &NewsI18n, paper: Paper<'_>) -> Prose {
        Self::compose(
            news.t(&Self::phrasing_key("b", story, news)),
            data,
            story,
            news,
            paper,
        )
    }

    /// The translation key for the phrasing this story gets.
    ///
    /// The kinds a paper prints every week (match reports above all)
    /// carry several headline/body pairs in the bundles — the base
    /// `news_h_<stem>` plus `news_h_<stem>_v2`, `_v3`, … — because the
    /// same sentence about a different Saturday is the loudest tell
    /// that nobody writes this page. The pick is a stable hash of the
    /// story's identity: the same story reads the same on every visit
    /// and in every back issue, while another story of the same kind —
    /// or the same theme in a different week — comes out phrased
    /// differently.
    ///
    /// The pick runs over the phrasings this particular story can
    /// FILL, not over everything the bundles carry: a match report
    /// that knows its scorer may land on the copy that names him,
    /// while the same report without one stays on the phrasings that
    /// only need a scoreline. See [`Self::eligible_phrasings`].
    ///
    /// Headline and body always use the same index, so a pair written
    /// to share a register is printed as the pair it was written as.
    /// The bundles ship variants in matched, gap-free pairs (enforced
    /// by `variant_copy_ships_in_matched_pairs`), which is what lets
    /// the count be read off the headline keys alone.
    fn phrasing_key(part: &str, story: &NewsStory, news: &NewsI18n) -> String {
        let stem = story.kind.key_stem();
        let eligible = Self::eligible_phrasings(story, news, stem);
        let index = eligible[Self::phrasing_index(story, eligible.len())];
        Self::phrasing_key_at(part, stem, index)
    }

    /// The key one phrasing lives under: the bare stem for the base
    /// pair, `_v2`, `_v3`, … for the variants.
    fn phrasing_key_at(part: &str, stem: &str, index: usize) -> String {
        if index == 0 {
            format!("news_{}_{}", part, stem)
        } else {
            format!("news_{}_{}_v{}", part, stem, index + 1)
        }
    }

    /// Which of a stem's phrasings this story can actually set.
    ///
    /// Copy that names `{player}` is only on the table when the story
    /// carries one. That is what lets situational copy exist at all:
    /// the desks attach a protagonist when the week had one, the
    /// writers write phrasings around him, and a story without one can
    /// never land on "an unnamed player was the difference" — the stub
    /// sentence this whole mechanism exists to prevent.
    fn eligible_phrasings(story: &NewsStory, news: &NewsI18n, stem: &str) -> Vec<usize> {
        let count = Self::phrasing_count(news, stem);
        let eligible: Vec<usize> = (0..count)
            .filter(|&index| {
                story.player_id != 0 || !Self::phrasing_names_a_player(news, stem, index)
            })
            .collect();

        if eligible.is_empty() {
            // Every phrasing wants a player the story does not carry —
            // broken copy, not a quiet week. The base pair goes out with
            // its fallback name, which at least shows what is missing.
            return vec![0];
        }
        eligible
    }

    fn phrasing_names_a_player(news: &NewsI18n, stem: &str, index: usize) -> bool {
        let headline = Self::phrasing_key_at("h", stem, index);
        let body = Self::phrasing_key_at("b", stem, index);
        news.t(&headline).contains("{player}") || news.t(&body).contains("{player}")
    }

    /// How many phrasings the bundles carry for a stem. A missing key
    /// comes back from `t()` as the key itself, which is the probe.
    fn phrasing_count(news: &NewsI18n, stem: &str) -> usize {
        let mut count = 1;
        for suffix in 2..=Self::MAX_VARIANTS {
            let key = format!("news_h_{}_v{}", stem, suffix);
            if news.t(&key) == key {
                break;
            }
            count = suffix;
        }
        count
    }

    /// Deterministic pick, folded from the fields that make a story
    /// itself: who it is about, who else was involved, its figures and
    /// its date. FNV-1a rather than the standard hasher so the pick
    /// never shifts under a toolchain upgrade — an old edition must
    /// keep saying what it said on the day.
    fn phrasing_index(story: &NewsStory, count: usize) -> usize {
        if count <= 1 {
            return 0;
        }

        let mut hash: u32 = 0x811c_9dc5;
        for value in [
            story.player_id,
            story.other_id,
            story.staff_id,
            story.a as u32,
            story.b as u32,
            story.date.num_days_from_ce() as u32,
        ] {
            for byte in value.to_le_bytes() {
                hash ^= u32::from(byte);
                hash = hash.wrapping_mul(0x0100_0193);
            }
        }

        (hash % count as u32) as usize
    }

    /// Walks a translated string once, filling each `{slot}` as it comes
    /// and keeping the proper names apart from the type around them.
    ///
    /// One pass rather than a stack of `replace` calls because the
    /// output is no longer one string: a name has to come out as its own
    /// span so the page can underscore it alone, and a chain of
    /// replacements would have lost track of where the names ended up
    /// the moment a club was called something that also reads as a word.
    /// An unrecognised slot is copied through verbatim — a literal
    /// `{foo}` on the page is a bug report, and swallowing it is not.
    fn compose(
        template: &str,
        data: &SimulatorData,
        story: &NewsStory,
        news: &NewsI18n,
        paper: Paper<'_>,
    ) -> Prose {
        let mut spans: Vec<Span> = Vec::new();
        let mut plain = String::new();
        let mut rest = template;

        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}').map(|at| open + at) else {
                break;
            };

            plain.push_str(&rest[..open]);

            match Self::slot(&rest[open + 1..close], data, story, news, paper) {
                Slot::Text(text) => plain.push_str(&text),
                Slot::Name {
                    text,
                    section,
                    slug,
                } => {
                    let span = Span::link(text, section, slug);
                    if span.is_link() {
                        if !plain.is_empty() {
                            spans.push(Span::text(std::mem::take(&mut plain)));
                        }
                        spans.push(span);
                    } else {
                        plain.push_str(&span.text);
                    }
                }
                Slot::Unknown => plain.push_str(&rest[open..=close]),
            }

            rest = &rest[close + 1..];
        }

        plain.push_str(rest);
        if !plain.is_empty() {
            spans.push(Span::text(plain));
        }

        Prose { spans }
    }

    /// What one `{slot}` in a piece of copy resolves to.
    fn slot(
        name: &str,
        data: &SimulatorData,
        story: &NewsStory,
        news: &NewsI18n,
        paper: Paper<'_>,
    ) -> Slot {
        match name {
            // The side whose paper this is. It is a name in the copy like
            // any other, so it leads where the name leads — the club's
            // own page, which is a different page from the one carrying
            // the paper.
            "club" => Slot::Name {
                text: paper.name.to_string(),
                section: "teams",
                slug: paper.slug.to_string(),
            },
            "player" => {
                let (name, slug) = PlayerName::resolve(data, story.player_id);
                if name.is_empty() {
                    return Slot::Text(news.t("newspaper_unnamed_player").to_string());
                }
                Slot::Name {
                    text: name,
                    section: "players",
                    slug,
                }
            }
            // The man in the dugout. A paper that writes "the manager"
            // where it could write his name is a paper nobody believes
            // was written by anybody, and a sacking is the one story a
            // town discusses by name for a decade.
            "manager" => {
                let name = ManagerName::resolve(data, paper.club_id, story.staff_id);
                if name.is_empty() {
                    return Slot::Text(news.t("newspaper_unnamed_manager").to_string());
                }
                Slot::Name {
                    text: name,
                    section: "staff",
                    slug: story.staff_id.to_string(),
                }
            }
            // `{opponent}` names the other team in a match report,
            // `{other}` the other club in a transfer. Both come from the
            // same slot.
            "opponent" | "other" => {
                let (name, slug) = Self::other_party(data, story);
                if name.is_empty() {
                    return Slot::Text(news.t("newspaper_another_club").to_string());
                }
                Slot::Name {
                    text: name,
                    section: "teams",
                    slug,
                }
            }
            // The scoreline: set solid on a non-breaking hyphen, and —
            // in a match report that knows its fixture — a link to the
            // match record itself. Any other desk quoting a score, and
            // any report whose fixture cannot be placed, keeps it as
            // plain type rather than minting a link to nowhere.
            "score" => {
                let text = MatchPage::score(story.a, story.b);
                if story.kind.desk() == NewsDesk::Match && story.other_id != 0 && paper.team_id != 0
                {
                    return Slot::Name {
                        text,
                        section: "match",
                        slug: MatchPage::id(story.date, story.home, paper.team_id, story.other_id),
                    };
                }
                Slot::Text(text)
            }
            "fee" => Slot::Text(format!(
                "${}",
                FormattingUtils::format_money(story.money as f64)
            )),
            "rating" => Slot::Text(format!("{:.2}", story.b as f32 / 100.0)),
            "n" => Slot::Text(story.a.to_string()),
            // `{pts}` and `{m}` are the same slot under two names: the
            // table stories were written around points long before the
            // squad and loan desks needed a plain second number, and
            // renaming the key in nine bundles would buy nothing.
            "pts" | "m" => Slot::Text(story.b.to_string()),
            _ => Slot::Unknown,
        }
    }

    /// Match reports name a team; every other desk names a club. Both
    /// arrive in the same slot, so the desk decides which lookup runs —
    /// keyed off the desk rather than a list of kinds, so a new story
    /// kind resolves its counterparty without anyone remembering to add
    /// it here.
    ///
    /// A club is read about through its flagship — the club itself has
    /// no page — so a club with no main side resolves to a name without
    /// a link rather than to a URL that goes nowhere.
    fn other_party(data: &SimulatorData, story: &NewsStory) -> (String, String) {
        if story.other_id == 0 {
            return (String::new(), String::new());
        }

        if story.kind.desk() == NewsDesk::Match {
            return data
                .team_data(story.other_id)
                .map(|team| (team.name.clone(), team.slug.clone()))
                .unwrap_or_default();
        }

        data.club(story.other_id)
            .map(|club| {
                (
                    club.name.clone(),
                    club.teams
                        .main()
                        .map(|team| team.slug.clone())
                        .unwrap_or_default(),
                )
            })
            .unwrap_or_default()
    }
}

/// How the paper names somebody from the dugout.
///
/// A manager is harder to find than a player: he is not in the player
/// index, and by the time the edition reporting his sacking is read he
/// has left the club it names. So the lookup walks the three places he
/// can actually be, cheapest first, and never falls back to a
/// brute-force sweep of the world — a front page renders a dozen
/// stories and a whole-world walk per story would cost more than the
/// rest of the page put together.
///
/// 1. The club whose paper this is: covers the appointment, the new
///    deal, the ultimatum, the caretaker — every story about the man
///    currently in the seat.
/// 2. The free-agent staff pool: covers the man who has just been
///    sacked, which is where sackings put him the same tick.
/// 3. The staff index: covers a poached manager and a rival club's
///    target. It is a straight map lookup, and the id is re-checked
///    against the team it points at, so a stale entry resolves to
///    nobody rather than to the wrong man.
struct ManagerName;

impl ManagerName {
    fn resolve(data: &SimulatorData, club_id: u32, staff_id: u32) -> String {
        if staff_id == 0 {
            return String::new();
        }

        if let Some(club) = data.club(club_id) {
            if let Some(staff) = club
                .teams
                .iter()
                .find_map(|team| team.staffs.find(staff_id))
            {
                return PlayerName::display(&staff.full_name);
            }
        }

        if let Some(staff) = data
            .free_agent_staff
            .iter()
            .find(|staff| staff.id == staff_id)
        {
            return PlayerName::display(&staff.full_name);
        }

        data.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_staff_location(staff_id))
            .and_then(|(_, _, club_id, team_id)| {
                data.club(club_id)
                    .and_then(|club| club.teams.find(team_id))
                    .and_then(|team| team.staffs.find(staff_id))
            })
            .map(|staff| PlayerName::display(&staff.full_name))
            .unwrap_or_default()
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
        ("en", include_bytes!("../../../assets/i18n/news/en.json")),
        ("de", include_bytes!("../../../assets/i18n/news/de.json")),
        ("es", include_bytes!("../../../assets/i18n/news/es.json")),
        ("fr", include_bytes!("../../../assets/i18n/news/fr.json")),
        ("ja", include_bytes!("../../../assets/i18n/news/ja.json")),
        ("pt", include_bytes!("../../../assets/i18n/news/pt.json")),
        ("ru", include_bytes!("../../../assets/i18n/news/ru.json")),
        ("tr", include_bytes!("../../../assets/i18n/news/tr.json")),
        ("zh", include_bytes!("../../../assets/i18n/news/zh.json")),
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

            for desk in NewsDesk::ALL {
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

            // Paper chrome only. The bare `newspaper` tab label belongs to
            // the page, not the press, and stays in the chrome bundle.
            for chrome in [
                "newspaper_edition",
                "newspaper_results",
                "newspaper_in_brief",
                "newspaper_back_issues",
                "newspaper_our_correspondent",
                "newspaper_presses_idle",
                "newspaper_no_issues",
                "newspaper_no_player_news",
                "newspaper_unnamed_player",
                "newspaper_unnamed_manager",
                "newspaper_another_club",
                "newspaper_cup_tie",
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

    /// Every headline/body pair a stem carries, base first, in the
    /// order the composer would index them. Walking this instead of the
    /// base key alone is what keeps every copy contract below binding
    /// on variant phrasings too.
    fn phrasings<'a>(
        bundle: &'a BTreeMap<String, String>,
        stem: &str,
    ) -> Vec<(String, &'a String, &'a String)> {
        let mut pairs = Vec::new();
        for suffix in 1.. {
            let (h_key, b_key) = if suffix == 1 {
                (format!("news_h_{}", stem), format!("news_b_{}", stem))
            } else {
                (
                    format!("news_h_{}_v{}", stem, suffix),
                    format!("news_b_{}_v{}", stem, suffix),
                )
            };
            let (Some(headline), Some(body)) = (bundle.get(&h_key), bundle.get(&b_key)) else {
                break;
            };
            pairs.push((h_key, headline, body));
        }
        pairs
    }

    /// A story flagged as a quote is set as a pull-quote on the page.
    /// Copy that forgets to put speech marks around it prints a
    /// blockquote nobody said, in every language at once — and a
    /// variant phrasing is just as capable of the mistake as the base.
    #[test]
    fn every_quoted_story_actually_quotes_somebody() {
        const OPENERS: [char; 6] = [
            '"', '\u{201c}', '\u{00ab}', '\u{201e}', '\u{300c}', '\u{2018}',
        ];

        for (lang, _) in BUNDLES {
            let bundle = PressKeys::bundle(lang);

            for kind in NewsStoryKind::ALL.iter().filter(|kind| kind.is_quote()) {
                for (key, _, body) in phrasings(&bundle, kind.key_stem()) {
                    assert!(
                        body.chars().any(|c| OPENERS.contains(&c)),
                        "{}.json key {} is set as a pull-quote but carries no speech marks: {}",
                        lang,
                        key,
                        body
                    );
                }
            }
        }
    }

    /// Variant copy ships in matched, gap-free pairs: every
    /// `news_h_<stem>_vN` has its `news_b_<stem>_vN`, numbering starts
    /// at `_v2` and runs without holes, every variant belongs to a real
    /// story kind, and nothing exceeds what the composer probes for.
    /// The composer counts headline keys and applies the same index to
    /// the body, so a bundle that breaks any of this either prints a
    /// raw key on the page or strands copy nobody can ever read.
    #[test]
    fn variant_copy_ships_in_matched_pairs() {
        let stems: Vec<&str> = NewsStoryKind::ALL
            .iter()
            .map(|kind| kind.key_stem())
            .collect();

        let split = |rest: &str| -> Option<(String, usize)> {
            let at = rest.rfind("_v")?;
            let digits = &rest[at + 2..];
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            Some((rest[..at].to_string(), digits.parse().ok()?))
        };

        for (lang, _) in BUNDLES {
            let bundle = PressKeys::bundle(lang);
            let mut found: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();

            for key in bundle.keys() {
                let (part, rest) = if let Some(rest) = key.strip_prefix("news_h_") {
                    ("h", rest)
                } else if let Some(rest) = key.strip_prefix("news_b_") {
                    ("b", rest)
                } else {
                    continue;
                };
                if let Some((stem, suffix)) = split(rest) {
                    found
                        .entry((stem, part.to_string()))
                        .or_default()
                        .push(suffix);
                }
            }

            let mut by_stem: BTreeMap<String, [Vec<usize>; 2]> = BTreeMap::new();
            for ((stem, part), mut suffixes) in found {
                suffixes.sort_unstable();
                let slot = if part == "h" { 0 } else { 1 };
                by_stem.entry(stem).or_default()[slot] = suffixes;
            }

            for (stem, [h, b]) in by_stem {
                assert!(
                    stems.contains(&stem.as_str()),
                    "{}.json carries variant copy for '{}', which is no story kind",
                    lang,
                    stem
                );
                assert_eq!(
                    h, b,
                    "{}.json: headline and body variants for '{}' do not pair up",
                    lang, stem
                );
                let expected: Vec<usize> = (2..=h.len() + 1).collect();
                assert_eq!(
                    h, expected,
                    "{}.json: variants for '{}' must run _v2, _v3, … without gaps",
                    lang, stem
                );
                assert!(
                    h.last().copied().unwrap_or(0) <= super::StoryComposer::MAX_VARIANTS,
                    "{}.json: '{}' exceeds the composer's probe ceiling",
                    lang,
                    stem
                );
            }
        }
    }

    /// The pick between phrasings is part of an edition's identity: the
    /// same story must read the same on every visit and in every back
    /// issue, while different stories of one kind spread across the
    /// available phrasings instead of all landing on the base copy.
    #[test]
    fn a_storys_phrasing_is_stable_and_stories_spread_across_phrasings() {
        use super::StoryComposer;
        use chrono::NaiveDate;
        use core::club::news::NewsStory;
        use std::collections::HashSet;

        let day = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
        let story = |player_id: u32, day: NaiveDate| {
            NewsStory::new(NewsStoryKind::LeagueWin, day).about(player_id)
        };

        for count in 2..=3 {
            let first = StoryComposer::phrasing_index(&story(7, day), count);
            assert_eq!(
                first,
                StoryComposer::phrasing_index(&story(7, day), count),
                "the same story must always pick the same phrasing"
            );
            assert!(first < count);

            let across_players: HashSet<usize> = (0..24)
                .map(|id| StoryComposer::phrasing_index(&story(id, day), count))
                .collect();
            assert!(
                across_players.len() > 1,
                "twenty-four different subjects all landed on one phrasing"
            );

            let across_weeks: HashSet<usize> = (0..8)
                .map(|week| {
                    StoryComposer::phrasing_index(
                        &story(7, day + chrono::Duration::days(7 * week)),
                        count,
                    )
                })
                .collect();
            assert!(
                across_weeks.len() > 1,
                "eight editions of the same theme all landed on one phrasing"
            );
        }
    }

    /// The editor refuses a story whose copy quotes a figure it does not
    /// carry — but it decides that from `quotes_a_rating` /
    /// `quotes_a_fee` in Rust, while the placeholders themselves live in
    /// the translation bundles. If the two drift, the guard silently
    /// stops guarding: copy gains a `{rating}` the editor never checks,
    /// and "a season average of 0.00" reaches a front page again.
    ///
    /// This is what pins them together. The placeholder-parity test in
    /// `i18n` already forces the other eight locales to match English,
    /// so checking English is enough to cover all nine.
    #[test]
    fn a_kind_that_quotes_a_figure_declares_it() {
        let bundle = PressKeys::bundle("en");

        for kind in NewsStoryKind::ALL {
            // Every phrasing is held to the declaration, not just the
            // base pair: a `_v2` that gains a `{rating}` the kind never
            // declared is exactly how the guard would quietly stop
            // guarding.
            for (key, headline, body) in phrasings(&bundle, kind.key_stem()) {
                let copy = format!("{}{}", headline, body);

                assert_eq!(
                    copy.contains("{rating}"),
                    kind.quotes_a_rating(),
                    "{:?} ({}): copy and NewsStoryKind::quotes_a_rating disagree about {{rating}}",
                    kind,
                    key
                );
                assert_eq!(
                    copy.contains("{fee}"),
                    kind.quotes_a_fee(),
                    "{:?} ({}): copy and NewsStoryKind::quotes_a_fee disagree about {{fee}}",
                    kind,
                    key
                );
            }
        }
    }

    /// The dugout's version of the same lockstep. A kind that carries a
    /// staff id but whose copy never names him wastes the one detail
    /// that makes a sacking read like news; a kind whose copy asks for a
    /// name it is never given prints "the manager" at every club in the
    /// world. Both are silent — only the pairing catches them.
    #[test]
    fn a_kind_that_names_a_manager_declares_it() {
        for (lang, _) in BUNDLES {
            let bundle = PressKeys::bundle(lang);

            for kind in NewsStoryKind::ALL {
                for (key, headline, body) in phrasings(&bundle, kind.key_stem()) {
                    assert_eq!(
                        format!("{}{}", headline, body).contains("{manager}"),
                        kind.names_a_manager(),
                        "{}.json key {}: copy and NewsStoryKind::names_a_manager disagree",
                        lang,
                        key
                    );
                }
            }
        }
    }

    /// The bug the market desk was rebuilt around, guarded from the copy
    /// side: no headline about a move without a guaranteed fee may quote
    /// one. The familiar-face kinds are here because their detectors
    /// fire on free moves as readily as on paid ones.
    #[test]
    fn only_a_real_sale_may_print_a_fee() {
        let feeless = [
            NewsStoryKind::FreeExit,
            NewsStoryKind::LoanExit,
            NewsStoryKind::FreeSigning,
            NewsStoryKind::LoanArrival,
            NewsStoryKind::LoanReturn,
            NewsStoryKind::HomecomingSigning,
            NewsStoryKind::LoanMadePermanent,
            NewsStoryKind::ProspectSigned,
            NewsStoryKind::VeteranArrives,
        ];

        for (lang, _) in BUNDLES {
            let bundle = PressKeys::bundle(lang);

            for kind in feeless {
                for (key, headline, body) in phrasings(&bundle, kind.key_stem()) {
                    assert!(
                        !headline.contains("{fee}") && !body.contains("{fee}"),
                        "{}.json key {} quotes a fee for a move that never had one",
                        lang,
                        key
                    );
                }
            }
        }
    }

    /// A masthead that forgot `{club}` would print the same title for
    /// every paper in the world. The slot is filled with the name of the
    /// side the edition belongs to — see `PressDesk` — so it is what
    /// tells "The Spartak Moscow 2 Chronicle" from the first team's.
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

    /// The i18n fixture the eligibility tests run against: one stem
    /// with two plain phrasings and one that names the scorer.
    fn press_with_player_variant() -> crate::NewsI18n {
        let mut map = std::collections::HashMap::new();
        map.insert(
            "news_h_league_win".to_string(),
            "{club} beat {opponent}".to_string(),
        );
        map.insert("news_b_league_win".to_string(), "{score}.".to_string());
        map.insert(
            "news_h_league_win_v2".to_string(),
            "{club} see off {opponent}".to_string(),
        );
        map.insert(
            "news_b_league_win_v2".to_string(),
            "A {score} win.".to_string(),
        );
        map.insert(
            "news_h_league_win_v3".to_string(),
            "{player} the difference".to_string(),
        );
        map.insert(
            "news_b_league_win_v3".to_string(),
            "{player} settled it, {score}.".to_string(),
        );
        crate::NewsI18n::for_test(map)
    }

    /// Copy that names the scorer may only be set when the story
    /// carries one. A report without its protagonist landing on a
    /// `{player}` phrasing would print "an unnamed player was the
    /// difference" — the stub sentence the mechanism exists to prevent.
    #[test]
    fn a_phrasing_that_names_a_player_is_never_picked_without_one() {
        use super::StoryComposer;
        use chrono::NaiveDate;
        use core::club::news::{NewsStory, NewsStoryKind};

        let news = press_with_player_variant();
        let day = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();

        for week in 0..40 {
            let story =
                NewsStory::new(NewsStoryKind::LeagueWin, day + chrono::Duration::days(week))
                    .against(week as u32 + 1);
            let key = StoryComposer::phrasing_key("h", &story, &news);
            assert!(
                !key.ends_with("_v3"),
                "a report with no scorer picked the phrasing that names him"
            );
        }
    }

    /// …while a report that knows its man spreads across the whole
    /// pool, scorer copy included — and each story keeps its phrasing.
    #[test]
    fn a_report_with_its_scorer_can_reach_the_copy_that_names_him() {
        use super::StoryComposer;
        use chrono::NaiveDate;
        use core::club::news::{NewsStory, NewsStoryKind};
        use std::collections::HashSet;

        let news = press_with_player_variant();
        let day = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();

        let keys: HashSet<String> = (1..40)
            .map(|id| {
                let story = NewsStory::new(NewsStoryKind::LeagueWin, day)
                    .about(id)
                    .against(7);
                let headline = StoryComposer::phrasing_key("h", &story, &news);
                assert_eq!(
                    headline,
                    StoryComposer::phrasing_key("h", &story, &news),
                    "the pick must be stable per story"
                );
                headline
            })
            .collect();

        assert!(
            keys.iter().any(|key| key.ends_with("_v3")),
            "forty scorers never reached the copy written for them: {:?}",
            keys
        );
        assert!(keys.len() > 1, "the pool collapsed to one phrasing");
    }

    /// Broken copy, not a crash: if every phrasing of a stem demands a
    /// player, a story without one falls back to the base pair rather
    /// than panicking on an empty pool.
    #[test]
    fn a_stem_whose_every_phrasing_needs_a_player_falls_back_to_base() {
        use super::StoryComposer;
        use chrono::NaiveDate;
        use core::club::news::{NewsStory, NewsStoryKind};

        let mut map = std::collections::HashMap::new();
        map.insert(
            "news_h_league_win".to_string(),
            "{player} wins it".to_string(),
        );
        map.insert(
            "news_b_league_win".to_string(),
            "{player} again.".to_string(),
        );
        let news = crate::NewsI18n::for_test(map);

        let day = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
        let story = NewsStory::new(NewsStoryKind::LeagueWin, day).against(7);

        assert_eq!(
            StoryComposer::phrasing_key("h", &story, &news),
            "news_h_league_win"
        );
    }

    /// A scoreline is set solid — the non-breaking hyphen is what keeps
    /// "2-1" off a line end — and its match id is the schedule's own:
    /// date first, then the home side, then the away side, whichever of
    /// them this paper covers.
    #[test]
    fn a_scoreline_is_set_solid_and_opens_its_match_record() {
        use super::MatchPage;
        use chrono::NaiveDate;

        let day = NaiveDate::from_ymd_opt(2026, 8, 8).unwrap();
        assert_eq!(
            MatchPage::id(day, true, 1301104, 1525),
            "2026-08-08_1301104_1525"
        );
        assert_eq!(
            MatchPage::id(day, false, 1525, 1301104),
            "2026-08-08_1301104_1525",
            "an away report puts the opponent first"
        );
        assert_eq!(MatchPage::score(2, 1), "2\u{2011}1");
    }

    /// Every match report must stay printable without its scorer — a
    /// stored result can arrive without details, and a 0-0 has nobody
    /// to name. Each match stem therefore keeps at least one phrasing
    /// in every locale that needs no `{player}` at all.
    #[test]
    fn a_match_report_can_always_print_without_its_scorer() {
        for (lang, _) in BUNDLES {
            let bundle = PressKeys::bundle(lang);

            for kind in NewsStoryKind::ALL
                .iter()
                .filter(|kind| kind.desk() == NewsDesk::Match)
            {
                let playerless =
                    phrasings(&bundle, kind.key_stem())
                        .iter()
                        .any(|(_, headline, body)| {
                            !headline.contains("{player}") && !body.contains("{player}")
                        });
                assert!(
                    playerless,
                    "{}.json: every phrasing of {} demands a player the report may not have",
                    lang,
                    kind.key_stem()
                );
            }
        }
    }
}

/// The arithmetic that decides which run of type a story is set in.
/// Exercised without a world behind it: the question is only how many
/// pieces each part of the page gets, at every edition size the editor
/// can produce.
#[cfg(test)]
mod tier_tests {
    use super::{PressDesk, Prose, Span, StoryView};
    use core::club::TeamNewsroom;

    fn stories(count: usize) -> Vec<StoryView> {
        (0..count)
            .map(|index| StoryView {
                kicker: "Squad".to_string(),
                headline: Prose {
                    spans: vec![Span::text(format!("Story {}", index))],
                },
                body: Prose {
                    spans: vec![Span::text("Body.".to_string())],
                },
                date: "2 March 2026".to_string(),
                player_slug: String::new(),
                player_name: String::new(),
                is_quote: false,
                drop_cap: true,
                marked: false,
                subject_marked: false,
            })
            .collect()
    }

    /// However full the edition, the band under the fold never takes
    /// more than its four lines — the surplus is set as news instead.
    #[test]
    fn the_briefs_band_never_grows_past_its_four_lines() {
        for count in 0..=TeamNewsroom::MAX_STORIES {
            let tiers = PressDesk::tiers(stories(count), true);

            assert!(
                tiers.briefs.len() <= PressDesk::BRIEF_SLOTS,
                "{} stories put {} lines under the fold",
                count,
                tiers.briefs.len()
            );

            let set = usize::from(tiers.lead.is_some())
                + tiers.secondary.len()
                + tiers.run.len()
                + tiers.briefs.len();
            assert_eq!(set, count, "an edition of {} lost or gained a story", count);
        }
    }

    /// A quiet week sets everything it has properly. Three stories is a
    /// lead and two pieces beside the rail, not a lead and two lines.
    #[test]
    fn a_thin_edition_keeps_its_stories_off_the_briefs_band() {
        for count in 0..=(PressDesk::SECONDARY_SLOTS + 1) {
            let tiers = PressDesk::tiers(stories(count), true);

            assert!(
                tiers.briefs.is_empty(),
                "{} stories reached the band",
                count
            );
            assert!(tiers.run.is_empty(), "{} stories reached the run", count);
        }
    }

    /// A full edition fills the page from the top: the two slots beside
    /// the rail, then the run across the measure, and only what is left
    /// over goes under the fold.
    #[test]
    fn a_full_edition_fills_the_page_before_the_fold() {
        let tiers = PressDesk::tiers(stories(TeamNewsroom::MAX_STORIES), true);

        assert!(tiers.lead.is_some());
        assert_eq!(tiers.secondary.len(), PressDesk::SECONDARY_SLOTS);
        assert_eq!(tiers.run.len(), 6);
        // Nine below the pair, four of which the band would have taken —
        // but five is a row and two thirds, so the run took a third one.
        assert_eq!(tiers.briefs.len(), 3);

        // The order the editor ranked them in survives the share-out:
        // the band under the fold holds the last of them and nothing else.
        assert_eq!(tiers.briefs[0].headline.text(), "Story 9");
        assert_eq!(tiers.briefs[2].headline.text(), "Story 11");
    }

    /// The page never ends on a part-built row. Whatever the edition
    /// size, the run comes out to a whole number of rows and the band
    /// under the fold is what pays for it.
    #[test]
    fn the_run_always_comes_out_to_whole_rows() {
        for count in 0..=TeamNewsroom::MAX_STORIES {
            let tiers = PressDesk::tiers(stories(count), true);

            assert_eq!(
                tiers.run.len() % PressDesk::RUN_COLUMNS,
                0,
                "an edition of {} left {} stories alone on the last row",
                count,
                tiers.run.len() % PressDesk::RUN_COLUMNS
            );
        }
    }

    /// A paper with no band sets nothing under the fold, at any size.
    /// This is the whole of what the division's monthly asked for.
    #[test]
    fn an_unbanded_paper_never_sets_a_brief() {
        for count in 0..=TeamNewsroom::MAX_STORIES {
            let tiers = PressDesk::tiers(stories(count), false);

            assert!(
                tiers.briefs.is_empty(),
                "an unbanded edition of {} put {} lines under the fold",
                count,
                tiers.briefs.len()
            );
        }
    }

    /// With no band to raid, the run rounds down instead of up — so the
    /// page still never ends on a part-built row.
    #[test]
    fn an_unbanded_run_still_comes_out_to_whole_rows() {
        for count in 0..=TeamNewsroom::MAX_STORIES {
            let tiers = PressDesk::tiers(stories(count), false);

            assert_eq!(
                tiers.run.len() % PressDesk::RUN_COLUMNS,
                0,
                "an unbanded edition of {} left {} stories alone on the last row",
                count,
                tiers.run.len() % PressDesk::RUN_COLUMNS
            );
        }
    }

    /// Rounding down costs the tail, and the cost is bounded: never
    /// more than a row short of two. The stories arrive ranked, so what
    /// goes is always the least newsworthy.
    #[test]
    fn an_unbanded_page_drops_only_the_tail_of_the_ranking() {
        for count in 0..=TeamNewsroom::MAX_STORIES {
            let tiers = PressDesk::tiers(stories(count), false);
            let set = usize::from(tiers.lead.is_some()) + tiers.secondary.len() + tiers.run.len();

            assert!(
                count - set < PressDesk::RUN_COLUMNS,
                "an unbanded edition of {} dropped {} stories",
                count,
                count - set
            );

            // Whatever survived is an unbroken prefix of the ranking.
            let printed: Vec<String> = tiers
                .lead
                .iter()
                .chain(tiers.secondary.iter())
                .chain(tiers.run.iter())
                .map(|story| story.headline.text())
                .collect();
            let expected: Vec<String> = (0..set).map(|index| format!("Story {}", index)).collect();
            assert_eq!(
                printed, expected,
                "an edition of {} reordered itself",
                count
            );
        }
    }

    /// The division's full monthly — a five-place scoring chart and the
    /// four the mill allows — comes out at exactly a lead, the pair
    /// beside the rail and two whole rows, losing nothing.
    #[test]
    fn a_full_league_monthly_fits_the_page_exactly() {
        let tiers = PressDesk::tiers(stories(9), false);

        assert!(tiers.lead.is_some());
        assert_eq!(tiers.secondary.len(), PressDesk::SECONDARY_SLOTS);
        assert_eq!(tiers.run.len(), 2 * PressDesk::RUN_COLUMNS);
        assert!(tiers.briefs.is_empty());
    }
}

#[cfg(test)]
mod render_tests {
    use super::{
        IssueView, PortraitView, Prose, ResultView, Span, StoryView, TeamNewspaperTemplate,
    };
    use crate::I18n;
    use askama::Template;
    use std::collections::HashMap;

    /// Fixture copy with the page's cross-references written into it.
    /// `{player}` and `{club}` come out as the link spans the composer
    /// emits, everything else as plain type — so the templates are
    /// exercised on the shape they really receive, without a world
    /// behind them.
    struct Set;

    impl Set {
        fn prose(text: &str, player: &str) -> Prose {
            let mut spans: Vec<Span> = Vec::new();
            let mut rest = text;

            while let Some(open) = rest.find('{') {
                let close = open
                    + rest[open..]
                        .find('}')
                        .expect("unclosed slot in fixture copy");
                if open > 0 {
                    spans.push(Span::text(rest[..open].to_string()));
                }
                spans.push(match &rest[open + 1..close] {
                    "player" => {
                        Span::link(player.to_string(), "players", "17-diego-mora".to_string())
                    }
                    "club" => Span::link("Córdoba".to_string(), "teams", "cordoba".to_string()),
                    other => panic!("fixture copy uses an unknown slot {{{}}}", other),
                });
                rest = &rest[close + 1..];
            }

            if !rest.is_empty() {
                spans.push(Span::text(rest.to_string()));
            }

            Prose { spans }
        }
    }

    /// Builds a page the way the handler would, so the template itself
    /// is exercised — the app can render a front page without anyone
    /// having to start the server to find out.
    struct Page;

    impl Page {
        /// Real English page chrome, so a preview screenshot shows the
        /// page a reader gets rather than a grid of raw keys.
        fn chrome() -> HashMap<String, String> {
            Self::map(&[("newspaper", "Newspaper"), ("site_name", "Open Football")])
        }

        /// The same, for the press scope the edition partial resolves
        /// against.
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
                    "newspaper_no_issues",
                    "Nothing has been printed about this club yet.",
                ),
                ("newspaper_cup_tie", "Cup"),
                ("news_desk_loan", "Loan watch"),
                ("news_desk_fans", "The terraces"),
            ])
        }

        fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect()
        }

        fn template(issues: Vec<IssueView>) -> TeamNewspaperTemplate {
            TeamNewspaperTemplate {
                css_version: "test",
                computer_name: "test",
                cpu_brand: "test",
                cores_count: 1,
                i18n: I18n::for_test(Self::chrome()),
                news: crate::NewsI18n::for_test(Self::press()),
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
                newspaper_count: issues.len(),
                issues,
            }
        }

        /// `player` is the man the story is about, empty for a story
        /// about the club. Headline copy that leaves `{player}` out of
        /// it while still naming a subject exercises the fall-back path
        /// on purpose — that is real copy, not an oversight.
        fn story(kicker: &str, headline: &str, body: &str, player: &str) -> StoryView {
            StoryView {
                kicker: kicker.to_string(),
                headline: Set::prose(headline, player),
                body: Set::prose(body, player),
                date: "2 March 2026".to_string(),
                player_slug: if player.is_empty() {
                    String::new()
                } else {
                    "17-diego-mora".to_string()
                },
                player_name: player.to_string(),
                is_quote: false,
                drop_cap: true,
                // Nobody in particular reads a club's own front page.
                marked: false,
                subject_marked: false,
            }
        }

        fn quote(kicker: &str, headline: &str, body: &str, player: &str) -> StoryView {
            StoryView {
                is_quote: true,
                ..Self::story(kicker, headline, body, player)
            }
        }

        fn full_issue() -> IssueView {
            IssueView {
                number: 12,
                masthead: "The Córdoba Chronicle".to_string(),
                paper_slug: String::new(),
                date: "2 March 2026".to_string(),
                mood_label: "Triumph".to_string(),
                mood_slug: "triumph",
                mood_stamped: true,
                lead: Some(Self::story(
                    "Match",
                    "{club} tear Sevilla apart",
                    "Rarely has this fixture seen a display like it: 4-1 against Sevilla, \
                     and it could have been more.",
                    "",
                )),
                secondary: vec![
                    Self::story(
                        "Squad",
                        "Three goals for {player}",
                        "The match ball belongs to {player}, whose 3 goals turned a \
                         difficult afternoon into a procession.",
                        "Diego Mora",
                    ),
                    Self::quote(
                        "Loan watch",
                        "{player} wants to come home",
                        "\u{201c}I did not go there to sit and watch. I want to come back \
                         and fight for my place at {club}.\u{201d}",
                        "Diego Mora",
                    ),
                ],
                run: vec![
                    Self::story(
                        "Match",
                        "{club} make it 4 in a row",
                        "Four wins on the run, and the table now reads better than it has \
                         at any point this season.",
                        "",
                    ),
                    Self::story(
                        "Market",
                        "{player} placed on the list",
                        "A month of training with the reserves ends the only way it was \
                         ever going to end.",
                        "Rubén Ortega",
                    ),
                    Self::story(
                        "Boardroom",
                        "{club} board back the manager",
                        "A public word of support and, more usefully, an understanding \
                         about what the next window will look like.",
                        "",
                    ),
                    Self::story(
                        "Squad",
                        "180 appearances for {player}",
                        "Eight seasons, one club, and a milestone the dressing room made \
                         rather more of than he did.",
                        "Andrés Vidal",
                    ),
                    Self::story(
                        "Loan watch",
                        "4 goals on loan for {player}",
                        "The reports coming back are good enough that his return is now a \
                         question of when rather than whether.",
                        "Pau Ferrer",
                    ),
                    // Real copy that never names its subject. There is
                    // nothing in the line to underscore, so this one — and
                    // only this one — keeps the whole-headline rule.
                    Self::story(
                        "Market",
                        "A new admirer in the stands",
                        "Nothing has been put in writing yet, which is not the same thing \
                         as nothing having been said.",
                        "Iván Salas",
                    ),
                ],
                briefs: vec![
                    Self::story("Squad", "{player} back in training", "", "Diego Mora"),
                    Self::story("Squad", "{player} out for 42 days", "", "Nico Reyes"),
                    Self::story("The terraces", "The crowd stays with them", "", ""),
                ],
                results: vec![
                    // A fixture the paper could not place keeps its
                    // scoreline as plain type — the other branch of the
                    // column's template.
                    ResultView {
                        date: "25.02".to_string(),
                        opponent_name: "Real Zaragoza".to_string(),
                        opponent_slug: "real-zaragoza".to_string(),
                        score: "1-1".to_string(),
                        outcome: "d",
                        is_cup: true,
                        match_id: String::new(),
                    },
                    ResultView {
                        date: "01.03".to_string(),
                        opponent_name: "Sevilla".to_string(),
                        opponent_slug: "sevilla".to_string(),
                        score: "4-1".to_string(),
                        outcome: "w",
                        is_cup: false,
                        match_id: "2026-03-01_301_58".to_string(),
                    },
                ],
                portrait: Some(PortraitView {
                    player_id: 17,
                    player_slug: "17-diego-mora".to_string(),
                    player_name: "Diego Mora".to_string(),
                    player_generated: true,
                    caption: "Three goals for Diego Mora".to_string(),
                    marked: false,
                }),
            }
        }

        /// A week the paper leads on a person rather than a result —
        /// the case that puts a player link under the big headline.
        fn young_star_issue() -> IssueView {
            IssueView {
                number: 10,
                masthead: "The Córdoba Chronicle".to_string(),
                paper_slug: String::new(),
                date: "16 February 2026".to_string(),
                mood_label: "Upbeat".to_string(),
                mood_slug: "upbeat",
                mood_stamped: false,
                lead: Some(Self::story(
                    "Squad",
                    "{player} is the real thing at 18",
                    "A season average of 7.34 at an age when most of his year group are \
                     still in the youth team. {club} know what they have, and so, by now, \
                     does everybody else.",
                    "Diego Mora",
                )),
                secondary: Vec::new(),
                run: Vec::new(),
                briefs: vec![
                    Self::story(
                        "Squad",
                        "{player} called up by his country",
                        "",
                        "Diego Mora",
                    ),
                    Self::story("Market", "Scouts sent to watch {player}", "", "Diego Mora"),
                ],
                results: Vec::new(),
                portrait: None,
            }
        }

        /// A quiet week: stories but no fixtures and nothing left over
        /// for the briefs column.
        fn bare_issue() -> IssueView {
            IssueView {
                number: 11,
                masthead: "The Córdoba Chronicle".to_string(),
                paper_slug: String::new(),
                date: "23 February 2026".to_string(),
                mood_label: "Steady".to_string(),
                mood_slug: "steady",
                mood_stamped: false,
                lead: Some(Self::story(
                    "Boardroom",
                    "{club} board back the manager",
                    "A public word of support from the board and, more usefully, an \
                     understanding about what the next window will look like.",
                    "",
                )),
                secondary: Vec::new(),
                run: Vec::new(),
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
        // The club's name is a cross-reference, so the headline arrives
        // in two pieces with the anchor between them.
        assert!(html.contains("cordoba\" title=\"Córdoba\">Córdoba</a> tear Sevilla apart"));
        assert!(html.contains("np-folio-mood-triumph np-stamp"));
        assert!(html.contains("/api/players/17/face.svg"));
        assert!(html.contains("np-result np-result-w"));
        assert!(html.contains("/en/teams/sevilla"));
        assert!(html.contains("np-brief-link"));
        assert!(html.contains("np-run"));
    }

    /// The band under the fold is a handful of one-liners, and the rest
    /// of the week is set as news. A busy edition used to put nine
    /// stories in that band and print three of them properly; the split
    /// runs the other way now.
    #[test]
    fn a_busy_edition_sets_its_news_and_briefs_only_the_small_change() {
        let html = Page::template(vec![Page::full_issue()]).render().unwrap();

        assert_eq!(
            html.matches("np-brief\"").count(),
            3,
            "the band under the fold takes what the run left it, and no more \
             than four either way"
        );
        // Two beside the rail, six across the measure below it — every
        // one of them with a body, which is what a brief does not get.
        assert_eq!(html.matches("np-split\"").count(), 8);
        assert!(html.contains("Four wins on the run"));
    }

    /// The tabbar badge counts newspapers — the number of mastheads a
    /// reader scrolls past on the tab it points at.
    #[test]
    fn the_tabbar_badge_counts_newspapers() {
        let html = Page::template(vec![Page::full_issue(), Page::young_star_issue()])
            .render()
            .unwrap();

        // Counted back off the rendered page rather than off the same
        // length the badge was built from, so the two can actually
        // disagree. One `np-sheet` per edition.
        let printed = html.matches("np-sheet").count();

        assert_eq!(printed, 2, "the fixture stopped printing two editions");
        assert!(
            html.contains(&format!("<span class=\"fm-tab-badge\">{}</span>", printed)),
            "badge does not match the {} editions actually set on the page",
            printed
        );
    }

    /// A masthead on the club's own page is not a link. Pointing the
    /// nameplate at the page the reader is already on is a dead end,
    /// and the empty `paper_slug` is what keeps it plain type.
    #[test]
    fn a_clubs_own_nameplate_is_not_a_link() {
        let html = Page::template(vec![Page::full_issue()]).render().unwrap();

        assert!(html.contains("The Córdoba Chronicle"));
        assert!(!html.contains("np-masthead-link"));
    }

    /// A club whose presses have never run shows no badge at all. A
    /// grey "0" next to a tab is worse than nothing — it reads as a
    /// broken counter rather than as an empty shelf.
    #[test]
    fn an_idle_newsroom_shows_no_badge() {
        let html = Page::template(Vec::new()).render().unwrap();

        assert!(!html.contains("fm-tab-badge"));
    }

    /// A drop cap is a letter. Copy that opens on a scoreline or a fee
    /// would set its first character at 54px — a giant "4" hanging off
    /// "4-1 against Sevilla" — and that reads as a printing fault. The
    /// page decides from the composed text, so no bundle in any of the
    /// nine languages has to avoid the construction.
    #[test]
    fn a_lead_that_opens_on_a_number_stands_the_drop_cap_down() {
        let numeric = IssueView {
            lead: Some(StoryView {
                drop_cap: false,
                ..Page::story(
                    "Match",
                    "{club} tear Sevilla apart",
                    "4-1 against Sevilla, and it could have been more.",
                    "",
                )
            }),
            ..Page::full_issue()
        };

        let html = Page::template(vec![numeric]).render().unwrap();
        assert!(html.contains("np-lead-body np-no-cap"));

        // …and a lead that opens on a letter still gets its flourish.
        let lettered = Page::template(vec![Page::full_issue()]).render().unwrap();
        assert!(!lettered.contains("np-no-cap"));
    }

    /// A loanee's interview is the one piece of copy on the page that is
    /// somebody talking, and it has to look like it.
    #[test]
    fn an_interview_is_set_as_a_pull_quote() {
        let html = Page::template(vec![Page::full_issue()]).render().unwrap();

        assert!(html.contains("np-split-body np-quote"));
        assert!(html.contains("I want to come back and fight for my place"));
    }

    /// The ruled column's scoreline opens the match record when the
    /// paper could place the fixture — and stays plain type when it
    /// could not, because a dead link is worse than a figure.
    #[test]
    fn the_results_columns_scoreline_opens_the_match_record() {
        let html = Page::template(vec![Page::full_issue()]).render().unwrap();

        assert!(html.contains(r#"href="/en/match/2026-03-01_301_58""#));
        assert!(html.contains(r#"<span class="np-result-score">1-1</span>"#));
    }

    /// A cup tie in the ruled column is marked, because "1-1" against a
    /// second-division side reads very differently in a knockout round.
    #[test]
    fn a_cup_tie_is_marked_in_the_results_column() {
        let html = Page::template(vec![Page::full_issue()]).render().unwrap();

        assert_eq!(html.matches("np-result-cup").count(), 1);
    }

    /// The rule goes under the name and stops there. A headline is a
    /// line of type with people and clubs in it, not a link to an
    /// article — there is no article — so "Three goals for Diego Mora"
    /// underscores three words, not five.
    #[test]
    fn a_headline_underscores_the_name_and_not_the_sentence() {
        let html = Page::template(vec![Page::full_issue()]).render().unwrap();

        assert!(html.contains(
            "Three goals for <a class=\"np-story-link\" href=\"/en/players/17-diego-mora\" \
             title=\"Diego Mora\">Diego Mora</a>"
        ));
        assert!(
            !html.contains(">Three goals for Diego Mora</a>"),
            "the whole headline was wrapped in the link again"
        );
    }

    /// …and the same rule applies to a club named anywhere in the copy,
    /// headline or body. A reader who meets a club in the middle of a
    /// paragraph can go and look at it.
    #[test]
    fn a_club_named_in_the_copy_leads_to_the_club() {
        let html = Page::template(vec![Page::full_issue()]).render().unwrap();

        // Three headlines name the club; the pull-quote names it in the
        // body, where the rule is the quieter one.
        assert_eq!(
            html.matches("<a class=\"np-story-link\" href=\"/en/teams/cordoba\"")
                .count(),
            3
        );
        assert!(html.contains(
            "fight for my place at <a class=\"np-prose-link\" href=\"/en/teams/cordoba\" \
             title=\"Córdoba\">Córdoba</a>."
        ));
    }

    /// A player named in the body is a cross-reference too, set with the
    /// quieter rule so a paragraph carrying three of them still reads as
    /// a paragraph.
    #[test]
    fn a_body_names_its_people_as_links() {
        let html = Page::template(vec![Page::full_issue()]).render().unwrap();

        assert!(html.contains(
            "The match ball belongs to <a class=\"np-prose-link\" \
             href=\"/en/players/17-diego-mora\" title=\"Diego Mora\">Diego Mora</a>, whose"
        ));
    }

    /// The one case that still underscores a whole headline: real copy
    /// that is about a player without ever naming him. There is nothing
    /// in the line to mark, and dropping the link would leave the story
    /// with no route to its subject at all.
    #[test]
    fn a_headline_that_names_nobody_keeps_the_whole_line_as_the_link() {
        let html = Page::template(vec![Page::full_issue()]).render().unwrap();

        assert!(html.contains(
            "<a class=\"np-story-link\" href=\"/en/players/17-diego-mora\" \
             title=\"Iván Salas\">A new admirer in the stands</a>"
        ));
    }

    /// A brief marks the name in its line, and a brief about nobody
    /// stays plain type — a dead link on a one-liner is worse than none.
    #[test]
    fn a_brief_underscores_the_name_in_its_line() {
        let html = Page::template(vec![Page::full_issue()]).render().unwrap();

        assert!(html.contains(
            "<a class=\"np-brief-link\" href=\"/en/players/17-diego-mora\" \
             title=\"Nico Reyes\">Nico Reyes</a> out for 42 days"
        ));
        assert!(html.contains("<span class=\"np-brief-text\">The crowd stays with them</span>"));
    }

    /// A headline with nobody behind it must stay plain text — wrapping
    /// "Córdoba board back the manager" in a dead player link would be
    /// worse than not linking at all.
    #[test]
    fn a_club_headline_carries_no_player_link() {
        let html = Page::template(vec![Page::bare_issue()]).render().unwrap();

        assert!(
            !html.contains("/en/players/"),
            "a story about the board was given a link to a player"
        );
        assert!(html.contains(">Córdoba</a> board back the manager"));
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
        assert!(html.contains(">Córdoba</a> board back the manager"));
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

        let page = Page::template(vec![
            Page::full_issue(),
            Page::young_star_issue(),
            Page::bare_issue(),
        ])
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

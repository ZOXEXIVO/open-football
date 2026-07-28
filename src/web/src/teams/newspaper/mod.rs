pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::slug::player_history_slug;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Datelike;
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
    pub headline: String,
    pub body: String,
    pub date: String,
    pub player_slug: String,
    pub player_name: String,
    /// The body is somebody talking rather than the correspondent
    /// writing, and the page sets it as a pull-quote.
    pub is_quote: bool,
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
    /// Cup ties carry a competition mark; a 0-1 in a knockout round is
    /// a different result from a 0-1 on a league Saturday.
    pub is_cup: bool,
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

    let issues = PressTeam::covering(simulator_data, team)
        .map(|paper| PressDesk::typeset(simulator_data, paper, &i18n))
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

    fn typeset(data: &SimulatorData, team: &Team, i18n: &I18n) -> Vec<IssueView> {
        let masthead = Self::masthead(team, i18n);

        team.newsroom
            .issues
            .iter()
            .map(|issue| Self::issue(data, team, issue, &masthead, i18n))
            .collect()
    }

    /// This side's nameplate, with the club named in it.
    pub(crate) fn masthead(team: &Team, i18n: &I18n) -> String {
        i18n.t(team.newsroom.masthead_key())
            .replace("{club}", &team.name)
    }

    pub(crate) fn issue(
        data: &SimulatorData,
        team: &Team,
        issue: &NewspaperIssue,
        masthead: &str,
        i18n: &I18n,
    ) -> IssueView {
        let paper = Paper {
            name: &team.name,
            club_id: team.club_id,
        };
        let stories = issue
            .stories
            .iter()
            .map(|story| Self::story(data, paper, story, i18n))
            .collect::<Vec<_>>();

        let Tiers {
            lead,
            secondary,
            run,
            briefs,
        } = Self::tiers(stories);

        let portrait = Self::portrait(data, team, issue, i18n);

        IssueView {
            number: issue.number,
            masthead: masthead.to_string(),
            // The club's own page never links its own nameplate; the
            // player page fills this in after typesetting.
            paper_slug: String::new(),
            date: i18n.format_date(issue.date),
            mood_label: i18n.t(issue.mood.i18n_key()).to_string(),
            mood_slug: issue.mood.slug(),
            mood_stamped: issue.mood.is_stamped(),
            lead,
            secondary,
            run,
            briefs,
            results: issue
                .results
                .iter()
                .map(|result| Self::result(data, result))
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
    fn tiers(stories: Vec<StoryView>) -> Tiers {
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

    /// The first story on the page with a face behind it. A paper never
    /// runs a front page of pure text when it has a picture available.
    fn portrait(
        data: &SimulatorData,
        team: &Team,
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
            caption: StoryComposer::headline(
                data,
                story,
                i18n,
                Paper {
                    name: &team.name,
                    club_id: team.club_id,
                },
            ),
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
    ) -> StoryView {
        let (player_name, player_slug) = PlayerName::resolve(data, story.player_id);
        let body = StoryComposer::body(data, story, i18n, paper);

        StoryView {
            kicker: i18n.t(story.kind.desk().i18n_key()).to_string(),
            headline: StoryComposer::headline(data, story, i18n, paper),
            // Read off the composed text, after the placeholders are
            // filled: whether a body opens with a letter depends on the
            // scoreline and the club name of this particular story, not
            // on the template it came from.
            drop_cap: body.chars().next().is_some_and(char::is_alphabetic),
            body,
            date: i18n.format_date(story.date),
            player_slug,
            player_name,
            is_quote: story.kind.is_quote(),
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
            is_cup: result.is_cup(),
        }
    }
}

/// The edition a story ran in, as the composer needs it.
///
/// `name` is the side the paper belongs to and fills the `{club}` slot;
/// `club_id` is the club behind it, which is where a `{manager}` is
/// looked for first. Carried together because the two always travel
/// together and a story lifted onto a player's page has to keep
/// crediting the newsroom that printed it.
#[derive(Clone, Copy)]
pub(crate) struct Paper<'a> {
    pub name: &'a str,
    pub club_id: u32,
}

/// Fills the blanks in a translated headline or body with the real
/// names, numbers and money of the story it belongs to.
struct StoryComposer;

impl StoryComposer {
    /// Highest variant suffix the composer probes for. Copy can grow to
    /// `_v6` for any stem without touching code.
    const MAX_VARIANTS: usize = 6;

    fn headline(data: &SimulatorData, story: &NewsStory, i18n: &I18n, paper: Paper<'_>) -> String {
        Self::compose(
            i18n.t(&Self::phrasing_key("h", story, i18n)),
            data,
            story,
            i18n,
            paper,
        )
    }

    fn body(data: &SimulatorData, story: &NewsStory, i18n: &I18n, paper: Paper<'_>) -> String {
        Self::compose(
            i18n.t(&Self::phrasing_key("b", story, i18n)),
            data,
            story,
            i18n,
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
    /// Headline and body always use the same index, so a pair written
    /// to share a register is printed as the pair it was written as.
    /// The bundles ship variants in matched, gap-free pairs (enforced
    /// by `variant_copy_ships_in_matched_pairs`), which is what lets
    /// the count be read off the headline keys alone.
    fn phrasing_key(part: &str, story: &NewsStory, i18n: &I18n) -> String {
        let stem = story.kind.key_stem();
        let index = Self::phrasing_index(story, Self::phrasing_count(i18n, stem));
        if index == 0 {
            format!("news_{}_{}", part, stem)
        } else {
            format!("news_{}_{}_v{}", part, stem, index + 1)
        }
    }

    /// How many phrasings the bundles carry for a stem. A missing key
    /// comes back from `t()` as the key itself, which is the probe.
    fn phrasing_count(i18n: &I18n, stem: &str) -> usize {
        let mut count = 1;
        for suffix in 2..=Self::MAX_VARIANTS {
            let key = format!("news_h_{}_v{}", stem, suffix);
            if i18n.t(&key) == key {
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

    fn compose(
        template: &str,
        data: &SimulatorData,
        story: &NewsStory,
        i18n: &I18n,
        paper: Paper<'_>,
    ) -> String {
        let mut text = template.to_string();

        if text.contains("{club}") {
            text = text.replace("{club}", paper.name);
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
        // The man in the dugout. A paper that writes "the manager" where
        // it could write his name is a paper nobody believes was written
        // by anybody, and a sacking is the one story a town discusses by
        // name for a decade.
        if text.contains("{manager}") {
            let mut name = ManagerName::resolve(data, paper.club_id, story.staff_id);
            if name.is_empty() {
                name = i18n.t("newspaper_unnamed_manager").to_string();
            }
            text = text.replace("{manager}", &name);
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
        // `{pts}` and `{m}` are the same slot under two names: the table
        // stories were written around points long before the squad and
        // loan desks needed a plain second number, and renaming the key
        // in nine bundles would buy nothing.
        if text.contains("{pts}") {
            text = text.replace("{pts}", &story.b.to_string());
        }
        if text.contains("{m}") {
            text = text.replace("{m}", &story.b.to_string());
        }

        text
    }

    /// Match reports name a team; every other desk names a club. Both
    /// arrive in the same slot, so the desk decides which lookup runs —
    /// keyed off the desk rather than a list of kinds, so a new story
    /// kind resolves its counterparty without anyone remembering to add
    /// it here.
    fn other_party(data: &SimulatorData, story: &NewsStory) -> String {
        if story.other_id == 0 {
            return String::new();
        }

        if story.kind.desk() == NewsDesk::Match {
            return data
                .team_data(story.other_id)
                .map(|team| team.name.clone())
                .unwrap_or_default();
        }

        data.club(story.other_id)
            .map(|club| club.name.clone())
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

            for chrome in [
                "newspaper",
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
}

/// The arithmetic that decides which run of type a story is set in.
/// Exercised without a world behind it: the question is only how many
/// pieces each part of the page gets, at every edition size the editor
/// can produce.
#[cfg(test)]
mod tier_tests {
    use super::{PressDesk, StoryView};
    use core::club::TeamNewsroom;

    fn stories(count: usize) -> Vec<StoryView> {
        (0..count)
            .map(|index| StoryView {
                kicker: "Squad".to_string(),
                headline: format!("Story {}", index),
                body: "Body.".to_string(),
                date: "2 March 2026".to_string(),
                player_slug: String::new(),
                player_name: String::new(),
                is_quote: false,
                drop_cap: true,
            })
            .collect()
    }

    /// However full the edition, the band under the fold never takes
    /// more than its four lines — the surplus is set as news instead.
    #[test]
    fn the_briefs_band_never_grows_past_its_four_lines() {
        for count in 0..=TeamNewsroom::MAX_STORIES {
            let tiers = PressDesk::tiers(stories(count));

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
            let tiers = PressDesk::tiers(stories(count));

            assert!(tiers.briefs.is_empty(), "{} stories reached the band", count);
            assert!(tiers.run.is_empty(), "{} stories reached the run", count);
        }
    }

    /// A full edition fills the page from the top: the two slots beside
    /// the rail, then the run across the measure, and only what is left
    /// over goes under the fold.
    #[test]
    fn a_full_edition_fills_the_page_before_the_fold() {
        let tiers = PressDesk::tiers(stories(TeamNewsroom::MAX_STORIES));

        assert!(tiers.lead.is_some());
        assert_eq!(tiers.secondary.len(), PressDesk::SECONDARY_SLOTS);
        assert_eq!(tiers.run.len(), 6);
        // Nine below the pair, four of which the band would have taken —
        // but five is a row and two thirds, so the run took a third one.
        assert_eq!(tiers.briefs.len(), 3);

        // The order the editor ranked them in survives the share-out:
        // the band under the fold holds the last of them and nothing else.
        assert_eq!(tiers.briefs[0].headline, "Story 9");
        assert_eq!(tiers.briefs[2].headline, "Story 11");
    }

    /// The page never ends on a part-built row. Whatever the edition
    /// size, the run comes out to a whole number of rows and the band
    /// under the fold is what pays for it.
    #[test]
    fn the_run_always_comes_out_to_whole_rows() {
        for count in 0..=TeamNewsroom::MAX_STORIES {
            let tiers = PressDesk::tiers(stories(count));

            assert_eq!(
                tiers.run.len() % PressDesk::RUN_COLUMNS,
                0,
                "an edition of {} left {} stories alone on the last row",
                count,
                tiers.run.len() % PressDesk::RUN_COLUMNS
            );
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
                ("newspaper_cup_tie", "Cup"),
                ("news_desk_loan", "Loan watch"),
                ("news_desk_fans", "The terraces"),
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
                newspaper_count: issues.len(),
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
                is_quote: false,
                drop_cap: true,
            }
        }

        fn quote(kicker: &str, headline: &str, body: &str) -> StoryView {
            StoryView {
                is_quote: true,
                ..Self::story(kicker, headline, body, true)
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
                    Self::quote(
                        "Loan watch",
                        "Diego Mora wants to come home",
                        "\u{201c}I did not go there to sit and watch. I want to come back \
                         and fight for my place at Córdoba.\u{201d}",
                    ),
                ],
                run: vec![
                    Self::story(
                        "Match",
                        "Córdoba make it 4 in a row",
                        "Four wins on the run, and the table now reads better than it has \
                         at any point this season.",
                        false,
                    ),
                    Self::story(
                        "Market",
                        "Rubén Ortega placed on the list",
                        "A month of training with the reserves ends the only way it was \
                         ever going to end.",
                        true,
                    ),
                    Self::story(
                        "Boardroom",
                        "Córdoba board back the manager",
                        "A public word of support and, more usefully, an understanding \
                         about what the next window will look like.",
                        false,
                    ),
                    Self::story(
                        "Squad",
                        "180 appearances for Andrés Vidal",
                        "Eight seasons, one club, and a milestone the dressing room made \
                         rather more of than he did.",
                        true,
                    ),
                    Self::story(
                        "Loan watch",
                        "4 goals on loan for Pau Ferrer",
                        "The reports coming back are good enough that his return is now a \
                         question of when rather than whether.",
                        true,
                    ),
                    Self::story(
                        "Market",
                        "Sevilla circle for Iván Salas",
                        "Nothing has been put in writing yet, which is not the same thing \
                         as nothing having been said.",
                        true,
                    ),
                ],
                briefs: vec![
                    Self::story("Squad", "Diego Mora back in training", "", true),
                    Self::story("Squad", "Nico Reyes out for 42 days", "", true),
                    Self::story("The terraces", "The crowd stays with them", "", false),
                ],
                results: vec![
                    ResultView {
                        date: "25.02".to_string(),
                        opponent_name: "Real Zaragoza".to_string(),
                        opponent_slug: "real-zaragoza".to_string(),
                        score: "1-1".to_string(),
                        outcome: "d",
                        is_cup: true,
                    },
                    ResultView {
                        date: "01.03".to_string(),
                        opponent_name: "Sevilla".to_string(),
                        opponent_slug: "sevilla".to_string(),
                        score: "4-1".to_string(),
                        outcome: "w",
                        is_cup: false,
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
                    "Diego Mora is the real thing at 18",
                    "A season average of 7.34 at an age when most of his year group are \
                     still in the youth team. Córdoba know what they have, and so, by now, \
                     does everybody else.",
                    true,
                )),
                secondary: Vec::new(),
                run: Vec::new(),
                briefs: vec![
                    Self::story("Squad", "Diego Mora called up by his country", "", true),
                    Self::story("Market", "Scouts sent to watch Diego Mora", "", true),
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
                    "Córdoba board back the manager",
                    "A public word of support from the board and, more usefully, an \
                     understanding about what the next window will look like.",
                    false,
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
        assert!(html.contains("Córdoba tear Sevilla apart"));
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
                body: "4-1 against Sevilla, and it could have been more.".to_string(),
                drop_cap: false,
                ..Page::story("Match", "Córdoba tear Sevilla apart", "", false)
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

    /// A cup tie in the ruled column is marked, because "1-1" against a
    /// second-division side reads very differently in a knockout round.
    #[test]
    fn a_cup_tie_is_marked_in_the_results_column() {
        let html = Page::template(vec![Page::full_issue()]).render().unwrap();

        assert_eq!(html.matches("np-result-cup").count(), 1);
    }

    /// Every headline about a person leads to that person. A reader who
    /// wants to know who "Diego Mora" is should not have to go back to
    /// the squad list and find him by hand.
    #[test]
    fn a_headline_about_a_player_links_to_him() {
        let html = Page::template(vec![Page::full_issue()]).render().unwrap();

        // Both secondaries, four of the six stories in the run and the
        // portrait caption are about a player; the lead is a match report
        // and correctly carries no link.
        assert_eq!(html.matches("np-story-link").count(), 7);
        assert!(html.contains(
            "<a class=\"np-story-link\" href=\"/en/players/17-diego-mora\" \
             title=\"Diego Mora\">Three goals for Diego Mora</a>"
        ));
    }

    /// A headline with nobody behind it must stay plain text — wrapping
    /// "Córdoba board back the manager" in a dead player link would be
    /// worse than not linking at all.
    #[test]
    fn a_club_headline_carries_no_player_link() {
        let html = Page::template(vec![Page::bare_issue()]).render().unwrap();

        assert!(!html.contains("np-story-link"));
        assert!(html.contains("Córdoba board back the manager"));
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

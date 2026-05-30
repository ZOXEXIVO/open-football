use crate::I18n;
use core::Club;
use core::SimulatorData;
use core::league::League;

/// Build the parent-club's left-menu team list ready for the template:
/// sorted with Main first, then Second, B, Reserve, U23..U18, with
/// reputation tiebreaking inside a type. Each entry is `(label, slug)`.
pub fn neighbor_teams(club: &Club, i18n: &I18n) -> Vec<(String, String)> {
    use std::cmp::Reverse;
    let club_name = club.name.as_str();
    let mut entries: Vec<(String, String, u8, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| {
            (
                team.team_type.menu_label(
                    club_name,
                    &team.name,
                    i18n.t(team.team_type.as_i18n_key()),
                ),
                team.slug.clone(),
                team.team_type.menu_order(),
                team.reputation.world,
            )
        })
        .collect();
    entries.sort_by_key(|(_, _, ord, rep)| (*ord, Reverse(*rep)));
    entries
        .into_iter()
        .map(|(label, slug, _, _)| (label, slug))
        .collect()
}

pub fn club_country_info(simulator_data: &SimulatorData, club_id: u32) -> (&str, &str) {
    simulator_data
        .country_by_club(club_id)
        .map(|c| (c.name.as_str(), c.slug.as_str()))
        .unwrap_or_default()
}

pub fn league_display_name(league: &League, i18n: &I18n, simulator_data: &SimulatorData) -> String {
    let country_adj = simulator_data
        .country(league.country_id)
        .map(|c| i18n.country_en(&c.code))
        .unwrap_or("");
    if country_adj.is_empty() {
        league.name.clone()
    } else {
        format!("{} {}", country_adj, league.name)
    }
}

pub struct MenuSection {
    pub items: Vec<MenuItem>,
    pub collapsible: bool,
    pub expanded: bool,
}

impl MenuSection {
    fn plain(items: Vec<MenuItem>) -> Self {
        Self {
            items,
            collapsible: false,
            expanded: false,
        }
    }

    /// Build a section whose items collapse to the first 2 when there are
    /// more than 2 entries. Expands on initial render if the active item
    /// would otherwise be hidden.
    fn collapsible_after_two(items: Vec<MenuItem>) -> Self {
        let collapsible = items.len() > 2;
        let expanded = collapsible && items.iter().skip(2).any(|i| i.active);
        Self {
            items,
            collapsible,
            expanded,
        }
    }
}

pub struct MenuItem {
    pub title: String,
    pub url: String,
    pub icon: String,
    pub active: bool,
}

pub struct MenuParams<'a> {
    pub i18n: &'a I18n,
    pub lang: &'a str,
    pub current_path: &'a str,
    pub country_name: &'a str,
    pub country_slug: &'a str,
}

impl<'a> MenuParams<'a> {
    fn home_and_country_sections(&self) -> Vec<MenuSection> {
        // Country landing (→ leagues), then the two national-team squads.
        // The level switch lives here in the left menu, not on-page tabs:
        //   "England"      → senior squad (base country URL)
        //   "England U21"  → U21 squad (`/u21`)
        // Each national-team row is active only on its own page.
        let senior_url = format!("/{}/countries/{}", self.lang, self.country_slug);
        let u21_url = format!("/{}/countries/{}/u21", self.lang, self.country_slug);
        let leagues_url = format!("/{}/countries/{}/leagues", self.lang, self.country_slug);
        vec![
            home_section(self.i18n, self.lang),
            search_section(self.i18n, self.lang, self.current_path),
            MenuSection::plain(vec![MenuItem {
                title: self.country_name.to_string(),
                active: self.current_path == leagues_url,
                url: leagues_url,
                icon: "fa-home".to_string(),
            }]),
            MenuSection::plain(vec![
                MenuItem {
                    active: self.current_path == senior_url,
                    title: format!("{} {}", self.country_name, self.i18n.t("team")),
                    url: senior_url,
                    icon: "fa-users".to_string(),
                },
                MenuItem {
                    active: self.current_path == u21_url,
                    title: format!(
                        "{} {} {}",
                        self.country_name,
                        self.i18n.t("u21"),
                        self.i18n.t("team")
                    ),
                    url: u21_url,
                    icon: "fa-users".to_string(),
                },
            ]),
        ]
    }
}

fn home_section(i18n: &I18n, lang: &str) -> MenuSection {
    MenuSection::plain(vec![MenuItem {
        title: i18n.t("home").to_string(),
        url: format!("/{}", lang),
        icon: "fa-home".to_string(),
        active: false,
    }])
}

fn search_section(i18n: &I18n, lang: &str, current_path: &str) -> MenuSection {
    let search_url = format!("/{}/search", lang);
    MenuSection::plain(vec![MenuItem {
        active: current_path == search_url,
        title: i18n.t("search").to_string(),
        url: search_url,
        icon: "fa-search".to_string(),
    }])
}

pub fn ai_menu(i18n: &I18n, lang: &str, current_path: &str) -> Vec<MenuSection> {
    let ai_url = format!("/{}/ai", lang);
    vec![
        home_section(i18n, lang),
        search_section(i18n, lang, current_path),
        MenuSection::plain(vec![MenuItem {
            active: current_path == ai_url,
            title: i18n.t("ai_management").to_string(),
            url: ai_url,
            icon: "fa-robot".to_string(),
        }]),
    ]
}

pub fn watchlist_menu(i18n: &I18n, lang: &str, current_path: &str) -> Vec<MenuSection> {
    vec![
        home_section(i18n, lang),
        search_section(i18n, lang, current_path),
        watchlist_section(i18n, lang, current_path),
    ]
}

pub fn search_menu(i18n: &I18n, lang: &str, current_path: &str) -> Vec<MenuSection> {
    vec![
        home_section(i18n, lang),
        search_section(i18n, lang, current_path),
    ]
}

/// Continent ids (matching the embedded database).
const CONTINENT_EUROPE: u32 = 1;
const CONTINENT_SOUTH_AMERICA: u32 = 3;

/// Continental-cup links scoped to the viewer's continent: UEFA club
/// competitions (Champions/Europa/Conference League) for European
/// countries, Copa Libertadores for South American ones. Other
/// continents get no continental-cup section. Returns `None` when there
/// is nothing relevant to show.
fn continental_section(
    i18n: &I18n,
    lang: &str,
    current_path: &str,
    continent_id: u32,
) -> Option<MenuSection> {
    let star = |key: &str, path: String| MenuItem {
        active: current_path == path,
        title: i18n.t(key).to_string(),
        url: path,
        icon: "fa-star".to_string(),
    };

    let items = match continent_id {
        CONTINENT_EUROPE => vec![
            star("champions_league", format!("/{}/champions-league", lang)),
            star("europa_league", format!("/{}/europa-league", lang)),
            star("conference_league", format!("/{}/conference-league", lang)),
        ],
        CONTINENT_SOUTH_AMERICA => vec![star(
            "copa_libertadores",
            format!("/{}/copa-libertadores", lang),
        )],
        _ => Vec::new(),
    };

    if items.is_empty() {
        None
    } else {
        Some(MenuSection::plain(items))
    }
}

fn national_section(i18n: &I18n, lang: &str, current_path: &str) -> MenuSection {
    let nat_url = format!("/{}/national-competitions", lang);
    MenuSection::plain(vec![MenuItem {
        active: current_path == nat_url,
        title: i18n.t("national_competitions").to_string(),
        url: nat_url,
        icon: "fa-flag".to_string(),
    }])
}

fn watchlist_section(i18n: &I18n, lang: &str, current_path: &str) -> MenuSection {
    let watchlist_url = format!("/{}/watchlist", lang);
    MenuSection::plain(vec![MenuItem {
        active: current_path == watchlist_url,
        title: i18n.t("watchlist").to_string(),
        url: watchlist_url,
        icon: "fa-eye".to_string(),
    }])
}

/// Build the country's league-pyramid menu section: each league links to
/// `/leagues/{slug}`, collapsing to the first two with a toggle when there
/// are more. Active state is derived from `current_path`, so the same
/// builder serves the league, country and cup pages.
fn leagues_section(p: &MenuParams, country_leagues: &[(&str, &str)]) -> MenuSection {
    let is_active = |url: &str| -> bool {
        p.current_path == url || p.current_path.starts_with(&format!("{}/", url))
    };
    let items: Vec<MenuItem> = country_leagues
        .iter()
        .map(|(name, slug)| {
            let url = format!("/{}/leagues/{}", p.lang, slug);
            MenuItem {
                active: is_active(&url),
                title: name.to_string(),
                url,
                icon: "fa-trophy".to_string(),
            }
        })
        .collect();
    MenuSection::collapsible_after_two(items)
}

/// Build the domestic-cup menu section. Kept as its own standalone section
/// — sitting directly beneath the league pyramid and separated from it by
/// the usual section divider — rather than appended to the (collapsible)
/// league list, so the cup link always stays visible and never hides
/// behind the "more leagues" toggle.
fn cup_section(p: &MenuParams, cup: (&str, &str)) -> MenuSection {
    let (name, slug) = cup;
    let url = format!("/{}/cups/{}", p.lang, slug);
    let active = p.current_path == url || p.current_path.starts_with(&format!("{}/", url));
    MenuSection::plain(vec![MenuItem {
        active,
        title: name.to_string(),
        url,
        icon: "fa-trophy".to_string(),
    }])
}

pub fn league_menu(
    p: &MenuParams,
    country_leagues: &[(&str, &str)],
    cup: Option<(&str, &str)>,
) -> Vec<MenuSection> {
    let mut sections = p.home_and_country_sections();

    if !country_leagues.is_empty() {
        sections.push(leagues_section(p, country_leagues));
    }
    if let Some(c) = cup {
        sections.push(cup_section(p, c));
    }

    sections.push(watchlist_section(p.i18n, p.lang, p.current_path));
    sections
}

/// Left-menu for the domestic-cup page. Same competition layout as
/// `league_menu` (the country's league pyramid plus the cup), with the cup
/// itself marked active via `current_path`. The cup has no transfers/awards
/// sub-pages, so those league-only links are omitted.
pub fn cup_menu(
    p: &MenuParams,
    _cup_slug: &str,
    country_leagues: &[(&str, &str)],
    cup_name: &str,
    continent_id: u32,
) -> Vec<MenuSection> {
    let mut sections = p.home_and_country_sections();
    if !country_leagues.is_empty() {
        sections.push(leagues_section(p, country_leagues));
    }
    sections.push(cup_section(p, (cup_name, _cup_slug)));
    if let Some(s) = continental_section(p.i18n, p.lang, p.current_path, continent_id) {
        sections.push(s);
    }
    sections.push(national_section(p.i18n, p.lang, p.current_path));
    sections.push(watchlist_section(p.i18n, p.lang, p.current_path));
    sections
}

pub fn team_menu(
    p: &MenuParams,
    neighbor_teams: &[(&str, &str)],
    leagues: &[(&str, &str)],
) -> Vec<MenuSection> {
    let mut sections = p.home_and_country_sections();

    if !leagues.is_empty() {
        sections.push(MenuSection::collapsible_after_two(
            leagues
                .iter()
                .map(|(league_name, league_slug)| {
                    let league_url = format!("/{}/leagues/{}", p.lang, league_slug);
                    MenuItem {
                        active: false,
                        title: league_name.to_string(),
                        url: league_url,
                        icon: "fa-trophy".to_string(),
                    }
                })
                .collect(),
        ));
    }

    if !neighbor_teams.is_empty() {
        sections.push(MenuSection::plain(
            neighbor_teams
                .iter()
                .map(|(name, slug)| {
                    let url = format!("/{}/teams/{}", p.lang, slug);
                    let is_active =
                        p.current_path == url || p.current_path.starts_with(&format!("{}/", url));
                    MenuItem {
                        active: is_active,
                        title: name.to_string(),
                        url,
                        icon: "fa-light fa-people-group".to_string(),
                    }
                })
                .collect(),
        ));
    }

    sections.push(watchlist_section(p.i18n, p.lang, p.current_path));

    sections
}

pub fn country_menu(
    p: &MenuParams,
    country_leagues: &[(&str, &str)],
    cup: Option<(&str, &str)>,
    continent_id: u32,
) -> Vec<MenuSection> {
    let mut sections = p.home_and_country_sections();

    if !country_leagues.is_empty() {
        sections.push(leagues_section(p, country_leagues));
    }
    if let Some(c) = cup {
        sections.push(cup_section(p, c));
    }

    if let Some(s) = continental_section(p.i18n, p.lang, p.current_path, continent_id) {
        sections.push(s);
    }
    sections.push(national_section(p.i18n, p.lang, p.current_path));
    sections.push(watchlist_section(p.i18n, p.lang, p.current_path));

    sections
}

#[allow(dead_code)]
pub fn match_menu(i18n: &I18n, lang: &str, current_path: &str) -> Vec<MenuSection> {
    let mut sections = vec![home_section(i18n, lang)];
    sections.push(search_section(i18n, lang, current_path));
    sections.push(watchlist_section(i18n, lang, current_path));
    sections
}

fn continental_competitions_menu(i18n: &I18n, lang: &str, current_path: &str) -> Vec<MenuSection> {
    let cl_url = format!("/{}/champions-league", lang);
    let el_url = format!("/{}/europa-league", lang);
    let conf_url = format!("/{}/conference-league", lang);
    let copa_url = format!("/{}/copa-libertadores", lang);
    let nat_url = format!("/{}/national-competitions", lang);
    vec![
        home_section(i18n, lang),
        search_section(i18n, lang, current_path),
        MenuSection::plain(vec![
            MenuItem {
                active: current_path == cl_url,
                title: i18n.t("champions_league").to_string(),
                url: cl_url,
                icon: "fa-trophy".to_string(),
            },
            MenuItem {
                active: current_path == el_url,
                title: i18n.t("europa_league").to_string(),
                url: el_url,
                icon: "fa-trophy".to_string(),
            },
            MenuItem {
                active: current_path == conf_url,
                title: i18n.t("conference_league").to_string(),
                url: conf_url,
                icon: "fa-trophy".to_string(),
            },
            MenuItem {
                active: current_path == copa_url,
                title: i18n.t("copa_libertadores").to_string(),
                url: copa_url,
                icon: "fa-trophy".to_string(),
            },
        ]),
        MenuSection::plain(vec![MenuItem {
            active: current_path == nat_url,
            title: i18n.t("national_competitions").to_string(),
            url: nat_url,
            icon: "fa-flag".to_string(),
        }]),
        watchlist_section(i18n, lang, current_path),
    ]
}

pub fn champions_league_menu(i18n: &I18n, lang: &str, current_path: &str) -> Vec<MenuSection> {
    continental_competitions_menu(i18n, lang, current_path)
}

pub fn europa_league_menu(i18n: &I18n, lang: &str, current_path: &str) -> Vec<MenuSection> {
    continental_competitions_menu(i18n, lang, current_path)
}

pub fn conference_league_menu(i18n: &I18n, lang: &str, current_path: &str) -> Vec<MenuSection> {
    continental_competitions_menu(i18n, lang, current_path)
}

pub fn copa_libertadores_menu(i18n: &I18n, lang: &str, current_path: &str) -> Vec<MenuSection> {
    continental_competitions_menu(i18n, lang, current_path)
}

pub fn national_competitions_menu(i18n: &I18n, lang: &str, current_path: &str) -> Vec<MenuSection> {
    continental_competitions_menu(i18n, lang, current_path)
}

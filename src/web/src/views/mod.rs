use crate::I18n;

pub struct MenuSection {
    pub items: Vec<MenuItem>,
}

pub struct MenuItem {
    pub title: String,
    pub url: String,
    pub icon: String,
    pub active: bool,
}

pub fn ai_menu(i18n: &I18n, lang: &str, current_path: &str) -> Vec<MenuSection> {
    let ai_url = format!("/{}/ai", lang);
    vec![
        MenuSection {
            items: vec![MenuItem {
                title: i18n.t("home").to_string(),
                url: format!("/{}", lang),
                icon: "fa-home".to_string(),
                active: false,
            }],
        },
        MenuSection {
            items: vec![MenuItem {
                active: current_path == ai_url,
                title: i18n.t("ai_management").to_string(),
                url: ai_url,
                icon: "fa-robot".to_string(),
            }],
        },
        source_code_section(),
    ]
}

pub fn watchlist_menu(i18n: &I18n, lang: &str, current_path: &str) -> Vec<MenuSection> {
    vec![
        MenuSection {
            items: vec![MenuItem {
                title: i18n.t("home").to_string(),
                url: format!("/{}", lang),
                icon: "fa-home".to_string(),
                active: false,
            }],
        },
        watchlist_section(i18n, lang, current_path),
        source_code_section(),
    ]
}

fn source_code_section() -> MenuSection {
    MenuSection {
        items: vec![MenuItem {
            active: false,
            title: "Source code".to_string(),
            url: "https://github.com/ZOXEXIVO/open-football".to_string(),
            icon: "fa-brands fa-github".to_string(),
        }],
    }
}

fn watchlist_section(i18n: &I18n, lang: &str, current_path: &str) -> MenuSection {
    let watchlist_url = format!("/{}/watchlist", lang);
    MenuSection {
        items: vec![MenuItem {
            active: current_path == watchlist_url,
            title: i18n.t("watchlist").to_string(),
            url: watchlist_url,
            icon: "fa-eye".to_string(),
        }],
    }
}

pub fn league_menu(i18n: &I18n, lang: &str, country_name: &str, country_slug: &str, league_slug: &str, current_path: &str, country_leagues: &[(&str, &str)]) -> Vec<MenuSection> {
    let transfers_url = format!("/{}/leagues/{}/transfers", lang, league_slug);
    let mut sections = vec![
        MenuSection {
            items: vec![MenuItem {
                title: i18n.t("home").to_string(),
                url: format!("/{}", lang),
                icon: "fa-home".to_string(),
                active: false,
            }],
        },
        MenuSection {
            items: vec![MenuItem {
                title: country_name.to_string(),
                url: format!("/{}/countries/{}", lang, country_slug),
                icon: "fa-home".to_string(),
                active: false,
            }],
        },
        MenuSection {
            items: country_leagues
                .iter()
                .map(|(name, slug)| {
                    let url = format!("/{}/leagues/{}", lang, slug);
                    let is_active = current_path == url || current_path.starts_with(&format!("{}/", url));
                    MenuItem {
                        active: is_active,
                        title: name.to_string(),
                        url,
                        icon: "fa-trophy".to_string(),
                    }
                })
                .collect(),
        },
        MenuSection {
            items: vec![MenuItem {
                active: current_path == transfers_url,
                title: i18n.t("transfers").to_string(),
                url: transfers_url,
                icon: "fa-exchange".to_string(),
            }],
        },
    ];
    sections.push(watchlist_section(i18n, lang, current_path));
    sections.push(source_code_section());
    sections
}

pub fn team_menu(i18n: &I18n, lang: &str, neighbor_teams: &[(&str, &str)], team_slug: &str, current_path: &str, leagues: &[(&str, &str)], is_main_team: bool) -> Vec<MenuSection> {
    let mut sections = vec![
        MenuSection {
            items: vec![MenuItem {
                title: i18n.t("home").to_string(),
                url: format!("/{}", lang),
                icon: "fa-home".to_string(),
                active: false,
            }],
        },
    ];

    if !leagues.is_empty() {
        sections.push(MenuSection {
            items: leagues
                .iter()
                .map(|(league_name, league_slug)| {
                    let league_url = format!("/{}/leagues/{}", lang, league_slug);
                    MenuItem {
                        active: false,
                        title: league_name.to_string(),
                        url: league_url,
                        icon: "fa-trophy".to_string(),
                    }
                })
                .collect(),
        });
    }

    if !neighbor_teams.is_empty() {
        sections.push(MenuSection {
            items: neighbor_teams
                .iter()
                .map(|(name, slug)| {
                    let url = format!("/{}/teams/{}", lang, slug);
                    let is_active = current_path == url || current_path.starts_with(&format!("{}/", url));
                    MenuItem {
                        active: is_active,
                        title: name.to_string(),
                        url,
                        icon: "fa-light fa-people-group".to_string(),
                    }
                })
                .collect(),
        });
    }

    let staff_url = format!("/{}/teams/{}/staff", lang, team_slug);

    sections.push(MenuSection {
        items: vec![
            MenuItem {
                active: current_path == staff_url,
                title: i18n.t("staff").to_string(),
                url: staff_url,
                icon: "fa-id-badge".to_string(),
            },
        ],
    });

    let tactics_url = format!("/{}/teams/{}/tactics", lang, team_slug);
    let schedule_url = format!("/{}/teams/{}/schedule", lang, team_slug);
    let transfers_url = format!("/{}/teams/{}/transfers", lang, team_slug);

    let mut items = vec![
        MenuItem {
            active: current_path == tactics_url,
            title: i18n.t("tactics").to_string(),
            url: tactics_url,
            icon: "fa-chess".to_string(),
        },
    ];

    if is_main_team {
        let finances_url = format!("/{}/teams/{}/finances", lang, team_slug);
        let academy_url = format!("/{}/teams/{}/academy", lang, team_slug);
        items.push(MenuItem {
            active: current_path == finances_url,
            title: i18n.t("finances").to_string(),
            url: finances_url,
            icon: "fa-coins".to_string(),
        });
        items.push(MenuItem {
            active: current_path == academy_url,
            title: i18n.t("academy").to_string(),
            url: academy_url,
            icon: "fa-graduation-cap".to_string(),
        });
    }

    items.push(MenuItem {
        active: current_path == schedule_url,
        title: i18n.t("schedule").to_string(),
        url: schedule_url,
        icon: "fa-calendar".to_string(),
    });
    items.push(MenuItem {
        active: current_path == transfers_url,
        title: i18n.t("transfers").to_string(),
        url: transfers_url,
        icon: "fa-exchange".to_string(),
    });

    sections.push(MenuSection { items });

    sections.push(watchlist_section(i18n, lang, current_path));
    sections.push(source_code_section());

    sections
}

pub fn player_menu(i18n: &I18n, lang: &str, neighbor_teams: &[(&str, &str)], team_slug: &str, current_path: &str, leagues: &[(&str, &str)], is_main_team: bool) -> Vec<MenuSection> {
    let mut sections = vec![
        MenuSection {
            items: vec![MenuItem {
                title: i18n.t("home").to_string(),
                url: format!("/{}", lang),
                icon: "fa-home".to_string(),
                active: false,
            }],
        },
    ];

    if !leagues.is_empty() {
        sections.push(MenuSection {
            items: leagues
                .iter()
                .map(|(league_name, league_slug)| {
                    let league_url = format!("/{}/leagues/{}", lang, league_slug);
                    MenuItem {
                        active: false,
                        title: league_name.to_string(),
                        url: league_url,
                        icon: "fa-trophy".to_string(),
                    }
                })
                .collect(),
        });
    }

    if !neighbor_teams.is_empty() {
        sections.push(MenuSection {
            items: neighbor_teams
                .iter()
                .map(|(name, slug)| {
                    let url = format!("/{}/teams/{}", lang, slug);
                    let is_active = current_path == url || current_path.starts_with(&format!("{}/", url));
                    MenuItem {
                        active: is_active,
                        title: name.to_string(),
                        url,
                        icon: "fa-light fa-people-group".to_string(),
                    }
                })
                .collect(),
        });
    }

    let staff_url = format!("/{}/teams/{}/staff", lang, team_slug);

    sections.push(MenuSection {
        items: vec![
            MenuItem {
                active: current_path == staff_url,
                title: i18n.t("staff").to_string(),
                url: staff_url,
                icon: "fa-id-badge".to_string(),
            },
        ],
    });

    let tactics_url = format!("/{}/teams/{}/tactics", lang, team_slug);
    let schedule_url = format!("/{}/teams/{}/schedule", lang, team_slug);
    let transfers_url = format!("/{}/teams/{}/transfers", lang, team_slug);

    let mut items = vec![
        MenuItem {
            active: current_path == tactics_url,
            title: i18n.t("tactics").to_string(),
            url: tactics_url,
            icon: "fa-chess".to_string(),
        },
    ];

    if is_main_team {
        let finances_url = format!("/{}/teams/{}/finances", lang, team_slug);
        let academy_url = format!("/{}/teams/{}/academy", lang, team_slug);
        items.push(MenuItem {
            active: current_path == finances_url,
            title: i18n.t("finances").to_string(),
            url: finances_url,
            icon: "fa-coins".to_string(),
        });
        items.push(MenuItem {
            active: current_path == academy_url,
            title: i18n.t("academy").to_string(),
            url: academy_url,
            icon: "fa-graduation-cap".to_string(),
        });
    }

    items.push(MenuItem {
        active: current_path == schedule_url,
        title: i18n.t("schedule").to_string(),
        url: schedule_url,
        icon: "fa-calendar".to_string(),
    });
    items.push(MenuItem {
        active: current_path == transfers_url,
        title: i18n.t("transfers").to_string(),
        url: transfers_url,
        icon: "fa-exchange".to_string(),
    });

    sections.push(MenuSection { items });

    sections.push(watchlist_section(i18n, lang, current_path));
    sections.push(source_code_section());

    sections
}

pub fn staff_menu(i18n: &I18n, lang: &str, neighbor_teams: &[(&str, &str)], team_slug: &str, current_path: &str, leagues: &[(&str, &str)], is_main_team: bool) -> Vec<MenuSection> {
    team_menu(i18n, lang, neighbor_teams, team_slug, current_path, leagues, is_main_team)
}

pub fn country_menu(i18n: &I18n, lang: &str, _country_slug: &str, current_path: &str, country_leagues: &[(&str, &str)]) -> Vec<MenuSection> {
    let mut sections = vec![
        MenuSection {
            items: vec![MenuItem {
                title: i18n.t("home").to_string(),
                url: format!("/{}", lang),
                icon: "fa-home".to_string(),
                active: false,
            }],
        },
    ];

    if !country_leagues.is_empty() {
        sections.push(MenuSection {
            items: country_leagues
                .iter()
                .map(|(name, slug)| {
                    let url = format!("/{}/leagues/{}", lang, slug);
                    let is_active = current_path == url || current_path.starts_with(&format!("{}/", url));
                    MenuItem {
                        active: is_active,
                        title: name.to_string(),
                        url,
                        icon: "fa-trophy".to_string(),
                    }
                })
                .collect(),
        });

        let first_league_slug = country_leagues[0].1;
        let transfers_url = format!("/{}/leagues/{}/transfers", lang, first_league_slug);
        sections.push(MenuSection {
            items: vec![MenuItem {
                active: current_path == transfers_url,
                title: i18n.t("transfers").to_string(),
                url: transfers_url,
                icon: "fa-exchange".to_string(),
            }],
        });
    }

    sections.push(watchlist_section(i18n, lang, current_path));
    sections.push(source_code_section());

    sections
}

#[allow(dead_code)]
pub fn match_menu(i18n: &I18n, lang: &str, current_path: &str) -> Vec<MenuSection> {
    let mut sections = vec![MenuSection {
        items: vec![MenuItem {
            title: i18n.t("home").to_string(),
            url: format!("/{}", lang),
            icon: "fa-home".to_string(),
            active: false,
        }],
    }];
    sections.push(watchlist_section(i18n, lang, current_path));
    sections.push(source_code_section());
    sections
}

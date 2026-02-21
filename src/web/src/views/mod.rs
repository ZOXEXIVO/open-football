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

pub fn league_menu(i18n: &I18n, lang: &str, country_name: &str, country_slug: &str, league_slug: &str, current_path: &str, country_leagues: &[(&str, &str)]) -> Vec<MenuSection> {
    let transfers_url = format!("/{}/leagues/{}/transfers", lang, league_slug);
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
    ]
}

pub fn team_menu(i18n: &I18n, lang: &str, neighbor_teams: &[(&str, &str)], team_slug: &str, current_path: &str, leagues: &[(&str, &str)]) -> Vec<MenuSection> {
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

    sections.push(MenuSection {
        items: vec![
            MenuItem {
                active: current_path == tactics_url,
                title: i18n.t("tactics").to_string(),
                url: tactics_url,
                icon: "fa-chess".to_string(),
            },
            MenuItem {
                active: current_path == schedule_url,
                title: i18n.t("schedule").to_string(),
                url: schedule_url,
                icon: "fa-calendar".to_string(),
            },
            MenuItem {
                active: current_path == transfers_url,
                title: i18n.t("transfers").to_string(),
                url: transfers_url,
                icon: "fa-exchange".to_string(),
            },
        ],
    });

    sections
}

pub fn player_menu(i18n: &I18n, lang: &str, neighbor_teams: &[(&str, &str)], team_slug: &str, current_path: &str, leagues: &[(&str, &str)]) -> Vec<MenuSection> {
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

    sections.push(MenuSection {
        items: vec![
            MenuItem {
                active: current_path == tactics_url,
                title: i18n.t("tactics").to_string(),
                url: tactics_url,
                icon: "fa-chess".to_string(),
            },
            MenuItem {
                active: current_path == schedule_url,
                title: i18n.t("schedule").to_string(),
                url: schedule_url,
                icon: "fa-calendar".to_string(),
            },
            MenuItem {
                active: current_path == transfers_url,
                title: i18n.t("transfers").to_string(),
                url: transfers_url,
                icon: "fa-exchange".to_string(),
            },
        ],
    });

    sections
}

pub fn staff_menu(i18n: &I18n, lang: &str, neighbor_teams: &[(&str, &str)], team_slug: &str, current_path: &str, leagues: &[(&str, &str)]) -> Vec<MenuSection> {
    team_menu(i18n, lang, neighbor_teams, team_slug, current_path, leagues)
}

#[allow(dead_code)]
pub fn match_menu(i18n: &I18n, lang: &str) -> Vec<MenuSection> {
    vec![MenuSection {
        items: vec![MenuItem {
            title: i18n.t("home").to_string(),
            url: format!("/{}", lang),
            icon: "fa-home".to_string(),
            active: false,
        }],
    }]
}

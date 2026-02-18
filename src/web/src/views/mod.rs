pub struct MenuSection {
    pub items: Vec<MenuItem>,
}

pub struct MenuItem {
    pub title: String,
    pub url: String,
    pub icon: String,
    pub active: bool,
}

pub fn league_menu(country_name: &str, country_slug: &str, league_name: &str, league_slug: &str, current_path: &str) -> Vec<MenuSection> {
    let league_url = format!("/leagues/{}", league_slug);
    let transfers_url = format!("/leagues/{}/transfers", league_slug);
    vec![
        MenuSection {
            items: vec![MenuItem {
                title: "Home".to_string(),
                url: "/".to_string(),
                icon: "fa-home".to_string(),
                active: false,
            }],
        },
        MenuSection {
            items: vec![MenuItem {
                title: country_name.to_string(),
                url: format!("/countries/{}", country_slug),
                icon: "fa-home".to_string(),
                active: false,
            }],
        },
        MenuSection {
            items: vec![
                MenuItem {
                    active: current_path == league_url,
                    title: league_name.to_string(),
                    url: league_url,
                    icon: "fa-home".to_string(),
                },
                MenuItem {
                    active: current_path == transfers_url,
                    title: "Transfers".to_string(),
                    url: transfers_url,
                    icon: "fa-exchange".to_string(),
                },
            ],
        },
    ]
}

pub fn team_menu(neighbor_teams: &[(&str, &str)], team_slug: &str, current_path: &str) -> Vec<MenuSection> {
    let mut sections = vec![
        MenuSection {
            items: vec![MenuItem {
                title: "Home".to_string(),
                url: "/".to_string(),
                icon: "fa-home".to_string(),
                active: false,
            }],
        },
    ];

    if !neighbor_teams.is_empty() {
        sections.push(MenuSection {
            items: neighbor_teams
                .iter()
                .map(|(name, slug)| {
                    let url = format!("/teams/{}", slug);
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

    let tactics_url = format!("/teams/{}/tactics", team_slug);
    let schedule_url = format!("/teams/{}/schedule", team_slug);
    let transfers_url = format!("/teams/{}/transfers", team_slug);

    sections.push(MenuSection {
        items: vec![
            MenuItem {
                active: current_path == tactics_url,
                title: "Tactics".to_string(),
                url: tactics_url,
                icon: "fa-chess".to_string(),
            },
            MenuItem {
                active: current_path == schedule_url,
                title: "Schedule".to_string(),
                url: schedule_url,
                icon: "fa-calendar".to_string(),
            },
            MenuItem {
                active: current_path == transfers_url,
                title: "Transfers".to_string(),
                url: transfers_url,
                icon: "fa-exchange".to_string(),
            },
        ],
    });

    sections
}

pub fn player_menu(neighbor_teams: &[(&str, &str)], team_slug: &str, current_path: &str) -> Vec<MenuSection> {
    team_menu(neighbor_teams, team_slug, current_path)
}

#[allow(dead_code)]
pub fn match_menu() -> Vec<MenuSection> {
    vec![MenuSection {
        items: vec![MenuItem {
            title: "Home".to_string(),
            url: "/".to_string(),
            icon: "fa-home".to_string(),
            active: false,
        }],
    }]
}

pub struct MenuSection {
    pub items: Vec<MenuItem>,
}

pub struct MenuItem {
    pub title: String,
    pub url: String,
    pub icon: String,
}

pub fn home_menu() -> Vec<MenuSection> {
    vec![MenuSection {
        items: vec![MenuItem {
            title: "Home".to_string(),
            url: "/".to_string(),
            icon: "fa-home".to_string(),
        }],
    }]
}

pub fn country_menu() -> Vec<MenuSection> {
    vec![MenuSection {
        items: vec![MenuItem {
            title: "Home".to_string(),
            url: "/".to_string(),
            icon: "fa-home".to_string(),
        }],
    }]
}

pub fn league_menu(country_name: &str, country_slug: &str, league_name: &str, league_slug: &str) -> Vec<MenuSection> {
    vec![
        MenuSection {
            items: vec![MenuItem {
                title: "Home".to_string(),
                url: "/".to_string(),
                icon: "fa-home".to_string(),
            }],
        },
        MenuSection {
            items: vec![MenuItem {
                title: country_name.to_string(),
                url: format!("/countries/{}", country_slug),
                icon: "fa-home".to_string(),
            }],
        },
        MenuSection {
            items: vec![
                MenuItem {
                    title: league_name.to_string(),
                    url: format!("/leagues/{}", league_slug),
                    icon: "fa-home".to_string(),
                },
                MenuItem {
                    title: "Transfers".to_string(),
                    url: format!("/leagues/{}/transfers", league_slug),
                    icon: "fa-exchange".to_string(),
                },
            ],
        },
    ]
}

pub fn team_menu(neighbor_teams: &[(&str, &str)], team_slug: &str) -> Vec<MenuSection> {
    let mut sections = vec![
        MenuSection {
            items: vec![MenuItem {
                title: "Home".to_string(),
                url: "/".to_string(),
                icon: "fa-home".to_string(),
            }],
        },
    ];

    if !neighbor_teams.is_empty() {
        sections.push(MenuSection {
            items: neighbor_teams
                .iter()
                .map(|(name, slug)| MenuItem {
                    title: name.to_string(),
                    url: format!("/teams/{}", slug),
                    icon: "fa-light fa-people-group".to_string(),
                })
                .collect(),
        });
    }

    sections.push(MenuSection {
        items: vec![
            MenuItem {
                title: "Tactics".to_string(),
                url: format!("/teams/{}/tactics", team_slug),
                icon: "fa-chess".to_string(),
            },
            MenuItem {
                title: "Schedule".to_string(),
                url: format!("/teams/{}/schedule", team_slug),
                icon: "fa-calendar".to_string(),
            },
            MenuItem {
                title: "Transfers".to_string(),
                url: format!("/teams/{}/transfers", team_slug),
                icon: "fa-exchange".to_string(),
            },
        ],
    });

    sections
}

pub fn player_menu(neighbor_teams: &[(&str, &str)], team_slug: &str) -> Vec<MenuSection> {
    team_menu(neighbor_teams, team_slug)
}

#[allow(dead_code)]
pub fn match_menu() -> Vec<MenuSection> {
    vec![MenuSection {
        items: vec![MenuItem {
            title: "Home".to_string(),
            url: "/".to_string(),
            icon: "fa-home".to_string(),
        }],
    }]
}

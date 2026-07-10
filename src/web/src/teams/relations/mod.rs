pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::Player;
use core::SimulatorData;
use core::Team;
use core::TeamType;
use serde::Deserialize;
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Deserialize)]
pub struct TeamRelationsGetRequest {
    lang: String,
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/relations/index.html")]
pub struct TeamRelationsTemplate {
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
    /// Player nodes that take part in at least one good/hate edge.
    pub nodes: Vec<RelNode>,
    /// Edges serialised for the client-side force layout
    /// (`[{s,t,k,p,w}, …]`). Empty `[]` when there are no relations.
    pub edges_json: String,
    /// Summary chip counts by tier.
    pub bond_count: usize,
    pub friendly_count: usize,
    pub tension_count: usize,
    pub rivalry_count: usize,
}

pub struct RelNode {
    pub player_id: u32,
    pub slug: String,
    pub last_name: String,
    pub is_generated: bool,
    pub is_goalkeeper: bool,
}

/// One edge in the client payload. Kept terse (`s`/`t`/`k`/`p`/`w`) because
/// it's inlined into the page as JSON and read straight back by the layout
/// script — no need for verbose keys on the wire.
#[derive(Serialize)]
struct EdgeJson {
    /// source node index
    s: usize,
    /// target node index
    t: usize,
    /// tier: "bond" | "friendly" | "tension" | "rivalry"
    k: &'static str,
    /// polarity: 1 = positive, -1 = negative
    p: i8,
    /// 0.15..1.0 line weight, from the combined relationship strength
    w: f32,
}

pub async fn team_relations_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamRelationsGetRequest>,
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

    let league = team.league_id.and_then(|id| simulator_data.league(id));

    let graph = RelationsGraph::build(team);

    let neighborhood = TeamNeighborhood::for_club(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighborhood
        .teams
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let league_refs: Vec<(&str, &str)> = neighborhood
        .leagues
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();

    let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
    let current_path = format!("/{}/teams/{}/relations", &route_params.lang, &team.slug);
    let menu_params = views::MenuParams {
        i18n: &i18n,
        lang: &route_params.lang,
        current_path: &current_path,
        country_name: cn,
        country_slug: cs,
    };
    let menu_sections = views::team_menu(&menu_params, &neighbor_refs, &league_refs);
    let title = team.name.clone();
    let league_title = league
        .map(|l| views::league_display_name(l, &i18n, simulator_data))
        .unwrap_or_default();

    Ok(TeamRelationsTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league_title,
        sub_title_link: league
            .map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug))
            .unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.background.clone())
            .unwrap_or_default(),
        foreground_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.foreground.clone())
            .unwrap_or_default(),
        menu_sections,
        team_slug: team.slug.clone(),
        active_tab: "relations",
        show_finances_tab: team.team_type.is_own_team(),
        show_academy_tab: team.team_type == TeamType::Main || team.team_type == TeamType::U18,
        edges_json: serde_json::to_string(&graph.edges).unwrap_or_else(|_| "[]".to_string()),
        bond_count: graph.bond_count,
        friendly_count: graph.friendly_count,
        tension_count: graph.tension_count,
        rivalry_count: graph.rivalry_count,
        nodes: graph.nodes,
    })
}

/// Squad social graph: nodes are players who take part in at least one
/// strong (good or hostile) relationship, edges are those relationships.
///
/// Relations are stored per-player and are directional — A's view of B can
/// differ from B's view of A. For an undirected "who-likes-whom" map we fold
/// both directions into a single edge, averaging the levels and OR-ing the
/// rivalry flag, then bucket the result into a tier. Only relationships
/// between two *current* squad members are considered so every node resolves
/// to a real photo + slug.
struct RelationsGraph {
    nodes: Vec<RelNode>,
    edges: Vec<EdgeJson>,
    bond_count: usize,
    friendly_count: usize,
    tension_count: usize,
    rivalry_count: usize,
}

/// Combined level at/above which a positive relationship is a warm friendship.
const FRIENDLY_FLOOR: f32 = 20.0;
/// …and at/above which it's a close bond.
const BOND_FLOOR: f32 = 55.0;
/// Combined level at/below which a negative relationship shows as tension.
const TENSION_CEIL: f32 = -20.0;
/// …and at/below which (or when a rivalry is flagged) it's an open rivalry.
const RIVALRY_CEIL: f32 = -55.0;

impl RelationsGraph {
    fn build(team: &Team) -> Self {
        let players: Vec<&Player> = team.players();
        let team_ids: HashSet<u32> = players.iter().map(|p| p.id).collect();

        // Fold both directions of each relationship into one entry keyed by
        // the ordered (low_id, high_id) pair: (level_sum, samples, rivalry).
        let mut pairs: HashMap<(u32, u32), (f32, u32, bool)> = HashMap::new();
        for player in &players {
            let owner = player.id;
            for (target_id, rel) in player.relations().player_relations_iter() {
                let target = *target_id;
                if target == owner || !team_ids.contains(&target) {
                    continue;
                }
                let key = if owner < target {
                    (owner, target)
                } else {
                    (target, owner)
                };
                let entry = pairs.entry(key).or_insert((0.0, 0, false));
                entry.0 += rel.level;
                entry.1 += 1;
                if !rel.rivalry_with.is_empty() {
                    entry.2 = true;
                }
            }
        }

        // Classify each folded pair into a tier; drop neutral pairs.
        struct RawEdge {
            a: u32,
            b: u32,
            kind: &'static str,
            polarity: i8,
            weight: f32,
        }
        let mut raw: Vec<RawEdge> = Vec::new();
        let (mut bond_count, mut friendly_count, mut tension_count, mut rivalry_count) =
            (0usize, 0usize, 0usize, 0usize);

        for ((a, b), (sum, count, rivalry)) in pairs {
            let combined = sum / count.max(1) as f32;
            let (kind, polarity) = if rivalry || combined <= RIVALRY_CEIL {
                rivalry_count += 1;
                ("rivalry", -1i8)
            } else if combined <= TENSION_CEIL {
                tension_count += 1;
                ("tension", -1)
            } else if combined >= BOND_FLOOR {
                bond_count += 1;
                ("bond", 1)
            } else if combined >= FRIENDLY_FLOOR {
                friendly_count += 1;
                ("friendly", 1)
            } else {
                continue;
            };
            let weight = (combined.abs() / 100.0).clamp(0.15, 1.0);
            raw.push(RawEdge {
                a,
                b,
                kind,
                polarity,
                weight,
            });
        }

        // Node set = players touched by a kept edge, in stable squad order so
        // the layout is reproducible across reloads.
        let mut degree: HashMap<u32, usize> = HashMap::new();
        for e in &raw {
            *degree.entry(e.a).or_insert(0) += 1;
            *degree.entry(e.b).or_insert(0) += 1;
        }

        let mut index_of: HashMap<u32, usize> = HashMap::new();
        let mut nodes: Vec<RelNode> = Vec::new();
        for player in &players {
            if !degree.contains_key(&player.id) {
                continue;
            }
            index_of.insert(player.id, nodes.len());
            nodes.push(RelNode {
                player_id: player.id,
                slug: player.slug(),
                last_name: player.full_name.display_last_name().to_string(),
                is_generated: player.is_generated(),
                is_goalkeeper: player.positions.is_goalkeeper(),
            });
        }

        let edges: Vec<EdgeJson> = raw
            .iter()
            .filter_map(|e| {
                Some(EdgeJson {
                    s: *index_of.get(&e.a)?,
                    t: *index_of.get(&e.b)?,
                    k: e.kind,
                    p: e.polarity,
                    w: (e.weight * 100.0).round() / 100.0,
                })
            })
            .collect();

        RelationsGraph {
            nodes,
            edges,
            bond_count,
            friendly_count,
            tension_count,
            rivalry_count,
        }
    }
}

struct TeamNeighborhood {
    teams: Vec<(String, String)>,
    leagues: Vec<(String, String)>,
}

impl TeamNeighborhood {
    fn for_club(club_id: u32, data: &SimulatorData, i18n: &I18n) -> Result<Self, ApiError> {
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

        Ok(TeamNeighborhood {
            teams,
            leagues: country_leagues
                .into_iter()
                .map(|(_, name, slug)| (name, slug))
                .collect(),
        })
    }
}

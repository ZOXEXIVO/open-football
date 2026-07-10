pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::Player;
use core::PlayerFieldPositionGroup;
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
    /// Formation row for the layered layout: 0 = GK, 1 = DEF, 2 = MID,
    /// 3 = FWD. The client pins each node to its row band and lets it slide
    /// only sideways, so the social web reads as the squad's shape.
    pub row: u8,
    /// True only for the subject of a player-centric (ego) graph — the
    /// client lifts this node into its own band above the formation rows.
    /// Always false in the whole-squad team graph.
    pub is_root: bool,
}

/// One edge in the client payload. Kept terse (`s`/`t`/`k`/`p`/`w`) because
/// it's inlined into the page as JSON and read straight back by the layout
/// script — no need for verbose keys on the wire.
#[derive(Serialize)]
pub(crate) struct EdgeJson {
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

    // Pool every squad that shares this team's dressing-room web, so the
    // graph spans the whole collection (Main + Reserve, or the older-youth
    // sides together) rather than a single registered squad.
    let pool = RelationsGroup::collect_pool(team, simulator_data);

    let graph = RelationsGraph::build(&pool);

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

/// Which club teams share one dressing-room relations web.
///
/// The graph is drawn per *collection*, not per registered squad, so the
/// social web spans everyone who trains together:
/// - `Senior` — the first team plus the brand-sharing `Reserve` side.
/// - `Youth` — the older academy squads (U19..U23) pooled into one web.
/// - `Solo` — U18, and the senior reserve sides that carry their own brand
///   (`B` / `Second`), each keep a self-contained collection.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelationsGroup {
    Senior,
    Youth,
    Solo(TeamType),
}

impl RelationsGroup {
    fn of(team_type: TeamType) -> Self {
        match team_type {
            TeamType::Main | TeamType::Reserve => RelationsGroup::Senior,
            TeamType::U19 | TeamType::U20 | TeamType::U21 | TeamType::U23 => RelationsGroup::Youth,
            // U18, B, Second — one collection each.
            other => RelationsGroup::Solo(other),
        }
    }

    /// Pool every squad that shares `team`'s dressing-room web into one
    /// player list, deduped by id. Shared by the whole-squad team graph and
    /// the player-centric ego graph so both draw from the same collection
    /// (Main + Reserve, the older-youth sides together, or a self-contained
    /// B / Second / U18 side). Falls back to the single team when the club
    /// can't be resolved.
    pub(crate) fn collect_pool<'a>(team: &'a Team, data: &'a SimulatorData) -> Vec<&'a Player> {
        let group = RelationsGroup::of(team.team_type);
        let mut seen_ids: HashSet<u32> = HashSet::new();
        let mut pool: Vec<&Player> = Vec::new();
        match data.club(team.club_id) {
            Some(club) => {
                for sibling in &club.teams.teams {
                    if RelationsGroup::of(sibling.team_type) != group {
                        continue;
                    }
                    for player in sibling.players() {
                        if seen_ids.insert(player.id) {
                            pool.push(player);
                        }
                    }
                }
            }
            None => {
                for player in team.players() {
                    if seen_ids.insert(player.id) {
                        pool.push(player);
                    }
                }
            }
        }
        pool
    }
}

/// Squad social graph: nodes are players who take part in at least one
/// strong (good or hostile) relationship, edges are those relationships.
///
/// Relations are stored per-player and are directional — A's view of B can
/// differ from B's view of A. For an undirected "who-likes-whom" map we fold
/// both directions into a single edge, averaging the levels and OR-ing each
/// side's open-rivalry state, then bucket the result into a tier. Only relationships
/// between two *current* squad members are considered so every node resolves
/// to a real photo + slug.
pub(crate) struct RelationsGraph {
    pub(crate) nodes: Vec<RelNode>,
    pub(crate) edges: Vec<EdgeJson>,
    pub(crate) bond_count: usize,
    pub(crate) friendly_count: usize,
    pub(crate) tension_count: usize,
    pub(crate) rivalry_count: usize,
}

/// A folded, tier-classified relationship between two pooled players, before
/// it's turned into node-index edge payload.
struct RawEdge {
    a: u32,
    b: u32,
    kind: &'static str,
    polarity: i8,
    weight: f32,
}

/// Combined level at/above which a positive relationship is a warm friendship.
const FRIENDLY_FLOOR: f32 = 20.0;
/// …and at/above which it's a close bond.
const BOND_FLOOR: f32 = 55.0;
/// Combined level at/below which a negative relationship shows as tension.
const TENSION_CEIL: f32 = -20.0;
/// …and at/below which (or when either side's feud is declared) it's an open rivalry.
const RIVALRY_CEIL: f32 = -55.0;

impl RelationsGraph {
    /// Whole-squad graph: every kept-tier relationship between two pooled
    /// players, and every player touched by one becomes a node.
    pub(crate) fn build(players: &[&Player]) -> Self {
        let raw = Self::classify(Self::fold_pairs(players));

        // Node set = players touched by a kept edge.
        let mut touched: HashSet<u32> = HashSet::new();
        for e in &raw {
            touched.insert(e.a);
            touched.insert(e.b);
        }

        Self::assemble(players, raw, &touched, None)
    }

    /// Player-centric (ego) graph: the subject plus every teammate they share
    /// a kept-tier relationship with. Edges among those neighbours are kept
    /// too, so the subject's corner of the dressing room reads in full, but
    /// nobody outside their direct circle is drawn. The subject is flagged
    /// `is_root` so the client lifts it onto its own band above the formation.
    pub(crate) fn build_ego(players: &[&Player], root_id: u32) -> Self {
        let raw = Self::classify(Self::fold_pairs(players));

        // Keep the root and the other endpoint of every kept edge touching it.
        let mut keep: HashSet<u32> = HashSet::new();
        keep.insert(root_id);
        for e in &raw {
            if e.a == root_id {
                keep.insert(e.b);
            } else if e.b == root_id {
                keep.insert(e.a);
            }
        }

        // Drop neighbour-to-outsider threads: keep an edge only when both
        // ends are the root or one of its direct neighbours.
        let raw: Vec<RawEdge> = raw
            .into_iter()
            .filter(|e| keep.contains(&e.a) && keep.contains(&e.b))
            .collect();

        Self::assemble(players, raw, &keep, Some(root_id))
    }

    /// Fold both directions of every relationship between two pooled players
    /// into one entry keyed by the ordered (low_id, high_id) pair:
    /// (level_sum, samples, any-side-open-rivalry).
    fn fold_pairs(players: &[&Player]) -> HashMap<(u32, u32), (f32, u32, bool)> {
        let team_ids: HashSet<u32> = players.iter().map(|p| p.id).collect();
        let mut pairs: HashMap<(u32, u32), (f32, u32, bool)> = HashMap::new();
        for player in players {
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
                if rel.is_open_rivalry() {
                    entry.2 = true;
                }
            }
        }
        pairs
    }

    /// Bucket each folded pair into a tier, dropping neutral pairs.
    fn classify(pairs: HashMap<(u32, u32), (f32, u32, bool)>) -> Vec<RawEdge> {
        let mut raw: Vec<RawEdge> = Vec::new();
        for ((a, b), (sum, count, rivalry)) in pairs {
            let combined = sum / count.max(1) as f32;
            let (kind, polarity) = if rivalry || combined <= RIVALRY_CEIL {
                ("rivalry", -1i8)
            } else if combined <= TENSION_CEIL {
                ("tension", -1)
            } else if combined >= BOND_FLOOR {
                ("bond", 1)
            } else if combined >= FRIENDLY_FLOOR {
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
        raw
    }

    /// Turn the kept raw edges into the final node list + client edge payload.
    /// Nodes are the pooled players whose id is in `keep`, in stable squad
    /// order; when `root_id` is set that player is emitted first (node 0) and
    /// flagged `is_root`. Tier counts are taken from the kept edges.
    fn assemble(
        players: &[&Player],
        raw: Vec<RawEdge>,
        keep: &HashSet<u32>,
        root_id: Option<u32>,
    ) -> Self {
        let (mut bond_count, mut friendly_count, mut tension_count, mut rivalry_count) =
            (0usize, 0usize, 0usize, 0usize);
        for e in &raw {
            match e.kind {
                "bond" => bond_count += 1,
                "friendly" => friendly_count += 1,
                "tension" => tension_count += 1,
                _ => rivalry_count += 1,
            }
        }

        let mut index_of: HashMap<u32, usize> = HashMap::new();
        let mut nodes: Vec<RelNode> = Vec::new();

        // Root first so it lands at node 0 and the client can pin it on top.
        if let Some(rid) = root_id {
            if let Some(player) = players.iter().find(|p| p.id == rid) {
                index_of.insert(rid, nodes.len());
                nodes.push(Self::node_for(player, true));
            }
        }
        for player in players {
            if Some(player.id) == root_id || !keep.contains(&player.id) {
                continue;
            }
            index_of.insert(player.id, nodes.len());
            nodes.push(Self::node_for(player, false));
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

    fn node_for(player: &Player, is_root: bool) -> RelNode {
        let row = match player.position().position_group() {
            PlayerFieldPositionGroup::Goalkeeper => 0u8,
            PlayerFieldPositionGroup::Defender => 1,
            PlayerFieldPositionGroup::Midfielder => 2,
            PlayerFieldPositionGroup::Forward => 3,
        };
        RelNode {
            player_id: player.id,
            slug: player.slug(),
            last_name: player.full_name.display_last_name().to_string(),
            is_generated: player.is_generated(),
            is_goalkeeper: player.positions.is_goalkeeper(),
            row,
            is_root,
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

use crate::ai::client::ToolSchema;
use core::SimulatorData;
use core::utils::DateUtils;
use serde_json::{Value, json};
use std::sync::Arc;

/// Executes the AI agent's tool calls against a snapshot of the simulator
/// world. Holds an `Arc<SimulatorData>` (cloned before the read lock is
/// released) so the slow agent loop never blocks the simulation.
pub struct AiTools {
    data: Arc<SimulatorData>,
}

impl AiTools {
    pub fn new(data: Arc<SimulatorData>) -> Self {
        AiTools { data }
    }

    /// OpenAI function-tool schemas advertised to the model.
    pub fn schemas() -> Vec<ToolSchema> {
        vec![
            ToolSchema {
                kind: "function",
                function: json!({
                    "name": "club_get_by_id",
                    "description": "Full club record — identity, finances, status, academy, facilities, rivals and the teams (Main/B/youth) it fields — as JSON for the given club id.",
                    "parameters": {
                        "type": "object",
                        "properties": { "club_id": { "type": "integer", "description": "numeric club id" } },
                        "required": ["club_id"]
                    }
                }),
            },
            ToolSchema {
                kind: "function",
                function: json!({
                    "name": "club_players",
                    "description": "The club's squad split by team. Each player has id, name, age, position, current ability (ca) and potential ability (pa) — no detailed skills.",
                    "parameters": {
                        "type": "object",
                        "properties": { "club_id": { "type": "integer", "description": "numeric club id" } },
                        "required": ["club_id"]
                    }
                }),
            },
            ToolSchema {
                kind: "function",
                function: json!({
                    "name": "player_get_by_id",
                    "description": "A single player's full record including all technical/mental/physical/goalkeeping skills and attributes, for the given player id.",
                    "parameters": {
                        "type": "object",
                        "properties": { "player_id": { "type": "integer", "description": "numeric player id" } },
                        "required": ["player_id"]
                    }
                }),
            },
        ]
    }

    /// Run a tool by name with its raw JSON argument string; always returns
    /// a JSON string (an `{"error": …}` object on any failure) so the agent
    /// loop can feed it straight back to the model.
    pub fn dispatch(&self, name: &str, arguments: &str) -> String {
        let args: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
        match name {
            "club_get_by_id" => match Self::u32_arg(&args, "club_id") {
                Some(id) => self.club_get_by_id(id),
                None => Self::error("missing or invalid club_id"),
            },
            "club_players" => match Self::u32_arg(&args, "club_id") {
                Some(id) => self.club_players(id),
                None => Self::error("missing or invalid club_id"),
            },
            "player_get_by_id" => match Self::u32_arg(&args, "player_id") {
                Some(id) => self.player_get_by_id(id),
                None => Self::error("missing or invalid player_id"),
            },
            other => Self::error(&format!("unknown tool '{other}'")),
        }
    }

    /// Accept the id as a JSON integer or a numeric string (models are
    /// inconsistent about which they emit).
    fn u32_arg(args: &Value, key: &str) -> Option<u32> {
        let raw = args.get(key)?;
        raw.as_u64()
            .or_else(|| raw.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
            .map(|n| n as u32)
    }

    fn error(message: &str) -> String {
        json!({ "error": message }).to_string()
    }

    fn club_get_by_id(&self, id: u32) -> String {
        let Some(club) = self.data.club(id) else {
            return Self::error("club not found");
        };
        let teams: Vec<Value> = club
            .teams
            .teams
            .iter()
            .map(|t| {
                json!({
                    "id": t.id,
                    "name": t.name,
                    "type": format!("{:?}", t.team_type),
                    "slug": t.slug,
                    "league_id": t.league_id,
                    "player_count": t.players.players.len(),
                    "reputation": format!("{:?}", t.reputation),
                })
            })
            .collect();
        json!({
            "id": club.id,
            "name": club.name,
            "philosophy": format!("{:?}", club.philosophy),
            "location": format!("{:?}", club.location),
            "colors": {
                "background": club.colors.background,
                "foreground": club.colors.foreground,
            },
            "status": format!("{:?}", club.status),
            "finance": format!("{:?}", club.finance),
            "facilities": format!("{:?}", club.facilities),
            "academy": format!("{:?}", club.academy),
            "rivals": club.rivals,
            "teams": teams,
        })
        .to_string()
    }

    fn club_players(&self, id: u32) -> String {
        let Some(club) = self.data.club(id) else {
            return Self::error("club not found");
        };
        let now = self.data.date.date();
        let teams: Vec<Value> = club
            .teams
            .teams
            .iter()
            .map(|t| {
                let players: Vec<Value> = t
                    .players
                    .players
                    .iter()
                    .map(|p| {
                        json!({
                            "id": p.id,
                            "name": p.full_name.to_string(),
                            "age": DateUtils::age(p.birth_date, now),
                            "position": p.positions.display_positions_compact(),
                            "ca": p.player_attributes.current_ability,
                            "pa": p.player_attributes.potential_ability,
                        })
                    })
                    .collect();
                json!({
                    "team_id": t.id,
                    "team_name": t.name,
                    "team_type": format!("{:?}", t.team_type),
                    "players": players,
                })
            })
            .collect();
        json!({ "club_id": club.id, "club_name": club.name, "teams": teams }).to_string()
    }

    fn player_get_by_id(&self, id: u32) -> String {
        let (player, team) = match self.data.player_with_team(id) {
            Some((p, t)) => (p, Some(t)),
            None => match self.data.player(id) {
                Some(p) => (p, None),
                None => return Self::error("player not found"),
            },
        };
        let now = self.data.date.date();
        json!({
            "id": player.id,
            "name": player.full_name.to_string(),
            "first_name": player.full_name.first_name,
            "last_name": player.full_name.last_name,
            "age": DateUtils::age(player.birth_date, now),
            "birth_date": player.birth_date.to_string(),
            "country_id": player.country_id,
            "team": team.map(|t| json!({ "id": t.id, "name": t.name, "type": format!("{:?}", t.team_type) })),
            "positions": player.positions.display_positions_compact(),
            "preferred_foot": format!("{:?}", player.preferred_foot),
            "current_ability": player.player_attributes.current_ability,
            "potential_ability": player.player_attributes.potential_ability,
            "attributes": serde_json::to_value(&player.player_attributes).unwrap_or(Value::Null),
            "skills": serde_json::to_value(&player.skills).unwrap_or(Value::Null),
            "personality": format!("{:?}", player.attributes),
        })
        .to_string()
    }
}

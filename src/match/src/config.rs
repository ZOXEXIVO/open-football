use bevy::prelude::{Color, Resource, Srgba};
use serde::Deserialize;

/// Everything the match page already knows and the viewer would otherwise have
/// to re-derive: which recording to stream, who is on the pitch, and in which
/// colours. Handed over as a JSON document when the page starts the viewer.
#[derive(Resource, Deserialize)]
pub struct ViewerConfig {
    /// CSS selector of the canvas element to render into.
    pub canvas: String,
    /// Prefix shared by the recording endpoints, e.g. `/api/match/1234`.
    pub api_base: String,
    /// Full-time timestamp of the recording, in milliseconds.
    pub match_time_ms: f64,
    pub home: TeamColors,
    pub away: TeamColors,
    pub players: Vec<PlayerInfo>,
    #[serde(default)]
    pub goals: Vec<GoalInfo>,
    /// Display strings, already translated by the page. Keeping them on this
    /// side of the boundary is what lets the viewer stay free of i18n.
    #[serde(default)]
    pub labels: ViewerLabels,
    /// Turns on the engine-facing overlays the `.dev/match` harness needs:
    /// per-player state names, a playback-speed control and a live readout of
    /// the ball's engine coordinates. Off for the game itself, where none of
    /// that means anything to a player.
    #[serde(default)]
    pub debug: bool,
}

#[derive(Deserialize)]
#[serde(default)]
pub struct ViewerLabels {
    pub first_half: String,
    pub second_half: String,
    pub loading: String,
}

impl Default for ViewerLabels {
    fn default() -> Self {
        ViewerLabels {
            first_half: "1st".to_string(),
            second_half: "2nd".to_string(),
            loading: "Loading match…".to_string(),
        }
    }
}

impl ViewerConfig {
    pub fn metadata_url(&self) -> String {
        format!("{}/metadata", self.api_base)
    }

    pub fn chunk_url(&self, index: usize) -> String {
        format!("{}/chunk/{}", self.api_base, index)
    }

    /// True when the goal was scored by the home side — an own goal counts for
    /// the opposing team, same rule the scoreboard uses.
    pub fn goal_belongs_to_home(&self, goal: &GoalInfo) -> bool {
        let scorer_is_home = self
            .players
            .iter()
            .find(|p| p.id == goal.player_id)
            .is_some_and(|p| p.is_home);
        scorer_is_home != goal.is_auto_goal
    }
}

#[derive(Deserialize)]
pub struct TeamColors {
    pub background: String,
    pub foreground: String,
}

impl TeamColors {
    pub fn background_color(&self, fallback: Color) -> Color {
        Self::parse(&self.background, fallback)
    }

    pub fn foreground_color(&self, fallback: Color) -> Color {
        Self::parse(&self.foreground, fallback)
    }

    fn parse(hex: &str, fallback: Color) -> Color {
        Srgba::hex(hex).map(Color::from).unwrap_or(fallback)
    }
}

#[derive(Deserialize)]
pub struct PlayerInfo {
    pub id: u32,
    pub shirt_number: u8,
    pub last_name: String,
    pub position: String,
    pub is_home: bool,
}

impl PlayerInfo {
    pub fn is_goalkeeper(&self) -> bool {
        self.position == "GK"
    }
}

#[derive(Deserialize)]
pub struct GoalInfo {
    pub player_id: u32,
    pub time: f64,
    #[serde(default)]
    pub is_auto_goal: bool,
}

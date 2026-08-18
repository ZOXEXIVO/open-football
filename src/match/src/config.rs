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
    pub no_recording: String,
}

impl Default for ViewerLabels {
    fn default() -> Self {
        ViewerLabels {
            first_half: "1st".to_string(),
            second_half: "2nd".to_string(),
            loading: "Loading match…".to_string(),
            no_recording: "Nothing was recorded in this match".to_string(),
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
    /// What he looks like: indices into `shared::Palette`'s three tables,
    /// decided from his nationality by the page that served this document.
    ///
    /// The viewer used to cut a complexion out of a hash of the player id,
    /// which put five tones through every squad in the world regardless of
    /// where its players were from — and gave the same man a different face
    /// here and on his profile page. It is not something this side can work
    /// out: it needs the country table, which never crosses into the browser.
    /// Defaulted only so a malformed document still fields a team.
    #[serde(default)]
    pub skin: u8,
    #[serde(default)]
    pub hair: u8,
    #[serde(default)]
    pub eyes: u8,
    /// Where his PHOTOGRAPH is, for the players who have one — the same head
    /// shot his profile page shows, which the viewer fetches once the match is
    /// on screen and lays over the front of his skull (see
    /// [`crate::portrait`]). Absent for a regen, who has never been
    /// photographed by anybody.
    #[serde(default)]
    pub photo: Option<String>,
    /// …and the DRAWN portrait, which every player has: the head his profile
    /// page shows when there is no photograph of him, asked for as a cutout.
    /// Tried when the photograph is missing or cannot be read.
    ///
    /// Whole URLs rather than ids to look up, so where this game keeps its
    /// pictures stays a decision of the page that serves it. Both absent is a
    /// legal document: the face this crate paints itself is what a player
    /// wears until something better arrives, and it is a face already.
    #[serde(default)]
    pub face: Option<String>,
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

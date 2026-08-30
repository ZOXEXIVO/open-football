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
    /// The near misses the match kept — two or three a side at most, each with
    /// a clip behind it in the recording. Absent on a document written before
    /// chances were recorded, which reads as a match of goals and grey.
    #[serde(default)]
    pub chances: Vec<ChanceInfo>,
    /// Every change either side made, each with a clip of its own in the
    /// recording — the twelve seconds the match stops for while one man walks
    /// off and another walks on. Absent on a document written before
    /// substitutions were played out, which reads as a match nobody changed.
    #[serde(default)]
    pub substitutions: Vec<SubstitutionInfo>,
    /// Display strings, already translated by the page. Keeping them on this
    /// side of the boundary is what lets the viewer stay free of i18n.
    #[serde(default)]
    pub labels: ViewerLabels,
    /// **The ground this was played at** — how much of one there is, and how
    /// many came to it.
    ///
    /// Absent on a document written before the stands answered to the
    /// fixture, which reads as a full-size ground with an ordinary gate in
    /// it: exactly the stadium every match used to be played in.
    #[serde(default)]
    pub venue: VenueInfo,
    /// Turns on the engine-facing overlays the `.dev/match` harness needs:
    /// per-player state names, a playback-speed control and a live readout of
    /// the ball's engine coordinates. Off for the game itself, where none of
    /// that means anything to a player.
    #[serde(default)]
    pub debug: bool,
    /// Whether to walk the two teams out before the replay starts — the line
    /// on the touchline, and the camera that goes down it. See
    /// [`Lineup`](crate::broadcast::lineup::Lineup).
    ///
    /// Absent means yes: every match gets its line-up, and a document written
    /// before there was one gets it too. The `.dev/match` harness is the only
    /// caller that ever turns it off, because fifteen seconds of ceremony in
    /// front of every run of a screenshot loop is fifteen seconds of ceremony.
    #[serde(default = "ViewerConfig::walked_out")]
    pub lineup: bool,
    /// The most frames a second the replay will draw. Zero means uncapped —
    /// the browser's own refresh rate is then the only ceiling.
    ///
    /// Absent means a hundred and twenty, which is the product's answer: a
    /// high-refresh panel keeps the extra smoothness of motion up to twice a
    /// broadcast frame rate, and the 240 Hz class stops burning double that
    /// again on pictures nobody can tell apart — which on a laptop is the
    /// fan coming on for a replay. A 60 Hz display never sees the cap at
    /// all; its own refresh is the tighter ceiling. The `.dev/match` harness
    /// passes zero, because it is the measuring instrument and a capped
    /// instrument reads the cap instead of the scene.
    #[serde(default = "ViewerConfig::high_refresh")]
    pub fps_cap: f32,
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

/// The ground, as the four facts that decide what is built round the pitch.
///
/// Every one of them is a FACT about the fixture rather than a decision about
/// the scene: how big a stand that comes to is
/// [`Stature`](crate::scene::crowd::Stature)'s to say, on this side, where the
/// stand is. The page that serves the document cannot know how many rows of
/// concrete a twelve-thousand-seat ground is, and should not have to.
#[derive(Deserialize)]
#[serde(default)]
pub struct VenueInfo {
    /// What the home club's ground holds. Never zero in the game — the
    /// simulator seeds a capacity for every club that has no recorded one —
    /// but zero is read here as "nobody said" and falls back to a full-size
    /// stadium.
    pub capacity: u32,
    /// What it typically draws. Zero where nobody has ever counted, which is
    /// read as an ordinary gate rather than as an empty ground.
    pub attendance: u32,
    /// World reputation of the side whose ground it is, on the simulator's
    /// 0..10_000 scale.
    pub reputation: u16,
    /// …and of the side visiting it.
    ///
    /// Who is coming is half of what decides whether a ground fills. Nobody
    /// buys a ticket to watch the home team in the abstract, and the same
    /// stand is three quarters empty for a midweek game against the bottom
    /// club and full for the one that matters.
    pub visitor: u16,
    /// Whether this is an age-restricted fixture. A club's under-18s play at
    /// the training ground whoever their parent club is, and Manchester
    /// United's youth team does not fill Old Trafford.
    pub youth: bool,
}

impl Default for VenueInfo {
    /// A great ground, comfortably full — which is what the viewer built for
    /// every match before there was anything to say about the venue, and so
    /// what a document written before this field gets.
    fn default() -> Self {
        VenueInfo {
            capacity: 60_000,
            attendance: 50_000,
            reputation: 10_000,
            visitor: 10_000,
            youth: false,
        }
    }
}

impl ViewerConfig {
    /// The default for [`Self::lineup`], which serde wants as a function.
    fn walked_out() -> bool {
        true
    }

    fn high_refresh() -> f32 {
        120.0
    }

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

    /// True when the home side created the chance. No own-goal twist here: a
    /// chance belongs to whoever struck the ball, and nobody attacks their own
    /// net on purpose.
    pub fn chance_belongs_to_home(&self, chance: &ChanceInfo) -> bool {
        self.players
            .iter()
            .find(|p| p.id == chance.player_id)
            .is_some_and(|p| p.is_home)
    }

    /// True when it was the home side that made the change. Read off the man
    /// coming ON — he is the one the document is guaranteed to carry, because
    /// a substitute who never played is still in the squad list.
    pub fn substitution_belongs_to_home(&self, change: &SubstitutionInfo) -> bool {
        self.players
            .iter()
            .find(|p| p.id == change.player_in_id)
            .is_some_and(|p| p.is_home)
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
    /// **Whether he was on the team sheet rather than the bench.**
    ///
    /// Only the eleven walk out before kick-off, and the recording cannot say
    /// which eleven those were: on a goals-only recording a starter who came
    /// off before the first goal has no track in the document at all, and a
    /// substitute who came on before it has one that opens at the same instant
    /// as everybody else's. Absent on a document written before there was a
    /// line-up, which [`Lineup`](crate::broadcast::lineup::Lineup) reads as
    /// "take the first eleven of each side" — the order both producers write
    /// them in.
    #[serde(default)]
    pub starting: bool,
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
    /// [`crate::players::portrait`]). Absent for a regen, who has never been
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

#[derive(Deserialize)]
pub struct ChanceInfo {
    pub player_id: u32,
    pub time: f64,
}

#[derive(Deserialize)]
pub struct SubstitutionInfo {
    pub player_in_id: u32,
    /// And the man he replaced. **Nothing is ever pointed at him** — the shot
    /// is of the substitute, from his face round to the name on his back and
    /// then out onto the pitch behind him.
    ///
    /// [`ChangeoverShot`](crate::broadcast::changeover::ChangeoverShot) wants
    /// him anyway, to leave him OUT of the sight-line test: he and the man
    /// replacing him are the only bodies on the ground that move while a change
    /// is being played out, and a lens that gave way to them would lurch
    /// through somebody else's close-up. Zero on a document written before the
    /// recording carried him, which costs the shot nothing.
    #[serde(default)]
    pub player_out_id: u32,
    pub time: f64,
    /// How long the match stopped for while this change was played out, in
    /// ms — the engine's window closes on the tick the last man reaches his
    /// slot, so it is nine or ten seconds for an ordinary change and nearly
    /// twice that when somebody has the width of the pitch to cross.
    ///
    /// [`ChangeoverShot`](crate::broadcast::changeover::ChangeoverShot) holds
    /// its pitch-side camera for exactly this long. Zero on a document written
    /// before the change was played out at all, which the shot reads as "use
    /// your own constant".
    #[serde(default)]
    pub break_ms: u64,
}

#[cfg(test)]
impl ViewerConfig {
    /// A document with nothing in it but a squad — what the parts of the
    /// viewer that only ever read the team sheets are checked against.
    pub fn of_players(players: Vec<PlayerInfo>) -> ViewerConfig {
        ViewerConfig {
            canvas: String::new(),
            api_base: String::new(),
            match_time_ms: 0.0,
            home: TeamColors {
                background: "#ffffff".to_string(),
                foreground: "#000000".to_string(),
            },
            away: TeamColors {
                background: "#000000".to_string(),
                foreground: "#ffffff".to_string(),
            },
            players,
            goals: Vec::new(),
            chances: Vec::new(),
            substitutions: Vec::new(),
            labels: ViewerLabels::default(),
            venue: VenueInfo::default(),
            debug: false,
            lineup: true,
            fps_cap: Self::high_refresh(),
        }
    }
}

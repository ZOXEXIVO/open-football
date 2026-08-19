use nalgebra::Vector3;
use serde::Serialize;
use serde::Serializer;
use serde::ser::{SerializeMap, SerializeSeq};
use std::collections::{HashMap, VecDeque};
use std::fmt::Display;

#[derive(Debug, Clone, Serialize)]
pub struct PassEventData {
    pub timestamp: u64,
    pub from_player_id: u32,
    pub to_player_id: u32,
}

impl PassEventData {
    pub fn new(timestamp: u64, from_player_id: u32, to_player_id: u32) -> Self {
        PassEventData {
            timestamp,
            from_player_id,
            to_player_id,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchEventData {
    pub timestamp: u64,
    pub category: String,
    pub description: String,
}

/// Position data item — stored in memory as full-precision values,
/// serialized as compact JSON arrays: [timestamp, x, y, z] or [timestamp, x, y]
#[derive(Debug, Clone)]
pub struct ResultPositionDataItem {
    pub timestamp: u64,
    pub position: Vector3<f32>,
}

impl ResultPositionDataItem {
    pub fn new(timestamp: u64, position: Vector3<f32>) -> Self {
        ResultPositionDataItem {
            timestamp,
            position,
        }
    }
}

/// Compact serialization: [timestamp, x, y] or [timestamp, x, y, z]
/// Omits z when it's effectively zero (players on ground), saving ~5 bytes/entry in JSON.
impl Serialize for ResultPositionDataItem {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Rounded per axis for compact JSON output — see `Quantize` for why
        // the vertical one does not share the horizontal step.
        let x = Quantize::ground(self.position.x);
        let y = Quantize::ground(self.position.y);
        let z = Quantize::height(self.position.z);

        if z.abs() < Quantize::GROUNDED {
            // 2D entry: [timestamp, x, y]
            let mut seq = serializer.serialize_seq(Some(3))?;
            seq.serialize_element(&self.timestamp)?;
            seq.serialize_element(&x)?;
            seq.serialize_element(&y)?;
            seq.end()
        } else {
            // 3D entry: [timestamp, x, y, z]
            let mut seq = serializer.serialize_seq(Some(4))?;
            seq.serialize_element(&self.timestamp)?;
            seq.serialize_element(&x)?;
            seq.serialize_element(&y)?;
            seq.serialize_element(&z)?;
            seq.end()
        }
    }
}

/// Tolerance-based squared distance threshold for deduplication.
/// Positions within 0.3 game units are considered unchanged.
/// 0.3 units on an 840-unit field = 0.036% — completely imperceptible.
///
/// Game units on every axis, including the vertical one, which is stored in
/// metres and has to be converted before it is compared — see
/// [`Quantize::separation_sq`]. 0.3 u is 3.75 cm, so the ball needs to be
/// moving faster than about 1.25 m/s to clear it in a 30 ms sample, whichever
/// direction it is moving in.
const DEDUP_TOLERANCE_SQ: f32 = 0.09; // 0.3 * 0.3

/// Maximum interval between recorded samples for any on-pitch player.
/// A stationary GK or sweeper could otherwise go minutes without a new
/// sample (dedup threshold never tripped). Replay viewers use the gap
/// between samples as a "player left the pitch" signal.
///
/// MUST stay below the viewer's hide-on-gap threshold (1000 ms at the
/// time of writing). At the old 2 s value, any stationary player got a
/// sample at t=0, then none until t=2000 — but the viewer hid them the
/// moment `time > lastTs + 1000`, so the player blinked invisible for
/// half of every 2-second window. Noticeable as "players disappearing
/// a few minutes into the match", especially once the NaN-velocity
/// guard started silencing state bugs by zeroing velocity (which
/// left those players perfectly stationary and fully exposed to the
/// blink). 750 ms keeps them continuously visible with ~1 extra KB
/// of storage per idle player per minute — negligible.
const HEARTBEAT_INTERVAL_MS: u64 = 750;

/// How much of a match a recording keeps.
///
/// A full recording of one match is a few hundred kilobytes and ninety
/// minutes of samples for twenty-three entities — fine for the one fixture
/// the dev harness plays, ruinous for a world that plays thousands a season.
/// The game therefore records [`RecordingScope::Goals`]: the moments anybody
/// actually wants to watch, and nothing in between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingScope {
    /// Every sample, first whistle to last. What `.dev/match` needs — its
    /// analyses read the whole match back off the recording.
    Full,
    /// The highlights: [`GOAL_CLIP_PRE_ROLL_MS`] either side of each goal, and
    /// of each near miss the match sheet shortlisted (`HighlightSelector`).
    ///
    /// Named for the goals because they were once all of it. At the measured
    /// rate — 2.6 goals and 4.6 kept chances a match — that is about 72 seconds
    /// of a 5,400-second match, or a seventy-fifth of a full recording; goals
    /// alone were a two-hundredth. A nil-nil used to keep literally nothing and
    /// say so, and now has a reel like any other match.
    Goals,
}

impl RecordingScope {
    pub fn as_u8(self) -> u8 {
        match self {
            RecordingScope::Full => 0,
            RecordingScope::Goals => 1,
        }
    }

    /// Anything unrecognised is `Full` — the scope that loses no data.
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => RecordingScope::Goals,
            _ => RecordingScope::Full,
        }
    }
}

/// How long before a goal — or a chance — a clip starts.
///
/// Five seconds is the build-up you need for the moment to make sense: the pass
/// that released the run, the cross, the save that fell to somebody. Less and
/// the ball simply appears in the box.
pub const GOAL_CLIP_PRE_ROLL_MS: u64 = 5_000;

/// How long after it a clip runs.
///
/// The ball settling in the net and the first of the celebration — the engine
/// plays that out rather than teleporting everyone back (see
/// `handle_goal_reset`), so there is something there to keep. A chance gets the
/// same five seconds, which is what carries the save, the rebound and whatever
/// the follow-up was.
pub const GOAL_CLIP_POST_ROLL_MS: u64 = 5_000;

/// Rounding applied to a recorded coordinate, once on the way into the buffer
/// and again on the way out to JSON.
///
/// **The two axes are not the same size.** `x` and `y` are game units of
/// 0.125 m; `z` is metres, because the engine's vertical axis is metric (see
/// `GRAVITY_PER_TICK` — the crossbar is 2.44 and a jump reaches 3.5). One
/// shared step of 0.1 therefore bought 1.25 cm of resolution across the pitch
/// and 10 cm of it up the pitch, and the ball was recorded to the nearest ten
/// centimetres of height.
///
/// That is coarser than most of the things height is used to express. There
/// was nothing at all between "on the deck" and "ten centimetres up": a 4 cm
/// bounce rounded to zero and was written down as a ball that never left the
/// ground, a 6 cm one rounded up to 0.1 and was written down as half again as
/// high as it was, and a driven shot crossing the box climbed in visible 10 cm
/// steps. `Ball::carry_height`, the 1.15 m a keeper holds it at, came out as
/// 1.1. None of it was wrong in the simulation — only in what was kept of it.
struct Quantize;

impl Quantize {
    /// Horizontal, in game units. 0.1 u = 1.25 cm.
    #[inline]
    fn ground(v: f32) -> f32 {
        (v * 10.0).round() / 10.0
    }

    /// Vertical, in metres. 0.01 m = 1 cm — a shade finer than the horizontal
    /// step in real terms, and still two decimal places rather than four,
    /// which is what keeps the JSON short.
    #[inline]
    fn height(v: f32) -> f32 {
        (v * 100.0).round() / 100.0
    }

    /// Under half a vertical step there is no height left to record and the
    /// serialiser drops the element entirely. Kept in lock-step with
    /// [`Quantize::height`]: a value this test lets through has to survive
    /// that rounding, or the wire carries an explicit zero.
    const GROUNDED: f32 = 0.005;

    /// Game units per metre.
    ///
    /// The dedup below measures a horizontal delta in units against a vertical
    /// one in metres, so one of them has to be converted or the comparison
    /// means nothing — and it did not: the 0.3 tolerance was 3.75 cm across
    /// the pitch and 30 cm up it. A ball dropping vertically out of the sky
    /// recorded no sample at all until it had fallen a third of a metre.
    const UNITS_PER_METRE: f32 = 8.0;

    /// A recorded position, rounded on the axis each coordinate belongs to.
    #[inline]
    fn position(position: Vector3<f32>) -> Vector3<f32> {
        Vector3::new(
            Self::ground(position.x),
            Self::ground(position.y),
            Self::height(position.z),
        )
    }

    /// Squared distance between two recorded positions, in game units on every
    /// axis, for comparison against [`DEDUP_TOLERANCE_SQ`].
    #[inline]
    fn separation_sq(a: Vector3<f32>, b: Vector3<f32>) -> f32 {
        let dx = a.x - b.x;
        let dy = a.y - b.y;
        let dz = (a.z - b.z) * Self::UNITS_PER_METRE;
        dx * dx + dy * dy + dz * dz
    }
}

/// Player state change: recorded only when the state actually changes.
/// Serializes as [timestamp, "StateName"] for compact JSON.
#[derive(Debug, Clone)]
pub struct PlayerStateEntry {
    pub timestamp: u64,
    pub state: String,
}

impl Serialize for PlayerStateEntry {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(2))?;
        seq.serialize_element(&self.timestamp)?;
        seq.serialize_element(&self.state)?;
        seq.end()
    }
}

#[derive(Debug, Clone)]
pub struct ResultMatchPositionData {
    ball: Vec<ResultPositionDataItem>,
    players: HashMap<u32, Vec<ResultPositionDataItem>>,
    passes: Vec<PassEventData>,
    events: Vec<MatchEventData>,
    /// Per-player state changes — only populated when track_events is true.
    player_states: HashMap<u32, Vec<PlayerStateEntry>>,
    /// Fast dedup: last recorded state compact ID per player (avoids String allocation)
    last_state_ids: HashMap<u32, u16>,
    track_events: bool,
    track_positions: bool,
    scope: RecordingScope,
    /// The rolling pre-roll, under [`RecordingScope::Goals`].
    ///
    /// A goal cannot be seen coming, so the seconds before one have to be held
    /// somewhere until it either happens — at which point they are promoted
    /// into `ball` / `players` — or ages out of the window and is dropped. It
    /// is always the TAIL of each stream, which is what lets the dedup below
    /// keep working: the last sample recorded for an entity is the back of its
    /// pending queue, or the end of its kept vector when that queue is empty.
    pending_ball: VecDeque<ResultPositionDataItem>,
    pending_players: HashMap<u32, VecDeque<ResultPositionDataItem>>,
    /// One entry per moment the engine asked to keep, in the order it asked.
    /// Goals are final; chances are provisional until [`Self::finish_retaining`]
    /// is told which of them the match sheet kept — see [`Clip`].
    clips: Vec<Clip>,
    /// Time ranges the recording actually covers, merged and in order.
    ///
    /// Derived from the surviving `clips` at full time rather than maintained
    /// as they arrive, because until then it is not known which chance clips
    /// survive. Only meaningful under [`RecordingScope::Goals`]; a full
    /// recording covers everything and reports `None` from
    /// [`Self::recorded_segments`].
    segments: Vec<(u64, u64)>,
    /// End of the clip currently being written, while there is one. Samples at
    /// or before it go straight into the kept vectors.
    capture_until: Option<u64>,
}

/// A stretch of match the recorder was asked to hold on to, and why.
///
/// The why matters because the two kinds are decided at different times. A goal
/// is a goal the instant the ball crosses the line and its clip is never in
/// doubt; a chance is only ever a candidate — whether it was one of the best
/// two or three its side had is a question about the whole match, which nobody
/// can answer while it is still being played. So chances are recorded
/// speculatively and the losers are thrown away at the whistle, which is also
/// the only moment the segment list can honestly be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Clip {
    /// The instant the clip is cut around — the ball crossing the line, or the
    /// ball being struck. Doubles as the clip's identity: the shortlist handed
    /// to [`ResultMatchPositionData::finish_retaining`] is a list of these.
    at: u64,
    start: u64,
    end: u64,
    kind: ClipKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipKind {
    Goal,
    Chance,
}

/// Compact top-level serialization.
/// Uses same key names as before for frontend compatibility.
impl Serialize for ResultMatchPositionData {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let has_states = self.track_events && !self.player_states.is_empty();
        let field_count =
            2 + if self.track_events { 2 } else { 0 } + if has_states { 1 } else { 0 };
        let mut map = serializer.serialize_map(Some(field_count))?;

        map.serialize_entry("ball", &self.ball)?;
        map.serialize_entry("players", &self.players)?;

        if self.track_events {
            map.serialize_entry("passes", &self.passes)?;
            map.serialize_entry("events", &self.events)?;
        }

        if has_states {
            map.serialize_entry("states", &self.player_states)?;
        }

        map.end()
    }
}

impl ResultMatchPositionData {
    /// Shared shape. Every recording starts [`RecordingScope::Full`] — the
    /// scope that keeps everything — and the game narrows it explicitly.
    fn base(track_events: bool, track_positions: bool) -> Self {
        let capacity = if track_positions { 44 } else { 0 };
        ResultMatchPositionData {
            ball: Vec::new(),
            players: HashMap::with_capacity(capacity),
            passes: Vec::new(),
            events: Vec::new(),
            player_states: HashMap::with_capacity(if track_events { capacity } else { 0 }),
            last_state_ids: HashMap::with_capacity(if track_events { capacity } else { 0 }),
            track_events,
            track_positions,
            scope: RecordingScope::Full,
            pending_ball: VecDeque::new(),
            pending_players: HashMap::new(),
            clips: Vec::new(),
            segments: Vec::new(),
            capture_until: None,
        }
    }

    pub fn new() -> Self {
        Self::base(false, true)
    }

    pub fn new_with_tracking() -> Self {
        Self::base(true, true)
    }

    pub fn empty() -> Self {
        Self::base(false, false)
    }

    /// Narrow what the recording keeps. See [`RecordingScope`].
    pub fn with_scope(mut self, scope: RecordingScope) -> Self {
        self.scope = scope;
        self
    }

    /// Build a coarse heatmap (bucket-count grid) for a single player from
    /// their recorded position samples. The output is a `rows x cols` grid,
    /// row-major, where each cell holds the number of position samples that
    /// fell into it. Caller supplies the field dimensions used when the
    /// match was simulated.
    ///
    /// Typical usage: 10×14 or 12×16 buckets is enough to render a readable
    /// FM-style player heatmap in the UI.
    pub fn player_heatmap(
        &self,
        player_id: u32,
        field_width: f32,
        field_height: f32,
        cols: usize,
        rows: usize,
    ) -> Vec<u32> {
        let mut grid = vec![0u32; cols * rows];
        let positions = match self.players.get(&player_id) {
            Some(p) if !p.is_empty() => p,
            _ => return grid,
        };

        let cw = field_width / cols as f32;
        let ch = field_height / rows as f32;
        if cw <= 0.0 || ch <= 0.0 {
            return grid;
        }
        for item in positions {
            let cx = (item.position.x / cw).floor() as isize;
            let cy = (item.position.y / ch).floor() as isize;
            if cx < 0 || cy < 0 {
                continue;
            }
            let cx = (cx as usize).min(cols - 1);
            let cy = (cy as usize).min(rows - 1);
            grid[cy * cols + cx] = grid[cy * cols + cx].saturating_add(1);
        }
        grid
    }

    /// Average position across all samples for a player, or None if no
    /// samples. Useful as the anchor point for an FM-style formation map.
    pub fn player_average_position(&self, player_id: u32) -> Option<(f32, f32)> {
        let positions = self.players.get(&player_id)?;
        if positions.is_empty() {
            return None;
        }
        let (sx, sy) = positions.iter().fold((0.0f32, 0.0f32), |(ax, ay), p| {
            (ax + p.position.x, ay + p.position.y)
        });
        let n = positions.len() as f32;
        Some((sx / n, sy / n))
    }

    /// Split the data into chunks based on time ranges
    /// Returns a vector of chunks, each containing data for a specific time window
    pub fn split_into_chunks(&self, chunk_duration_ms: u64) -> Vec<ResultMatchPositionData> {
        if self.ball.is_empty() {
            return vec![self.clone()];
        }

        let max_timestamp = self.max_timestamp();
        let num_chunks = ((max_timestamp as f64 / chunk_duration_ms as f64).ceil() as usize).max(1);
        let mut chunks = Vec::with_capacity(num_chunks);

        for chunk_idx in 0..num_chunks {
            let start_time = chunk_idx as u64 * chunk_duration_ms;
            let end_time = start_time + chunk_duration_ms;

            let mut chunk = Self::base(self.track_events, self.track_positions);

            // Filter ball positions for this time window
            chunk.ball = self
                .ball
                .iter()
                .filter(|item| item.timestamp >= start_time && item.timestamp < end_time)
                .cloned()
                .collect();

            // Filter player positions for this time window
            for (player_id, positions) in &self.players {
                let filtered_positions: Vec<ResultPositionDataItem> = positions
                    .iter()
                    .filter(|item| item.timestamp >= start_time && item.timestamp < end_time)
                    .cloned()
                    .collect();

                if !filtered_positions.is_empty() {
                    chunk.players.insert(*player_id, filtered_positions);
                }
            }

            // Filter passes and events for this time window
            if self.track_events {
                chunk.passes = self
                    .passes
                    .iter()
                    .filter(|pass| pass.timestamp >= start_time && pass.timestamp < end_time)
                    .cloned()
                    .collect();

                chunk.events = self
                    .events
                    .iter()
                    .filter(|evt| evt.timestamp >= start_time && evt.timestamp < end_time)
                    .cloned()
                    .collect();

                // Filter player states: include last state before chunk start + states in window
                for (player_id, states) in &self.player_states {
                    let mut chunk_states = Vec::new();

                    // Find the most recent state before this chunk starts (carry-over)
                    if let Some(last_before) =
                        states.iter().rev().find(|s| s.timestamp < start_time)
                    {
                        chunk_states.push(PlayerStateEntry {
                            timestamp: start_time,
                            state: last_before.state.clone(),
                        });
                    }

                    // Add states within this chunk's window
                    for s in states
                        .iter()
                        .filter(|s| s.timestamp >= start_time && s.timestamp < end_time)
                    {
                        chunk_states.push(s.clone());
                    }

                    if !chunk_states.is_empty() {
                        chunk.player_states.insert(*player_id, chunk_states);
                    }
                }
            }

            chunks.push(chunk);
        }

        chunks
    }

    /// Check if event tracking is enabled
    #[inline]
    pub fn is_tracking_events(&self) -> bool {
        self.track_events
    }

    /// Check if position tracking is enabled
    #[inline]
    pub fn is_tracking_positions(&self) -> bool {
        self.track_positions
    }

    /// Whether a sample taken at `timestamp` is being kept outright, rather
    /// than parked in the pre-roll to see whether a goal claims it.
    #[inline]
    fn capturing(&self, timestamp: u64) -> bool {
        match self.scope {
            RecordingScope::Full => true,
            RecordingScope::Goals => self.capture_until.is_some_and(|end| timestamp <= end),
        }
    }

    /// Should this sample be written down at all, given the last one recorded
    /// for the same entity?
    ///
    /// Tolerance dedup + heartbeat: skip tiny movements unless we're overdue
    /// for a sample. Without the heartbeat, a GK planted in the six-yard box
    /// gets no updates until a save, and replay viewers can't distinguish
    /// "on-pitch, idle" from "subbed off"; an owned-and-stationary ball
    /// freezes `max_timestamp` and the chunk split discards everything after
    /// it.
    ///
    /// The height delta is weighted into game units before the comparison.
    /// Unweighted, this is the test that lost every near-vertical ball in the
    /// recording: a drop, the top of a lob, a bounce coming up under a boot
    /// all sat inside the tolerance until they had travelled 30 cm.
    #[inline]
    fn worth_recording(
        last: Option<&ResultPositionDataItem>,
        timestamp: u64,
        position: Vector3<f32>,
    ) -> bool {
        let Some(last) = last else {
            return true;
        };
        let since_last = timestamp.saturating_sub(last.timestamp);
        Quantize::separation_sq(position, last.position) >= DEDUP_TOLERANCE_SQ
            || since_last >= HEARTBEAT_INTERVAL_MS
    }

    /// Drop everything at the front of a pre-roll queue that a goal happening
    /// *now* would no longer want.
    #[inline]
    fn expire(pending: &mut VecDeque<ResultPositionDataItem>, timestamp: u64) {
        let oldest = timestamp.saturating_sub(GOAL_CLIP_PRE_ROLL_MS);
        while pending.front().is_some_and(|item| item.timestamp < oldest) {
            pending.pop_front();
        }
    }

    /// Add player position with quantization and tolerance-based dedup.
    /// Skips recording if the player hasn't moved more than 0.3 units since last entry.
    pub fn add_player_positions(&mut self, player_id: u32, timestamp: u64, position: Vector3<f32>) {
        if !self.track_positions {
            return;
        }

        // Rounded per axis — reduces float noise and produces shorter JSON.
        let position = Quantize::position(position);

        if self.capturing(timestamp) {
            let kept = self.players.entry(player_id).or_default();
            if Self::worth_recording(kept.last(), timestamp, position) {
                kept.push(ResultPositionDataItem::new(timestamp, position));
            }
            return;
        }

        let pending = self.pending_players.entry(player_id).or_default();
        let last = pending
            .back()
            .or_else(|| self.players.get(&player_id).and_then(|kept| kept.last()));
        if Self::worth_recording(last, timestamp, position) {
            pending.push_back(ResultPositionDataItem::new(timestamp, position));
        }
        Self::expire(pending, timestamp);
    }

    /// Add ball position with quantization and tolerance-based dedup.
    /// Previous implementation had a bug: PartialEq compared timestamps too,
    /// so ball positions were NEVER deduplicated (timestamps always differ).
    pub fn add_ball_positions(&mut self, timestamp: u64, position: Vector3<f32>) {
        if !self.track_positions {
            return;
        }

        let position = Quantize::position(position);

        if self.capturing(timestamp) {
            if Self::worth_recording(self.ball.last(), timestamp, position) {
                self.ball
                    .push(ResultPositionDataItem::new(timestamp, position));
            }
            return;
        }

        let last = self.pending_ball.back().or_else(|| self.ball.last());
        if Self::worth_recording(last, timestamp, position) {
            self.pending_ball
                .push_back(ResultPositionDataItem::new(timestamp, position));
        }
        Self::expire(&mut self.pending_ball, timestamp);
    }

    /// The ball has crossed the line: keep this goal.
    ///
    /// Promotes the pre-roll already in hand and opens the clip's post-roll,
    /// after which sampling falls back to the rolling window. A second goal
    /// inside an open clip simply extends it — the two ranges merge rather
    /// than producing an overlapping pair the viewer would have to reconcile.
    ///
    /// A no-op under [`RecordingScope::Full`], where every sample is kept
    /// anyway and the whole match is one segment.
    pub fn mark_goal(&mut self, timestamp: u64) {
        self.open_clip(timestamp, ClipKind::Goal);
    }

    /// A shot worth calling a chance was just struck: keep it *for now*.
    ///
    /// Cut exactly like a goal, and speculative in one more way than a goal's
    /// pre-roll already is — this clip only survives if the strike turns out to
    /// be one of the best its side had, which is settled at the whistle by
    /// `HighlightSelector` and applied by [`Self::finish_retaining`]. Marking
    /// generously and pruning late is what buys a correctly-anchored clip:
    /// there is no way to open one five seconds before a shot that has already
    /// been taken.
    ///
    /// `timestamp` is the clip's identity as well as its centre — it has to be
    /// the same instant the match sheet stamps on the chance, or the shortlist
    /// and the footage stop lining up.
    pub fn mark_chance(&mut self, timestamp: u64) {
        self.open_clip(timestamp, ClipKind::Chance);
    }

    /// Keep the seconds either side of `timestamp`, and remember why.
    fn open_clip(&mut self, timestamp: u64, kind: ClipKind) {
        if !self.track_positions || self.scope != RecordingScope::Goals {
            return;
        }

        let start = timestamp.saturating_sub(GOAL_CLIP_PRE_ROLL_MS);
        let end = timestamp.saturating_add(GOAL_CLIP_POST_ROLL_MS);

        self.clips.push(Clip {
            at: timestamp,
            start,
            end,
            kind,
        });
        self.capture_until = Some(match self.capture_until {
            Some(open) => open.max(end),
            None => end,
        });

        while self
            .pending_ball
            .front()
            .is_some_and(|item| item.timestamp < start)
        {
            self.pending_ball.pop_front();
        }
        self.ball.extend(self.pending_ball.drain(..));

        for (player_id, pending) in self.pending_players.iter_mut() {
            while pending.front().is_some_and(|item| item.timestamp < start) {
                pending.pop_front();
            }
            if pending.is_empty() {
                continue;
            }
            self.players
                .entry(*player_id)
                .or_default()
                .extend(pending.drain(..));
        }
    }

    /// Full time, with no chance surviving. Goals only — what the recorder did
    /// before it kept anything else, and what a caller who never marked a
    /// chance gets either way.
    pub fn finish(&mut self, total_match_time: u64) {
        self.finish_retaining(total_match_time, &[]);
    }

    /// Full time. Releases the pre-roll — whatever is left in it belongs to no
    /// goal and never will — drops every chance clip the match sheet did not
    /// keep, and trims what remains back to the final whistle so the viewer
    /// doesn't grey in five seconds of injury time that never existed.
    ///
    /// `kept_chances` are the timestamps handed to [`Self::mark_chance`] that
    /// survived selection, exactly as `HighlightSelector::select` returned
    /// them. Everything else marked as a chance goes, along with the samples no
    /// surviving clip is still holding — which is the whole point of marking
    /// speculatively in the first place.
    pub fn finish_retaining(&mut self, total_match_time: u64, kept_chances: &[u64]) {
        self.pending_ball = VecDeque::new();
        self.pending_players = HashMap::new();
        self.capture_until = None;

        // A full recording has no clips and no segments — it is all one piece,
        // and there is nothing here to prune it against.
        if self.scope != RecordingScope::Goals {
            return;
        }

        self.clips
            .retain(|clip| clip.kind == ClipKind::Goal || kept_chances.contains(&clip.at));

        // Merge what is left into the segment list. Clips are marked in match
        // order, but two of them can still overlap — a goal off a rebound from
        // a chance five seconds earlier — and a viewer handed overlapping
        // ranges has to reconcile them.
        self.clips.sort_by_key(|clip| clip.start);
        self.segments = Vec::with_capacity(self.clips.len());
        for clip in &self.clips {
            let end = clip.end.min(total_match_time.max(clip.start));
            match self.segments.last_mut() {
                Some(last) if clip.start <= last.1 => last.1 = last.1.max(end),
                _ => self.segments.push((clip.start, end)),
            }
        }

        // And the samples themselves. Everything inside a dropped clip was kept
        // outright while it was open (there was no telling then that it would
        // be dropped), so the segment list is the only thing that knows which
        // of them still belong to the recording.
        let segments = &self.segments;
        let covered = |timestamp: u64| {
            segments
                .iter()
                .any(|(start, end)| timestamp >= *start && timestamp <= *end)
        };
        self.ball.retain(|item| covered(item.timestamp));
        self.players.retain(|_, samples| {
            samples.retain(|item| covered(item.timestamp));
            !samples.is_empty()
        });
    }

    /// The parts of the match this recording covers, or `None` when it covers
    /// all of it.
    ///
    /// An empty slice is the answer that matters most, because the viewer
    /// cannot work it out for itself: nothing was kept, so it should say so
    /// rather than wait forever for a chunk that was never written. Two
    /// different matches produce it — a goalless one under
    /// [`RecordingScope::Goals`], and one that was never sampled at all
    /// (`empty()`), which is what every match played on a remote worker looks
    /// like once the wire has dropped its position track.
    pub fn recorded_segments(&self) -> Option<&[(u64, u64)]> {
        if !self.track_positions {
            return Some(&[]);
        }
        match self.scope {
            RecordingScope::Full => None,
            RecordingScope::Goals => Some(&self.segments),
        }
    }

    /// Nothing was kept here. A goals-only recording produces one of these for
    /// every five-minute window without a goal in it, and there is no point
    /// writing the file.
    pub fn is_empty(&self) -> bool {
        self.ball.is_empty() && self.players.is_empty()
    }

    /// Get the maximum timestamp in the recorded data
    pub fn max_timestamp(&self) -> u64 {
        self.ball.last().map(|item| item.timestamp).unwrap_or(0)
    }

    /// Get ball position at a specific timestamp (uses nearest neighbor)
    pub fn get_ball_position_at(&self, timestamp: u64) -> Option<Vector3<f32>> {
        if self.ball.is_empty() {
            return None;
        }

        // Binary search for the closest timestamp
        let idx = self
            .ball
            .binary_search_by_key(&timestamp, |item| item.timestamp)
            .unwrap_or_else(|idx| {
                if idx == 0 {
                    0
                } else if idx >= self.ball.len() {
                    self.ball.len() - 1
                } else {
                    // Choose nearest between idx-1 and idx
                    let before = &self.ball[idx - 1];
                    let after = &self.ball[idx];
                    if timestamp - before.timestamp < after.timestamp - timestamp {
                        idx - 1
                    } else {
                        idx
                    }
                }
            });

        Some(self.ball[idx].position)
    }

    /// Get player position at a specific timestamp (uses nearest neighbor)
    pub fn get_player_position_at(&self, player_id: u32, timestamp: u64) -> Option<Vector3<f32>> {
        let player_data = self.players.get(&player_id)?;

        if player_data.is_empty() {
            return None;
        }

        // Binary search for the closest timestamp
        let idx = player_data
            .binary_search_by_key(&timestamp, |item| item.timestamp)
            .unwrap_or_else(|idx| {
                if idx == 0 {
                    0
                } else if idx >= player_data.len() {
                    player_data.len() - 1
                } else {
                    // Choose nearest between idx-1 and idx
                    let before = &player_data[idx - 1];
                    let after = &player_data[idx];
                    if timestamp - before.timestamp < after.timestamp - timestamp {
                        idx - 1
                    } else {
                        idx
                    }
                }
            });

        Some(player_data[idx].position)
    }

    /// Get all player IDs that have recorded positions
    pub fn get_player_ids(&self) -> Vec<u32> {
        self.players.keys().copied().collect()
    }

    /// Add a match event (only if event tracking is enabled)
    pub fn add_match_event(&mut self, timestamp: u64, category: &str, description: String) {
        if self.track_events {
            self.events.push(MatchEventData {
                timestamp,
                category: category.to_string(),
                description,
            });
        }
    }

    /// Add a pass event (only if event tracking is enabled)
    pub fn add_pass_event(&mut self, timestamp: u64, from_player_id: u32, to_player_id: u32) {
        if self.track_events {
            self.passes
                .push(PassEventData::new(timestamp, from_player_id, to_player_id));
        }
    }

    /// Record a player state change. Uses a cheap integer ID for fast dedup,
    /// only allocating the display String when the state actually changed.
    pub fn add_player_state(
        &mut self,
        player_id: u32,
        timestamp: u64,
        state_id: u16,
        state: &impl Display,
    ) {
        if !self.track_events {
            return;
        }

        // Fast dedup using integer comparison — avoids to_string() ~90% of the time
        if let Some(&last_id) = self.last_state_ids.get(&player_id) {
            if last_id == state_id {
                return;
            }
        }

        self.last_state_ids.insert(player_id, state_id);
        let state_name = state.to_string();

        if let Some(entries) = self.player_states.get_mut(&player_id) {
            entries.push(PlayerStateEntry {
                timestamp,
                state: state_name,
            });
        } else {
            self.player_states.insert(
                player_id,
                vec![PlayerStateEntry {
                    timestamp,
                    state: state_name,
                }],
            );
        }
    }

    /// Get the most recent pass event at or before a timestamp
    pub fn get_recent_pass_at(&self, timestamp: u64) -> Option<&PassEventData> {
        // Find most recent pass that occurred at or before this timestamp
        self.passes
            .iter()
            .rev() // Search from most recent
            .find(|pass| pass.timestamp <= timestamp)
    }

    /// Get all passes that occurred within a time window around the timestamp
    pub fn get_passes_in_window(&self, timestamp: u64, window_ms: u64) -> Vec<&PassEventData> {
        let start = timestamp.saturating_sub(window_ms);
        let end = timestamp + window_ms;

        self.passes
            .iter()
            .filter(|pass| pass.timestamp >= start && pass.timestamp <= end)
            .collect()
    }
}

pub trait VectorExtensions {
    fn length(&self) -> f32;
    fn distance_to(&self, other: &Vector3<f32>) -> f32;
}

impl VectorExtensions for Vector3<f32> {
    #[inline]
    fn length(&self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    #[inline]
    fn distance_to(&self, other: &Vector3<f32>) -> f32 {
        let diff = self - other;
        diff.dot(&diff).sqrt()
    }
}

/// What a goals-only recording keeps, and — just as load-bearing — what it
/// throws away. A clip that loses the build-up is worthless to watch, and a
/// recorder that quietly keeps the whole match is the cost this exists to
/// avoid; neither failure shows up anywhere but here.
#[cfg(test)]
mod goal_clip_tests {
    use super::*;

    /// Plays `duration_ms` of ball and one player, moving both far enough
    /// every sample that nothing is deduplicated, and scores at each time in
    /// `goals`.
    ///
    /// The goal is marked BEFORE the sample at the same instant, which is the
    /// order the engine does it in: the dispatch that awards the goal runs
    /// inside the tick, and `write_match_positions` runs after it.
    fn record(scope: RecordingScope, duration_ms: u64, goals: &[u64]) -> ResultMatchPositionData {
        let mut data = ResultMatchPositionData::new().with_scope(scope);
        let mut t = 30;
        while t <= duration_ms {
            if goals.contains(&t) {
                data.mark_goal(t);
            }
            // An eighth of a metre of travel per sample — comfortably clear of
            // the dedup tolerance, so sample count equals tick count.
            let drift = (t / 30) as f32;
            data.add_ball_positions(t, Vector3::new(drift, 272.0, 0.0));
            data.add_player_positions(7, t, Vector3::new(drift, 200.0, 0.0));
            t += 30;
        }
        data.finish(duration_ms);
        data
    }

    fn stamps(samples: &[ResultPositionDataItem]) -> (u64, u64) {
        (
            samples.first().expect("a sample").timestamp,
            samples.last().expect("a sample").timestamp,
        )
    }

    #[test]
    fn a_goal_keeps_five_seconds_either_side_and_nothing_else() {
        let data = record(RecordingScope::Goals, 60_000, &[30_000]);

        let (first, last) = stamps(&data.ball);
        assert!(
            first >= 25_000 && last <= 35_000,
            "kept ball samples outside the clip: {first}..{last}"
        );
        // And the WHOLE clip, not just the half after the ball crossed the
        // line — the pre-roll is the part that has to be held speculatively,
        // so it is the part that goes missing.
        assert!(
            first <= 25_030,
            "the build-up was dropped: starts at {first}"
        );
        assert!(
            last >= 34_970,
            "the celebration was dropped: ends at {last}"
        );

        let player = data.players.get(&7).expect("the player was recorded");
        let (first, last) = stamps(player);
        assert!(
            first <= 25_030 && last >= 34_970,
            "the player's clip does not match the ball's: {first}..{last}"
        );

        assert_eq!(data.recorded_segments(), Some(&[(25_000, 35_000)][..]));
    }

    #[test]
    fn goals_inside_one_anothers_clips_merge_into_a_single_segment() {
        // A goal three seconds after another one has an overlapping clip. Two
        // overlapping ranges would double-record the shared seconds and leave
        // the viewer to reconcile them.
        let data = record(RecordingScope::Goals, 60_000, &[30_000, 33_000]);
        assert_eq!(data.recorded_segments(), Some(&[(25_000, 38_000)][..]));

        let stamps: Vec<u64> = data.ball.iter().map(|item| item.timestamp).collect();
        let mut sorted = stamps.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            stamps, sorted,
            "the merged clip repeats or reorders samples"
        );
    }

    #[test]
    fn goals_far_apart_are_separate_clips() {
        let data = record(RecordingScope::Goals, 120_000, &[30_000, 90_000]);
        assert_eq!(
            data.recorded_segments(),
            Some(&[(25_000, 35_000), (85_000, 95_000)][..])
        );
        assert!(
            !data
                .ball
                .iter()
                .any(|item| (35_000..85_000).contains(&item.timestamp)),
            "the hole between two goals was recorded"
        );
    }

    #[test]
    fn a_goalless_match_records_nothing_and_says_so() {
        let data = record(RecordingScope::Goals, 60_000, &[]);
        assert!(data.is_empty(), "a goalless match kept samples");
        // `Some(empty)`, never `None`: the viewer reads a missing segment list
        // as "the whole match is here" and would wait forever for it.
        assert_eq!(data.recorded_segments(), Some::<&[(u64, u64)]>(&[]));
    }

    /// A match played on a remote worker arrives at the coordinator through
    /// `MatchResultRaw`'s `#[serde(skip)]` position track — that is, as an
    /// `empty()` recorder, which still gets stored and still gets a metadata
    /// file. It has to report the same "nothing here" a goalless match does,
    /// or the viewer reads the absent segment list as "the whole match is
    /// recorded" and spins on the loading notice waiting for chunks that were
    /// never written.
    #[test]
    fn a_recording_that_was_never_sampled_reports_nothing_rather_than_everything() {
        let data = ResultMatchPositionData::empty();
        assert_eq!(data.recorded_segments(), Some::<&[(u64, u64)]>(&[]));
        assert!(data.is_empty());
    }

    #[test]
    fn a_full_recording_keeps_the_whole_match() {
        let data = record(RecordingScope::Full, 60_000, &[30_000]);
        assert_eq!(
            data.recorded_segments(),
            None,
            "a full recording must not advertise segments — that is what the \
             dev harness and every pre-clipping recording look like"
        );
        assert_eq!(data.ball.len(), 2_000, "samples went missing");
    }

    #[test]
    fn a_goal_at_the_death_does_not_claim_time_that_never_happened() {
        // Five seconds past a whistle that has already gone. Left alone, the
        // timeline draws a clip running past full time.
        let data = record(RecordingScope::Goals, 60_000, &[57_000]);
        assert_eq!(data.recorded_segments(), Some(&[(52_000, 60_000)][..]));
    }

    #[test]
    fn the_pre_roll_holds_a_window_rather_than_a_match() {
        // The whole point of clipping is that an unrecorded match costs almost
        // nothing to play. If the speculative buffer grew without bound it
        // would cost MORE than recording everything did.
        let mut data = ResultMatchPositionData::new().with_scope(RecordingScope::Goals);
        let mut t = 30;
        while t <= 600_000 {
            let drift = (t / 30) as f32;
            data.add_ball_positions(t, Vector3::new(drift, 272.0, 0.0));
            t += 30;
        }
        assert!(
            data.pending_ball.len() <= 200,
            "the pre-roll is holding {} samples of a ten-minute goalless spell",
            data.pending_ball.len()
        );
        let oldest = data.pending_ball.front().expect("a sample").timestamp;
        assert!(
            oldest >= 600_000 - GOAL_CLIP_PRE_ROLL_MS,
            "the pre-roll is holding samples from {oldest}, older than its window"
        );
    }
}

/// The recorded position format is a wire contract with the replay viewer, and
/// the vertical axis is the half of it that carries a different unit from the
/// other two. These pin the rounding, the 2D/3D split and the dedup on that
/// axis, because every one of them was wrong in the same direction — treating
/// a metre as though it were a game unit — and nothing downstream complains
/// when a height quietly disappears.
#[cfg(test)]
mod height_recording_tests {
    use super::*;

    /// Samples the recorder keeps for a ball following `path`, offered every
    /// 30 ms — the engine's own recording cadence.
    fn recorded(path: &[Vector3<f32>]) -> usize {
        let mut data = ResultMatchPositionData::new();
        for (step, position) in path.iter().enumerate() {
            data.add_ball_positions(step as u64 * 30, *position);
        }
        data.ball.len()
    }

    fn as_json(position: Vector3<f32>) -> String {
        serde_json::to_string(&ResultPositionDataItem::new(0, position)).unwrap()
    }

    #[test]
    fn a_ball_dropping_vertically_is_recorded_all_the_way_down() {
        // Free fall from 4 m, sampled at 30 ms, moving on no other axis. This
        // is the case the shared tolerance lost: with the height delta left in
        // metres the ball had to fall 30 cm before anything was written down,
        // so a dropping ball arrived in the replay as three or four samples
        // and was interpolated into a glide.
        let mut path = Vec::new();
        let (mut z, mut fall) = (4.0f32, 0.0f32);
        while z > 0.0 {
            path.push(Vector3::new(400.0, 272.0, z));
            fall += 9.81 * 0.03;
            z -= fall * 0.03;
        }
        let kept = recorded(&path);
        assert!(
            kept >= path.len() - 2,
            "a vertical drop must survive the dedup: kept {kept} of {} samples",
            path.len()
        );
    }

    #[test]
    fn a_ball_sitting_still_is_still_deduplicated() {
        // The other half of the contract. Weighting the height axis by eight
        // must not turn float noise on a dead ball into a sample per tick —
        // the dedup exists to keep a match's recording down to a few hundred
        // kilobytes.
        let path = vec![Vector3::new(400.0, 272.0, 0.0); 20];
        assert_eq!(recorded(&path), 1, "a dead ball is one sample");
    }

    #[test]
    fn heights_below_a_tenth_of_a_metre_survive() {
        // With one shared step there was nothing between "on the deck" and
        // "ten centimetres up": a 4 cm bounce rounded to zero and serialised
        // as a ball that never left the ground, and a 6 cm one rounded up to
        // 0.1 and was written down as half again as high as it was. Both
        // failures are the same missing resolution.
        for (height, wanted) in [(0.04f32, "0.04"), (0.06, "0.06")] {
            let json = as_json(Vector3::new(400.0, 272.0, height));
            assert!(
                json.contains(wanted),
                "a {:.0} cm bounce must reach the wire as {wanted}, got {json}",
                height * 100.0
            );
        }
    }

    #[test]
    fn a_ball_in_a_keepers_gloves_records_the_height_it_is_carried_at() {
        // `Ball::carry_height` is 1.15 m, and the viewer reads that band to
        // know the ball is in a keeper's hands. Rounded to 0.1 it landed on
        // 1.1; the band has to be widened to catch it, and every other height
        // in the engine loses the same 5 cm.
        let json = as_json(Vector3::new(400.0, 272.0, 1.15));
        assert!(
            json.contains("1.15"),
            "the carry height must round-trip exactly, got {json}"
        );
    }

    #[test]
    fn a_grounded_ball_serialises_without_a_height_at_all() {
        // The 2D/3D split is what keeps the common case — every player, every
        // rolling ball — at three elements instead of four.
        assert_eq!(as_json(Vector3::new(400.0, 272.0, 0.0)), "[0,400.0,272.0]");
    }

    #[test]
    fn the_horizontal_axes_are_unchanged() {
        // Only the vertical axis moved. x and y are game units of 0.125 m and
        // 0.1 u of resolution is 1.25 cm, which was never the problem.
        assert_eq!(Quantize::ground(400.04), 400.0);
        assert_eq!(Quantize::ground(400.06), 400.1);
    }
}

/// The other half of a clipped recording: the near misses, which are marked on
/// spec and pruned at the whistle.
///
/// The pruning is the part that can go quietly wrong. A recorder that keeps
/// every marked chance costs several times what clipping was supposed to save
/// and nobody notices until a season's worth of recordings is on disk; one that
/// drops the segment but leaves the samples behind writes footage no segment
/// points at, which the chunker then splits around. Neither shows up in a
/// recording you watch — only here.
#[cfg(test)]
mod chance_clip_tests {
    use super::*;

    /// Plays `duration_ms` of ball and one player, scoring at each time in
    /// `goals` and striking a chance at each time in `chances`, then finishes
    /// keeping only `kept`.
    fn record(
        duration_ms: u64,
        goals: &[u64],
        chances: &[u64],
        kept: &[u64],
    ) -> ResultMatchPositionData {
        let mut data = ResultMatchPositionData::new().with_scope(RecordingScope::Goals);
        let mut t = 30;
        while t <= duration_ms {
            if goals.contains(&t) {
                data.mark_goal(t);
            }
            if chances.contains(&t) {
                data.mark_chance(t);
            }
            let drift = (t / 30) as f32;
            data.add_ball_positions(t, Vector3::new(drift, 272.0, 0.0));
            data.add_player_positions(7, t, Vector3::new(drift, 200.0, 0.0));
            t += 30;
        }
        data.finish_retaining(duration_ms, kept);
        data
    }

    #[test]
    fn a_kept_chance_is_clipped_exactly_like_a_goal() {
        let data = record(120_000, &[], &[60_000], &[60_000]);
        assert_eq!(data.recorded_segments(), Some(&[(55_000, 65_000)][..]));
        assert!(
            data.ball
                .iter()
                .all(|item| (55_000..=65_000).contains(&item.timestamp)),
            "a chance clip kept samples outside its window"
        );
    }

    #[test]
    fn a_chance_the_match_sheet_dropped_leaves_nothing_behind() {
        // Marked, held for the whole ten seconds, and then not chosen. Both the
        // segment AND the samples have to go: a segment list is what the viewer
        // navigates by, so footage outside it is bytes nobody can ever reach.
        let data = record(120_000, &[30_000], &[60_000, 90_000], &[]);

        assert_eq!(data.recorded_segments(), Some(&[(25_000, 35_000)][..]));
        assert!(
            !data
                .ball
                .iter()
                .any(|item| item.timestamp >= 55_000 && item.timestamp <= 95_000),
            "the dropped chances left their samples in the recording"
        );
        let player = data.players.get(&7).expect("the player was recorded");
        assert!(
            !player
                .iter()
                .any(|item| item.timestamp >= 55_000 && item.timestamp <= 95_000),
            "the dropped chances left the player's samples in the recording"
        );
    }

    #[test]
    fn a_goalless_match_can_still_have_a_reel() {
        // What this whole feature is for. Nil-nil used to record literally
        // nothing and the viewer said so; now the chances are the recording.
        let data = record(120_000, &[], &[39_000, 81_000], &[39_000, 81_000]);
        assert_eq!(
            data.recorded_segments(),
            Some(&[(34_000, 44_000), (76_000, 86_000)][..])
        );
        assert!(!data.is_empty());
    }

    #[test]
    fn a_chance_that_became_a_goal_is_one_clip_rather_than_two() {
        // The save and the rebound put away two seconds later. The shortlist
        // drops the chance (see `HighlightSelector`), and what is left is the
        // goal's own window — not an overlapping pair for the viewer to
        // reconcile.
        let data = record(120_000, &[63_000], &[60_000], &[]);
        assert_eq!(data.recorded_segments(), Some(&[(58_000, 68_000)][..]));
    }

    #[test]
    fn overlapping_clips_of_different_kinds_merge() {
        // If the shortlist DOES keep a chance next to a goal — a different
        // team's, at the other end — the two ranges still have to come out as
        // one segment.
        let data = record(120_000, &[60_000], &[66_000], &[66_000]);
        assert_eq!(data.recorded_segments(), Some(&[(55_000, 71_000)][..]));

        let stamps: Vec<u64> = data.ball.iter().map(|item| item.timestamp).collect();
        let mut sorted = stamps.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            stamps, sorted,
            "the merged clip repeats or reorders samples"
        );
    }

    #[test]
    fn a_full_recording_ignores_the_shortlist_entirely() {
        // `.dev/match` and the calibration harness read the whole match back
        // off the recording. A prune that ran here would gut them silently —
        // and the shortlist they are handed is the same one the game uses.
        let mut data = ResultMatchPositionData::new().with_scope(RecordingScope::Full);
        let mut t = 30;
        while t <= 60_000 {
            if t == 30_000 {
                data.mark_chance(t);
            }
            let drift = (t / 30) as f32;
            data.add_ball_positions(t, Vector3::new(drift, 272.0, 0.0));
            t += 30;
        }
        data.finish_retaining(60_000, &[]);

        assert_eq!(data.recorded_segments(), None);
        assert_eq!(data.ball.len(), 2_000, "samples went missing");
    }
}

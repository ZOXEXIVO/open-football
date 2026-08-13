use nalgebra::Vector3;
use serde::Serialize;
use serde::Serializer;
use serde::ser::{SerializeMap, SerializeSeq};
use std::collections::HashMap;
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
    pub fn new() -> Self {
        ResultMatchPositionData {
            ball: Vec::new(),
            players: HashMap::with_capacity(44),
            passes: Vec::new(),
            events: Vec::new(),
            player_states: HashMap::new(),
            last_state_ids: HashMap::new(),
            track_events: false,
            track_positions: true,
        }
    }

    pub fn new_with_tracking() -> Self {
        ResultMatchPositionData {
            ball: Vec::new(),
            players: HashMap::with_capacity(44),
            passes: Vec::new(),
            events: Vec::new(),
            player_states: HashMap::with_capacity(44),
            last_state_ids: HashMap::with_capacity(44),
            track_events: true,
            track_positions: true,
        }
    }

    pub fn empty() -> Self {
        ResultMatchPositionData {
            ball: Vec::new(),
            players: HashMap::new(),
            passes: Vec::new(),
            events: Vec::new(),
            player_states: HashMap::new(),
            last_state_ids: HashMap::new(),
            track_events: false,
            track_positions: false,
        }
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

            let mut chunk = ResultMatchPositionData {
                ball: Vec::new(),
                players: HashMap::new(),
                passes: Vec::new(),
                events: Vec::new(),
                player_states: HashMap::new(),
                last_state_ids: HashMap::new(),
                track_events: self.track_events,
                track_positions: self.track_positions,
            };

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

    /// Add player position with quantization and tolerance-based dedup.
    /// Skips recording if the player hasn't moved more than 0.3 units since last entry.
    pub fn add_player_positions(&mut self, player_id: u32, timestamp: u64, position: Vector3<f32>) {
        if !self.track_positions {
            return;
        }

        // Rounded per axis — reduces float noise and produces shorter JSON.
        let position = Quantize::position(position);

        if let Some(player_data) = self.players.get_mut(&player_id) {
            let last = player_data.last().unwrap();

            // Tolerance dedup + heartbeat: skip tiny movements unless we're
            // overdue for a sample. Without the heartbeat, a GK planted in
            // the six-yard box gets no updates until a save, and replay
            // viewers can't distinguish "on-pitch, idle" from "subbed off".
            let distance_sq = Quantize::separation_sq(position, last.position);
            let since_last = timestamp.saturating_sub(last.timestamp);
            if distance_sq < DEDUP_TOLERANCE_SQ && since_last < HEARTBEAT_INTERVAL_MS {
                return;
            }

            player_data.push(ResultPositionDataItem::new(timestamp, position));
        } else {
            self.players.insert(
                player_id,
                vec![ResultPositionDataItem::new(timestamp, position)],
            );
        }
    }

    /// Add ball position with quantization and tolerance-based dedup.
    /// Previous implementation had a bug: PartialEq compared timestamps too,
    /// so ball positions were NEVER deduplicated (timestamps always differ).
    pub fn add_ball_positions(&mut self, timestamp: u64, position: Vector3<f32>) {
        if !self.track_positions {
            return;
        }

        let position = Quantize::position(position);

        if let Some(last) = self.ball.last() {
            // Tolerance dedup + heartbeat. Without the heartbeat, an
            // owned-and-stationary ball (stuck with a player who isn't
            // passing) gets no ball samples for the rest of the match
            // — `max_timestamp` freezes at the last movement and the
            // chunk split discards everything after that point, even
            // though the sim is still running. Player positions use
            // the same heartbeat for the same reason.
            //
            // The height delta is weighted into game units before the
            // comparison. Unweighted, this is the test that lost every
            // near-vertical ball in the recording: a drop, the top of a lob, a
            // bounce coming up under a boot all sat inside the tolerance until
            // they had travelled 30 cm.
            let since_last = timestamp.saturating_sub(last.timestamp);
            if Quantize::separation_sq(position, last.position) < DEDUP_TOLERANCE_SQ
                && since_last < HEARTBEAT_INTERVAL_MS
            {
                return;
            }
        }

        self.ball
            .push(ResultPositionDataItem::new(timestamp, position));
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

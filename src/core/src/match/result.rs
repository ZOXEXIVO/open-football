use nalgebra::Vector3;
use serde::Serialize;
use std::collections::HashMap;

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

impl PartialEq<ResultPositionDataItem> for ResultPositionDataItem {
    fn eq(&self, other: &Self) -> bool {
        self.timestamp == other.timestamp && self.position == other.position
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ResultMatchPositionData {
    ball: Vec<ResultPositionDataItem>,
    players: HashMap<u32, Vec<ResultPositionDataItem>>,
    passes: Vec<PassEventData>,
    #[serde(skip)]
    track_events: bool,
}

impl ResultMatchPositionData {
    pub fn new() -> Self {
        ResultMatchPositionData {
            ball: Vec::new(),
            players: HashMap::with_capacity(22 * 2 * 9000),
            passes: Vec::new(),
            track_events: false,
        }
    }

    pub fn new_with_tracking() -> Self {
        ResultMatchPositionData {
            ball: Vec::new(),
            players: HashMap::with_capacity(22 * 2 * 9000),
            passes: Vec::new(),
            track_events: true,
        }
    }

    pub fn compress(&mut self) {}

    /// Check if event tracking is enabled
    #[inline]
    pub fn is_tracking_events(&self) -> bool {
        self.track_events
    }

    pub fn add_player_positions(&mut self, player_id: u32, timestamp: u64, position: Vector3<f32>) {
        if let Some(player_data) = self.players.get_mut(&player_id) {
            let last_data = player_data.last().unwrap();
            if last_data.position.x != position.x
                || last_data.position.y != position.y
                || last_data.position.z != position.z
            {
                let position_data = ResultPositionDataItem::new(timestamp, position);
                player_data.push(position_data);
            }
        } else {
            self.players
                .insert(player_id, vec![ResultPositionDataItem::new(timestamp, position)]);
        }
    }

    pub fn add_ball_positions(&mut self, timestamp: u64, position: Vector3<f32>) {
        let position = ResultPositionDataItem::new(timestamp, position);

        if let Some(last_position) = self.ball.last() {
            if last_position != &position {
                self.ball.push(position);
            }
        } else {
            self.ball.push(position);
        }
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
        let idx = self.ball.binary_search_by_key(&timestamp, |item| item.timestamp)
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
        let idx = player_data.binary_search_by_key(&timestamp, |item| item.timestamp)
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

    /// Add a pass event (only if event tracking is enabled)
    pub fn add_pass_event(&mut self, timestamp: u64, from_player_id: u32, to_player_id: u32) {
        if self.track_events {
            self.passes.push(PassEventData::new(timestamp, from_player_id, to_player_id));
        }
    }

    /// Get the most recent pass event at or before a timestamp
    pub fn get_recent_pass_at(&self, timestamp: u64) -> Option<&PassEventData> {
        // Find most recent pass that occurred at or before this timestamp
        self.passes.iter()
            .rev()  // Search from most recent
            .find(|pass| pass.timestamp <= timestamp)
    }

    /// Get all passes that occurred within a time window around the timestamp
    pub fn get_passes_in_window(&self, timestamp: u64, window_ms: u64) -> Vec<&PassEventData> {
        let start = timestamp.saturating_sub(window_ms);
        let end = timestamp + window_ms;

        self.passes.iter()
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

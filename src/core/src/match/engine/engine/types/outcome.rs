//! What a simulated match hands back: the per-player events the result
//! builder credits, and the per-state tick result.

pub enum MatchEvent {
    MatchPlayed(u32, bool, u8),
    Goal(u32),
    Assist(u32),
    Injury(u32),
}

#[derive(Default, Clone)]
pub struct PlayMatchStateResult {
    pub additional_time: u64,
}

use crate::simulator::SimulatorData;

pub type AiResponseHandler = Box<dyn FnOnce(&str, &mut SimulatorData) + Send>;

pub struct PendingAiRequest {
    pub club_id: u32,
    pub priority: u8,
    pub query: String,
    pub format: String,
    pub handler: AiResponseHandler,
}

pub struct CompletedAiRequest {
    pub club_id: u32,
    pub priority: u8,
    pub response: Option<String>,
    pub handler: AiResponseHandler,
}

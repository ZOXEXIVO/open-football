use crate::ai::CompletedAiRequest;
use crate::simulator::SimulatorData;

pub fn apply_ai_responses(mut responses: Vec<CompletedAiRequest>, data: &mut SimulatorData) {
    responses.sort_by_key(|r| r.priority);

    for completed in responses {
        if let Some(response) = completed.response {
            (completed.handler)(&response, data);
        }
    }
}

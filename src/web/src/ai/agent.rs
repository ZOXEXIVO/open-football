use crate::ai::LlmSettings;
use crate::ai::client::AiClient;
use crate::ai::jobs::AiJobHandle;
use crate::ai::tools::AiTools;
use core::SimulatorData;
use serde_json::{Value, json};
use std::sync::Arc;

/// Safety bound on the agent loop so a misbehaving model can't spin forever.
const MAX_STEPS: usize = 30;

/// Drives the model ↔ tools loop: the model decides which tools to call, we
/// execute them against the world snapshot and feed the JSON back, until the
/// model returns a final written report (or we hit the step bound). Progress
/// is reported through an `AiJobHandle` so the dialog can render tool calls in
/// real time; the prompt is supplied by the caller (co-located with its page).
pub struct AiAgent {
    client: AiClient,
    tools: AiTools,
}

impl AiAgent {
    pub fn new(settings: LlmSettings, data: Arc<SimulatorData>) -> Self {
        AiAgent {
            client: AiClient::new(settings),
            tools: AiTools::new(data),
        }
    }

    /// Run the agent to completion, writing progress + the final result into
    /// `handle`. `system` is the page-specific system prompt; `task` is the
    /// concrete instruction (which club to report on, etc.).
    pub async fn run(&self, system: &str, task: &str, handle: &AiJobHandle) {
        let mut messages: Vec<Value> = vec![
            json!({ "role": "system", "content": system }),
            json!({ "role": "user", "content": task }),
        ];
        let schemas = AiTools::schemas();

        for _ in 0..MAX_STEPS {
            let turn = match self.client.chat(&messages, &schemas).await {
                Ok(turn) => turn,
                Err(detail) => {
                    handle.fail(detail);
                    return;
                }
            };

            if turn.tool_calls.is_empty() {
                handle.finish(turn.content.unwrap_or_default());
                return;
            }

            // Echo the assistant's tool-call message back verbatim (with an
            // explicit `type: function`) before appending each tool result.
            let echoed: Vec<Value> = turn
                .tool_calls
                .iter()
                .map(|tc| {
                    json!({
                        "id": tc.id,
                        "type": "function",
                        "function": { "name": tc.function.name, "arguments": tc.function.arguments },
                    })
                })
                .collect();
            messages.push(json!({
                "role": "assistant",
                "content": turn.content,
                "tool_calls": echoed,
            }));

            for tc in &turn.tool_calls {
                handle.push_tool(tc.function.name.clone(), tc.function.arguments.clone());
                let result = self
                    .tools
                    .dispatch(&tc.function.name, &tc.function.arguments);
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tc.id,
                    "content": result,
                }));
            }
        }

        handle.fail(format!("the agent did not finish within {MAX_STEPS} steps"));
    }
}

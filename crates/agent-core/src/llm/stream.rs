use crate::error::AgentCoreResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type LlmStream = Box<dyn Iterator<Item = AgentCoreResult<LlmStreamEvent>> + Send>;

pub fn stream_from_events(events: Vec<LlmStreamEvent>) -> LlmStream {
    Box::new(events.into_iter().map(Ok))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum LlmStreamEvent {
    PreparedRequest {
        provider: String,
        body: Value,
    },
    ResponseCreated {
        response_id: String,
    },
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolCallDelta {
        call_id: String,
        name: Option<String>,
        arguments_delta: String,
    },
    ToolCallCompleted {
        call_id: String,
        name: String,
        arguments: Value,
    },
    Usage {
        usage: LlmUsage,
    },
    Completed {
        response_id: Option<String>,
    },
    Error {
        message: String,
        recoverable: bool,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

use crate::llm_model::ModelId;
use crate::run_message::RunMessage;
use crate::tool_schema::ToolSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model: ModelId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<RunMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSchema>,
    #[serde(default)]
    pub options: LlmRequestOptions,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl LlmRequest {
    pub fn new(model: impl Into<ModelId>) -> Self {
        Self {
            model: model.into(),
            api: None,
            instructions: None,
            messages: Vec::new(),
            tools: Vec::new(),
            options: LlmRequestOptions::default(),
            metadata: BTreeMap::new(),
            headers: BTreeMap::new(),
        }
    }

    pub fn push_message(&mut self, message: RunMessage) {
        self.messages.push(message);
    }

    pub fn extend_messages(&mut self, messages: impl IntoIterator<Item = RunMessage>) {
        self.messages.extend(messages);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmRequestOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default)]
    pub store: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
}

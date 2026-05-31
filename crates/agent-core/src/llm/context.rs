use crate::agent_definition::AgentDefinition;
use crate::error::AgentCoreResult;
use crate::llm_model::ModelId;
use crate::llm_request::{LlmRequest, LlmRequestOptions};
use crate::run_message::{MessageRole, RunMessage};
use crate::tool_schema::ToolSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextBuildInput {
    pub model: ModelId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions_override: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<RunMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visible_tool_schemas: Vec<ToolSchema>,
    #[serde(default)]
    pub options: LlmRequestOptions,
    #[serde(default)]
    pub include_diagnostics: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

impl ContextBuildInput {
    pub fn new(model: impl Into<ModelId>) -> Self {
        Self {
            model: model.into(),
            api: None,
            instructions_override: None,
            messages: Vec::new(),
            visible_tool_schemas: Vec::new(),
            options: LlmRequestOptions::default(),
            include_diagnostics: false,
            metadata: BTreeMap::new(),
        }
    }

    pub fn push_message(&mut self, message: RunMessage) {
        self.messages.push(message);
    }

    pub fn extend_messages(&mut self, messages: impl IntoIterator<Item = RunMessage>) {
        self.messages.extend(messages);
    }
}

#[derive(Clone, Debug, Default)]
pub struct ContextBuilder;

impl ContextBuilder {
    pub fn new() -> Self {
        Self
    }

    pub fn build(
        &self,
        definition: &AgentDefinition,
        input: ContextBuildInput,
    ) -> AgentCoreResult<LlmRequest> {
        let mut request = LlmRequest::new(input.model);
        request.api = input.api;
        request.instructions = Some(
            input
                .instructions_override
                .unwrap_or_else(|| definition.system_prompt().to_string()),
        );
        request.tools = input.visible_tool_schemas;
        request.options = input.options;
        request.metadata = input.metadata;
        request.extend_messages(input.messages.into_iter().filter(|message| {
            input.include_diagnostics || message.role != MessageRole::Diagnostic
        }));

        Ok(request)
    }
}

pub fn message_value_to_run_message(message: &Value) -> AgentCoreResult<RunMessage> {
    Ok(serde_json::from_value(message.clone())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_definition::AgentDefinitionBuilder;
    use crate::content_block::{ContentBlock, DiagnosticLevel};
    use serde_json::json;

    #[test]
    fn context_builder_preserves_messages_as_request_messages() {
        let definition = AgentDefinitionBuilder::new()
            .name("agent")
            .system_prompt("system")
            .build()
            .unwrap();
        let user = RunMessage::user(vec![ContentBlock::text("hello")]).unwrap();
        let diagnostic = RunMessage::diagnostic(vec![ContentBlock::diagnostic(
            DiagnosticLevel::Info,
            "debug",
        )])
        .unwrap();

        let mut input = ContextBuildInput::new("model");
        input.extend_messages([user.clone(), diagnostic]);
        let request = ContextBuilder::new().build(&definition, input).unwrap();

        assert_eq!(request.messages, vec![user]);
    }

    #[test]
    fn message_value_parses_as_run_message() {
        let message = message_value_to_run_message(&json!({
            "id": uuid::Uuid::new_v4(),
            "role": "tool",
            "content": [{
                "type": "tool_result",
                "call_id": "call-1",
                "tool_name": "search",
                "is_error": false,
                "output": {"answer": 42}
            }],
            "status": "finalized",
            "created_at_ms": 1
        }))
        .unwrap();

        assert_eq!(message.role, MessageRole::Tool);
    }
}

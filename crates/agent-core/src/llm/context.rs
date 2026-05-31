use crate::agent_definition::AgentDefinition;
use crate::content_block::ContentBlock;
use crate::error::{AgentCoreError, AgentCoreResult};
use crate::llm_model::ModelId;
use crate::llm_request::{
    LlmContentPart, LlmInputItem, LlmMessageRole, LlmRequest, LlmRequestOptions,
};
use crate::run_message::MessageRole;
use crate::tool_schema::ToolSchema;
use crate::RunMessage;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContextBuildInput {
    pub model: ModelId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions_override: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replay_messages: Vec<RunMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub run_messages: Vec<RunMessage>,
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
            replay_messages: Vec::new(),
            run_messages: Vec::new(),
            visible_tool_schemas: Vec::new(),
            options: LlmRequestOptions::default(),
            include_diagnostics: false,
            metadata: BTreeMap::new(),
        }
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

        for message in input
            .replay_messages
            .iter()
            .chain(input.run_messages.iter())
        {
            request.input.extend(run_message_to_input_items(
                message,
                input.include_diagnostics,
            )?);
        }

        Ok(request)
    }
}

pub fn run_message_to_input_items(
    message: &RunMessage,
    include_diagnostics: bool,
) -> AgentCoreResult<Vec<LlmInputItem>> {
    let role = message_role_to_llm_role(message.role);
    if role == LlmMessageRole::Diagnostic && !include_diagnostics {
        return Ok(Vec::new());
    }

    let mut items = Vec::new();
    let mut message_parts = Vec::new();

    for block in &message.content {
        match block {
            ContentBlock::ToolResult {
                call_id,
                output,
                is_error,
                ..
            } => items.push(LlmInputItem::FunctionCallOutput {
                call_id: call_id.clone(),
                output: output.clone(),
                is_error: *is_error,
            }),
            ContentBlock::ToolCall {
                call_id,
                tool_name,
                arguments,
            } => items.push(LlmInputItem::FunctionCall {
                call_id: call_id.clone(),
                name: tool_name.clone(),
                arguments: arguments.clone(),
            }),
            ContentBlock::Text { text } => {
                message_parts.push(LlmContentPart::Text { text: text.clone() })
            }
            ContentBlock::Reasoning { text } => {
                message_parts.push(LlmContentPart::Reasoning { text: text.clone() })
            }
            ContentBlock::ImageReference { uri, .. } => {
                message_parts.push(LlmContentPart::ImageRef { uri: uri.clone() })
            }
            ContentBlock::FileReference { uri, .. } => {
                message_parts.push(LlmContentPart::FileRef { uri: uri.clone() })
            }
            ContentBlock::AudioReference { uri, mime_type } => {
                message_parts.push(LlmContentPart::Custom {
                    value: json!({
                        "type": "audio_reference",
                        "uri": uri,
                        "mime_type": mime_type,
                    }),
                });
            }
            ContentBlock::Diagnostic { message, .. } => {
                if include_diagnostics {
                    message_parts.push(LlmContentPart::Diagnostic {
                        message: message.clone(),
                    });
                }
            }
            ContentBlock::Custom { value } => {
                message_parts.push(LlmContentPart::Custom {
                    value: value.clone(),
                });
            }
        }
    }

    if !message_parts.is_empty() {
        items.insert(
            0,
            LlmInputItem::Message {
                role,
                content: message_parts,
                metadata: message.metadata.clone(),
            },
        );
    }

    Ok(items)
}

pub fn message_value_to_input_items(
    message: &Value,
    include_diagnostics: bool,
) -> AgentCoreResult<Vec<LlmInputItem>> {
    let role = parse_role(message)?;
    if role == LlmMessageRole::Diagnostic && !include_diagnostics {
        return Ok(Vec::new());
    }

    let mut items = Vec::new();
    let mut message_parts = Vec::new();
    let content = message
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    for block in content {
        match block_type(&block) {
            Some("tool_result") => {
                let call_id = required_string(&block, "call_id")?;
                let output = block
                    .get("output")
                    .cloned()
                    .unwrap_or_else(|| block.get("content").cloned().unwrap_or(Value::Null));
                let is_error = block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                items.push(LlmInputItem::FunctionCallOutput {
                    call_id,
                    output,
                    is_error,
                });
            }
            Some("tool_call") => {
                let call_id = required_string(&block, "call_id")?;
                let name = required_string(&block, "name")
                    .or_else(|_| required_string(&block, "tool_name"))?;
                let arguments = block.get("arguments").cloned().unwrap_or(Value::Null);
                items.push(LlmInputItem::FunctionCall {
                    call_id,
                    name,
                    arguments,
                });
            }
            Some("text") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    message_parts.push(LlmContentPart::Text {
                        text: text.to_string(),
                    });
                }
            }
            Some("reasoning") => {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    message_parts.push(LlmContentPart::Reasoning {
                        text: text.to_string(),
                    });
                }
            }
            Some("image_ref") | Some("image_reference") | Some("image") => {
                if let Some(uri) = block.get("uri").and_then(Value::as_str) {
                    message_parts.push(LlmContentPart::ImageRef {
                        uri: uri.to_string(),
                    });
                }
            }
            Some("file_ref") | Some("file_reference") | Some("file") => {
                if let Some(uri) = block.get("uri").and_then(Value::as_str) {
                    message_parts.push(LlmContentPart::FileRef {
                        uri: uri.to_string(),
                    });
                }
            }
            Some("diagnostic") => {
                if include_diagnostics {
                    if let Some(diagnostic) = block.get("message").and_then(Value::as_str) {
                        message_parts.push(LlmContentPart::Diagnostic {
                            message: diagnostic.to_string(),
                        });
                    }
                }
            }
            _ => message_parts.push(LlmContentPart::Custom { value: block }),
        }
    }

    if !message_parts.is_empty() {
        items.insert(
            0,
            LlmInputItem::Message {
                role,
                content: message_parts,
                metadata: message
                    .get("metadata")
                    .and_then(Value::as_object)
                    .map(|object| {
                        object
                            .iter()
                            .map(|(key, value)| (key.clone(), value.clone()))
                            .collect()
                    })
                    .unwrap_or_default(),
            },
        );
    }

    Ok(items)
}

fn parse_role(message: &Value) -> AgentCoreResult<LlmMessageRole> {
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| AgentCoreError::InvalidInput("message role is missing".to_string()))?;

    match role {
        "user" => Ok(LlmMessageRole::User),
        "assistant" => Ok(LlmMessageRole::Assistant),
        "developer" | "system" => Ok(LlmMessageRole::Developer),
        "tool" => Ok(LlmMessageRole::Tool),
        "diagnostic" => Ok(LlmMessageRole::Diagnostic),
        other => Err(AgentCoreError::InvalidInput(format!(
            "unsupported message role `{other}`"
        ))),
    }
}

fn message_role_to_llm_role(role: MessageRole) -> LlmMessageRole {
    match role {
        MessageRole::User => LlmMessageRole::User,
        MessageRole::Assistant => LlmMessageRole::Assistant,
        MessageRole::Tool => LlmMessageRole::Tool,
        MessageRole::Diagnostic => LlmMessageRole::Diagnostic,
    }
}

fn block_type(block: &Value) -> Option<&str> {
    block.get("type").and_then(Value::as_str)
}

fn required_string(value: &Value, field: &str) -> AgentCoreResult<String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| AgentCoreError::InvalidInput(format!("field `{field}` is missing")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_tool_result_message_to_function_call_output() {
        let items = message_value_to_input_items(
            &json!({
                "role": "tool",
                "content": [{
                    "type": "tool_result",
                    "call_id": "call-1",
                    "tool_name": "search",
                    "is_error": false,
                    "output": {"answer": 42}
                }]
            }),
            false,
        )
        .unwrap();

        assert_eq!(
            items,
            vec![LlmInputItem::FunctionCallOutput {
                call_id: "call-1".to_string(),
                output: json!({"answer": 42}),
                is_error: false,
            }]
        );
    }

    #[test]
    fn drops_diagnostic_messages_by_default() {
        let items = message_value_to_input_items(
            &json!({
                "role": "diagnostic",
                "content": [{"type": "diagnostic", "message": "debug"}]
            }),
            false,
        )
        .unwrap();

        assert!(items.is_empty());
    }
}

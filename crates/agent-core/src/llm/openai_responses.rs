use crate::error::{AgentCoreError, AgentCoreResult};
use crate::llm_model::{LlmApi, LlmModel};
use crate::llm_provider::{LlmProvider, PreparedLlmRequest};
use crate::llm_request::{LlmContentPart, LlmInputItem, LlmMessageRole, LlmRequest};
use crate::llm_stream::{LlmStreamEvent, LlmUsage};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const OPENAI_RESPONSES_API: &str = "openai_responses";

#[derive(Clone, Debug)]
pub struct OpenAiResponsesProvider {
    provider_id: String,
    endpoint: String,
}

impl OpenAiResponsesProvider {
    pub fn new() -> Self {
        Self {
            provider_id: "openai".to_string(),
            endpoint: "https://api.openai.com/v1/responses".to_string(),
        }
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    pub fn default_model(model_id: impl Into<String>) -> LlmModel {
        let model_id: String = model_id.into();
        LlmModel::new(model_id, "openai", LlmApi::OpenAiResponses)
    }

    pub fn request_body(request: &LlmRequest) -> AgentCoreResult<Value> {
        let mut body = Map::new();
        body.insert(
            "model".to_string(),
            Value::String(request.model.as_str().to_string()),
        );

        if let Some(instructions) = &request.instructions {
            body.insert(
                "instructions".to_string(),
                Value::String(instructions.clone()),
            );
        }

        body.insert(
            "input".to_string(),
            Value::Array(
                request
                    .input
                    .iter()
                    .map(map_input_item)
                    .collect::<AgentCoreResult<Vec<_>>>()?,
            ),
        );

        if !request.tools.is_empty() {
            body.insert(
                "tools".to_string(),
                Value::Array(
                    request
                        .tools
                        .iter()
                        .map(|schema| schema.to_function_tool_schema())
                        .collect(),
                ),
            );
        }

        if let Some(temperature) = request.options.temperature {
            body.insert("temperature".to_string(), json!(temperature));
        }

        if let Some(max_output_tokens) = request.options.max_output_tokens {
            body.insert("max_output_tokens".to_string(), json!(max_output_tokens));
        }

        if let Some(parallel_tool_calls) = request.options.parallel_tool_calls {
            body.insert(
                "parallel_tool_calls".to_string(),
                json!(parallel_tool_calls),
            );
        }

        if request.options.store {
            body.insert("store".to_string(), Value::Bool(true));
        }

        if let Some(previous_response_id) = &request.options.previous_response_id {
            body.insert(
                "previous_response_id".to_string(),
                Value::String(previous_response_id.clone()),
            );
        }

        if !request.metadata.is_empty() {
            body.insert(
                "metadata".to_string(),
                Value::Object(
                    request
                        .metadata
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                ),
            );
        }

        Ok(Value::Object(body))
    }

    pub fn stream_event_from_value(value: &Value) -> AgentCoreResult<Option<LlmStreamEvent>> {
        let event_type = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();

        match event_type {
            "response.created" => {
                let response_id = value
                    .get("response")
                    .and_then(|response| response.get("id"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Ok(Some(LlmStreamEvent::ResponseCreated { response_id }))
            }
            "response.output_text.delta" => {
                let text = value
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Ok(Some(LlmStreamEvent::TextDelta { text }))
            }
            "response.reasoning_text.delta" => {
                let text = value
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Ok(Some(LlmStreamEvent::ReasoningDelta { text }))
            }
            "response.output_item.done" => map_output_item_done(value),
            "response.completed" => {
                let response_id = value
                    .get("response")
                    .and_then(|response| response.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                Ok(Some(LlmStreamEvent::Completed { response_id }))
            }
            "response.failed" => {
                let message = value
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("provider request failed")
                    .to_string();
                Ok(Some(LlmStreamEvent::Error {
                    message,
                    recoverable: true,
                }))
            }
            "response.usage" => {
                let usage = value.get("usage").unwrap_or(&Value::Null);
                Ok(Some(LlmStreamEvent::Usage {
                    usage: LlmUsage {
                        input_tokens: usage
                            .get("input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        output_tokens: usage
                            .get("output_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        total_tokens: usage
                            .get("total_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    },
                }))
            }
            _ => Ok(None),
        }
    }
}

impl Default for OpenAiResponsesProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmProvider for OpenAiResponsesProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn api(&self) -> &str {
        OPENAI_RESPONSES_API
    }

    fn prepare_request(&self, request: &LlmRequest) -> AgentCoreResult<PreparedLlmRequest> {
        Ok(PreparedLlmRequest {
            provider: self.provider_id.clone(),
            api: OPENAI_RESPONSES_API.to_string(),
            endpoint: self.endpoint.clone(),
            headers: request.headers.clone(),
            body: Self::request_body(request)?,
            metadata: BTreeMap::new(),
        })
    }
}

fn map_input_item(item: &LlmInputItem) -> AgentCoreResult<Value> {
    match item {
        LlmInputItem::Message {
            role,
            content,
            metadata: _,
        } => Ok(json!({
            "role": map_role(*role),
            "content": content.iter().map(map_content_part).collect::<Vec<_>>(),
        })),
        LlmInputItem::FunctionCall {
            call_id,
            name,
            arguments,
        } => Ok(json!({
            "type": "function_call",
            "call_id": call_id,
            "name": name,
            "arguments": arguments_to_string(arguments)?,
        })),
        LlmInputItem::FunctionCallOutput {
            call_id,
            output,
            is_error,
        } => {
            let mut item = json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": output_to_string(output)?,
            });
            if *is_error {
                item["is_error"] = Value::Bool(true);
            }
            Ok(item)
        }
        LlmInputItem::Custom { value } => Ok(value.clone()),
    }
}

fn map_role(role: LlmMessageRole) -> &'static str {
    match role {
        LlmMessageRole::User => "user",
        LlmMessageRole::Assistant => "assistant",
        LlmMessageRole::Developer => "developer",
        LlmMessageRole::Tool => "user",
        LlmMessageRole::Diagnostic => "developer",
    }
}

fn map_content_part(part: &LlmContentPart) -> Value {
    match part {
        LlmContentPart::Text { text } => json!({
            "type": "input_text",
            "text": text,
        }),
        LlmContentPart::Reasoning { text } => json!({
            "type": "input_text",
            "text": text,
        }),
        LlmContentPart::Json { value } => json!({
            "type": "input_text",
            "text": value.to_string(),
        }),
        LlmContentPart::ImageRef { uri } => json!({
            "type": "input_image",
            "image_url": uri,
        }),
        LlmContentPart::FileRef { uri } => json!({
            "type": "input_file",
            "file_url": uri,
        }),
        LlmContentPart::Diagnostic { message } => json!({
            "type": "input_text",
            "text": message,
        }),
        LlmContentPart::Custom { value } => value.clone(),
    }
}

fn output_to_string(output: &Value) -> AgentCoreResult<String> {
    match output {
        Value::String(text) => Ok(text.clone()),
        other => serde_json::to_string(other).map_err(AgentCoreError::from),
    }
}

fn arguments_to_string(arguments: &Value) -> AgentCoreResult<String> {
    match arguments {
        Value::String(text) => Ok(text.clone()),
        other => serde_json::to_string(other).map_err(AgentCoreError::from),
    }
}

fn map_output_item_done(value: &Value) -> AgentCoreResult<Option<LlmStreamEvent>> {
    let item = value.get("item").unwrap_or(&Value::Null);
    if item.get("type").and_then(Value::as_str) != Some("function_call") {
        return Ok(None);
    }

    let call_id = item
        .get("call_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AgentCoreError::InvalidInput("function_call call_id missing".to_string()))?
        .to_string();
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| AgentCoreError::InvalidInput("function_call name missing".to_string()))?
        .to_string();
    let raw_arguments = item
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let arguments = serde_json::from_str(raw_arguments)?;

    Ok(Some(LlmStreamEvent::ToolCallCompleted {
        call_id,
        name,
        arguments,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_request::{LlmInputItem, LlmRequest};
    use serde_json::json;

    #[test]
    fn maps_tool_result_to_function_call_output() {
        let mut request = LlmRequest::new("gpt-test");
        request.input.push(LlmInputItem::FunctionCallOutput {
            call_id: "call-1".to_string(),
            output: json!({"answer": 42}),
            is_error: false,
        });

        let body = OpenAiResponsesProvider::request_body(&request).unwrap();
        assert_eq!(body["input"][0]["type"], "function_call_output");
        assert_eq!(body["input"][0]["call_id"], "call-1");
        assert_eq!(body["input"][0]["output"], "{\"answer\":42}");
    }
}

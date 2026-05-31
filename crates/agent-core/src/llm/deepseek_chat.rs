use crate::error::{AgentCoreError, AgentCoreResult};
use crate::llm_model::{LlmApi, LlmModel};
use crate::llm_provider::{LlmProvider, PreparedLlmRequest};
use crate::llm_request::{LlmContentPart, LlmInputItem, LlmMessageRole, LlmRequest};
use crate::llm_stream::{LlmStreamEvent, LlmUsage};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fmt;

pub const DEEPSEEK_CHAT_API: &str = "deepseek_chat";
pub const DEEPSEEK_DEFAULT_ENDPOINT: &str = "https://api.deepseek.com/chat/completions";
pub const DEEPSEEK_API_KEY_ENV: &str = "DEEPSEEK_API_KEY";

#[derive(Clone)]
pub struct DeepSeekChatProvider {
    provider_id: String,
    endpoint: String,
    api_key: Option<String>,
}

impl DeepSeekChatProvider {
    pub fn new() -> Self {
        Self {
            provider_id: "deepseek".to_string(),
            endpoint: DEEPSEEK_DEFAULT_ENDPOINT.to_string(),
            api_key: None,
        }
    }

    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    pub fn with_api_key_from_env(mut self) -> AgentCoreResult<Self> {
        let api_key = std::env::var(DEEPSEEK_API_KEY_ENV).map_err(|_| {
            AgentCoreError::InvalidConfig(format!(
                "environment variable `{DEEPSEEK_API_KEY_ENV}` is required"
            ))
        })?;
        self.api_key = Some(api_key);
        Ok(self)
    }

    pub fn default_model(model_id: impl Into<String>) -> LlmModel {
        let mut model = LlmModel::new(model_id.into(), "deepseek", LlmApi::DeepSeekChat);
        model.capabilities.reasoning = true;
        model
    }

    pub fn request_body(request: &LlmRequest) -> AgentCoreResult<Value> {
        let mut body = Map::new();
        body.insert(
            "model".to_string(),
            Value::String(request.model.as_str().to_string()),
        );

        let messages = map_messages(request)?;
        if messages.is_empty() {
            return Err(AgentCoreError::InvalidInput(
                "deepseek chat request requires at least one message".to_string(),
            ));
        }
        body.insert("messages".to_string(), Value::Array(messages));

        if !request.tools.is_empty() {
            body.insert(
                "tools".to_string(),
                Value::Array(request.tools.iter().map(map_tool_schema).collect()),
            );
        }

        if let Some(temperature) = request.options.temperature {
            body.insert("temperature".to_string(), json!(temperature));
        }

        if let Some(max_output_tokens) = request.options.max_output_tokens {
            body.insert("max_tokens".to_string(), json!(max_output_tokens));
        }

        copy_provider_body_field(request, &mut body, "deepseek.thinking", "thinking");
        copy_provider_body_field(
            request,
            &mut body,
            "deepseek.reasoning_effort",
            "reasoning_effort",
        );
        copy_provider_body_field(
            request,
            &mut body,
            "deepseek.response_format",
            "response_format",
        );
        copy_provider_body_field(request, &mut body, "deepseek.tool_choice", "tool_choice");
        copy_provider_body_field(request, &mut body, "deepseek.stream", "stream");
        copy_provider_body_field(
            request,
            &mut body,
            "deepseek.stream_options",
            "stream_options",
        );

        Ok(Value::Object(body))
    }

    pub fn stream_event_from_value(value: &Value) -> AgentCoreResult<Option<LlmStreamEvent>> {
        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            return usage_event(value);
        };

        let delta = choice.get("delta").unwrap_or(&Value::Null);

        if let Some(reasoning) = delta.get("reasoning_content").and_then(Value::as_str) {
            if !reasoning.is_empty() {
                return Ok(Some(LlmStreamEvent::ReasoningDelta {
                    text: reasoning.to_string(),
                }));
            }
        }

        if let Some(content) = delta.get("content").and_then(Value::as_str) {
            if !content.is_empty() {
                return Ok(Some(LlmStreamEvent::TextDelta {
                    text: content.to_string(),
                }));
            }
        }

        if let Some(tool_call) = delta
            .get("tool_calls")
            .and_then(Value::as_array)
            .and_then(|calls| calls.first())
        {
            return Ok(Some(LlmStreamEvent::ToolCallDelta {
                call_id: tool_call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                name: tool_call
                    .get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
                arguments_delta: tool_call
                    .get("function")
                    .and_then(|function| function.get("arguments"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            }));
        }

        if choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .is_some()
        {
            return Ok(Some(LlmStreamEvent::Completed {
                response_id: value.get("id").and_then(Value::as_str).map(str::to_string),
            }));
        }

        usage_event(value)
    }

    pub fn response_events_from_value(value: &Value) -> AgentCoreResult<Vec<LlmStreamEvent>> {
        let mut events = Vec::new();

        if let Some(response_id) = value.get("id").and_then(Value::as_str) {
            events.push(LlmStreamEvent::ResponseCreated {
                response_id: response_id.to_string(),
            });
        }

        for choice in value
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let message = choice.get("message").unwrap_or(&Value::Null);

            if let Some(reasoning) = message.get("reasoning_content").and_then(Value::as_str) {
                if !reasoning.is_empty() {
                    events.push(LlmStreamEvent::ReasoningDelta {
                        text: reasoning.to_string(),
                    });
                }
            }

            if let Some(content) = message.get("content").and_then(Value::as_str) {
                if !content.is_empty() {
                    events.push(LlmStreamEvent::TextDelta {
                        text: content.to_string(),
                    });
                }
            }

            for tool_call in message
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                events.push(map_completed_tool_call(tool_call)?);
            }
        }

        if let Some(usage_event) = usage_event(value)? {
            events.push(usage_event);
        }

        events.push(LlmStreamEvent::Completed {
            response_id: value.get("id").and_then(Value::as_str).map(str::to_string),
        });

        Ok(events)
    }
}

impl Default for DeepSeekChatProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for DeepSeekChatProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeepSeekChatProvider")
            .field("provider_id", &self.provider_id)
            .field("endpoint", &self.endpoint)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl LlmProvider for DeepSeekChatProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn api(&self) -> &str {
        DEEPSEEK_CHAT_API
    }

    fn prepare_request(&self, request: &LlmRequest) -> AgentCoreResult<PreparedLlmRequest> {
        let mut headers = request.headers.clone();
        headers
            .entry("Content-Type".to_string())
            .or_insert_with(|| "application/json".to_string());
        if let Some(api_key) = &self.api_key {
            headers
                .entry("Authorization".to_string())
                .or_insert_with(|| format!("Bearer {api_key}"));
        }

        Ok(PreparedLlmRequest {
            provider: self.provider_id.clone(),
            api: DEEPSEEK_CHAT_API.to_string(),
            endpoint: self.endpoint.clone(),
            headers,
            body: Self::request_body(request)?,
            metadata: BTreeMap::new(),
        })
    }
}

fn map_messages(request: &LlmRequest) -> AgentCoreResult<Vec<Value>> {
    let mut messages = Vec::new();

    if let Some(instructions) = &request.instructions {
        if !instructions.trim().is_empty() {
            messages.push(json!({
                "role": "system",
                "content": instructions,
            }));
        }
    }

    for item in &request.input {
        match item {
            LlmInputItem::Message {
                role,
                content,
                metadata: _,
            } => messages.push(map_message(*role, content)?),
            LlmInputItem::FunctionCall {
                call_id,
                name,
                arguments,
            } => attach_or_push_tool_call(&mut messages, call_id, name, arguments)?,
            LlmInputItem::FunctionCallOutput {
                call_id,
                output,
                is_error,
            } => {
                let mut message = json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output_to_string(output)?,
                });
                if *is_error {
                    message["content"] = Value::String(format!(
                        "tool returned error: {}",
                        output_to_string(output)?
                    ));
                }
                messages.push(message);
            }
            LlmInputItem::Custom { value } => messages.push(value.clone()),
        }
    }

    Ok(messages)
}

fn map_message(role: LlmMessageRole, content: &[LlmContentPart]) -> AgentCoreResult<Value> {
    let mut content_fragments = Vec::new();
    let mut reasoning_fragments = Vec::new();

    for part in content {
        match part {
            LlmContentPart::Text { text } => content_fragments.push(text.clone()),
            LlmContentPart::Reasoning { text } => {
                if role == LlmMessageRole::Assistant {
                    reasoning_fragments.push(text.clone());
                } else {
                    content_fragments.push(text.clone());
                }
            }
            LlmContentPart::Json { value } | LlmContentPart::Custom { value } => {
                content_fragments.push(value.to_string());
            }
            LlmContentPart::ImageRef { uri } => {
                content_fragments.push(format!("[image: {uri}]"));
            }
            LlmContentPart::FileRef { uri } => {
                content_fragments.push(format!("[file: {uri}]"));
            }
            LlmContentPart::Diagnostic { message } => content_fragments.push(message.clone()),
        }
    }

    let mut message = Map::new();
    message.insert(
        "role".to_string(),
        Value::String(map_role(role).to_string()),
    );
    message.insert(
        "content".to_string(),
        Value::String(content_fragments.join("\n")),
    );

    if role == LlmMessageRole::Assistant && !reasoning_fragments.is_empty() {
        message.insert(
            "reasoning_content".to_string(),
            Value::String(reasoning_fragments.join("\n")),
        );
    }

    Ok(Value::Object(message))
}

fn map_role(role: LlmMessageRole) -> &'static str {
    match role {
        LlmMessageRole::User => "user",
        LlmMessageRole::Assistant => "assistant",
        LlmMessageRole::Developer | LlmMessageRole::Diagnostic => "system",
        LlmMessageRole::Tool => "tool",
    }
}

fn attach_or_push_tool_call(
    messages: &mut Vec<Value>,
    call_id: &str,
    name: &str,
    arguments: &Value,
) -> AgentCoreResult<()> {
    let tool_call = json!({
        "id": call_id,
        "type": "function",
        "function": {
            "name": name,
            "arguments": arguments_to_string(arguments)?,
        }
    });

    if let Some(last) = messages.last_mut() {
        if last.get("role").and_then(Value::as_str) == Some("assistant") {
            if last.get("content").is_none() {
                last["content"] = Value::Null;
            }
            let tool_calls = last
                .as_object_mut()
                .expect("message json object")
                .entry("tool_calls")
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Some(tool_calls) = tool_calls.as_array_mut() {
                tool_calls.push(tool_call);
                return Ok(());
            }
        }
    }

    messages.push(json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [tool_call],
    }));

    Ok(())
}

fn map_tool_schema(schema: &crate::tool_schema::ToolSchema) -> Value {
    let mut function = Map::new();
    function.insert("name".to_string(), Value::String(schema.name.clone()));
    function.insert(
        "description".to_string(),
        Value::String(schema.description.clone()),
    );
    function.insert("parameters".to_string(), schema.input_schema.clone());

    if let Some(strict) = schema.annotations.get("strict") {
        function.insert("strict".to_string(), strict.clone());
    }

    json!({
        "type": "function",
        "function": Value::Object(function),
    })
}

fn copy_provider_body_field(
    request: &LlmRequest,
    body: &mut Map<String, Value>,
    metadata_key: &str,
    body_key: &str,
) {
    if let Some(value) = request.metadata.get(metadata_key) {
        body.insert(body_key.to_string(), value.clone());
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

fn usage_event(value: &Value) -> AgentCoreResult<Option<LlmStreamEvent>> {
    let Some(usage) = value.get("usage") else {
        return Ok(None);
    };

    Ok(Some(LlmStreamEvent::Usage {
        usage: LlmUsage {
            input_tokens: usage
                .get("prompt_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            output_tokens: usage
                .get("completion_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            total_tokens: usage
                .get("total_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        },
    }))
}

fn map_completed_tool_call(tool_call: &Value) -> AgentCoreResult<LlmStreamEvent> {
    let call_id = tool_call
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| AgentCoreError::InvalidInput("tool call id missing".to_string()))?
        .to_string();
    let function = tool_call.get("function").unwrap_or(&Value::Null);
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| AgentCoreError::InvalidInput("tool call name missing".to_string()))?
        .to_string();
    let raw_arguments = function
        .get("arguments")
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let arguments = serde_json::from_str(raw_arguments)?;

    Ok(LlmStreamEvent::ToolCallCompleted {
        call_id,
        name,
        arguments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_request::{LlmContentPart, LlmInputItem, LlmMessageRole, LlmRequest};
    use crate::tool_schema::ToolSchema;
    use serde_json::json;

    #[test]
    fn maps_request_to_deepseek_chat_body() {
        let mut request = LlmRequest::new("deepseek-v4-flash");
        request.instructions = Some("system prompt".to_string());
        request.input.push(LlmInputItem::Message {
            role: LlmMessageRole::User,
            content: vec![LlmContentPart::Text {
                text: "hello".to_string(),
            }],
            metadata: BTreeMap::new(),
        });
        request.tools.push(
            ToolSchema::new(
                "search",
                "Search docs",
                json!({
                    "type": "object",
                    "required": ["query"],
                    "properties": {"query": {"type": "string"}}
                }),
            )
            .unwrap(),
        );

        let body = DeepSeekChatProvider::request_body(&request).unwrap();

        assert_eq!(body["model"], "deepseek-v4-flash");
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "hello");
        assert_eq!(body["tools"][0]["function"]["name"], "search");
    }

    #[test]
    fn maps_tool_call_and_tool_result_messages() {
        let mut request = LlmRequest::new("deepseek-v4-flash");
        request.input.push(LlmInputItem::Message {
            role: LlmMessageRole::Assistant,
            content: vec![LlmContentPart::Reasoning {
                text: "need a tool".to_string(),
            }],
            metadata: BTreeMap::new(),
        });
        request.input.push(LlmInputItem::FunctionCall {
            call_id: "call-1".to_string(),
            name: "search".to_string(),
            arguments: json!({"query": "rust"}),
        });
        request.input.push(LlmInputItem::FunctionCallOutput {
            call_id: "call-1".to_string(),
            output: json!({"answer": 42}),
            is_error: false,
        });

        let body = DeepSeekChatProvider::request_body(&request).unwrap();

        assert_eq!(body["messages"][0]["role"], "assistant");
        assert_eq!(body["messages"][0]["reasoning_content"], "need a tool");
        assert_eq!(body["messages"][0]["tool_calls"][0]["id"], "call-1");
        assert_eq!(body["messages"][1]["role"], "tool");
        assert_eq!(body["messages"][1]["tool_call_id"], "call-1");
    }

    #[test]
    fn maps_stream_reasoning_and_usage() {
        let reasoning = DeepSeekChatProvider::stream_event_from_value(&json!({
            "choices": [{
                "delta": {"reasoning_content": "think"}
            }]
        }))
        .unwrap();
        assert_eq!(
            reasoning,
            Some(LlmStreamEvent::ReasoningDelta {
                text: "think".to_string()
            })
        );

        let usage = DeepSeekChatProvider::stream_event_from_value(&json!({
            "usage": {
                "prompt_tokens": 2,
                "completion_tokens": 3,
                "total_tokens": 5
            }
        }))
        .unwrap();
        assert_eq!(
            usage,
            Some(LlmStreamEvent::Usage {
                usage: LlmUsage {
                    input_tokens: 2,
                    output_tokens: 3,
                    total_tokens: 5,
                }
            })
        );
    }

    #[test]
    fn prepare_request_redacts_debug_and_adds_auth_header() {
        let provider = DeepSeekChatProvider::new().with_api_key("secret");
        assert!(format!("{provider:?}").contains("<redacted>"));
        assert!(!format!("{provider:?}").contains("secret"));

        let mut request = LlmRequest::new("deepseek-v4-flash");
        request.input.push(LlmInputItem::Message {
            role: LlmMessageRole::User,
            content: vec![LlmContentPart::Text {
                text: "hello".to_string(),
            }],
            metadata: BTreeMap::new(),
        });

        let prepared = provider.prepare_request(&request).unwrap();
        assert_eq!(prepared.endpoint, DEEPSEEK_DEFAULT_ENDPOINT);
        assert_eq!(
            prepared.headers.get("Authorization"),
            Some(&"Bearer secret".to_string())
        );
    }
}

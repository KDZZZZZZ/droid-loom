use std::sync::{Arc, Mutex};

use agent_core::agent_definition::{AgentDefinition, AgentDefinitionBuilder, ToolVisibility};
use agent_core::assistant_builder::AssistantBuilder;
use agent_core::content_block::ContentBlock;
use agent_core::context::{ContextBuildInput, ContextBuilder};
use agent_core::llm_deepseek_chat::DeepSeekChatProvider;
use agent_core::llm_provider::{LlmProvider, PreparedLlmRequest};
use agent_core::llm_stream::LlmStreamEvent;
use agent_core::run_message::{MessageUsage, RunMessage};
use agent_core::tool_schema::ToolSchema;
use reqwest::{Proxy, blocking::Client};
use serde_json::{Value, json};

const DEFAULT_API_BASE: &str = "https://api.xiaomimimo.com/v1";
const DEFAULT_MODEL: &str = "mimo-v2.5-pro";

#[derive(Debug, Default)]
struct AgentState {
    messages: Vec<RunMessage>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema_json: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ToolTrace {
    pub name: String,
    pub input_json: String,
    pub output: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AgentResponse {
    pub answer: String,
    pub message_count: u32,
    pub tool_traces: Vec<ToolTrace>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AgentError {
    #[error("HTTP request failed: {detail}")]
    Http { detail: String },
    #[error("Model API returned {status}: {body}")]
    Api { status: String, body: String },
    #[error("Failed to parse data: {detail}")]
    Parse { detail: String },
    #[error("Model response did not contain assistant content")]
    EmptyResponse,
    #[error("Agent state lock is poisoned")]
    StateLock,
    #[error("Tool execution failed: {detail}")]
    Tool { detail: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for AgentError {
    fn from(err: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Tool {
            detail: format!("foreign callback failed: {err}"),
        }
    }
}

#[uniffi::export(with_foreign)]
pub trait PlatformToolHost: Send + Sync + std::fmt::Debug {
    fn execute_tool(&self, name: String, input_json: String) -> Result<String, AgentError>;
}

#[derive(uniffi::Object)]
pub struct AgentCore {
    api_base: String,
    api_key: String,
    model: String,
    definition: AgentDefinition,
    http: Client,
    state: Mutex<AgentState>,
    tools: Mutex<Vec<ToolSpec>>,
    platform_host: Mutex<Option<Arc<dyn PlatformToolHost>>>,
}

#[derive(Debug, Clone)]
struct CompletedToolCall {
    call_id: String,
    name: String,
    arguments: Value,
}

#[derive(Debug, Clone)]
struct AssistantTurn {
    message: RunMessage,
    answer: String,
    tool_calls: Vec<CompletedToolCall>,
    usage: Option<MessageUsage>,
}

#[uniffi::export]
impl AgentCore {
    #[uniffi::constructor]
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self::new_with_base_url(
            DEFAULT_API_BASE.to_string(),
            api_key,
            model.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
        )
    }

    #[uniffi::constructor]
    pub fn new_with_base_url(api_base: String, api_key: String, model: String) -> Self {
        Self::from_client(api_base, api_key, model, Client::new())
    }

    #[uniffi::constructor]
    pub fn new_with_base_url_and_proxy(
        api_base: String,
        api_key: String,
        model: String,
        proxy_url: String,
    ) -> Result<Self, AgentError> {
        let proxy = Proxy::all(proxy_url.trim()).map_err(|err| AgentError::Http {
            detail: format!("invalid proxy URL: {err}"),
        })?;
        let http = Client::builder()
            .proxy(proxy)
            .build()
            .map_err(|err| AgentError::Http {
                detail: format!("failed to build HTTP client: {err}"),
            })?;

        Ok(Self::from_client(api_base, api_key, model, http))
    }

    pub fn set_platform_tool_host(
        &self,
        host: Arc<dyn PlatformToolHost>,
    ) -> Result<(), AgentError> {
        let mut platform_host = self
            .platform_host
            .lock()
            .map_err(|_| AgentError::StateLock)?;
        *platform_host = Some(host);
        Ok(())
    }

    pub fn register_tool(
        &self,
        name: String,
        description: String,
        input_schema_json: String,
    ) -> Result<(), AgentError> {
        let schema = parse_json(&input_schema_json, "tool schema")?;
        ToolSchema::new(name.clone(), description.clone(), schema).map_err(map_core_error)?;

        let mut tools = self.tools.lock().map_err(|_| AgentError::StateLock)?;
        if let Some(existing) = tools.iter_mut().find(|tool| tool.name == name) {
            *existing = ToolSpec {
                name,
                description,
                input_schema_json,
            };
        } else {
            tools.push(ToolSpec {
                name,
                description,
                input_schema_json,
            });
        }
        Ok(())
    }

    pub fn list_tools(&self) -> Result<Vec<ToolSpec>, AgentError> {
        let tools = self.tools.lock().map_err(|_| AgentError::StateLock)?;
        Ok(tools.clone())
    }

    pub fn execute_tool(&self, name: String, input_json: String) -> Result<ToolTrace, AgentError> {
        self.execute_registered_tool(&name, normalize_json_object(&input_json)?.as_str())
    }

    pub fn prompt(&self, input: String) -> Result<AgentResponse, AgentError> {
        let mut state = self.state.lock().map_err(|_| AgentError::StateLock)?;
        state
            .messages
            .push(RunMessage::user(vec![ContentBlock::text(input)]).map_err(map_core_error)?);

        let assistant = self.chat(&state.messages, false)?;
        if assistant.answer.trim().is_empty() {
            return Err(AgentError::EmptyResponse);
        }
        state.messages.push(assistant.message);

        Ok(AgentResponse {
            answer: assistant.answer,
            message_count: state.messages.len() as u32,
            tool_traces: Vec::new(),
            input_tokens: assistant
                .usage
                .as_ref()
                .map(|usage| usage.input_tokens)
                .unwrap_or(0),
            output_tokens: assistant
                .usage
                .as_ref()
                .map(|usage| usage.output_tokens)
                .unwrap_or(0),
            total_tokens: assistant
                .usage
                .as_ref()
                .map(|usage| usage.total_tokens)
                .unwrap_or(0),
        })
    }

    pub fn prompt_with_tools(&self, input: String) -> Result<AgentResponse, AgentError> {
        self.prompt_with_tools_limit(input, 8)
    }

    pub fn prompt_with_tools_limit(
        &self,
        input: String,
        max_tool_rounds: u32,
    ) -> Result<AgentResponse, AgentError> {
        let mut state = self.state.lock().map_err(|_| AgentError::StateLock)?;
        state
            .messages
            .push(RunMessage::user(vec![ContentBlock::text(input)]).map_err(map_core_error)?);
        let mut tool_traces = Vec::new();
        let mut aggregate_usage = MessageUsage {
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
        };
        let max_tool_rounds = max_tool_rounds.clamp(1, 128);

        for _ in 0..max_tool_rounds {
            let assistant = self.chat(&state.messages, true)?;
            if let Some(usage) = assistant.usage.as_ref() {
                aggregate_usage.input_tokens += usage.input_tokens;
                aggregate_usage.output_tokens += usage.output_tokens;
                aggregate_usage.total_tokens += usage.total_tokens;
            }
            if assistant.tool_calls.is_empty() {
                if assistant.answer.trim().is_empty() {
                    return Err(AgentError::EmptyResponse);
                }
                let answer = assistant.answer.clone();
                state.messages.push(assistant.message);
                return Ok(AgentResponse {
                    answer,
                    message_count: state.messages.len() as u32,
                    tool_traces,
                    input_tokens: aggregate_usage.input_tokens,
                    output_tokens: aggregate_usage.output_tokens,
                    total_tokens: aggregate_usage.total_tokens,
                });
            }

            state.messages.push(assistant.message);
            for call in assistant.tool_calls {
                let input_json = normalize_json_object(&call.arguments.to_string())?;
                let trace = self.execute_registered_tool(&call.name, input_json.as_str())?;
                let output = parse_json(&trace.output, "tool output")
                    .unwrap_or_else(|_| Value::String(trace.output.clone()));
                state.messages.push(
                    RunMessage::tool(vec![ContentBlock::tool_result(
                        call.call_id,
                        Some(call.name),
                        output,
                        false,
                    )])
                    .map_err(map_core_error)?,
                );
                tool_traces.push(trace);
            }
        }

        Err(AgentError::Tool {
            detail: format!("too many tool-call rounds: limit={max_tool_rounds}"),
        })
    }

    pub fn reset(&self) -> Result<(), AgentError> {
        let mut state = self.state.lock().map_err(|_| AgentError::StateLock)?;
        state.messages.clear();
        Ok(())
    }

    pub fn message_count(&self) -> Result<u32, AgentError> {
        let state = self.state.lock().map_err(|_| AgentError::StateLock)?;
        Ok(state.messages.len() as u32)
    }

    pub fn model(&self) -> String {
        self.model.clone()
    }
}

impl AgentCore {
    fn from_client(api_base: String, api_key: String, model: String, http: Client) -> Self {
        Self {
            api_base,
            api_key,
            model,
            definition: default_definition(),
            http,
            state: Mutex::new(AgentState::default()),
            tools: Mutex::new(Vec::new()),
            platform_host: Mutex::new(None),
        }
    }

    fn chat(
        &self,
        history: &[RunMessage],
        include_tools: bool,
    ) -> Result<AssistantTurn, AgentError> {
        let prepared = self.prepare_request(history, include_tools)?;
        let mut request = self.http.post(&prepared.endpoint);
        for (name, value) in prepared.headers {
            request = request.header(name.as_str(), value.as_str());
        }

        let response = request
            .json(&prepared.body)
            .send()
            .map_err(|err| AgentError::Http {
                detail: format!("{err:?}"),
            })?;

        let status = response.status();
        let text = response.text().map_err(|err| AgentError::Http {
            detail: format!("{err:?}"),
        })?;

        if !status.is_success() {
            return Err(AgentError::Api {
                status: status.to_string(),
                body: text,
            });
        }

        let value: Value = serde_json::from_str(&text).map_err(|err| AgentError::Parse {
            detail: format!("provider response is not JSON: {err}"),
        })?;
        let events =
            DeepSeekChatProvider::response_events_from_value(&value).map_err(map_core_error)?;
        assistant_turn_from_events(events)
    }

    fn prepare_request(
        &self,
        history: &[RunMessage],
        include_tools: bool,
    ) -> Result<PreparedLlmRequest, AgentError> {
        let mut input = ContextBuildInput::new(self.model.clone());
        input.run_messages = history.to_vec();
        input
            .metadata
            .insert("deepseek.stream".to_string(), json!(false));
        input.metadata.insert(
            "deepseek.thinking".to_string(),
            json!({ "type": "disabled" }),
        );

        if include_tools {
            input.instructions_override = Some(format!(
                "{} Use registered tools for Android device actions. Never claim a device action happened unless a tool result confirms it.",
                self.definition.system_prompt()
            ));
            input.visible_tool_schemas = self.visible_tool_schemas()?;
            if !input.visible_tool_schemas.is_empty() {
                input
                    .metadata
                    .insert("deepseek.tool_choice".to_string(), json!("auto"));
            }
        }

        let llm_request = ContextBuilder::new()
            .build(&self.definition, input)
            .map_err(map_core_error)?;
        let endpoint = format!("{}/chat/completions", self.api_base.trim_end_matches('/'));
        DeepSeekChatProvider::new()
            .with_endpoint(endpoint)
            .with_api_key(self.api_key.clone())
            .prepare_request(&llm_request)
            .map_err(map_core_error)
    }

    fn visible_tool_schemas(&self) -> Result<Vec<ToolSchema>, AgentError> {
        let tools = self.tools.lock().map_err(|_| AgentError::StateLock)?;
        tools
            .iter()
            .map(|tool| {
                let schema = parse_json(&tool.input_schema_json, "tool schema")?;
                ToolSchema::new(tool.name.clone(), tool.description.clone(), schema)
                    .map_err(map_core_error)
            })
            .collect()
    }

    fn execute_registered_tool(
        &self,
        name: &str,
        input_json: &str,
    ) -> Result<ToolTrace, AgentError> {
        {
            let tools = self.tools.lock().map_err(|_| AgentError::StateLock)?;
            if !tools.iter().any(|tool| tool.name == name) {
                return Err(AgentError::Tool {
                    detail: format!("tool is not registered: {name}"),
                });
            }
        }

        let host = self
            .platform_host
            .lock()
            .map_err(|_| AgentError::StateLock)?
            .clone()
            .ok_or_else(|| AgentError::Tool {
                detail: "platform tool host is not set".to_string(),
            })?;

        let output = host.execute_tool(name.to_string(), input_json.to_string())?;
        Ok(ToolTrace {
            name: name.to_string(),
            input_json: input_json.to_string(),
            output,
        })
    }
}

fn default_definition() -> AgentDefinition {
    AgentDefinitionBuilder::new()
        .name("android_agent")
        .system_prompt("You are a concise helpful assistant.")
        .tool_visibility("android_device_info", ToolVisibility::Direct)
        .tool_visibility("android_battery", ToolVisibility::Direct)
        .tool_visibility("android_show_toast", ToolVisibility::Direct)
        .tool_visibility("android_set_clipboard", ToolVisibility::Direct)
        .tool_visibility("android_open_settings", ToolVisibility::Searchable)
        .build()
        .expect("default agent definition is valid")
}

fn assistant_turn_from_events(events: Vec<LlmStreamEvent>) -> Result<AssistantTurn, AgentError> {
    let mut builder = AssistantBuilder::new();
    let mut answer = String::new();
    let mut tool_calls = Vec::new();
    let mut usage = None;

    for event in events {
        match event {
            LlmStreamEvent::TextDelta { text } => {
                answer.push_str(&text);
                builder.push_text_delta(text);
            }
            LlmStreamEvent::ReasoningDelta { text } => {
                builder.push_reasoning_delta(text);
            }
            LlmStreamEvent::ToolCallCompleted {
                call_id,
                name,
                arguments,
            } => {
                builder.push_tool_call(call_id.clone(), name.clone(), arguments.clone());
                tool_calls.push(CompletedToolCall {
                    call_id,
                    name,
                    arguments,
                });
            }
            LlmStreamEvent::Usage { usage: event_usage } => {
                usage = Some(MessageUsage {
                    input_tokens: event_usage.input_tokens,
                    output_tokens: event_usage.output_tokens,
                    total_tokens: event_usage.total_tokens,
                });
            }
            LlmStreamEvent::Error { message, .. } => {
                return Err(AgentError::Api {
                    status: "provider_error".to_string(),
                    body: message,
                });
            }
            LlmStreamEvent::PreparedRequest { .. }
            | LlmStreamEvent::ResponseCreated { .. }
            | LlmStreamEvent::ToolCallDelta { .. }
            | LlmStreamEvent::Completed { .. } => {}
        }
    }

    let mut message = builder.finish().map_err(map_core_error)?;
    if let Some(usage) = usage.clone() {
        message = message.with_usage(usage);
    }
    Ok(AssistantTurn {
        message,
        answer,
        tool_calls,
        usage,
    })
}

fn parse_json(input: &str, label: &str) -> Result<Value, AgentError> {
    serde_json::from_str(input).map_err(|err| AgentError::Parse {
        detail: format!("{label} is invalid JSON: {err}"),
    })
}

fn normalize_json_object(input: &str) -> Result<String, AgentError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok("{}".to_string());
    }

    let value: Value = serde_json::from_str(trimmed).map_err(|err| AgentError::Parse {
        detail: format!("tool input is invalid JSON: {err}; input={trimmed}"),
    })?;

    if !value.is_object() {
        return Err(AgentError::Parse {
            detail: format!("tool input must be a JSON object: {trimmed}"),
        });
    }

    Ok(value.to_string())
}

fn map_core_error(error: agent_core::AgentCoreError) -> AgentError {
    AgentError::Parse {
        detail: format!("agent-core: {error}"),
    }
}

uniffi::setup_scaffolding!();

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::llm_stream::LlmUsage;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn assistant_turn_preserves_usage() {
        let turn = assistant_turn_from_events(vec![
            LlmStreamEvent::TextDelta {
                text: "done".to_string(),
            },
            LlmStreamEvent::Usage {
                usage: LlmUsage {
                    input_tokens: 10,
                    output_tokens: 3,
                    total_tokens: 13,
                },
            },
        ])
        .unwrap();

        assert_eq!(turn.usage.unwrap().total_tokens, 13);
        assert_eq!(turn.message.usage.unwrap().input_tokens, 10);
    }

    #[test]
    fn prompt_with_tools_runs_provider_tool_loop_against_mock_chat_api() {
        #[derive(Debug)]
        struct MockHost;

        impl PlatformToolHost for MockHost {
            fn execute_tool(&self, name: String, input_json: String) -> Result<String, AgentError> {
                assert_eq!(name, "android_map_goal_task");
                let input: Value = serde_json::from_str(&input_json).unwrap();
                assert_eq!(input["cycles"], 6);
                Ok(json!({
                    "ok": true,
                    "completed": true,
                    "steps": 111,
                    "session_message_count": 224,
                    "probability_graph": {
                        "tool_call_events": 111,
                        "transition_count": 110
                    }
                })
                .to_string())
            }
        }

        let server = MockChatServer::start(vec![
            json!({
                "id": "chatcmpl-tool",
                "object": "chat.completion",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": "call-map-goal",
                            "type": "function",
                            "function": {
                                "name": "android_map_goal_task",
                                "arguments": "{\"cycles\":6}"
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 10,
                    "total_tokens": 110
                }
            }),
            json!({
                "id": "chatcmpl-final",
                "object": "chat.completion",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "地图任务完成：111 steps，224 session messages。"
                    },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 80,
                    "completion_tokens": 12,
                    "total_tokens": 92
                }
            }),
        ]);

        let agent = AgentCore::new_with_base_url(
            server.base_url(),
            "test-key".to_string(),
            "mock-model".to_string(),
        );
        agent.set_platform_tool_host(Arc::new(MockHost)).unwrap();
        agent
            .register_tool(
                "android_map_goal_task".to_string(),
                "Run map goal task.".to_string(),
                r#"{"type":"object","properties":{"cycles":{"type":"integer"}},"additionalProperties":false}"#.to_string(),
            )
            .unwrap();

        let response = agent
            .prompt_with_tools_limit("run the Android map task".to_string(), 4)
            .unwrap();

        assert!(response.answer.contains("111 steps"));
        assert_eq!(response.tool_traces.len(), 1);
        assert_eq!(response.tool_traces[0].name, "android_map_goal_task");
        assert_eq!(response.message_count, 4);
        assert_eq!(response.input_tokens, 180);
        assert_eq!(response.output_tokens, 22);
        assert_eq!(response.total_tokens, 202);
        assert_eq!(server.request_count(), 2);
    }

    #[test]
    fn prompt_with_tools_handles_single_graph_hundred_phone_tool_calls() {
        #[derive(Debug)]
        struct PhoneHost;

        impl PlatformToolHost for PhoneHost {
            fn execute_tool(
                &self,
                name: String,
                _input_json: String,
            ) -> Result<String, AgentError> {
                assert!(!name.contains("goal_task"));
                assert!(!name.contains("long_task_smoke"));
                Ok(json!({ "ok": true, "tool": name }).to_string())
            }
        }

        fn phone_tool(index: usize) -> (&'static str, Value) {
            match index % 10 {
                0 => ("android_device_info", json!({})),
                1 => ("android_battery", json!({})),
                2 => ("android_open_settings", json!({ "screen": "settings" })),
                3 => (
                    "android_map_observe",
                    json!({ "task_hint": "settings overview" }),
                ),
                4 => (
                    "android_map_plan_path",
                    json!({ "query": "settings", "limit": 5 }),
                ),
                5 => ("android_open_settings", json!({ "screen": "wifi" })),
                6 => (
                    "android_map_observe",
                    json!({ "task_hint": "wifi readiness" }),
                ),
                7 => ("android_global_action", json!({ "action": "home" })),
                8 => (
                    "android_map_observe",
                    json!({ "task_hint": "launcher checkpoint" }),
                ),
                _ if index % 20 == 9 => (
                    "android_set_clipboard",
                    json!({ "text": "phone audit checkpoint" }),
                ),
                _ => (
                    "android_show_toast",
                    json!({ "message": "phone audit checkpoint" }),
                ),
            }
        }

        let mut responses = Vec::new();
        for index in 0..100 {
            let (name, arguments) = phone_tool(index);
            responses.push(json!({
                "id": format!("chatcmpl-tool-{index}"),
                "object": "chat.completion",
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [{
                            "id": format!("call-phone-{index}"),
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": arguments.to_string()
                            }
                        }]
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {
                    "prompt_tokens": 1,
                    "completion_tokens": 1,
                    "total_tokens": 2
                }
            }));
        }
        responses.push(json!({
            "id": "chatcmpl-final",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "单次 graph loop 完成：100 次 primitive phone tool calls。"
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 1,
                "total_tokens": 2
            }
        }));

        let server = MockChatServer::start(responses);
        let agent = AgentCore::new_with_base_url(
            server.base_url(),
            "test-key".to_string(),
            "mock-model".to_string(),
        );
        agent.set_platform_tool_host(Arc::new(PhoneHost)).unwrap();
        for tool_name in [
            "android_device_info",
            "android_battery",
            "android_open_settings",
            "android_map_observe",
            "android_map_plan_path",
            "android_global_action",
            "android_set_clipboard",
            "android_show_toast",
        ] {
            agent
                .register_tool(
                    tool_name.to_string(),
                    "Primitive phone tool.".to_string(),
                    r#"{"type":"object","properties":{},"additionalProperties":true}"#.to_string(),
                )
                .unwrap();
        }

        let response = agent
            .prompt_with_tools_limit(
                "run one graph loop for 100 phone tool calls".to_string(),
                128,
            )
            .unwrap();

        assert!(response.answer.contains("100"));
        assert_eq!(response.tool_traces.len(), 100);
        assert_eq!(response.message_count, 202);
        assert_eq!(response.total_tokens, 202);
        assert_eq!(server.request_count(), 101);
        assert!(
            response
                .tool_traces
                .iter()
                .all(|trace| !trace.name.contains("goal_task"))
        );
    }

    struct MockChatServer {
        endpoint: String,
        handle: Option<thread::JoinHandle<Vec<String>>>,
    }

    impl MockChatServer {
        fn start(responses: Vec<Value>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let handle = thread::spawn(move || {
                let mut bodies = Vec::new();
                for response in responses {
                    let (mut stream, _) = listener.accept().unwrap();
                    let mut request = Vec::new();
                    let mut buffer = [0_u8; 4096];
                    loop {
                        let read = stream.read(&mut buffer).unwrap();
                        request.extend_from_slice(&buffer[..read]);
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request_text = String::from_utf8_lossy(&request);
                    let content_length = request_text
                        .lines()
                        .find_map(|line| {
                            line.strip_prefix("content-length:")
                                .or_else(|| line.strip_prefix("Content-Length:"))
                        })
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    let header_end = request
                        .windows(4)
                        .position(|window| window == b"\r\n\r\n")
                        .map(|position| position + 4)
                        .unwrap();
                    let already_read = request.len().saturating_sub(header_end);
                    let remaining = content_length.saturating_sub(already_read);
                    if remaining > 0 {
                        let mut body_rest = vec![0_u8; remaining];
                        stream.read_exact(&mut body_rest).unwrap();
                        request.extend_from_slice(&body_rest);
                    }
                    let body = String::from_utf8_lossy(&request[header_end..]).to_string();
                    bodies.push(body);

                    let response_body = response.to_string();
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        response_body.len(),
                        response_body
                    )
                    .unwrap();
                }
                bodies
            });

            Self {
                endpoint,
                handle: Some(handle),
            }
        }

        fn base_url(&self) -> String {
            self.endpoint.clone()
        }

        fn request_count(mut self) -> usize {
            self.handle.take().unwrap().join().unwrap().len()
        }
    }
}

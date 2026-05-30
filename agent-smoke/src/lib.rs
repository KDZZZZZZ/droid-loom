use std::sync::{Arc, Mutex};

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Proxy, blocking::Client};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

impl ChatMessage {
    fn system(content: String) -> Self {
        Self {
            role: "system".to_string(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn user(content: String) -> Self {
        Self {
            role: "user".to_string(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn assistant(content: String) -> Self {
        Self {
            role: "assistant".to_string(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    fn tool(tool_call_id: String, content: String) -> Self {
        Self {
            role: "tool".to_string(),
            content: Some(content),
            tool_calls: None,
            tool_call_id: Some(tool_call_id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: ToolFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Default)]
struct AgentState {
    messages: Vec<ChatMessage>,
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
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AgentError {
    #[error("HTTP request failed: {detail}")]
    Http { detail: String },
    #[error("DeepSeek API returned {status}: {body}")]
    Api { status: String, body: String },
    #[error("Failed to parse data: {detail}")]
    Parse { detail: String },
    #[error("DeepSeek response did not contain assistant content")]
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
    system_prompt: String,
    http: Client,
    state: Mutex<AgentState>,
    tools: Mutex<Vec<ToolSpec>>,
    platform_host: Mutex<Option<Arc<dyn PlatformToolHost>>>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[uniffi::export]
impl AgentCore {
    #[uniffi::constructor]
    pub fn new(api_key: String, model: Option<String>) -> Self {
        Self::new_with_base_url(
            "https://api.deepseek.com".to_string(),
            api_key,
            model.unwrap_or_else(|| "deepseek-v4-pro".to_string()),
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
        validate_json_schema(&input_schema_json)?;

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
        state.messages.push(ChatMessage::user(input));

        let message = self.chat(&state.messages, false)?;
        let answer = message
            .content
            .filter(|content| !content.trim().is_empty())
            .ok_or(AgentError::EmptyResponse)?;

        state.messages.push(ChatMessage::assistant(answer.clone()));

        Ok(AgentResponse {
            answer,
            message_count: state.messages.len() as u32,
            tool_traces: Vec::new(),
        })
    }

    pub fn prompt_with_tools(&self, input: String) -> Result<AgentResponse, AgentError> {
        let mut state = self.state.lock().map_err(|_| AgentError::StateLock)?;
        state.messages.push(ChatMessage::user(input));
        let mut tool_traces = Vec::new();

        for _ in 0..4 {
            let assistant = self.chat(&state.messages, true)?;
            match assistant.tool_calls.clone() {
                Some(tool_calls) if !tool_calls.is_empty() => {
                    state.messages.push(assistant);
                    for tool_call in tool_calls {
                        let input_json = normalize_json_object(&tool_call.function.arguments)?;
                        let trace = self.execute_registered_tool(
                            &tool_call.function.name,
                            input_json.as_str(),
                        )?;
                        state
                            .messages
                            .push(ChatMessage::tool(tool_call.id, trace.output.clone()));
                        tool_traces.push(trace);
                    }
                }
                _ => {
                    let answer = assistant
                        .content
                        .filter(|content| !content.trim().is_empty())
                        .ok_or(AgentError::EmptyResponse)?;
                    state.messages.push(ChatMessage::assistant(answer.clone()));
                    return Ok(AgentResponse {
                        answer,
                        message_count: state.messages.len() as u32,
                        tool_traces,
                    });
                }
            }
        }

        Err(AgentError::Tool {
            detail: "too many tool-call rounds".to_string(),
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
            system_prompt: "You are a concise helpful assistant.".to_string(),
            http,
            state: Mutex::new(AgentState::default()),
            tools: Mutex::new(Vec::new()),
            platform_host: Mutex::new(None),
        }
    }

    fn chat(
        &self,
        history: &[ChatMessage],
        include_tools: bool,
    ) -> Result<ChatMessage, AgentError> {
        let mut messages = vec![ChatMessage::system(self.system_prompt(include_tools))];
        messages.extend_from_slice(history);

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "thinking": { "type": "enabled" },
            "reasoning_effort": "high",
            "stream": false
        });

        if include_tools {
            let tools = self.tools_for_api()?;
            if !tools.is_empty() {
                body["tools"] = Value::Array(tools);
                body["tool_choice"] = Value::String("auto".to_string());
            }
        }

        let url = format!("{}/chat/completions", self.api_base.trim_end_matches('/'));
        let response = self
            .http
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .json(&body)
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

        let parsed: ChatCompletionResponse =
            serde_json::from_str(&text).map_err(|err| AgentError::Parse {
                detail: err.to_string(),
            })?;

        parsed
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message)
            .ok_or(AgentError::EmptyResponse)
    }

    fn system_prompt(&self, include_tools: bool) -> String {
        if include_tools {
            format!(
                "{} Use registered tools for Android device actions. Never claim a device action happened unless a tool result confirms it.",
                self.system_prompt
            )
        } else {
            self.system_prompt.clone()
        }
    }

    fn tools_for_api(&self) -> Result<Vec<Value>, AgentError> {
        let tools = self.tools.lock().map_err(|_| AgentError::StateLock)?;
        tools
            .iter()
            .map(|tool| {
                let parameters: Value =
                    serde_json::from_str(&tool.input_schema_json).map_err(|err| {
                        AgentError::Parse {
                            detail: format!("tool schema for {} is invalid: {err}", tool.name),
                        }
                    })?;
                Ok(json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": parameters
                    }
                }))
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

fn validate_json_schema(input: &str) -> Result<(), AgentError> {
    serde_json::from_str::<Value>(input).map_err(|err| AgentError::Parse {
        detail: format!("tool schema is invalid JSON: {err}"),
    })?;
    Ok(())
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

uniffi::setup_scaffolding!();

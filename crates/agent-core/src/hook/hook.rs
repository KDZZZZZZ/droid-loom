use crate::run_message::RunMessage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "name")]
pub enum HookName {
    Input,
    BeforeAgentStart,
    Context,
    BeforeProviderRequest,
    AfterProviderResponse,
    ToolCall,
    ToolResult,
    BeforeSessionCommit,
    ProviderRequest,
    ToolExecution,
    NodeExecution,
    CustomPoint(String),
    CustomWrapper(String),
}

impl HookName {
    pub fn kind(&self) -> HookKind {
        match self {
            Self::ProviderRequest
            | Self::ToolExecution
            | Self::NodeExecution
            | Self::CustomWrapper(_) => HookKind::Wrapper,
            Self::Input
            | Self::BeforeAgentStart
            | Self::Context
            | Self::BeforeProviderRequest
            | Self::AfterProviderResponse
            | Self::ToolCall
            | Self::ToolResult
            | Self::BeforeSessionCommit
            | Self::CustomPoint(_) => HookKind::Point,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookKind {
    Point,
    Wrapper,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HookPayload {
    pub hook: HookName,
    pub data: Value,
    pub metadata: BTreeMap<String, Value>,
}

impl HookPayload {
    pub fn new(hook: HookName) -> Self {
        Self {
            hook,
            data: Value::Null,
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum PointHookDecision {
    Continue,
    Rewrite(HookPayload),
    Block { reason: String },
    Stop { reason: String },
    Emit { event: HookEventRequest },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WrapperRequest {
    pub hook: HookName,
    pub data: Value,
    pub metadata: BTreeMap<String, Value>,
}

impl WrapperRequest {
    pub fn new(hook: HookName) -> Self {
        Self {
            hook,
            data: Value::Null,
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WrapperResponse {
    pub data: Value,
    pub metadata: BTreeMap<String, Value>,
    pub messages: Vec<RunMessage>,
}

impl WrapperResponse {
    pub fn new(data: Value) -> Self {
        Self {
            data,
            metadata: BTreeMap::new(),
            messages: Vec::new(),
        }
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum WrapperResult {
    Continue(WrapperResponse),
    Rewrite(WrapperResponse),
    Retry { reason: String },
    Recover { messages: Vec<RunMessage> },
    Stop { reason: String },
    Fail { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HookEventRequest {
    pub name: String,
    pub data: Value,
}

impl HookEventRequest {
    pub fn new(name: impl Into<String>, data: Value) -> Self {
        Self {
            name: name.into(),
            data,
        }
    }
}

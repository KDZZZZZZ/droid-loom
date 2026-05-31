use crate::agent_definition::ToolVisibility;
use crate::error::AgentCoreResult;
use crate::tool_schema::ToolSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Debug;

pub trait Tool: Debug + Send + Sync {
    fn metadata(&self) -> &ToolMetadata;

    fn invoke(&self, invocation: ToolInvocation) -> AgentCoreResult<ToolOutput>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub schema: ToolSchema,
    pub default_visibility: ToolVisibility,
    #[serde(default)]
    pub capabilities: ToolCapabilities,
    #[serde(default)]
    pub execution: ToolExecutionMetadata,
}

impl ToolMetadata {
    pub fn new(schema: ToolSchema, default_visibility: ToolVisibility) -> Self {
        Self {
            schema,
            default_visibility,
            capabilities: ToolCapabilities::default(),
            execution: ToolExecutionMetadata::default(),
        }
    }

    pub fn name(&self) -> &str {
        &self.schema.name
    }

    pub fn can_preexecute(&self) -> bool {
        self.capabilities.read_only
            && self.capabilities.idempotent
            && !self.capabilities.destructive
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCapabilities {
    pub read_only: bool,
    pub idempotent: bool,
    pub destructive: bool,
    pub requires_network: bool,
    pub supports_streaming: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolExecutionMetadata {
    pub timeout_ms: Option<u64>,
    pub interruptible: bool,
    pub result_policy: ToolResultPolicy,
}

impl Default for ToolExecutionMetadata {
    fn default() -> Self {
        Self {
            timeout_ms: None,
            interruptible: true,
            result_policy: ToolResultPolicy::ReturnFull,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultPolicy {
    ReturnFull,
    ReturnSummary,
    Hidden,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolInvocation {
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

impl ToolInvocation {
    pub fn new(call_id: impl Into<String>, tool_name: impl Into<String>, arguments: Value) -> Self {
        Self {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            arguments,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolOutput {
    pub output: Value,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

impl ToolOutput {
    pub fn new(output: Value) -> Self {
        Self {
            output,
            metadata: BTreeMap::new(),
        }
    }
}

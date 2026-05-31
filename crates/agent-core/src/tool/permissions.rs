use crate::error::AgentCoreResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub trait ToolPermissionPolicy: Send + Sync {
    fn decide(&self, context: &ToolPermissionContext) -> AgentCoreResult<ToolPermissionDecision>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolPermissionContext {
    pub agent_name: String,
    pub call_id: String,
    pub tool_name: String,
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

impl ToolPermissionContext {
    pub fn new(
        agent_name: impl Into<String>,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: Value,
    ) -> Self {
        Self {
            agent_name: agent_name.into(),
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            arguments,
            metadata: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum ToolPermissionDecision {
    Allow {
        #[serde(default)]
        arguments: Option<Value>,
    },
    Ask {
        reason: String,
    },
    Deny {
        reason: String,
    },
    Passthrough,
}

impl ToolPermissionDecision {
    pub fn allowed_arguments(self, original: Value) -> Option<Value> {
        match self {
            Self::Allow { arguments } => Some(arguments.unwrap_or(original)),
            Self::Passthrough => Some(original),
            Self::Ask { .. } | Self::Deny { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AllowAllToolPermissionPolicy;

impl ToolPermissionPolicy for AllowAllToolPermissionPolicy {
    fn decide(&self, _context: &ToolPermissionContext) -> AgentCoreResult<ToolPermissionDecision> {
        Ok(ToolPermissionDecision::Allow { arguments: None })
    }
}

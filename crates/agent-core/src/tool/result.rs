use crate::content_block::ContentBlock;
use crate::error::AgentCoreResult;
use crate::RunMessage;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub tool_name: String,
    pub status: ToolResultStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<ToolResultContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ToolResultError>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

impl ToolResult {
    pub fn success(
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        output: Value,
    ) -> Self {
        Self {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            status: ToolResultStatus::Success,
            content: vec![ToolResultContent::Json(output.clone())],
            raw_output: Some(output),
            error: None,
            metadata: BTreeMap::new(),
        }
    }

    pub fn denied(
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        let reason = reason.into();
        Self {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            status: ToolResultStatus::Denied,
            content: vec![ToolResultContent::Text(reason.clone())],
            raw_output: None,
            error: Some(ToolResultError {
                message: reason,
                recoverable: true,
            }),
            metadata: BTreeMap::new(),
        }
    }

    pub fn failed(
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        message: impl Into<String>,
        recoverable: bool,
    ) -> Self {
        let message = message.into();
        Self {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            status: ToolResultStatus::Error,
            content: vec![ToolResultContent::Text(message.clone())],
            raw_output: None,
            error: Some(ToolResultError {
                message,
                recoverable,
            }),
            metadata: BTreeMap::new(),
        }
    }

    pub fn is_error(&self) -> bool {
        !matches!(self.status, ToolResultStatus::Success)
    }

    pub fn output_for_model(&self) -> Value {
        if let Some(output) = &self.raw_output {
            return output.clone();
        }

        if self.content.len() == 1 {
            return self.content[0].to_value();
        }

        Value::Array(
            self.content
                .iter()
                .map(ToolResultContent::to_value)
                .collect(),
        )
    }

    pub fn to_content_block_value(&self) -> Value {
        json!({
            "type": "tool_result",
            "call_id": self.call_id,
            "tool_name": self.tool_name,
            "is_error": self.is_error(),
            "status": self.status,
            "content": self.content,
            "output": self.output_for_model(),
            "error": self.error,
            "metadata": self.metadata,
        })
    }

    pub fn to_run_message_value(&self) -> Value {
        json!({
            "role": "tool",
            "content": [self.to_content_block_value()],
            "metadata": {
                "tool_name": self.tool_name,
                "call_id": self.call_id,
            }
        })
    }

    pub fn into_run_message(&self) -> AgentCoreResult<RunMessage> {
        RunMessage::tool(vec![ContentBlock::tool_result(
            self.call_id.clone(),
            Some(self.tool_name.clone()),
            self.output_for_model(),
            self.is_error(),
        )])
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolResultStatus {
    Success,
    Denied,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ToolResultContent {
    Text(String),
    Json(Value),
}

impl ToolResultContent {
    pub fn to_value(&self) -> Value {
        match self {
            Self::Text(text) => Value::String(text.clone()),
            Self::Json(value) => value.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultError {
    pub message: String,
    pub recoverable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wraps_tool_result_as_tool_message_value() {
        let result = ToolResult::success("call-1", "search", json!({"answer": 42}));
        let message = result.to_run_message_value();

        assert_eq!(message["role"], "tool");
        assert_eq!(message["content"][0]["type"], "tool_result");
        assert_eq!(message["content"][0]["call_id"], "call-1");
        assert_eq!(message["content"][0]["output"], json!({"answer": 42}));
    }

    #[test]
    fn wraps_tool_result_as_run_message() {
        let result = ToolResult::success("call-1", "search", json!({"answer": 42}));
        let message = result.into_run_message().unwrap();

        assert_eq!(message.role, crate::run_message::MessageRole::Tool);
        assert_eq!(
            message.content,
            vec![ContentBlock::tool_result(
                "call-1",
                Some("search".to_string()),
                json!({"answer": 42}),
                false
            )]
        );
    }
}

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::content_block::ContentBlock;
use crate::error::{AgentCoreError, AgentCoreResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
    Diagnostic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    Streaming,
    Finalized,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunMessage {
    pub id: Uuid,
    pub role: MessageRole,
    pub content: Vec<ContentBlock>,
    pub status: MessageStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<MessageUsage>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
    pub created_at_ms: u64,
}

impl RunMessage {
    pub fn new(role: MessageRole, content: Vec<ContentBlock>) -> AgentCoreResult<Self> {
        if content.is_empty() {
            return Err(AgentCoreError::InvalidInput(
                "run message content cannot be empty".to_string(),
            ));
        }

        Ok(Self {
            id: Uuid::new_v4(),
            role,
            content,
            status: MessageStatus::Finalized,
            source_node_id: None,
            provider_response_id: None,
            usage: None,
            metadata: BTreeMap::new(),
            created_at_ms: now_ms(),
        })
    }

    pub fn streaming(role: MessageRole) -> Self {
        Self {
            id: Uuid::new_v4(),
            role,
            content: Vec::new(),
            status: MessageStatus::Streaming,
            source_node_id: None,
            provider_response_id: None,
            usage: None,
            metadata: BTreeMap::new(),
            created_at_ms: now_ms(),
        }
    }

    pub fn user(content: Vec<ContentBlock>) -> AgentCoreResult<Self> {
        Self::new(MessageRole::User, content)
    }

    pub fn user_text(text: impl Into<String>) -> AgentCoreResult<Self> {
        Self::user(vec![ContentBlock::text(text)])
    }

    pub fn assistant(content: Vec<ContentBlock>) -> AgentCoreResult<Self> {
        Self::new(MessageRole::Assistant, content)
    }

    pub fn assistant_text(text: impl Into<String>) -> AgentCoreResult<Self> {
        Self::assistant(vec![ContentBlock::text(text)])
    }

    pub fn assistant_tool_call(
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: Value,
    ) -> AgentCoreResult<Self> {
        Self::assistant(vec![ContentBlock::tool_call(call_id, tool_name, arguments)])
    }

    pub fn tool(content: Vec<ContentBlock>) -> AgentCoreResult<Self> {
        Self::new(MessageRole::Tool, content)
    }

    pub fn tool_result(
        call_id: impl Into<String>,
        tool_name: Option<String>,
        output: Value,
        is_error: bool,
    ) -> AgentCoreResult<Self> {
        Self::tool(vec![ContentBlock::tool_result(
            call_id, tool_name, output, is_error,
        )])
    }

    pub fn diagnostic(content: Vec<ContentBlock>) -> AgentCoreResult<Self> {
        Self::new(MessageRole::Diagnostic, content)
    }

    pub fn finalize(&mut self) -> AgentCoreResult<()> {
        if self.content.is_empty() {
            return Err(AgentCoreError::InvalidInput(
                "cannot finalize an empty run message".to_string(),
            ));
        }
        self.status = MessageStatus::Finalized;
        Ok(())
    }

    pub fn abort(&mut self) {
        self.status = MessageStatus::Aborted;
    }

    pub fn push_content(&mut self, block: ContentBlock) {
        self.content.push(block);
    }

    pub fn with_source_node_id(mut self, source_node_id: impl Into<String>) -> Self {
        self.source_node_id = Some(source_node_id.into());
        self
    }

    pub fn set_source_node_id_if_empty(&mut self, source_node_id: impl Into<String>) {
        if self.source_node_id.is_none() {
            self.source_node_id = Some(source_node_id.into());
        }
    }

    pub fn with_provider_response_id(mut self, provider_response_id: impl Into<String>) -> Self {
        self.provider_response_id = Some(provider_response_id.into());
        self
    }

    pub fn with_usage(mut self, usage: MessageUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_requires_content() {
        let result = RunMessage::user(Vec::new());

        assert!(matches!(result, Err(AgentCoreError::InvalidInput(_))));
    }

    #[test]
    fn streaming_message_can_be_finalized_after_content_arrives() {
        let mut message = RunMessage::streaming(MessageRole::Assistant);

        message.push_content(ContentBlock::text("done"));
        message.finalize().unwrap();

        assert_eq!(message.status, MessageStatus::Finalized);
    }
}

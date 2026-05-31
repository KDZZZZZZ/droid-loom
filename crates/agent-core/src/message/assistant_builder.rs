use serde_json::Value;

use crate::content_block::{ContentBlock, DiagnosticLevel};
use crate::error::{AgentCoreError, AgentCoreResult};
use crate::run_message::{MessageRole, RunMessage};

#[derive(Debug, Clone)]
pub struct AssistantBuilder {
    message: RunMessage,
}

impl AssistantBuilder {
    pub fn new() -> Self {
        Self {
            message: RunMessage::streaming(MessageRole::Assistant),
        }
    }

    pub fn push_text_delta(&mut self, delta: impl AsRef<str>) {
        self.push_text_like_delta(delta.as_ref(), TextLikeKind::Text);
    }

    pub fn push_reasoning_delta(&mut self, delta: impl AsRef<str>) {
        self.push_text_like_delta(delta.as_ref(), TextLikeKind::Reasoning);
    }

    pub fn push_tool_call(
        &mut self,
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: Value,
    ) {
        self.message
            .push_content(ContentBlock::tool_call(call_id, tool_name, arguments));
    }

    pub fn push_diagnostic(&mut self, level: DiagnosticLevel, message: impl Into<String>) {
        self.message
            .push_content(ContentBlock::diagnostic(level, message));
    }

    pub fn snapshot(&self) -> RunMessage {
        self.message.clone()
    }

    pub fn finish(mut self) -> AgentCoreResult<RunMessage> {
        if self.message.content.is_empty()
            || self
                .message
                .content
                .iter()
                .all(ContentBlock::is_empty_text_like)
        {
            return Err(AgentCoreError::InvalidInput(
                "assistant message cannot be empty".to_string(),
            ));
        }

        self.message.finalize()?;
        Ok(self.message)
    }

    fn push_text_like_delta(&mut self, delta: &str, kind: TextLikeKind) {
        if delta.is_empty() {
            return;
        }

        if let Some(last) = self.message.content.last_mut() {
            let same_kind = match kind {
                TextLikeKind::Text => matches!(last, ContentBlock::Text { .. }),
                TextLikeKind::Reasoning => matches!(last, ContentBlock::Reasoning { .. }),
            };
            if same_kind && last.append_text(delta) {
                return;
            }
        }

        let block = match kind {
            TextLikeKind::Text => ContentBlock::text(delta),
            TextLikeKind::Reasoning => ContentBlock::reasoning(delta),
        };
        self.message.push_content(block);
    }
}

impl Default for AssistantBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy)]
enum TextLikeKind {
    Text,
    Reasoning,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_deltas_are_coalesced() {
        let mut builder = AssistantBuilder::new();
        builder.push_text_delta("hel");
        builder.push_text_delta("lo");

        let message = builder.finish().unwrap();

        assert_eq!(message.content, vec![ContentBlock::text("hello")]);
    }

    #[test]
    fn tool_call_can_be_part_of_assistant_message() {
        let mut builder = AssistantBuilder::new();
        builder.push_tool_call("call-1", "read_file", json!({ "path": "README.md" }));

        let message = builder.finish().unwrap();

        assert_eq!(message.content.len(), 1);
    }
}

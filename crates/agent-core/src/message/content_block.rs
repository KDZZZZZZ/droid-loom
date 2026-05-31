use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticLevel {
    Debug,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    ToolCall {
        call_id: String,
        tool_name: String,
        arguments: Value,
    },
    ToolResult {
        call_id: String,
        tool_name: Option<String>,
        output: Value,
        is_error: bool,
    },
    FileReference {
        uri: String,
        mime_type: Option<String>,
        name: Option<String>,
    },
    ImageReference {
        uri: String,
        mime_type: Option<String>,
        alt_text: Option<String>,
    },
    AudioReference {
        uri: String,
        mime_type: Option<String>,
    },
    Diagnostic {
        level: DiagnosticLevel,
        message: String,
    },
    Custom {
        value: Value,
    },
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn reasoning(text: impl Into<String>) -> Self {
        Self::Reasoning { text: text.into() }
    }

    pub fn tool_call(
        call_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments: Value,
    ) -> Self {
        Self::ToolCall {
            call_id: call_id.into(),
            tool_name: tool_name.into(),
            arguments,
        }
    }

    pub fn tool_result(
        call_id: impl Into<String>,
        tool_name: Option<String>,
        output: Value,
        is_error: bool,
    ) -> Self {
        Self::ToolResult {
            call_id: call_id.into(),
            tool_name,
            output,
            is_error,
        }
    }

    pub fn file_reference(
        uri: impl Into<String>,
        mime_type: Option<String>,
        name: Option<String>,
    ) -> Self {
        Self::FileReference {
            uri: uri.into(),
            mime_type,
            name,
        }
    }

    pub fn image_reference(
        uri: impl Into<String>,
        mime_type: Option<String>,
        alt_text: Option<String>,
    ) -> Self {
        Self::ImageReference {
            uri: uri.into(),
            mime_type,
            alt_text,
        }
    }

    pub fn audio_reference(uri: impl Into<String>, mime_type: Option<String>) -> Self {
        Self::AudioReference {
            uri: uri.into(),
            mime_type,
        }
    }

    pub fn diagnostic(level: DiagnosticLevel, message: impl Into<String>) -> Self {
        Self::Diagnostic {
            level,
            message: message.into(),
        }
    }

    pub fn custom(value: Value) -> Self {
        Self::Custom { value }
    }

    pub fn append_text(&mut self, delta: &str) -> bool {
        match self {
            Self::Text { text } | Self::Reasoning { text } => {
                text.push_str(delta);
                true
            }
            _ => false,
        }
    }

    pub fn is_empty_text_like(&self) -> bool {
        match self {
            Self::Text { text } | Self::Reasoning { text } => text.is_empty(),
            Self::Diagnostic { message, .. } => message.is_empty(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_call_keeps_typed_arguments() {
        let block = ContentBlock::tool_call("call-1", "search", json!({ "q": "rust" }));

        assert_eq!(
            block,
            ContentBlock::ToolCall {
                call_id: "call-1".to_string(),
                tool_name: "search".to_string(),
                arguments: json!({ "q": "rust" }),
            }
        );
    }

    #[test]
    fn text_like_block_can_append_delta() {
        let mut block = ContentBlock::text("hel");

        assert!(block.append_text("lo"));
        assert_eq!(block, ContentBlock::text("hello"));
    }
}

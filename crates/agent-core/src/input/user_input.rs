use crate::content_block::ContentBlock;
use crate::error::{AgentCoreError, AgentCoreResult};
use crate::run_message::RunMessage;

pub fn text_message(text: impl Into<String>) -> AgentCoreResult<RunMessage> {
    let text = text.into();
    if text.trim().is_empty() {
        return Err(AgentCoreError::InvalidInput(
            "user text input cannot be empty".to_string(),
        ));
    }

    RunMessage::user(vec![ContentBlock::text(text)])
}

pub fn content_blocks_message(blocks: Vec<ContentBlock>) -> AgentCoreResult<RunMessage> {
    if blocks.is_empty() {
        return Err(AgentCoreError::InvalidInput(
            "user input must contain at least one content block".to_string(),
        ));
    }

    RunMessage::user(blocks)
}

pub fn file_reference_message(
    uri: impl Into<String>,
    mime_type: Option<String>,
    name: Option<String>,
) -> AgentCoreResult<RunMessage> {
    let uri = require_uri(uri.into())?;
    RunMessage::user(vec![ContentBlock::file_reference(uri, mime_type, name)])
}

pub fn image_reference_message(
    uri: impl Into<String>,
    mime_type: Option<String>,
    alt_text: Option<String>,
) -> AgentCoreResult<RunMessage> {
    let uri = require_uri(uri.into())?;
    RunMessage::user(vec![ContentBlock::image_reference(
        uri, mime_type, alt_text,
    )])
}

pub fn audio_reference_message(
    uri: impl Into<String>,
    mime_type: Option<String>,
) -> AgentCoreResult<RunMessage> {
    let uri = require_uri(uri.into())?;
    RunMessage::user(vec![ContentBlock::audio_reference(uri, mime_type)])
}

fn require_uri(uri: String) -> AgentCoreResult<String> {
    if uri.trim().is_empty() {
        return Err(AgentCoreError::InvalidInput(
            "user input reference uri cannot be empty".to_string(),
        ));
    }
    Ok(uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_message::MessageRole;

    #[test]
    fn text_input_becomes_user_message() {
        let message = text_message("hello").unwrap();

        assert_eq!(message.role, MessageRole::User);
        assert_eq!(message.content, vec![ContentBlock::text("hello")]);
    }

    #[test]
    fn empty_text_is_rejected() {
        let result = text_message("  ");

        assert!(matches!(result, Err(AgentCoreError::InvalidInput(_))));
    }
}

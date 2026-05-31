use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AgentCoreError, AgentCoreResult};
use crate::run_message::{MessageStatus, RunMessage};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEntry {
    pub id: Uuid,
    pub parent_id: Option<Uuid>,
    pub timestamp_ms: u64,
    pub kind: SessionEntryKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEntryKind {
    Header {
        session_id: Uuid,
    },
    Message {
        message: RunMessage,
    },
    Compaction {
        summary: String,
        first_kept_entry_id: Option<Uuid>,
    },
}

impl SessionEntry {
    pub fn new(parent_id: Option<Uuid>, kind: SessionEntryKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            parent_id,
            timestamp_ms: now_ms(),
            kind,
        }
    }

    pub fn header(session_id: Uuid) -> Self {
        Self::new(None, SessionEntryKind::Header { session_id })
    }

    pub fn message(parent_id: Option<Uuid>, message: RunMessage) -> AgentCoreResult<Self> {
        if message.status != MessageStatus::Finalized {
            return Err(AgentCoreError::InvalidInput(
                "session can only commit finalized messages".to_string(),
            ));
        }

        Ok(Self::new(parent_id, SessionEntryKind::Message { message }))
    }

    pub fn compaction(
        parent_id: Option<Uuid>,
        summary: impl Into<String>,
        first_kept_entry_id: Option<Uuid>,
    ) -> Self {
        Self::new(
            parent_id,
            SessionEntryKind::Compaction {
                summary: summary.into(),
                first_kept_entry_id,
            },
        )
    }

    pub fn as_message(&self) -> Option<&RunMessage> {
        match &self.kind {
            SessionEntryKind::Message { message } => Some(message),
            _ => None,
        }
    }
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
    use crate::content_block::ContentBlock;
    use crate::run_message::RunMessage;

    #[test]
    fn message_entry_wraps_finalized_run_message() {
        let message = RunMessage::user(vec![ContentBlock::text("hi")]).unwrap();
        let entry = SessionEntry::message(None, message.clone()).unwrap();

        assert_eq!(entry.as_message(), Some(&message));
    }
}

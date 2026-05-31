use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AgentCoreResult;
use crate::run_message::RunMessage;
use crate::session_entry::{SessionEntry, SessionEntryKind};
use crate::session_tree::SessionTree;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplaySnapshot {
    pub leaf_id: Option<Uuid>,
    pub entries: Vec<SessionEntry>,
    pub messages: Vec<RunMessage>,
    pub summaries: Vec<String>,
}

impl ReplaySnapshot {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub fn replay_active_branch(tree: &SessionTree) -> AgentCoreResult<ReplaySnapshot> {
    match tree.active_leaf() {
        Some(leaf_id) => replay_branch(tree, leaf_id),
        None => Ok(ReplaySnapshot {
            leaf_id: None,
            entries: Vec::new(),
            messages: Vec::new(),
            summaries: Vec::new(),
        }),
    }
}

pub fn replay_branch(tree: &SessionTree, leaf_id: Uuid) -> AgentCoreResult<ReplaySnapshot> {
    let entries: Vec<SessionEntry> = tree.branch_to(leaf_id)?.into_iter().cloned().collect();
    let mut messages = Vec::new();
    let mut summaries = Vec::new();
    let mut message_start_index = 0;

    for (index, entry) in entries.iter().enumerate() {
        if let SessionEntryKind::Compaction {
            summary,
            first_kept_entry_id,
        } = &entry.kind
        {
            summaries.push(summary.clone());
            message_start_index = first_kept_entry_id
                .and_then(|first_kept_entry_id| {
                    entries
                        .iter()
                        .position(|entry| entry.id == first_kept_entry_id)
                })
                .unwrap_or(index + 1);
        }
    }

    for entry in entries.iter().skip(message_start_index) {
        if let SessionEntryKind::Message { message } = &entry.kind {
            messages.push(message.clone());
        }
    }

    Ok(ReplaySnapshot {
        leaf_id: Some(leaf_id),
        entries,
        messages,
        summaries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_block::ContentBlock;
    use crate::run_message::RunMessage;

    #[test]
    fn replay_collects_messages_on_active_branch() {
        let mut tree = SessionTree::new();
        let root_id = tree.append(SessionEntry::header(Uuid::new_v4())).unwrap();
        tree.append(
            SessionEntry::message(
                Some(root_id),
                RunMessage::user(vec![ContentBlock::text("hello")]).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();

        let snapshot = replay_active_branch(&tree).unwrap();

        assert_eq!(snapshot.messages.len(), 1);
    }

    #[test]
    fn replay_uses_compaction_as_message_boundary() {
        let mut tree = SessionTree::new();
        let root_id = tree.append(SessionEntry::header(Uuid::new_v4())).unwrap();
        let first_message_id = tree
            .append(
                SessionEntry::message(
                    Some(root_id),
                    RunMessage::user(vec![ContentBlock::text("old")]).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();
        let compaction_id = tree
            .append(SessionEntry::compaction(
                Some(first_message_id),
                "summary",
                None,
            ))
            .unwrap();
        tree.append(
            SessionEntry::message(
                Some(compaction_id),
                RunMessage::user(vec![ContentBlock::text("kept")]).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();

        let snapshot = replay_active_branch(&tree).unwrap();

        assert_eq!(snapshot.summaries, vec!["summary"]);
        assert_eq!(snapshot.messages.len(), 1);
        assert_eq!(
            snapshot.messages[0].content,
            vec![ContentBlock::text("kept")]
        );
    }
}

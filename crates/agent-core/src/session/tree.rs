use std::collections::HashMap;

use uuid::Uuid;

use crate::error::{AgentCoreError, AgentCoreResult};
use crate::session_entry::SessionEntry;

#[derive(Debug, Clone, Default)]
pub struct SessionTree {
    entries: HashMap<Uuid, SessionEntry>,
    children: HashMap<Uuid, Vec<Uuid>>,
    roots: Vec<Uuid>,
    active_leaf: Option<Uuid>,
}

impl SessionTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&mut self, entry: SessionEntry) -> AgentCoreResult<Uuid> {
        if self.entries.contains_key(&entry.id) {
            return Err(AgentCoreError::InvalidInput(format!(
                "duplicate session entry id: {}",
                entry.id
            )));
        }

        if let Some(parent_id) = entry.parent_id {
            if !self.entries.contains_key(&parent_id) {
                return Err(AgentCoreError::NotFound(format!(
                    "session parent entry not found: {parent_id}"
                )));
            }
            self.children.entry(parent_id).or_default().push(entry.id);
        } else {
            self.roots.push(entry.id);
        }

        let id = entry.id;
        self.entries.insert(id, entry);
        self.active_leaf = Some(id);
        Ok(id)
    }

    pub fn get(&self, id: Uuid) -> Option<&SessionEntry> {
        self.entries.get(&id)
    }

    pub fn roots(&self) -> &[Uuid] {
        &self.roots
    }

    pub fn children_of(&self, id: Uuid) -> &[Uuid] {
        self.children.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn active_leaf(&self) -> Option<Uuid> {
        self.active_leaf
    }

    pub fn set_active_leaf(&mut self, id: Uuid) -> AgentCoreResult<()> {
        if !self.entries.contains_key(&id) {
            return Err(AgentCoreError::NotFound(format!(
                "session entry not found: {id}"
            )));
        }
        self.active_leaf = Some(id);
        Ok(())
    }

    pub fn branch_to(&self, leaf_id: Uuid) -> AgentCoreResult<Vec<&SessionEntry>> {
        let mut ids = Vec::new();
        let mut current = Some(leaf_id);

        while let Some(id) = current {
            let entry = self.entries.get(&id).ok_or_else(|| {
                AgentCoreError::NotFound(format!("session entry not found: {id}"))
            })?;
            ids.push(id);
            current = entry.parent_id;
        }

        ids.reverse();
        Ok(ids
            .into_iter()
            .filter_map(|id| self.entries.get(&id))
            .collect())
    }

    pub fn active_branch(&self) -> AgentCoreResult<Vec<&SessionEntry>> {
        let leaf_id = self
            .active_leaf
            .ok_or_else(|| AgentCoreError::NotFound("session tree is empty".to_string()))?;
        self.branch_to(leaf_id)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_block::ContentBlock;
    use crate::run_message::RunMessage;
    use crate::session_entry::SessionEntry;

    #[test]
    fn appends_entries_as_tree_and_replays_branch_order() {
        let mut tree = SessionTree::new();
        let root = tree.append(SessionEntry::header(Uuid::new_v4())).unwrap();
        let child = tree
            .append(
                SessionEntry::message(
                    Some(root),
                    RunMessage::user(vec![ContentBlock::text("hello")]).unwrap(),
                )
                .unwrap(),
            )
            .unwrap();

        let branch = tree.branch_to(child).unwrap();

        assert_eq!(branch.len(), 2);
        assert_eq!(branch[0].id, root);
        assert_eq!(branch[1].id, child);
    }

    #[test]
    fn rejects_missing_parent() {
        let mut tree = SessionTree::new();
        let entry = SessionEntry::message(
            Some(Uuid::new_v4()),
            RunMessage::user(vec![ContentBlock::text("hello")]).unwrap(),
        )
        .unwrap();

        let result = tree.append(entry);

        assert!(matches!(result, Err(AgentCoreError::NotFound(_))));
    }
}

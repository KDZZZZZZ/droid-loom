use crate::error::AgentCoreResult;
use crate::session_entry::SessionEntry;
use crate::session_tree::SessionTree;

pub trait SessionStore {
    fn append(&mut self, entry: SessionEntry) -> AgentCoreResult<()>;
    fn entries(&self) -> &[SessionEntry];

    fn load_tree(&self) -> AgentCoreResult<SessionTree> {
        let mut tree = SessionTree::new();
        for entry in self.entries().iter().cloned() {
            tree.append(entry)?;
        }
        Ok(tree)
    }
}

#[derive(Debug, Clone, Default)]
pub struct InMemorySessionStore {
    entries: Vec<SessionEntry>,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl SessionStore for InMemorySessionStore {
    fn append(&mut self, entry: SessionEntry) -> AgentCoreResult<()> {
        self.entries.push(entry);
        Ok(())
    }

    fn entries(&self) -> &[SessionEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    use crate::session_entry::SessionEntry;

    #[test]
    fn in_memory_store_loads_tree_from_append_order() {
        let mut store = InMemorySessionStore::new();
        let root = SessionEntry::header(Uuid::new_v4());
        let root_id = root.id;

        store.append(root).unwrap();

        let tree = store.load_tree().unwrap();

        assert_eq!(tree.active_leaf(), Some(root_id));
    }
}

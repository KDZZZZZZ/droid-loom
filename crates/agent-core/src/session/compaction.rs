use uuid::Uuid;

use crate::error::{AgentCoreError, AgentCoreResult};
use crate::session_entry::SessionEntry;
use crate::session_tree::SessionTree;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionPlan {
    pub parent_id: Option<Uuid>,
    pub summary: String,
    pub first_kept_entry_id: Option<Uuid>,
}

impl CompactionPlan {
    pub fn new(
        parent_id: Option<Uuid>,
        summary: impl Into<String>,
        first_kept_entry_id: Option<Uuid>,
    ) -> AgentCoreResult<Self> {
        let summary = summary.into();
        if summary.trim().is_empty() {
            return Err(AgentCoreError::InvalidInput(
                "compaction summary cannot be empty".to_string(),
            ));
        }

        Ok(Self {
            parent_id,
            summary,
            first_kept_entry_id,
        })
    }

    pub fn into_entry(self) -> SessionEntry {
        SessionEntry::compaction(self.parent_id, self.summary, self.first_kept_entry_id)
    }
}

pub fn plan_active_branch_compaction(
    tree: &SessionTree,
    summary: impl Into<String>,
    first_kept_entry_id: Option<Uuid>,
) -> AgentCoreResult<CompactionPlan> {
    CompactionPlan::new(tree.active_leaf(), summary, first_kept_entry_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compaction_plan_requires_summary() {
        let result = CompactionPlan::new(None, " ", None);

        assert!(matches!(result, Err(AgentCoreError::InvalidInput(_))));
    }

    #[test]
    fn compaction_plan_builds_session_entry() {
        let entry = CompactionPlan::new(None, "summary", None)
            .unwrap()
            .into_entry();

        assert!(matches!(
            entry.kind,
            crate::session_entry::SessionEntryKind::Compaction { .. }
        ));
    }
}

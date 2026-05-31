use crate::error::{AgentCoreError, AgentCoreResult};
use crate::graph_edge::EdgeId;
use crate::graph_node::NodeId;
use crate::run_message::RunMessage;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphState {
    run_id: Uuid,
    budget: GraphStateBudget,
    node_messages: BTreeMap<NodeId, Vec<NodeMessageRecord>>,
    message_log: Vec<NodeMessageRecord>,
    node_message_versions: BTreeMap<NodeId, u64>,
    fired_edges: BTreeSet<FiredEdgeRecord>,
    total_node_executions: usize,
    node_execution_counts: BTreeMap<NodeId, usize>,
    stop_requested: bool,
}

impl GraphState {
    pub fn new(run_id: Uuid, budget: GraphStateBudget) -> Self {
        Self {
            run_id,
            budget,
            node_messages: BTreeMap::new(),
            message_log: Vec::new(),
            node_message_versions: BTreeMap::new(),
            fired_edges: BTreeSet::new(),
            total_node_executions: 0,
            node_execution_counts: BTreeMap::new(),
            stop_requested: false,
        }
    }

    pub fn run_id(&self) -> Uuid {
        self.run_id
    }

    pub fn budget(&self) -> &GraphStateBudget {
        &self.budget
    }

    pub fn view(&self) -> GraphStateView<'_> {
        GraphStateView { state: self }
    }

    pub fn append_message(
        &mut self,
        node_id: impl Into<NodeId>,
        mut message: RunMessage,
    ) -> NodeMessageRecord {
        let node_id = node_id.into();
        message.set_source_node_id_if_empty(node_id.clone());
        let version = self
            .node_message_versions
            .entry(node_id.clone())
            .or_insert(0);
        *version += 1;

        let record = NodeMessageRecord {
            node_id: node_id.clone(),
            version: *version,
            message,
        };
        self.node_messages
            .entry(node_id)
            .or_default()
            .push(record.clone());
        self.message_log.push(record.clone());
        record
    }

    pub fn record_node_execution(&mut self, node_id: impl Into<NodeId>) -> AgentCoreResult<()> {
        let node_id = node_id.into();
        let next_total = self.total_node_executions + 1;
        if let Some(max) = self.budget.max_total_node_executions {
            if next_total > max {
                return Err(AgentCoreError::Fatal(format!(
                    "graph total node execution budget exceeded: {next_total} > {max}"
                )));
            }
        }

        let next_node_count = self.node_execution_count(&node_id) + 1;
        if let Some(max) = self.budget.max_node_executions {
            if next_node_count > max {
                return Err(AgentCoreError::Fatal(format!(
                    "graph node execution budget exceeded for {node_id}: {next_node_count} > {max}"
                )));
            }
        }

        self.total_node_executions = next_total;
        self.node_execution_counts.insert(node_id, next_node_count);
        Ok(())
    }

    pub fn total_node_executions(&self) -> usize {
        self.total_node_executions
    }

    pub fn node_execution_count(&self, node_id: &str) -> usize {
        self.node_execution_counts
            .get(node_id)
            .copied()
            .unwrap_or_default()
    }

    pub fn message_count(&self, node_id: &str) -> usize {
        self.node_messages
            .get(node_id)
            .map(Vec::len)
            .unwrap_or_default()
    }

    pub fn message_version(&self, node_id: &str) -> u64 {
        self.node_message_versions
            .get(node_id)
            .copied()
            .unwrap_or_default()
    }

    pub fn messages_for_node(&self, node_id: &str) -> &[NodeMessageRecord] {
        self.node_messages
            .get(node_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn all_messages(&self) -> Vec<RunMessage> {
        self.message_log
            .iter()
            .map(|record| record.message.clone())
            .collect()
    }

    pub fn has_edge_fired(&self, record: &FiredEdgeRecord) -> bool {
        self.fired_edges.contains(record)
    }

    pub fn mark_edge_fired(&mut self, record: FiredEdgeRecord) -> bool {
        self.fired_edges.insert(record)
    }

    pub fn fired_edges(&self) -> &BTreeSet<FiredEdgeRecord> {
        &self.fired_edges
    }

    pub fn request_stop(&mut self) {
        self.stop_requested = true;
    }

    pub fn is_stop_requested(&self) -> bool {
        self.stop_requested
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeMessageRecord {
    pub node_id: NodeId,
    pub version: u64,
    pub message: RunMessage,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct FiredEdgeRecord {
    pub edge_id: EdgeId,
    pub source_node_id: NodeId,
    pub source_message_version: u64,
    pub target_node_id: NodeId,
}

impl FiredEdgeRecord {
    pub fn new(
        edge_id: impl Into<EdgeId>,
        source_node_id: impl Into<NodeId>,
        source_message_version: u64,
        target_node_id: impl Into<NodeId>,
    ) -> Self {
        Self {
            edge_id: edge_id.into(),
            source_node_id: source_node_id.into(),
            source_message_version,
            target_node_id: target_node_id.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphStateBudget {
    pub max_total_node_executions: Option<usize>,
    pub max_node_executions: Option<usize>,
    pub max_no_progress_ticks: Option<usize>,
}

impl GraphStateBudget {
    pub fn unlimited() -> Self {
        Self {
            max_total_node_executions: None,
            max_node_executions: None,
            max_no_progress_ticks: None,
        }
    }
}

impl Default for GraphStateBudget {
    fn default() -> Self {
        Self {
            max_total_node_executions: Some(100),
            max_node_executions: Some(20),
            max_no_progress_ticks: Some(10),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GraphStateView<'a> {
    state: &'a GraphState,
}

impl<'a> GraphStateView<'a> {
    pub fn run_id(&self) -> Uuid {
        self.state.run_id()
    }

    pub fn message_count(&self, node_id: &str) -> usize {
        self.state.message_count(node_id)
    }

    pub fn message_version(&self, node_id: &str) -> u64 {
        self.state.message_version(node_id)
    }

    pub fn total_node_executions(&self) -> usize {
        self.state.total_node_executions()
    }

    pub fn node_execution_count(&self, node_id: &str) -> usize {
        self.state.node_execution_count(node_id)
    }

    pub fn has_edge_fired(&self, record: &FiredEdgeRecord) -> bool {
        self.state.has_edge_fired(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_execution_budget_blocks_over_limit() {
        let mut state = GraphState::new(
            Uuid::new_v4(),
            GraphStateBudget {
                max_total_node_executions: Some(1),
                max_node_executions: Some(1),
                max_no_progress_ticks: None,
            },
        );

        state.record_node_execution("a").unwrap();

        let err = state.record_node_execution("a").unwrap_err();
        assert!(matches!(err, AgentCoreError::Fatal(_)));
    }
}

use crate::error::{AgentCoreError, AgentCoreResult};
use crate::graph_edge::GraphEdge;
use crate::graph_node::{GraphNode, NodeId};
use crate::graph_state::GraphStateBudget;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Graph {
    name: String,
    nodes: BTreeMap<NodeId, GraphNode>,
    edges: Vec<GraphEdge>,
    start_node_ids: Vec<NodeId>,
    end_node_ids: BTreeSet<NodeId>,
    budget: GraphStateBudget,
}

impl Graph {
    pub fn builder(name: impl Into<String>) -> GraphBuilder {
        GraphBuilder::new(name)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn node(&self, node_id: &str) -> Option<&GraphNode> {
        self.nodes.get(node_id)
    }

    pub fn nodes(&self) -> &BTreeMap<NodeId, GraphNode> {
        &self.nodes
    }

    pub fn edges(&self) -> &[GraphEdge] {
        &self.edges
    }

    pub fn start_node_ids(&self) -> &[NodeId] {
        &self.start_node_ids
    }

    pub fn is_end_node(&self, node_id: &str) -> bool {
        self.end_node_ids.contains(node_id)
    }

    pub fn budget(&self) -> &GraphStateBudget {
        &self.budget
    }

    pub fn outgoing_edges<'a>(
        &'a self,
        node_id: &'a str,
    ) -> impl Iterator<Item = &'a GraphEdge> + 'a {
        self.edges
            .iter()
            .filter(move |edge| edge.source_node_id() == node_id)
    }
}

#[derive(Clone, Debug)]
pub struct GraphBuilder {
    name: String,
    nodes: BTreeMap<NodeId, GraphNode>,
    edges: Vec<GraphEdge>,
    start_node_ids: Vec<NodeId>,
    end_node_ids: BTreeSet<NodeId>,
    budget: GraphStateBudget,
}

impl GraphBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            nodes: BTreeMap::new(),
            edges: Vec::new(),
            start_node_ids: Vec::new(),
            end_node_ids: BTreeSet::new(),
            budget: GraphStateBudget::default(),
        }
    }

    pub fn node(mut self, node: GraphNode) -> Self {
        self.nodes.insert(node.id().to_string(), node);
        self
    }

    pub fn edge(mut self, edge: GraphEdge) -> Self {
        self.edges.push(edge);
        self
    }

    pub fn start_node(mut self, node_id: impl Into<NodeId>) -> Self {
        self.start_node_ids.push(node_id.into());
        self
    }

    pub fn end_node(mut self, node_id: impl Into<NodeId>) -> Self {
        self.end_node_ids.insert(node_id.into());
        self
    }

    pub fn budget(mut self, budget: GraphStateBudget) -> Self {
        self.budget = budget;
        self
    }

    pub fn build(mut self) -> AgentCoreResult<Graph> {
        if self.name.trim().is_empty() {
            return Err(AgentCoreError::InvalidConfig(
                "graph name must not be empty".to_string(),
            ));
        }

        if self.start_node_ids.is_empty() {
            return Err(AgentCoreError::InvalidConfig(
                "graph must define at least one start node".to_string(),
            ));
        }

        for node_id in self.nodes.keys() {
            if node_id.trim().is_empty() {
                return Err(AgentCoreError::InvalidConfig(
                    "graph node id must not be empty".to_string(),
                ));
            }
        }

        for start_node_id in &self.start_node_ids {
            if !self.nodes.contains_key(start_node_id) {
                return Err(AgentCoreError::InvalidConfig(format!(
                    "start node not found: {start_node_id}"
                )));
            }
        }

        for end_node_id in &self.end_node_ids {
            if !self.nodes.contains_key(end_node_id) {
                return Err(AgentCoreError::InvalidConfig(format!(
                    "end node not found: {end_node_id}"
                )));
            }
        }

        for edge in &self.edges {
            if !self.nodes.contains_key(edge.source_node_id()) {
                return Err(AgentCoreError::InvalidConfig(format!(
                    "edge {} source node not found: {}",
                    edge.id(),
                    edge.source_node_id()
                )));
            }

            if !self.nodes.contains_key(edge.target_node_id()) {
                return Err(AgentCoreError::InvalidConfig(format!(
                    "edge {} target node not found: {}",
                    edge.id(),
                    edge.target_node_id()
                )));
            }
        }

        self.edges.sort_by(|left, right| {
            left.source_node_id()
                .cmp(right.source_node_id())
                .then_with(|| left.priority().cmp(&right.priority()))
                .then_with(|| left.id().cmp(right.id()))
        });

        Ok(Graph {
            name: self.name,
            nodes: self.nodes,
            edges: self.edges,
            start_node_ids: self.start_node_ids,
            end_node_ids: self.end_node_ids,
            budget: self.budget,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_rejects_missing_start_node() {
        let err = Graph::builder("test")
            .start_node("start")
            .build()
            .unwrap_err();
        assert!(matches!(err, AgentCoreError::InvalidConfig(_)));
    }

    #[test]
    fn builder_accepts_minimal_graph() {
        let graph = Graph::builder("test")
            .node(GraphNode::new("start").terminal(true))
            .start_node("start")
            .end_node("start")
            .build()
            .unwrap();

        assert_eq!(graph.name(), "test");
        assert!(graph.is_end_node("start"));
    }
}

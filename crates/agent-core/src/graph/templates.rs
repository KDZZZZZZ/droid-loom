use crate::error::AgentCoreResult;
use crate::graph::Graph;
use crate::graph_edge::{ActivationCondition, GraphEdge};
use crate::graph_node::{GraphNode, GraphNodeAction};

pub const DEFAULT_REACT_GRAPH: &str = "default_react";

pub fn default_react_graph() -> AgentCoreResult<Graph> {
    Graph::builder(DEFAULT_REACT_GRAPH)
        .node(GraphNode::new("start").with_action(GraphNodeAction::PassthroughInput))
        .node(GraphNode::new("build_context"))
        .node(GraphNode::new("provider_request"))
        .node(GraphNode::new("collect_assistant"))
        .node(GraphNode::new("execute_tools"))
        .node(GraphNode::new("finalize_turn").terminal(true))
        .node(GraphNode::new("handle_error").terminal(true))
        .start_node("start")
        .end_node("finalize_turn")
        .end_node("handle_error")
        .edge(GraphEdge::new("start_to_context", "start", "build_context"))
        .edge(GraphEdge::new(
            "context_to_provider",
            "build_context",
            "provider_request",
        ))
        .edge(GraphEdge::new(
            "provider_to_collect",
            "provider_request",
            "collect_assistant",
        ))
        .edge(
            GraphEdge::new("collect_to_tools", "collect_assistant", "execute_tools")
                .with_activation_condition(ActivationCondition::MessageHasToolCall),
        )
        .edge(
            GraphEdge::new("collect_to_final", "collect_assistant", "finalize_turn")
                .with_activation_condition(ActivationCondition::MessageHasText)
                .with_priority(10),
        )
        .edge(GraphEdge::new(
            "tools_to_context",
            "execute_tools",
            "build_context",
        ))
        .edge(
            GraphEdge::new("provider_to_error", "provider_request", "handle_error")
                .with_activation_condition(ActivationCondition::Never),
        )
        .build()
}

pub fn single_node_graph(
    name: impl Into<String>,
    node_id: impl Into<String>,
) -> AgentCoreResult<Graph> {
    let node_id = node_id.into();
    Graph::builder(name)
        .node(GraphNode::new(node_id.clone()).terminal(true))
        .start_node(node_id.clone())
        .end_node(node_id)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_default_react_template() {
        let graph = default_react_graph().unwrap();

        assert_eq!(graph.name(), DEFAULT_REACT_GRAPH);
        assert!(graph.node("start").is_some());
        assert!(graph.node("finalize_turn").is_some());
    }
}

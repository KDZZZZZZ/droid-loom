use crate::error::AgentCoreResult;
use crate::graph::Graph;
use crate::graph_node::{
    Cardinality, GraphNode, InputPackageSpec, MessageQuery, NodeConcurrency, NodeKind,
};
use serde_json::json;

pub const DEFAULT_REACT_GRAPH: &str = "default_react";

pub fn default_react_graph() -> AgentCoreResult<Graph> {
    Graph::builder(DEFAULT_REACT_GRAPH)
        .node(
            GraphNode::agent(
                "agent",
                "default_agent",
                InputPackageSpec::new("context")
                    .required(
                        "turn",
                        MessageQuery::where_eq("role", "user"),
                        Cardinality::Latest,
                    )
                    .optional(
                        "tool_result",
                        MessageQuery::where_eq("content[*].type", "tool_result"),
                        Cardinality::Latest,
                    ),
            )
            .concurrency(NodeConcurrency::Serial),
        )
        .node(
            GraphNode::tool(
                "tool",
                "dispatch_tool_call",
                "tool_call",
                InputPackageSpec::new("calls").required(
                    "tool_call",
                    MessageQuery::where_eq("content[*].type", "tool_call"),
                    Cardinality::Latest,
                ),
            )
            .concurrency(NodeConcurrency::Parallel { max: 4 }),
        )
        .node(GraphNode::final_node(
            "final",
            InputPackageSpec::new("answer").required(
                "assistant_answer",
                MessageQuery::where_exists("content[*].text"),
                Cardinality::Latest,
            ),
        ))
        .edge("input_to_agent", "input", ("agent", "context"))
        .edge("agent_to_tool", "agent", ("tool", "calls"))
        .edge("tool_to_agent", "tool", ("agent", "context"))
        .edge("agent_to_final", "agent", ("final", "answer"))
        .finish_at("final")
        .build()
}

pub fn single_node_graph(
    name: impl Into<String>,
    node_id: impl Into<String>,
) -> AgentCoreResult<Graph> {
    let node_id = node_id.into();
    Graph::builder(name)
        .node(GraphNode::new(
            node_id.clone(),
            NodeKind::Transform {
                executor: "single_node".to_string(),
                config: json!({}),
            },
            InputPackageSpec::new("input").required(
                "turn",
                MessageQuery::any(),
                Cardinality::Latest,
            ),
        ))
        .edge("input_to_node", "input", (node_id.clone(), "input"))
        .finish_at(node_id)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_default_react_template() {
        let graph = default_react_graph().unwrap();

        assert_eq!(graph.name(), DEFAULT_REACT_GRAPH);
        assert!(graph.node("agent").is_some());
        assert_eq!(graph.finish_node(), Some("final"));
        assert_eq!(graph.edges().len(), 4);
    }
}

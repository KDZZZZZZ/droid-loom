use crate::error::AgentCoreResult;
use crate::event::CoreEvent;
use crate::graph::Graph;
use crate::graph_edge::EdgeDecision;
use crate::graph_node::{NodeExecutionInput, NodeId};
use crate::graph_state::{FiredEdgeRecord, GraphState, NodeMessageRecord};
use crate::run_message::RunMessage;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use uuid::Uuid;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GraphRunInput {
    pub run_id: Option<Uuid>,
    pub initial_messages: Vec<RunMessage>,
    pub stop_requested: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphRunResult {
    pub run_id: Uuid,
    pub status: GraphRunStatus,
    pub messages: Vec<RunMessage>,
    pub events: Vec<CoreEvent>,
    pub state: GraphState,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphRunStatus {
    Completed,
    Cancelled,
    BudgetExceeded,
    Failed,
}

#[derive(Clone, Debug, Default)]
pub struct GraphRunner;

impl GraphRunner {
    pub fn new() -> Self {
        Self
    }

    pub fn run(&self, graph: &Graph, input: GraphRunInput) -> AgentCoreResult<GraphRunResult> {
        let run_id = input.run_id.unwrap_or_else(Uuid::new_v4);
        let mut state = GraphState::new(run_id, graph.budget().clone());
        let mut events = vec![CoreEvent::GraphStarted {
            run_id,
            graph_name: graph.name().to_string(),
        }];
        let mut runnable = graph
            .start_node_ids()
            .iter()
            .cloned()
            .map(|node_id| RunnableNode {
                node_id,
                input_messages: input.initial_messages.clone(),
            })
            .collect::<VecDeque<_>>();

        if input.stop_requested {
            state.request_stop();
        }

        while let Some(runnable_node) = runnable.pop_front() {
            if state.is_stop_requested() {
                return Ok(Self::finish(
                    graph,
                    GraphRunStatus::Cancelled,
                    state,
                    events,
                    None,
                ));
            }

            let node_id = runnable_node.node_id;
            let Some(node) = graph.node(&node_id) else {
                return Ok(Self::finish(
                    graph,
                    GraphRunStatus::Failed,
                    state,
                    events,
                    Some(format!("graph node not found: {node_id}")),
                ));
            };

            if let Err(error) = state.record_node_execution(node.id()) {
                return Ok(Self::finish(
                    graph,
                    GraphRunStatus::BudgetExceeded,
                    state,
                    events,
                    Some(error.to_string()),
                ));
            }

            events.push(CoreEvent::NodeStarted {
                run_id,
                node_id: node_id.clone(),
            });

            let node_result = node.execute(
                NodeExecutionInput {
                    input_messages: runnable_node.input_messages,
                },
                |message| {
                    let record = state.append_message(node_id.clone(), message);
                    events.push(CoreEvent::MessageEmitted {
                        run_id,
                        node_id: record.node_id.clone(),
                        message_id: record.message.id,
                    });
                    Self::process_node_message(graph, &mut state, record, &mut runnable)
                },
            )?;

            events.push(CoreEvent::NodeEnded {
                run_id,
                node_id: node_id.clone(),
                emitted_messages: node_result.emitted_messages,
            });

            if node.is_terminal() || graph.is_end_node(node.id()) {
                return Ok(Self::finish(
                    graph,
                    GraphRunStatus::Completed,
                    state,
                    events,
                    None,
                ));
            }
        }

        Ok(Self::finish(
            graph,
            GraphRunStatus::Completed,
            state,
            events,
            None,
        ))
    }

    fn process_node_message(
        graph: &Graph,
        state: &mut GraphState,
        record: NodeMessageRecord,
        runnable: &mut VecDeque<RunnableNode>,
    ) -> AgentCoreResult<()> {
        for edge in graph.outgoing_edges(&record.node_id) {
            let fired_record = FiredEdgeRecord::new(
                edge.id(),
                record.node_id.clone(),
                record.version,
                edge.target_node_id(),
            );

            if state.has_edge_fired(&fired_record) {
                continue;
            }

            let decision = edge.evaluate(&record.message, record.version, &state.view())?;
            if let EdgeDecision::Activate { target_node_id, .. } = decision {
                if state.mark_edge_fired(fired_record) {
                    let input_messages = state
                        .messages_for_node(&record.node_id)
                        .iter()
                        .map(|record| record.message.clone())
                        .collect();
                    runnable.push_back(RunnableNode {
                        node_id: target_node_id,
                        input_messages,
                    });
                }
            }
        }

        Ok(())
    }

    fn finish(
        graph: &Graph,
        status: GraphRunStatus,
        state: GraphState,
        mut events: Vec<CoreEvent>,
        error: Option<String>,
    ) -> GraphRunResult {
        events.push(CoreEvent::GraphEnded {
            run_id: state.run_id(),
            graph_name: graph.name().to_string(),
            status: status.as_str().to_string(),
        });

        GraphRunResult {
            run_id: state.run_id(),
            status,
            messages: state.all_messages(),
            events,
            state,
            error,
        }
    }
}

impl GraphRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::BudgetExceeded => "budget_exceeded",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug)]
struct RunnableNode {
    node_id: NodeId,
    input_messages: Vec<RunMessage>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_block::ContentBlock;
    use crate::graph::Graph;
    use crate::graph_edge::{ActivationCondition, GraphEdge};
    use crate::graph_node::{GraphNode, GraphNodeAction};
    use crate::graph_state::GraphStateBudget;

    fn message(text: &str) -> RunMessage {
        RunMessage::assistant(vec![ContentBlock::text(text)]).unwrap()
    }

    #[test]
    fn runner_executes_terminal_start_node() {
        let graph = Graph::builder("test")
            .node(GraphNode::new("start").terminal(true))
            .start_node("start")
            .end_node("start")
            .build()
            .unwrap();

        let result = GraphRunner::new()
            .run(&graph, GraphRunInput::default())
            .unwrap();

        assert_eq!(result.status, GraphRunStatus::Completed);
        assert_eq!(result.state.total_node_executions(), 1);
    }

    #[test]
    fn runner_activates_target_when_source_emits_matching_message() {
        let graph = Graph::builder("test")
            .node(
                GraphNode::new("source")
                    .with_action(GraphNodeAction::EmitMessages(vec![message("go")])),
            )
            .node(GraphNode::new("target").terminal(true))
            .start_node("source")
            .end_node("target")
            .edge(
                GraphEdge::new("source_to_target", "source", "target")
                    .with_activation_condition(ActivationCondition::MessageHasText),
            )
            .build()
            .unwrap();

        let result = GraphRunner::new()
            .run(&graph, GraphRunInput::default())
            .unwrap();

        assert_eq!(result.status, GraphRunStatus::Completed);
        assert_eq!(result.state.node_execution_count("source"), 1);
        assert_eq!(result.state.node_execution_count("target"), 1);
        assert_eq!(result.state.fired_edges().len(), 1);
    }

    #[test]
    fn runner_passes_full_source_context_to_activated_target() {
        let graph = Graph::builder("test")
            .node(
                GraphNode::new("source")
                    .with_action(GraphNodeAction::EmitMessages(vec![message("go")])),
            )
            .node(GraphNode::new("target").with_action(GraphNodeAction::PassthroughInput))
            .start_node("source")
            .edge(
                GraphEdge::new("source_to_target", "source", "target")
                    .with_activation_condition(ActivationCondition::MessageHasText),
            )
            .build()
            .unwrap();

        let result = GraphRunner::new()
            .run(&graph, GraphRunInput::default())
            .unwrap();

        assert_eq!(result.state.node_execution_count("target"), 1);
        assert_eq!(result.state.message_count("target"), 1);
        assert_eq!(
            result.state.messages_for_node("target")[0].message.content,
            vec![ContentBlock::text("go")]
        );
    }

    #[test]
    fn runner_reports_budget_exceeded() {
        let graph = Graph::builder("test")
            .node(GraphNode::new("start"))
            .start_node("start")
            .budget(GraphStateBudget {
                max_total_node_executions: Some(0),
                max_node_executions: Some(1),
                max_no_progress_ticks: None,
            })
            .build()
            .unwrap();

        let result = GraphRunner::new()
            .run(&graph, GraphRunInput::default())
            .unwrap();

        assert_eq!(result.status, GraphRunStatus::BudgetExceeded);
        assert!(result.error.is_some());
    }
}

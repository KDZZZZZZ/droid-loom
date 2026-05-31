use std::fmt;
use std::sync::Arc;

use futures::executor::block_on;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AgentCoreError, AgentCoreResult};
use crate::event::CoreEvent;
use crate::graph::Graph;
use crate::graph_runtime::{
    self as rt, GraphRunLedger, GraphRuntime, GraphRuntimeServices, GraphRuntimeState,
    NodeExecutionContext, NodeExecutor, NodeInput, NodeKind, NodeOutput,
};
use crate::run_message::RunMessage;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphRunInput {
    pub run_id: Option<Uuid>,
    pub initial_messages: Vec<RunMessage>,
    pub stop_requested: bool,
    pub max_ticks: usize,
}

impl GraphRunInput {
    pub fn new(initial_messages: Vec<RunMessage>) -> Self {
        Self {
            run_id: None,
            initial_messages,
            stop_requested: false,
            max_ticks: 10_000,
        }
    }

    pub fn with_run_id(mut self, run_id: Uuid) -> Self {
        self.run_id = Some(run_id);
        self
    }

    pub fn with_stop_requested(mut self, stop_requested: bool) -> Self {
        self.stop_requested = stop_requested;
        self
    }

    pub fn with_max_ticks(mut self, max_ticks: usize) -> Self {
        self.max_ticks = max_ticks;
        self
    }
}

impl Default for GraphRunInput {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphRunResult {
    pub run_id: Uuid,
    pub status: GraphRunStatus,
    pub messages: Vec<RunMessage>,
    pub events: Vec<CoreEvent>,
    pub state: GraphRuntimeState,
    pub ledger: GraphRunLedger,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphRunStatus {
    Completed,
    Drained,
    Cancelled,
    BudgetExceeded,
    Failed,
}

#[derive(Clone)]
pub struct GraphRunner {
    services: GraphRuntimeServices,
}

impl GraphRunner {
    pub fn new() -> Self {
        Self::with_executor(Arc::new(DefaultNodeExecutor))
    }

    pub fn with_executor(executor: Arc<dyn NodeExecutor>) -> Self {
        Self {
            services: GraphRuntimeServices::new(executor),
        }
    }

    pub fn with_services(services: GraphRuntimeServices) -> Self {
        Self { services }
    }

    pub fn run(&self, graph: &Graph, input: GraphRunInput) -> AgentCoreResult<GraphRunResult> {
        let run_id = input.run_id.unwrap_or_else(Uuid::new_v4);
        let mut events = vec![CoreEvent::GraphStarted {
            run_id,
            graph_name: graph.name().to_string(),
        }];

        if input.stop_requested {
            events.push(CoreEvent::GraphEnded {
                run_id,
                graph_name: graph.name().to_string(),
                status: GraphRunStatus::Cancelled.as_str().to_string(),
            });
            return Ok(GraphRunResult {
                run_id,
                status: GraphRunStatus::Cancelled,
                messages: input.initial_messages,
                events,
                state: GraphRuntimeState::default(),
                ledger: GraphRunLedger::default(),
                error: None,
            });
        }

        let runtime_input = rt::GraphRunInput::new(input.initial_messages)
            .with_run_id(run_id)
            .with_max_ticks(input.max_ticks);
        let output =
            block_on(GraphRuntime::new(graph.clone(), self.services.clone()).run(runtime_input))?;
        let status = GraphRunStatus::from(output.status);

        events.extend(core_events_from_runtime(
            run_id,
            &output.state,
            &output.ledger,
        ));
        if let Some(error) = &output.error {
            events.push(CoreEvent::Error {
                run_id: Some(run_id),
                message: error.clone(),
                recoverable: status != GraphRunStatus::Failed,
            });
        }
        events.push(CoreEvent::GraphEnded {
            run_id,
            graph_name: graph.name().to_string(),
            status: status.as_str().to_string(),
        });

        Ok(GraphRunResult {
            run_id,
            status,
            messages: output.messages,
            events,
            state: output.state,
            ledger: output.ledger,
            error: output.error,
        })
    }
}

impl Default for GraphRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for GraphRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GraphRunner").finish_non_exhaustive()
    }
}

impl GraphRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Drained => "drained",
            Self::Cancelled => "cancelled",
            Self::BudgetExceeded => "budget_exceeded",
            Self::Failed => "failed",
        }
    }
}

impl From<rt::GraphRunStatus> for GraphRunStatus {
    fn from(value: rt::GraphRunStatus) -> Self {
        match value {
            rt::GraphRunStatus::Completed => Self::Completed,
            rt::GraphRunStatus::Drained => Self::Drained,
            rt::GraphRunStatus::BudgetExceeded => Self::BudgetExceeded,
            rt::GraphRunStatus::Failed => Self::Failed,
        }
    }
}

fn core_events_from_runtime(
    run_id: Uuid,
    state: &GraphRuntimeState,
    ledger: &GraphRunLedger,
) -> Vec<CoreEvent> {
    let mut events = Vec::new();
    for attempt in &ledger.node_attempts {
        events.push(CoreEvent::NodeStarted {
            run_id,
            node_id: attempt.node.clone(),
        });
        for entry in state
            .output_logs
            .values()
            .flat_map(|entries| entries.iter())
            .filter(|entry| entry.node == attempt.node)
        {
            events.push(CoreEvent::MessageEmitted {
                run_id,
                node_id: attempt.node.clone(),
                message_id: entry.message.id,
            });
        }
        events.push(CoreEvent::NodeEnded {
            run_id,
            node_id: attempt.node.clone(),
            emitted_messages: state
                .output_logs
                .values()
                .flat_map(|entries| entries.iter())
                .filter(|entry| entry.node == attempt.node)
                .count(),
        });
    }
    events
}

#[derive(Debug)]
struct DefaultNodeExecutor;

impl NodeExecutor for DefaultNodeExecutor {
    fn execute(
        &self,
        node: rt::NodeSpec,
        _input: NodeInput,
        _ctx: NodeExecutionContext,
    ) -> BoxFuture<'static, AgentCoreResult<NodeOutput>> {
        Box::pin(async move {
            match node.kind {
                NodeKind::Final | NodeKind::Transform { .. } => Ok(NodeOutput::new()),
                NodeKind::Agent(spec) => Err(AgentCoreError::InvalidConfig(format!(
                    "agent node `{}` requires a GraphRunner executor",
                    spec.agent_name
                ))),
                NodeKind::Tool(spec) => Err(AgentCoreError::InvalidConfig(format!(
                    "tool node `{}` requires a GraphRunner executor",
                    spec.tool_name
                ))),
                NodeKind::Graph { graph_name } => Err(AgentCoreError::InvalidConfig(format!(
                    "subgraph node `{graph_name}` requires a GraphRunner executor"
                ))),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content_block::ContentBlock;
    use crate::graph::Graph;
    use crate::graph_node::{Cardinality, GraphNode, InputPackageSpec, MessageQuery, NodeKind};
    use serde_json::json;

    fn message(text: &str) -> RunMessage {
        RunMessage::user(vec![ContentBlock::text(text)]).unwrap()
    }

    #[derive(Debug)]
    struct EmitExecutor;

    impl NodeExecutor for EmitExecutor {
        fn execute(
            &self,
            node: rt::NodeSpec,
            _input: NodeInput,
            _ctx: NodeExecutionContext,
        ) -> BoxFuture<'static, AgentCoreResult<NodeOutput>> {
            Box::pin(async move {
                match node.kind {
                    NodeKind::Final => Ok(NodeOutput::new()),
                    _ => Ok(NodeOutput::new().with_message(
                        "out",
                        RunMessage::assistant(vec![ContentBlock::text(format!(
                            "{} output",
                            node.id
                        ))])?,
                    )),
                }
            })
        }
    }

    #[test]
    fn runner_executes_package_graph_with_runtime_state() {
        let graph = Graph::builder("test")
            .node(GraphNode::final_node(
                "final",
                InputPackageSpec::new("input").required(
                    "turn",
                    MessageQuery::any(),
                    Cardinality::Latest,
                ),
            ))
            .edge("input_to_final", ("input", "messages"), ("final", "input"))
            .finish_at("final")
            .build()
            .unwrap();

        let result = GraphRunner::new()
            .run(&graph, GraphRunInput::new(vec![message("go")]))
            .unwrap();

        assert_eq!(result.status, GraphRunStatus::Completed);
        assert_eq!(result.ledger.transfers.len(), 1);
        assert_eq!(result.ledger.node_attempts.len(), 1);
        assert_eq!(
            result.state.package_states.values().next().unwrap().version,
            1
        );
    }

    #[test]
    fn runner_uses_custom_executor_for_agent_or_tool_nodes() {
        let graph = Graph::builder("test")
            .node(
                GraphNode::new(
                    "source",
                    NodeKind::Transform {
                        executor: "emit".to_string(),
                        config: json!({}),
                    },
                    InputPackageSpec::new("input").required(
                        "turn",
                        MessageQuery::any(),
                        Cardinality::Latest,
                    ),
                )
                .output("out"),
            )
            .node(GraphNode::final_node(
                "final",
                InputPackageSpec::new("done").required(
                    "text",
                    MessageQuery::where_exists("content[*].text"),
                    Cardinality::Latest,
                ),
            ))
            .edge(
                "input_to_source",
                ("input", "messages"),
                ("source", "input"),
            )
            .edge("source_to_final", ("source", "out"), ("final", "done"))
            .finish_at("final")
            .build()
            .unwrap();

        let result = GraphRunner::with_executor(Arc::new(EmitExecutor))
            .run(&graph, GraphRunInput::new(vec![message("go")]))
            .unwrap();

        assert_eq!(result.status, GraphRunStatus::Completed);
        assert!(result.messages.iter().any(|message| message
            .content
            .contains(&ContentBlock::text("source output"))));
    }

    #[test]
    fn runner_reports_cancelled_before_runtime_starts() {
        let graph = Graph::builder("test")
            .node(GraphNode::final_node(
                "final",
                InputPackageSpec::new("input").required(
                    "turn",
                    MessageQuery::any(),
                    Cardinality::Latest,
                ),
            ))
            .finish_at("final")
            .build()
            .unwrap();

        let result = GraphRunner::new()
            .run(
                &graph,
                GraphRunInput::new(vec![message("stop")]).with_stop_requested(true),
            )
            .unwrap();

        assert_eq!(result.status, GraphRunStatus::Cancelled);
        assert!(result.state.output_logs.is_empty());
    }
}

use crate::agent_definition::AgentDefinition;
use crate::agent_factory::AgentServices;
use crate::error::AgentCoreResult;
use crate::event::CoreEvent;
use crate::graph::Graph;
use crate::graph_runner::{GraphRunInput, GraphRunStatus};
use crate::run_message::RunMessage;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use uuid::Uuid;

#[derive(Debug)]
pub struct Agent {
    id: Uuid,
    definition: Arc<AgentDefinition>,
    services: AgentServices,
    cancellation: Arc<AtomicBool>,
}

impl Agent {
    pub fn new(definition: Arc<AgentDefinition>, services: AgentServices) -> Self {
        Self {
            id: Uuid::new_v4(),
            definition,
            services,
            cancellation: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn definition(&self) -> &AgentDefinition {
        &self.definition
    }

    pub fn services(&self) -> &AgentServices {
        &self.services
    }

    pub fn cancellation_token(&self) -> AgentCancellationToken {
        AgentCancellationToken {
            inner: Arc::clone(&self.cancellation),
        }
    }

    pub fn cancel(&self) {
        self.cancellation.store(true, Ordering::SeqCst);
    }

    pub fn run(&self, input: AgentRunInput) -> AgentCoreResult<AgentRunResult> {
        let graph_name = input.graph.name().to_string();
        let stop_requested = input.stop_requested || self.cancellation.load(Ordering::SeqCst);
        let mut events = vec![CoreEvent::AgentStarted {
            agent_id: self.id,
            agent_name: self.definition.name().to_string(),
            graph_name: graph_name.clone(),
        }];

        let mut graph_input =
            GraphRunInput::new(input.initial_messages).with_stop_requested(stop_requested);
        if let Some(run_id) = input.run_id {
            graph_input = graph_input.with_run_id(run_id);
        }
        let graph_result = self.services.graph_runner.run(&input.graph, graph_input)?;

        events.extend(graph_result.events.clone());

        let status = match graph_result.status {
            GraphRunStatus::Completed | GraphRunStatus::Drained => AgentRunStatus::Completed,
            GraphRunStatus::Cancelled => AgentRunStatus::Cancelled,
            GraphRunStatus::BudgetExceeded | GraphRunStatus::Failed => AgentRunStatus::Failed,
        };

        events.push(CoreEvent::AgentEnded {
            agent_id: self.id,
            agent_name: self.definition.name().to_string(),
            status: status.as_str().to_string(),
        });

        Ok(AgentRunResult {
            agent_id: self.id,
            run_id: graph_result.run_id,
            graph_name: Some(graph_name),
            messages: graph_result.messages,
            events,
            status,
            error: graph_result.error,
        })
    }
}

#[derive(Clone, Debug)]
pub struct AgentCancellationToken {
    inner: Arc<AtomicBool>,
}

impl AgentCancellationToken {
    pub fn is_cancelled(&self) -> bool {
        self.inner.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Debug)]
pub struct AgentRunInput {
    pub run_id: Option<Uuid>,
    pub graph: Graph,
    pub initial_messages: Vec<RunMessage>,
    pub stop_requested: bool,
}

impl AgentRunInput {
    pub fn new(graph: Graph) -> Self {
        Self {
            run_id: None,
            graph,
            initial_messages: Vec::new(),
            stop_requested: false,
        }
    }

    pub fn with_initial_messages(mut self, messages: Vec<RunMessage>) -> Self {
        self.initial_messages = messages;
        self
    }

    pub fn with_stop_requested(mut self, stop_requested: bool) -> Self {
        self.stop_requested = stop_requested;
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentRunResult {
    pub agent_id: Uuid,
    pub run_id: Uuid,
    pub graph_name: Option<String>,
    pub messages: Vec<RunMessage>,
    pub events: Vec<CoreEvent>,
    pub status: AgentRunStatus,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Completed,
    Cancelled,
    Failed,
}

impl AgentRunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_definition::AgentDefinitionBuilder;
    use crate::content_block::ContentBlock;
    use crate::graph::Graph;
    use crate::graph_node::{Cardinality, GraphNode, InputPackageSpec, MessageQuery};

    fn test_agent() -> Agent {
        let definition = AgentDefinitionBuilder::new()
            .name("coder")
            .system_prompt("prompt")
            .build()
            .unwrap();
        Agent::new(Arc::new(definition), AgentServices::default())
    }

    #[test]
    fn cancellation_token_tracks_agent_cancel() {
        let agent = test_agent();
        let token = agent.cancellation_token();

        assert!(!token.is_cancelled());
        agent.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn run_delegates_to_graph_runner() {
        let agent = test_agent();
        let graph = Graph::builder("test")
            .node(GraphNode::final_node(
                "final",
                InputPackageSpec::new("input").required(
                    "turn",
                    MessageQuery::any(),
                    Cardinality::Latest,
                ),
            ))
            .edge("input_to_final", "input", ("final", "input"))
            .finish_at("final")
            .build()
            .unwrap();
        let message = RunMessage::user(vec![ContentBlock::text("hello")]).unwrap();

        let result = agent
            .run(AgentRunInput::new(graph).with_initial_messages(vec![message]))
            .unwrap();

        assert_eq!(result.status, AgentRunStatus::Completed);
        assert_eq!(result.messages.len(), 1);
        assert!(result
            .events
            .iter()
            .any(|event| matches!(event, CoreEvent::AgentStarted { .. })));
    }
}

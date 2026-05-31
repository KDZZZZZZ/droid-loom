use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AgentCoreError, AgentCoreResult};
use crate::event::CoreEvent;
use crate::graph::Graph;
use crate::graph_runner::{GraphRunInput, GraphRunResult, GraphRunner};
use crate::graph_runtime::NodeExecutor;
use crate::run_message::{MessageRole, MessageStatus, RunMessage};

pub type TurnGenInputFn =
    Arc<dyn Fn(TurnGenInput) -> AgentCoreResult<TurnGenInputResult> + Send + Sync>;
pub type TurnPrepareGraphFn =
    Arc<dyn Fn(TurnPrepareGraphInput) -> AgentCoreResult<Graph> + Send + Sync>;
pub type TurnEventHandlerFn = Arc<dyn Fn(TurnEventBatch) -> AgentCoreResult<()> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnLoopState {
    Idle,
    Running,
    Stopping,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum TurnContextPolicy {
    FullHistory,
    LatestUserOnly,
    LastMessages(usize),
}

impl Default for TurnContextPolicy {
    fn default() -> Self {
        Self::FullHistory
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnGenInput {
    pub pending_items: Vec<RunMessage>,
    pub history: Vec<RunMessage>,
    pub context_policy: TurnContextPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnGenInputResult {
    pub input_messages: Vec<RunMessage>,
    pub consumed: Vec<Uuid>,
    pub remaining: Vec<RunMessage>,
}

impl TurnGenInputResult {
    pub fn new(input_messages: Vec<RunMessage>, consumed: Vec<Uuid>) -> Self {
        Self {
            input_messages,
            consumed,
            remaining: Vec::new(),
        }
    }

    pub fn with_remaining(mut self, remaining: Vec<RunMessage>) -> Self {
        self.remaining = remaining;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnPrepareGraphInput {
    pub turn_id: Uuid,
    pub consumed: Vec<RunMessage>,
    pub input_messages: Vec<RunMessage>,
    pub history: Vec<RunMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnEventBatch {
    pub turn_id: Uuid,
    pub events: Vec<CoreEvent>,
    pub appended_messages: Vec<RunMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnRunResult {
    pub turn_id: Uuid,
    pub consumed: Vec<RunMessage>,
    pub remaining: Vec<RunMessage>,
    pub context_messages: Vec<RunMessage>,
    pub appended_messages: Vec<RunMessage>,
    pub graph: GraphRunResult,
}

#[derive(Clone)]
pub struct TurnLoop {
    state: TurnLoopState,
    buffer: VecDeque<RunMessage>,
    late_items: Vec<RunMessage>,
    messages: Vec<RunMessage>,
    runner: GraphRunner,
    graph: Option<Graph>,
    gen_input: TurnGenInputFn,
    prepare_graph: Option<TurnPrepareGraphFn>,
    on_turn_events: Option<TurnEventHandlerFn>,
    context_policy: TurnContextPolicy,
    max_ticks: usize,
    active_turn_id: Option<Uuid>,
    stop_requested: bool,
}

impl TurnLoop {
    pub fn new() -> Self {
        Self::with_runner(GraphRunner::new())
    }

    fn with_runner(runner: GraphRunner) -> Self {
        Self {
            state: TurnLoopState::Idle,
            buffer: VecDeque::new(),
            late_items: Vec::new(),
            messages: Vec::new(),
            runner,
            graph: None,
            gen_input: default_gen_input(),
            prepare_graph: None,
            on_turn_events: None,
            context_policy: TurnContextPolicy::default(),
            max_ticks: 10_000,
            active_turn_id: None,
            stop_requested: false,
        }
    }

    pub fn with_executor(executor: Arc<dyn NodeExecutor>) -> Self {
        Self::with_runner(GraphRunner::with_executor(executor))
    }

    pub fn with_graph(mut self, graph: Graph) -> Self {
        self.graph = Some(graph);
        self
    }

    pub fn set_graph(&mut self, graph: Graph) {
        self.graph = Some(graph);
    }

    pub fn with_gen_input(mut self, gen_input: TurnGenInputFn) -> Self {
        self.gen_input = gen_input;
        self
    }

    pub fn set_gen_input(&mut self, gen_input: TurnGenInputFn) {
        self.gen_input = gen_input;
    }

    pub fn with_prepare_graph(mut self, prepare_graph: TurnPrepareGraphFn) -> Self {
        self.prepare_graph = Some(prepare_graph);
        self
    }

    pub fn set_prepare_graph(&mut self, prepare_graph: TurnPrepareGraphFn) {
        self.prepare_graph = Some(prepare_graph);
    }

    pub fn with_on_turn_events(mut self, on_turn_events: TurnEventHandlerFn) -> Self {
        self.on_turn_events = Some(on_turn_events);
        self
    }

    pub fn set_on_turn_events(&mut self, on_turn_events: TurnEventHandlerFn) {
        self.on_turn_events = Some(on_turn_events);
    }

    pub fn with_context_policy(mut self, policy: TurnContextPolicy) -> Self {
        self.context_policy = policy;
        self
    }

    pub fn set_context_policy(&mut self, policy: TurnContextPolicy) {
        self.context_policy = policy;
    }

    pub fn context_policy(&self) -> &TurnContextPolicy {
        &self.context_policy
    }

    pub fn with_max_ticks(mut self, max_ticks: usize) -> Self {
        self.max_ticks = max_ticks;
        self
    }

    pub fn set_max_ticks(&mut self, max_ticks: usize) {
        self.max_ticks = max_ticks;
    }

    pub fn state(&self) -> TurnLoopState {
        self.state
    }

    pub fn pending_len(&self) -> usize {
        self.buffer.len()
    }

    pub fn messages(&self) -> &[RunMessage] {
        &self.messages
    }

    pub fn late_items(&self) -> &[RunMessage] {
        &self.late_items
    }

    pub fn take_late_items(&mut self) -> Vec<RunMessage> {
        std::mem::take(&mut self.late_items)
    }

    pub fn append_messages(
        &mut self,
        messages: impl IntoIterator<Item = RunMessage>,
    ) -> AgentCoreResult<usize> {
        Ok(self.append_new_messages(messages).len())
    }

    pub fn push(&mut self, item: RunMessage) -> AgentCoreResult<bool> {
        self.validate_push_item(&item)?;
        if self.stop_requested || matches!(self.state, TurnLoopState::Stopped) {
            self.late_items.push(item);
            return Ok(false);
        }
        self.buffer.push_back(item);
        Ok(true)
    }

    pub fn run_message(&mut self, message: RunMessage) -> AgentCoreResult<TurnRunResult> {
        if message.role != MessageRole::User {
            return Err(AgentCoreError::InvalidInput(
                "turn loop run_message only accepts role=user messages".to_string(),
            ));
        }
        if !self.push(message)? {
            return Err(AgentCoreError::InvalidInput(
                "cannot run a message while turn loop is stopped".to_string(),
            ));
        }
        self.run_once()?.ok_or_else(|| {
            AgentCoreError::InvalidInput("pushed message did not produce a turn".to_string())
        })
    }

    pub fn run_once(&mut self) -> AgentCoreResult<Option<TurnRunResult>> {
        if self.stop_requested
            || matches!(self.state, TurnLoopState::Stopping | TurnLoopState::Stopped)
        {
            self.state = TurnLoopState::Stopped;
            return Ok(None);
        }
        if self.active_turn_id.is_some() {
            return Err(AgentCoreError::InvalidInput(
                "cannot start a new turn while another turn is running".to_string(),
            ));
        }
        if self.buffer.is_empty() {
            self.state = TurnLoopState::Idle;
            return Ok(None);
        }

        let turn_id = Uuid::new_v4();
        self.active_turn_id = Some(turn_id);
        self.state = TurnLoopState::Running;

        let pending_items: Vec<_> = self.buffer.iter().cloned().collect();
        let gen_input = (self.gen_input)(TurnGenInput {
            pending_items: pending_items.clone(),
            history: self.messages.clone(),
            context_policy: self.context_policy.clone(),
        });

        let gen_input = match gen_input {
            Ok(value) => value,
            Err(error) => {
                self.finish_active_turn();
                return Err(error);
            }
        };

        let consumed = consumed_items(&pending_items, &gen_input.consumed)?;
        self.buffer = VecDeque::from(gen_input.remaining.clone());
        if gen_input.input_messages.is_empty() || consumed.is_empty() {
            self.finish_active_turn();
            return Ok(None);
        }

        let context_message_ids = message_ids(gen_input.input_messages.iter());
        self.append_new_messages(consumed.clone());

        let graph = match self.prepare_graph(turn_id, &consumed, &gen_input.input_messages) {
            Ok(graph) => graph,
            Err(error) => {
                self.finish_active_turn();
                return Err(error);
            }
        };

        let graph_result = self.runner.run(
            &graph,
            GraphRunInput::new(gen_input.input_messages.clone()).with_max_ticks(self.max_ticks),
        );

        match graph_result {
            Ok(graph) => {
                let appended_messages = self.append_new_messages(
                    graph
                        .messages
                        .iter()
                        .filter(|message| !context_message_ids.contains(&message.id))
                        .cloned(),
                );
                if let Some(on_turn_events) = &self.on_turn_events {
                    if let Err(error) = on_turn_events(TurnEventBatch {
                        turn_id,
                        events: graph.events.clone(),
                        appended_messages: appended_messages.clone(),
                    }) {
                        self.finish_active_turn();
                        return Err(error);
                    }
                }
                self.finish_active_turn();
                Ok(Some(TurnRunResult {
                    turn_id,
                    consumed,
                    remaining: gen_input.remaining,
                    context_messages: gen_input.input_messages,
                    appended_messages,
                    graph,
                }))
            }
            Err(error) => {
                self.finish_active_turn();
                Err(error)
            }
        }
    }

    pub fn run_pending(&mut self) -> AgentCoreResult<Vec<TurnRunResult>> {
        let mut turns = Vec::new();
        while !self.buffer.is_empty() {
            let Some(turn) = self.run_once()? else {
                break;
            };
            turns.push(turn);
        }
        Ok(turns)
    }

    pub fn request_stop(&mut self) {
        self.stop_requested = true;
        self.state = if self.active_turn_id.is_some() {
            TurnLoopState::Stopping
        } else {
            TurnLoopState::Stopped
        };
    }

    pub fn clear_stop(&mut self) {
        self.stop_requested = false;
        if self.active_turn_id.is_none() {
            self.state = TurnLoopState::Idle;
        }
    }

    pub fn stop_requested(&self) -> bool {
        self.stop_requested
    }

    fn validate_push_item(&self, item: &RunMessage) -> AgentCoreResult<()> {
        if item.status == MessageStatus::Streaming {
            return Err(AgentCoreError::InvalidInput(
                "turn loop only accepts finalized or aborted messages".to_string(),
            ));
        }
        Ok(())
    }

    fn prepare_graph(
        &self,
        turn_id: Uuid,
        consumed: &[RunMessage],
        input_messages: &[RunMessage],
    ) -> AgentCoreResult<Graph> {
        if let Some(prepare_graph) = &self.prepare_graph {
            return prepare_graph(TurnPrepareGraphInput {
                turn_id,
                consumed: consumed.to_vec(),
                input_messages: input_messages.to_vec(),
                history: self.messages.clone(),
            });
        }
        self.graph.clone().ok_or_else(|| {
            AgentCoreError::InvalidConfig(
                "turn loop has no graph; call with_graph, set_graph, or with_prepare_graph"
                    .to_string(),
            )
        })
    }

    fn finish_active_turn(&mut self) {
        self.active_turn_id = None;
        self.state = if self.stop_requested {
            TurnLoopState::Stopped
        } else {
            TurnLoopState::Idle
        };
    }

    fn append_new_messages(
        &mut self,
        messages: impl IntoIterator<Item = RunMessage>,
    ) -> Vec<RunMessage> {
        let mut known_ids = message_ids(self.messages.iter());
        let mut appended = Vec::new();
        for message in messages {
            if known_ids.insert(message.id) {
                self.messages.push(message.clone());
                appended.push(message);
            }
        }
        appended
    }
}

fn default_gen_input() -> TurnGenInputFn {
    Arc::new(|input: TurnGenInput| {
        let mut input_messages = match input.context_policy {
            TurnContextPolicy::FullHistory => input.history.clone(),
            TurnContextPolicy::LatestUserOnly => Vec::new(),
            TurnContextPolicy::LastMessages(max_messages) => {
                let start = input.history.len().saturating_sub(max_messages);
                input.history[start..].to_vec()
            }
        };
        input_messages.extend(input.pending_items.iter().cloned());
        let consumed = input
            .pending_items
            .iter()
            .map(|message| message.id)
            .collect();
        Ok(TurnGenInputResult::new(input_messages, consumed))
    })
}

fn consumed_items(
    pending_items: &[RunMessage],
    consumed_ids: &[Uuid],
) -> AgentCoreResult<Vec<RunMessage>> {
    let pending_ids = message_ids(pending_items.iter());
    for consumed_id in consumed_ids {
        if !pending_ids.contains(consumed_id) {
            return Err(AgentCoreError::InvalidInput(format!(
                "GenInput consumed unknown item id: {consumed_id}"
            )));
        }
    }
    let consumed_id_set: HashSet<_> = consumed_ids.iter().copied().collect();
    Ok(pending_items
        .iter()
        .filter(|message| consumed_id_set.contains(&message.id))
        .cloned()
        .collect())
}

fn message_ids<'a>(messages: impl IntoIterator<Item = &'a RunMessage>) -> HashSet<Uuid> {
    messages.into_iter().map(|message| message.id).collect()
}

impl Default for TurnLoop {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TurnLoop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnLoop")
            .field("state", &self.state)
            .field("pending_len", &self.buffer.len())
            .field("late_items", &self.late_items.len())
            .field("messages", &self.messages.len())
            .field("context_policy", &self.context_policy)
            .field("max_ticks", &self.max_ticks)
            .field("active_turn_id", &self.active_turn_id)
            .field("stop_requested", &self.stop_requested)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph_node::{Cardinality, GraphNode, InputPackageSpec, MessageQuery, NodeKind};
    use crate::graph_runner::GraphRunStatus;
    use crate::graph_runtime::{NodeExecutionContext, NodeInput, NodeResult, NodeSpec};
    use futures::future::BoxFuture;
    use serde_json::json;

    #[test]
    fn default_gen_input_builds_graph_start_snapshot() {
        let mut turn_loop = TurnLoop::new().with_graph(final_graph());
        let old_user = RunMessage::user_text("old user").unwrap();
        let old_assistant = RunMessage::assistant_text("old assistant").unwrap();
        let new_user = RunMessage::user_text("new user").unwrap();

        turn_loop
            .append_messages([old_user.clone(), old_assistant.clone()])
            .unwrap();
        turn_loop.push(new_user.clone()).unwrap();
        let turn = turn_loop.run_once().unwrap().unwrap();
        assert_eq!(turn.context_messages.len(), 3);

        let mut turn_loop = TurnLoop::new()
            .with_graph(final_graph())
            .with_context_policy(TurnContextPolicy::LatestUserOnly);
        turn_loop
            .append_messages([old_user.clone(), old_assistant.clone()])
            .unwrap();
        turn_loop.push(new_user.clone()).unwrap();
        let turn = turn_loop.run_once().unwrap().unwrap();
        assert_eq!(turn.context_messages, vec![new_user.clone()]);

        let mut turn_loop = TurnLoop::new()
            .with_graph(final_graph())
            .with_context_policy(TurnContextPolicy::LastMessages(1));
        turn_loop
            .append_messages([old_user, old_assistant.clone()])
            .unwrap();
        turn_loop.push(new_user.clone()).unwrap();
        let turn = turn_loop.run_once().unwrap().unwrap();
        assert_eq!(turn.context_messages, vec![old_assistant, new_user]);
    }

    #[test]
    fn run_message_executes_graph_and_appends_only_emitted_messages() {
        let graph = Graph::builder("turn_graph")
            .node(GraphNode::new(
                "agent",
                NodeKind::Transform {
                    executor: "emit".to_string(),
                    config: json!({}),
                },
                InputPackageSpec::new("context").required(
                    "turn",
                    MessageQuery::any(),
                    Cardinality::Latest,
                ),
            ))
            .node(GraphNode::final_node(
                "final",
                InputPackageSpec::new("answer").required(
                    "answer",
                    MessageQuery::where_exists("content[*].text"),
                    Cardinality::Latest,
                ),
            ))
            .edge("input_to_agent", "input", ("agent", "context"))
            .edge("agent_to_final", "agent", ("final", "answer"))
            .finish_at("final")
            .build()
            .unwrap();
        let mut turn_loop = TurnLoop::with_executor(Arc::new(EmitExecutor)).with_graph(graph);
        let run = turn_loop
            .run_message(RunMessage::user_text("inspect settings").unwrap())
            .unwrap();

        assert_eq!(run.graph.status, GraphRunStatus::Completed);
        assert_eq!(run.consumed.len(), 1);
        assert_eq!(run.context_messages.len(), 1);
        assert_eq!(run.appended_messages.len(), 1);
        assert_eq!(turn_loop.messages().len(), 2);
        assert_eq!(turn_loop.state(), TurnLoopState::Idle);
    }

    #[test]
    fn custom_gen_input_can_leave_items_for_next_turn() {
        let gen_input: TurnGenInputFn = Arc::new(|input| {
            let first = input.pending_items.first().unwrap().clone();
            let remaining = input.pending_items.iter().skip(1).cloned().collect();
            Ok(TurnGenInputResult::new(vec![first.clone()], vec![first.id])
                .with_remaining(remaining))
        });
        let mut turn_loop = TurnLoop::new()
            .with_graph(final_graph())
            .with_gen_input(gen_input);
        turn_loop
            .push(RunMessage::user_text("one").unwrap())
            .unwrap();
        turn_loop
            .push(RunMessage::user_text("two").unwrap())
            .unwrap();

        let first = turn_loop.run_once().unwrap().unwrap();
        assert_eq!(first.consumed.len(), 1);
        assert_eq!(turn_loop.pending_len(), 1);
        let second = turn_loop.run_once().unwrap().unwrap();
        assert_eq!(second.consumed.len(), 1);
        assert_eq!(turn_loop.pending_len(), 0);
    }

    #[test]
    fn prepare_graph_can_select_graph_per_turn() {
        let prepare_graph: TurnPrepareGraphFn = Arc::new(|input| {
            assert_eq!(input.consumed.len(), 1);
            Ok(final_graph())
        });
        let mut turn_loop = TurnLoop::new().with_prepare_graph(prepare_graph);
        turn_loop
            .push(RunMessage::user_text("go").unwrap())
            .unwrap();

        let turn = turn_loop.run_once().unwrap().unwrap();
        assert_eq!(turn.graph.status, GraphRunStatus::Completed);
    }

    #[test]
    fn on_turn_events_observes_graph_events() {
        let seen = Arc::new(std::sync::Mutex::new(0usize));
        let seen_clone = Arc::clone(&seen);
        let on_events: TurnEventHandlerFn = Arc::new(move |batch| {
            *seen_clone.lock().unwrap() += batch.events.len();
            Ok(())
        });
        let mut turn_loop = TurnLoop::new()
            .with_graph(final_graph())
            .with_on_turn_events(on_events);

        turn_loop
            .run_message(RunMessage::user_text("go").unwrap())
            .unwrap();

        assert!(*seen.lock().unwrap() > 0);
    }

    #[test]
    fn stop_rejects_push_and_records_late_items() {
        let mut turn_loop = TurnLoop::new().with_graph(final_graph());
        turn_loop.request_stop();

        assert!(!turn_loop
            .push(RunMessage::user_text("late").unwrap())
            .unwrap());
        assert_eq!(turn_loop.late_items().len(), 1);
        assert!(turn_loop
            .run_message(RunMessage::user_text("hello").unwrap())
            .is_err());
        assert_eq!(turn_loop.take_late_items().len(), 2);
        assert_eq!(turn_loop.state(), TurnLoopState::Stopped);
    }

    fn final_graph() -> Graph {
        Graph::builder("final_only")
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
            .unwrap()
    }

    #[derive(Debug)]
    struct EmitExecutor;

    impl NodeExecutor for EmitExecutor {
        fn execute(
            &self,
            node: NodeSpec,
            _input: NodeInput,
            _ctx: NodeExecutionContext,
        ) -> BoxFuture<'static, AgentCoreResult<NodeResult>> {
            Box::pin(async move {
                match node.kind {
                    NodeKind::Final => Ok(NodeResult::new()),
                    _ => Ok(NodeResult::new()
                        .with_message(RunMessage::assistant_text(format!("{} reply", node.id))?)),
                }
            })
        }
    }
}

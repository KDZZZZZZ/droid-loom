use agent_core::agent_definition::{AgentDefinition, ToolVisibility};
use agent_core::content_block::ContentBlock;
use agent_core::run_message::{MessageRole, RunMessage};
use agent_core::session_replay::ReplaySnapshot;
use agent_core::tool_executor::{ToolCall, ToolExecutor};
use agent_core::tool_registry::ToolRegistry;
use agent_core::tool_result::ToolResult;
use agent_core::AgentCoreResult;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default)]
pub struct TrajectoryProbabilityGraph {
    session_count: usize,
    node_counts: BTreeMap<String, usize>,
    transitions: BTreeMap<String, BTreeMap<String, usize>>,
    tool_call_counts: BTreeMap<String, usize>,
    tool_session_counts: BTreeMap<String, usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransitionPrediction {
    pub event: String,
    pub probability: f64,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreexecutionPlan {
    pub calls: Vec<ToolCall>,
    pub skipped: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreexecutionOutcome {
    pub results: Vec<ToolResult>,
    pub messages: Vec<RunMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolLayerPlan {
    pub direct_tools: Vec<String>,
    pub dynamic_tools: Vec<String>,
    pub hidden_tools: Vec<String>,
    pub visibility: BTreeMap<String, ToolVisibility>,
}

impl TrajectoryProbabilityGraph {
    pub fn from_sessions(sessions: &[Vec<RunMessage>]) -> Self {
        let mut graph = Self::default();
        for session in sessions {
            graph.observe_session(session);
        }
        graph
    }

    pub fn from_replay_snapshots(snapshots: &[ReplaySnapshot]) -> Self {
        let sessions = snapshots
            .iter()
            .map(|snapshot| snapshot.messages.clone())
            .collect::<Vec<_>>();
        Self::from_sessions(&sessions)
    }

    pub fn observe_session(&mut self, messages: &[RunMessage]) {
        self.session_count += 1;
        let events = message_events(messages);
        let mut tools_seen_in_session = BTreeSet::new();

        for event in &events {
            *self.node_counts.entry(event.clone()).or_default() += 1;
            if let Some(tool_name) = tool_call_name(event) {
                *self
                    .tool_call_counts
                    .entry(tool_name.to_string())
                    .or_default() += 1;
                tools_seen_in_session.insert(tool_name.to_string());
            }
        }

        for tool_name in tools_seen_in_session {
            *self.tool_session_counts.entry(tool_name).or_default() += 1;
        }

        for pair in events.windows(2) {
            let from = pair[0].clone();
            let to = pair[1].clone();
            *self
                .transitions
                .entry(from)
                .or_default()
                .entry(to)
                .or_default() += 1;
        }
    }

    pub fn session_count(&self) -> usize {
        self.session_count
    }

    pub fn likely_next(&self, event: &str, limit: usize) -> Vec<TransitionPrediction> {
        let Some(outgoing) = self.transitions.get(event) else {
            return Vec::new();
        };
        let total = outgoing.values().sum::<usize>();
        if total == 0 {
            return Vec::new();
        }

        let mut predictions = outgoing
            .iter()
            .map(|(event, count)| TransitionPrediction {
                event: event.clone(),
                probability: *count as f64 / total as f64,
                count: *count,
            })
            .collect::<Vec<_>>();
        predictions.sort_by(|left, right| {
            right
                .probability
                .partial_cmp(&left.probability)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.event.cmp(&right.event))
        });
        predictions.truncate(limit);
        predictions
    }

    pub fn tool_use_probability(&self, tool_name: &str) -> f64 {
        if self.session_count == 0 {
            return 0.0;
        }
        self.tool_session_counts
            .get(tool_name)
            .copied()
            .unwrap_or_default() as f64
            / self.session_count as f64
    }

    pub fn plan_preexecution(
        &self,
        registry: &ToolRegistry,
        definition: &AgentDefinition,
        candidates: Vec<ToolCall>,
        min_probability: f64,
    ) -> PreexecutionPlan {
        let mut calls = Vec::new();
        let mut skipped = Vec::new();

        for candidate in candidates {
            let probability = self.tool_use_probability(&candidate.tool_name);
            if probability < min_probability {
                skipped.push(format!("{}:probability", candidate.tool_name));
                continue;
            }

            match registry.get_for_agent(definition, &candidate.tool_name) {
                Ok(tool) if tool.metadata().can_preexecute() => calls.push(candidate),
                Ok(_) => skipped.push(format!("{}:not_preexecutable", candidate.tool_name)),
                Err(_) => skipped.push(format!("{}:not_visible", candidate.tool_name)),
            }
        }

        PreexecutionPlan { calls, skipped }
    }

    pub fn execute_preexecution_plan(
        executor: &ToolExecutor,
        definition: &AgentDefinition,
        plan: PreexecutionPlan,
    ) -> AgentCoreResult<PreexecutionOutcome> {
        let results = executor.execute_batch_parallel(definition, plan.calls)?;
        let messages = results
            .iter()
            .map(|result| result.into_run_message())
            .collect::<AgentCoreResult<Vec<_>>>()?;
        Ok(PreexecutionOutcome { results, messages })
    }

    pub fn plan_tool_layers(
        &self,
        base_visibility: &BTreeMap<String, ToolVisibility>,
        min_hot_probability: f64,
    ) -> ToolLayerPlan {
        let mut direct_tools = Vec::new();
        let mut dynamic_tools = Vec::new();
        let mut hidden_tools = Vec::new();
        let mut visibility = BTreeMap::new();

        for (tool_name, current_visibility) in base_visibility {
            let next_visibility = match current_visibility {
                ToolVisibility::Hidden => {
                    hidden_tools.push(tool_name.clone());
                    ToolVisibility::Hidden
                }
                _ if self.tool_use_probability(tool_name) >= min_hot_probability => {
                    direct_tools.push(tool_name.clone());
                    ToolVisibility::Direct
                }
                _ => {
                    dynamic_tools.push(tool_name.clone());
                    ToolVisibility::Searchable
                }
            };
            visibility.insert(tool_name.clone(), next_visibility);
        }

        ToolLayerPlan {
            direct_tools,
            dynamic_tools,
            hidden_tools,
            visibility,
        }
    }
}

fn message_events(messages: &[RunMessage]) -> Vec<String> {
    let mut events = Vec::new();
    for message in messages {
        events.push(role_event(message.role).to_string());
        for block in &message.content {
            match block {
                ContentBlock::ToolCall { tool_name, .. } => {
                    events.push(format!("tool_call:{tool_name}"));
                }
                ContentBlock::ToolResult {
                    tool_name,
                    is_error,
                    ..
                } => {
                    let name = tool_name.as_deref().unwrap_or("unknown");
                    let status = if *is_error { "error" } else { "ok" };
                    events.push(format!("tool_result:{name}:{status}"));
                }
                _ => {}
            }
        }
    }
    events
}

fn role_event(role: MessageRole) -> &'static str {
    match role {
        MessageRole::User => "message:user",
        MessageRole::Assistant => "message:assistant",
        MessageRole::Tool => "message:tool",
        MessageRole::Diagnostic => "message:diagnostic",
    }
}

fn tool_call_name(event: &str) -> Option<&str> {
    event.strip_prefix("tool_call:")
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::assistant_builder::AssistantBuilder;
    use agent_core::content_block::ContentBlock;
    use agent_core::session_entry::SessionEntry;
    use agent_core::session_replay::replay_active_branch;
    use agent_core::session_store::{InMemorySessionStore, SessionStore};
    use agent_core::tool::{Tool, ToolInvocation, ToolMetadata, ToolOutput};
    use agent_core::tool_schema::ToolSchema;
    use agent_core::{AgentCoreResult, ToolVisibility};
    use serde_json::json;
    use std::sync::Arc;

    #[derive(Debug)]
    struct ReadOnlyTool {
        metadata: ToolMetadata,
    }

    impl ReadOnlyTool {
        fn new(name: &str) -> Self {
            let mut metadata = ToolMetadata::new(
                ToolSchema::empty_object(name, format!("{name} tool")).unwrap(),
                ToolVisibility::Direct,
            );
            metadata.capabilities.read_only = true;
            metadata.capabilities.idempotent = true;
            Self { metadata }
        }
    }

    impl Tool for ReadOnlyTool {
        fn metadata(&self) -> &ToolMetadata {
            &self.metadata
        }

        fn invoke(&self, invocation: ToolInvocation) -> AgentCoreResult<ToolOutput> {
            Ok(ToolOutput::new(json!({"call_id": invocation.call_id})))
        }
    }

    fn assistant_with_tools(tools: &[&str]) -> RunMessage {
        let mut builder = AssistantBuilder::new();
        for tool in tools {
            builder.push_tool_call(format!("call-{tool}"), *tool, json!({}));
        }
        builder.finish().unwrap()
    }

    #[test]
    fn predicts_next_events_and_preexecutes_read_only_tools() {
        let sessions = vec![
            vec![
                RunMessage::user(vec![ContentBlock::text("task")]).unwrap(),
                assistant_with_tools(&["ui_state", "search_database"]),
            ],
            vec![
                RunMessage::user(vec![ContentBlock::text("task")]).unwrap(),
                assistant_with_tools(&["ui_state"]),
            ],
        ];
        let graph = TrajectoryProbabilityGraph::from_sessions(&sessions);

        assert_eq!(graph.session_count(), 2);
        assert_eq!(
            graph.likely_next("message:assistant", 1)[0].event,
            "tool_call:ui_state"
        );

        let definition = agent_core::agent_definition::AgentDefinitionBuilder::new()
            .name("agent")
            .system_prompt("prompt")
            .tool_visibility("ui_state", ToolVisibility::Direct)
            .build()
            .unwrap();
        let mut registry = ToolRegistry::new();
        registry.register(ReadOnlyTool::new("ui_state")).unwrap();
        let executor = ToolExecutor::new(Arc::new(registry.clone()));

        let plan = graph.plan_preexecution(
            &registry,
            &definition,
            vec![ToolCall::new("pre-ui", "ui_state", json!({}))],
            0.9,
        );
        let outcome =
            TrajectoryProbabilityGraph::execute_preexecution_plan(&executor, &definition, plan)
                .unwrap();

        assert_eq!(outcome.results.len(), 1);
        assert_eq!(outcome.messages.len(), 1);
    }

    #[test]
    fn builds_from_session_replay_snapshots() {
        let user = RunMessage::user(vec![ContentBlock::text("task")]).unwrap();
        let assistant = assistant_with_tools(&["ui_state"]);
        let mut store = InMemorySessionStore::new();
        let header = SessionEntry::header(user.id);
        let mut parent_id = header.id;
        store.append(header).unwrap();
        for message in [user, assistant] {
            let entry = SessionEntry::message(Some(parent_id), message).unwrap();
            parent_id = entry.id;
            store.append(entry).unwrap();
        }

        let tree = store.load_tree().unwrap();
        let snapshot = replay_active_branch(&tree).unwrap();
        let graph = TrajectoryProbabilityGraph::from_replay_snapshots(&[snapshot]);

        assert_eq!(graph.session_count(), 1);
        assert_eq!(graph.tool_use_probability("ui_state"), 1.0);
    }
}

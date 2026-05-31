use agent_core::agent::{AgentRunInput, AgentRunStatus};
use agent_core::agent_definition::{AgentDefinitionBuilder, ToolVisibility};
use agent_core::assistant_builder::AssistantBuilder;
use agent_core::content_block::{ContentBlock, DiagnosticLevel};
use agent_core::context::{
    message_value_to_input_items, run_message_to_input_items, ContextBuildInput, ContextBuilder,
};
use agent_core::event::{CoreEvent, EventLog};
use agent_core::graph::Graph;
use agent_core::graph_edge::{ActivationCondition, ContextInheritPolicy, EdgeDecision, GraphEdge};
use agent_core::graph_node::{GraphNode, GraphNodeAction, NodeExecutionInput};
use agent_core::graph_runner::{GraphRunInput, GraphRunStatus, GraphRunner};
use agent_core::graph_state::{FiredEdgeRecord, GraphState, GraphStateBudget};
use agent_core::graph_templates::{default_react_graph, single_node_graph};
use agent_core::hook::{
    HookEventRequest, HookKind, HookName, HookPayload, PointHookDecision, WrapperRequest,
    WrapperResponse, WrapperResult,
};
use agent_core::hook_handler::{
    HandlerKind, HandlerRegistry, HandlerScope, PointHookStatus, WrapperNext,
};
use agent_core::llm_deepseek_chat::DeepSeekChatProvider;
use agent_core::llm_model::{LlmApi, LlmModel, ModelId};
use agent_core::llm_openai_responses::OpenAiResponsesProvider;
use agent_core::llm_provider::{LlmProvider, PreparedLlmRequest, ProviderService};
use agent_core::llm_registry::LlmRegistry;
use agent_core::llm_request::{
    LlmContentPart, LlmInputItem, LlmMessageRole, LlmRequest, LlmRequestOptions,
};
use agent_core::llm_stream::{stream_from_events, LlmStreamEvent, LlmUsage};
use agent_core::run_message::{MessageRole, MessageStatus, MessageUsage, RunMessage};
use agent_core::session_compaction::{plan_active_branch_compaction, CompactionPlan};
use agent_core::session_entry::{SessionEntry, SessionEntryKind};
use agent_core::session_replay::{replay_active_branch, replay_branch, ReplaySnapshot};
use agent_core::session_store::{InMemorySessionStore, SessionStore};
use agent_core::session_tree::SessionTree;
use agent_core::tool::{
    Tool, ToolCapabilities, ToolExecutionMetadata, ToolInvocation, ToolMetadata, ToolOutput,
    ToolResultPolicy,
};
use agent_core::tool_adapter::{ToolAdapter, ToolAdapterRegistry};
use agent_core::tool_executor::{ToolCall, ToolExecutor};
use agent_core::tool_permissions::{
    AllowAllToolPermissionPolicy, ToolPermissionContext, ToolPermissionDecision,
    ToolPermissionPolicy,
};
use agent_core::tool_registry::ToolRegistry;
use agent_core::tool_result::{ToolResult, ToolResultContent, ToolResultStatus};
use agent_core::tool_schema::{validate_tool_name, ToolSchema};
use agent_core::turn_loop::{TurnLoop, TurnLoopState};
use agent_core::user_input;
use agent_core::{AgentCoreError, AgentCoreResult, AgentDefinition, AgentFactory, AgentServices};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug)]
struct ContractTool {
    metadata: ToolMetadata,
}

impl ContractTool {
    fn new(name: &str, visibility: ToolVisibility, preexecutable: bool) -> Self {
        let mut metadata = ToolMetadata::new(
            ToolSchema::empty_object(name, format!("{name} contract tool")).unwrap(),
            visibility,
        );
        metadata.capabilities.read_only = preexecutable;
        metadata.capabilities.idempotent = preexecutable;
        metadata.execution.timeout_ms = Some(1_000);
        Self { metadata }
    }
}

impl Tool for ContractTool {
    fn metadata(&self) -> &ToolMetadata {
        &self.metadata
    }

    fn invoke(&self, invocation: ToolInvocation) -> AgentCoreResult<ToolOutput> {
        let mut output = ToolOutput::new(json!({
            "tool": invocation.tool_name,
            "call_id": invocation.call_id,
            "arguments": invocation.arguments,
        }));
        output.metadata.insert("contract".to_string(), json!(true));
        Ok(output)
    }
}

#[derive(Debug)]
struct ContractAdapter;

impl ToolAdapter for ContractAdapter {
    fn adapter_name(&self) -> &str {
        "contract_adapter"
    }

    fn load_tools(&self) -> AgentCoreResult<Vec<Arc<dyn Tool>>> {
        Ok(vec![Arc::new(ContractTool::new(
            "adapter_tool",
            ToolVisibility::Direct,
            true,
        ))])
    }
}

#[derive(Debug)]
struct MockProvider;

impl LlmProvider for MockProvider {
    fn provider_id(&self) -> &str {
        "mock"
    }

    fn api(&self) -> &str {
        "mock_api"
    }

    fn prepare_request(&self, _request: &LlmRequest) -> AgentCoreResult<PreparedLlmRequest> {
        Ok(PreparedLlmRequest {
            provider: "mock".to_string(),
            api: "mock_api".to_string(),
            endpoint: "mock://local".to_string(),
            headers: BTreeMap::new(),
            body: json!({"ok": true}),
            metadata: BTreeMap::new(),
        })
    }
}

fn contract_definition() -> AgentCoreResult<AgentDefinition> {
    AgentDefinitionBuilder::new()
        .name("contract-agent")
        .system_prompt("Follow the contract test.")
        .tool_visibility("contract_read", ToolVisibility::Direct)
        .tool_visibility("contract_search", ToolVisibility::Searchable)
        .tool_visibility("contract_hidden", ToolVisibility::Hidden)
        .build()
}

fn passthrough_graph() -> AgentCoreResult<Graph> {
    Graph::builder("contract_graph")
        .node(
            GraphNode::new("start")
                .with_label("Start")
                .with_action(GraphNodeAction::PassthroughInput)
                .terminal(true),
        )
        .start_node("start")
        .end_node("start")
        .budget(GraphStateBudget::unlimited())
        .build()
}

#[test]
fn agent_public_api_contract() -> AgentCoreResult<()> {
    let definition = contract_definition()?;
    assert_eq!(definition.name(), "contract-agent");
    assert!(definition.system_prompt().contains("contract"));
    assert_eq!(
        definition.tool_visibility().get("contract_search"),
        Some(&ToolVisibility::Searchable)
    );
    definition.validate()?;

    let direct_definition = AgentDefinition::new("direct-agent", "prompt", BTreeMap::new())?;
    assert_eq!(direct_definition.name(), "direct-agent");

    let factory = AgentFactory::new(AgentServices::default());
    let agent = factory.create(definition)?;
    assert_eq!(agent.definition().name(), "contract-agent");
    let _services = agent.services();
    let token = agent.cancellation_token();
    assert!(!token.is_cancelled());

    let result = agent.run(
        AgentRunInput::new(passthrough_graph()?)
            .with_initial_messages(vec![user_input::text_message("hello")?]),
    )?;
    assert_eq!(result.agent_id, agent.id());
    assert_eq!(result.status, AgentRunStatus::Completed);
    assert_eq!(AgentRunStatus::Completed.as_str(), "completed");
    assert_eq!(result.graph_name.as_deref(), Some("contract_graph"));
    assert_eq!(result.messages.len(), 1);
    assert!(result.error.is_none());
    assert!(result
        .events
        .iter()
        .any(|event| matches!(event, CoreEvent::AgentStarted { .. })));

    let stopped_agent = factory.create(contract_definition()?)?;
    let stopped =
        stopped_agent.run(AgentRunInput::new(passthrough_graph()?).with_stop_requested(true))?;
    assert_eq!(stopped.status, AgentRunStatus::Cancelled);

    agent.cancel();
    assert!(token.is_cancelled());
    Ok(())
}

#[test]
fn tool_public_api_contract() -> AgentCoreResult<()> {
    validate_tool_name("contract_read")?;
    let schema = ToolSchema::new(
        "contract_read",
        "Read a stable value.",
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {"query": {"type": "string"}},
            "additionalProperties": false
        }),
    )?
    .with_output_schema(json!({"type": "object"}))
    .with_annotation("strict", json!(true));
    schema.validate_metadata()?;
    schema.validate_arguments(&json!({"query": "state"}))?;
    assert_eq!(schema.to_function_tool_schema()["name"], "contract_read");

    let empty_schema = ToolSchema::empty_object("empty_tool", "No arguments")?;
    empty_schema.validate_arguments(&json!({}))?;

    let mut metadata = ToolMetadata::new(empty_schema, ToolVisibility::Direct);
    assert_eq!(metadata.name(), "empty_tool");
    assert!(!metadata.can_preexecute());
    metadata.capabilities = ToolCapabilities {
        read_only: true,
        idempotent: true,
        destructive: false,
        requires_network: false,
        supports_streaming: false,
    };
    metadata.execution = ToolExecutionMetadata {
        timeout_ms: Some(2_000),
        interruptible: true,
        result_policy: ToolResultPolicy::ReturnSummary,
    };
    assert!(metadata.can_preexecute());

    let invocation = ToolInvocation::new("manual-call", "contract_read", json!({"query": "x"}));
    assert_eq!(invocation.tool_name, "contract_read");
    assert_eq!(ToolOutput::new(json!({"ok": true})).output["ok"], true);

    let definition = contract_definition()?;
    let mut registry = ToolRegistry::new();
    registry.register(ContractTool::new(
        "contract_read",
        ToolVisibility::Direct,
        true,
    ))?;
    registry.register(ContractTool::new(
        "contract_search",
        ToolVisibility::Searchable,
        false,
    ))?;
    registry.register(ContractTool::new(
        "contract_hidden",
        ToolVisibility::Hidden,
        false,
    ))?;

    assert!(registry.names().contains(&"contract_read"));
    assert!(registry.get("contract_read").is_some());
    assert_eq!(registry.direct_schemas(&definition).len(), 1);
    assert_eq!(
        registry.search(&definition, "search", 5)[0].name,
        "contract_search"
    );
    assert_eq!(
        registry.visibility_for_agent(
            &definition,
            registry.get("contract_hidden").unwrap().metadata()
        ),
        ToolVisibility::Hidden
    );
    assert!(registry
        .get_for_agent(&definition, "contract_hidden")
        .is_err());

    let executor = ToolExecutor::new(Arc::new(registry.clone()));
    let result = executor.execute_one(
        &definition,
        ToolCall::new("call-1", "contract_read", json!({"query": "state"})),
    )?;
    assert_eq!(result.status, ToolResultStatus::Success);
    assert!(!result.is_error());
    assert_eq!(result.output_for_model()["tool"], "contract_read");
    assert_eq!(
        result.to_content_block_value()["tool_name"],
        "contract_read"
    );
    assert_eq!(result.to_run_message_value()["role"], "tool");
    assert_eq!(result.into_run_message()?.role, MessageRole::Tool);

    let batch = executor.execute_batch_parallel(
        &definition,
        vec![
            ToolCall::new("call-2", "contract_read", json!({"query": "a"})),
            ToolCall::new("call-3", "contract_read", json!({"query": "b"})),
        ],
    )?;
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].call_id, "call-2");

    let denied = ToolResult::denied("call-4", "contract_read", "not allowed");
    let failed = ToolResult::failed("call-5", "contract_read", "boom", true);
    assert!(denied.is_error());
    assert!(failed.is_error());
    assert_eq!(
        ToolResultContent::Text("text".to_string()).to_value(),
        Value::String("text".to_string())
    );

    let mut adapters = ToolAdapterRegistry::new();
    adapters.register(ContractAdapter)?;
    assert_eq!(adapters.load_all()?.len(), 1);

    let permission_context = ToolPermissionContext::new(
        "contract-agent",
        "call-6",
        "contract_read",
        json!({"query": "state"}),
    );
    assert_eq!(
        ToolPermissionDecision::Passthrough.allowed_arguments(permission_context.arguments.clone()),
        Some(json!({"query": "state"}))
    );
    assert_eq!(
        AllowAllToolPermissionPolicy.decide(&permission_context)?,
        ToolPermissionDecision::Allow { arguments: None }
    );
    assert_eq!(
        ToolPermissionDecision::Deny {
            reason: "blocked".to_string()
        }
        .allowed_arguments(json!({})),
        None
    );

    Ok(())
}

#[test]
fn graph_public_api_contract() -> AgentCoreResult<()> {
    let source_message = RunMessage::assistant(vec![ContentBlock::text("go")])?;
    let source = GraphNode::new("source")
        .with_label("Source")
        .with_action(GraphNodeAction::EmitMessages(vec![source_message.clone()]));
    assert_eq!(source.id(), "source");
    assert_eq!(source.label(), Some("Source"));
    assert!(matches!(source.action(), GraphNodeAction::EmitMessages(_)));
    assert!(!source.is_terminal());

    let node_result = GraphNode::new("manual").execute(NodeExecutionInput::default(), |_| {
        unreachable!("noop node must not emit messages")
    })?;
    assert_eq!(node_result.node_id, "manual");

    let edge = GraphEdge::new("source_to_target", "source", "target")
        .with_inherit_policy(ContextInheritPolicy::Full)
        .with_activation_condition(ActivationCondition::MessageHasText)
        .with_priority(10);
    assert_eq!(edge.id(), "source_to_target");
    assert_eq!(edge.source_node_id(), "source");
    assert_eq!(edge.target_node_id(), "target");
    assert!(matches!(edge.inherit_policy(), ContextInheritPolicy::Full));
    assert!(matches!(
        edge.activation_condition(),
        ActivationCondition::MessageHasText
    ));
    assert_eq!(edge.priority(), 10);

    let mut state = GraphState::new(Uuid::new_v4(), GraphStateBudget::unlimited());
    let record = state.append_message("source", source_message.clone());
    state.record_node_execution("source")?;
    assert!(matches!(
        edge.evaluate(&source_message, record.version, &state.view())?,
        EdgeDecision::Activate { .. }
    ));

    let conditions = vec![
        ActivationCondition::OnAnyMessage,
        ActivationCondition::Always,
        ActivationCondition::Never,
        ActivationCondition::MessageRoleIs {
            role: MessageRole::Assistant,
        },
        ActivationCondition::MessageHasText,
        ActivationCondition::MessageHasToolCall,
        ActivationCondition::MessageHasToolResult { is_error: None },
        ActivationCondition::SourceVersionAtLeast { version: 1 },
        ActivationCondition::SourceMessageCountAtLeast { count: 1 },
    ];
    for condition in conditions {
        let _ = condition.matches("source", &source_message, record.version, &state.view());
    }

    let graph = Graph::builder("graph_contract")
        .node(source)
        .node(GraphNode::new("target").with_action(GraphNodeAction::PassthroughInput))
        .start_node("source")
        .end_node("target")
        .edge(edge)
        .budget(GraphStateBudget {
            max_total_node_executions: Some(10),
            max_node_executions: Some(5),
            max_no_progress_ticks: Some(2),
        })
        .build()?;
    assert_eq!(graph.name(), "graph_contract");
    assert!(graph.node("source").is_some());
    assert_eq!(graph.nodes().len(), 2);
    assert_eq!(graph.edges().len(), 1);
    assert_eq!(graph.start_node_ids(), &["source".to_string()]);
    assert!(graph.is_end_node("target"));
    assert_eq!(graph.outgoing_edges("source").count(), 1);

    let run_id = Uuid::new_v4();
    let result = GraphRunner::new().run(
        &graph,
        GraphRunInput {
            run_id: Some(run_id),
            initial_messages: vec![RunMessage::user(vec![ContentBlock::text("input")])?],
            stop_requested: false,
        },
    )?;
    assert_eq!(result.run_id, run_id);
    assert_eq!(result.status, GraphRunStatus::Completed);
    assert_eq!(GraphRunStatus::Completed.as_str(), "completed");
    assert!(result.error.is_none());
    assert!(result.state.total_node_executions() >= 1);
    assert!(!result.messages.is_empty());
    assert!(result
        .events
        .iter()
        .any(|event| matches!(event, CoreEvent::GraphStarted { .. })));

    let mut direct_state = GraphState::new(Uuid::new_v4(), GraphStateBudget::default());
    let fired = FiredEdgeRecord::new("e", "source", 1, "target");
    assert!(direct_state.mark_edge_fired(fired.clone()));
    assert!(direct_state.has_edge_fired(&fired));
    assert_eq!(direct_state.fired_edges().len(), 1);
    assert_eq!(direct_state.view().run_id(), direct_state.run_id());
    direct_state.request_stop();
    assert!(direct_state.is_stop_requested());

    assert_eq!(default_react_graph()?.name(), "default_react");
    assert_eq!(
        single_node_graph("single", "only")?.start_node_ids()[0],
        "only"
    );
    Ok(())
}

#[test]
fn hook_public_api_contract() -> AgentCoreResult<()> {
    assert_eq!(HookName::Input.kind(), HookKind::Point);
    assert_eq!(HookName::NodeExecution.kind(), HookKind::Wrapper);

    let payload = HookPayload::new(HookName::Input)
        .with_data(json!({"step": 1}))
        .with_metadata("source", json!("contract"));
    assert_eq!(payload.data["step"], 1);

    let mut registry = HandlerRegistry::new();
    let point_registration = registry.register_point_scoped(
        HookName::Input,
        0,
        HandlerScope::Run,
        |payload: HookPayload| {
            Ok(PointHookDecision::Rewrite(
                payload.with_metadata("rewritten", json!(true)),
            ))
        },
    )?;
    assert_eq!(point_registration.kind, HandlerKind::Point);
    assert_eq!(point_registration.scope, HandlerScope::Run);
    registry.register_point(HookName::Input, 10, |_payload| {
        Ok(PointHookDecision::Emit {
            event: HookEventRequest::new("contract.point", json!({"ok": true})),
        })
    })?;
    let outcome = registry.run_point(payload)?;
    assert_eq!(outcome.status, PointHookStatus::Continued);
    assert_eq!(outcome.events.len(), 1);
    assert_eq!(outcome.payload.metadata["rewritten"], true);

    let blocked = HandlerRegistry::new().run_point(HookPayload::new(HookName::ToolCall))?;
    assert_eq!(blocked.status, PointHookStatus::Continued);
    let _decisions = [
        PointHookDecision::Continue,
        PointHookDecision::Block {
            reason: "block".to_string(),
        },
        PointHookDecision::Stop {
            reason: "stop".to_string(),
        },
    ];

    let request = WrapperRequest::new(HookName::NodeExecution)
        .with_data(json!({"node": "start"}))
        .with_metadata("before", json!(true));
    let response = WrapperResponse::new(json!({"ok": true})).with_metadata("after", json!(true));
    assert_eq!(response.metadata["after"], true);

    let next = WrapperNext::new(Arc::new(|request: WrapperRequest| {
        Ok(WrapperResult::Continue(WrapperResponse::new(json!({
            "data": request.data,
            "metadata": request.metadata,
        }))))
    }));
    assert!(matches!(
        next.run(request.clone())?,
        WrapperResult::Continue(_)
    ));

    let wrapper_registration = registry.register_wrapper(
        HookName::NodeExecution,
        0,
        |request: WrapperRequest, next| {
            let request = request.with_metadata("layer", json!(true));
            next.run(request)
        },
    )?;
    assert_eq!(wrapper_registration.kind, HandlerKind::Wrapper);
    let wrapped = registry.run_wrapper(request, |request| {
        Ok(WrapperResult::Continue(WrapperResponse::new(json!(
            request.metadata
        ))))
    })?;
    let WrapperResult::Continue(wrapped_response) = wrapped else {
        panic!("expected continue wrapper result");
    };
    assert_eq!(wrapped_response.data["layer"], true);

    let _wrapper_results = [
        WrapperResult::Rewrite(response.clone()),
        WrapperResult::Retry {
            reason: "retry".to_string(),
        },
        WrapperResult::Recover {
            messages: vec![RunMessage::assistant(vec![ContentBlock::text("recover")])?],
        },
        WrapperResult::Stop {
            reason: "stop".to_string(),
        },
        WrapperResult::Fail {
            reason: "fail".to_string(),
        },
    ];
    Ok(())
}

#[test]
fn message_input_event_and_context_public_api_contract() -> AgentCoreResult<()> {
    let text_message = user_input::text_message("hello")?;
    let content_message = user_input::content_blocks_message(vec![ContentBlock::text("blocks")])?;
    let file_message = user_input::file_reference_message(
        "file:///tmp/a.txt",
        Some("text/plain".to_string()),
        Some("a.txt".to_string()),
    )?;
    let image_message = user_input::image_reference_message(
        "screen://current",
        Some("image/png".to_string()),
        Some("current screen".to_string()),
    )?;
    let audio_message =
        user_input::audio_reference_message("file:///tmp/a.wav", Some("audio/wav".to_string()))?;
    assert_eq!(text_message.role, MessageRole::User);
    assert_eq!(content_message.content.len(), 1);
    assert_eq!(file_message.content.len(), 1);
    assert_eq!(image_message.content.len(), 1);
    assert_eq!(audio_message.content.len(), 1);

    let mut text_block = ContentBlock::text("hel");
    assert!(text_block.append_text("lo"));
    assert_eq!(text_block, ContentBlock::text("hello"));
    assert!(ContentBlock::text("").is_empty_text_like());
    let blocks = vec![
        ContentBlock::reasoning("because"),
        ContentBlock::tool_call("call-1", "contract_read", json!({})),
        ContentBlock::tool_result(
            "call-1",
            Some("contract_read".to_string()),
            json!({}),
            false,
        ),
        ContentBlock::file_reference("file:///tmp/a.txt", None, None),
        ContentBlock::image_reference("screen://current", None, None),
        ContentBlock::audio_reference("file:///tmp/a.wav", None),
        ContentBlock::diagnostic(DiagnosticLevel::Info, "info"),
        ContentBlock::custom(json!({"kind": "custom"})),
    ];
    assert_eq!(blocks.len(), 8);
    let _diagnostic_levels = [
        DiagnosticLevel::Debug,
        DiagnosticLevel::Info,
        DiagnosticLevel::Warning,
        DiagnosticLevel::Error,
    ];

    let mut streaming = RunMessage::streaming(MessageRole::Assistant);
    streaming.push_content(ContentBlock::text("done"));
    streaming.finalize()?;
    assert_eq!(streaming.status, MessageStatus::Finalized);
    streaming.abort();
    assert_eq!(streaming.status, MessageStatus::Aborted);

    let mut sourced = RunMessage::assistant(vec![ContentBlock::text("answer")])?
        .with_source_node_id("node-a")
        .with_provider_response_id("resp-1")
        .with_usage(MessageUsage {
            input_tokens: 1,
            output_tokens: 2,
            total_tokens: 3,
        })
        .with_metadata("k", json!("v"));
    sourced.set_source_node_id_if_empty("node-b");
    assert_eq!(sourced.source_node_id.as_deref(), Some("node-a"));
    assert_eq!(sourced.provider_response_id.as_deref(), Some("resp-1"));
    assert_eq!(sourced.usage.unwrap().total_tokens, 3);
    assert_eq!(sourced.metadata["k"], "v");
    assert_eq!(
        RunMessage::diagnostic(vec![ContentBlock::diagnostic(
            DiagnosticLevel::Debug,
            "debug"
        )])?
        .role,
        MessageRole::Diagnostic
    );

    let mut assistant = AssistantBuilder::new();
    assistant.push_reasoning_delta("think");
    assistant.push_text_delta("answer");
    assistant.push_tool_call("call-2", "contract_read", json!({}));
    assistant.push_diagnostic(DiagnosticLevel::Info, "trace");
    assert_eq!(assistant.snapshot().status, MessageStatus::Streaming);
    let assistant_message = assistant.finish()?;
    assert_eq!(assistant_message.role, MessageRole::Assistant);

    let mut log = EventLog::new();
    let run_id = Uuid::new_v4();
    log.push(CoreEvent::HookEmitted {
        name: "contract".to_string(),
        data: json!({"ok": true}),
    });
    log.extend([CoreEvent::Error {
        run_id: Some(run_id),
        message: "recoverable".to_string(),
        recoverable: true,
    }]);
    assert_eq!(log.events().len(), 2);
    assert_eq!(log.into_events().len(), 2);

    let definition = contract_definition()?;
    let mut context_input = ContextBuildInput::new(ModelId::new("contract-model"));
    context_input.api = Some("mock_api".to_string());
    context_input.instructions_override = Some("override".to_string());
    context_input.replay_messages = vec![text_message.clone()];
    context_input.run_messages = vec![assistant_message.clone()];
    context_input.visible_tool_schemas =
        vec![ToolSchema::empty_object("contract_read", "Read state")?];
    context_input.options = LlmRequestOptions {
        temperature: Some(0.1),
        max_output_tokens: Some(64),
        parallel_tool_calls: Some(true),
        store: true,
        previous_response_id: Some("prev".to_string()),
    };
    context_input.include_diagnostics = true;
    context_input
        .metadata
        .insert("m".to_string(), json!("contract"));

    let llm_request = ContextBuilder::new().build(&definition, context_input)?;
    assert_eq!(llm_request.model.as_str(), "contract-model");
    assert_eq!(llm_request.api.as_deref(), Some("mock_api"));
    assert_eq!(llm_request.instructions.as_deref(), Some("override"));
    assert_eq!(llm_request.tools.len(), 1);
    assert!(llm_request.input.len() >= 2);

    assert!(!run_message_to_input_items(&assistant_message, true)?.is_empty());
    assert_eq!(
        message_value_to_input_items(
            &json!({
                "role": "tool",
                "content": [{
                    "type": "tool_result",
                    "call_id": "call-1",
                    "tool_name": "contract_read",
                    "output": {"ok": true}
                }]
            }),
            false,
        )?[0],
        LlmInputItem::FunctionCallOutput {
            call_id: "call-1".to_string(),
            output: json!({"ok": true}),
            is_error: false,
        }
    );

    let mut direct_request = LlmRequest::new("contract-model");
    direct_request.instructions = Some("instructions".to_string());
    direct_request.input.push(LlmInputItem::Message {
        role: LlmMessageRole::User,
        content: vec![
            LlmContentPart::Text {
                text: "hello".to_string(),
            },
            LlmContentPart::Reasoning {
                text: "reason".to_string(),
            },
            LlmContentPart::Json { value: json!({}) },
            LlmContentPart::ImageRef {
                uri: "screen://current".to_string(),
            },
            LlmContentPart::FileRef {
                uri: "file:///tmp/a.txt".to_string(),
            },
            LlmContentPart::Diagnostic {
                message: "diag".to_string(),
            },
            LlmContentPart::Custom {
                value: json!({"x": true}),
            },
        ],
        metadata: BTreeMap::new(),
    });
    direct_request.input.push(LlmInputItem::FunctionCall {
        call_id: "call-1".to_string(),
        name: "contract_read".to_string(),
        arguments: json!({}),
    });
    direct_request.input.push(LlmInputItem::Custom {
        value: json!({"type": "custom"}),
    });
    assert_eq!(direct_request.model, ModelId::from("contract-model"));

    Ok(())
}

#[test]
fn llm_provider_and_registry_public_api_contract() -> AgentCoreResult<()> {
    let mut registry = LlmRegistry::new();
    registry.register_provider(MockProvider)?;
    registry.register_model(LlmModel::new(
        "contract-model",
        "mock",
        LlmApi::Custom("mock_api".to_string()),
    ))?;
    let request = LlmRequest::new("contract-model");
    assert_eq!(
        registry
            .model(&ModelId::from("contract-model"))
            .unwrap()
            .provider,
        "mock"
    );
    assert_eq!(
        registry.provider_for_request(&request)?.provider_id(),
        "mock"
    );
    assert_eq!(registry.provider_by_api("mock_api")?.api(), "mock_api");

    let openai_model = OpenAiResponsesProvider::default_model("gpt-contract");
    assert_eq!(openai_model.api.as_key(), "openai_responses");
    let mut openai_request = LlmRequest::new("gpt-contract");
    openai_request.input.push(LlmInputItem::Message {
        role: LlmMessageRole::User,
        content: vec![LlmContentPart::Text {
            text: "hello".to_string(),
        }],
        metadata: BTreeMap::new(),
    });
    let openai = OpenAiResponsesProvider::new().with_endpoint("http://127.0.0.1/responses");
    let prepared_openai = openai.prepare_request(&openai_request)?;
    assert_eq!(prepared_openai.endpoint, "http://127.0.0.1/responses");
    assert_eq!(
        OpenAiResponsesProvider::stream_event_from_value(&json!({
            "type": "response.output_text.delta",
            "delta": "hi"
        }))?,
        Some(LlmStreamEvent::TextDelta {
            text: "hi".to_string()
        })
    );

    let deepseek_model = DeepSeekChatProvider::default_model("deepseek-contract");
    assert_eq!(deepseek_model.api.as_key(), "deepseek_chat");
    let deepseek = DeepSeekChatProvider::new()
        .with_endpoint("http://127.0.0.1/chat")
        .with_api_key("fixture-key");
    let mut chat_request = LlmRequest::new("deepseek-contract");
    chat_request.input.push(LlmInputItem::Message {
        role: LlmMessageRole::User,
        content: vec![LlmContentPart::Text {
            text: "hello".to_string(),
        }],
        metadata: BTreeMap::new(),
    });
    let prepared_chat = deepseek.prepare_request(&chat_request)?;
    assert_eq!(
        prepared_chat.headers.get("Authorization"),
        Some(&"Bearer fixture-key".to_string())
    );
    assert!(DeepSeekChatProvider::response_events_from_value(&json!({
        "id": "chatcmpl-1",
        "choices": [{"message": {"content": "done"}}],
        "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
    }))?
    .iter()
    .any(|event| matches!(event, LlmStreamEvent::Completed { .. })));

    let mut stream = stream_from_events(vec![
        LlmStreamEvent::PreparedRequest {
            provider: "mock".to_string(),
            body: json!({}),
        },
        LlmStreamEvent::Usage {
            usage: LlmUsage {
                input_tokens: 1,
                output_tokens: 2,
                total_tokens: 3,
            },
        },
    ]);
    assert!(matches!(
        stream.next().unwrap()?,
        LlmStreamEvent::PreparedRequest { .. }
    ));

    let provider_service = registry.provider_by_api("mock_api")?;
    assert_eq!(
        provider_service
            .prepare_request(&LlmRequest::new("contract-model"))?
            .api,
        "mock_api"
    );
    let mut provider_stream = MockProvider.call(LlmRequest::new("contract-model"))?;
    assert!(matches!(
        provider_stream.next().unwrap()?,
        LlmStreamEvent::PreparedRequest { .. }
    ));

    Ok(())
}

#[test]
fn session_turn_and_error_public_api_contract() -> AgentCoreResult<()> {
    let mut turn_loop = TurnLoop::new();
    assert_eq!(turn_loop.state(), TurnLoopState::Idle);
    let turn_message = user_input::text_message("run turn")?;
    turn_loop.submit_user_message(turn_message.clone())?;
    assert_eq!(turn_loop.buffer_len(), 1);
    let turn = turn_loop.prepare_turn()?.expect("turn should be ready");
    assert_eq!(turn.user_message, turn_message);
    assert_eq!(turn_loop.active_turn_id(), Some(turn.turn_id));
    assert_eq!(turn_loop.state(), TurnLoopState::Running);
    turn_loop.finish_turn(turn.turn_id)?;
    assert_eq!(turn_loop.state(), TurnLoopState::Idle);
    turn_loop.request_stop();
    assert!(turn_loop.stop_requested());
    turn_loop.clear_stop();
    assert!(!turn_loop.stop_requested());
    turn_loop.submit_user_message(user_input::text_message("abort turn")?)?;
    let abort_turn = turn_loop.prepare_turn()?.unwrap();
    turn_loop.abort_turn(abort_turn.turn_id)?;
    assert_eq!(turn_loop.state(), TurnLoopState::Stopped);

    let session_id = Uuid::new_v4();
    let header = SessionEntry::header(session_id);
    let message = RunMessage::user(vec![ContentBlock::text("session message")])?;
    let message_entry = SessionEntry::message(Some(header.id), message.clone())?;
    let compaction_entry = SessionEntry::compaction(Some(message_entry.id), "summary", None);
    assert!(matches!(header.kind, SessionEntryKind::Header { .. }));
    assert_eq!(message_entry.as_message(), Some(&message));
    assert!(matches!(
        compaction_entry.kind,
        SessionEntryKind::Compaction { .. }
    ));

    let mut store = InMemorySessionStore::new();
    assert!(store.is_empty());
    store.append(header.clone())?;
    store.append(message_entry.clone())?;
    assert_eq!(store.len(), 2);
    assert_eq!(store.entries().len(), 2);
    let mut tree = store.load_tree()?;
    assert_eq!(tree.roots(), &[header.id]);
    assert_eq!(tree.children_of(header.id), &[message_entry.id]);
    assert_eq!(tree.active_leaf(), Some(message_entry.id));
    assert_eq!(tree.active_branch()?.len(), 2);
    tree.set_active_leaf(header.id)?;
    assert_eq!(tree.active_leaf(), Some(header.id));
    assert_eq!(tree.get(header.id).unwrap().id, header.id);
    assert_eq!(tree.len(), 2);
    assert!(!tree.is_empty());

    let snapshot = replay_branch(&store.load_tree()?, message_entry.id)?;
    assert!(!snapshot.is_empty());
    assert_eq!(snapshot.leaf_id, Some(message_entry.id));
    assert_eq!(snapshot.messages, vec![message.clone()]);
    let empty_snapshot = replay_active_branch(&SessionTree::new())?;
    assert!(empty_snapshot.is_empty());
    let explicit_snapshot = ReplaySnapshot {
        leaf_id: None,
        entries: Vec::new(),
        messages: Vec::new(),
        summaries: Vec::new(),
    };
    assert!(explicit_snapshot.is_empty());

    let plan = CompactionPlan::new(Some(message_entry.id), "compact summary", None)?;
    assert_eq!(plan.summary, "compact summary");
    let active_plan = plan_active_branch_compaction(&store.load_tree()?, "active summary", None)?;
    assert_eq!(active_plan.parent_id, Some(message_entry.id));
    assert!(matches!(
        active_plan.into_entry().kind,
        SessionEntryKind::Compaction { .. }
    ));

    #[derive(Default)]
    struct VecSessionStore {
        entries: Vec<SessionEntry>,
    }

    impl SessionStore for VecSessionStore {
        fn append(&mut self, entry: SessionEntry) -> AgentCoreResult<()> {
            self.entries.push(entry);
            Ok(())
        }

        fn entries(&self) -> &[SessionEntry] {
            &self.entries
        }
    }

    let mut custom_store = VecSessionStore::default();
    custom_store.append(header)?;
    assert_eq!(custom_store.load_tree()?.len(), 1);

    let result_alias: AgentCoreResult<()> = Ok(());
    assert!(result_alias.is_ok());
    let errors = vec![
        AgentCoreError::InvalidInput("input".to_string()),
        AgentCoreError::InvalidConfig("config".to_string()),
        AgentCoreError::NotFound("missing".to_string()),
        AgentCoreError::PermissionDenied("denied".to_string()),
        AgentCoreError::Recoverable("retry".to_string()),
        AgentCoreError::Fatal("fatal".to_string()),
    ];
    assert_eq!(errors.len(), 6);
    let serialization_error: AgentCoreError =
        serde_json::from_str::<Value>("{").unwrap_err().into();
    assert!(matches!(
        serialization_error,
        AgentCoreError::Serialization(_)
    ));

    Ok(())
}

use agent_core::agent::{AgentRunInput, AgentRunStatus};
use agent_core::agent_definition::{AgentDefinitionBuilder, ToolVisibility};
use agent_core::assistant_builder::AssistantBuilder;
use agent_core::content_block::{ContentBlock, DiagnosticLevel};
use agent_core::context::{message_value_to_run_message, ContextBuildInput, ContextBuilder};
use agent_core::event::{CoreEvent, EventLog};
use agent_core::graph::Graph;
use agent_core::graph_edge::{GraphEdge, PackageRef};
use agent_core::graph_node::{
    Cardinality, GraphNode, InputPackageSpec, MessageQuery, NodeConcurrency, NodeKind,
};
use agent_core::graph_runner::{GraphRunInput, GraphRunStatus, GraphRunner};
use agent_core::graph_runtime as graph_rt;
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
use agent_core::llm_request::{LlmRequest, LlmRequestOptions};
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
use agent_core::turn_loop::{
    TurnContextPolicy, TurnEventHandlerFn, TurnGenInputFn, TurnGenInputResult, TurnLoop,
    TurnLoopState, TurnPrepareGraphFn,
};
use agent_core::user_input;
use agent_core::{AgentCoreError, AgentCoreResult, AgentDefinition, AgentFactory, AgentServices};
use futures::executor::block_on;
use futures::future::BoxFuture;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
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

#[derive(Debug)]
struct ContractRuntimeExecutor;

impl graph_rt::NodeExecutor for ContractRuntimeExecutor {
    fn execute(
        &self,
        node: graph_rt::NodeSpec,
        _input: graph_rt::NodeInput,
        _ctx: graph_rt::NodeExecutionContext,
    ) -> BoxFuture<'static, AgentCoreResult<graph_rt::NodeResult>> {
        Box::pin(async move {
            let output = match node.kind {
                graph_rt::NodeKind::Final => graph_rt::NodeResult::new(),
                _ => {
                    let message = RunMessage::assistant(vec![ContentBlock::text("runtime")])?
                        .with_metadata("kind", json!(node.id.clone()));
                    graph_rt::NodeResult::new().with_message(message)
                }
            };
            Ok(output)
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
    let result_message = executor.execute_one_message(
        &definition,
        ToolCall::new("call-1m", "contract_read", json!({"query": "state"})),
    )?;
    assert_eq!(result_message.role, MessageRole::Tool);

    let batch = executor.execute_batch_parallel(
        &definition,
        vec![
            ToolCall::new("call-2", "contract_read", json!({"query": "a"})),
            ToolCall::new("call-3", "contract_read", json!({"query": "b"})),
        ],
    )?;
    assert_eq!(batch.len(), 2);
    assert_eq!(batch[0].call_id, "call-2");
    let batch_messages = executor.execute_batch_parallel_messages(
        &definition,
        vec![
            ToolCall::new("call-2m", "contract_read", json!({"query": "a"})),
            ToolCall::new("call-3m", "contract_read", json!({"query": "b"})),
        ],
    )?;
    assert_eq!(batch_messages.len(), 2);
    assert!(batch_messages
        .iter()
        .all(|message| message.role == MessageRole::Tool));

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
    let instruction_query =
        MessageQuery::where_eq("metadata.kind", "instruction").select(["content[*].text"]);
    let observe = GraphNode::new(
        "observe_screen",
        NodeKind::Transform {
            executor: "observe_screen".to_string(),
            config: json!({"read_only": true}),
        },
        InputPackageSpec::new("task").required(
            "instruction",
            instruction_query.clone(),
            Cardinality::Latest,
        ),
    )
    .concurrency(NodeConcurrency::Parallel { max: 2 });
    assert_eq!(observe.id(), "observe_screen");
    assert!(matches!(
        observe.concurrency,
        NodeConcurrency::Parallel { max: 2 }
    ));

    let lookup = GraphNode::new(
        "lookup_catalog",
        NodeKind::Transform {
            executor: "lookup_catalog".to_string(),
            config: json!({"cacheable": true}),
        },
        InputPackageSpec::new("task").required(
            "instruction",
            instruction_query.clone(),
            Cardinality::Latest,
        ),
    );
    let final_node = GraphNode::final_node(
        "final",
        InputPackageSpec::new("decision")
            .required(
                "screen",
                MessageQuery::where_eq("metadata.kind", "observe_screen"),
                Cardinality::Latest,
            )
            .required(
                "catalog",
                MessageQuery::where_eq("metadata.kind", "lookup_catalog"),
                Cardinality::Latest,
            )
            .optional(
                "diagnostic",
                MessageQuery::where_eq("role", "diagnostic"),
                Cardinality::Latest,
            ),
    );

    let edge = GraphEdge::new("observe_to_final", "observe_screen", ("final", "decision"));
    assert_eq!(edge.id(), "observe_to_final");
    assert_eq!(edge.from(), "observe_screen");
    assert_eq!(edge.to(), &PackageRef::new("final", "decision"));

    let graph = Graph::builder("graph_contract")
        .node(observe)
        .node(lookup)
        .node(final_node)
        .edge("input_to_observe", "input", ("observe_screen", "task"))
        .edge("input_to_lookup", "input", ("lookup_catalog", "task"))
        .edge("lookup_to_final", "lookup_catalog", ("final", "decision"))
        .edge("observe_to_final", "observe_screen", ("final", "decision"))
        .finish_at("final")
        .build()?;
    assert_eq!(graph.name(), "graph_contract");
    assert!(graph.node("observe_screen").is_some());
    assert_eq!(graph.nodes().len(), 3);
    assert_eq!(graph.edges().len(), 4);
    assert_eq!(graph.input(), "input");
    assert_eq!(graph.finish_node(), Some("final"));

    let run_id = Uuid::new_v4();
    let initial = RunMessage::user(vec![ContentBlock::text("compare screen and catalog")])?
        .with_metadata("kind", json!("instruction"));
    let result = GraphRunner::with_executor(Arc::new(ContractRuntimeExecutor)).run(
        &graph,
        GraphRunInput::new(vec![initial]).with_run_id(run_id),
    )?;
    assert_eq!(result.run_id, run_id);
    assert_eq!(result.status, GraphRunStatus::Completed);
    assert_eq!(GraphRunStatus::Completed.as_str(), "completed");
    assert!(result.error.is_none());
    assert_eq!(result.ledger.node_attempts.len(), 3);
    assert_eq!(result.ledger.transfers.len(), 4);
    let final_package = result
        .state
        .package_states
        .get(&PackageRef::new("final", "decision"))
        .unwrap();
    assert!(final_package.items.contains_key("screen"));
    assert!(final_package.items.contains_key("catalog"));
    assert!(!result.messages.is_empty());
    assert!(result
        .events
        .iter()
        .any(|event| matches!(event, CoreEvent::GraphStarted { .. })));

    let loop_graph = Graph::builder("bounded_loop")
        .node(GraphNode::new(
            "loop",
            NodeKind::Transform {
                executor: "loop".to_string(),
                config: json!({}),
            },
            InputPackageSpec::new("input").required(
                "turn",
                MessageQuery::any(),
                Cardinality::Latest,
            ),
        ))
        .edge("input_to_loop", "input", ("loop", "input"))
        .edge("loop_to_loop", "loop", ("loop", "input"))
        .build()?;
    let no_progress = GraphRunner::with_executor(Arc::new(ContractRuntimeExecutor)).run(
        &loop_graph,
        GraphRunInput::new(vec![RunMessage::user(vec![ContentBlock::text("loop")])?])
            .with_max_ticks(2),
    )?;
    assert_eq!(no_progress.status, GraphRunStatus::BudgetExceeded);
    assert!(no_progress
        .error
        .as_deref()
        .unwrap_or_default()
        .contains("tick budget exceeded"));

    assert_eq!(default_react_graph()?.name(), "default_react");
    assert_eq!(
        single_node_graph("single", "only")?.finish_node(),
        Some("only")
    );

    let runtime_graph = graph_rt::GraphSpec::builder("runtime_contract")
        .node(graph_rt::NodeSpec::final_node(
            "target",
            graph_rt::InputPackageSpec::new("input").required(
                "turn",
                graph_rt::MessageQuery::any(),
                graph_rt::Cardinality::Latest,
            ),
        ))
        .edge("input_to_target", "input", ("target", "input"))
        .finish_at("target")
        .build()?;
    let runtime_output = block_on(
        graph_rt::GraphRuntime::new(
            runtime_graph,
            graph_rt::GraphRuntimeServices::new(Arc::new(ContractRuntimeExecutor)),
        )
        .run(graph_rt::GraphRunInput::new(vec![RunMessage::user(vec![
            ContentBlock::text("runtime input"),
        ])?])),
    )?;
    assert_eq!(runtime_output.status, graph_rt::GraphRunStatus::Completed);
    assert_eq!(runtime_output.ledger.transfers.len(), 1);
    assert_eq!(runtime_output.ledger.node_attempts[0].node, "target");

    let package_edge = GraphEdge::new("package_edge", "agent", ("tool", "calls"));
    assert_eq!(package_edge.from, "agent");
    assert_eq!(package_edge.to.package, "calls");
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
    context_input.extend_messages([text_message.clone(), assistant_message.clone()]);
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
    assert_eq!(llm_request.messages.len(), 2);
    assert_eq!(llm_request.messages[1], assistant_message);

    let parsed_message = message_value_to_run_message(&json!({
        "id": Uuid::new_v4(),
        "role": "tool",
        "content": [{
            "type": "tool_result",
            "call_id": "call-1",
            "tool_name": "contract_read",
            "output": {"ok": true},
            "is_error": false
        }],
        "status": "finalized",
        "created_at_ms": 1
    }))?;
    assert_eq!(parsed_message.role, MessageRole::Tool);

    let mut direct_request = LlmRequest::new("contract-model");
    direct_request.instructions = Some("instructions".to_string());
    direct_request.push_message(RunMessage::user(vec![
        ContentBlock::text("hello"),
        ContentBlock::reasoning("reason"),
        ContentBlock::image_reference("screen://current", None, None),
        ContentBlock::file_reference("file:///tmp/a.txt", None, None),
        ContentBlock::diagnostic(DiagnosticLevel::Info, "diag"),
        ContentBlock::custom(json!({"x": true})),
    ])?);
    direct_request.push_message(RunMessage::assistant_tool_call(
        "call-1",
        "contract_read",
        json!({}),
    )?);
    assert_eq!(direct_request.model, ModelId::from("contract-model"));
    assert_eq!(direct_request.messages.len(), 2);

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
    openai_request.push_message(RunMessage::user_text("hello")?);
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
    chat_request.push_message(RunMessage::user_text("hello")?);
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
    let loop_graph = Graph::builder("turn_loop_contract")
        .node(GraphNode::final_node(
            "final",
            InputPackageSpec::new("input").required(
                "turn",
                MessageQuery::where_eq("role", "user"),
                Cardinality::Latest,
            ),
        ))
        .edge("input_to_final", "input", ("final", "input"))
        .finish_at("final")
        .build()?;

    let seen_events = Arc::new(Mutex::new(0usize));
    let seen_events_for_handler = Arc::clone(&seen_events);
    let on_events: TurnEventHandlerFn = Arc::new(move |batch| {
        *seen_events_for_handler.lock().unwrap() += batch.events.len();
        Ok(())
    });
    let mut turn_loop = TurnLoop::new()
        .with_graph(loop_graph.clone())
        .with_context_policy(TurnContextPolicy::LastMessages(1))
        .with_on_turn_events(on_events);
    assert_eq!(turn_loop.state(), TurnLoopState::Idle);
    turn_loop.append_messages([RunMessage::assistant_text("restored context")?])?;
    assert!(turn_loop.push(user_input::text_message("run graph")?)?);
    assert_eq!(turn_loop.pending_len(), 1);

    let loop_run = turn_loop.run_once()?.expect("turn should run");
    assert_eq!(loop_run.graph.status, GraphRunStatus::Completed);
    assert_eq!(loop_run.context_messages.len(), 2);
    assert_eq!(turn_loop.messages().len(), 2);
    assert_eq!(turn_loop.pending_len(), 0);
    assert!(*seen_events.lock().unwrap() > 0);

    let fast_graph = loop_graph.clone();
    let prepare_graph: TurnPrepareGraphFn = Arc::new(move |_turn| Ok(fast_graph.clone()));
    let gen_input: TurnGenInputFn = Arc::new(|input| {
        let first = input.pending_items.first().unwrap().clone();
        let remaining = input.pending_items.iter().skip(1).cloned().collect();
        Ok(TurnGenInputResult::new(vec![first.clone()], vec![first.id]).with_remaining(remaining))
    });
    let mut queued_loop = TurnLoop::new()
        .with_prepare_graph(prepare_graph)
        .with_gen_input(gen_input);
    queued_loop.push(user_input::text_message("first queued")?)?;
    queued_loop.push(user_input::text_message("second queued")?)?;
    let queued_turns = queued_loop.run_pending()?;
    assert_eq!(queued_turns.len(), 2);
    assert_eq!(queued_loop.messages().len(), 2);

    turn_loop.request_stop();
    assert!(turn_loop.stop_requested());
    assert!(!turn_loop.push(user_input::text_message("late turn")?)?);
    assert_eq!(turn_loop.late_items().len(), 1);
    assert_eq!(turn_loop.take_late_items().len(), 1);
    turn_loop.clear_stop();
    assert!(!turn_loop.stop_requested());
    assert_eq!(turn_loop.state(), TurnLoopState::Idle);

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
    let errors = [
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

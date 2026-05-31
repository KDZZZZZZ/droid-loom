use crate::app_map_memory::{sample_cross_app_map, AppMapMemory, ForgetScope};
use crate::context_stability::{
    mark_stability, order_by_stability, stability_order_labels, stable_prefix_id, ContextStability,
};
use crate::cross_app_task::run_hundred_step_cross_app_task;
use crate::execution_probability::TrajectoryProbabilityGraph;
use crate::key_routing::{KeyRoutePlan, StaticSecretResolver};
use crate::mobile_tools::{expected_action_count, register_mobilerun_tools};
use crate::prompt::{default_tool_visibility, ExecutionMode, MobileRunLikeConfig};
use crate::task_context::sample_mobile_task_dag;
use agent_core::agent::AgentRunInput;
use agent_core::assistant_builder::AssistantBuilder;
use agent_core::content_block::{ContentBlock, DiagnosticLevel};
use agent_core::context::{ContextBuildInput, ContextBuilder};
use agent_core::event::{CoreEvent, EventLog};
use agent_core::graph::Graph;
use agent_core::graph_edge::{ActivationCondition, GraphEdge};
use agent_core::graph_node::{GraphNode, GraphNodeAction};
use agent_core::graph_runner::{GraphRunInput, GraphRunner};
use agent_core::graph_state::GraphStateBudget;
use agent_core::hook::{
    HookEventRequest, HookName, HookPayload, PointHookDecision, WrapperRequest, WrapperResponse,
    WrapperResult,
};
use agent_core::hook_handler::{HandlerRegistry, PointHookStatus};
use agent_core::run_message::RunMessage;
use agent_core::session_entry::SessionEntry;
use agent_core::session_replay::{replay_active_branch, ReplaySnapshot};
use agent_core::session_store::{InMemorySessionStore, SessionStore};
use agent_core::tool_executor::{ToolCall, ToolExecutor};
use agent_core::tool_registry::ToolRegistry;
use agent_core::turn_loop::TurnLoop;
use agent_core::{user_input, AgentCoreResult, AgentFactory};
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Debug)]
pub struct BoundaryReport {
    checks: Vec<CapabilityCheck>,
    gaps: Vec<&'static str>,
}

#[derive(Debug)]
struct CapabilityCheck {
    name: &'static str,
    status: &'static str,
    detail: String,
}

impl BoundaryReport {
    pub fn print(&self) {
        println!("Mobilerun-like agent boundary probe");
        println!("====================================");
        for check in &self.checks {
            println!("[{}] {} - {}", check.status, check.name, check.detail);
        }

        println!();
        println!("Known core gaps surfaced by this example:");
        for gap in &self.gaps {
            println!("- {gap}");
        }
    }
}

pub fn run_boundary_probe() -> AgentCoreResult<BoundaryReport> {
    let config = MobileRunLikeConfig::boundary_default();
    let definition = config.agent_definition()?;
    let mut reasoning_config = config.clone();
    reasoning_config.agent_name = "mobilerun_like_reasoning_agent".to_string();
    reasoning_config.mode = ExecutionMode::Reasoning;
    let reasoning_definition = reasoning_config.agent_definition()?;

    let mut registry = ToolRegistry::new();
    register_mobilerun_tools(&mut registry)?;
    let registry = Arc::new(registry);
    let registered_tool_count = registry.names().len();
    let direct_schemas = registry.direct_schemas(&definition);
    let searched_schemas = registry.search(&definition, "database", 5);
    let hidden_tool_blocked = registry
        .get_for_agent(&definition, "raw_adb_shell")
        .is_err();
    let executor = ToolExecutor::new(registry.clone());

    let user_message = user_input::content_blocks_message(vec![
        ContentBlock::text(config.render_user_prompt()),
        ContentBlock::image_reference(
            "screen://current",
            Some("image/png".to_string()),
            Some("current Android screen".to_string()),
        ),
    ])?;

    let provider_request =
        build_provider_request(&definition, &direct_schemas, vec![user_message.clone()])?;
    let turn_loop_summary = run_turn_loop_probe(user_message.clone())?;

    let assistant_message = scripted_assistant_message()?;
    let tool_calls = scripted_tool_calls();
    let tool_results = executor.execute_batch(&definition, tool_calls)?;
    let tool_messages = tool_results
        .iter()
        .map(|result| result.into_run_message())
        .collect::<AgentCoreResult<Vec<_>>>()?;
    let tool_error_count = tool_results
        .iter()
        .filter(|result| result.is_error())
        .count();
    let coverage_results = executor.execute_batch(&definition, coverage_tool_calls())?;
    let coverage_error_count = coverage_results
        .iter()
        .filter(|result| result.is_error())
        .count();

    let mut full_turn_messages = vec![user_message.clone(), assistant_message.clone()];
    full_turn_messages.extend(tool_messages.clone());
    let replay_request = build_provider_request(&definition, &direct_schemas, full_turn_messages)?;
    let hidden_execution_blocked = executor
        .execute_one(
            &definition,
            ToolCall::new(
                "call-hidden",
                "raw_adb_shell",
                json!({"command": "input keyevent 3"}),
            ),
        )
        .is_err();

    let session_snapshots = execution_session_snapshots(
        user_message.clone(),
        assistant_message.clone(),
        tool_messages.clone(),
    )?;
    let replay_message_count = session_snapshots
        .iter()
        .map(|snapshot| snapshot.messages.len())
        .sum::<usize>();
    let probability_graph = TrajectoryProbabilityGraph::from_replay_snapshots(&session_snapshots);
    let tool_layer_plan = probability_graph.plan_tool_layers(&default_tool_visibility(), 0.5);
    let optimized_definition =
        config.agent_definition_with_tool_visibility(tool_layer_plan.visibility.clone())?;
    let optimized_direct_schemas = registry.direct_schemas(&optimized_definition);
    let preexecution_plan = probability_graph.plan_preexecution(
        registry.as_ref(),
        &optimized_definition,
        vec![
            ToolCall::new("pre-ui", "ui_state", json!({})),
            ToolCall::new(
                "pre-search",
                "search_database",
                json!({"query": "wireless charger"}),
            ),
            ToolCall::new("pre-click", "click_at", json!({"x": 512, "y": 176})),
        ],
        0.5,
    );
    let preexecution_skipped = preexecution_plan.skipped.clone();
    let preexecution_outcome = TrajectoryProbabilityGraph::execute_preexecution_plan(
        &executor,
        &optimized_definition,
        preexecution_plan,
    )?;
    let parallel_results = executor.execute_batch_parallel(
        &definition,
        vec![
            ToolCall::new("parallel-ui", "ui_state", json!({})),
            ToolCall::new("parallel-shot", "screenshot", json!({})),
        ],
    )?;
    let task_graph = sample_mobile_task_dag()?;
    let mut task_context_messages = task_graph.context_for("finalize")?;
    task_context_messages.push(mark_stability(
        RunMessage::user(vec![ContentBlock::text(
            "Stable mobile automation prefix: use observed state, tool results, and task artifacts.",
        )])?,
        ContextStability::StablePrefix,
    ));
    task_context_messages.push(mark_stability(
        user_message.clone(),
        ContextStability::VolatileObservation,
    ));
    let ordered_context_messages = order_by_stability(task_context_messages);
    let stability_labels = stability_order_labels(&ordered_context_messages);
    let prefix_id = stable_prefix_id(
        optimized_definition.system_prompt(),
        &tool_layer_plan.direct_tools,
    );
    let cache_aware_request = build_provider_request(
        &optimized_definition,
        &optimized_direct_schemas,
        ordered_context_messages,
    )?;
    let key_route_plan = KeyRoutePlan::new(prefix_id.clone());
    let route_resolver = StaticSecretResolver::new([
        ("MOBILERUN_STABLE_PREFIX_API_KEY", "stable-route-fixture"),
        ("MOBILERUN_GENERAL_POOL_API_KEY", "general-route-fixture"),
    ]);
    let stable_routed =
        key_route_plan.apply(cache_aware_request.clone(), &prefix_id, &route_resolver)?;
    let general_routed =
        key_route_plan.apply(cache_aware_request, "volatile-prefix", &route_resolver)?;
    let map_memory_summary = run_app_map_probe()?;

    let handler_summary = run_handler_probe()?;
    let graph_probe = run_graph_probe(&definition, user_message, assistant_message)?;
    let reasoning_graph_summary = run_reasoning_graph_probe(&reasoning_definition)?;
    let complex_graph_summary = run_complex_graph_probe(&reasoning_definition)?;
    let hundred_step_report = run_hundred_step_cross_app_task(&executor, &definition)?;
    let event_stream_summary = project_event_stream(&graph_probe.events, coverage_results.len());
    let structured_ok = validate_structured_output(&json!({
        "success": true,
        "remembered_items": [{"key": "first_result", "value": "Wireless Charger Stand"}],
        "summary": "Demo task completed with mocked mobile actions."
    }));

    let mut checks = Vec::new();
    checks.push(check(
        "agent definition",
        "supported",
        format!(
            "name={}, reasoning_name={}, direct_tools={}, searchable_hits={}, hidden_blocked={}",
            definition.name(),
            reasoning_definition.name(),
            direct_schemas.len(),
            searched_schemas.len(),
            hidden_tool_blocked
        ),
    ));
    checks.push(check(
        "mobilerun-like tool surface",
        "simulated",
        format!(
            "registered {registered_tool_count}/{} mocked action tools",
            expected_action_count()
        ),
    ));
    checks.push(check(
        "message and content blocks",
        "supported",
        format!(
            "user request contains {} provider input item(s), including an image reference",
            provider_request.input.len()
        ),
    ));
    checks.push(check(
        "context builder",
        "supported",
        format!(
            "model={}, tools_visible_to_provider={}, replay_items={}",
            provider_request.model.as_str(),
            provider_request.tools.len(),
            replay_request.input.len()
        ),
    ));
    checks.push(check(
        "assistant tool-call turn",
        "supported",
        format!(
            "assistant emitted {} tool calls; tool_errors={tool_error_count}",
            scripted_tool_calls().len()
        ),
    ));
    checks.push(check(
        "tool execution coverage",
        if coverage_error_count == 0 {
            "supported"
        } else {
            "gap"
        },
        format!(
            "executed {}/{} visible tools; coverage_errors={coverage_error_count}",
            coverage_results.len(),
            coverage_tool_calls().len()
        ),
    ));
    checks.push(check(
        "tool visibility guard",
        if hidden_execution_blocked {
            "supported"
        } else {
            "gap"
        },
        "hidden raw_adb_shell cannot be executed by this agent".to_string(),
    ));
    checks.push(check(
        "trajectory probability graph",
        "supported",
        format!(
            "session_branches={}, replay_messages={}, likely_after_assistant={:?}, ui_state_p={:.2}",
            probability_graph.session_count(),
            replay_message_count,
            probability_graph.likely_next("message:assistant", 2),
            probability_graph.tool_use_probability("ui_state")
        ),
    ));
    checks.push(check(
        "tool hot/cold layering",
        "supported",
        format!(
            "direct_hot={}, dynamic_cold={}, hidden={}, optimized_visible_tools={}",
            tool_layer_plan.direct_tools.len(),
            tool_layer_plan.dynamic_tools.len(),
            tool_layer_plan.hidden_tools.len(),
            optimized_direct_schemas.len()
        ),
    ));
    checks.push(check(
        "tool preexecution",
        "supported",
        format!(
            "preexecuted={}, result_messages={}, skipped={:?}",
            preexecution_outcome.results.len(),
            preexecution_outcome.messages.len(),
            preexecution_skipped
        ),
    ));
    checks.push(check(
        "parallel tool execution",
        "supported",
        format!(
            "parallel_results={}, first_call={}, second_call={}",
            parallel_results.len(),
            parallel_results
                .first()
                .map(|result| result.call_id.as_str())
                .unwrap_or("none"),
            parallel_results
                .get(1)
                .map(|result| result.call_id.as_str())
                .unwrap_or("none")
        ),
    ));
    checks.push(check(
        "cache-aware context ordering",
        "supported",
        format!("ordered_stability={stability_labels:?}, prefix_id={prefix_id}"),
    ));
    checks.push(check(
        "task-bound context dag",
        "supported",
        format!(
            "dependency_edges={}, final_context_inherits_dependency_artifacts_only",
            task_graph.dependency_edges().len()
        ),
    ));
    checks.push(check(
        "key/account routing",
        "supported",
        format!(
            "stable_account={}, general_account={}, separate_headers={}",
            stable_routed.account_label,
            general_routed.account_label,
            stable_routed.request.headers.get("Authorization")
                != general_routed.request.headers.get("Authorization")
        ),
    ));
    checks.push(check("app map memory", "supported", map_memory_summary));
    checks.push(check("handler hooks", "supported", handler_summary));
    checks.push(check(
        "turn loop input queue",
        "supported",
        turn_loop_summary,
    ));
    checks.push(check("graph runner", "supported", graph_probe.detail));
    checks.push(check(
        "event stream projection",
        "supported",
        event_stream_summary,
    ));
    checks.push(check(
        "reasoning manager/executor graph",
        "supported",
        reasoning_graph_summary,
    ));
    checks.push(check(
        "complex task graph",
        "supported",
        complex_graph_summary,
    ));
    checks.push(check(
        "hundred-step cross-app task",
        "simulated",
        hundred_step_report.detail(),
    ));
    checks.push(check(
        "token optimization effects",
        "supported",
        format!(
            "tool_layer_direct={}->{}, local_map_tokens={} vs full_map_tokens={}, long_task_token_savings={:.1}%",
            default_tool_visibility()
                .values()
                .filter(|visibility| **visibility == agent_core::ToolVisibility::Direct)
                .count(),
            optimized_direct_schemas.len(),
            hundred_step_report.local_view_token_total,
            hundred_step_report.full_map_token_total,
            hundred_step_report.token_savings_percent
        ),
    ));
    checks.push(check(
        "structured final output",
        if structured_ok { "simulated" } else { "gap" },
        "example validates the expected JSON shape outside core".to_string(),
    ));

    Ok(BoundaryReport {
        checks,
        gaps: vec![
            "GraphRunner can execute built-in graph nodes, but provider/tool node execution is still an external orchestration concern.",
            "HandlerRegistry is callable through public API, but core runners do not automatically inject handlers into every hook point yet.",
            "Tool timeout metadata is declared, but this synchronous ToolExecutor does not enforce wall-clock deadlines.",
            "Structured output schema validation, credential vaults, and real mobile observation parsing remain outside core.",
        ],
    })
}

fn build_provider_request(
    definition: &agent_core::AgentDefinition,
    direct_schemas: &[agent_core::tool_schema::ToolSchema],
    messages: Vec<RunMessage>,
) -> AgentCoreResult<agent_core::llm_request::LlmRequest> {
    let mut input = ContextBuildInput::new("mimo-v2.5-pro");
    input.run_messages = messages;
    input.visible_tool_schemas = direct_schemas.to_vec();
    input
        .metadata
        .insert("openai.stream".to_string(), json!(false));
    input
        .metadata
        .insert("openai.tool_choice".to_string(), json!("auto"));
    ContextBuilder::new().build(definition, input)
}

fn run_app_map_probe() -> AgentCoreResult<String> {
    let map = sample_cross_app_map()?;
    let current_page = map
        .current_page()
        .ok_or_else(|| agent_core::AgentCoreError::Fatal("map has no current page".to_string()))?;
    let search_hits = map.semantic_search("Share", 2);
    let target_page = search_hits
        .first()
        .map(|hit| hit.page_id.as_str())
        .ok_or_else(|| agent_core::AgentCoreError::NotFound("Share page not found".to_string()))?;
    let path = map.plan_path(current_page, target_page).ok_or_else(|| {
        agent_core::AgentCoreError::NotFound("path to target page not found".to_string())
    })?;
    let local_view = map.local_view(Some(current_page), 1, Some("Share"));
    let local_tokens = AppMapMemory::estimate_context_tokens(&local_view);
    let full_tokens = map.estimate_full_map_tokens();
    let mut forget_map = map.clone();
    let removed = forget_map.forget(ForgetScope::Query {
        query: "notes".to_string(),
    });
    let mut maintenance_map = map.clone();
    let first_transition = maintenance_map.transitions().first().cloned();
    if let Some(transition) = first_transition {
        maintenance_map.mark_transition_failed(&transition.from_page, &transition.action.id);
        maintenance_map.mark_transition_failed(&transition.from_page, &transition.action.id);
        maintenance_map.mark_transition_failed(&transition.from_page, &transition.action.id);
    }
    let stale_paths = maintenance_map
        .transitions()
        .iter()
        .filter(|transition| transition.stale)
        .count();
    Ok(format!(
        "pages={}, transitions={}, search_hits={}, path_len={}, local_nodes={}, candidate_actions={}, local_tokens={}, full_tokens={}, forget_removed={}, stale_paths_after_failures={}",
        map.page_count(),
        map.transition_count(),
        search_hits.len(),
        path.len(),
        local_view.nodes.len(),
        local_view.candidate_actions.len(),
        local_tokens,
        full_tokens,
        removed,
        stale_paths
    ))
}

fn scripted_assistant_message() -> AgentCoreResult<RunMessage> {
    let mut builder = AssistantBuilder::new();
    builder.push_reasoning_delta(
        "Need observe, tap search, type query, remember first result, finish.",
    );
    for call in scripted_tool_calls() {
        builder.push_tool_call(call.call_id, call.tool_name, call.arguments);
    }
    builder.finish()
}

fn scripted_tool_calls() -> Vec<ToolCall> {
    vec![
        ToolCall::new("call-ui", "ui_state", json!({})),
        ToolCall::new("call-tap", "click_at", json!({"x": 512, "y": 176})),
        ToolCall::new(
            "call-type",
            "type",
            json!({"text": "wireless charger", "index": 0}),
        ),
        ToolCall::new(
            "call-search",
            "search_database",
            json!({"query": "wireless charger"}),
        ),
        ToolCall::new(
            "call-remember",
            "remember",
            json!({"information": "first_result=Wireless Charger Stand"}),
        ),
        ToolCall::new(
            "call-secret",
            "type_secret",
            json!({"secret_id": "demo_checkout_pin", "index": 2}),
        ),
        ToolCall::new(
            "call-complete",
            "complete",
            json!({"success": true, "reason": "first result remembered"}),
        ),
    ]
}

fn coverage_tool_calls() -> Vec<ToolCall> {
    vec![
        ToolCall::new("cover-screenshot", "screenshot", json!({})),
        ToolCall::new("cover-ui", "ui_state", json!({})),
        ToolCall::new("cover-click", "click", json!({"index": 1})),
        ToolCall::new("cover-click-at", "click_at", json!({"x": 100, "y": 200})),
        ToolCall::new(
            "cover-click-area",
            "click_area",
            json!({"x1": 10, "y1": 20, "x2": 180, "y2": 220}),
        ),
        ToolCall::new("cover-long-press", "long_press", json!({"index": 1})),
        ToolCall::new(
            "cover-long-press-at",
            "long_press_at",
            json!({"x": 300, "y": 400, "duration_ms": 800}),
        ),
        ToolCall::new(
            "cover-type",
            "type",
            json!({"text": "wireless charger", "index": 0}),
        ),
        ToolCall::new(
            "cover-type-secret",
            "type_secret",
            json!({"secret_id": "demo_password", "index": 2}),
        ),
        ToolCall::new("cover-button", "system_button", json!({"button": "back"})),
        ToolCall::new(
            "cover-swipe",
            "swipe",
            json!({"coordinate": "100,900", "coordinate2": "100,200"}),
        ),
        ToolCall::new("cover-wait", "wait", json!({"duration": 1.0})),
        ToolCall::new("cover-open-app", "open_app", json!({"text": "Demo Shop"})),
        ToolCall::new(
            "cover-remember",
            "remember",
            json!({"information": "first_result=Wireless Charger Stand"}),
        ),
        ToolCall::new(
            "cover-search-db",
            "search_database",
            json!({"query": "wireless charger"}),
        ),
        ToolCall::new(
            "cover-complete",
            "complete",
            json!({"success": true, "reason": "coverage run done"}),
        ),
    ]
}

fn run_turn_loop_probe(user_message: RunMessage) -> AgentCoreResult<String> {
    let mut turn_loop = TurnLoop::new();
    turn_loop.submit_user_message(user_message)?;
    let state_after_submit = turn_loop.state();
    let buffered_after_submit = turn_loop.buffer_len();

    let turn = turn_loop
        .prepare_turn()?
        .ok_or_else(|| agent_core::AgentCoreError::Fatal("turn was not prepared".to_string()))?;
    let state_after_prepare = turn_loop.state();
    turn_loop.finish_turn(turn.turn_id)?;

    Ok(format!(
        "after_submit={:?}, buffered={}, after_prepare={:?}, final_state={:?}",
        state_after_submit,
        buffered_after_submit,
        state_after_prepare,
        turn_loop.state()
    ))
}

fn run_handler_probe() -> AgentCoreResult<String> {
    let mut handlers = HandlerRegistry::new();
    handlers.register_point(HookName::Input, 0, |payload: HookPayload| {
        let mut next = payload;
        next.metadata.insert("normalized".to_string(), json!(true));
        Ok(PointHookDecision::Rewrite(next))
    })?;
    handlers.register_point(HookName::ToolResult, 0, |payload: HookPayload| {
        Ok(PointHookDecision::Emit {
            event: HookEventRequest::new("tool_result_seen", payload.data),
        })
    })?;
    handlers.register_wrapper(
        HookName::ToolExecution,
        0,
        |request: WrapperRequest, next| {
            let request = request.with_metadata("deadline_ms", json!(10_000));
            next.run(request)
        },
    )?;
    let recover_message = RunMessage::diagnostic(vec![ContentBlock::diagnostic(
        DiagnosticLevel::Warning,
        "provider request can recover by returning a model-visible retry message",
    )])?;
    handlers.register_wrapper(
        HookName::ProviderRequest,
        0,
        move |request: WrapperRequest, next| {
            if request.data.get("recoverable").and_then(Value::as_bool) == Some(true) {
                return Ok(WrapperResult::Recover {
                    messages: vec![recover_message.clone()],
                });
            }
            next.run(request)
        },
    )?;

    let input_outcome = handlers.run_point(
        HookPayload::new(HookName::Input).with_data(json!({"text": "run mobile task"})),
    )?;
    let result_outcome = handlers
        .run_point(HookPayload::new(HookName::ToolResult).with_data(json!({"tool": "click_at"})))?;
    let wrapper_result = handlers.run_wrapper(
        WrapperRequest::new(HookName::ToolExecution).with_data(json!({"tool": "click_at"})),
        |request| {
            Ok(WrapperResult::Continue(WrapperResponse::new(json!({
                "metadata": request.metadata
            }))))
        },
    )?;
    let recover_result = handlers.run_wrapper(
        WrapperRequest::new(HookName::ProviderRequest).with_data(json!({
            "recoverable": true,
            "error": "provider overloaded"
        })),
        |_| {
            Ok(WrapperResult::Fail {
                reason: "terminal provider failure".to_string(),
            })
        },
    )?;

    let rewritten = matches!(input_outcome.status, PointHookStatus::Continued)
        && input_outcome.payload.metadata.get("normalized") == Some(&json!(true));
    let emitted_events = result_outcome.events.len();
    let wrapped = match wrapper_result {
        WrapperResult::Continue(response) => {
            response.data["metadata"]["deadline_ms"] == json!(10_000)
        }
        _ => false,
    };
    let recovered_messages = match recover_result {
        WrapperResult::Recover { messages } => messages.len(),
        _ => 0,
    };

    Ok(format!(
        "point_rewrite={rewritten}, emitted_events={emitted_events}, wrapper_metadata={wrapped}, recover_messages={recovered_messages}"
    ))
}

#[derive(Debug)]
struct GraphProbe {
    detail: String,
    events: Vec<CoreEvent>,
}

fn run_graph_probe(
    definition: &agent_core::AgentDefinition,
    user_message: RunMessage,
    assistant_message: RunMessage,
) -> AgentCoreResult<GraphProbe> {
    let done_message = RunMessage::assistant(vec![ContentBlock::text("mobile task completed")])?;
    let graph = Graph::builder("mobilerun_fast_turn")
        .node(GraphNode::new("input").with_action(GraphNodeAction::PassthroughInput))
        .node(
            GraphNode::new("agent_turn")
                .with_action(GraphNodeAction::EmitMessages(vec![assistant_message])),
        )
        .node(
            GraphNode::new("done")
                .with_action(GraphNodeAction::EmitMessages(vec![done_message]))
                .terminal(true),
        )
        .start_node("input")
        .end_node("done")
        .edge(
            GraphEdge::new("input_to_agent", "input", "agent_turn")
                .with_activation_condition(ActivationCondition::MessageHasText),
        )
        .edge(
            GraphEdge::new("agent_to_done", "agent_turn", "done")
                .with_activation_condition(ActivationCondition::MessageHasToolCall),
        )
        .build()?;

    let agent = AgentFactory::default().create(definition.clone())?;
    let run = agent.run(AgentRunInput::new(graph).with_initial_messages(vec![user_message]))?;

    let cancel_graph = Graph::builder("cancel_before_start")
        .node(GraphNode::new("start").with_action(GraphNodeAction::PassthroughInput))
        .start_node("start")
        .build()?;
    let cancelled = agent.run(AgentRunInput::new(cancel_graph).with_stop_requested(true))?;

    let budget_graph = Graph::builder("budget_guard")
        .node(GraphNode::new("start"))
        .start_node("start")
        .budget(GraphStateBudget {
            max_total_node_executions: Some(0),
            max_node_executions: Some(1),
            max_no_progress_ticks: None,
        })
        .build()?;
    let budgeted = GraphRunner::new().run(&budget_graph, GraphRunInput::default())?;

    let detail = format!(
        "status={}, messages={}, events={}, first_event={}, cancel={}, budget_guard={}",
        run.status.as_str(),
        run.messages.len(),
        run.events.len(),
        run.events.first().map(event_name).unwrap_or("none"),
        cancelled.status.as_str(),
        budgeted.status.as_str()
    );

    Ok(GraphProbe {
        detail,
        events: run.events,
    })
}

fn run_reasoning_graph_probe(definition: &agent_core::AgentDefinition) -> AgentCoreResult<String> {
    let manager_plan = RunMessage::assistant(vec![
        ContentBlock::reasoning("Plan: inspect current screen, search catalog, remember result."),
        ContentBlock::text(
            "Subgoal for executor: search for wireless charger and report first result.",
        ),
    ])?;
    let executor_action = RunMessage::assistant(vec![
        ContentBlock::reasoning("Executor will call the direct typing and completion tools."),
        ContentBlock::tool_call(
            "executor-type",
            "type",
            json!({"text": "wireless charger", "index": 0}),
        ),
        ContentBlock::tool_call(
            "executor-complete",
            "complete",
            json!({"success": true, "reason": "executor completed subgoal"}),
        ),
    ])?;
    let manager_check = RunMessage::assistant(vec![ContentBlock::text(
        "Manager verified the executor result and can finalize.",
    )])?;

    let graph = Graph::builder("mobilerun_reasoning_manager_executor")
        .node(
            GraphNode::new("manager_plan")
                .with_action(GraphNodeAction::EmitMessages(vec![manager_plan])),
        )
        .node(
            GraphNode::new("executor_action")
                .with_action(GraphNodeAction::EmitMessages(vec![executor_action])),
        )
        .node(
            GraphNode::new("manager_check")
                .with_action(GraphNodeAction::EmitMessages(vec![manager_check]))
                .terminal(true),
        )
        .start_node("manager_plan")
        .end_node("manager_check")
        .edge(
            GraphEdge::new("manager_to_executor", "manager_plan", "executor_action")
                .with_activation_condition(ActivationCondition::MessageHasText),
        )
        .edge(
            GraphEdge::new("executor_to_manager", "executor_action", "manager_check")
                .with_activation_condition(ActivationCondition::MessageHasToolCall),
        )
        .build()?;

    let run = AgentFactory::default()
        .create(definition.clone())?
        .run(AgentRunInput::new(graph))?;

    Ok(format!(
        "status={}, messages={}, manager_executor_edges={}",
        run.status.as_str(),
        run.messages.len(),
        run.events
            .iter()
            .filter(|event| matches!(event, CoreEvent::MessageEmitted { .. }))
            .count()
    ))
}

fn run_complex_graph_probe(_definition: &agent_core::AgentDefinition) -> AgentCoreResult<String> {
    let task_package = RunMessage::user(vec![ContentBlock::text(
        "Task package: search Demo Shop for wireless charger.",
    )])?;
    let dependency_context = RunMessage::user(vec![ContentBlock::text(
        "Dependency result: app_open=true; screen=home.",
    )])?;
    let observe_screen = RunMessage::assistant(vec![ContentBlock::tool_call(
        "complex-ui",
        "ui_state",
        json!({}),
    )])?;
    let lookup_catalog = RunMessage::assistant(vec![ContentBlock::tool_call(
        "complex-search",
        "search_database",
        json!({"query": "wireless charger"}),
    )])?;
    let summarize = RunMessage::assistant(vec![ContentBlock::text(
        "Branch output available for manager reconciliation.",
    )])?;

    let graph = Graph::builder("mobilerun_complex_task_dag")
        .node(
            GraphNode::new("task_package")
                .with_action(GraphNodeAction::EmitMessages(vec![task_package])),
        )
        .node(
            GraphNode::new("dependency_context")
                .with_action(GraphNodeAction::EmitMessages(vec![dependency_context])),
        )
        .node(
            GraphNode::new("observe_screen")
                .with_action(GraphNodeAction::EmitMessages(vec![observe_screen])),
        )
        .node(
            GraphNode::new("lookup_catalog")
                .with_action(GraphNodeAction::EmitMessages(vec![lookup_catalog])),
        )
        .node(
            GraphNode::new("summarize_branch")
                .with_action(GraphNodeAction::EmitMessages(vec![summarize])),
        )
        .start_node("task_package")
        .start_node("dependency_context")
        .edge(
            GraphEdge::new("task_to_observe", "task_package", "observe_screen")
                .with_activation_condition(ActivationCondition::MessageHasText),
        )
        .edge(
            GraphEdge::new(
                "dependency_to_lookup",
                "dependency_context",
                "lookup_catalog",
            )
            .with_activation_condition(ActivationCondition::MessageHasText),
        )
        .edge(
            GraphEdge::new("observe_to_summary", "observe_screen", "summarize_branch")
                .with_activation_condition(ActivationCondition::MessageHasToolCall),
        )
        .edge(
            GraphEdge::new("lookup_to_summary", "lookup_catalog", "summarize_branch")
                .with_activation_condition(ActivationCondition::MessageHasToolCall),
        )
        .build()?;

    let run = GraphRunner::new().run(&graph, GraphRunInput::default())?;

    Ok(format!(
        "status={}, start_nodes=2, total_node_executions={}, fired_edges={}, summarize_runs={}",
        run.status.as_str(),
        run.state.total_node_executions(),
        run.state.fired_edges().len(),
        run.state.node_execution_count("summarize_branch")
    ))
}

fn execution_session_snapshots(
    user_message: RunMessage,
    assistant_message: RunMessage,
    tool_messages: Vec<RunMessage>,
) -> AgentCoreResult<Vec<ReplaySnapshot>> {
    execution_histories(user_message, assistant_message, tool_messages)?
        .into_iter()
        .map(session_snapshot_from_messages)
        .collect()
}

fn session_snapshot_from_messages(messages: Vec<RunMessage>) -> AgentCoreResult<ReplaySnapshot> {
    let session_id = messages
        .first()
        .map(|message| message.id)
        .ok_or_else(|| agent_core::AgentCoreError::InvalidInput("empty session".to_string()))?;
    let mut store = InMemorySessionStore::new();
    let header = SessionEntry::header(session_id);
    let mut parent_id = header.id;
    store.append(header)?;

    for message in messages {
        let entry = SessionEntry::message(Some(parent_id), message)?;
        parent_id = entry.id;
        store.append(entry)?;
    }

    let tree = store.load_tree()?;
    replay_active_branch(&tree)
}

fn execution_histories(
    user_message: RunMessage,
    assistant_message: RunMessage,
    tool_messages: Vec<RunMessage>,
) -> AgentCoreResult<Vec<Vec<RunMessage>>> {
    let mut full_history = vec![user_message.clone(), assistant_message];
    full_history.extend(tool_messages);

    Ok(vec![
        full_history,
        vec![
            user_message.clone(),
            assistant_from_calls(vec![
                ToolCall::new("hist-ui", "ui_state", json!({})),
                ToolCall::new("hist-tap", "click_at", json!({"x": 512, "y": 176})),
                ToolCall::new("hist-type", "type", json!({"text": "wireless charger"})),
                ToolCall::new(
                    "hist-search",
                    "search_database",
                    json!({"query": "wireless charger"}),
                ),
                ToolCall::new(
                    "hist-remember",
                    "remember",
                    json!({"information": "first_result=Wireless Charger Stand"}),
                ),
                ToolCall::new(
                    "hist-complete",
                    "complete",
                    json!({"success": true, "reason": "result remembered"}),
                ),
            ])?,
        ],
        vec![
            user_message,
            assistant_from_calls(vec![
                ToolCall::new("hist-shot", "screenshot", json!({})),
                ToolCall::new("hist-ui2", "ui_state", json!({})),
                ToolCall::new("hist-wait", "wait", json!({"duration": 1.0})),
                ToolCall::new(
                    "hist-complete2",
                    "complete",
                    json!({"success": true, "reason": "screen already settled"}),
                ),
            ])?,
        ],
    ])
}

fn assistant_from_calls(calls: Vec<ToolCall>) -> AgentCoreResult<RunMessage> {
    let mut builder = AssistantBuilder::new();
    builder.push_reasoning_delta("Historical trajectory sample.");
    for call in calls {
        builder.push_tool_call(call.call_id, call.tool_name, call.arguments);
    }
    builder.finish()
}

fn validate_structured_output(value: &Value) -> bool {
    value.get("success").and_then(Value::as_bool).is_some()
        && value
            .get("remembered_items")
            .and_then(Value::as_array)
            .is_some()
        && value.get("summary").and_then(Value::as_str).is_some()
}

fn project_event_stream(core_events: &[CoreEvent], tool_execution_count: usize) -> String {
    let mut log = EventLog::new();
    log.extend(core_events.iter().cloned());
    for index in 0..tool_execution_count {
        log.push(CoreEvent::HookEmitted {
            name: "tool_execution".to_string(),
            data: json!({
                "step_number": index + 1,
                "success": true,
                "summary": "mock tool dispatch completed"
            }),
        });
    }

    let projected = log
        .events()
        .iter()
        .map(MobileRunEventKind::from_core_event)
        .collect::<Vec<_>>();
    let agent_events = projected
        .iter()
        .filter(|kind| matches!(kind, MobileRunEventKind::AgentLifecycle))
        .count();
    let step_events = projected
        .iter()
        .filter(|kind| matches!(kind, MobileRunEventKind::Step))
        .count();
    let tool_events = projected
        .iter()
        .filter(|kind| matches!(kind, MobileRunEventKind::ToolExecution))
        .count();

    format!(
        "core_events={}, projected_events={}, agent_events={}, step_events={}, tool_events={}",
        core_events.len(),
        projected.len(),
        agent_events,
        step_events,
        tool_events
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MobileRunEventKind {
    AgentLifecycle,
    Workflow,
    Step,
    ToolExecution,
    Message,
    Error,
}

impl MobileRunEventKind {
    fn from_core_event(event: &CoreEvent) -> Self {
        match event {
            CoreEvent::AgentStarted { .. } | CoreEvent::AgentEnded { .. } => Self::AgentLifecycle,
            CoreEvent::GraphStarted { .. } | CoreEvent::GraphEnded { .. } => Self::Workflow,
            CoreEvent::NodeStarted { .. } | CoreEvent::NodeEnded { .. } => Self::Step,
            CoreEvent::MessageEmitted { .. } => Self::Message,
            CoreEvent::HookEmitted { name, .. } if name == "tool_execution" => Self::ToolExecution,
            CoreEvent::HookEmitted { .. } => Self::Workflow,
            CoreEvent::Error { .. } => Self::Error,
        }
    }
}

fn check(name: &'static str, status: &'static str, detail: String) -> CapabilityCheck {
    CapabilityCheck {
        name,
        status,
        detail,
    }
}

fn event_name(event: &CoreEvent) -> &'static str {
    match event {
        CoreEvent::AgentStarted { .. } => "agent_started",
        CoreEvent::AgentEnded { .. } => "agent_ended",
        CoreEvent::GraphStarted { .. } => "graph_started",
        CoreEvent::GraphEnded { .. } => "graph_ended",
        CoreEvent::NodeStarted { .. } => "node_started",
        CoreEvent::NodeEnded { .. } => "node_ended",
        CoreEvent::MessageEmitted { .. } => "message_emitted",
        CoreEvent::HookEmitted { .. } => "hook_emitted",
        CoreEvent::Error { .. } => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_runs_all_visible_tools_and_graph_modes() {
        let report = run_boundary_probe().unwrap();
        let details = report
            .checks
            .iter()
            .map(|check| (check.name, check.detail.as_str()))
            .collect::<Vec<_>>();

        assert!(details.iter().any(|(name, detail)| {
            *name == "tool execution coverage" && detail.contains("executed 16/16 visible tools")
        }));
        assert!(details.iter().any(
            |(name, detail)| *name == "handler hooks" && detail.contains("recover_messages=1")
        ));
        assert!(details.iter().any(|(name, detail)| {
            *name == "reasoning manager/executor graph" && detail.contains("status=completed")
        }));
        assert!(details.iter().any(|(name, detail)| {
            *name == "turn loop input queue" && detail.contains("after_submit=Ready")
        }));
        assert!(details.iter().any(|(name, detail)| {
            *name == "event stream projection" && detail.contains("tool_events=16")
        }));
        assert!(details.iter().any(|(name, detail)| {
            *name == "trajectory probability graph" && detail.contains("session_branches=3")
        }));
        assert!(
            details
                .iter()
                .any(|(name, detail)| *name == "tool preexecution"
                    && detail.contains("preexecuted=2"))
        );
        assert!(details.iter().any(|(name, detail)| {
            *name == "parallel tool execution" && detail.contains("parallel_results=2")
        }));
        assert!(details.iter().any(|(name, detail)| {
            *name == "cache-aware context ordering" && detail.contains("stable_prefix")
        }));
        assert!(details
            .iter()
            .any(|(name, detail)| *name == "key/account routing"
                && detail.contains("separate_headers=true")));
        assert!(details.iter().any(|(name, detail)| {
            *name == "complex task graph" && detail.contains("status=completed")
        }));
        assert!(details.iter().any(|(name, detail)| {
            *name == "app map memory" && detail.contains("forget_removed=1")
        }));
        assert!(details.iter().any(|(name, detail)| {
            *name == "hundred-step cross-app task"
                && detail.contains("completed=true")
                && detail.contains("steps=")
        }));
        assert!(details.iter().any(|(name, detail)| {
            *name == "token optimization effects" && detail.contains("long_task_token_savings=")
        }));
    }
}

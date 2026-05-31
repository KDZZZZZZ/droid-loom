use crate::app_map_memory::{sample_cross_app_map, AppMapMemory, ForgetScope};
use crate::context_stability::{
    mark_stability, order_by_stability, stability_order_labels, stable_prefix_id, ContextStability,
};
use crate::cross_app_task::run_hundred_step_cross_app_task;
use crate::execution_probability::TrajectoryProbabilityGraph;
use crate::key_routing::{EnvSecretResolver, KeyRoutePlan, SecretResolver, StaticSecretResolver};
use crate::mobile_tools::{expected_action_count, register_mobilerun_tools};
use crate::prompt::{default_tool_visibility, render_template, ExecutionMode, MobileRunLikeConfig};
use crate::scripted_agent;
use crate::task_context::{
    sample_mobile_task_dag, TaskContextPackage, TaskDependencyGraph, TaskNode,
};
use agent_core::assistant_builder::AssistantBuilder;
use agent_core::content_block::ContentBlock;
use agent_core::llm_request::LlmRequest;
use agent_core::run_message::RunMessage;
use agent_core::tool_executor::{ToolCall, ToolExecutor};
use agent_core::tool_registry::ToolRegistry;
use agent_core::{AgentCoreResult, ToolVisibility};
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::Arc;

fn assistant_with_tools(tools: &[&str]) -> AgentCoreResult<RunMessage> {
    let mut builder = AssistantBuilder::new();
    for tool in tools {
        builder.push_tool_call(format!("call-{tool}"), *tool, json!({}));
    }
    builder.finish()
}

#[test]
fn prompt_and_tool_pack_public_api_contract() -> AgentCoreResult<()> {
    let config = MobileRunLikeConfig::boundary_default();
    assert!(config.render_user_prompt().contains("wireless charger"));
    assert!(config.render_system_prompt().contains("Use direct tools"));

    let definition = config.agent_definition()?;
    assert_eq!(definition.name(), "mobilerun_like_fast_agent");
    assert_eq!(
        definition.tool_visibility().get("screenshot"),
        Some(&ToolVisibility::Direct)
    );

    let mut reasoning_config = config.clone();
    reasoning_config.mode = ExecutionMode::Reasoning;
    let reasoning_definition = reasoning_config.agent_definition_with_tool_visibility(
        BTreeMap::from([("ui_state".to_string(), ToolVisibility::Direct)]),
    )?;
    assert_eq!(
        reasoning_definition.tool_visibility().get("ui_state"),
        Some(&ToolVisibility::Direct)
    );
    assert!(reasoning_definition
        .system_prompt()
        .contains("reasoning mobile automation"));

    let visibility = default_tool_visibility();
    assert_eq!(
        visibility.get("raw_adb_shell"),
        Some(&ToolVisibility::Hidden)
    );
    assert_eq!(
        render_template(
            "Open {{ app_name }}",
            &BTreeMap::from([("app_name".to_string(), "Demo Shop".to_string())])
        ),
        "Open Demo Shop"
    );

    let mut registry = ToolRegistry::new();
    register_mobilerun_tools(&mut registry)?;
    assert_eq!(registry.names().len(), expected_action_count());
    assert_eq!(registry.direct_schemas(&definition).len(), 8);
    assert!(!registry.search(&definition, "database", 5).is_empty());
    assert!(registry
        .get_for_agent(&definition, "raw_adb_shell")
        .is_err());
    Ok(())
}

#[test]
fn probability_graph_task_context_and_key_routing_public_api_contract() -> AgentCoreResult<()> {
    let sessions = vec![
        vec![
            RunMessage::user(vec![ContentBlock::text("task")])?,
            assistant_with_tools(&["ui_state", "search_database"])?,
        ],
        vec![
            RunMessage::user(vec![ContentBlock::text("task")])?,
            assistant_with_tools(&["ui_state"])?,
        ],
    ];
    let mut probability_graph = TrajectoryProbabilityGraph::from_sessions(&sessions);
    probability_graph.observe_session(&sessions[0]);
    assert_eq!(probability_graph.session_count(), 3);
    assert_eq!(
        probability_graph.likely_next("message:assistant", 1)[0].event,
        "tool_call:ui_state"
    );
    assert!(probability_graph.tool_use_probability("ui_state") > 0.9);

    let replay_graph = TrajectoryProbabilityGraph::from_replay_snapshots(&[
        agent_core::session_replay::ReplaySnapshot {
            leaf_id: None,
            entries: Vec::new(),
            messages: sessions[0].clone(),
            summaries: Vec::new(),
        },
    ]);
    assert_eq!(replay_graph.session_count(), 1);

    let config = MobileRunLikeConfig::boundary_default();
    let definition = config.agent_definition()?;
    let mut registry = ToolRegistry::new();
    register_mobilerun_tools(&mut registry)?;
    let plan = probability_graph.plan_preexecution(
        &registry,
        &definition,
        vec![
            ToolCall::new("pre-ui", "ui_state", json!({})),
            ToolCall::new("pre-click", "click", json!({"index": 0})),
        ],
        0.8,
    );
    assert_eq!(plan.calls.len(), 1);
    assert!(plan.skipped.iter().any(|entry| entry.contains("click")));
    let executor = ToolExecutor::new(Arc::new(registry.clone()));
    let outcome =
        TrajectoryProbabilityGraph::execute_preexecution_plan(&executor, &definition, plan)?;
    assert_eq!(outcome.results.len(), 1);
    assert_eq!(outcome.messages.len(), 1);

    let layers = probability_graph.plan_tool_layers(&default_tool_visibility(), 0.8);
    assert!(layers.direct_tools.contains(&"ui_state".to_string()));
    assert!(layers.dynamic_tools.contains(&"remember".to_string()));
    assert!(layers.hidden_tools.contains(&"raw_adb_shell".to_string()));
    assert_eq!(
        layers.visibility.get("raw_adb_shell"),
        Some(&ToolVisibility::Hidden)
    );

    let package = TaskContextPackage::new("collect", "Collect account state")
        .with_inputs(["account_id=fixture"])
        .with_artifacts(["account_state=ready"])
        .with_required_state(["screen=account"]);
    let task = TaskNode::new(package).depends_on(["open_app"]);
    assert_eq!(task.depends_on, vec!["open_app".to_string()]);

    let mut task_graph = TaskDependencyGraph::new();
    task_graph.add_task(TaskNode::new(
        TaskContextPackage::new("open_app", "Open the app").with_artifacts(["screen=home"]),
    ))?;
    task_graph.add_task(task)?;
    assert_eq!(
        task_graph.dependency_edges(),
        vec![("open_app".to_string(), "collect".to_string())]
    );
    assert_eq!(task_graph.context_for("collect")?.len(), 2);
    assert!(sample_mobile_task_dag()?.context_for("finalize")?.len() >= 2);

    let ordered = order_by_stability(vec![
        mark_stability(
            RunMessage::user(vec![ContentBlock::text("screen")])?,
            ContextStability::VolatileObservation,
        ),
        mark_stability(
            RunMessage::user(vec![ContentBlock::text("prefix")])?,
            ContextStability::StablePrefix,
        ),
        mark_stability(
            RunMessage::user(vec![ContentBlock::text("task")])?,
            ContextStability::TaskPackage,
        ),
    ]);
    assert_eq!(
        stability_order_labels(&ordered),
        vec!["stable_prefix", "task_package", "volatile_observation"]
    );
    let prefix_id = stable_prefix_id(
        definition.system_prompt(),
        &registry
            .direct_schemas(&definition)
            .into_iter()
            .map(|schema| schema.name)
            .collect::<Vec<_>>(),
    );
    assert!(prefix_id.starts_with("stable-prefix-"));

    let route_plan = KeyRoutePlan::new(prefix_id.clone());
    assert_eq!(
        route_plan.route_for_prefix(&prefix_id).account_label,
        "stable-prefix-account"
    );
    assert_eq!(
        route_plan.route_for_prefix("volatile-prefix").account_label,
        "general-pool-account"
    );
    let resolver = StaticSecretResolver::new([
        ("MOBILERUN_STABLE_PREFIX_API_KEY", "stable-fixture"),
        ("MOBILERUN_GENERAL_POOL_API_KEY", "general-fixture"),
    ]);
    let routed = route_plan.apply(LlmRequest::new("model"), &prefix_id, &resolver)?;
    assert_eq!(routed.account_label, "stable-prefix-account");
    assert_eq!(
        routed.request.headers.get("Authorization"),
        Some(&"Bearer stable-fixture".to_string())
    );
    assert!(EnvSecretResolver
        .resolve("MOBILERUN_CONTRACT_TEST_UNSET_API_KEY")
        .is_none());

    Ok(())
}

#[test]
fn app_map_and_hundred_step_task_public_api_contract() -> AgentCoreResult<()> {
    let mut map = AppMapMemory::new();
    let observed = map.observe_ui_state(
        &json!({
            "package": "com.example.shop",
            "nodes": [
                {"text": "Search", "class": "android.widget.EditText", "clickable": true, "editable": true},
                {"text": "Orders", "class": "android.widget.Button", "clickable": true}
            ]
        }),
        Some("find orders"),
    )?;
    assert!(observed.created);
    assert_eq!(map.page_count(), 1);
    assert!(map.current_page().is_some());
    assert!(!observed.candidate_actions.is_empty());
    assert_eq!(
        observed.candidate_actions[0]
            .to_tool_call("map-call")
            .call_id,
        "map-call"
    );

    let mut cross_app_map = sample_cross_app_map()?;
    let home = cross_app_map.current_page().unwrap().to_string();
    let hit = cross_app_map.semantic_search("Share", 1).remove(0);
    let path = cross_app_map.plan_path(&home, &hit.page_id).unwrap();
    assert_eq!(path.len(), 2);
    let view = cross_app_map.local_view(Some(&home), 1, Some("Share"));
    assert!(
        AppMapMemory::estimate_context_tokens(&view) < cross_app_map.estimate_full_map_tokens()
    );
    let first_page = cross_app_map.pages().next().unwrap().id.clone();
    assert!(cross_app_map.mark_page_stale(&first_page));
    assert_eq!(cross_app_map.forget(ForgetScope::Stale), 1);
    assert!(cross_app_map.transition_count() <= 3);

    let config = MobileRunLikeConfig::boundary_default();
    let definition = config.agent_definition()?;
    let mut registry = ToolRegistry::new();
    register_mobilerun_tools(&mut registry)?;
    let executor = ToolExecutor::new(Arc::new(registry));
    let report = run_hundred_step_cross_app_task(&executor, &definition)?;
    assert!(report.completed);
    assert!(report.step_count >= 100);
    assert!(report.map_reuse_hits >= 100);
    assert!(report.detail().contains("completed=true"));
    Ok(())
}

#[test]
fn boundary_probe_public_entry_runs() -> AgentCoreResult<()> {
    let report = scripted_agent::run_boundary_probe()?;
    report.print();
    Ok(())
}

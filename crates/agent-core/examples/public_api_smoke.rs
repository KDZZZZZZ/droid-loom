use agent_core::agent::AgentRunInput;
use agent_core::agent_definition::AgentDefinitionBuilder;
use agent_core::event::CoreEvent;
use agent_core::graph::Graph;
use agent_core::graph_node::{Cardinality, GraphNode, InputPackageSpec, MessageQuery};
use agent_core::hook::{
    HookEventRequest, HookName, HookPayload, PointHookDecision, WrapperRequest, WrapperResponse,
    WrapperResult,
};
use agent_core::hook_handler::HandlerRegistry;
use agent_core::tool::{Tool, ToolInvocation, ToolMetadata, ToolOutput};
use agent_core::tool_executor::{ToolCall, ToolExecutor};
use agent_core::tool_registry::ToolRegistry;
use agent_core::tool_schema::ToolSchema;
use agent_core::{user_input, AgentCoreResult, AgentFactory, ToolVisibility};
use serde_json::json;
use std::sync::Arc;

#[derive(Debug)]
struct EchoTool {
    metadata: ToolMetadata,
}

impl EchoTool {
    fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        default_visibility: ToolVisibility,
    ) -> AgentCoreResult<Self> {
        let schema = ToolSchema::new(
            name,
            description,
            json!({
                "type": "object",
                "required": ["text"],
                "properties": {
                    "text": { "type": "string" }
                },
                "additionalProperties": false
            }),
        )?;

        Ok(Self {
            metadata: ToolMetadata::new(schema, default_visibility),
        })
    }
}

impl Tool for EchoTool {
    fn metadata(&self) -> &ToolMetadata {
        &self.metadata
    }

    fn invoke(&self, invocation: ToolInvocation) -> AgentCoreResult<ToolOutput> {
        let text = invocation
            .arguments
            .get("text")
            .and_then(|value| value.as_str())
            .unwrap_or_default();

        Ok(ToolOutput::new(json!({
            "echo": text,
            "call_id": invocation.call_id
        })))
    }
}

fn main() -> AgentCoreResult<()> {
    let definition = AgentDefinitionBuilder::new()
        .name("public_api_demo")
        .system_prompt("You are a demo agent used by the public API smoke example.")
        .tool_visibility("echo", ToolVisibility::Direct)
        .build()?;

    let agent = AgentFactory::default().create(definition.clone())?;

    let mut tools = ToolRegistry::new();
    tools.register(EchoTool::new(
        "echo",
        "Echo input text",
        ToolVisibility::Searchable,
    )?)?;
    tools.register(EchoTool::new(
        "lookup_notes",
        "Search notes by query",
        ToolVisibility::Searchable,
    )?)?;
    tools.register(EchoTool::new(
        "secret_admin",
        "Hidden admin operation",
        ToolVisibility::Hidden,
    )?)?;
    let direct_tool_count = tools.direct_schemas(&definition).len();
    let searchable_tool_count = tools.search(&definition, "notes", 5).len();

    let executor = ToolExecutor::new(Arc::new(tools));
    let tool_result = executor.execute_one(
        &definition,
        ToolCall::new("call-1", "echo", json!({ "text": "hello tool" })),
    )?;
    let tool_message = tool_result.into_run_message()?;

    let mut handlers = HandlerRegistry::new();
    handlers.register_point(HookName::Input, 0, |payload: HookPayload| {
        Ok(PointHookDecision::Emit {
            event: HookEventRequest::new("input_seen", payload.data),
        })
    })?;
    handlers.register_wrapper(
        HookName::NodeExecution,
        0,
        |request: WrapperRequest, next| next.run(request),
    )?;

    let hook_outcome = handlers
        .run_point(HookPayload::new(HookName::Input).with_data(json!({ "text": "hello agent" })))?;
    let wrapper_result =
        handlers.run_wrapper(WrapperRequest::new(HookName::NodeExecution), |_| {
            Ok(WrapperResult::Continue(WrapperResponse::new(json!({
                "node": "demo"
            }))))
        })?;

    let user_message = user_input::text_message("hello agent")?;
    let graph = Graph::builder("public_api_smoke")
        .node(GraphNode::final_node(
            "done",
            InputPackageSpec::new("input").required(
                "turn",
                MessageQuery::any(),
                Cardinality::Latest,
            ),
        ))
        .edge("input_to_done", ("input", "messages"), ("done", "input"))
        .finish_at("done")
        .build()?;

    let result = agent
        .run(AgentRunInput::new(graph).with_initial_messages(vec![user_message, tool_message]))?;

    let cancel_agent = AgentFactory::default().create(definition)?;
    let cancel_graph = Graph::builder("cancelled_before_start")
        .node(GraphNode::final_node(
            "start",
            InputPackageSpec::new("input").required(
                "turn",
                MessageQuery::any(),
                Cardinality::Latest,
            ),
        ))
        .edge("input_to_start", ("input", "messages"), ("start", "input"))
        .finish_at("start")
        .build()?;
    let cancelled = cancel_agent.run(AgentRunInput::new(cancel_graph).with_stop_requested(true))?;

    println!("agent={}", agent.definition().name());
    println!("direct_tools={direct_tool_count}");
    println!("searchable_tools={searchable_tool_count}");
    println!("hook_events={}", hook_outcome.events.len());
    println!("wrapper_result={}", wrapper_result_name(&wrapper_result));
    println!("run_status={}", result.status.as_str());
    println!("messages={}", result.messages.len());
    println!("events={}", result.events.len());
    println!("cancel_status={}", cancelled.status.as_str());

    if let Some(first_event) = result.events.first() {
        println!("first_event={}", event_name(first_event));
    }

    Ok(())
}

fn wrapper_result_name(result: &WrapperResult) -> &'static str {
    match result {
        WrapperResult::Continue(_) => "continue",
        WrapperResult::Rewrite(_) => "rewrite",
        WrapperResult::Retry { .. } => "retry",
        WrapperResult::Recover { .. } => "recover",
        WrapperResult::Stop { .. } => "stop",
        WrapperResult::Fail { .. } => "fail",
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

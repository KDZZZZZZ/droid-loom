# Core Public API 文档

日期：2026-05-31

## 范围

本文规定外部代码如何使用 `agent-core`。第一版只承诺七类能力：

- 定义 agent
- 定义 tool
- 构建 graph
- 在指定 hook 挂 handler
- 读取 event
- 输入用户信息
- 终止 agent graph

外部调用方包括 CLI、TUI、HTTP server、测试 harness、tool pack、plugin adapter 和后续多 agent 编排层。外部代码不直接改 graph runtime state、不直接改 session tree 内部结构、不绕过 registry 调用 tool。

已经跑通的真实 ReAct 最小路径只展示 Android 百步级 phone-using 任务。该任务通过 `agent-smoke` 触发真实 provider，在模拟器中运行单次 ReAct graph loop：

```powershell
cd agent-smoke
$env:MIMO_API_KEY="<runtime key>"
.\scripts\run-android-autonomous-map-task.ps1 `
  -TargetToolCalls 100 `
  -MinimumRequiredToolCalls 95 `
  -ReactSelfCheckInterval 10 `
  -MaxToolRounds 128 `
  -WaitSeconds 900
```

验收口径：graph 为 `start -> react_loop -> final_summary`，`react_loop` 自环；只注册 primitive phone tools，不注册 `android_map_goal_task`、`android_map_long_task_smoke` 等宏/debug tools。报告写入 `agent-smoke/android-shell/app-map-autonomous-report.json`，只有在 `minimum_required_tool_calls` 达标、`macro_tool_traces=0`、`repeated_fixed_action_loop=false`，并且真实审计子目标和工具覆盖达标时才算通过。最近一次真实 provider run 摘要为 `actual_tool_calls=96/100`、`provider_tool_traces=96`、`macro_tool_traces=0`、`task_subgoal_coverage_count=10/10`。

## 使用方案

标准接入顺序：

1. 配置层创建 `AgentDefinition`。
2. runtime setup 注册 provider、tool、handler 等服务依赖。
3. 应用层选择或构造 `Graph`。
4. input 层把用户输入包装成 `RunMessage(role=user)`。
5. run 层调用 `Agent::run(AgentRunInput)`。
6. event 层从 result 读取 `CoreEvent`，用于 UI、日志和测试断言。
7. session 层只提交 finalized messages，不提交 streaming 中间态。

新增公开入口前必须先回答三件事：

- 它属于七类能力中的哪一类。
- 它是否能复用现有 `AgentDefinition`、`Tool`、`Graph`、`HookName`、`CoreEvent`、`RunMessage`。
- 它是否会让外部调用方直接接触 runtime mutable state；如果会，默认不开放。

## 具体 API 速查

下面所有代码块都是 API 调用片段，不是独立 runnable path。唯一已经跑通的真实 ReAct runnable path 是本文顶部的 Android 百步级 phone-using 任务。

### 定义 Agent

调用方：配置加载器、CLI preset、测试构造器、多 agent 编排器。

#### `AgentDefinitionBuilder`

构造 agent definition 的 builder。

```rust
use agent_core::agent_definition::{AgentDefinitionBuilder, ToolVisibility};

let definition = AgentDefinitionBuilder::new()
    .name("mobile_fast_agent")
    .system_prompt("Use tools for every phone action.")
    .tool_visibility("android_ui_state", ToolVisibility::Direct)
    .tool_visibility("android_map_search", ToolVisibility::Searchable)
    .build()?;
```

#### `AgentDefinition`

读取已构造的 agent 配置。

```rust
let name = definition.name();
let system_prompt = definition.system_prompt();
let visibility = definition.tool_visibility().get("android_ui_state");
definition.validate()?;
```

#### `ToolVisibility`

设置工具曝光层级。它本身只是 agent definition 上的配置值，生效点在 `ToolRegistry` 和 `ContextBuildInput`。

```rust
use agent_core::agent_definition::ToolVisibility;

let direct = ToolVisibility::Direct;
let searchable = ToolVisibility::Searchable;
let hidden = ToolVisibility::Hidden;
```

#### `AgentFactory`

把 definition 装配成可运行的 agent。

```rust
use agent_core::{AgentFactory, AgentServices};

let factory = AgentFactory::new(AgentServices::default());
let agent = factory.create(definition.clone())?;
```

#### `Agent`

读取 agent 信息并执行 graph。

```rust
use agent_core::agent::AgentRunInput;

let agent_id = agent.id();
let definition_name = agent.definition().name();
let result = agent.run(AgentRunInput::new(graph).with_initial_messages(messages))?;
```

#### `AgentRunResult`

读取 agent run 结果。

```rust
let result = agent.run(input)?;
let run_id = result.run_id;
let messages = result.messages;
let events = result.events;
let error = result.error;
```

#### `AgentRunStatus`

匹配 agent run 状态。

```rust
use agent_core::agent::AgentRunStatus;

match result.status {
    AgentRunStatus::Completed => {}
    AgentRunStatus::Cancelled => {}
    AgentRunStatus::Failed => {}
}
```

### 定义 Tool

调用方：tool pack、MCP adapter、plugin adapter、测试 mock tool。

#### `Tool`

实现一个工具。

```rust
use agent_core::tool::{Tool, ToolInvocation, ToolMetadata, ToolOutput};
use agent_core::AgentCoreResult;

#[derive(Debug)]
struct UiStateTool {
    metadata: ToolMetadata,
}

impl Tool for UiStateTool {
    fn metadata(&self) -> &ToolMetadata {
        &self.metadata
    }

    fn invoke(&self, invocation: ToolInvocation) -> AgentCoreResult<ToolOutput> {
        Ok(ToolOutput::new(serde_json::json!({
            "call_id": invocation.call_id,
            "screen": "settings"
        })))
    }
}
```

#### `ToolMetadata`

绑定 schema、默认可见性和工具能力。

```rust
use agent_core::tool::ToolMetadata;
use agent_core::ToolVisibility;

let mut metadata = ToolMetadata::new(schema, ToolVisibility::Direct);
metadata.capabilities.read_only = true;
metadata.capabilities.idempotent = true;
assert!(metadata.can_preexecute());
```

#### `ToolInvocation`

工具执行时收到的调用对象。

```rust
use agent_core::tool::ToolInvocation;

let invocation = ToolInvocation::new(
    "call-ui-1",
    "android_ui_state",
    serde_json::json!({}),
);
let tool_name = invocation.tool_name.as_str();
```

#### `ToolOutput`

工具返回值。

```rust
use agent_core::tool::ToolOutput;

let output = ToolOutput::new(serde_json::json!({
    "ok": true,
    "screen": "settings"
}));
```

#### `ToolSchema`

定义和校验工具入参。

```rust
use agent_core::tool_schema::ToolSchema;

let schema = ToolSchema::new(
    "android_click_text",
    "Click visible text on the current screen.",
    serde_json::json!({
        "type": "object",
        "required": ["text"],
        "properties": {
            "text": { "type": "string" }
        },
        "additionalProperties": false
    }),
)?;
schema.validate_arguments(&serde_json::json!({"text": "Settings"}))?;
```

#### `ToolRegistry`

注册工具并按 agent definition 解析可见工具。

```rust
use agent_core::tool_registry::ToolRegistry;

let mut registry = ToolRegistry::new();
registry.register(ui_state_tool)?;
let direct_tools = registry.direct_schemas(&definition);
let searched = registry.search(&definition, "map", 5);
let visible_tool = registry.get_for_agent(&definition, "android_ui_state")?;
let hidden_blocked = registry.get_for_agent(&definition, "raw_adb_shell").is_err();
```

#### `ToolExecutor`

通过 registry、schema 和 permission guard 执行工具。

```rust
use agent_core::tool_executor::{ToolCall, ToolExecutor};
use std::sync::Arc;

let executor = ToolExecutor::new(Arc::new(registry));
let result = executor.execute_one(
    &definition,
    ToolCall::new("call-ui-1", "android_ui_state", serde_json::json!({})),
)?;
let batch = executor.execute_batch_parallel(&definition, calls)?;
```

#### `ToolCall`

构造待执行的工具调用。

```rust
use agent_core::tool_executor::ToolCall;

let call = ToolCall::new(
    "call-click-1",
    "android_click_text",
    serde_json::json!({"text": "Network & internet"}),
);
```

#### `ToolResult`

读取工具执行结果，并转为 tool message。

```rust
use agent_core::tool_result::ToolResult;

let result = ToolResult::success(
    "call-ui-1",
    "android_ui_state",
    serde_json::json!({"screen": "settings"}),
);
assert!(!result.is_error());
let message = result.into_run_message()?;
```

#### `ToolResultStatus`

判断工具结果状态。

```rust
use agent_core::tool_result::ToolResultStatus;

match result.status {
    ToolResultStatus::Success => {}
    ToolResultStatus::Denied => {}
    ToolResultStatus::Error => {}
}
```

### 构建 Graph

调用方：默认 ReAct graph provider、多 agent 编排器、复杂 phone-using harness、graph 调度测试。

#### `Graph`

`Graph` 是内容包 graph。初始输入固定写到 `("input", "messages")` output log；下游 node 通过 edge 把某个 output port 接到某个 input package。node 不再声明 `start_node`，只要它的 required package items 满足，就可以解锁运行。

```rust
use agent_core::graph::Graph;
use agent_core::graph_node::{Cardinality, GraphNode, InputPackageSpec, MessageQuery};

let graph = Graph::builder("phone_react")
    .node(GraphNode::agent(
        "agent",
        "phone_agent",
        InputPackageSpec::new("context")
            .required("turn", MessageQuery::where_eq("role", "user"), Cardinality::Latest)
            .optional(
                "tool_result",
                MessageQuery::where_eq("content[*].type", "tool_result"),
                Cardinality::Latest,
            ),
    ).output("tool_calls").output("final"))
    .node(GraphNode::tool(
        "tap_tool",
        "tap",
        "tool_call",
        "results",
        InputPackageSpec::new("calls").required(
            "tool_call",
            MessageQuery::where_eq("content[*].type", "tool_call"),
            Cardinality::Latest,
        ),
    ))
    .node(GraphNode::final_node(
        "final",
        InputPackageSpec::new("answer").required(
            "final_answer",
            MessageQuery::where_exists("content[*].text"),
            Cardinality::Latest,
        ),
    ))
    .edge("input_to_agent", ("input", "messages"), ("agent", "context"))
    .edge("agent_to_tool", ("agent", "tool_calls"), ("tap_tool", "calls"))
    .edge("tool_to_agent", ("tap_tool", "results"), ("agent", "context"))
    .edge("agent_to_final", ("agent", "final"), ("final", "answer"))
    .finish_at("final")
    .build()?;
```

#### `GraphNode`

定义一等 node。`GraphNode` 是 `NodeSpec` 的公开别名，支持普通 transform、tool node、agent node、subgraph node 和 final node。

```rust
use agent_core::graph_node::{GraphNode, InputPackageSpec, NodeKind};
use serde_json::json;

let transform = GraphNode::new(
    "observe_screen",
    NodeKind::Transform {
        executor: "observe_screen".to_string(),
        config: json!({"read_only": true}),
    },
    InputPackageSpec::new("context"),
).output("observation");

let agent = GraphNode::agent("manager", "phone_manager", InputPackageSpec::new("task"))
    .output("tool_calls")
    .output("final");

let final_node = GraphNode::final_node("final", InputPackageSpec::new("answer"));
```

#### `GraphEdge`

edge 只做内容传输：从 `(source_node, output_port)` 读取 output log，写入 `(target_node, input_package)`。分流靠 output port，解锁靠目标 package 的 required/optional item。

```rust
use agent_core::graph_edge::GraphEdge;

let edge = GraphEdge::new(
    "agent_tool_calls_to_tap",
    ("agent", "tool_calls"),
    ("tap_tool", "calls"),
);

assert_eq!(edge.from().port, "tool_calls");
assert_eq!(edge.to().package, "calls");
```

#### `InputPackageSpec`

声明下游 node 需要的内容包。required items 全满足才解锁；optional items 会随包一起传入，但不阻塞解锁。

```rust
use agent_core::graph_node::{Cardinality, InputPackageSpec, MessageQuery};

let package = InputPackageSpec::new("context")
    .required("turn", MessageQuery::any(), Cardinality::Latest)
    .optional(
        "tool_result",
        MessageQuery::where_eq("content[*].type", "tool_result"),
        Cardinality::Latest,
    );
```

#### `MessageQuery`

筛选 message 及其字段。被 query 筛掉的 message 不进入 package，也不算新内容；选择字段的 hash 不变时，不会重复推进 package version。对于 `content[*]` 查询，select 只暴露匹配的 content blocks。

```rust
use agent_core::graph_node::{FieldOp, MessageQuery};
use serde_json::json;

let tool_call = MessageQuery::where_eq("content[*].type", "tool_call")
    .select([
        "id",
        "content[*].call_id",
        "content[*].tool_name",
        "content[*].arguments",
    ]);

let important = MessageQuery::any()
    .filter("metadata.kind", FieldOp::In(vec![json!("task"), json!("dependency")]));
```

#### `NodeOutput`

node executor 用 output port 分流。多个 edge 可以从同一个 port fan-out；不同语义的输出应优先写到不同 port。

```rust
use agent_core::graph_node::NodeOutput;

let output = NodeOutput::new()
    .with_message("tool_calls", assistant_tool_call_message)
    .with_message("final", assistant_final_message);
```

#### `NodeConcurrency`

控制同一个 node 的 activation 并发。`Serial` 保证同一 node 一次只跑一个 activation；`Parallel` 允许多个 activation 同时跑；`ByKey` 按某个 package item 的 selected hash 分组限流。

```rust
use agent_core::graph_node::{ConcurrencyKey, GraphNode, NodeConcurrency};

let node = GraphNode::tool("lookup_catalog", "search_database", "task", "results", package)
    .concurrency(NodeConcurrency::ByKey {
        key: ConcurrencyKey::package_item("task"),
        max_per_key: 1,
    });
```

#### `GraphRunner`

同步门面，内部执行 async graph runtime。默认 executor 只处理 final/empty transform；agent/tool/subgraph node 需要传入自定义 `NodeExecutor`。

```rust
use std::sync::Arc;
use agent_core::graph_runner::{GraphRunInput, GraphRunner};

let runner = GraphRunner::with_executor(Arc::new(RuntimeExecutor));
let run = runner.run(
    &graph,
    GraphRunInput::new(vec![user_message]).with_max_ticks(10_000),
)?;

assert_eq!(run.status.as_str(), "completed");
let transfers = run.ledger.transfers;
let state = run.state;
```

#### `GraphRunInput`

直接运行 graph 时传入初始消息、可选 run id、停止标记和 tick 预算。

```rust
use agent_core::graph_runner::GraphRunInput;
use uuid::Uuid;

let input = GraphRunInput::new(vec![user_message])
    .with_run_id(Uuid::new_v4())
    .with_stop_requested(false)
    .with_max_ticks(1_000);
```

#### `GraphRunResult`

读取 graph runner 的输出、runtime state 和 ledger。`state` 记录 output logs、edge 游标和 package 状态；`ledger` 记录 edge transfer 和 node attempt。

```rust
let run = runner.run(&graph, input)?;
let status = run.status;
let messages = run.messages;
let package_states = run.state.package_states;
let transfers = run.ledger.transfers;
```

#### `GraphRunStatus`

匹配 graph run 状态。

```rust
use agent_core::graph_runner::GraphRunStatus;

match run.status {
    GraphRunStatus::Completed | GraphRunStatus::Drained => {}
    GraphRunStatus::Cancelled => {}
    GraphRunStatus::BudgetExceeded | GraphRunStatus::Failed => {}
}
```

#### `NodeExecutor`

执行一等 node。tool node、agent node、subgraph node、final node 都通过同一个 async executor 接口调度。

```rust
use agent_core::{AgentCoreResult, graph_runtime as rt};
use futures::future::BoxFuture;

struct RuntimeExecutor;

impl rt::NodeExecutor for RuntimeExecutor {
    fn execute(
        &self,
        node: rt::NodeSpec,
        input: rt::NodeInput,
        ctx: rt::NodeExecutionContext,
    ) -> BoxFuture<'static, AgentCoreResult<rt::NodeOutput>> {
        Box::pin(async move {
            match node.kind {
                rt::NodeKind::Agent(agent) => run_agent_node(agent, input, ctx).await,
                rt::NodeKind::Tool(tool) => run_tool_node(tool, input, ctx).await,
                rt::NodeKind::Final => Ok(rt::NodeOutput::new()),
                _ => Ok(rt::NodeOutput::new()),
            }
        })
    }
}
```

#### `GraphRuntime`

原生 async runtime。需要异步环境时直接用它；需要同步调用时用 `GraphRunner`。

```rust
use std::sync::Arc;
use agent_core::graph_runtime as rt;

let services = rt::GraphRuntimeServices::new(Arc::new(RuntimeExecutor));
let output = rt::GraphRuntime::new(graph, services)
    .run(rt::GraphRunInput::new(vec![user_message]).with_max_ticks(10_000))
    .await?;

assert_eq!(output.status, rt::GraphRunStatus::Completed);
let transfers = output.ledger.transfers;
let attempts = output.ledger.node_attempts;
```

#### `AgentRunInput`

把 graph 和初始消息交给 agent。

```rust
use agent_core::agent::AgentRunInput;

let input = AgentRunInput::new(graph)
    .with_initial_messages(vec![user_message])
    .with_stop_requested(false);
let result = agent.run(input)?;
```

### 挂 Hook Handler

调用方：extension、permission policy、audit/logging、recoverable error policy、测试 handler。

#### `HookName`

选择 hook 点。

```rust
use agent_core::hook::{HookKind, HookName};

let hook = HookName::Input;
assert_eq!(hook.kind(), HookKind::Point);
let wrapper = HookName::NodeExecution;
assert_eq!(wrapper.kind(), HookKind::Wrapper);
```

#### `HookPayload`

构造 point hook 的输入。

```rust
use agent_core::hook::{HookName, HookPayload};

let payload = HookPayload::new(HookName::Input)
    .with_data(serde_json::json!({"text": "open settings"}))
    .with_metadata("source", serde_json::json!("ui"));
```

#### `PointHookDecision`

返回 point hook 决策。

```rust
use agent_core::hook::{HookEventRequest, PointHookDecision};

let continue_decision = PointHookDecision::Continue;
let emit_decision = PointHookDecision::Emit {
    event: HookEventRequest::new("input_seen", serde_json::json!({"ok": true})),
};
```

#### `WrapperRequest`

构造 wrapper hook 的输入。

```rust
use agent_core::hook::{HookName, WrapperRequest};

let request = WrapperRequest::new(HookName::NodeExecution)
    .with_data(serde_json::json!({"node": "react_loop"}))
    .with_metadata("attempt", serde_json::json!(1));
```

#### `WrapperResponse`

构造 wrapper hook 的正常输出。

```rust
use agent_core::hook::WrapperResponse;

let response = WrapperResponse::new(serde_json::json!({"ok": true}))
    .with_metadata("duration_ms", serde_json::json!(42));
```

#### `WrapperResult`

表达 wrapper hook 的继续、恢复或失败。

```rust
use agent_core::hook::{WrapperResponse, WrapperResult};

let ok = WrapperResult::Continue(WrapperResponse::new(serde_json::json!({"ok": true})));
let recover = WrapperResult::Recover {
    messages: vec![diagnostic_message],
};
let fail = WrapperResult::Fail {
    reason: "node failed".to_string(),
};
```

#### `HandlerRegistry`

注册并运行 point/wrapper handlers。

```rust
use agent_core::hook::{HookName, HookPayload, PointHookDecision, WrapperRequest, WrapperResponse, WrapperResult};
use agent_core::hook_handler::HandlerRegistry;

let mut handlers = HandlerRegistry::new();
handlers.register_point(HookName::Input, 0, |_payload: HookPayload| {
    Ok(PointHookDecision::Continue)
})?;
handlers.register_wrapper(HookName::NodeExecution, 0, |request, next| {
    next.run(request)
})?;

let point = handlers.run_point(HookPayload::new(HookName::Input))?;
let wrapped = handlers.run_wrapper(
    WrapperRequest::new(HookName::NodeExecution),
    |_| Ok(WrapperResult::Continue(WrapperResponse::new(serde_json::json!({"ok": true})))),
)?;
```

#### `WrapperNext`

在 wrapper handler 中调用内层服务。

```rust
handlers.register_wrapper(HookName::NodeExecution, 0, |request, next| {
    let result = next.run(request)?;
    Ok(result)
})?;
```

### 读取 Event

调用方：CLI renderer、TUI/HTTP streaming endpoint、trace recorder、测试断言。

#### `CoreEvent`

匹配 core 事件类型。

```rust
use agent_core::event::CoreEvent;

let kind = match event {
    CoreEvent::AgentStarted { .. } => "agent_started",
    CoreEvent::GraphEnded { status, .. } if status == "completed" => "graph_completed",
    CoreEvent::MessageEmitted { .. } => "message_emitted",
    CoreEvent::Error { recoverable, .. } if *recoverable => "recoverable_error",
    _ => "other",
};
```

#### `EventLog`

聚合和读取 core event。

```rust
use agent_core::event::{CoreEvent, EventLog};

let mut log = EventLog::new();
log.push(CoreEvent::HookEmitted {
    name: "tool_execution".to_string(),
    data: serde_json::json!({"tool": "android_ui_state"}),
});
log.extend(result.events.clone());
let events = log.events();
let owned_events = log.into_events();
```

#### `AgentRunResult.events`

读取 agent run 的事件。

```rust
let result = agent.run(input)?;
let agent_event_count = result.events.len();
let message_events = result
    .events
    .iter()
    .filter(|event| matches!(event, CoreEvent::MessageEmitted { .. }))
    .count();
```

#### `GraphRunResult.events`

读取 graph runner 的事件。

```rust
use agent_core::graph_runner::{GraphRunInput, GraphRunner};

let run = GraphRunner::new().run(&graph, GraphRunInput::default())?;
let graph_event_count = run.events.len();
```

### 输入用户信息

调用方：CLI input loop、TUI composer、HTTP message endpoint、测试。

#### `user_input::text_message`

创建纯文本 user message。

```rust
use agent_core::user_input;

let message = user_input::text_message("检查当前手机设置状态")?;
```

#### `user_input::content_blocks_message`

创建多模态 user message。

```rust
use agent_core::content_block::ContentBlock;
use agent_core::user_input;

let message = user_input::content_blocks_message(vec![
    ContentBlock::text("根据截图决定下一步"),
    ContentBlock::image_reference("screen://current", Some("image/png".to_string()), None),
])?;
```

#### `user_input::file_reference_message`

创建文件引用 user message。

```rust
let message = user_input::file_reference_message(
    "file:///tmp/report.json",
    Some("application/json".to_string()),
    Some("report.json".to_string()),
)?;
```

#### `user_input::image_reference_message`

创建图片引用 user message。

```rust
let message = user_input::image_reference_message(
    "screen://current",
    Some("image/png".to_string()),
    Some("current screenshot".to_string()),
)?;
```

#### `user_input::audio_reference_message`

创建音频引用 user message。

```rust
let message = user_input::audio_reference_message(
    "file:///tmp/instruction.wav",
    Some("audio/wav".to_string()),
)?;
```

#### `ContentBlock`

手动构造消息块。

```rust
use agent_core::content_block::{ContentBlock, DiagnosticLevel};

let text = ContentBlock::text("open settings");
let call = ContentBlock::tool_call("call-1", "android_ui_state", serde_json::json!({}));
let diagnostic = ContentBlock::diagnostic(DiagnosticLevel::Info, "screen observed");
```

#### `RunMessage`

构造和标记运行消息。

```rust
use agent_core::run_message::{MessageRole, RunMessage};

let mut streaming = RunMessage::streaming(MessageRole::Assistant);
streaming.push_content(ContentBlock::text("thinking"));
streaming.finalize()?;

let user = RunMessage::user(vec![ContentBlock::text("open settings")])?
    .with_metadata("task.id", serde_json::json!("settings_audit"));
```

#### `DiagnosticLevel`

标记 diagnostic message 的严重级别。

```rust
use agent_core::content_block::{ContentBlock, DiagnosticLevel};
use agent_core::run_message::RunMessage;

let diagnostic = RunMessage::diagnostic(vec![ContentBlock::diagnostic(
    DiagnosticLevel::Warning,
    "retrying node execution",
)])?;
```

### 构建模型上下文

调用方：provider adapter、agent runtime facade、测试 harness。

#### `AssistantBuilder`

增量构造 assistant message。

```rust
use agent_core::assistant_builder::AssistantBuilder;

let mut builder = AssistantBuilder::new();
builder.push_text_delta("I will inspect the screen.");
builder.push_tool_call(
    "call-ui-1",
    "android_ui_state",
    serde_json::json!({}),
);
let assistant = builder.finish()?;
```

#### `ContextBuildInput`

准备构建 provider-neutral request 所需的消息、工具和 metadata。
如果前面用 `ToolRegistry::direct_schemas` 得到了 `direct_tools`，这里才是它真正生效的位置。

```rust
use agent_core::context::ContextBuildInput;

let direct_tools = registry.direct_schemas(&definition);

let mut input = ContextBuildInput::new("mimo-v2.5-pro");
input.run_messages.push(user_message);
input.visible_tool_schemas = direct_tools;
input.metadata.insert(
    "stable_prefix_id".to_string(),
    serde_json::json!(prefix_id),
);
```

#### `ContextBuilder`

把 definition、messages 和 tool schemas 编译成 `LlmRequest`。

```rust
use agent_core::context::ContextBuilder;

let request = ContextBuilder::new().build(&definition, input)?;
```

#### `LlmRequest`

读取或修改 provider-neutral request。

```rust
use agent_core::llm_request::LlmRequest;

let mut request = LlmRequest::new("mimo-v2.5-pro");
request.metadata.insert(
    "route.account".to_string(),
    serde_json::json!("stable-prefix-account"),
);
```

### Session 和 Turn

调用方：会话存储、轨迹回放、turn loop runtime。

#### `TurnLoop`

提交用户消息并准备下一轮执行。

```rust
use agent_core::turn_loop::TurnLoop;

let mut turn_loop = TurnLoop::new();
turn_loop.submit_user_message(user_message)?;
let prepared = turn_loop.prepare_turn()?.expect("turn should be ready");
turn_loop.finish_turn(prepared.turn_id)?;
```

#### `SessionEntry`

把 finalized message 包成 session entry。

```rust
use agent_core::session_entry::SessionEntry;

let header = SessionEntry::header(session_id);
let entry = SessionEntry::message(Some(header.id), user_message)?;
```

#### `InMemorySessionStore`

本地内存 session store。

```rust
use agent_core::session_store::{InMemorySessionStore, SessionStore};

let mut store = InMemorySessionStore::new();
store.append(header)?;
store.append(entry)?;
let tree = store.load_tree()?;
```

#### `SessionStore`

实现自定义 session store。

```rust
use agent_core::session_entry::SessionEntry;
use agent_core::session_store::SessionStore;

struct MySessionStore {
    entries: Vec<SessionEntry>,
}

impl SessionStore for MySessionStore {
    fn append(&mut self, entry: SessionEntry) -> agent_core::AgentCoreResult<()> {
        self.entries.push(entry);
        Ok(())
    }

    fn entries(&self) -> &[SessionEntry] {
        &self.entries
    }
}
```

#### `ReplaySnapshot`

读取 replay 后保留下来的消息和 compaction summary。

```rust
use agent_core::session_replay::ReplaySnapshot;

let snapshot: ReplaySnapshot = replay_active_branch(&tree)?;
let messages = snapshot.messages;
let summaries = snapshot.summaries;
```

#### `replay_active_branch()`

从当前 active branch 回放 session。

```rust
use agent_core::session_replay::replay_active_branch;

let snapshot = replay_active_branch(&tree)?;
```

#### `replay_branch()`

按指定 leaf 回放 session。

```rust
use agent_core::session_replay::replay_branch;

let snapshot = replay_branch(&tree, leaf_id)?;
```

### 终止 Agent Graph

调用方：CLI ctrl-c、TUI stop button、HTTP cancel endpoint、test harness、supervision graph。

#### `Agent::cancel`

设置 agent 级取消标记。

```rust
agent.cancel();
```

#### `Agent::cancellation_token`

读取取消标记。

```rust
let token = agent.cancellation_token();
let is_cancelled = token.is_cancelled();
```

#### `AgentRunInput::with_stop_requested`

对单次 run 请求停止。

```rust
let result = agent.run(
    AgentRunInput::new(graph).with_stop_requested(true),
)?;
assert_eq!(result.status.as_str(), "cancelled");
```

### Error 和 Result

调用方：所有 public API 调用者。

#### `AgentCoreResult`

作为 core public API 的统一返回类型。

```rust
use agent_core::AgentCoreResult;

fn build_definition() -> AgentCoreResult<AgentDefinition> {
    AgentDefinitionBuilder::new()
        .name("agent")
        .system_prompt("prompt")
        .build()
}
```

#### `AgentCoreError`

返回可分类错误。

```rust
use agent_core::AgentCoreError;

fn require_task_id(task_id: &str) -> Result<(), AgentCoreError> {
    if task_id.trim().is_empty() {
        return Err(AgentCoreError::InvalidInput(
            "task id must not be empty".to_string(),
        ));
    }
    Ok(())
}
```

### Mobilerun-like Example 调用 core 的公开入口

本节只写 `example/mobilerun-agent-boundary` 怎样调用 `agent-core`。Android shell、UniFFI、真实移动端 adapter、credential vault 和 provider HTTP runtime 不写在这里。

#### `scripted_agent::run_boundary_probe()`

总入口。它构建 agent definition、注册 tools、生成 context、跑 graph、执行 tools、回放 session、统计概率图，最后返回 `BoundaryReport`。

```rust
let report = scripted_agent::run_boundary_probe()?;
report.print();
```

#### `MobileRunLikeConfig::agent_definition()`

把 Mobilerun-like prompt 和默认 tool visibility 编译成 core `AgentDefinition`。

```rust
let config = MobileRunLikeConfig::boundary_default();
let definition = config.agent_definition()?;
```

#### `MobileRunLikeConfig::agent_definition_with_tool_visibility()`

用外部计算出的 tool visibility 编译 definition。

```rust
let visibility = default_tool_visibility();
let definition = config.agent_definition_with_tool_visibility(visibility)?;
```

#### `MobileRunLikeConfig::render_user_prompt()`

渲染带变量替换的 user prompt。

```rust
let user_prompt = config.render_user_prompt();
let user_message = user_input::text_message(user_prompt)?;
```

#### `MobileRunLikeConfig::render_system_prompt()`

渲染 system prompt，供 `AgentDefinitionBuilder` 使用。

```rust
let system_prompt = config.render_system_prompt();
let definition = AgentDefinitionBuilder::new()
    .name(config.agent_name.clone())
    .system_prompt(system_prompt)
    .build()?;
```

#### `default_tool_visibility()`

生成 Mobilerun-like 默认工具冷热层。

```rust
let visibility = default_tool_visibility();
let ui_state_visibility = visibility.get("ui_state");
```

#### `render_template()`

替换 prompt template 中的自定义变量。

```rust
let rendered = render_template("Open {{app_name}}", &config.variables);
```

#### `register_mobilerun_tools()`

把 Mobilerun-like action tool pack 注册进 core registry。

```rust
let mut registry = ToolRegistry::new();
register_mobilerun_tools(&mut registry)?;
```

#### `expected_action_count()`

读取示例声明的 action tool 数量，用于覆盖率断言。

```rust
let expected = expected_action_count();
assert_eq!(registry.names().len(), expected);
```

#### `TrajectoryProbabilityGraph::from_sessions()`

直接从多条 message session 构建概率图。

```rust
let graph = TrajectoryProbabilityGraph::from_sessions(&sessions);
```

#### `TrajectoryProbabilityGraph::from_replay_snapshots()`

从 session replay 的 snapshot 构建概率图。

```rust
let graph = TrajectoryProbabilityGraph::from_replay_snapshots(&snapshots);
```

#### `TrajectoryProbabilityGraph::observe_session()`

增量加入一条 session。

```rust
let mut graph = TrajectoryProbabilityGraph::default();
graph.observe_session(&messages);
```

#### `TrajectoryProbabilityGraph::likely_next()`

预测某个事件之后的高概率事件。

```rust
let predictions = graph.likely_next("message:assistant", 3);
```

#### `TrajectoryProbabilityGraph::tool_use_probability()`

查询工具在历史 session 中出现的概率。

```rust
let p = graph.tool_use_probability("ui_state");
```

#### `TrajectoryProbabilityGraph::plan_preexecution()`

筛选可预执行工具调用。

```rust
let plan = graph.plan_preexecution(
    &registry,
    &definition,
    candidates,
    0.9,
);
```

#### `TrajectoryProbabilityGraph::execute_preexecution_plan()`

执行预执行计划并转回 tool messages。

```rust
let outcome = TrajectoryProbabilityGraph::execute_preexecution_plan(
    &executor,
    &definition,
    plan,
)?;
```

#### `TrajectoryProbabilityGraph::plan_tool_layers()`

根据历史概率生成 direct/searchable/hidden 分层。

```rust
let layer_plan = graph.plan_tool_layers(&base_visibility, 0.8);
```

#### `TaskContextPackage`

定义 task 自带上下文包。

```rust
let package = TaskContextPackage::new("search_catalog", "Search product")
    .with_inputs(["search_term=wireless charger"])
    .with_artifacts(["first_result=Wireless Charger Stand"])
    .with_required_state(["screen=home"]);
```

#### `TaskNode`

把 task package 放进 DAG 节点并声明依赖。

```rust
let node = TaskNode::new(package).depends_on(["open_app"]);
```

#### `TaskDependencyGraph`

组装 task DAG 并生成当前 task 的 messages。

```rust
let mut dag = TaskDependencyGraph::new();
dag.add_task(TaskNode::new(TaskContextPackage::new("open_app", "Open app")))?;
dag.add_task(node)?;
let context_messages = dag.context_for("search_catalog")?;
let edges = dag.dependency_edges();
```

#### `sample_mobile_task_dag()`

生成示例 DAG。

```rust
let dag = sample_mobile_task_dag()?;
let final_context = dag.context_for("finalize")?;
```

#### `mark_stability()`

给 message 标记稳定性。

```rust
let stable_message = mark_stability(message, ContextStability::StablePrefix);
```

#### `order_by_stability()`

按稳定性排序上下文块。

```rust
let ordered = order_by_stability(messages);
```

#### `stability_order_labels()`

读取排序后的稳定性标签，用于断言和报告。

```rust
let labels = stability_order_labels(&ordered);
```

#### `stable_prefix_id()`

为稳定 prefix 生成路由/cache key。

```rust
let prefix_id = stable_prefix_id(definition.system_prompt(), &direct_tool_names);
```

#### `KeyRoutePlan::new()`

创建 key/account 路由计划。

```rust
let route_plan = KeyRoutePlan::new(prefix_id.clone());
```

#### `KeyRoutePlan::route_for_prefix()`

查看某个 prefix 会走哪个 account/env slot。

```rust
let route = route_plan.route_for_prefix(&prefix_id);
let account = route.account_label.as_str();
```

#### `KeyRoutePlan::apply()`

把路由应用到 provider-neutral `LlmRequest`。

```rust
let routed = route_plan.apply(request, &prefix_id, &resolver)?;
```

#### `StaticSecretResolver::new()`

测试中提供 fake key resolver。

```rust
let resolver = StaticSecretResolver::new([
    ("MOBILERUN_STABLE_PREFIX_API_KEY", "fake-stable-key"),
    ("MOBILERUN_GENERAL_POOL_API_KEY", "fake-general-key"),
]);
```

#### `EnvSecretResolver`

真实运行时从环境变量槽位读取 secret。

```rust
let resolver = EnvSecretResolver;
let routed = route_plan.apply(request, &prefix_id, &resolver)?;
```

#### `run_hundred_step_cross_app_task()`

用 core `ToolExecutor` 执行百步级 boundary probe。

```rust
let report = run_hundred_step_cross_app_task(&executor, &definition)?;
assert!(report.completed);
```

#### `CrossAppTaskReport::detail()`

输出百步任务摘要。

```rust
let detail = report.detail();
```

#### `AppMapMemory::new()`

创建 GUI-Explorer 风格地图记忆。

```rust
let mut memory = AppMapMemory::new();
```

#### `AppMapMemory::observe_ui_state()`

用 UI state 更新地图。

```rust
let update = memory.observe_ui_state(&ui_state_json, Some("settings root"))?;
```

#### `AppMapMemory::local_view()`

读取当前位置附近的局部地图。

```rust
let view = memory.local_view(memory.current_page(), 2, Some("wifi settings"));
```

#### `AppMapMemory::semantic_search()`

按语义目标搜索页面。

```rust
let hits = memory.semantic_search("wifi", 5);
```

#### `AppMapMemory::plan_path()`

规划已知页面之间的路径。

```rust
let path = memory.plan_path("settings_root", "wifi_settings");
```

#### `AppMapMemory::forget()`

按 scope 删除地图记忆。

```rust
let removed = memory.forget(ForgetScope::Stale);
```

#### `sample_cross_app_map()`

生成示例跨 App 地图。

```rust
let memory = sample_cross_app_map()?;
```

#### 回归检查

回归测试覆盖 tool 注册、visibility guard、session replay 概率图、工具预执行、task-bound context、key routing、复杂 graph 和百步级工具任务。逐项功能意义审计见 [18-public-api-meaning-audit](../18-public-api-meaning-audit/README.md)。本文不把这些测试命令作为可运行示例；唯一的 runnable path 是顶部已经跑通的真实 ReAct Android 百步任务。

## 稳定性说明

- Mobilerun-like example 当前依赖的 public API surface 以上一节为准；新增 example 对 core 的调用前，必须同步更新该说明和 `CAPABILITY_MAP.md`。
- provider-specific 类型不能泄漏到 tool API。
- `GraphRunResult.state` 可用于 debug/checkpoint，但外部不要直接可变写入 runtime state。
- 所有 public error 使用 `AgentCoreError`。
- 示例能编译运行才算 API 文档有效。

## 后续拓展方案

- 增加 runtime-level event subscription，而不是只从 run result 读取事件。
- 增加可组合的 node executor registry，让 agent/tool/subgraph node 能按名字路由到不同执行器。
- 把 `HandlerRegistry` 接入 `AgentServices`，让 runner 自动触发生命周期 hook。
- 增加跨进程 tool/provider adapter 和 plugin manifest。
- 增加 background run、remote session store 和 trace viewer。

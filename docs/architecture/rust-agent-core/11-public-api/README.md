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
5. run 层只通过 `TurnLoop` 追加消息并启动 graph。
6. event 层从 turn result 读取 `CoreEvent`，用于 UI、日志和测试断言。
7. session 层只提交 finalized messages，不提交 streaming 中间态。

新增公开入口前必须先回答三件事：

- 它属于七类能力中的哪一类。
- 它是否能复用现有 `AgentDefinition`、`Tool`、`Graph`、`HookName`、`CoreEvent`、`RunMessage`。
- 它是否会让外部调用方直接接触 runtime mutable state；如果会，默认不开放。

## 具体 API 速查

下面所有代码块都是 API 调用片段，不是独立 runnable path。唯一已经跑通的真实 ReAct runnable path 是本文顶部的 Android 百步级 phone-using 任务。正文先放常规外部调用路径；低频扩展点和底层调度接口统一放在文末“高级/低频接口参考”。

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

读取 agent 信息。普通外部执行入口是 `TurnLoop`，不是直接调用 `Agent::run`。

```rust
let agent_id = agent.id();
let definition_name = agent.definition().name();
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

通过 registry、schema 和 permission guard 执行工具。主路径直接拿 `RunMessage(role=tool)`；需要审计原始状态时再读取 `ToolResult`。

```rust
use agent_core::tool_executor::{ToolCall, ToolExecutor};
use std::sync::Arc;

let executor = ToolExecutor::new(Arc::new(registry));
let message = executor.execute_one_message(
    &definition,
    ToolCall::new("call-ui-1", "android_ui_state", serde_json::json!({})),
)?;
let messages = executor.execute_batch_parallel_messages(&definition, calls)?;
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

读取工具执行的原始状态。正常 agent/message 主路径不需要手动处理它，直接用 `ToolExecutor::execute_one_message()` 或 `execute_batch_messages()` 会自动得到 `RunMessage(role=tool)`；只有审计、错误分支判断或底层测试才读取 `ToolResult`。

```rust
use agent_core::tool_result::ToolResult;

let result = ToolResult::success(
    "call-ui-1",
    "android_ui_state",
    serde_json::json!({"screen": "settings"}),
);
assert!(!result.is_error());
let raw_status = result.status;
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

`Graph` 是内容包 graph。初始输入写入虚拟源节点 `input`；edge 从上游 node message log 读取新 message，写入下游 input package。node 不再声明 `start_node`，只要它的 required package items 满足，就可以解锁运行。

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
    ))
    .node(GraphNode::tool(
        "tap_tool",
        "tap",
        "tool_call",
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
    .edge("input_to_agent", "input", ("agent", "context"))
    .edge("agent_to_tool", "agent", ("tap_tool", "calls"))
    .edge("tool_to_agent", "tap_tool", ("agent", "context"))
    .edge("agent_to_final", "agent", ("final", "answer"))
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
);

let agent = GraphNode::agent("manager", "phone_manager", InputPackageSpec::new("task"));

let final_node = GraphNode::final_node("final", InputPackageSpec::new("answer"));
```

#### `GraphEdge`

edge 只做内容传输：从 `source_node` 的 message log 读取新 message，写入 `(target_node, input_package)`。分流和解锁都靠目标 package 的 required/optional item query。

```rust
use agent_core::graph_edge::GraphEdge;

let edge = GraphEdge::new(
    "agent_tool_calls_to_tap",
    "agent",
    ("tap_tool", "calls"),
);

assert_eq!(edge.from(), "agent");
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

#### `NodeResult`

node executor 返回一组消息。多个 edge 可以从同一个 source node fan-out；不同下游要继承哪些消息，由各自 input package 的 `MessageQuery` 决定。

```rust
use agent_core::graph_node::NodeResult;

let result = NodeResult::new()
    .with_message(assistant_tool_call_message)
    .with_message(assistant_final_message);
```

#### `NodeConcurrency`

控制同一个 node 的 activation 并发。`Serial` 保证同一 node 一次只跑一个 activation；`Parallel` 允许多个 activation 同时跑；`ByKey` 按某个 package item 的 selected hash 分组限流。

```rust
use agent_core::graph_node::{ConcurrencyKey, GraphNode, NodeConcurrency};

let node = GraphNode::tool("lookup_catalog", "search_database", "task", package)
    .concurrency(NodeConcurrency::ByKey {
        key: ConcurrencyKey::package_item("task"),
        max_per_key: 1,
    });
```

#### `TurnLoop`

graph 的常规执行入口。应用层只需要把历史消息 append 到 loop，再把本轮 user message 交给 loop；loop 会按上下文策略构建 graph 启动快照、执行 graph，并只把 graph 新产生的消息追加回历史。

```rust
use std::sync::Arc;
use agent_core::turn_loop::{TurnContextPolicy, TurnLoop};

let mut turn_loop = TurnLoop::with_executor(Arc::new(RuntimeExecutor))
    .with_graph(graph)
    .with_context_policy(TurnContextPolicy::FullHistory)
    .with_max_ticks(10_000);

turn_loop.append_messages(replayed_messages)?;
let turn = turn_loop.run_message(user_message)?;

assert_eq!(turn.graph.status.as_str(), "completed");
let graph_context = turn.context_messages;
let new_messages = turn.appended_messages;
let full_history = turn_loop.messages();
```

#### `TurnLoop::push`

Eino-style 输入入口。`push` 只把 finalized/aborted message 放入 loop buffer，不直接执行 graph；已 stopped 的 loop 会把消息记录到 late items 并返回 `false`。

```rust
let accepted = turn_loop.push(user_message)?;
assert!(accepted);
assert_eq!(turn_loop.pending_len(), 1);
```

#### `TurnLoop::run_once`

执行一轮 turn。内部顺序是 `GenInput -> PrepareGraph -> GraphRunner -> OnTurnEvents -> append new messages`。

```rust
if let Some(turn) = turn_loop.run_once()? {
    assert_eq!(turn.graph.status.as_str(), "completed");
    let consumed = turn.consumed;
    let still_pending = turn_loop.pending_len();
}
```

#### `TurnLoop::run_pending`

持续消费 buffer，直到没有 pending item、stop 被请求，或 `GenInput` 没有产出可执行 turn。

```rust
turn_loop.push(first_user_message)?;
turn_loop.push(second_user_message)?;

let turns = turn_loop.run_pending()?;
let history = turn_loop.messages();
```

#### `TurnGenInputFn`

自定义每轮输入构建。它拿到 pending items、历史消息和 context policy，返回本轮 graph input、已消费 item id 和需要留给下一轮的 items。

```rust
use std::sync::Arc;
use agent_core::turn_loop::{TurnGenInputFn, TurnGenInputResult};

let gen_input: TurnGenInputFn = Arc::new(|input| {
    let first = input.pending_items[0].clone();
    let remaining = input.pending_items.iter().skip(1).cloned().collect();

    Ok(TurnGenInputResult::new(vec![first.clone()], vec![first.id])
        .with_remaining(remaining))
});

turn_loop.set_gen_input(gen_input);
```

#### `TurnPrepareGraphFn`

自定义每轮 graph 选择，等价于 Eino 的 `PrepareAgent`。固定 graph 的常规路径用 `with_graph(graph)` 即可。

```rust
use std::sync::Arc;
use agent_core::turn_loop::TurnPrepareGraphFn;

let prepare_graph: TurnPrepareGraphFn = Arc::new(|turn| {
    if turn.consumed[0].metadata.contains_key("reasoning") {
        Ok(reasoning_graph.clone())
    } else {
        Ok(fast_graph.clone())
    }
});

turn_loop.set_prepare_graph(prepare_graph);
```

#### `TurnEventHandlerFn`

消费每轮 graph 事件和新追加消息，等价于 Eino 的 `OnAgentEvents`。

```rust
use std::sync::Arc;
use agent_core::turn_loop::TurnEventHandlerFn;

let on_events: TurnEventHandlerFn = Arc::new(|batch| {
    let event_count = batch.events.len();
    let new_message_count = batch.appended_messages.len();
    Ok(())
});

turn_loop.set_on_turn_events(on_events);
```

#### `TurnContextPolicy`

规定每次 graph 启动时从 loop 历史中继承哪些消息。当前 user message 永远作为最后一条进入 graph context。

```rust
use agent_core::turn_loop::TurnContextPolicy;

let all_history = TurnContextPolicy::FullHistory;
let latest_user_only = TurnContextPolicy::LatestUserOnly;
let short_memory = TurnContextPolicy::LastMessages(24);
```

graph 启动上下文规则：

- 先从 `TurnLoop::messages()` 按 `TurnContextPolicy` 选择历史消息。
- 默认 `GenInput` 会把 pending items 全部消费，并把它们追加到 context 最后。
- 这组 `context_messages` 是本次 graph run 的不可变快照；graph 内部只通过 edge/package/query 继续筛选。
- graph 完成后，`TurnLoop` 只追加 `context_messages` 之外的新 message id；被 edge/query 筛掉的内容不会作为新消息追加或触发下游包版本。
- 工具调用、工具结果、reasoning、diagnostic 都仍然是 `RunMessage.content` 里的 block，不需要额外 payload。

#### `TurnLoop::append_messages`

把外部已经确认的 finalized message 放回 loop 历史，常用于 session replay、人工注入上下文或恢复断点。重复的 message id 会被忽略。

```rust
turn_loop.append_messages([
    restored_user_message,
    restored_tool_result,
])?;
```

#### `TurnLoop::late_items`

stop 之后 push 进来的消息不会丢失，会进入 late items，外层可以取走并转交新的 loop。

```rust
turn_loop.request_stop();
let accepted = turn_loop.push(late_user_message)?;
assert!(!accepted);

let late = turn_loop.take_late_items();
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
log.extend(turn.graph.events.clone());
let events = log.events();
let owned_events = log.into_events();
```

#### `TurnRunResult.graph.events`

读取本轮 graph run 的事件。

```rust
let turn = turn_loop.run_message(user_message)?;
let graph_event_count = turn.graph.events.len();
let message_events = turn
    .graph
    .events
    .iter()
    .filter(|event| matches!(event, CoreEvent::MessageEmitted { .. }))
    .count();
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

手动构造 message 内部块。普通文本、assistant tool call 和 tool result 都已经有自动包装入口；只有多模态输入、diagnostic、自定义块或精确字段筛选时才需要直接碰 `ContentBlock`。

```rust
use agent_core::content_block::{ContentBlock, DiagnosticLevel};

let text = ContentBlock::text("open settings");
let image = ContentBlock::image_reference(
    "screen://current",
    Some("image/png".to_string()),
    Some("current screen".to_string()),
);
let diagnostic = ContentBlock::diagnostic(DiagnosticLevel::Info, "screen observed");
```

#### `RunMessage`

构造和标记运行消息。对外优先传 `RunMessage`，不要自己拼 provider-specific item；provider adapter 会在最后一跳把 message content blocks 转成具体协议。

```rust
use agent_core::run_message::RunMessage;
use agent_core::tool_executor::ToolCall;

let user = RunMessage::user_text("open settings")?
    .with_metadata("task.id", serde_json::json!("settings_audit"));

let assistant = RunMessage::assistant_tool_call(
    "call-ui-1",
    "android_ui_state",
    serde_json::json!({}),
)?;

let tool = executor.execute_one_message(
    &definition,
    ToolCall::new("call-ui-1", "android_ui_state", serde_json::json!({})),
)?;
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

#### `TurnLoop::request_stop`

阻止新的 turn 启动。已进入底层 runtime 的取消属于高级执行器集成，常规调用面只操作 turn loop。

```rust
turn_loop.request_stop();
assert!(turn_loop.stop_requested());
assert!(turn_loop.run_message(user_message).is_err());
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

## 高级/低频接口参考

本节放不属于常规外部调用主路径的接口。它们仍然是 public API，但主要给真实能力接入、provider adapter、hook 扩展、session/replay runtime 和底层调度测试使用。

### Node 执行扩展

#### `NodeExecutor`

真实能力接入点。`TurnLoop` 运行 graph 时会经由内部 runner/runtime 调用这个接口；tool node、agent node、subgraph node、final node 都通过同一个 async executor 接口调度。`NodeInput` 的主读取方式是 `input.messages("item_name")`，拿到的是被 edge/query 筛选后的 `RunMessage`。只有需要接入真实工具、child agent、provider 或子图执行时，外部系统才需要实现它。

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
    ) -> BoxFuture<'static, AgentCoreResult<rt::NodeResult>> {
        Box::pin(async move {
            let _turn_messages = input.messages("turn");
            match node.kind {
                rt::NodeKind::Agent(agent) => run_agent_node(agent, input, ctx).await,
                rt::NodeKind::Tool(tool) => run_tool_node(tool, input, ctx).await,
                rt::NodeKind::Final => Ok(rt::NodeResult::new()),
                _ => Ok(rt::NodeResult::new()),
            }
        })
    }
}
```

#### `GraphRuntime`

高级可选接口。普通外部调用走 `TurnLoop`，不需要直接接触它。只有在调用方本身就是 async 宿主、需要避免同步 `block_on`，或者要做 runtime 级测试/调度集成时，才直接使用 `GraphRuntime`。

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

准备构建 provider-neutral request 所需的 messages、工具和 metadata。这里直接放 `RunMessage`；不需要把上下文块拆成 provider item，也不需要区分 replay/run 两套消息。
如果前面用 `ToolRegistry::direct_schemas` 得到了 `direct_tools`，这里才是它真正生效的位置。

```rust
use agent_core::context::ContextBuildInput;

let direct_tools = registry.direct_schemas(&definition);

let mut input = ContextBuildInput::new("mimo-v2.5-pro");
input.extend_messages([user_message, assistant_message, tool_message]);
input.visible_tool_schemas = direct_tools;
input.metadata.insert(
    "stable_prefix_id".to_string(),
    serde_json::json!(prefix_id),
);
```

#### `ContextBuilder`

把 definition、messages 和 tool schemas 编译成 message-first 的 `LlmRequest`。它只做上下文装配和过滤，消息本身仍然是 `RunMessage`。

```rust
use agent_core::context::ContextBuilder;

let request = ContextBuilder::new().build(&definition, input)?;
assert_eq!(request.messages.len(), 3);
```

#### `LlmRequest`

读取或修改 provider-neutral request。`LlmRequest.messages` 保存 `RunMessage`，provider adapter 在最后一跳把 message content blocks 序列化为 OpenAI/DeepSeek 等具体协议字段。

```rust
use agent_core::llm_request::LlmRequest;

let mut request = LlmRequest::new("mimo-v2.5-pro");
request.push_message(user_message);
request.metadata.insert(
    "route.account".to_string(),
    serde_json::json!("stable-prefix-account"),
);
```

### Session 和 Turn

调用方：会话存储、轨迹回放、turn loop runtime。

#### `TurnLoop`

会话层使用 Eino-style turn loop：恢复历史用 `append_messages`，新输入用 `push`，执行用 `run_once` 或 `run_pending`。单条用户消息可以用 `run_message` 作为 `push + run_once` 的便捷路径。

```rust
use agent_core::turn_loop::{TurnContextPolicy, TurnLoop};

let mut turn_loop = TurnLoop::new()
    .with_graph(graph)
    .with_context_policy(TurnContextPolicy::LastMessages(32));

turn_loop.append_messages(replay.messages)?;
turn_loop.push(user_message)?;
let turn = turn_loop.run_once()?.expect("turn should run");
let history_after_turn = turn_loop.messages();
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

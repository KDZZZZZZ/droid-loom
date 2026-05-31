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

外部调用方包括 CLI、TUI、HTTP server、测试 harness、tool pack、plugin adapter 和后续多 agent 编排层。外部代码不直接写 `GraphState`、不直接改 session tree 内部结构、不绕过 registry 调用 tool。

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

### 定义 Agent

调用方：配置加载器、CLI preset、测试构造器、多 agent 编排器。

公开入口：

- `agent_core::agent_definition::AgentDefinitionBuilder`
- `agent_core::agent_definition::AgentDefinition`
- `agent_core::agent_definition::ToolVisibility`
- `agent_core::agent_factory::AgentFactory`
- `agent_core::agent::Agent`

调用说明：

- `AgentDefinition` 只保存 `name`、`system_prompt`、`tool_visibility`。
- `system_prompt` 是用户配置，不属于运行时 mutable state。
- `ToolVisibility::Direct` 表示常用工具直接暴露给模型。
- `ToolVisibility::Searchable` 表示不直接暴露，搜索或显式选择后才显示。
- `ToolVisibility::Hidden` 表示不加载、不搜索、不执行。
- `AgentFactory` 只负责校验 definition 和装配 services，不解析 tool schema，不构建 provider request，不写 session。
- 同一个 `AgentDefinition` 可以创建多个 `Agent`，每个 `Agent` 有独立取消标记。
- 调用顺序是 `AgentDefinitionBuilder::new()` 设置 `name/system_prompt/tool_visibility`，再 `build()` 得到 `AgentDefinition`，最后交给 `AgentFactory::create()` 创建 `Agent`。

调用示例（片段）：

```rust
use agent_core::agent_definition::{AgentDefinitionBuilder, ToolVisibility};
use agent_core::AgentFactory;

let definition = AgentDefinitionBuilder::new()
    .name("mobile_fast_agent")
    .system_prompt("Use tools for every phone action.")
    .tool_visibility("android_ui_state", ToolVisibility::Direct)
    .tool_visibility("android_map_search", ToolVisibility::Searchable)
    .tool_visibility("raw_adb_shell", ToolVisibility::Hidden)
    .build()?;

let agent = AgentFactory::default().create(definition.clone())?;
let agent_name = agent.definition().name();
```

### 定义 Tool

调用方：tool pack、MCP adapter、plugin adapter、测试 mock tool。

公开入口：

- `agent_core::tool::Tool`
- `agent_core::tool::ToolMetadata`
- `agent_core::tool::ToolInvocation`
- `agent_core::tool::ToolOutput`
- `agent_core::tool_schema::ToolSchema`
- `agent_core::tool_registry::ToolRegistry`
- `agent_core::tool_executor::ToolExecutor`
- `agent_core::tool_executor::ToolCall`
- `agent_core::tool_result::ToolResult`

调用说明：

- core 不内置具体工具，只定义 trait、schema、registry、permission、executor 和 result mapping。
- tool pack 实现 `Tool::metadata()` 和 `Tool::invoke()`。
- `ToolRegistry` 根据 `AgentDefinition.tool_visibility()` 解析工具可见性。
- `ToolRegistry::direct_schemas()` 只返回 direct 工具 schema。
- `ToolRegistry::search()` 只搜索 searchable 工具。
- `ToolExecutor::execute_one()` 校验可见性、参数 schema 和权限后调用 tool。
- `ToolExecutor::execute_batch_parallel()` 可并行执行彼此独立的 tool calls，并保持返回结果顺序。
- `ToolResult::into_run_message()` 把工具结果包装成 `RunMessage(role=tool)`。
- tool 不直接写 session，不直接生成 provider-specific payload。
- 调用顺序是先用 `ToolSchema::new()` 定义输入，`ToolMetadata::new()` 绑定默认可见性，tool pack 实现 `Tool` 后注册到 `ToolRegistry`，再由 `ToolExecutor::execute_one()` 或 `execute_batch_parallel()` 执行。
- 工具结果进入模型上下文前调用 `ToolResult::into_run_message()`，统一变成 `RunMessage(role=tool)`。
- 只读且幂等的工具可以由 runtime 根据轨迹概率提前执行；判断条件统一走 `ToolMetadata::can_preexecute()`，即 `read_only + idempotent + !destructive`。

调用示例（片段）：

```rust
use agent_core::tool::{Tool, ToolInvocation, ToolMetadata, ToolOutput};
use agent_core::tool_executor::{ToolCall, ToolExecutor};
use agent_core::tool_registry::ToolRegistry;
use agent_core::tool_schema::ToolSchema;
use agent_core::{AgentCoreResult, ToolVisibility};
use serde_json::json;
use std::sync::Arc;

#[derive(Debug)]
struct UiStateTool {
    metadata: ToolMetadata,
}

impl UiStateTool {
    fn new() -> AgentCoreResult<Self> {
        let schema = ToolSchema::empty_object(
            "android_ui_state",
            "Read the current screen through the platform adapter.",
        )?;
        let mut metadata = ToolMetadata::new(schema, ToolVisibility::Direct);
        metadata.capabilities.read_only = true;
        metadata.capabilities.idempotent = true;
        Ok(Self { metadata })
    }
}

impl Tool for UiStateTool {
    fn metadata(&self) -> &ToolMetadata {
        &self.metadata
    }

    fn invoke(&self, invocation: ToolInvocation) -> AgentCoreResult<ToolOutput> {
        Ok(ToolOutput::new(json!({
            "call_id": invocation.call_id,
            "screen": "settings"
        })))
    }
}

let mut registry = ToolRegistry::new();
registry.register(UiStateTool::new()?)?;
let executor = ToolExecutor::new(Arc::new(registry));

let result = executor.execute_one(
    &definition,
    ToolCall::new("call-ui-1", "android_ui_state", json!({})),
)?;
let tool_message = result.into_run_message()?;
```

### 构建 Graph

调用方：默认 ReAct graph provider、多 agent 编排器、测试 harness、后续高级用户。

公开入口：

- `agent_core::graph::Graph`
- `agent_core::graph_node::GraphNode`
- `agent_core::graph_node::GraphNodeAction`
- `agent_core::graph_edge::GraphEdge`
- `agent_core::graph_edge::ActivationCondition`
- `agent_core::graph_edge::ContextInheritPolicy`
- `agent_core::graph_state::GraphStateBudget`
- `agent_core::agent::AgentRunInput`

调用说明：

- `Graph` 是一次 agent run 的编排模板，不是会话级 turn loop。
- node 可以实时 emit `RunMessage`。
- runner 每收到一个 node message，就立即检查该 node 的出边。
- edge 只读取 source node 本次 message 和只读 `GraphStateView`，返回睡眠或激活。
- 第一版 edge 只保留 `ContextInheritPolicy::Full`。
- summary、selected、isolated 由上游 node output 或 target node 显式 input 构造，不作为 edge primitive。
- 最大循环数、预算和停止请求由 `GraphRunner` 管理，不由单条 edge 管理。
- 调用顺序是用 `Graph::builder()` 添加 `GraphNode`、`GraphEdge`、start/end node 和 budget，`build()` 后放入 `AgentRunInput`，再由 `Agent::run()` 或 `GraphRunner::run()` 执行。

调用示例（片段）：

```rust
use agent_core::agent::AgentRunInput;
use agent_core::content_block::ContentBlock;
use agent_core::graph::Graph;
use agent_core::graph_edge::{ActivationCondition, GraphEdge};
use agent_core::graph_node::{GraphNode, GraphNodeAction};
use agent_core::graph_state::GraphStateBudget;
use agent_core::run_message::RunMessage;

let assistant_message = RunMessage::assistant(vec![
    ContentBlock::tool_call("call-ui-1", "android_ui_state", serde_json::json!({})),
])?;

let graph = Graph::builder("single_react_turn")
    .node(GraphNode::new("input").with_action(GraphNodeAction::PassthroughInput))
    .node(GraphNode::new("react_loop").with_action(GraphNodeAction::EmitMessages(vec![
        assistant_message,
    ])))
    .start_node("input")
    .edge(
        GraphEdge::new("input_to_react", "input", "react_loop")
            .with_activation_condition(ActivationCondition::MessageHasText),
    )
    .budget(GraphStateBudget {
        max_total_node_executions: Some(128),
        max_node_executions: Some(128),
        max_no_progress_ticks: None,
    })
    .build()?;

let run_input = AgentRunInput::new(graph).with_initial_messages(vec![user_message]);
let result = agent.run(run_input)?;
```

### 挂 Hook Handler

调用方：extension、permission policy、audit/logging、recoverable error policy、测试 handler。

公开入口：

- `agent_core::hook::HookName`
- `agent_core::hook::HookPayload`
- `agent_core::hook::PointHookDecision`
- `agent_core::hook::WrapperRequest`
- `agent_core::hook::WrapperResponse`
- `agent_core::hook::WrapperResult`
- `agent_core::hook_handler::HandlerRegistry`
- `agent_core::hook_handler::WrapperNext`

调用说明：

- hook 是生命周期点，handler 是被 hook 调用的函数。
- point handler 适合 input、context、tool result 等离散点，返回 `Continue`、`Rewrite`、`Block`、`Stop` 或 `Emit`。
- wrapper handler 是 middleware，包住一段可执行行为，能调用 `next.run(request)`、改写请求、重试、恢复、停止或失败。
- 可恢复错误是一种 wrapper handler 行为，用 `WrapperResult::Recover { messages }` 把修复消息交回主流程。
- 当前版本 `HandlerRegistry` 提供可运行的 handler 链 API；GraphRunner 还没有自动读取 registry，运行时自动挂载属于下一阶段集成点。
- 调用顺序是创建 `HandlerRegistry`，用 `register_point()` 挂离散 hook，用 `register_wrapper()` 挂 middleware；wrapper 内部通过 `WrapperNext::run()` 继续执行内层服务。

调用示例（片段）：

```rust
use agent_core::hook::{
    HookName, HookPayload, PointHookDecision, WrapperRequest, WrapperResponse, WrapperResult,
};
use agent_core::hook_handler::HandlerRegistry;
use agent_core::content_block::{ContentBlock, DiagnosticLevel};
use agent_core::run_message::RunMessage;
use serde_json::json;

let mut handlers = HandlerRegistry::new();

handlers.register_point(HookName::Input, 0, |payload: HookPayload| {
    let payload = payload.with_metadata("seen_by", json!("input_guard"));
    Ok(PointHookDecision::Rewrite(payload))
})?;

handlers.register_wrapper(
    HookName::NodeExecution,
    0,
    |request: WrapperRequest, next| {
        let result = next.run(request)?;
        Ok(match result {
            WrapperResult::Continue(response) => WrapperResult::Continue(response),
            WrapperResult::Fail { reason } => WrapperResult::Recover {
                messages: vec![RunMessage::diagnostic(vec![ContentBlock::diagnostic(
                    DiagnosticLevel::Warning,
                    reason,
                )])?],
            },
            other => other,
        })
    },
)?;

let outcome = handlers.run_point(HookPayload::new(HookName::Input))?;
let wrapper_result = handlers.run_wrapper(
    WrapperRequest::new(HookName::NodeExecution),
    |_| Ok(WrapperResult::Continue(WrapperResponse::new(json!({"ok": true})))),
)?;
```

### 读取 Event

调用方：CLI renderer、TUI/HTTP streaming endpoint、trace recorder、测试断言。

公开入口：

- `agent_core::event::CoreEvent`
- `agent_core::agent::AgentRunResult.events`
- `agent_core::graph_runner::GraphRunResult.events`

调用说明：

- event 是观察流，不是 session entry。
- 第一版 in-process API 从 run result 读取事件。
- `CoreEvent` 覆盖 agent start/end、graph start/end、node start/end、message emitted、hook emitted、error。
- UI 不通过 event 反向修改 runtime；需要改变行为时注册 handler。
- 调用方式是从 `AgentRunResult.events` 或 `GraphRunResult.events` 读取 `CoreEvent`，再投影到 UI、trace、日志或测试断言。

调用示例（片段）：

```rust
use agent_core::event::{CoreEvent, EventLog};

let result = agent.run(input)?;
let mut log = EventLog::new();
log.extend(result.events.clone());

let emitted_message_count = log
    .events()
    .iter()
    .filter(|event| matches!(event, CoreEvent::MessageEmitted { .. }))
    .count();
```

### 输入用户信息

调用方：CLI input loop、TUI composer、HTTP message endpoint、测试。

公开入口：

- `agent_core::user_input::text_message`
- `agent_core::user_input::content_blocks_message`
- `agent_core::user_input::file_reference_message`
- `agent_core::user_input::image_reference_message`
- `agent_core::user_input::audio_reference_message`
- `agent_core::content_block::ContentBlock`
- `agent_core::run_message::RunMessage`

调用说明：

- input 层只负责按模态选择 `ContentBlock` 并包装 user message。
- input 层不排队、不抢占、不写 session、不构造 provider request。
- 常规应用把 user message 提交给 `TurnLoop` 和 session；测试或无持久化场景可以直接放进 `AgentRunInput`。
- 调用方式是根据输入模态选择 `user_input::*_message()` helper；这些 helper 只产出 finalized `RunMessage(role=user)`。

调用示例（片段）：

```rust
use agent_core::content_block::ContentBlock;
use agent_core::user_input;

let text = user_input::text_message("检查当前手机设置状态")?;
let image = user_input::image_reference_message(
    "screen://current",
    Some("image/png".to_string()),
    Some("current screenshot".to_string()),
)?;
let mixed = user_input::content_blocks_message(vec![
    ContentBlock::text("根据截图决定下一步"),
    ContentBlock::image_reference(
        "screen://current",
        Some("image/png".to_string()),
        Some("current screenshot".to_string()),
    ),
])?;
```

### 终止 Agent Graph

调用方：CLI ctrl-c、TUI stop button、HTTP cancel endpoint、test harness、supervision graph。

公开入口：

- `agent_core::agent::Agent::cancel`
- `agent_core::agent::Agent::cancellation_token`
- `agent_core::agent::AgentRunInput::with_stop_requested`

调用说明：

- 停止请求进入 `Agent` 或 run input，不通过 edge 表达。
- runner 在安全点观察停止标记。
- 已 finalized messages 可以提交 session；未 finalized streaming message 如何记录 aborted entry 由 session policy 决定。
- 调用方式是通过 `Agent::cancel()` 设置 agent 级取消标记，或用 `AgentRunInput::with_stop_requested(true)` 对单次 run 请求停止；观察状态用 `Agent::cancellation_token()`。

调用示例（片段）：

```rust
let token = agent.cancellation_token();
assert!(!token.is_cancelled());

agent.cancel();
assert!(token.is_cancelled());

let cancelled = agent.run(
    AgentRunInput::new(cancel_graph).with_stop_requested(true),
)?;
assert_eq!(cancelled.status.as_str(), "cancelled");
```

### Mobilerun-like Example 调用 core 的公开入口

本节只写 `example/mobilerun-agent-boundary` 怎样调用 `agent-core`。Android shell、UniFFI、真实移动端 adapter、credential vault 和 provider HTTP runtime 不写在这里。

#### 入口总览

| example 入口 | 调用的 core API | 用法 |
| --- | --- | --- |
| `scripted_agent::run_boundary_probe()` | 几乎所有下列 core API | 总入口。构建 agent definition、注册 tools、生成 context、跑 graph、执行 tools、回放 session、统计概率图，最后返回 `BoundaryReport`。 |
| `MobileRunLikeConfig::agent_definition()` | `AgentDefinitionBuilder`、`AgentDefinition`、`ToolVisibility` | 把 Mobilerun-like prompt 和 tool visibility 编译成 core agent definition。 |
| `register_mobilerun_tools()` | `Tool`、`ToolMetadata`、`ToolSchema`、`ToolRegistry` | 把 action tool pack 注册进 core registry。 |
| `TrajectoryProbabilityGraph::*` | `RunMessage`、`ReplaySnapshot`、`ToolExecutor`、`ToolRegistry` | 从 session message 轨迹建概率图，并驱动工具预执行和冷热分层。 |
| `TaskDependencyGraph::context_for()` | `RunMessage`、`ContentBlock`、`AgentCoreResult` | 为单个 task 生成上下文包，只继承依赖 artifact。 |
| `KeyRoutePlan::apply()` | `LlmRequest`、`AgentCoreResult` | 给 provider-neutral request 选择 account/env slot，不持久化真实 key。 |
| `run_hundred_step_cross_app_task()` | `AgentDefinition`、`ToolExecutor`、`ToolCall`、`ToolResultStatus` | 用 core tool executor 执行百步级模拟跨 App 工具任务。 |

#### 1. 定义 agent

入口：`MobileRunLikeConfig::agent_definition()` 和 `agent_definition_with_tool_visibility()`。

调用方式：

```rust
let config = MobileRunLikeConfig::boundary_default();
let definition = config.agent_definition()?;
let reasoning = config.agent_definition_with_tool_visibility(default_tool_visibility())?;
```

它内部只调用：

- `AgentDefinitionBuilder::new()`
- `.name(...)`
- `.system_prompt(...)`
- `.tool_visibility(tool_name, visibility)`
- `.build()?`

输出是 `AgentDefinition`。后续 `ToolRegistry`、`ToolExecutor`、`ContextBuilder` 和 `AgentFactory` 都读取这个 definition，不直接改 runtime state。

#### 2. 注册工具

入口：`register_mobilerun_tools(registry: &mut ToolRegistry)`。

调用方式：

```rust
let mut registry = ToolRegistry::new();
register_mobilerun_tools(&mut registry)?;
let registry = Arc::new(registry);
```

每个 Mobilerun-like action tool 都实现 core 的 `Tool` trait：

```rust
impl Tool for MockMobileTool {
    fn metadata(&self) -> &ToolMetadata { ... }
    fn invoke(&self, invocation: ToolInvocation) -> AgentCoreResult<ToolOutput> { ... }
}
```

工具 schema 用 `ToolSchema::new(...)` 创建，工具元数据用 `ToolMetadata::new(schema, default_visibility)` 创建。示例不把真实 Android/iOS/browser 操作放进 core；真实平台能力应由外部 adapter 实现同样的 `Tool` 边界。

#### 3. 执行工具

入口：`ToolExecutor` 在 `scripted_agent.rs` 和 `cross_app_task.rs` 中使用。

调用方式：

```rust
let executor = ToolExecutor::new(registry.clone());
let result = executor.execute_one(
    &definition,
    ToolCall::new("call-1", "ui_state", json!({})),
)?;
let message = result.into_run_message()?;
```

批量只读工具预执行使用：

```rust
let results = executor.execute_batch_parallel(&definition, calls)?;
```

执行前必须经过 `ToolRegistry::get_for_agent()` 的 visibility guard、schema validation 和 permission policy。example 不直接持有 tool 实例并绕过 executor。

#### 4. 构建模型上下文

入口：`ContextBuilder::build(...)`。

调用方式：

```rust
let mut input = ContextBuildInput::new("mimo-v2.5-pro");
input.run_messages = messages;
input.visible_tool_schemas = registry.direct_schemas(&definition);
let request = ContextBuilder::new().build(&definition, input)?;
```

输出是 provider-neutral `LlmRequest`。example 只检查 request 形状和 token/context 策略，不在这里发真实 provider 请求。

#### 5. 表达 graph

入口：`Graph::builder(...)`、`GraphNode`、`GraphEdge`、`GraphRunner`。

调用方式：

```rust
let graph = Graph::builder("mobilerun_fast_turn")
    .node(GraphNode::new("input").with_action(GraphNodeAction::PassthroughInput))
    .node(GraphNode::new("agent_turn").with_action(GraphNodeAction::EmitMessages(vec![assistant])))
    .start_node("input")
    .edge(GraphEdge::new("input_to_agent", "input", "agent_turn")
        .with_activation_condition(ActivationCondition::MessageHasText))
    .build()?;

let run = GraphRunner::new().run(&graph, GraphRunInput::default())?;
```

Reasoning manager/executor、复杂 DAG、cancel 和 budget guard 都用同一组 graph API 表达。`GraphStateBudget` 只作为 runner budget 输入，不由 example 直接可变写入 `GraphState`。

#### 6. 处理 hook

入口：`HandlerRegistry`。

调用方式：

```rust
let mut handlers = HandlerRegistry::new();
handlers.register_point(HookName::Input, 0, |payload| {
    Ok(PointHookDecision::Continue)
})?;

handlers.register_wrapper(HookName::NodeExecution, 0, |request, next| {
    next.run(request)
})?;
```

可恢复错误用 `WrapperResult::Recover { messages }` 表达。example 不把 recover policy 编码成 tool output 字符串。

#### 7. 读取 event

入口：`CoreEvent` 和 `EventLog`。

调用方式：

```rust
let result = agent.run(input)?;
let mut log = EventLog::new();
log.extend(result.events.clone());
```

example 只把 core events 投影成 Mobilerun-style event stream。event 是观察输出，不反向修改 graph、session 或 tool runtime。

#### 8. 管理用户输入 turn

入口：`user_input` 和 `TurnLoop`。

调用方式：

```rust
let message = user_input::text_message("open the app")?;
turn_loop.submit_user_message(message)?;
let prepared = turn_loop.prepare_next_turn()?;
```

输入先变成 finalized `RunMessage(role=user)`，再进入会话级 turn queue。

#### 9. 回放 session

入口：`SessionEntry`、`InMemorySessionStore`、`SessionStore`、`replay_active_branch()`。

调用方式：

```rust
let mut store = InMemorySessionStore::new();
store.append(SessionEntry::header(session_id))?;
store.append(SessionEntry::message(Some(parent_id), message)?)?;
let tree = store.load_tree()?;
let snapshot = replay_active_branch(&tree)?;
```

概率图只读取 `ReplaySnapshot.messages` 中的 finalized `RunMessage`。它不重放 provider、tool 或 graph 内部状态。

#### 10. 统计概率图和预执行

入口：`TrajectoryProbabilityGraph`。

调用方式：

```rust
let graph = TrajectoryProbabilityGraph::from_replay_snapshots(&snapshots);
let next = graph.likely_next("message:assistant", 3);
let plan = graph.plan_preexecution(&registry, &definition, candidates, 0.9);
let outcome = TrajectoryProbabilityGraph::execute_preexecution_plan(
    &executor,
    &definition,
    plan,
)?;
```

`plan_preexecution()` 只允许 `ToolMetadata::can_preexecute()` 为 true 的工具进入计划，也就是 `read_only + idempotent + !destructive`。执行结果会转回 `RunMessage(role=tool)`，继续作为普通轨迹消息使用。

#### 11. 绑定 task 上下文

入口：`TaskContextPackage`、`TaskNode`、`TaskDependencyGraph`。

调用方式：

```rust
let mut graph = TaskDependencyGraph::new();
graph.add_task(TaskNode::new(TaskContextPackage::new("open_app", "Open app")))?;
let messages = graph.context_for("open_app")?;
```

`context_for(task_id)` 返回 `Vec<RunMessage>`。子任务只继承依赖任务的 artifact，不继承中间 tool call、失败尝试或临时屏幕状态。

#### 12. 排列上下文稳定性

入口：`mark_stability()`、`order_by_stability()`、`stable_prefix_id()`。

调用方式：

```rust
let stable = mark_stability(message, ContextStability::StablePrefix);
let ordered = order_by_stability(vec![stable, volatile]);
let prefix_id = stable_prefix_id(definition.system_prompt(), &direct_tool_names);
```

排序只写 `RunMessage.metadata["context.stability"]`，不访问 core 内部 cache。稳定 prefix id 可交给 key/account routing 使用。

#### 13. 路由 key/account

入口：`KeyRoutePlan::apply()`。

调用方式：

```rust
let plan = KeyRoutePlan::new(stable_prefix_id);
let routed = plan.apply(request, &prefix_id, &resolver)?;
```

`apply()` 输入和输出都是 provider-neutral `LlmRequest` 包装。真实 key 只从 `SecretResolver` 进入发送前 header；不能写入 request body、metadata、session message、日志或文档。

#### 14. 百步级工具任务

入口：`run_hundred_step_cross_app_task(...)`。

调用方式：

```rust
let report = run_hundred_step_cross_app_task(&definition, &executor)?;
assert!(report.completed);
```

这个任务使用 `ToolCall` 描述每一步动作，通过 `ToolExecutor` 执行，并用 `ToolResultStatus` 判断成功失败。它是 boundary 层的可重复 probe；真实 Android 百步 phone-using run 只在本文顶部的 `run-android-autonomous-map-task.ps1` 示例中展示。

#### 回归检查

回归测试覆盖 tool 注册、visibility guard、session replay 概率图、工具预执行、task-bound context、key routing、复杂 graph 和百步级工具任务。本文不把这些测试命令作为可运行示例；唯一的 runnable path 是顶部已经跑通的真实 ReAct Android 百步任务。

## 稳定性说明

- Mobilerun-like example 当前依赖的 public API surface 以上一节为准；新增 example 对 core 的调用前，必须同步更新该说明和 `CAPABILITY_MAP.md`。
- provider-specific 类型不能泄漏到 tool API。
- `GraphState` 可用于 debug/checkpoint，但外部不要直接可变写入。
- 所有 public error 使用 `AgentCoreError`。
- 示例能编译运行才算 API 文档有效。

## 后续拓展方案

- 增加 runtime-level event subscription，而不是只从 run result 读取事件。
- 增加外部 node executor，让 `GraphNodeAction::Agent` 真正调用 child `Agent`。
- 把 `HandlerRegistry` 接入 `AgentServices`，让 runner 自动触发生命周期 hook。
- 增加跨进程 tool/provider adapter 和 plugin manifest。
- 增加 background run、remote session store 和 trace viewer。

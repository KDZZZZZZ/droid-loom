# Mobilerun-like Example 如何使用 Agent Core

日期：2026-05-31

## 范围

本文只解释 `example/mobilerun-agent-boundary` 和 `agent-smoke` 是怎样调用 `crates/agent-core` 的。它不是 API reference，也不描述真实 secret、provider key 或 Android UI 细节。

核心结论：

- `agent-core` 提供 agent definition、message、tool registry/executor、graph、session、context builder、hook 等运行时原语。
- `example/mobilerun-agent-boundary` 不修改 core 内部，只在 example 层把这些原语组合成 Mobilerun-like agent runtime。
- `agent-smoke` 是真实 Android/provider 测评入口，用 ADB 启动 App、打开 AccessibilityService、传入 provider 配置并校验百步级 phone-using 结果。

## 两条实现线

### Boundary 线

入口：

- `example/mobilerun-agent-boundary/src/main.rs`
- `example/mobilerun-agent-boundary/src/scripted_agent.rs`
- `example/mobilerun-agent-boundary/src/cross_app_task.rs`

这条线跑在本地 Rust 测试和 `cargo run -p mobilerun-agent-boundary` 中。它用 mock mobile tools 验证 core 的边界能力：

- tool visibility guard
- provider-neutral context build
- tool execution and parallel execution
- session replay
- trajectory probability graph
- preexecution
- hot/cold tool layering
- task-bound context
- key/account routing
- app map memory
- hundred-step cross-app task

### Android Smoke 线

入口：

- `agent-smoke/scripts/run-android-autonomous-map-task.ps1`

这条线在模拟器或设备上运行真实 App。脚本通过 ADB：

- 安装 APK
- 授权 overlay 和 AccessibilityService
- 启动 `com.example.agentsmoke/.MainActivity`
- 传入 `debug_tool=agent_autonomous_map_task`
- 传入 provider endpoint、model、key 和 max tool rounds
- 从 logcat 读取 sanitized JSON report
- 校验近百次 provider-visible primitive phone tool calls

## Core 调用链

### 1. Prompt 变成 `AgentDefinition`

文件：

- `example/mobilerun-agent-boundary/src/prompt.rs`

`MobileRunLikeConfig::agent_definition()` 把 Mobilerun-like 配置编译成 core definition：

```rust
let definition = AgentDefinitionBuilder::new()
    .name(self.agent_name.clone())
    .system_prompt(system_prompt)
    .tool_visibility(tool_name, visibility)
    .build()?;
```

这里用到的 core API：

- `AgentDefinitionBuilder`
- `AgentDefinition`
- `ToolVisibility`

`ToolVisibility` 是冷热工具分层的核心承载：

- `Direct`：高频稳定工具直接进 prompt。
- `Searchable`：低频工具按需搜索或动态加入。
- `Hidden`：不可搜索、不可执行，例如 raw shell escape hatch。

### 2. Mobile action 变成 core `Tool`

文件：

- `example/mobilerun-agent-boundary/src/mobile_tools.rs`

每个 mobile action 都实现 core 的 `Tool` trait：

```rust
impl Tool for MobileMockTool {
    fn metadata(&self) -> &ToolMetadata {
        &self.metadata
    }

    fn invoke(&self, invocation: ToolInvocation) -> AgentCoreResult<ToolOutput> {
        Ok(ToolOutput::new(output))
    }
}
```

注册时走 core `ToolRegistry`：

```rust
let mut registry = ToolRegistry::new();
register_mobilerun_tools(&mut registry)?;
```

这里用到的 core API：

- `Tool`
- `ToolMetadata`
- `ToolSchema`
- `ToolInvocation`
- `ToolOutput`
- `ToolRegistry`

工具能力也写在 core metadata 上：

```rust
metadata.capabilities.read_only = matches!(name.as_str(), "screenshot" | "ui_state" | "search_database");
metadata.capabilities.idempotent = metadata.capabilities.read_only;
metadata.capabilities.destructive = name == "raw_adb_shell";
```

后续 `TrajectoryProbabilityGraph::plan_preexecution()` 会用 `ToolMetadata::can_preexecute()` 判断一个工具能不能提前执行。

### 3. Runtime 组装 registry、executor 和输入消息

文件：

- `example/mobilerun-agent-boundary/src/scripted_agent.rs`

`run_boundary_probe()` 是总装配入口：

```rust
let config = MobileRunLikeConfig::boundary_default();
let definition = config.agent_definition()?;

let mut registry = ToolRegistry::new();
register_mobilerun_tools(&mut registry)?;
let registry = Arc::new(registry);

let executor = ToolExecutor::new(registry.clone());
```

用户输入通过 core message API 构造：

```rust
let user_message = user_input::content_blocks_message(vec![
    ContentBlock::text(config.render_user_prompt()),
    ContentBlock::image_reference(
        "screen://current",
        Some("image/png".to_string()),
        Some("current Android screen".to_string()),
    ),
])?;
```

这里用到的 core API：

- `user_input`
- `ContentBlock`
- `RunMessage`
- `ToolExecutor`

### 4. Provider-neutral request 用 `ContextBuilder` 生成

文件：

- `example/mobilerun-agent-boundary/src/scripted_agent.rs`

`build_provider_request()` 把 definition、direct tool schemas 和 messages 编译成 core 的 `LlmRequest`：

```rust
let mut input = ContextBuildInput::new("mimo-v2.5-pro");
input.run_messages = messages;
input.visible_tool_schemas = direct_schemas.to_vec();

let request = ContextBuilder::new().build(definition, input)?;
```

这里用到的 core API：

- `ContextBuildInput`
- `ContextBuilder`
- `LlmRequest`
- `ToolSchema`
- `RunMessage`

重点是：Mobilerun-like 层不直接拼 provider payload。它只生成 provider-neutral `LlmRequest`，provider adapter 再负责转换成具体 HTTP body。

### 5. Assistant tool call 通过 `ToolExecutor` 执行

文件：

- `example/mobilerun-agent-boundary/src/scripted_agent.rs`
- `example/mobilerun-agent-boundary/src/cross_app_task.rs`

scripted probe 中批量执行工具：

```rust
let tool_results = executor.execute_batch(&definition, tool_calls)?;
let tool_messages = tool_results
    .iter()
    .map(|result| result.into_run_message())
    .collect::<AgentCoreResult<Vec<_>>>()?;
```

百步任务中，每一步都构造 core `ToolCall`：

```rust
ToolCall::new(format!("observe-{order_index}"), "ui_state", json!({}))
ToolCall::new(format!("lookup-order-{order_index}"), "search_database", json!({"query": query}))
ToolCall::new(format!("remember-order-{order_index}"), "remember", json!({"information": value}))
ToolCall::new(format!("type-note-{order_index}"), "type", json!({"text": text, "index": 0}))
```

最终统一走：

```rust
let result = executor.execute_one(definition, call.clone())?;
if result.status != ToolResultStatus::Success {
    return Err(AgentCoreError::Recoverable(...));
}
```

这里用到的 core API：

- `ToolCall`
- `ToolExecutor::execute_one`
- `ToolExecutor::execute_batch`
- `ToolExecutor::execute_batch_parallel`
- `ToolResult`
- `ToolResultStatus`
- `ToolResult::into_run_message`

关键点：复杂任务负责规划和构造动作，真正执行、visibility check、schema validation、permission decision 和 result wrapping 都由 core 完成。

### 6. Session 只记录 message 轨迹

文件：

- `example/mobilerun-agent-boundary/src/scripted_agent.rs`

`execution_session_snapshots()` 把 user、assistant、tool messages 写入 core session store：

```rust
let mut store = InMemorySessionStore::new();
store.append(SessionEntry::header(session_id))?;
store.append(SessionEntry::message(Some(parent_id), message)?)?;

let tree = store.load_tree()?;
let snapshot = replay_active_branch(&tree)?;
```

这里用到的 core API：

- `SessionEntry`
- `InMemorySessionStore`
- `SessionStore`
- `SessionTree`
- `ReplaySnapshot`
- `replay_active_branch`

设计约束是：工具调用和工具结果也是 message，所以轨迹统计只需要读 `RunMessage` 序列，不需要读取 core 私有状态。

### 7. 概率图从 replay message 生成

文件：

- `example/mobilerun-agent-boundary/src/execution_probability.rs`
- `example/mobilerun-agent-boundary/src/scripted_agent.rs`

`run_boundary_probe()` 从 session replay 生成概率图：

```rust
let probability_graph = TrajectoryProbabilityGraph::from_replay_snapshots(&session_snapshots);
```

概率图做三件事：

```rust
let tool_layer_plan = probability_graph.plan_tool_layers(&default_tool_visibility(), 0.5);
let preexecution_plan = probability_graph.plan_preexecution(
    registry.as_ref(),
    &optimized_definition,
    candidates,
    0.5,
);
let preexecution_outcome = TrajectoryProbabilityGraph::execute_preexecution_plan(
    &executor,
    &optimized_definition,
    preexecution_plan,
)?;
```

这里用到的 core API：

- `RunMessage`
- `ReplaySnapshot`
- `ToolRegistry`
- `AgentDefinition`
- `ToolCall`
- `ToolExecutor`
- `ToolResult::into_run_message`

概率图本身不是 core 类型。它是 example/runtime 层逻辑，但输入输出都围绕 core 的 message 和 tool API。

### 8. Task-bound context 产出 core messages

文件：

- `example/mobilerun-agent-boundary/src/task_context.rs`

`TaskDependencyGraph::context_for(task_id)` 返回 `Vec<RunMessage>`：

```rust
let task_graph = sample_mobile_task_dag()?;
let context_messages = task_graph.context_for("finalize")?;
```

这里用到的 core API：

- `RunMessage`
- `ContentBlock`

task DAG 的职责是避免每个任务继承全量历史。子任务只继承依赖任务的 artifact，不继承中间工具调用、失败尝试或临时屏幕状态。

### 9. 稳定上下文排序服务缓存命中

文件：

- `example/mobilerun-agent-boundary/src/context_stability.rs`
- `example/mobilerun-agent-boundary/src/scripted_agent.rs`

Mobilerun-like 层给 message 打稳定性标签：

```rust
let ordered_context_messages = order_by_stability(task_context_messages);
let prefix_id = stable_prefix_id(
    optimized_definition.system_prompt(),
    &tool_layer_plan.direct_tools,
);
```

这里用到的 core API：

- `RunMessage.metadata`
- `AgentDefinition::system_prompt`

排序目标是让稳定 prefix、task package、dependency result 放在前面， volatile observation 放在后面，从而提高 provider prompt cache 命中率。

### 10. Key/account routing 修改 `LlmRequest`

文件：

- `example/mobilerun-agent-boundary/src/key_routing.rs`
- `example/mobilerun-agent-boundary/src/scripted_agent.rs`

路由计划输入是 stable prefix id，输出是带 header 的 provider-neutral request：

```rust
let key_route_plan = KeyRoutePlan::new(prefix_id.clone());
let stable_routed = key_route_plan.apply(request, &prefix_id, &resolver)?;
```

这里用到的 core API：

- `LlmRequest`

约束：真实 key 只能进入发送前 header，不能进入 request metadata、provider body、session message、测试 fixture 或文档。

### 11. Graph 和 AgentFactory 验证复杂编排

文件：

- `example/mobilerun-agent-boundary/src/scripted_agent.rs`

Graph probe 使用 core graph primitives：

```rust
let graph = Graph::builder("mobilerun_fast_turn")
    .node(GraphNode::new("input").with_action(GraphNodeAction::PassthroughInput))
    .node(GraphNode::new("assistant").with_action(GraphNodeAction::EmitMessages(vec![assistant_message])))
    .edge(GraphEdge::new("input_to_assistant", "input", "assistant"))
    .start_node("input")
    .build()?;

let agent = AgentFactory::default().create(definition.clone())?;
let result = agent.run(AgentRunInput::new(graph).with_initial_messages(vec![user_message]))?;
```

这里用到的 core API：

- `Graph`
- `GraphNode`
- `GraphNodeAction`
- `GraphEdge`
- `ActivationCondition`
- `GraphStateBudget`
- `AgentFactory`
- `AgentRunInput`
- `AgentRunResult`
- `CoreEvent`

当前真实百步 phone-using 的 ReAct loop 在 Android smoke 线中验证；boundary graph probe 主要验证 core graph 编排能力。

## 百步任务如何落到 core

文件：

- `example/mobilerun-agent-boundary/src/cross_app_task.rs`
- `example/mobilerun-agent-boundary/src/app_map_memory.rs`

`run_hundred_step_cross_app_task()` 的签名体现边界：

```rust
pub fn run_hundred_step_cross_app_task(
    executor: &ToolExecutor,
    definition: &AgentDefinition,
) -> AgentCoreResult<CrossAppTaskReport>
```

它不直接调用 mobile tool 实现，而是做四层事情：

1. `AppMapMemory` 维护页面图、candidate actions、semantic search 和 path planning。
2. 任务层根据当前页面、目标页面和业务目标生成 `ToolCall`。
3. `execute_tool_step()` 把 `ToolCall` 交给 `ToolExecutor`。
4. 报告层记录 step count、app switch、map reuse、token savings 和 executed tool counts。

核心执行点：

```rust
let view = map.local_view(Some(page), 1, None);
let result = executor.execute_one(definition, call.clone())?;
report.step_count += 1;
```

这就是“真实复杂任务使用 core”的最小闭环：

```text
AppMapMemory -> CandidateAction -> ToolCall -> ToolExecutor -> ToolResult -> CrossAppTaskReport
```

## Android Smoke 如何使用 core

文件：

- `agent-smoke/scripts/run-android-autonomous-map-task.ps1`

脚本启动真实 Android 任务：

```powershell
.\scripts\run-android-autonomous-map-task.ps1 `
  -TargetToolCalls 100 `
  -MinimumRequiredToolCalls 95 `
  -ReactSelfCheckInterval 10 `
  -MaxToolRounds 128 `
  -WaitSeconds 900
```

关键参数通过 Activity extras 传给 App：

```powershell
--es debug_tool agent_autonomous_map_task
--es mimo_api_base $ApiBase
--es mimo_api_key $key.Value
--es mimo_model $Model
--ei agent_max_tool_rounds $MaxToolRounds
```

脚本还打开 AccessibilityService：

```powershell
settings put secure enabled_accessibility_services `
  com.example.agentsmoke/com.example.agentsmoke.AgentAccessibilityService
settings put secure accessibility_enabled 1
```

最后校验 report：

- `actual_tool_calls >= MinimumRequiredToolCalls`
- `provider_tool_traces >= MinimumRequiredToolCalls`
- `macro_tool_traces == 0`
- `meaningful_phone_task == true`
- navigation、observation、artifact、device read 工具数量达标
- `unique_tool_count >= 10`
- `argument_signature_count >= 30`
- `task_subgoal_coverage_count >= 8`
- `repeated_fixed_action_loop == false`

这条线证明的是：真实 provider-visible phone tools 在 ReAct loop 中被连续调用，不是 boundary mock 的宏工具循环。

## 阅读顺序

如果只想看它怎样用 core，按这个顺序读：

1. `example/mobilerun-agent-boundary/src/prompt.rs`
2. `example/mobilerun-agent-boundary/src/mobile_tools.rs`
3. `example/mobilerun-agent-boundary/src/scripted_agent.rs`
4. `example/mobilerun-agent-boundary/src/cross_app_task.rs`
5. `example/mobilerun-agent-boundary/src/app_map_memory.rs`
6. `example/mobilerun-agent-boundary/src/execution_probability.rs`
7. `example/mobilerun-agent-boundary/src/task_context.rs`
8. `example/mobilerun-agent-boundary/src/key_routing.rs`
9. `agent-smoke/scripts/run-android-autonomous-map-task.ps1`

## 回归测试

覆盖文档中这条调用链的测试：

```powershell
cargo test -p agent-core -p mobilerun-agent-boundary
```

重点测试：

- `crates/agent-core/tests/public_api_contract.rs`
- `example/mobilerun-agent-boundary/src/public_api_contract.rs`
- `example/mobilerun-agent-boundary/src/cross_app_task.rs`
- `example/mobilerun-agent-boundary/src/execution_probability.rs`
- `example/mobilerun-agent-boundary/src/key_routing.rs`

这些测试保证 example 继续通过公开 core API 调用，而不是依赖 core 私有实现。

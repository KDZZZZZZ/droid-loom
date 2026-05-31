# Mobilerun Agent Boundary Example

这个示例用一个离线、可复现的 Mobilerun-like 任务来测试 `agent-core` 的真实使用边界。它只依赖 `agent-core` 的公开 API，不引用 core 内部私有模块，也不实现真实 Android 控制、截图采集、账号密钥库或业务后端。

示例刻意把 Mobilerun 能力拆成两类：

- FastAgent：直接选择下一步移动端动作。
- Reasoning Agent：用 manager/executor graph 表达“规划 -> 执行 -> 校验”。

## 运行方式

从仓库根目录执行：

```powershell
cargo run -p mobilerun-agent-boundary
```

期望输出是一组能力矩阵：

- `supported`：当前 core API 已经能直接表达。
- `simulated`：示例能通过 core API 串起来，但生产能力仍由外部系统实现。
- `gap`：示例暴露出的 core 能力缺口。

逐项能力映射和验收表见 [CAPABILITY_MAP.md](CAPABILITY_MAP.md)。

## 文件职责

`src/main.rs`

入口文件，只调用 `scripted_agent::run_boundary_probe()` 并打印报告。以后不要在这里塞业务逻辑。

`src/prompt.rs`

定义 Mobilerun-like agent 的 prompt 配置和渲染逻辑。它负责把 agent name、system prompt、目标、变量、最大步数、视觉开关、自定义指令和工具可见性组合成 `AgentDefinition`。这里不注册工具、不执行 provider、不保存 session。

`src/mobile_tools.rs`

定义一组模拟移动端工具，并注册进 `ToolRegistry`。工具覆盖 Mobilerun FastAgent 动作：`click`、`click_at`、`click_area`、`long_press`、`long_press_at`、`type`、`type_secret`、`swipe`、`system_button`、`wait`、`open_app`、`remember`、`complete`；同时补充 `screenshot`、`ui_state` 作为 observation 工具、`search_database` 作为 custom tool、`raw_adb_shell` 作为 hidden guard。所有非隐藏工具都会通过 `ToolExecutor` 实际执行一次，用于验证 schema、可见性、执行结果和 tool result message。

`src/scripted_agent.rs`

把 core API 串成真实边界探测：构建 fast/reasoning agent definition、注册 tool、包装用户 message/content block、构建 provider-neutral request、构造 assistant tool call、执行工具、生成 tool result message、运行 handler、运行 turn loop 输入队列、运行 fast graph、运行 manager/executor graph、把 core event 投影成 Mobilerun-style event stream、测试取消和预算保护，并输出能力缺口。

`src/execution_probability.rs`

从 session message 轨迹抽取 message/tool call/tool result 事件，统计 transition probability 和 tool usage probability。它根据概率图做工具预执行计划和工具冷热分层，但只预执行 `read_only + idempotent` 且非 destructive 的工具。

`src/context_stability.rs`

给 context message 标记稳定性，并按 stable prefix、task package、dependency result、volatile observation 排序，保证稳定上下文块在 provider input 前缀中更靠前。

`src/task_context.rs`

定义 Mobilerun subgoal 风格的 task context package 和 task dependency DAG。子 task 只继承依赖 task 的关键 artifact，不继承中间工具调用、失败尝试或临时屏幕状态。

`src/key_routing.rs`

根据 `stable_prefix_id` 路由 provider request 到固定 account/key 槽位，通用或易变 prefix 走另一个槽位。真实 key 只从环境变量读取，不写入源码、文档、测试输出或 session。

## 具体使用方案

定义 agent：

`MobileRunLikeConfig::boundary_default()` 给出一个可运行配置，然后调用 `agent_definition()` 得到 `AgentDefinition`。业务代码只应该在配置层决定 agent name、system prompt 和工具可见性。

定义 tool：

实现 `agent_core::tool::Tool`，返回 `ToolMetadata` 和 `ToolOutput`，再注册到 `ToolRegistry`。常用工具设置为 `Direct`，不常用工具设置为 `Searchable`，危险或内部工具设置为 `Hidden`。调用方用 `ToolExecutor::execute_*_message(s)` 执行，结果会自动包装成 `RunMessage(role=tool)` 回传给模型。

本示例的工具可见性约定：

- `Direct`：`screenshot`、`ui_state`、`click`、`click_at`、`type`、`swipe`、`wait`、`complete`。
- `Searchable`：`click_area`、`long_press`、`long_press_at`、`system_button`、`type_secret`、`remember`、`open_app`、`search_database`。
- `Hidden`：`raw_adb_shell`，用于证明隐藏工具不能被该 agent 执行。

输入用户信息：

文本、图片、文件等用户输入通过 `agent_core::user_input` 包成 `RunMessage`。本示例使用 `content_blocks_message()` 同时传入 `Text` 和 `ImageReference`，用来模拟当前屏幕观测。

会话级入口：

Mobilerun 的 `send_user_message` 映射为“包装 `RunMessage(role=user)`，交给 `TurnLoop::run_message()` 启动本轮 graph”。本示例验证 graph run -> append new messages -> Idle 的生命周期；调用方不直接使用 `GraphRunner` 作为常规入口。

构建 provider request：

使用 `ContextBuilder` 把 `AgentDefinition`、历史 `RunMessage`、本轮 `RunMessage` 和 direct tool schema 转成 `LlmRequest`。示例模型名使用 `mimo-v2.5-pro`，但不会发真实 HTTP 请求。

基于轨迹优化：

历史执行轨迹通过 `InMemorySessionStore -> SessionTree -> ReplaySnapshot` 重建，概率图只读取 replay 出来的 `RunMessage` 序列。`execution_probability.rs` 会把 assistant tool call 和 tool result message 纳入同一个概率图，用来预测下一步高概率事件。示例用该图验证三件事：

- 对高概率且只读幂等的 `ui_state`、`search_database` 做预执行，并把结果继续包装成 tool message。
- 把高频工具放入 hot/direct 层，把低频工具放入 dynamic/searchable 层。
- 用 `ToolExecutor::execute_batch_parallel_messages()` 验证并行工具调用，并保持 result message 顺序。

Task 绑定上下文：

`TaskDependencyGraph` 把任务拆成 `open_app -> search_catalog -> finalize`。`finalize` 只继承 `search_catalog` 的关键 artifact，例如 `first_result=Wireless Charger Stand`，不会继承 `click_at`、失败尝试、临时屏幕等中间噪声。

缓存友好上下文：

`context_stability.rs` 把上下文块按稳定性排序：stable prefix 在最前，task package 其次，dependency result 再其次，当前屏幕 observation 放最后。`stable_prefix_id` 由 system prompt 和 hot direct tool names 生成，供 key/account routing 使用。

Key/account routing：

真实运行时使用两个环境变量槽位：

```powershell
$env:MOBILERUN_STABLE_PREFIX_API_KEY="<existing stable-prefix key>"
$env:MOBILERUN_GENERAL_POOL_API_KEY="<second test key>"
```

边界示例的单元测试使用 fake resolver 验证两把 key 会路由到不同 account/header，不使用真实 key。测试还会把 routed request 交给 OpenAI-compatible chat provider prepare，断言 fake key 只进入 `Authorization` header，不进入 request metadata、prepared metadata 或 provider body，避免污染可缓存上下文。

挂 handler：

使用 `HandlerRegistry` 注册 point handler 和 wrapper handler。point handler 适合输入改写、审计、阻断、事件记录；wrapper handler 适合包住 tool/provider/node 执行，注入 deadline、重试、恢复消息或短路结果。示例里的 provider wrapper 会把可恢复错误转成 `WrapperResult::Recover { messages }`，证明 handler 不只是 callback，也能作为 middleware 改变行为。

构建 graph：

使用 `Graph::builder()` 定义 node、output port、edge 和 input package。edge 只做内容传输：从 `(node, port)` 读取 output log，按目标 package item 的 `MessageQuery` 筛选后写入下游 package。示例展示了三种图：

- `mobilerun_fast_turn`：`input.messages -> agent.context -> mark_done.calls -> final.answer`。
- `mobilerun_reasoning_manager_executor`：manager agent -> executor agent -> manager check -> final。
- `mobilerun_complex_task_dag`：task/dependency package 解锁两个并行工具节点，再 fan-in 到 manager package。

这些图都只使用 core public graph API。真实 provider/tool/agent node 执行由 `NodeExecutor` 注入，本示例用 scripted executor 模拟真实服务结果。

读取 event：

`Agent::run()` 返回 `AgentRunResult`，其中 `events` 包含 agent、graph、node、message 的事件流。外部 UI、日志系统和 replay 系统都应该读这里，而不是侵入 runner 内部状态。

事件流投影：

Mobilerun-style streaming event 可以由 `CoreEvent` 投影出来。本示例使用 `EventLog` 聚合 `AgentStarted`、`GraphStarted`、`NodeStarted`、`MessageEmitted` 等 core event，并把 tool dispatch 的结果以 `HookEmitted(name="tool_execution")` 投影成 tool execution event。当前 core 只返回 run result event；实时 subscription 是后续可拓展点。

终止 graph：

调用 `TurnLoop::request_stop()` 可阻止新 turn 启动；循环保护通过 `TurnLoop::with_max_ticks()` 或 `set_max_ticks()` 传给内部 graph run 生效。更底层的 agent/graph 取消能力只作为高级集成接口保留。

## 可拓展点

- 把 mock tool 换成真实 Android/iOS/browser adapter，但保持 `Tool` trait 和 `ToolSchema` 不变。
- 把 scripted assistant 换成真实 provider streaming adapter，但保持 `AssistantBuilder` 和 `LlmStreamEvent` 到 `RunMessage` 的路径不变。
- 给 `HandlerRegistry` 增加生产级 handler，例如权限审批、可恢复错误转换、trace 上报、provider fallback。
- 扩展 graph node executor registry，让 agent/tool/subgraph node 按名字路由到真实 provider、工具和 child agent。
- 在 core 之外增加结构化输出校验器、credential vault、视觉 parser 和真实移动端 observation parser。

## 当前边界

这个示例刻意不实现 Mobilerun SDK 本身。它只复刻 agent core 需要表达的能力：prompt、FastAgent 动作集合、manager/executor reasoning graph、custom variables、custom tool、credential tool 形态、message/content block、turn loop 输入队列、tool call/result、handler middleware、event stream projection、取消和预算、轨迹概率图、工具预执行、冷热分层、task-bound context、cache-aware context ordering、key/account routing 和并行 tool batch。凡是设备连接、截图识别、账号凭据、业务服务调用、真实 provider HTTP，应该作为 core 外部 adapter 接入。

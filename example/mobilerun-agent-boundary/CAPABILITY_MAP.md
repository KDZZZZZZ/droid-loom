# Mobilerun Capability Map

本文把 Mobilerun agent 能力映射到当前 `agent-core` public API 和本示例的可运行证据。它是本示例的验收表：后续扩展必须先落在这里的某个能力行上，或者先新增一行说明边界和验证方式。

参考资料：

- MobileAgent SDK: https://docs.mobilerun.ai/framework/sdk/droid-agent
- Event Streaming: https://docs.mobilerun.ai/framework/concepts/events-and-workflows
- Custom Variables: https://docs.mobilerun.ai/framework/features/custom-variables
- Credentials: https://docs.mobilerun.ai/framework/features/credentials

## 状态定义

- `supported`：当前 core public API 可以直接表达，并且本示例有运行证据。
- `simulated`：当前 core public API 可以串起边界，但真实生产能力应由外部 adapter 实现。
- `external`：不是 agent core 职责，必须留在 Android/iOS/browser/provider shell。
- `gap`：当前 core public API 还缺少一等能力，只能通过外围逻辑临时代替。

## 能力映射

| Mobilerun 能力 | 当前表达方式 | 示例证据 | 状态 |
| --- | --- | --- | --- |
| `reasoning=False` 走 FastAgent | `Graph` 中的 `mobilerun_fast_turn`，由 `TurnLoop` 启动一个 agent turn 直接产出 tool calls | `turn loop graph - status=completed` | supported |
| `reasoning=True` 走 Manager/Executor | `Graph` 中的 `manager_plan -> executor_action -> manager_check` | `reasoning manager/executor graph - status=completed` | supported |
| `goal` | `MobileRunLikeConfig.goal` 渲染进 user prompt | `message and content blocks`、`context builder` | supported |
| `prompts` 覆盖 | `PromptSet.fast_system`、`PromptSet.reasoning_system`、`PromptSet.user_task` | `agent definition` 同时构建 fast/reasoning definition | supported |
| custom variables | `MobileRunLikeConfig.variables` + `render_template()` | user prompt 中渲染 `app_name`、`search_term` | supported |
| FastAgent 动作工具 | `ToolSchema` + `ToolRegistry` + `ToolExecutor` | `tool execution coverage - executed 16/16 visible tools` | supported |
| 不常用工具搜索后显示 | `ToolVisibility::Searchable` + `ToolRegistry::search()` | `searchable_hits=1` | supported |
| 隐藏工具不可见不可执行 | `ToolVisibility::Hidden` + `ToolRegistry::get_for_agent()` guard | `hidden_blocked=true`、`tool visibility guard` | supported |
| custom tools | `search_database` 作为 searchable tool | `cover-search-db` tool call 成功执行 | supported |
| credentials | `type_secret(secret_id, index)` tool 只暴露 secret id，结果 redacted | `cover-type-secret` 成功执行且输出 redacted | simulated |
| screenshot observation | `ContentBlock::ImageReference` + `screenshot` tool output `screen://current` | `message and content blocks`、`cover-screenshot` | simulated |
| UI state observation | `ui_state` tool 返回 mock element list | `cover-ui` tool call 成功执行 | simulated |
| App 地图记忆 | `AppMapMemory` 从 `ui_state` 维护页面节点、路径、局部视野、语义搜索、forget 和候选 action | `app map memory - pages=4, transitions=3` | supported |
| 百步级跨 App 连续任务 | `cross_app_task.rs` 用地图路径在 Shop/Notes/Calendar/Mail 之间自动执行 100+ 工具动作 | `hundred-step cross-app task - completed=true, steps=122` | simulated |
| `send_user_message` | `user_input` 包装消息，再由 `TurnLoop::run_message()` 启动 graph 并追加新消息 | `turn loop entry - graph_status=completed` | supported |
| `run_event_stream()` | `CoreEvent` + `EventLog` 投影成 Mobilerun-style event kind | `event stream projection - tool_events=16` | simulated |
| `ToolExecutionEvent` | `HookEmitted(name="tool_execution")` 投影 | `event stream projection - tool_events=16` | simulated |
| recoverable provider/tool error | wrapper handler 返回 `WrapperResult::Recover { messages }` | `handler hooks - recover_messages=1` | supported |
| structured output | 示例外部 JSON shape validator | `structured final output` | simulated |
| `max_steps` | prompt 中渲染，graph runtime 用 tick budget 单独验证 | `budget_guard=budget_exceeded` | supported |
| workflow timeout | tool metadata 有 `timeout_ms`，executor 未强制计时 | Known core gaps | gap |
| per-role LLMs (`manager`/`executor`/`fast_agent`) | 可由外部 runtime 对不同 `LlmRequest.model` 赋值；本示例只构建 provider-neutral request | `context builder - model=mimo-v2.5-pro` | simulated |
| driver/state provider | Android/iOS/browser shell 负责，不进入 core | README 当前边界 | external |
| real device action | 真实 Android/iOS adapter 实现 `Tool` trait | mock tools only | external |
| tracing/telemetry backend | 读取 `CoreEvent` 后由外部 trace sink 处理 | event projection only | external |

## 轨迹优化能力映射

| 优化能力 | 当前表达方式 | 示例证据 | 状态 |
| --- | --- | --- | --- |
| 基于轨迹的概率图 | `execution_probability.rs` 从 session replay 出来的 finalized `RunMessage` 提取 message/tool call/tool result 事件并统计 transition | `trajectory probability graph - session_branches=3` | supported |
| 工具预执行 | 只对 `ToolMetadata::can_preexecute()` 为 true 的高概率候选执行；结果仍转成 `role=tool` message | `tool preexecution - preexecuted=2` | supported |
| 工具冷热分层 | 概率图把高频工具设为 `Direct`，低频工具设为 `Searchable`，`Hidden` 保持隐藏 | `tool hot/cold layering` | supported |
| 前缀冷热分层 | `context_stability.rs` 用稳定 prefix + hot tool names 生成 `stable_prefix_id` | `cache-aware context ordering` | supported |
| key/account 路由 | `key_routing.rs` 将稳定 prefix 路由到固定 key/account 槽位，通用 prefix 路由到池化槽位；fake key 只进入发送前的 `Authorization` header，不进入 metadata/body | `key/account routing - separate_headers=true`；`provider_prepare_keeps_route_keys_in_headers_not_metadata` | supported |
| 稳定性排序上下文 | `RunMessage.metadata["context.stability"]` 排序为 stable -> task -> dependency -> volatile | `cache-aware context ordering - stable_prefix` | supported |
| Task 绑定上下文 | `TaskDependencyGraph` 只继承依赖任务 artifacts，不继承中间工具噪声 | `task-bound context dag` | supported |
| 地图局部视野降耗 | `local_view()` 只给当前页面附近几跳和任务相关入口，不把整图塞进 prompt | `local_map_tokens=12913 vs full_map_tokens=45140` | supported |
| 地图动态维护 | 路径失败多次后标记 stale，`forget()` 可按 query/page/stale 删除 | `stale_paths_after_failures=1`, `forget_removed=1` | supported |
| 复杂图验证 | task/dependency package 解锁两个并行工具 node，fan-in 到 manager package 后完成 final node | `complex task graph - status=completed` | supported |
| 并行工具调用 | `ToolExecutor::execute_batch_parallel_messages()` 并行执行且保持 result message 顺序 | `parallel tool execution - parallel_results=2` | supported |

## 使用方案

运行完整边界探针：

```powershell
cargo run -p mobilerun-agent-boundary
```

运行回归测试：

```powershell
cargo test -p mobilerun-agent-boundary
```

验收时至少检查这些输出：

- `registered 17/17 mocked action tools`
- `executed 16/16 visible tools`
- `recover_messages=1`
- `turn loop entry - graph_status=completed`
- `event stream projection - ... tool_events=16`
- `reasoning manager/executor graph - status=completed`
- `trajectory probability graph - session_branches=3`
- `tool preexecution - preexecuted=2`
- `parallel tool execution - parallel_results=2`
- `key/account routing - separate_headers=true`
- `app map memory - pages=4`
- `hundred-step cross-app task - completed=true`
- `token optimization effects - ... long_task_token_savings=71.4%`
- `complex task graph - status=completed`

## 后续拓展方案

- 要支持真实 credential vault，只新增外部 credential adapter；core 仍只看到 `type_secret(secret_id, index)` 和 redacted tool result。
- 要支持实时 event subscription，优先在 runtime facade 增加 event sink；不要让 event subscriber 直接修改 graph state。
- 要支持 provider/tool/agent node 自动执行，应沿 `NodeExecutor` / executor registry 的可拓展点实现，不把 provider/tool 逻辑塞进 edge。
- 要支持 workflow timeout，优先在 runtime/tool execution wrapper 层做 deadline enforcement；不要让 tool schema 自己管理时间。
- 要支持结构化输出，先作为 context/provider 后处理器或 wrapper handler；不要把 Pydantic/JSON schema validator 写死进 message 层。

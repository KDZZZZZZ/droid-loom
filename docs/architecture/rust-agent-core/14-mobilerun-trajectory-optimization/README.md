# Mobilerun-like 轨迹优化实现说明

日期：2026-05-31

## 范围

本文说明 `example/mobilerun-agent-boundary` 如何在不重构 core 的前提下复刻 Mobilerun agent runtime 能力：

- 用 session message 轨迹统计概率图。
- 基于概率图做只读幂等工具预执行。
- 将工具、prompt prefix 和 context 按冷热/稳定性分层。
- 将 task 绑定到自己的上下文包，并按依赖继承关键结果。
- 使用 key/account 路由减少稳定 prefix 的缓存污染。
- 验证复杂 task graph 和并行 tool call。

这些能力属于 runtime/example 层。`agent-core` 只新增了两个通用扩展点：`ToolCapabilities.idempotent` 和
`ToolExecutor::execute_batch_parallel()`。

## 文件职责

- `example/mobilerun-agent-boundary/src/execution_probability.rs`
  - 从 `RunMessage` 序列抽取 `message:*`、`tool_call:*`、`tool_result:*` 事件。
  - 支持从 `ReplaySnapshot` 读取 session replay 后的 message 轨迹，证明概率图输入来自 session。
  - 统计 transition count、tool session probability 和 high-probability next event。
  - 根据 `ToolMetadata::can_preexecute()` 只预执行 `read_only + idempotent + !destructive` 的候选工具。
  - 根据工具出现概率生成 hot direct tools 和 cold dynamic/searchable tools。

- `example/mobilerun-agent-boundary/src/context_stability.rs`
  - 给 `RunMessage.metadata["context.stability"]` 标记稳定性。
  - 排序顺序是 `stable_prefix -> task_package -> dependency_result -> volatile_observation`。
  - 用稳定 system prompt 和 hot direct tool names 生成 `stable_prefix_id`，供 key/account routing 使用。

- `example/mobilerun-agent-boundary/src/task_context.rs`
  - 定义 `TaskContextPackage` 和 `TaskDependencyGraph`。
  - 子任务只继承依赖任务的 `artifacts`，不继承中间 tool call、失败尝试或临时屏幕状态。
  - 示例 DAG 是 `open_app -> search_catalog -> finalize`。

- `example/mobilerun-agent-boundary/src/key_routing.rs`
  - `KeyRoutePlan` 根据 `stable_prefix_id` 路由到稳定账号，否则路由到通用池账号。
  - 实际 key 只从环境变量槽位解析：`MOBILERUN_STABLE_PREFIX_API_KEY` 和 `MOBILERUN_GENERAL_POOL_API_KEY`。
  - 单元测试使用 fake resolver 验证两把 key 的路由差异，不写入真实 key。
  - `provider_prepare_keeps_route_keys_in_headers_not_metadata` 会把 routed request 交给 OpenAI-compatible chat provider prepare，断言 fake key 只进入 `Authorization` header，不进入 request metadata、prepared metadata 或 provider body。

## 数据流

1. agent run 结束后，session 层只提交 finalized `RunMessage`。
2. 示例把 user/assistant/tool result messages 写入 `InMemorySessionStore`，通过 `SessionTree` 和 `replay_active_branch()` 重建 `ReplaySnapshot`。
3. 概率图读取 replay snapshot 中的 message，生成 tool 使用概率和 transition probability。
4. runtime 用概率图挑选候选预执行工具；只有只读且幂等的工具会被执行。
5. runtime 用概率图生成 hot/cold tool visibility：hot 进入 stable prompt，cold 仍按需搜索。
6. task context assembler 只为当前 task 构建上下文包，并按 DAG 继承依赖 artifact。
7. context stability sorter 把稳定块排在前面，把当前屏幕等易变块放在后面。
8. key router 将稳定 prefix 绑定到固定 account/key 槽位，通用或易变 prefix 走另一个 account/key 槽位。

## 验证

运行：

```powershell
cargo test -p agent-core -p mobilerun-agent-boundary
cargo run -p mobilerun-agent-boundary
```

边界示例输出必须包含：

- `trajectory probability graph - session_branches=3`
- `tool hot/cold layering`
- `tool preexecution`
- `parallel tool execution`
- `cache-aware context ordering`
- `task-bound context dag`
- `key/account routing`
- `app map memory`
- `complex task graph`
- `hundred-step cross-app task`
- `token optimization effects`

模拟器端另用 `agent-smoke` 验证真实 Android platform tools。Activity-bound demo 已跑通一个不离开 app 的复杂任务：
`android_device_info -> android_battery -> android_set_clipboard -> android_show_toast -> Chinese summary`。
当前实测输出包含 `message_count=7`、四个 tool trace 均 `ok=true`，assistant 用中文列出成功工具。

为了覆盖 Mobilerun-style 跨 App phone using，Android shell 现在新增 `AgentHostService`、全局悬浮球和
`AgentAccessibilityService`。真正的 `ui_state`、按文本点击、输入文本、back/home/recents 工具由 AccessibilityService 提供；
`ui_state` 优先读取多窗口 accessibility tree，悬浮球本身不会遮掉底层 App 上下文。Service 持有 agent runtime，
所以打开 Settings 或其他 App 后 agent 不再依赖 Activity 保持前台。

## 地图记忆与百步任务

GUI-Explorer 风格地图记忆在 `example/mobilerun-agent-boundary/src/app_map_memory.rs` 中实现。它维护页面节点、跳转路径、
当前位置局部视野、语义搜索、动态 stale 标记、forget 和预构造候选 action。`cross_app_task.rs` 基于这张地图执行
Shop -> Notes -> Calendar -> Mail 的 122 步跨 App 连续任务，并记录 token 降耗数据。

当前 `cargo run -p mobilerun-agent-boundary` 输出：

```text
app map memory - pages=4, transitions=3, search_hits=1, path_len=2, local_nodes=3, candidate_actions=11, local_tokens=163, full_tokens=207, forget_removed=1, stale_paths_after_failures=1
hundred-step cross-app task - completed=true, steps=122, app_switches=41, semantic_searches=10, map_reuse_hits=173, token_savings=71.4%
token optimization effects - tool_layer_direct=8->6, local_map_tokens=12913 vs full_map_tokens=45140, long_task_token_savings=71.4%
```

完整记录见 `docs/architecture/rust-agent-core/15-app-map-memory-token-results/README.md`。

Android 真实端也注册了 `android_map_*` tools。当前模拟器 `android_map_long_task_smoke` 不依赖模型 key，使用
AccessibilityService 更新地图并循环 Settings、Launcher、Agent app：

```text
completed=true, steps=100, app_switches=50, page_count=7, transition_count=54, local_map_tokens=32954 vs full_map_tokens=85604, token_savings=61.5%
```

新增目标驱动的真实端 `android_map_goal_task`，任务是跨 Settings、Wi-Fi Settings、Launcher 和 Agent app
生成设备准备简报，并把中间产物写入剪贴板、用 toast 显示状态。它会在任务内调用 `android_map_plan_path`
复用已记录路径：

```text
completed=true, steps=111, app_switches=18, semantic_searches=24, planned_paths=7, map_reuse_hits=15, page_count=6, transition_count=19, local_map_tokens=10281 vs full_map_tokens=15608, token_savings=34.1%, session_message_count=224, tool_trace_count=111, probability_graph.tool_call_events=111
```

该真实端概率图直接从 session-like 轨迹生成：`tool_call_events=111`、`transition_count=110`、`hot_tools=8`、`likely_next=10`、`preexecution_candidates=3`。当前热工具前四个是 `android_map_observe:36`、`android_map_plan_path:24`、`android_open_settings:12`、`android_set_clipboard:7`；可预执行候选是 `android_device_info`、`android_battery`、`android_map_plan_path`。
同一 report 的最终局部视野摘要是 `node_count=6`、`transition_count=19`、`candidate_action_count=5`，语义搜索 `hit_count=4`。验证脚本现在还会检查非零 page、transition、path reuse、局部视野节点、候选 action、语义搜索命中、完整 session/tool 轨迹和 token savings，避免 AccessibilityService 启动竞态导致空地图仍返回 `completed=true`。

动态维护也有真实端 probe：`android_map_stale_path_probe` 在 stale 前能规划到 probe path，连续三次失败后 `stale_transition_count=1`，同一 target 的 stale path 不再返回。
`android_map_stale_page_refresh_probe` 覆盖 page node 刷新：旧节点标记 stale 后，再观察同一 stable UI state 返回 `same_page=true`、`replaced_stale_node=true`。

为了支持真实 provider 自主执行，`agent-smoke` 增加了 `prompt_with_tools_limit` 和 usage 聚合字段；
Android 可用 `debug_tool=agent_autonomous_map_task` 触发模型调用地图工具。当前目标已经改成单次 ReAct graph loop 内的近百次 provider-visible primitive phone tool calls：该入口会创建一个不注册宏/debug tools 的临时 agent，graph 形状是 `start -> react_loop -> final_summary`，`react_loop` 自环直到达到目标；只有在 `minimum_required_tool_calls` 达标、`macro_tool_traces=0`、`repeated_fixed_action_loop=false`，并且真实审计子目标和工具覆盖达标时才返回 `ok=true`。

```text
required report shape - ok=true, actual_tool_calls>=100, provider_tool_traces>=100, macro_tool_traces=0, meaningful_phone_task=true
```

2026-05-31 真实 provider 实跑结果为 `graph=single_graph_loop`、`nodes=start->react_loop->final_summary`、`actual_tool_calls=96/100`、`provider_tool_traces=96`、`macro_tool_traces=0`、`repeated_fixed_action_loop=false`、`task_subgoal_coverage_count=10/10`、`argument_signature_count=61`、`unique_tool_count=13`、`total_tokens=3608111`。工具计数为 `android_device_info=2`、`android_battery=2`、`android_open_agent=3`、`android_ui_state=20`、`android_map_observe=17`、`android_map_view=2`、`android_set_clipboard=3`、`android_show_toast=6`、`android_open_settings=3`、`android_map_search=10`、`android_map_plan_path=3`、`android_global_action=6`、`android_click_text=19`。

旧的 4-tool provider run 只证明 provider/tool loop 通路可用；因为它调用了 `android_map_goal_task` 宏工具，不再符合新口径。模拟器直连默认 endpoint 时遇到 DNS 失败；主机代理可用时把代理地址从 `127.0.0.1` 转成 `10.0.2.2` 后可复现。

## Key 使用约束

不要把真实 API key 写进源码、文档、测试 fixture、session message 或日志。真实运行时应在进程环境中设置：

```powershell
$env:MOBILERUN_STABLE_PREFIX_API_KEY="<existing stable-prefix key>"
$env:MOBILERUN_GENERAL_POOL_API_KEY="<second test key>"
```

本地 runtime 检查脚本：

```powershell
cd agent-smoke
.\scripts\check-key-routing.ps1
```

脚本只记录环境变量槽位、是否存在和长度，不写真实 key。当前 shell 环境四个候选 key 槽位均为空，所以报告会是 `ok=false`；
当两把 key 分别放入 `MOBILERUN_STABLE_PREFIX_API_KEY` 和 `MOBILERUN_GENERAL_POOL_API_KEY` 后，它会验证 stable prefix 和 general pool 使用不同 account/env slot。

如果需要把这两个槽位映射到现有 provider，例如 MiMo/OpenAI-compatible Chat adapter，应在平台 shell 或 runtime
facade 中完成，不把 secret 传回 `agent-core` 的持久化对象。

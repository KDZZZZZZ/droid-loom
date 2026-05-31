# App Map Memory 与 Token 优化实测记录

本文记录当前 `example/mobilerun-agent-boundary` 对 GUI-Explorer 风格地图记忆和 token 降耗策略的实现与验证。
这里的数字来自 deterministic boundary probe，不是真实 provider 账单；token 使用 `chars / 4` 的粗估方式，适合比较同一套上下文策略的相对变化。

## 实现位置

- `example/mobilerun-agent-boundary/src/app_map_memory.rs`
  - `AppMapMemory` 维护可搜索、可更新的 App 地图。
  - `PageNode` 表示页面或稳定 UI state，保存功能过滤后的摘要、关键按钮、输入框、导航入口、危险操作和访问次数。
  - `PageTransition` 记录从一个页面到另一个页面的动作、成功/失败次数和 stale 状态。
  - `local_view()` 只返回当前位置附近几跳的节点、路径和候选 action。
  - `semantic_search()` 支持用 “账号安全”、“订单详情”、“发布页面” 这类语义目标搜索页面。
  - `forget()` 支持按 page、query、stale scope 删除地图记忆。
  - `observe_ui_state()` 每次执行后可用新的 accessibility tree 动态更新页面；路径失败多次后会被标记 stale。

- `example/mobilerun-agent-boundary/src/cross_app_task.rs`
  - 构造一个 Shop -> Notes -> Calendar -> Mail 的跨 App 连续任务。
  - 任务用地图语义搜索和已知路径自动执行 100+ 个工具动作。
  - 每一步记录 “给模型的局部地图视野 token” 和 “整张地图 token” 的估算对比。

- `example/mobilerun-agent-boundary/src/scripted_agent.rs`
  - 在 boundary probe 中输出 `app map memory`、`hundred-step cross-app task` 和 `token optimization effects`。

## 当前验证输出

命令：

```powershell
cargo run -p mobilerun-agent-boundary
```

关键结果：

```text
[supported] app map memory - pages=4, transitions=3, search_hits=1, path_len=2, local_nodes=3, candidate_actions=11, local_tokens=163, full_tokens=207, forget_removed=1, stale_paths_after_failures=1
[simulated] hundred-step cross-app task - completed=true, steps=122, app_switches=41, semantic_searches=10, map_reuse_hits=173, token_savings=71.4%
[supported] token optimization effects - tool_layer_direct=8->6, local_map_tokens=12913 vs full_map_tokens=45140, long_task_token_savings=71.4%
```

## 降耗策略效果

| 策略 | 当前实现 | 验证数字 |
| --- | --- | --- |
| 工具冷热分层 | 高频工具保持 Direct，低频工具降到 Searchable，Hidden 不暴露 | Direct tool schema 从 8 降到 6 |
| 工具预执行 | 对概率高且 `read_only + idempotent + !destructive` 的工具提前执行 | `ui_state`、`search_database` 预执行成功，`click_at` 被拒绝预执行 |
| 前缀冷热分层 | `context_stability.rs` 把 stable prefix 放在最前，volatile observation 放最后 | 输出 `ordered_stability=["stable_prefix", ... "volatile_observation"]` |
| key/account 路由 | stable prefix 走固定 account，一般 prefix 走 general pool | `separate_headers=true`，未写入真实 key |
| Task 绑定上下文 | 子任务只继承依赖任务 artifact，不继承工具噪声 | `final_context_inherits_dependency_artifacts_only` |
| 地图局部视野 | 当前页面附近一跳 + 任务相关搜索命中，不塞整图 | 单次地图视野 163 vs 整图 207 |
| 地图复用 | 百步任务重复使用路径和页面摘要，减少重新探索 | 122 步任务中 `map_reuse_hits=173` |
| 长任务地图 token | 每步只给局部地图视野 | 12913 vs 45140，估算节省 71.4% |

## 仍未完成的真实端验证

当前有两类验证：

1. Boundary 层 122 步模拟，证明 core API、地图记忆、路径规划、工具执行和 token 记录可以闭环。
2. Android 模拟器真实端 smoke，通过 `AgentAccessibilityService.android_ui_state` 更新平台层地图，循环 Settings、Launcher、Agent app，执行 100 步跨 App 操作。

真实端命令：

```powershell
adb install -r agent-smoke\android-shell\app\build\outputs\apk\debug\app-debug.apk
adb shell appops set com.example.agentsmoke SYSTEM_ALERT_WINDOW allow
adb shell settings put secure enabled_accessibility_services com.example.agentsmoke/com.example.agentsmoke.AgentAccessibilityService
adb shell settings put secure accessibility_enabled 1
adb shell am start -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -n com.example.agentsmoke/.MainActivity --es debug_tool android_map_long_task_smoke
```

真实端实测输出：

```text
android_map_long_task_smoke - completed=true, steps=100, cycles=20, app_switches=50, page_count=7, transition_count=54, local_map_tokens=32954, full_map_tokens=85604, token_savings_percent=61.5%
```

这仍是 deterministic smoke，不是由真实 provider 自主规划的用户级业务任务。它证明真实 AccessibilityService 输出已经能进入地图记忆、动态维护页面和路径，并产生百步级 token 降耗记录。

新增 `android_map_goal_task` 作为更接近用户级任务的本地 autonomous verifier：任务目标是生成设备准备简报，跨 Settings、Wi-Fi Settings、Launcher 和 Agent app 执行，记录子目标、工具轨迹样本、语义搜索、路径规划、剪贴板产物和 toast 状态。

```text
android_map_goal_task - completed=true, steps=111, cycles=6, app_switches=18, semantic_searches=24, planned_paths=7, map_reuse_hits=15, page_count=6, transition_count=19, local_map_tokens=10281, full_map_tokens=15608, token_savings_percent=34.1%, session_message_count=224, assistant_tool_call_messages=111, tool_result_messages=111, tool_trace_count=111, probability_graph.tool_call_events=111, probability_graph.transition_count=110
```

`android_map_goal_task` 还会从 111 个 tool-call event 生成真实端概率图。当前报告里的热工具前四个是 `android_map_observe:36`、`android_map_plan_path:24`、`android_open_settings:12`、`android_set_clipboard:7`；可预执行候选是 `android_device_info`、`android_battery`、`android_map_plan_path`。
当前验证脚本会把空地图视为失败：`completed=true` 之外，还要求 page、transition、path reuse、局部视野节点、候选 action、语义搜索命中、完整 session/tool 轨迹和 token savings 均为非零。当前 report 的最终局部视野摘要是 `node_count=6`、`transition_count=19`、`candidate_action_count=5`，语义搜索 `hit_count=4`。

同一报告还包含 `android_map_stale_path_probe`：`pre_stale_path_length=1`，三次失败后 `stale_transition_count=1`，`post_stale_found=false`。这证明旧路径失败后会被标记 stale，并从 planner 结果里移除。
`android_map_stale_page_refresh_probe` 进一步证明旧页面节点可刷新：旧 page 标记 stale 后，再观察同一 stable UI state 得到 `same_page=true`、`replaced_stale_node=true`。

这两组 Android 真实端数字来自可复现脚本：

```powershell
cd agent-smoke
.\scripts\run-android-map-verification.ps1
```

脚本每个任务前会 `force-stop` app，避免上一次运行污染地图内存；输出报告为 `agent-smoke/android-shell/app-map-verification-report.json`，报告不包含 API key。

## Provider 自主入口

`agent-smoke` 的 UniFFI facade 现在支持：

- `AgentCore.prompt_with_tools_limit(input, max_tool_rounds)`：长任务可把 tool-call round 从默认 8 提高到最多 128。
- `AgentResponse.input_tokens/output_tokens/total_tokens`：聚合本次 provider tool loop 的 usage。

Android service 侧新增 debug 入口 `agent_autonomous_map_task`。当前验收口径已经改成 provider-visible 的一百次工具调用：

- 该入口为本次 run 创建一个只注册 primitive phone tools 的临时 `AgentCore`。
- 不注册 `android_map_goal_task`、`android_map_long_task_smoke`、stale probe 这类宏/debug 工具。
- 单次 ReAct graph 形状是 `start -> react_loop -> final_summary`，其中 `react_loop` 自环；循环由模型按观察结果选择下一步，不是固定动作 checklist。
- 报告只有在 `minimum_required_tool_calls` 达标、`macro_tool_traces == 0`、`repeated_fixed_action_loop=false`，并且真实审计子目标和工具覆盖都达标时才是 `ok=true`。

命令形态：

```powershell
adb shell am start -a android.intent.action.MAIN -c android.intent.category.LAUNCHER `
  -n com.example.agentsmoke/.MainActivity `
  --es debug_tool agent_autonomous_map_task `
  --ei agent_max_tool_rounds 32
```

2026-05-31 已用运行时注入的 `MIMO_API_KEY` 跑通真实 provider 自主 run；报告不包含 key 本体，落在：

```text
agent-smoke/android-shell/app-map-autonomous-report.json
```

实测先遇到模拟器直连默认 endpoint DNS 失败；主机可解析 `api.xiaomimimo.com`，因此使用主机代理并把 `127.0.0.1` 转换为模拟器可访问的 `10.0.2.2` 后重跑通过。复现形态：

```powershell
cd agent-smoke
$env:MIMO_API_KEY="<runtime key>"
$env:MIMO_PROXY="http://10.0.2.2:<host-proxy-port>" # optional, only needed when emulator cannot resolve/connect directly
.\scripts\run-android-autonomous-map-task.ps1 -TargetToolCalls 100 -MinimumRequiredToolCalls 95 -ReactSelfCheckInterval 10 -MaxToolRounds 128 -WaitSeconds 900
```

旧的 2026-05-31 真实 provider run 只有 4 条 provider-visible tool trace，其中一条是 `android_map_goal_task` 宏工具。它证明 provider/tool loop 通路可用，但按新口径不再算作百次工具调用任务。

```text
superseded macro run - ok=true, tool_traces=4, tools=[android_map_observe, android_map_goal_task, android_map_view, android_map_plan_path]
```

2026-05-31 已按真实 ReAct graph-loop 口径重跑 provider 测评，报告摘要：

```text
agent_autonomous_map_task - graph=single_graph_loop, nodes=start->react_loop->final_summary, actual_tool_calls=96/100, provider_tool_traces=96, macro_tool_traces=0, repeated_fixed_action_loop=false, task_subgoal_coverage_count=10/10, argument_signature_count=61, unique_tool_count=13, input_tokens=3601443, output_tokens=6668, total_tokens=3608111
```

工具计数：

```text
android_device_info=2, android_battery=2, android_open_agent=3, android_ui_state=20, android_map_observe=17, android_map_view=2, android_set_clipboard=3, android_show_toast=6, android_open_settings=3, android_map_search=10, android_map_plan_path=3, android_global_action=6, android_click_text=19
```

成功覆盖计数：`successful_navigation_tool_calls=12`、`successful_observation_tool_calls=51`、`successful_artifact_tool_calls=9`、`successful_device_read_tool_calls=4`。十个真实审计子目标全部覆盖：baseline、Agent workspace、Settings root、Wi-Fi、Launcher recovery、map memory、raw UI、clipboard artifact、visible status、final verification。实跑后 secret-like scan 检查 `agent-smoke`、`example`、`docs`、`crates`，未发现 `sk-*` 或 `Authorization: Bearer ...` 落盘。

为避免只验证 debug tool，本地单测 `prompt_with_tools_runs_provider_tool_loop_against_mock_chat_api` 使用 mock OpenAI-compatible HTTP server 驱动 `AgentCore.prompt_with_tools_limit`：

```text
mock provider loop - http_chat_requests=2, tool_traces=1, message_count=4, input_tokens=180, output_tokens=22, total_tokens=202
```

这个测试证明 provider adapter、assistant tool_call、platform tool host、tool_result message 回填、final answer 和 usage 聚合路径是闭环的；它不是外部真实 provider run。

输出文件：

```text
agent-smoke/android-shell/app-map-autonomous-report.json
```

脚本只从环境变量读取 key，通过 runtime-only debug extra 注入 APK；结构化参数通过 `debug_input_base64` 传入，避免 adb shell 破坏 JSON。脚本不写源码、不写文档，也不会把 key 写入报告。实跑后用 secret-like scan 检查 `agent-smoke`、`example`、`docs`、`crates`，未发现 `sk-*` 或 `Authorization: Bearer ...` 落盘。

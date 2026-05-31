# 目标完成度审计

本文按当前目标逐项记录证据。结论是：地图记忆、真实 AccessibilityService 工具、百步跨 App 本地验证、token 降耗记录，以及真实 provider 单次 ReAct graph loop 内近百次 provider-visible primitive phone tool calls 均已具备可复现证据。

## 1. 地图记忆

| 要求 | 当前证据 | 状态 |
| --- | --- | --- |
| 记录页面节点 | `AndroidAppMapMemory.observe()` 用 Accessibility tree 建 `PageNode`，保存功能过滤后的摘要、按钮、输入框、导航入口、危险动作 | 已验证 |
| 记录跳转路径 | `recordTransition()` 记录 from/to/action/tool/arguments/success/failure/stale | 已验证 |
| 当前位置视野 | `localView(hops, query)` 只返回当前位置附近节点、可见 transition、候选 action 和 omitted count；真实端最终视野 `node_count=6`、`transition_count=19`、`candidate_action_count=5` | 已验证 |
| 语义搜索地点 | `search(query)` 和 `planPath(query)` 支持从语义目标找到页面并规划路径；真实端最终语义搜索 `hit_count=4` | 已验证 |
| 动态维护地图 | 每次 `android_map_observe` 重新识别 UI state；`android_map_stale_path_probe` 验证失败路径 stale 后不再被 planner 采用；`android_map_stale_page_refresh_probe` 验证旧 page 标记 stale 后同一 stable UI state 可刷新为 `replaced_stale_node=true` | 已验证 |
| 降低重复探索 | `android_map_goal_task` 实测 `planned_paths=7`、`map_reuse_hits=15` | 已验证 |
| forget | `android_map_forget` 支持 page/query/stale；goal task 开始时清理 stale | 已验证 |
| 预构造候选 Action | `candidateActions()` 为点击、输入、Back 生成候选工具调用 | 已验证 |

## 2. 百步跨 App 复杂任务

| 验证 | 命令/脚本 | 当前结果 |
| --- | --- | --- |
| Boundary 层模拟任务 | `cargo run -p mobilerun-agent-boundary` | `completed=true, steps=122, app_switches=41, map_reuse_hits=173, token_savings=71.4%` |
| 真实 Android smoke | `agent-smoke/scripts/run-android-map-verification.ps1` | `android_map_long_task_smoke: completed=true, steps=100, app_switches=50, page_count=7, transition_count=54` |
| 真实 Android 目标任务 | `agent-smoke/scripts/run-android-map-verification.ps1` | `android_map_goal_task: completed=true, steps=111, app_switches=18, semantic_searches=24, planned_paths=7, map_reuse_hits=15, session_message_count=224, tool_trace_count=111, probability_graph.tool_call_events=111` |
| 真实 Android stale path | `agent-smoke/scripts/run-android-map-verification.ps1` | `pre_stale_path_length=1, stale_transition_count=1, post_stale_found=false` |
| 真实 Android stale page refresh | `agent-smoke/scripts/run-android-map-verification.ps1` | `same_page=true, replaced_stale_node=true` |
| 真实 provider 近百 tool-call Android run | `agent-smoke/scripts/run-android-autonomous-map-task.ps1` | `graph=single_graph_loop, nodes=start->react_loop->final_summary, actual_tool_calls=96/100, provider_tool_traces=96, macro_tool_traces=0, repeated_fixed_action_loop=false, subgoal_coverage=10/10, argument_signature_count=61, unique_tool_count=13` |
| Provider tool loop mock | `cargo test -p agent-smoke prompt_with_tools_runs_provider_tool_loop_against_mock_chat_api` | `http_chat_requests=2, tool_traces=1, message_count=4, total_tokens=202` |
| Key routing provider-prepare mock | `cargo test -p mobilerun-agent-boundary provider_prepare_keeps_route_keys_in_headers_not_metadata` | stable/general fake keys produce different `Authorization` headers; fake keys do not appear in request metadata, prepared metadata, or provider body |

`android_map_goal_task` 的任务目标是生成设备准备简报：跨 Settings、Wi-Fi Settings、Launcher 和 Agent app，读取设备/电池状态，观察页面，规划路径，写剪贴板产物并显示 toast 状态。它不是 provider 自主规划，但是真实 AccessibilityService phone-using 工具链上的 autonomous verifier。

真实 provider 单次 ReAct graph-loop run 使用运行时 `MIMO_API_KEY` 注入，不把 key 写入源码、文档或报告。该 run 的 graph 是 `start -> react_loop -> final_summary`，`react_loop` 自环；最终产生 `actual_tool_calls=96`、`provider_tool_traces=96`、`macro_tool_traces=0`。它不是固定动作循环：`repeated_fixed_action_loop=false`、`task_subgoal_coverage_count=10/10`、`argument_signature_count=61`、`unique_tool_count=13`。工具覆盖为 `navigation_tool_calls=12`、`observation_tool_calls=52`、`artifact_tool_calls=9`、`device_read_tool_calls=4`。
mock provider 单测不依赖外部 key，但会完整经过 OpenAI-compatible request mapper、assistant tool_call、platform tool host、tool_result message 回填和 final answer。它继续作为可离线复现的 provider-loop 回归测试。
key routing provider-prepare 单测同样只使用 fake key，用来证明稳定 prefix 与通用 prefix 的请求头隔离，并确认 secret 不进入 metadata/body 这类可缓存或可持久化区域。

## 3. Token 降耗记录

| 策略 | 当前结果 |
| --- | --- |
| 概率图工具冷热分层 | boundary probe: `tool_layer_direct=8->6` |
| 工具预执行 | boundary probe: `preexecuted=2`，危险点击被跳过 |
| 上下文稳定性排序 | boundary probe: `stable_prefix -> task_package -> dependency_result -> volatile_observation` |
| Task 绑定上下文 | boundary probe: `final_context_inherits_dependency_artifacts_only` |
| 地图局部视野 | boundary 122 步任务: `12913 vs 45140`, 节省 `71.4%` |
| 真实 Android smoke | `32954 vs 85604`, 节省 `61.5%` |
| 真实 Android goal task | `10281 vs 15608`, 节省 `34.1%` |

## 仍未完成

当前没有把真实 key 持久化到 shell 环境。剩余缺真实运行证据的是：

- 两把真实 key 分别进入 stable-prefix account 和 general-pool account 后的路由实跑。

相关脚本已就绪，并且只写脱敏报告：

```powershell
cd agent-smoke
.\scripts\run-android-autonomous-map-task.ps1
.\scripts\check-key-routing.ps1
```

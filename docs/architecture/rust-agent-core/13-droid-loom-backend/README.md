# Droid Loom Backend 分支集成说明

日期：2026-05-30

## 功能定位

本文件说明 `backend` 分支如何使用新的 `crates/agent-core` 替换原来写在 `agent-smoke/src/lib.rs` 里的临时 agent core。

当前实现不是把 Android 平台能力塞进 core，而是拆成三层：

- `crates/agent-core`：跨平台 core，负责 agent definition、message/content block、context 构造、LLM provider adapter、tool schema/result、graph、hook、session 等稳定边界。
- `agent-smoke/src/lib.rs`：Android-facing UniFFI shell，负责暴露 Kotlin 能调用的 `AgentCore`、`PlatformToolHost`、`ToolSpec`、`AgentResponse`，并把 Android 注册的工具桥接到 shared core 的 message/provider 协议。
- `agent-smoke/android-shell`：常驻 Android host，包含 foreground service、全局悬浮球和 AccessibilityService。真正跨 App 的 screen read/click/type/back/home 能力在这一层实现，不进入 `agent-core`。

这意味着后续 macOS、desktop、server runtime 不应该复制 `agent-smoke/src/lib.rs` 的业务逻辑，而应该直接复用 `crates/agent-core`，再各自写很薄的平台 shell。

## 文件架构

### 根目录

- `Cargo.toml`
  - 职责：定义 workspace，把 `agent-smoke` 和 `crates/agent-core` 放进同一个构建单元。
  - 交互：`agent-smoke/Cargo.toml` 通过 workspace dependency 引用 `agent-core`。
  - 使用者：本地开发、CI、Android native lib 构建脚本。

- `Cargo.lock`
  - 职责：锁定 workspace Rust 依赖版本。
  - 交互：由 Cargo 生成，提交后保证 Android native lib 和本地 smoke test 依赖一致。
  - 使用者：CI、本地构建。

### `crates/agent-core`

职责见 [12-source-map](../12-source-map/README.md)。这里不重复定义 core 文件，只强调 Android 集成用到的核心路径：

- `src/message/content_block.rs` 和 `src/message/run_message.rs`
  - Android shell 用它们保存多轮对话、assistant tool call、tool result。

- `src/llm/context.rs`
  - Android shell 用 `ContextBuilder` 把 `AgentDefinition + RunMessage + ToolSchema` 转成 provider-neutral `LlmRequest`。

- `src/llm/deepseek_chat.rs`
  - Android shell 复用 `DeepSeekChatProvider` 作为 OpenAI-compatible Chat Completions request mapper；默认 endpoint 指向 MiMo `https://api.xiaomimimo.com/v1/chat/completions`，并把非流式 response 归一化成 `LlmStreamEvent`。

- `src/message/assistant_builder.rs`
  - Android shell 用它把 `LlmStreamEvent` 汇总成 finalized assistant `RunMessage`。

- `src/tool/schema.rs`
  - Android shell 注册 Kotlin 工具时用它校验 schema，并生成 provider 可见 tool schema。

### `agent-smoke`

- `agent-smoke/Cargo.toml`
  - 职责：定义 UniFFI cdylib/rlib 和 CLI binary，并依赖 shared `agent-core`。
  - 交互：Android Gradle 打包 `libagent_smoke.so`；UniFFI binding 读取该 crate 的导出接口。
  - 使用者：Android shell、desktop smoke、UniFFI 生成器。

- `agent-smoke/src/lib.rs`
  - 职责：Android-facing compatibility shell。
  - 内部逻辑：
    1. `AgentCore` 保存 API base/key/model、shared `AgentDefinition`、HTTP client、in-memory `RunMessage` state、已注册 tool specs 和可选 `PlatformToolHost`。
    2. `register_tool()` 只校验 JSON schema 并保存 tool spec，不实现具体 Android 工具。
    3. `prompt()` 把用户文本包成 `RunMessage(role=user)`，通过 `ContextBuilder + DeepSeekChatProvider` 调用 MiMo/OpenAI-compatible 模型，把 response events 汇总成 assistant message。
    4. `prompt_with_tools()` 最多跑 8 个 smoke tool-call 回合；当模型返回 tool call 时，通过 `PlatformToolHost.execute_tool()` 回调 Kotlin，拿到 JSON 结果后包装成 `ContentBlock::ToolResult` 再回填给模型。
    5. `reset()` 只清理 shell 内存消息，不清理 Kotlin 工具注册表。
  - 与其他文件交互：
    - 调用 `crates/agent-core` 的 message/context/provider/schema/builder。
    - 被 `bindings/kotlin/.../agent_smoke.kt` 调用。
    - 通过 `PlatformToolHost` 调回 `MainActivity.kt` 中的 Android tool host。
  - 使用者：Android Kotlin App、desktop smoke CLI、后续移动端平台壳。

- `agent-smoke/src/main.rs`
  - 职责：桌面 smoke CLI。
  - 逻辑：优先读取 `MIMO_API_KEY`、`MIMO_MODEL`、`MIMO_API_BASE` 等环境变量，兼容 fallback 到 `DEEPSEEK_*`，构造 `AgentCore` 并执行一次 prompt。
  - 使用者：本地验证、CI smoke。不要在这里写平台工具。

- `agent-smoke/bindings/kotlin/uniffi/agent_smoke/agent_smoke.kt`
  - 职责：UniFFI 自动生成 Kotlin binding。
  - 逻辑：把 Rust object/record/error/foreign trait 暴露给 Kotlin。
  - 使用者：Android app。它是生成文件，改 Rust 导出接口后重新生成，不手写业务逻辑。

- `agent-smoke/android-shell/app/src/main/kotlin/com/example/agentsmoke/MainActivity.kt`
  - 职责：Android host 设置入口。
  - 内部逻辑：
    1. 启动 `AgentHostService`。
    2. 引导用户授予 overlay 权限和开启 `AgentAccessibilityService`。
    3. 可以通过 intent extra 注入 debug-only provider 配置和初始 prompt。
  - 使用者：Android demo、模拟器验证。不要把 agent loop 逻辑写回 Activity。

- `agent-smoke/android-shell/app/src/main/kotlin/com/example/agentsmoke/AgentHostService.kt`
  - 职责：常驻 agent runtime host。
  - 内部逻辑：
    1. 以前台服务运行，持有 Rust `AgentCore`、tool registry 和 provider 配置。
    2. 创建全局悬浮球，用户点击后能输入命令。
    3. 注册 Android 工具：`android_device_info`、`android_battery`、`android_show_toast`、`android_set_clipboard`、`android_open_settings`、`android_ui_state`、`android_click_text`、`android_type_text`、`android_global_action`。
    4. 将工具调用分发到 Android API 或 `AgentAccessibilityService`。
  - 使用者：真正 phone-using demo。Service 是 agent runtime 的生命周期主体，Activity 只是设置入口。

- `agent-smoke/android-shell/app/src/main/kotlin/com/example/agentsmoke/AgentAccessibilityService.kt`
  - 职责：跨 App UI 观察和动作执行。
  - 内部逻辑：
    1. `screenSnapshot()` 优先从 `service.windows` 采集多窗口 UI tree，悬浮球常驻时仍能看到底层 App；没有窗口列表时 fallback 到 `rootInActiveWindow`。
    2. `clickText()` 遍历多窗口，按 text/contentDescription 找最近可点击节点并执行 click。
    3. `typeText()` 遍历多窗口，对 focused/editable 节点执行 `ACTION_SET_TEXT`。
    4. `performGlobal()` 执行 back/home/recents。
    5. AccessibilityService 工具统一切回主线程执行，避免后台 executor 线程读到不完整窗口状态。
  - 使用者：Service 中的 phone-using tools。没有开启无障碍服务时，工具返回 `accessibility service is not enabled`。

- `agent-smoke/android-shell/app/build.gradle.kts`
  - 职责：Android app 构建配置。
  - 逻辑：启用 BuildConfig，把环境变量注入 debug APK，并把 UniFFI Kotlin binding 目录加入 source set。
  - 使用者：Gradle/Android Studio。

- `agent-smoke/scripts/build-android-libs.ps1`
  - 职责：交叉编译 Rust cdylib 到 Android ABI。
  - 逻辑：读取 `ANDROID_HOME`，定位 NDK LLVM toolchain，构建 `aarch64-linux-android` 和 `x86_64-linux-android`，复制 `.so` 到 `jniLibs`。
  - 使用者：本地 Android 构建、CI。workspace 下必须显式使用 `--manifest-path Cargo.toml --target-dir target`，避免根 target 目录破坏脚本复制路径。

## 使用方案

### 1. 本地验证 core API

从仓库根目录执行：

```powershell
cargo fmt --check
cargo check -p agent-smoke
cargo test -p agent-core
cargo run -p agent-core --example public_api_smoke
```

这些命令不需要 API key，用来验证 shared core、UniFFI shell 和 public API 示例。

### 2. 验证 OpenAI-compatible Chat request adapter

从仓库根目录执行：

```powershell
cargo run -p agent-core --example deepseek_prepare_request
```

这个旧示例仍以 DeepSeek 命名，用于验证 Chat Completions mapper。MiMo 真实请求通过 `agent-smoke` 验证。

### 3. 桌面跑 agent-smoke

先在本机环境变量设置 key，不要写进仓库文件：

```powershell
$env:MIMO_API_KEY="<your key>"
$env:MIMO_MODEL="mimo-v2.5-pro"
$env:MIMO_API_BASE="https://api.xiaomimimo.com/v1"
cargo run -p agent-smoke -- "Say hello in one short sentence."
```

### 4. 生成 Kotlin binding

从仓库根目录执行：

```powershell
cargo build -p agent-smoke
uniffi-bindgen generate --library target\debug\agent_smoke.dll --language kotlin --out-dir agent-smoke\bindings\kotlin
```

如果本机没有 `ktlint`，生成器会提示无法自动格式化，但 binding 仍会生成。

### 5. 构建 Android native libs

```powershell
$env:ANDROID_HOME="D:\Android\Sdk"
$env:ANDROID_SDK_ROOT="D:\Android\Sdk"
cd agent-smoke
.\scripts\build-android-libs.ps1
```

输出：

```text
agent-smoke/android-shell/app/src/main/jniLibs/arm64-v8a/libagent_smoke.so
agent-smoke/android-shell/app/src/main/jniLibs/x86_64/libagent_smoke.so
```

### 6. 构建 APK

```powershell
$env:ANDROID_HOME="D:\Android\Sdk"
$env:ANDROID_SDK_ROOT="D:\Android\Sdk"
$env:GRADLE_USER_HOME="D:\Android\gradle-cache"
$env:MIMO_API_KEY="<your key>"
$env:MIMO_MODEL="mimo-v2.5-pro"
$env:MIMO_API_BASE="https://api.xiaomimimo.com/v1"
$env:MIMO_PROXY="http://10.0.2.2:11304"

cd agent-smoke\android-shell
gradle --no-daemon :app:assembleDebug
```

`MIMO_PROXY` 只在模拟器访问宿主机代理时需要。真实设备可留空。

### 7. 模拟器运行

```powershell
$env:ANDROID_HOME="D:\Android\Sdk"
$env:ANDROID_SDK_ROOT="D:\Android\Sdk"
$env:ANDROID_AVD_HOME="D:\Android\avd"

Start-Process D:\Android\Sdk\emulator\emulator.exe `
  -ArgumentList @("-avd","agent_smoke_api35","-gpu","swiftshader_indirect") `
  -WindowStyle Hidden

D:\Android\Sdk\platform-tools\adb.exe wait-for-device
D:\Android\Sdk\platform-tools\adb.exe install -r agent-smoke\android-shell\app\build\outputs\apk\debug\app-debug.apk
D:\Android\Sdk\platform-tools\adb.exe shell am start -n com.example.agentsmoke/.MainActivity
D:\Android\Sdk\platform-tools\adb.exe exec-out uiautomator dump /dev/tty
```

如果 APK 构建时没有注入 `MIMO_API_KEY`，UI 应显示 `Missing MIMO_API_KEY`。这说明 Android shell 能启动，但没有跑真实模型调用。

本地 debug 也可以通过 intent extra 临时注入 provider 配置，避免把 key 写进 Gradle 文件或源码：

```powershell
D:\Android\Sdk\platform-tools\adb.exe shell am start `
  -n com.example.agentsmoke/.MainActivity `
  --es mimo_api_key "<your key>" `
  --es mimo_api_base "https://api.xiaomimimo.com/v1" `
  --es mimo_model "mimo-v2.5-pro" `
  --es agent_prompt "Run android_device_info, android_battery, android_set_clipboard, and android_show_toast. Summarize in Chinese."
```

这个入口只允许本地 smoke 使用。app UI 不显示 key；不要把带 key 的命令写进文档、脚本或 shell history。

### 8. 复杂 phone-using 验证

已在 Android 15 模拟器 `sdk_gphone64_x86_64` 上验证一个多工具任务：

```powershell
$prompt='Run android_device_info, android_battery, android_set_clipboard with text complex phone task done, and android_show_toast with message complex phone task done. Use every requested tool, then summarize the device state and actions in Chinese.'
adb shell "am start -n com.example.agentsmoke/.MainActivity --es agent_prompt '$prompt'"
```

期望 UI 显示 `OK: Android platform tools demo`，并包含这些 tool traces：

- `android_device_info`
- `android_battery`
- `android_set_clipboard`
- `android_show_toast`

当前实测记录：

- `model=mimo-v2.5-pro`
- `message_count=7`
- `android_device_info` 返回 `manufacturer=Google`、`model=sdk_gphone64_x86_64`、`sdk_int=35`、`release=15`
- `android_battery` 返回 `level_percent=100`、`charging=false`
- `android_set_clipboard` 成功写入 `trajectory_cache_probe`
- `android_show_toast` 成功显示 `agent_core_smoke`
- assistant 返回中文总结并列出四个成功工具

这确认 Rust shell、shared `agent-core` message/tool-result 路径、Kotlin platform host、MiMo provider 回合和模拟器平台工具能串通。

旧 Activity-bound demo 中，`android_open_settings` 会把 app 切到后台，后续 provider 回合可能连接中断。现在 runtime 已迁到 `AgentHostService`，打开 Settings 后 Service 继续持有 agent/session/tool queue；真正的跨 App 观察和动作由 `AgentAccessibilityService` 提供。

要验证悬浮球和无障碍工具：

```powershell
adb install -r agent-smoke\android-shell\app\build\outputs\apk\debug\app-debug.apk
adb shell appops set com.example.agentsmoke SYSTEM_ALERT_WINDOW allow
adb shell settings put secure enabled_accessibility_services com.example.agentsmoke/com.example.agentsmoke.AgentAccessibilityService
adb shell settings put secure accessibility_enabled 1
adb shell am start -n com.example.agentsmoke/.MainActivity
```

启动后应看到前台服务通知；授权 overlay 后应出现悬浮球。通过悬浮球输入命令可让 agent 调用 `android_ui_state`、`android_click_text`、`android_type_text`、`android_global_action` 等真正跨 App 工具。本地 smoke test 也可以通过 `debug_tool` intent 走不依赖模型 key 的工具路径。

地图记忆工具也在 Android service 层注册：

- `android_map_observe`：用当前 AccessibilityService UI tree 更新 App 地图。
- `android_map_view`：只返回当前位置附近的局部地图视野。
- `android_map_search`：按语义目标搜索页面。
- `android_map_plan_path`：按语义目标搜索目标页面，并从当前位置沿已记录 transition 规划路径。
- `android_map_mark_transition_failed`：把某个 action path 标记为失败，连续失败后自动 stale。
- `android_map_forget`：按 page/query/stale 删除地图记忆。
- `android_map_click_text` / `android_map_type_text`：执行动作后刷新地图，并记录页面跳转。
- `android_map_long_task_smoke`：不依赖模型 key，跑真实模拟器 100 步跨 App smoke。
- `android_map_goal_task`：不依赖模型 key，跑目标驱动的设备准备简报任务，内部记录子目标、工具轨迹、路径规划和 token 节省。
- `android_map_stale_path_probe`：验证路径失败后 stale，stale 路径不再被 planner 采用。

当前真实端 `android_map_long_task_smoke` 实测：

```text
completed=true, steps=100, cycles=20, app_switches=50, page_count=7, transition_count=54, local_map_tokens=32954, full_map_tokens=85604, token_savings_percent=61.5%
```

当前真实端 `android_map_goal_task` 实测：

```text
completed=true, steps=111, cycles=6, app_switches=18, semantic_searches=24, planned_paths=7, map_reuse_hits=15, page_count=6, transition_count=19, local_map_tokens=10281, full_map_tokens=15608, token_savings_percent=34.1%, session_message_count=224, assistant_tool_call_messages=111, tool_result_messages=111, tool_trace_count=111, probability_graph.tool_call_events=111, probability_graph.transition_count=110
```
同一 report 的最终局部视野摘要是 `node_count=6`、`transition_count=19`、`candidate_action_count=5`，语义搜索 `hit_count=4`。

`AgentAccessibilityService.screenSnapshot()` 会在非主线程调用时短重试，处理 force-stop 后服务已启用但尚未 bind 的启动期窗口；验证脚本同时要求 page、transition、path reuse、局部视野节点、候选 action、语义搜索命中和 token savings 均为非零，避免把空地图误判为通过。

同一验证脚本还会跑 `android_map_stale_path_probe`：`pre_stale_path_length=1`，三次失败后 `stale_transition_count=1`，`post_stale_found=false`。
`android_map_stale_page_refresh_probe` 验证页面节点刷新：旧 page 标记 stale 后，再观察同一 stable UI state 得到 `same_page=true`、`replaced_stale_node=true`。

Provider 自主入口：

- `AgentCore.prompt_with_tools_limit(input, max_tool_rounds)` 已通过 UniFFI 暴露给 Android。
- `AgentResponse` 返回本次 provider loop 的 `input_tokens`、`output_tokens`、`total_tokens`。
- `debug_tool=agent_autonomous_map_task` 会让模型在单次 ReAct graph loop 中调用 primitive Android phone tools，目标是接近 100 条 provider-visible tool trace；graph 形状是 `start -> react_loop -> final_summary`，其中 `react_loop` 自环。该入口不注册 `android_map_goal_task`、`android_map_long_task_smoke` 等宏/debug tools。

新验收要求：`target_tool_calls=100`、默认 `minimum_required_tool_calls=95`、`macro_tool_traces=0`、`repeated_fixed_action_loop=false`，并且真实审计子目标、导航、观察、设备读取、产物工具都有覆盖。2026-05-31 真实 provider 实跑结果为 `actual_tool_calls=96/100`、`provider_tool_traces=96`、`macro_tool_traces=0`、`task_subgoal_coverage_count=10/10`、`total_tokens=3608111`。旧的 4-tool provider run 只证明通路可用，不再计入百次 phone-using 任务证据。当模拟器直连默认 endpoint DNS 失败时，可把主机代理地址从 `127.0.0.1` 转成 `10.0.2.2` 后通过 `MIMO_PROXY` 传给 App。

## 可拓展点

- 增加更通用的 provider base URL 注入和 provider id 命名，避免 MiMo/OpenAI-compatible Chat 继续借用 `DeepSeekChatProvider` 名称。
- 把 `prompt_with_tools()` 从 blocking HTTP 迁移到 async/background queue，避免长请求占用移动端线程。
- 在 UniFFI API 增加 event stream：`token_delta`、`tool_call`、`tool_result`、`agent_done`。
- 把 `PlatformToolHost` 扩展成 permission-aware host：每个 tool call 可要求 Android runtime permission 或用户确认。
- 后续 macOS shell 只实现自己的 host，不复制 Android tool 分发逻辑。

## 使用约束

- 不要把 API key 写入源码、文档、Gradle 文件或生成 binding。
- 不要在 Kotlin 里重新实现 agent loop；Kotlin 只做 UI、工具注册和平台能力执行。
- 不要在 `agent-core` 里引用 Android SDK 类型。
- 不要手改 UniFFI 生成文件里的业务逻辑；改 Rust 导出接口后重新生成。
- 新增 Android tool 时，只改 Kotlin host 和注册 schema；shared core 只看到 tool schema 和 tool result。

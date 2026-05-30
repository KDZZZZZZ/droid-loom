# Agent Android/Rust Architecture Feasibility Proof

本文说明当前 `agent-smoke` PoC 如何证明下面这条架构是可行的：

```text
                ┌────────────────────────┐
                │ Android App            │
                │ Kotlin                 │
                └───────────┬────────────┘
                            │ UniFFI bindings
                ┌───────────▼────────────┐
                │      Rust Agent Core    │
                │ workflow / state / ReAct│
                │ tools / memory / events │
                └───────┬─────────┬──────┘
                        │         │
             Rust HTTP client     Platform shell
             OpenAI/Claude/etc    Android/macOS abilities
```

## 结论

当前实现已经证明该架构在技术上可行：

1. Android/Kotlin 可以通过 UniFFI 调用 Rust `AgentCore`。
2. Rust `AgentCore` 可以维护 agent state，并通过 Rust HTTP client 调用 DeepSeek `deepseek-v4-pro`。
3. Rust 可以暴露工具注册表和 `prompt_with_tools` tool-call 循环。
4. Rust 可以通过 UniFFI foreign trait 回调 Kotlin，由 Kotlin 执行 Android 平台能力。
5. 模拟器上已经跑通完整链路：Android App -> UniFFI -> Rust AgentCore -> DeepSeek -> tool calls -> Rust -> Kotlin Android tools -> Rust -> DeepSeek -> Android UI。

这证明了“Rust 作为可迁移 Agent Core，Android/macOS 作为平台能力适配层”的核心方向成立。

## 当前实现位置

核心代码：

- Rust Agent Core: `agent-smoke/src/lib.rs`
- UniFFI 生成的 Kotlin binding: `agent-smoke/bindings/kotlin/uniffi/agent_smoke/agent_smoke.kt`
- Android demo: `agent-smoke/android-shell/app/src/main/kotlin/com/example/agentsmoke/MainActivity.kt`
- Android APK: `agent-smoke/android-shell/app/build/outputs/apk/debug/app-debug.apk`

Rust Android native libraries：

- `agent-smoke/android-shell/app/src/main/jniLibs/arm64-v8a/libagent_smoke.so`
- `agent-smoke/android-shell/app/src/main/jniLibs/x86_64/libagent_smoke.so`

## 架构映射

| 原始架构节点 | 当前实现 | 证明点 |
| --- | --- | --- |
| Android App / Kotlin | `MainActivity.kt` | Android App 能启动、注册工具、展示结果 |
| UniFFI bindings | `bindings/kotlin/uniffi/agent_smoke/agent_smoke.kt` | Kotlin 能调用 Rust object、Rust record、Rust error、foreign trait |
| Rust Agent Core | `AgentCore` | Rust 维护 state、tools、host、HTTP client |
| workflow / state | `AgentState { messages }` | 多轮消息进入 Rust state，最终 `message_count: 7` |
| ReAct / tools | `prompt_with_tools` + `ToolTrace` | LLM 返回 tool calls，Rust 执行工具，再把结果回填给 LLM |
| Rust HTTP client | `reqwest::blocking::Client` | Rust 直接调用 DeepSeek chat completions |
| Platform shell | `PlatformToolHost` + `AndroidPlatformToolHost` | Rust 调回 Kotlin，Kotlin 执行 Android API |
| Android 能力 | device info / battery / toast / clipboard / settings | 模拟器上真实调用 Android API |

## 已跑通的完整调用链

当前 demo 启动后执行下面的流程：

```text
MainActivity.onCreate()
  -> createAgent()
  -> AgentCore.new_with_base_url_and_proxy(...)
  -> agent.setPlatformToolHost(AndroidPlatformToolHost)
  -> agent.registerTool(...)
  -> agent.promptWithTools(...)
  -> Rust sends messages + tool schemas to DeepSeek
  -> DeepSeek returns tool_calls
  -> Rust executes registered tool by name
  -> Rust calls PlatformToolHost.executeTool(...)
  -> Kotlin AndroidPlatformToolHost executes Android API
  -> Kotlin returns JSON tool result to Rust
  -> Rust appends tool result into AgentState
  -> Rust asks DeepSeek for final answer
  -> Android UI renders tool traces + assistant answer
```

这条链路证明了两个最关键的边界都能工作：

- Kotlin -> Rust：Android App 调用 Rust Agent Core。
- Rust -> Kotlin：Rust Agent Core 反向调用平台能力。

后者尤其重要，因为 Android/macOS 能力不应该写死在 Rust core 里。当前的 `PlatformToolHost` 证明了 Rust 可以只定义抽象接口，具体平台用 Kotlin、Swift、或其他 shell 实现。

## 当前注册的 Android tools

Android demo 注册了 5 个工具：

| Tool | Android 实现 | 作用 |
| --- | --- | --- |
| `android_device_info` | `Build.*` + `activity.packageName` | 读取设备/模拟器信息 |
| `android_battery` | `ACTION_BATTERY_CHANGED` + `BatteryManager` | 读取电量和充电状态 |
| `android_show_toast` | `Toast.makeText(...)` | 在屏幕上显示 Toast |
| `android_set_clipboard` | `ClipboardManager` | 写入 Android 剪贴板 |
| `android_open_settings` | `Intent(Settings.ACTION_*)` | 打开系统设置页 |

demo 自动调用前 4 个工具。`android_open_settings` 已注册但没有自动调用，因为它会跳出 demo 结果页，影响验证输出展示。

## 模拟器验证结果

模拟器上已经安装并启动 APK，UI dump 中看到如下结果：

```text
OK: Android platform tools demo
model: deepseek-v4-pro
message_count: 7

registered tools:
- android_device_info
- android_battery
- android_show_toast
- android_set_clipboard
- android_open_settings

tool traces:
tool: android_device_info
output: {"ok":true,"manufacturer":"Google","model":"sdk_gphone64_x86_64","sdk_int":35,"release":"15","package":"com.example.agentsmoke"}

tool: android_battery
output: {"ok":true,"level_percent":100,"status":2,"plugged":1,"charging":true}

tool: android_show_toast
output: {"ok":true,"shown":true,"message":"Tool call worked"}

tool: android_set_clipboard
output: {"ok":true,"text":"copied by Rust Agent"}
```

这个结果说明：

- APK 能在 Android 15 x86_64 模拟器运行。
- Kotlin 能加载 `libagent_smoke.so`。
- Rust 能通过 UniFFI 接收 Kotlin 注册的工具。
- DeepSeek 能按工具 schema 生成 tool calls。
- Rust 能根据 tool call name 和 arguments 调用 Android host。
- Android host 能真实执行平台 API 并返回 JSON。
- Rust 能把工具结果带回 LLM 并生成最终回答。

## 为什么这能证明你原始架构可行

### 1. Rust core 可以成为跨平台 Agent 核心

`AgentCore` 不依赖 Android SDK。它只依赖：

- HTTP client
- JSON serialization
- UniFFI exported objects/records/errors
- `PlatformToolHost` 抽象接口

这意味着迁移到 macOS 时，Rust core 可以复用；需要替换的是平台 shell：

```text
Android: Kotlin implements PlatformToolHost
macOS: Swift implements PlatformToolHost
Desktop: another host implements PlatformToolHost
```

### 2. Android 能力不污染 Rust core

Rust core 不知道 `Toast`、`ClipboardManager`、`Intent`、`BatteryManager` 这些 Android 类型。Rust 只知道：

```text
execute_tool(name, input_json) -> output_json
```

这个边界足够稳定，后续可以扩展到：

- 文件选择器
- 通知
- 地理位置
- 相机
- 无障碍能力
- App 内导航
- macOS shell / AppleScript / Shortcuts

### 3. UniFFI 支持双向调用

本项目不仅验证了 Kotlin 调 Rust，也验证了 Rust 反向调 Kotlin。

这比简单 FFI demo 更接近真实 Agent 架构，因为 tool execution 本质上需要 Rust core 调用平台能力。`PlatformToolHost` foreign trait 正是这一点的证明。

### 4. LLM tool-call 循环已经成立

`prompt_with_tools` 不是手写顺序调用工具，而是把工具 schema 提交给模型，由模型返回 tool calls。Rust core 再执行工具并把结果作为 tool message 回填。

这证明 workflow/ReAct 层可以放在 Rust core 中，而平台能力只作为工具实现被调用。

### 5. Android 构建链路也成立

已经完成：

- Rust 编译为 Windows DLL，用于生成 Kotlin binding。
- Rust 交叉编译为 Android `arm64-v8a` 和 `x86_64` `.so`。
- Android Gradle 项目打包 `.so` 和 UniFFI Kotlin binding。
- APK 安装到模拟器并运行。

这说明不是只在桌面 JVM 上模拟，而是真的到了 Android runtime。

## 当前没有证明的部分

这份 PoC 证明了架构可行，但还不是生产级实现。

未完成或只做了最小实现的部分：

| 能力 | 当前状态 | 后续需要 |
| --- | --- | --- |
| memory persistence | 只有内存里的 `AgentState.messages` | SQLite/文件/加密存储 |
| event stream | 只有最终 `ToolTrace` | 流式事件、UI 实时更新、取消机制 |
| permission model | demo 只用低风险 API | Android runtime permission、tool allowlist |
| tool schema validation | 只验证 JSON 格式 | 严格 JSON Schema 校验 |
| secret handling | debug build 通过 env 注入 key | 正式应改为后端代理或系统安全存储 |
| async runtime | 当前用 blocking HTTP | 移动端生产建议改 async 或后台队列 |
| model provider abstraction | 当前只接 DeepSeek/OpenAI-compatible | provider trait、Claude/OpenAI/本地模型适配 |
| error recovery | 最小错误返回 | 重试、超时、网络状态检测 |

## 如何复验

启动模拟器：

```powershell
$env:ANDROID_HOME='D:\Android\Sdk'
$env:ANDROID_SDK_ROOT='D:\Android\Sdk'
$env:ANDROID_AVD_HOME='D:\Android\avd'

Start-Process D:\Android\Sdk\emulator\emulator.exe `
  -ArgumentList @('-avd','agent_smoke_api35','-gpu','swiftshader_indirect') `
  -WindowStyle Hidden
```

安装并启动 APK：

```powershell
D:\Android\Sdk\platform-tools\adb.exe wait-for-device
D:\Android\Sdk\platform-tools\adb.exe install -r agent-smoke\android-shell\app\build\outputs\apk\debug\app-debug.apk
D:\Android\Sdk\platform-tools\adb.exe shell am start -n com.example.agentsmoke/.MainActivity
```

检查屏幕文本：

```powershell
D:\Android\Sdk\platform-tools\adb.exe exec-out uiautomator dump /dev/tty
```

预期能看到：

```text
OK: Android platform tools demo
registered tools:
tool traces:
android_device_info
android_battery
android_show_toast
android_set_clipboard
```

重新构建 APK：

```powershell
$env:ANDROID_HOME='D:\Android\Sdk'
$env:ANDROID_SDK_ROOT='D:\Android\Sdk'
$env:GRADLE_USER_HOME='D:\Android\gradle-cache'
$env:DEEPSEEK_API_KEY='your_api_key'
$env:DEEPSEEK_MODEL='deepseek-v4-pro'
$env:DEEPSEEK_PROXY='http://10.0.2.2:11304'

cd agent-smoke\android-shell
gradle --no-daemon :app:assembleDebug
```

`DEEPSEEK_PROXY` 是为 Android 模拟器访问宿主机代理准备的。模拟器里 `10.0.2.2` 指向宿主机。

## 安全说明

当前 debug APK 是通过环境变量把 API key 注入 `BuildConfig` 后构建的，只适合本地验证，不应该分发。

生产方案建议：

1. Android App 不直接携带 LLM API key。
2. App 调自己的后端，由后端持有模型 key。
3. 如果必须本地直连，至少使用 Android Keystore/系统凭据方案，并加上 key rotation 和权限控制。
4. 工具调用必须有 allowlist、用户确认、权限审计和敏感操作分级。

## 下一步建议

下一阶段可以把 PoC 收敛成更接近产品形态的 Agent runtime：

1. 把 `PlatformToolHost` 拆成 `ToolRegistry`、`ToolExecutor`、`EventSink` 三个接口。
2. 把 `AgentState` 抽象为可持久化 state，先落 SQLite。
3. 增加 tool permission policy：只读工具自动执行，写操作需要用户确认。
4. 增加 streaming/event API：LLM token、tool_start、tool_result、agent_done。
5. 把 DeepSeek/OpenAI-compatible client 抽成 provider trait，方便接 OpenAI、Claude、Gemini 或本地模型。


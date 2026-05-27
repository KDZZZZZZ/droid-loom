# DroidLoom 技术调研资料

日期：2026-05-27

## 1. 项目判断

DroidLoom 的核心不是“让 LLM 随机点屏幕”，而是把 Android 可观察能力、可执行能力、用户定义工作流、LLM 推理和编译优化放进同一条受控链路：

```text
屏幕/API 观察 -> 标准化状态 -> 工作流 IR -> Planner/Agent -> 受保护动作 -> Verifier -> Trace
                                      |                         |
                                      +-> 编译优化 Pass         +-> 本地 LLM 后端
```

推荐路线：

- Android 端使用 Kotlin、Jetpack Compose 和前后台服务分层。
- 屏幕语义优先来自 AccessibilityService 的节点树，像素截图只作为补充通道。
- 操作优先使用稳定的 Android API、Intent、Shortcut、通知 action 和 Accessibility action；手势点击只作为兜底。
- LLM 后端以 MLC LLM 为主，因为它天然使用 TVM、Relax、TIR 风格的编译与打包链路，更适合后续工作流编译优化研究；llama.cpp 作为 GGUF 模型生态和快速落地后端。
- 工作流先定义独立 IR，再做 prompt、KV、action 层面的编译优化，不要一开始就修改底层推理引擎。

## 2. Android 能力链路

### 2.1 读取屏幕

首选通道是 AccessibilityService：

- `getRootInActiveWindow()` 可读取当前活动窗口节点树，前提是服务声明可读取窗口内容。
- `AccessibilityNodeInfo` 能提供文本、content description、bounds、可点击/可滚动状态、action 列表等结构化信息。
- `takeScreenshot()` 从 API 30 开始可由 accessibility service 获取指定 display 的截图；API 34 增加窗口级截图能力。
- `FLAG_RETRIEVE_INTERACTIVE_WINDOWS` 可用于多窗口场景，但会扩大数据面，需要权限说明和用户可见开关。

像素通道：

- MediaProjection 适合连续屏幕捕获、OCR、视觉定位，以及 accessibility tree 语义不足的场景。
- MediaProjection 必须通过 `createScreenCaptureIntent()` 获得用户授权 token。Android foreground service 对 `mediaProjection` 有显式 service type 和前置授权要求。
- MVP 不建议默认常驻 MediaProjection；应按工作流开启，任务结束立即释放。

辅助通道：

- ML Kit Text Recognition 可用于截图 OCR。
- NotificationListenerService 可读通知并执行通知 action，例如回复消息或打开目标 App，但它同样属于敏感能力。
- UI Automator 更适合测试和 benchmark，不适合作为普通用户安装 App 的核心执行通道。

### 2.2 执行动作

动作层应该按稳定性分级：

1. App/API action：Intent、Shortcut、通知 action、系统设置 deep link、公开 content provider/API。
2. Accessibility node action：`ACTION_CLICK`、`ACTION_SET_TEXT`、`ACTION_SCROLL_*`、focus、global back/home/recent。
3. Gesture fallback：`dispatchGesture()` 发送点击、滑动、长按。Android 文档明确该 API 会像用户触屏一样派发手势，并要求服务声明手势能力。
4. 人工确认：支付、转账、删除、发送公开内容、修改隐私设置等高风险动作必须进入确认节点。

不要把 ADB、root、shell 注入作为用户运行时前提。它们可以用于测试、实验室评估或开发者模式，但不应成为普通 Android App 的基础能力。

## 3. 权限、合规与分发风险

最大风险来自 Accessibility API policy。Google Play 的官方说明中，使用 Accessibility API 进行自动化的 App 必须保证代用户执行的动作具有“狭窄且清晰理解的目的”；使用该 API 让 App 自主发起、规划、执行动作或决策被明确禁止，除非它是符合条件的 accessibility tool。

因此建议：

- 第一阶段定位为 research/prototype/internal build/sideload，不承诺 Google Play 可上架。
- 产品形态上把能力分成“用户定义的确定性工作流”和“Agent 建议 + 用户确认”两类。
- 所有敏感权限必须有单独 disclosure、用途说明、数据留存说明和 revoke 开关。
- 默认本地推理、本地日志、短生命周期截图；禁止静默上传屏幕、通知、联系人、短信等个人数据。
- 如果未来走 Play Store，需要重新定义核心用户群和功能，或把自主 Agent 限制为 deterministic workflow executor。

## 4. LLM 后端选型

### 4.1 候选对比

| 后端 | 优点 | 风险 | 结论 |
| --- | --- | --- | --- |
| MLC LLM | Android SDK、模型编译/打包链路、TVM runtime、可配置 `context_window_size` 和 `prefill_chunk_size`，适合研究编译优化 | 构建链复杂，需要物理机 GPU；部分设备/模型布局有已知启动卡顿问题 | 默认主后端 |
| llama.cpp | GGUF 生态成熟，Android 示例和 NDK 交叉编译路径明确，CPU 兼容性好 | 编译优化更偏手写 kernel/backend，工作流编译与模型编译耦合度较低 | 作为 fallback 和快速原型 |
| MediaPipe/LiteRT LLM | Android 集成体验好，Google AI Edge 生态 | 对自定义 compiler pass 和 KV 生命周期研究的开放度较弱 | 暂不作为主线 |
| Cloud LLM | 质量高、VLM 能力强 | 隐私、成本、延迟、离线能力弱 | 仅作为可选远端 provider |

### 4.2 推荐后端抽象

定义一个窄接口，避免工作流层绑定具体推理库：

```kotlin
interface LlmEngine {
    suspend fun load(model: ModelHandle, profile: RuntimeProfile)
    suspend fun generate(request: GenerateRequest): TokenStream
    suspend fun embed(request: EmbedRequest): FloatArray
    fun capabilities(): EngineCapabilities
    suspend fun reset(sessionId: SessionId)
}
```

关键能力描述：

- 最大上下文窗口；
- 是否支持 function/tool calling 输出约束；
- 是否支持 prefix cache/session cache；
- 是否支持 batch；
- 是否支持 structured decoding；
- 支持的量化格式；
- 设备内存估算；
- cold start、warm prefill、decode tokens-per-second 指标。

### 4.3 模型策略

MVP 不要追求通用大 VLM。更稳的链路是：

- Accessibility tree + OCR 生成结构化 observation。
- 小模型负责规划、参数抽取、工具选择和结果校验。
- 视觉模型只处理 accessibility tree 不可靠的局部区域。
- 工作流可为常见任务预编译 prompt prefix 和 action schema。

模型类型建议：

- 1B-4B instruct 模型：本地 planner/router。
- 0.5B-2B 小模型：分类、slot filling、工作流分支判定。
- 可选远端或边缘 VLM：困难屏幕理解、图像 UI 元素定位。

## 5. 工作流 IR 与优化空间

### 5.1 IR 节点

工作流应类比 TVM 的计算图，但节点不是 tensor op，而是 Agent/runtime op：

| 节点 | 输入 | 输出 | 优化点 |
| --- | --- | --- | --- |
| `ObserveScreen` | window id、capture policy | screen state | 增量 diff、截图降采样、OCR cache |
| `NormalizeState` | tree、screenshot、OCR | typed observation | 节点剪枝、schema lowering |
| `PromptBuild` | goal、state、memory、tool schema | token segments | 常量折叠、prefix reuse |
| `LlmCall` | prompt segments、decoding policy | model output | KV lifetime、模型选择、batch |
| `ParseToolCall` | model output | typed action | 结构化校验 |
| `Guard` | action、policy、risk | allow/deny/confirm | 静态权限裁剪 |
| `ExecuteAction` | action | action result | API 优先、手势兜底 |
| `WaitForState` | predicate | new state | timeout scheduling |
| `Verify` | expected state | pass/fail | 失败恢复 |
| `TraceWrite` | events | persisted trace | 隐私裁剪、采样 |

### 5.2 编译优化 Pass

第一批 pass：

- Capability lowering：把高层动作降级到 Intent、Notification、Accessibility、Gesture 的最佳可用实现。
- Prompt constant folding：系统提示词、工具 schema、工作流固定说明作为常量段。
- Observation pruning：只保留目标 App、可见节点、可操作节点、相关文本和局部 OCR。
- Risk annotation：对发送、购买、删除、授权、支付等动作插入 `Guard`。
- Dead action elimination：删除被静态条件判定不可达的动作分支。
- Wait fusion：合并连续等待和观察节点，降低截图/树读取频率。
- Retry lowering：把高级 retry 策略降级为 bounded retry + verifier。
- Cost planning：根据模型、上下文长度、屏幕采样频率和电量状态选择执行计划。

### 5.3 KV Cache 生命周期分析

可先在工作流层做 backend-agnostic 的 KV 生命周期规划，而不是先改 MLC/llama.cpp 内核：

- 将 prompt 拆成 `system_prefix`、`workflow_prefix`、`tool_schema`、`goal`、`observation`、`scratchpad`。
- `system_prefix` 与 `workflow_prefix` 在同一模型和 decoding policy 下可跨工作流 session 复用。
- `tool_schema` 随工具集变化失效。
- `observation` 在屏幕 diff 后失效；局部无变化时只重建变化段。
- `scratchpad` 在分支回滚、用户打断、verification fail 后失效。
- 对每个 `LlmCall` 记录 `live_in`、`live_out`、`kill`，为将来接入 paged KV 或 prefix cache 提供静态提示。

PagedAttention 的启发是把 KV cache 看成分页内存，从而降低碎片并支持跨请求共享。移动端 MVP 不一定能直接复刻 vLLM 的 serving 模型，但可以借鉴两点：

- prefix block 元数据与引用计数；
- 在工作流分支间共享稳定 prefix，屏幕 observation 作为短生命周期 block。

MLC 已经提供上下文窗口和 prefill chunk 等内存相关配置；llama.cpp 在 Android 文档中也强调 context size 会影响内存峰值。DroidLoom 的 optimizer 应先根据设备 profile 给这些参数生成建议，再逐步深入 backend-specific cache control。

## 6. Android App 架构选型

推荐模块：

```text
app/                    Compose UI、onboarding、权限控制
service-accessibility/  AccessibilityService、节点树、动作、截图
service-capture/        MediaProjection session、OCR frame pipeline
runtime-agent/          planner loop、tool registry、guardrails
runtime-workflow/       workflow parser、IR、compiler passes、scheduler
runtime-llm/            MLC 和 llama.cpp adapters
storage/                Room/DataStore、加密 trace metadata
benchmark/              UI Automator、AndroidWorld 风格测试套件
```

Android Jetpack：

- Compose：配置、工作流编辑、trace viewer。
- Room：工作流、trace、model profile、permission grants。
- DataStore：轻量设置、feature flags、privacy toggles。
- WorkManager：后台下载模型、离线编译包准备、非实时维护任务。不要用它执行长时间屏幕控制任务。
- Foreground service：仅在用户明确启动的 Agent session 中运行，并显示常驻通知。

## 7. 测试与评估

测试分层：

- Unit：工作流 IR parser、compiler pass、risk policy、prompt segment invalidation。
- JVM/property tests：随机工作流图做 dominance/lifetime/kill set 验证。
- Instrumented Android tests：Accessibility action executor、MediaProjection lifecycle、permission revoke。
- UI Automator：真实系统/App 跨进程 UI 测试。
- Benchmark：借鉴 AndroidWorld 的任务初始化、成功检查和 teardown 方式，建立可复现任务集。

指标：

- 任务成功率；
- action 数量；
- 用户确认次数；
- 非预期动作率；
- screenshot/OCR 频率；
- 单任务 token 数；
- prefill 延迟、decode tok/s、cold start；
- peak RSS/VRAM 估算；
- 电量消耗；
- 权限拒绝恢复率。

## 8. 风险清单

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| Play policy 不允许自主 accessibility automation | 无法直接上架 | research/internal/sideload 起步；收窄为确定性工作流；高风险动作用户确认 |
| Accessibility tree 不完整或 App 自定义 canvas | Agent 误判 | OCR/MediaProjection 补充；动作前 verifier；局部视觉模型 |
| 本地 LLM 延迟和内存不足 | 体验差 | 小模型、多 profile、context pruning、prefix cache、工作流 specialization |
| 手势点击不稳定 | 误操作 | API/action 优先；坐标动作必须由 verifier 包裹 |
| 敏感数据泄露 | 安全事故 | 默认本地、最小日志、截图短生命周期、加密存储、导出前脱敏 |
| 工作流过度动态导致无法优化 | optimizer 价值低 | 限制 IR side effects；schema 化工具；静态 pass + runtime guard 混合 |

## 9. 主要参考资料

Android 平台：

- Android AccessibilityService API: https://developer.android.com/reference/android/accessibilityservice/AccessibilityService
- Android AccessibilityNodeInfo API: https://developer.android.com/reference/android/view/accessibility/AccessibilityNodeInfo
- Android MediaProjection guide: https://developer.android.com/media/grow/media-projection
- Android foreground service types, mediaProjection: https://developer.android.com/develop/background-work/services/fgs/service-types#media-projection
- Google Play AccessibilityService API policy: https://support.google.com/googleplay/android-developer/answer/10964491
- Android UI Automator docs: https://developer.android.com/training/testing/other-components/ui-automator
- Android NotificationListenerService API: https://developer.android.com/reference/android/service/notification/NotificationListenerService
- Android common intents: https://developer.android.com/guide/components/intents-common
- ML Kit text recognition on Android: https://developers.google.com/ml-kit/vision/text-recognition/v2/android
- Jetpack Compose docs: https://developer.android.com/develop/ui/compose/documentation
- Android app architecture guide: https://developer.android.com/topic/architecture
- Room docs: https://developer.android.com/training/data-storage/room
- WorkManager docs: https://developer.android.com/topic/libraries/architecture/workmanager

LLM 运行时与编译器：

- MLC LLM Android SDK: https://llm.mlc.ai/docs/deploy/android.html
- MLC LLM compile model libraries: https://llm.mlc.ai/docs/compilation/compile_models.html
- llama.cpp Android docs: https://github.com/ggml-org/llama.cpp/blob/master/docs/android.md
- llama.cpp repository: https://github.com/ggml-org/llama.cpp
- Apache TVM: https://tvm.apache.org/
- TVM architecture docs: https://tvm.apache.org/docs/arch/index.html
- TVM paper: https://arxiv.org/abs/1802.04799
- PagedAttention / vLLM paper: https://arxiv.org/abs/2309.06180
- vLLM paged attention docs: https://docs.vllm.ai/en/latest/design/paged_attention/
- FlashAttention paper: https://arxiv.org/abs/2205.14135
- FlashAttention-2 paper: https://arxiv.org/abs/2307.08691

移动端和 GUI Agent 研究：

- AndroidWorld: https://arxiv.org/abs/2405.14573
- AppAgent: https://arxiv.org/abs/2312.13771
- Mobile-Agent: https://arxiv.org/abs/2401.16158

# 路线图

## M0 - 仓库与设计文档

状态：初始仓库已完成。

- 项目命名、许可证、README、技术文档和 ADR。
- Android 能力链路调研。
- 本地 LLM 后端选型调研。
- 工作流编译器架构。
- 权限和分发边界。
- 参考 AutoGLM 后定义产品能力等级、禁止清单和人工接管机制。

## M1 - Android App 外壳

目标：构建最小 Android App，包含权限引导和空运行的 session controller。

- Kotlin + Gradle Android 项目。
- Compose UI：权限、session 和日志。
- AccessibilityService 注册与 disclosure 页面。
- 前台 session notification。
- 本地 Room/DataStore 初始化。
- 单元测试骨架。

验收标准：

- App 能安装到真实 Android 设备。
- 用户可以启用和关闭 accessibility service。
- App 可以记录本地、非敏感 session trace。

## M2 - 屏幕观察与动作执行器

目标：在不接入 LLM 的情况下跑通首个 observe-act-verify 闭环。

- Accessibility tree snapshot。
- Node normalization 和 pruning。
- Accessibility action executor。
- Gesture fallback executor。
- `TakeOver`、`Confirm`、`StopSession` 基础控制动作。
- 基础 verifier predicates。
- UI Automator test harness。
- 共享真机调试环境和手动 smoke 脚本。

验收标准：

- 确定性工作流可以打开系统设置、找到可见 UI 元素、点击并校验状态。
- 高风险动作默认被阻断，除非显式加入 allowlist。
- 没有 Android 手机的开发者可以通过共享设备实验室完成基础真机调试。

## M3 - LLM 运行时适配

目标：通过稳定的 `LlmEngine` 接口接入本地模型推理。

- MLC LLM Android adapter proof of concept。
- llama.cpp adapter proof of concept。
- Model profile registry。
- Prompt segment builder。
- Structured action output parser。

验收标准：

- 同一个 planner 请求可以运行在 MLC 或 llama.cpp adapter 上。
- 基础 model profile 能记录 cold start、prefill latency、decode tok/s 和 memory estimate。

## M4 - 工作流 IR 与编译优化 Pass

目标：让工作流成为可分析、可优化、可审计的对象。

- 工作流 schema 和 parser。
- IR graph model。
- Pass manager。
- Capability lowering。
- Prompt constant folding。
- Observation pruning。
- Risk annotation。
- Retry/verifier lowering。

验收标准：

- 示例工作流可以编译成可执行 plan。
- 编译器可以在执行前输出权限和动作摘要。

## M5 - KV 生命周期与上下文优化器

目标：先在工作流层优化 LLM 调用，再考虑推理内核内部改造。

- Prompt segment liveness analysis。
- Prefix invalidation keys。
- Session cache metadata。
- Context-window planner。
- 面向 MLC 的 context 和 prefill chunk 配置建议。
- 面向 llama.cpp 的 context size 配置建议。

验收标准：

- 重复运行工作流时能减少 prefill tokens 或 prompt rebuild 开销。
- 优化器可以解释哪些 prompt segments 被复用，哪些被失效。

## M6 - 像素与 OCR 兜底

目标：处理 accessibility tree 不完整的屏幕。

- MediaProjection lifecycle。
- Foreground service integration。
- OCR with ML Kit or pluggable OCR。
- Region-of-interest capture。
- Observation fusion between tree and OCR。

验收标准：

- 目标文本不在 accessibility nodes 中但可见于屏幕时，工作流可以通过 OCR 恢复。
- Capture session 只在用户明确操作后启动，并在工作流完成后停止。

## M7 - Benchmark 套件

目标：度量可靠性和成本。

- AndroidWorld-inspired tasks。
- UI Automator replay harness。
- Success/failure taxonomy。
- Latency、memory、battery、token metrics。
- Regression dashboard artifacts。
- Firebase Test Lab 或共享真机 runner 接入 release candidate 测试。

验收标准：

- 每个候选版本都能运行固定任务集。
- 任务失败时包含 trace、screenshot policy 结果和 verifier output。

## 暂不进入近期范围

- 依赖 root 的功能。
- 依赖 ADB 的用户运行时。
- 第三方工作流 marketplace。
- 静默后台操作。
- 自主执行高风险动作。
- 在工作流层 cache 规划证明价值前，定制 fork MLC 或 llama.cpp 的 KV 内部实现。

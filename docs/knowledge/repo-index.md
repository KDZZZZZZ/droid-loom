# Repo 索引

日期：2026-05-27

这个索引用来跟踪 DroidLoom 值得参考的开源项目。它不是依赖清单，也不是“全部照抄”的建议，而是把外部 repo 拆成可迁移设计点：

```text
Mobile agent repo       -> observation/action/CLI/portal
Benchmark repo          -> task/reset/success check/trace
Automation repo         -> device connection/UI tree/action API/report
LLM backend repo        -> Android 本地推理/KV snapshot/profile
KV/cache repo           -> prefix cache/KV block/cache admission
Workflow runtime repo   -> graph state/checkpoint/human-in-loop
GUI grounding repo      -> screenshot fallback/视觉定位
```

DroidLoom 的差异化定位应该固定为：

- workflow-first，不是 chat-first；
- local-first，不是 cloud/operator-first；
- KV/cache-first，不只是 GUI agent；
- backend-pluggable，MLC、llama.cpp、ExecuTorch 可插拔；
- traceable，每一步 observation、action、context、cache 都可记录；
- safety-guarded，按能力等级开放自动化。

## 快速优先级

最该先看这 10 个方向：

1. [Droidrun / Mobilerun](https://github.com/droidrun/droidrun)：手机 agent 的 observation/action/CLI/API。
2. [Droidrun Portal](https://github.com/droidrun/droidrun-portal)：Android Accessibility portal 和元素可视化。
3. [AndroidWorld](https://github.com/google-research/android_world)：benchmark、任务定义和 success checker。
4. [DroidBot](https://github.com/honeynet/droidbot)：UI transition graph 和自动探索。
5. [llama.cpp](https://github.com/ggml-org/llama.cpp)：本地 LLM fallback、server slot、prompt cache save/restore。
6. [MLC LLM](https://github.com/mlc-ai/mlc-llm)：Android 正式本地推理后端。
7. [vLLM](https://github.com/vllm-project/vllm)：PagedAttention 和 prefix caching 思想。
8. [SGLang](https://github.com/sgl-project/sglang)：RadixAttention、structured generation 和多次 LLM call runtime。
9. [LangGraph](https://github.com/langchain-ai/langgraph)：durable workflow、state graph、human-in-the-loop。
10. [UI-TARS](https://github.com/bytedance/UI-TARS)、[OS-Atlas](https://github.com/OS-Copilot/OS-Atlas)、[SeeClick](https://github.com/njucckevin/SeeClick)：可插拔视觉 GUI grounding 后端候选。

## P0：直接相关，优先看

| ID | Repo | 类型 | DroidLoom 应该看什么 |
| --- | --- | --- | --- |
| R0-01 | [droidrun/droidrun](https://github.com/droidrun/droidrun) | `mobile-agent` | 直接对标的 mobile automation framework。重点看它怎么抽象 UI state、screenshot、tap、swipe、type、多步 task、CLI 和 SDK。DroidLoom 不应照搬 cloud/agent 形态，但应参考 observation/action API 的开发者体验。 |
| R0-02 | [droidrun/droidrun-portal](https://github.com/droidrun/droidrun-portal) | `mobile-agent`, `accessibility` | Android Accessibility Service + UI 元素可视化。重点看 clickable、editable、scrollable、focusable 元素高亮和当前 phone state JSON。DroidLoom 可参考做 Accessibility portal、overlay、元素调试器。 |
| R0-03 | [google-research/android_world](https://github.com/google-research/android_world) | `benchmark` | Android Agent benchmark 核心参考。它运行在真实 Android emulator，并提供 116 个手工任务、20 个 App、动态参数化任务。DroidLoom 的 benchmark harness、success checker、reset/teardown 应重点对齐。 |
| R0-04 | [honeynet/droidbot](https://github.com/honeynet/droidbot) | `automation`, `benchmark` | Android UI-guided test input generator。重点看 UI transition graph、自动探索、状态记录和输入生成。DroidLoom 可借鉴 screen state graph、workflow exploration、UI transition logging。 |
| R0-05 | [ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp) | `llm-backend`, `kv-cache` | 本地 LLM fallback 首选。重点看 Android/NDK 路径、server slots、prompt cache save/restore。DroidLoom 可先用它验证 KV snapshot 和 prompt prefix 复用实验。 |
| R0-06 | [mlc-ai/mlc-llm](https://github.com/mlc-ai/mlc-llm) | `llm-backend`, `on-device` | 正式主后端候选。重点看 Android SDK、模型编译/打包、MLCEngine、跨平台 runtime。它和 DroidLoom 的 workflow 编译优化路线最一致。 |
| R0-07 | [vllm-project/vllm](https://github.com/vllm-project/vllm) | `kv-cache`, `runtime` | PagedAttention、prefix caching、continuous batching 的系统参考。DroidLoom 不直接移植服务端 vLLM，但要借鉴 KV block、prefix hash、cache admission 和 eviction。 |
| R0-08 | [sgl-project/sglang](https://github.com/sgl-project/sglang) | `workflow`, `kv-cache` | Structured generation runtime。重点看 RadixAttention、prefill/decode disaggregation、structured outputs、chunked prefill。DroidLoom 的 Workflow IR 和多次 LLM call cache 设计应重点参考。 |
| R0-09 | [langchain-ai/langgraph](https://github.com/langchain-ai/langgraph) | `workflow` | 长运行、有状态 Agent 工作流参考。重点看 graph state、checkpoint、interrupt/resume、human-in-the-loop。DroidLoom 不应套壳，但可以借鉴 durable execution 语义。 |
| R0-10 | [youichi-uda/droidpilot](https://github.com/youichi-uda/droidpilot) | `mobile-agent`, `mcp`, `accessibility` | 面向 AI agents 的 Android 自动化 MCP 思路。重点看 Accessibility Service 能力如何 server 化、tool/action 如何暴露给外部 agent。适合作为 DroidLoom 工具注册表和 MCP bridge 参考。 |
| R0-11 | [KarryViber/orb-eye](https://github.com/KarryViber/orb-eye) | `mobile-agent`, `accessibility` | Android Accessibility Service 暴露 HTTP API 的简化实现。重点看 UI tree、notifications、tap、swipe、setText 等能力如何变成服务端 API。适合参考最小可调试 agent bridge。 |
| R0-12 | [agents-io/PokeClaw](https://github.com/agents-io/PokeClaw) | `mobile-agent`, `local-first` | Android on-device phone agent。重点看 local-first 产品表达、本地模型路径、Accessibility 控制和 guard 逻辑。适合对照 DroidLoom 的产品边界。 |

## P1：强相关，但不要变成主线

| ID | Repo | 类型 | DroidLoom 应该看什么 |
| --- | --- | --- | --- |
| R1-01 | [OpenBMB/AppCopilot](https://github.com/OpenBMB/AppCopilot) | `mobile-agent`, `multi-agent` | 多模态、多 agent、跨 App on-device assistant。重点看完整系统如何组织数据、模型、部署和闭环，但不建议直接照抄架构。 |
| R1-02 | [TencentQQGYLab/AppAgent](https://github.com/TencentQQGYLab/AppAgent) | `mobile-agent` | AppAgent 论文对应 repo。重点看 simplified action space、探索流程、demo 组织。DroidLoom 可借鉴动作空间，但应优先使用 Android API 和 tool registry。 |
| R1-03 | [X-PLUG/MobileAgent](https://github.com/x-plug/mobileagent) | `mobile-agent`, `grounding` | Mobile-Agent/GUI-Owl 系列。重点看视觉感知、跨平台 GUI 操作、动作输出格式。DroidLoom 可作为视觉后端参考，不作为 workflow runtime 主线。 |
| R1-04 | [Tongyi-MAI/MobileWorld](https://github.com/Tongyi-MAI/MobileWorld) | `benchmark`, `mcp` | 面向 autonomous mobile agents 的 benchmark。重点看 Agent-User interactive 和 MCP-augmented environments。适合 DroidLoom 后续做人机协作任务。 |
| R1-05 | [security-pride/LLMDroid](https://github.com/security-pride/LLMDroid) | `automation`, `benchmark` | LLM + Android GUI testing。重点看如何把 LLM 接入 DroidBot/Humanoid/Fastbot2 这类传统测试工具。 |
| R1-06 | [openatx/uiautomator2](https://github.com/openatx/uiautomator2) | `automation` | 设备端 HTTP service + Python wrapper。重点看 device-side server、JSON-RPC、screenshot、text input、Python API。DroidLoom 可参考测试工具和设备连接，不作为用户 runtime。 |
| R1-07 | [appium/appium](https://github.com/appium/appium) | `automation` | 跨平台 automation framework。重点看 driver 分层、capability negotiation、reporting。适合 CI/测试工具链参考。 |
| R1-08 | [AirtestProject/Airtest](https://github.com/AirtestProject/Airtest) | `automation`, `testing` | 跨平台 UI 自动化和报告。重点看 case 组织、截图、报告产物和多设备 runner。 |
| R1-09 | [AirtestProject/Poco](https://github.com/AirtestProject/Poco) | `automation`, `ui-inspection` | 跨引擎 UI inspection/automation。适合参考 UI hierarchy 和 game/canvas 场景，但不作为 DroidLoom 主依赖。 |
| R1-10 | [Genymobile/scrcpy](https://github.com/Genymobile/scrcpy) | `device-lab` | 远程调试和共享真机基础工具。重点看 USB/TCP、低延迟显示控制、无需 root。用于设备实验室，不进入 App runtime。 |
| R1-11 | [pytorch/executorch](https://github.com/pytorch/executorch) | `llm-backend`, `on-device` | PyTorch on-device inference。适合后续小模型、OCR、embedding、screen parser，不一定做主 LLM。 |
| R1-12 | [lmcache/lmcache](https://github.com/LMCache/LMCache) | `kv-cache` | 外部 KV cache 层。重点看 KV 作为一等资源、GPU/CPU/Disk/S3 分层、lookup/pin/evict API。DroidLoom 可借鉴概念，移动端需要降级。 |
| R1-13 | [microsoft/agent-framework](https://github.com/microsoft/agent-framework) | `workflow`, `multi-agent` | 生产级 multi-agent workflow 框架。重点看 orchestration、handoff、workflow hosting。DroidLoom 只借鉴语义，不引入手机端。 |

## P2：跟踪即可

| ID | Repo | 类型 | DroidLoom 应该看什么 |
| --- | --- | --- | --- |
| R2-01 | [bytedance/UI-TARS](https://github.com/bytedance/UI-TARS) | `grounding`, `model` | 多模态 GUI agent 模型。后续如果接视觉 action model，可参考模型接口、动作输出和 benchmark。 |
| R2-02 | [bytedance/UI-TARS-desktop](https://github.com/bytedance/UI-TARS-desktop) | `agent-stack`, `product` | Agent TARS 桌面/浏览器/终端产品栈。可参考 CLI、trace、产品包装，不进入 Android runtime 主线。 |
| R2-03 | [OS-Copilot/OS-Atlas](https://github.com/OS-Copilot/OS-Atlas) | `grounding`, `model` | GUI grounding/action model。可作为视觉后端候选。 |
| R2-04 | [njucckevin/SeeClick](https://github.com/njucckevin/SeeClick) | `grounding` | ScreenSpot 和视觉 GUI grounding。用于 accessibility tree 不完整时的 screenshot fallback。 |
| R2-05 | [OSU-NLP-Group/UGround](https://github.com/OSU-NLP-Group/UGround) | `grounding` | Universal GUI visual grounding。适合跟踪通用 GUI grounding 封装方式。 |
| R2-06 | [ZJU-REAL/Awesome-GUI-Agents](https://github.com/ZJU-REAL/Awesome-GUI-Agents) | `tracking` | GUI agents 资源集合。用于持续跟踪新论文和新 repo。 |
| R2-07 | [microsoft/autogen](https://github.com/microsoft/autogen) | `multi-agent` | 多 agent 框架。当前更适合作为历史和概念参考；新的生产化方向应优先看 Microsoft Agent Framework。 |
| R2-08 | [OpenBMB/XAgent](https://github.com/OpenBMB/XAgent) | `workflow`, `multi-agent` | Planner/Actor/Dispatcher 架构参考。可借鉴角色拆分，不进入手机端主线。 |
| R2-09 | [openclaw/openclaw](https://github.com/openclaw/openclaw) | `assistant`, `skill-ecosystem` | 本地控制平面和 skill ecosystem 参考。DroidLoom 可借鉴插件生态思路，但能力边界要更收敛。 |
| R2-10 | OpenAutojs / Auto.js 系列 | `automation`, `scripting` | Android JS runtime + Accessibility 自动化环境。只作为反面/边界参考：DroidLoom 不应把任意脚本作为安全可分析 workflow runtime。 |

## 按 DroidLoom 模块查 repo

### `service-accessibility`

优先看：

- Droidrun Portal
- DroidPilot
- Orb Eye
- PokeClaw
- uiautomator2

要抽取：

- Accessibility tree snapshot。
- clickable/editable/scrollable/focusable 过滤。
- overlay 和元素高亮。
- UI state JSON schema。
- action API 的最小集合。

### `service-capture`

优先看：

- MobileAgent
- SeeClick
- UI-TARS
- UGround
- Airtest

要抽取：

- screenshot pipeline。
- OCR/视觉 grounding fallback。
- 坐标动作 verifier。
- 图片和 UI tree 融合策略。

### `runtime-agent`

优先看：

- Droidrun
- AppAgent
- AppCopilot
- PokeClaw
- LangGraph
- Microsoft Agent Framework

要抽取：

- action space。
- tool call schema。
- planner/verifier/reflection 拆分。
- human-in-the-loop。
- interrupt/resume。
- trace/event log。

### `runtime-workflow`

优先看：

- SGLang
- LangGraph
- Microsoft Agent Framework
- XAgent
- Droidrun

要抽取：

- graph state。
- checkpoint。
- durable execution。
- structured generation。
- workflow node 和 tool node。
- cache-aware scheduling。

### `runtime-llm`

优先看：

- MLC LLM
- llama.cpp
- ExecuTorch
- vLLM
- SGLang
- LMCache

要抽取：

- Android native runtime。
- model package/profile。
- prompt cache save/restore。
- prefix caching。
- KV block。
- cache admission/eviction。
- fake/small model 测试分层。

### `benchmark`

优先看：

- AndroidWorld
- DroidBot
- MobileWorld
- LLMDroid
- Appium
- Airtest

要抽取：

- task definition。
- dynamic parameter。
- reset/teardown。
- success checker。
- UI transition graph。
- multi-device runner。
- report artifact。

### `device-lab`

优先看：

- scrcpy
- AndroidWorld
- Appium
- Airtest
- uiautomator2

要抽取：

- ADB/device connection。
- emulator 和真机切换。
- screen mirror。
- failure artifact。
- 设备锁。
- 多设备调度。

## DroidLoom 不应该照抄的点

- 不要把 DroidLoom 做成单纯的 cloud phone operator。
- 不要让任意 JS/Python script 成为用户手机端 workflow runtime。
- 不要把 ADB/root/shell 注入作为普通用户运行前提。
- 不要把视觉 grounding 当成主观察通道；Accessibility tree 和 Android API 应优先。
- 不要把 LangGraph 或通用 agent framework 塞进 Android 端核心 runtime。
- 不要一开始追求 full general mobile agent；先做支持矩阵和受控 workflow。

## 建议的工程落地顺序

1. 读 Droidrun、Droidrun Portal、Orb Eye：定义 DroidLoom 的 observation/action/debug portal。
2. 读 AndroidWorld、DroidBot：定义 benchmark harness 和 UI state graph。
3. 读 llama.cpp、MLC LLM：定义 `LlmEngine`、Android native bridge、模型 profile。
4. 读 vLLM、SGLang、LMCache：定义 workflow-level KV/cache metadata。
5. 读 LangGraph、Microsoft Agent Framework：定义 checkpoint、interrupt/resume、human-in-the-loop。
6. 读 UI-TARS、OS-Atlas、SeeClick：评估视觉 grounding 插件边界。

## 维护规则

- 新 repo 先放 P2，除非它直接影响 M1-M3。
- 如果 repo 影响 Android runtime 模块边界，补 `docs/architecture.md` 或 ADR。
- 如果 repo 影响 CI/设备实验室，补 `docs/ci.md` 或 `docs/device-lab.md`。
- 如果 repo 只影响长期模型路线，保留在知识库，不推进实现。

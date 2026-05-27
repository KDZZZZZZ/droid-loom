# 论文索引

日期：2026-05-27

这个索引按“对 DroidLoom 的可借鉴程度”排序。它不是完整综述，而是工程入口：每篇论文都要回答“它能帮助 DroidLoom 的哪个模块做出更好的设计”。

## 快速阅读顺序

第一轮只读这 8 篇：

1. [AndroidWorld](https://arxiv.org/abs/2405.14573)
2. [AppAgent](https://arxiv.org/abs/2312.13771)
3. [Mobile-Agent](https://arxiv.org/abs/2401.16158)
4. [AndroidControl / On the Effects of Data Scale on UI Control Agents](https://openreview.net/forum?id=yUEBXN3cvX)
5. [SGLang](https://openreview.net/forum?id=VqkAKQibpq)
6. [PagedAttention / vLLM](https://arxiv.org/abs/2309.06180)
7. [TVM](https://arxiv.org/abs/1802.04799)
8. [Fast On-device LLM Inference with NPUs](https://arxiv.org/abs/2407.05858)

第二轮再读 HeteroLLM、PowerInfer-2、SeeClick、UI-TARS、OS-ATLAS。

## P0：优先读

| ID | 论文 | 标签 | 对 DroidLoom 的简介式索引 |
| --- | --- | --- | --- |
| P0-01 | [AndroidWorld: A Dynamic Benchmarking Environment for Autonomous Agents](https://arxiv.org/abs/2405.14573) | `mobile-agent`, `benchmark` | 最重要的 benchmark 参考。重点看任务初始化、success check、teardown、设备状态修改和可复现评测。DroidLoom 的 `benchmark/` 应直接对齐这种 harness，而不是只做人工 demo。 |
| P0-02 | [AppAgent: Multimodal Agents as Smartphone Users](https://arxiv.org/abs/2312.13771) | `mobile-agent`, `workflow` | 手机 Agent 动作空间和 App 使用知识库参考。重点看 simplified action space、探索/演示学习、跨 App 任务边界。DroidLoom 可借鉴动作抽象，但不要绕过 Android API/tool registry。 |
| P0-03 | [Mobile-Agent: Autonomous Multi-Modal Mobile Device Agent with Visual Perception](https://arxiv.org/abs/2401.16158) | `mobile-agent`, `grounding` | 视觉感知 + mobile action loop。重点看 screenshot-based observation、视觉定位、动作循环和失败恢复。用于 DroidLoom 的 MediaProjection/OCR fallback 设计。 |
| P0-04 | [On the Effects of Data Scale on UI Control Agents / AndroidControl](https://openreview.net/forum?id=yUEBXN3cvX) | `mobile-agent`, `benchmark`, `grounding` | 大规模 Android UI 控制数据和任务分布参考。重点看 action schema、训练/评测切分、数据规模对 UI control agent 的影响。用于定义 DroidLoom 支持 App/任务矩阵。 |
| P0-05 | [SeeClick: Harnessing GUI Grounding for Advanced Visual GUI Agents](https://arxiv.org/abs/2401.10935) | `grounding`, `mobile-agent` | GUI grounding 参考。重点看纯截图定位、ScreenSpot、指令到屏幕元素的定位能力。用于补齐 accessibility tree 不完整、canvas UI 或自绘控件场景。 |
| P0-06 | [UI-TARS: Pioneering Automated GUI Interaction with Native Agents](https://arxiv.org/abs/2501.12326) | `mobile-agent`, `grounding`, `benchmark` | 原生 GUI Agent 模型参考。重点看统一动作空间、反思式 trace 训练、AndroidWorld 评测和多步 GUI reasoning。用于理解模型侧能力上限，但不作为 DroidLoom MVP 前提。 |
| P0-07 | [OS-ATLAS: A Foundation Action Model for Generalist GUI Agents](https://arxiv.org/abs/2410.23218) | `mobile-agent`, `grounding` | 跨平台 GUI action model。重点看 GUI 元素数据合成、通用 action model、跨平台 OOD 泛化。用于 DroidLoom 长期的 action schema 泛化设计。 |
| P0-08 | [SGLang: Efficient Execution of Structured Language Model Programs](https://openreview.net/forum?id=VqkAKQibpq) | `workflow`, `kv-cache` | 多次 LLM 调用和结构化语言模型程序执行参考。重点看 structured generation、调度、cache reuse、RadixAttention。用于 DroidLoom 的 Workflow IR、prompt segment 和多 LLM call 调度。 |
| P0-09 | [Efficient Memory Management for Large Language Model Serving with PagedAttention](https://arxiv.org/abs/2309.06180) | `kv-cache`, `on-device` | KV Cache 生命周期分析核心参考。重点看 paging、block、引用计数、跨请求共享和碎片控制。DroidLoom 不直接复刻服务端 vLLM，但要借鉴 prefix block 和 lifetime metadata。 |
| P0-10 | [TVM: An Automated End-to-End Optimizing Compiler for Deep Learning](https://arxiv.org/abs/1802.04799) | `compiler`, `on-device` | Workflow IR/pass/cost model 的思想来源。重点看 graph-level optimization、operator lowering、cost model 和硬件后端抽象。DroidLoom 借鉴分层，不在 MVP 写 kernel/TIR。 |
| P0-11 | [Fast On-device LLM Inference with NPUs](https://arxiv.org/abs/2407.05858) | `on-device`, `kv-cache` | 手机 NPU 推理参考。重点看 prefill、变长 prompt 切固定 chunk、CPU/GPU/NPU 协同。适合 DroidLoom 后期做异构后端和 context planning。 |
| P0-12 | [HeteroLLM: Accelerating Large Language Model Inference on Mobile SoCs with Heterogeneous AI Accelerators](https://arxiv.org/abs/2501.14794) | `on-device` | 移动 SoC 异构推理参考。重点看统一内存、GPU/NPU 特性、prefill/decode 不同策略。用于本地 LLM profile 和设备能力建模。 |
| P0-13 | [PowerInfer-2: Fast Large Language Model Inference on a Smartphone](https://arxiv.org/abs/2406.06282) | `on-device`, `kv-cache` | 手机上模型资源调度参考。重点看 NPU/CPU/flash/I/O 协同和细粒度调度。用于后期本地模型资源规划，不进入 M1-M3。 |

## P1：第二批读

| ID | 论文 | 标签 | 对 DroidLoom 的简介式索引 |
| --- | --- | --- | --- |
| P1-01 | [OSWorld: Benchmarking Multimodal Agents for Open-Ended Tasks in Real Computer Environments](https://arxiv.org/abs/2404.07972) | `benchmark`, `mobile-agent` | 桌面 OS 环境 benchmark。重点看真实环境、任务 setup、execution-based evaluation。可迁移到 DroidLoom 的长链路任务评估和 trace/verifier 设计。 |
| P1-02 | [WebArena: A Realistic Web Environment for Building Autonomous Agents](https://arxiv.org/abs/2307.13854) | `benchmark`, `workflow` | 自托管、可复现 Web agent 环境。重点看环境初始化、任务隔离和真实服务编排。用于 DroidLoom 设计可复现手机任务环境。 |
| P1-03 | [Mind2Web: Towards a Generalist Agent for the Web](https://arxiv.org/abs/2306.06070) | `benchmark`, `tool-use` | 真实网站任务和 HTML 上下文过滤。可迁移到 Android accessibility tree pruning：先用轻量筛选缩小上下文，再交给 planner。 |
| P1-04 | [ToolLLM: Facilitating Large Language Models to Master 16000+ Real-world APIs](https://arxiv.org/abs/2307.16789) | `tool-use`, `workflow` | 大规模工具调用数据和 tool-use agent。DroidLoom 的 Intent、Shortcut、Notification action、Accessibility action 统一抽象，本质也是 mobile tool-use。 |
| P1-05 | [DistServe: Disaggregating Prefill and Decoding for Goodput-optimized LLM Serving](https://arxiv.org/abs/2401.09670) | `kv-cache`, `on-device` | prefill/decode 分离思想。移动端可转化为 context build/prefill 和短 action decode 的分阶段调度。 |
| P1-06 | [FlexGen: High-Throughput Generative Inference of Large Language Models with a Single GPU](https://arxiv.org/abs/2303.06865) | `on-device`, `kv-cache` | GPU/CPU/disk/压缩联合内存规划。可类比手机 DRAM/flash/KV snapshot，但系统目标不同，主要借鉴资源规划思想。 |
| P1-07 | [LLM in a flash: Efficient Large Language Model Inference with Limited Memory](https://arxiv.org/abs/2312.11514) | `on-device` | 有限内存设备上的 flash-aware 参数读取。适合后期研究大模型本地运行，不进入 MVP。 |
| P1-08 | [KIVI: A Tuning-Free Asymmetric 2bit Quantization for KV Cache](https://arxiv.org/abs/2402.02750) | `kv-cache` | KV Cache 量化参考。DroidLoom 如果深入 backend KV 压缩再读；当前只在 workflow 层做 lifetime metadata。 |
| P1-09 | [Efficient Streaming Language Models with Attention Sinks / StreamingLLM](https://arxiv.org/abs/2309.17453) | `kv-cache` | 长 agent session 的 KV 保留、滑窗和 attention sink 策略。适合 DroidLoom 处理长时间运行的 Agent session。 |
| P1-10 | [MInference 1.0: Accelerating Pre-filling for Long-Context LLMs via Dynamic Sparse Attention](https://arxiv.org/abs/2407.02490) | `kv-cache`, `on-device` | 长上下文 prefill 优化。DroidLoom 后续如果 prompt/history 很长，可借鉴 observation 和 trace 的稀疏化策略。 |
| P1-11 | [LLMLingua: Compressing Prompts for Accelerated Inference of Large Language Models](https://arxiv.org/abs/2310.05736) | `workflow`, `kv-cache` | Prompt compression 参考。可用于 observation pruning、screen summary 压缩、memory admission。 |
| P1-12 | [FlashAttention: Fast and Memory-Efficient Exact Attention with IO-Awareness](https://arxiv.org/abs/2205.14135) | `on-device`, `compiler` | 底层 attention IO-aware 思想。不是 DroidLoom 第一版重点，但后续读 MLC/TIR 和端侧 backend 时有用。 |
| P1-13 | [FlashAttention-2: Faster Attention with Better Parallelism and Work Partitioning](https://arxiv.org/abs/2307.08691) | `on-device`, `compiler` | 更偏 kernel/backend。DroidLoom 不应在 MVP 先碰这层，保留为后端优化背景材料。 |
| P1-14 | [Elastic On-Device LLM Service / ElastiLM](https://dl.acm.org/doi/10.1145/3680207.3765259) | `on-device`, `workflow` | 移动端 LLM service 和动态模型/Prompt 选择。贴近 DroidLoom 的本地 Agent runtime，但需要结合论文细节再做设计迁移。 |

## P2：产品、训练和扩展方向

| ID | 论文 | 标签 | 对 DroidLoom 的简介式索引 |
| --- | --- | --- | --- |
| P2-01 | [GUI-R1: A Generalist R1-Style Vision-Language Action Model for GUI Agents](https://arxiv.org/html/2504.10458v1) | `mobile-agent`, `grounding` | 如果未来训练自己的 GUI action model 或做 RL fine-tuning，可作为模型侧路线参考。MVP 不依赖。 |
| P2-02 | [Learning Mobile Device Operation Through Video-Guided Multi-Agent Collaboration](https://arxiv.org/html/2502.17110v1) | `mobile-agent`, `workflow` | 视频/演示学习手机操作。适合后续用 DroidLoom trace 做数据增强或 workflow discovery。 |
| P2-03 | [AndroidLab: Training and Systematic Benchmarking of Android Autonomous Agents](https://aclanthology.org/2025.acl-long.107.pdf) | `mobile-agent`, `benchmark` | Android Agent 训练和系统化 benchmark。用于补 AndroidWorld 之外的评测设计。 |
| P2-04 | Mobile-Agent-v2: Mobile Device Operation Assistant with Effective Navigation via Multi-Agent Collaboration | `mobile-agent`, `workflow` | Planner/verifier/reflector 多 Agent 架构参考。DroidLoom 可借鉴角色拆分，但需要坚持 Guard/Confirm/TakeOver 的产品边界。 |
| P2-05 | VisualWebArena: Evaluating Multimodal Agents on Realistic Visual Web Tasks | `benchmark`, `grounding` | 多模态网页 Agent 评测。适合借鉴视觉观察 + 真实任务评估方式。 |
| P2-06 | TheAgentCompany: Benchmarking LLM Agents on Consequential Real-World Tasks | `benchmark`, `safety` | 长程、多工具、结果导向任务评测。可借鉴 trace/verifier、任务后果和安全边界设计。 |

## 按 DroidLoom 模块查论文

### `benchmark/`

优先读：

- P0-01 AndroidWorld
- P1-01 OSWorld
- P1-02 WebArena
- P2-03 AndroidLab

要抽取：

- 任务初始化。
- 成功检查。
- teardown。
- 任务参数化。
- 设备状态隔离。
- 失败 trace 分类。

### `service-accessibility` 与 `service-capture`

优先读：

- P0-02 AppAgent
- P0-03 Mobile-Agent
- P0-05 SeeClick
- P0-06 UI-TARS

要抽取：

- 动作空间。
- screenshot/OCR fallback。
- GUI grounding。
- 坐标动作风险。
- 视觉观察和 accessibility tree 的融合边界。

### `runtime-agent`

优先读：

- P0-02 AppAgent
- P0-06 UI-TARS
- P1-04 ToolLLM
- P2-04 Mobile-Agent-v2

要抽取：

- Planner/verifier/reflection 是否值得拆分。
- tool schema 如何约束输出。
- action loop 如何处理失败。
- 何时触发人工接管。

### `runtime-workflow`

优先读：

- P0-08 SGLang
- P0-10 TVM
- P1-03 Mind2Web
- P1-11 LLMLingua

要抽取：

- Workflow IR。
- prompt segment。
- compiler pass。
- observation pruning。
- context compression。
- cost model。

### `runtime-llm`

优先读：

- P0-09 PagedAttention
- P0-11 llm.npu
- P0-12 HeteroLLM
- P0-13 PowerInfer-2
- P1-05 DistServe

要抽取：

- Prefill/decode 切分。
- KV cache block/lifetime。
- 移动 SoC profile。
- DRAM/flash/accelerator 资源规划。
- 小模型、fake model、真实 profile 的分层测试。

## 设计映射

| DroidLoom 设计点 | 对应论文 |
| --- | --- |
| 工作流 IR 和 pass manager | TVM、SGLang、Mind2Web |
| KV Cache 生命周期分析 | PagedAttention、SGLang、StreamingLLM、KIVI |
| prompt segment 和 prefix reuse | SGLang、PagedAttention、LLMLingua |
| Android benchmark harness | AndroidWorld、AndroidLab、OSWorld |
| 动作空间和工具抽象 | AppAgent、UI-TARS、ToolLLM、OS-ATLAS |
| OCR/截图 fallback | Mobile-Agent、SeeClick、UI-TARS |
| 真机/模拟器测试矩阵 | AndroidWorld、AndroidLab、WebArena |
| 端侧模型内存规划 | llm.npu、HeteroLLM、PowerInfer-2、FlexGen、LLM in a flash |
| 高风险动作确认与接管 | AppAgent、AutoGLM 资料、DroidLoom capability boundary |

## 维护规则

- 新增论文时先放入 P2，除非它直接影响当前架构。
- 如果论文改变 `runtime-workflow`、`runtime-agent` 或 `runtime-llm` 的设计，必须补 ADR。
- 如果论文只影响 benchmark 或开发流程，补 `docs/roadmap.md` 或 `docs/ci.md`。
- 深读后的单篇笔记放入 `docs/knowledge/papers/`。

# DroidLoom

DroidLoom 是一个 Android Agent 运行时与工作流编译器。它面向手机上的本地 Agent：在用户授权下读取屏幕、调用 Android 能力、执行可验证动作，并把用户或开发者定义的工作流编译成可优化的执行图。

这个名字的含义是：Android 任务由屏幕观察、系统 API、权限护栏、模型调用和验证逻辑共同“编织”而成；这些工作流随后会被降级、分析和优化，就像编译器处理计算图一样。

## 项目目标

构建一个 Android Agent App，支持：

- 通过 AccessibilityService 和可选的屏幕捕获能力观察当前屏幕；
- 通过无障碍动作、手势、Intent、通知操作和应用公开 API 操作手机；
- 为用户提供自定义工作流和 Agent 编排能力；
- 以 MLC LLM 作为优先本地推理后端，并保留 llama.cpp 作为 GGUF 生态和快速原型后端；
- 将工作流编译为中间表示，用于调度、上下文复用、KV Cache 生命周期分析、内存规划和安全校验。

## 当前状态

本仓库目前是文档优先的项目种子仓库，包含项目定义、技术调研、架构说明、路线图和架构决策记录。这样做是有意的：Android 权限模型、Google Play 合规边界、本地推理后端和工作流优化形态都需要先被明确，再进入 App 脚手架和运行时代码实现。

## 关键文档

- [技术调研资料](./docs/research.md)：端到端技术选型、风险和参考资料。
- [产品能力边界](./docs/capability-boundary.md)：参考 AutoGLM 后定义的能力等级、禁止清单和确认/接管机制。
- [系统架构](./docs/architecture.md)：运行时组件、数据流和模块边界。
- [路线图](./docs/roadmap.md)：分阶段构建计划和验收标准。
- [分支管理规范](./docs/branching.md)：分支命名、PR、合并、保护规则和 release 分支策略。
- [CI 设计与调研](./docs/ci.md)：当前 docs CI 和后续 Android/LLM CI 规划。
- [共享 Android 设备实验室与真机 CI](./docs/device-lab.md)：无真机开发者、共享手机调试、物理设备 CI 和安全边界。
- [知识库](./docs/knowledge/README.md)：论文索引、阅读顺序和后续单篇论文笔记路径。
- [架构决策 0001](./docs/adr/0001-llm-backend-strategy.md)：以 MLC LLM 为主、llama.cpp 为辅的本地推理后端策略。
- [架构决策 0002](./docs/adr/0002-workflow-ir-and-optimizer.md)：工作流中间表示与优化器策略。
- [架构决策 0003](./docs/adr/0003-permission-and-distribution-boundary.md)：Android 权限与分发边界。

## 重要边界

Android 无障碍自动化是敏感能力。Google Play 政策允许合规使用 AccessibilityService，但如果应用自主发起、规划和执行用户动作，除非它是符合条件的无障碍工具，否则会触碰政策边界。

因此，DroidLoom 应先作为研究型、内部测试或侧载项目推进。等产品范围、权限披露、用户确认机制和分发规则都明确后，再评估是否适合公开上架。

产品能力应按 L0-L5 分级开放：从只读观察、建议模式、确定性工作流，到受限 Agent、高风险确认和人工接管。详见 [产品能力边界](./docs/capability-boundary.md)。

## 许可证

本项目使用 Apache-2.0 许可证，见 [LICENSE](./LICENSE)。

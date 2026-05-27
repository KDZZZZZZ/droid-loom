# 架构决策 0001：LLM 后端策略

日期：2026-05-27

## 状态

已接受。

## 背景

DroidLoom 需要在 Android 上运行本地 LLM 推理，并且最终要围绕工作流做编译式优化。后端必须支持小型端侧模型、可预测的内存 profile，并暴露足够的构建期和运行期控制能力，用于研究 prompt 和 KV Cache 行为。

候选路径：

- MLC LLM。
- llama.cpp。
- MediaPipe/LiteRT LLM。
- 远端 LLM provider。

## 决策

以 MLC LLM 作为主后端，以 llama.cpp 作为次级 fallback。两者都通过统一的 `LlmEngine` 接口接入。

## 理由

- MLC LLM 提供 Android SDK 和模型编译/打包工作流，可以生成 Android 运行时产物。
- MLC 与 Apache TVM 关系紧密，契合本项目把工作流视为可优化图的目标。
- MLC 暴露上下文窗口和 prefill 相关配置，这些配置与内存规划直接相关。
- llama.cpp 拥有成熟的 GGUF 生态、Android 文档和更直接的 CPU-first fallback 路径。
- 统一后端接口可以避免工作流和编译器逻辑过早绑定某个推理运行时。

## 影响

正向影响：

- 编译优化研究可以优先面向 MLC，但不会阻断实用实验。
- llama.cpp 为不支持 MLC 的模型或设备提供稳妥退路。
- 模型 profile 可以用同一工作负载对比两个引擎。

负向影响：

- 两个 adapter 会增加集成和测试成本。
- MLC Android 环境比纯 Java/Kotlin 依赖更复杂。
- 后端特定的 prefix/KV 行为不一定完全可移植。

## 后续事项

- 在写后端专用代码前先定义 `LlmEngine` capability。
- 先建立 profiling harness，再做优化。
- 在工作流层 prompt segment 复用证明有可测收益前，不修改后端 KV 内部实现。

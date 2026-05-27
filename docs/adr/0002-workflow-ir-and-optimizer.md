# 架构决策 0002：工作流 IR 与优化器

日期：2026-05-27

## 状态

已接受。

## 背景

项目需要支持用户定义工作流和 Agent 行为，但自由脚本很难做安全审计、优化和验证。把工作流表示为图，可以让运行时分析 observation 成本、prompt 构造、action 风险、retry 行为和 KV Cache 生命周期。

## 决策

将工作流表示为中间表示：节点有明确类型，副作用显式声明，风险可标注，验证边可追踪。工作流定义先编译成可执行计划，再交给运行时调度。

初始 IR 操作：

- `ObserveScreen`
- `NormalizeState`
- `PromptBuild`
- `LlmCall`
- `ParseToolCall`
- `Guard`
- `ExecuteAction`
- `WaitForState`
- `Verify`
- `TraceWrite`

初始编译优化 pass：

- schema validation；
- capability lowering；
- observation pruning；
- prompt constant folding；
- risk annotation；
- retry/verifier lowering；
- prompt segment 和 KV Cache 生命周期分析；
- cost planning。

## 理由

- 图结构让副作用可以在运行前被检查。
- Prompt segments 可以像值一样拥有 invalidation key 和生命周期。
- 静态工作流摘要是权限披露的必要基础。
- Verification 和 retry 可以一致地插入，而不是在每个工作流中临时拼接。
- 该设计在系统层面借鉴 TVM，而不是过早绑定 TVM 内部实现。

## 影响

正向影响：

- 工作流成为可测试工件。
- 优化可以逐步添加。
- 运行时能在执行前拒绝不安全或不支持的工作流。

负向影响：

- 工作流作者必须接受受约束的模型，而不是任意代码。
- 一些动态 Agent 行为必须表示为显式、有界的节点。
- 工作流持久化后必须维护 IR 版本。

## 后续事项

- 从第一次实现开始就给 IR 加版本。
- 为图校验和 liveness 增加 property tests。
- 为每个工作流生成人类可读的编译说明。

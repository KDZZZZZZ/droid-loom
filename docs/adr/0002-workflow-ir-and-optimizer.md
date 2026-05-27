# 架构决策 0002：工作流 IR 与优化器

日期：2026-05-27

## 状态

已接受。

## 背景

项目需要支持用户定义工作流和 Agent 行为，但自由脚本很难做安全审计、优化和验证。把工作流表示为图，可以让运行时分析 observation 成本、prompt 构造、action 风险、retry 行为和 KV Cache 生命周期。

成熟 Agent 框架的设计也指向同一个结论：

- ReAct 是“推理和动作交错”的 agent loop 模式，适合表达模型如何思考、调用工具、接收 observation。
- LangGraph 把重点放在 long-running、stateful agent orchestration，并提供 durable execution、human-in-the-loop、memory、streaming 和 persistence。
- OpenAI Agents SDK 有内置 agent loop，但把 handoffs、guardrails、sessions、human-in-the-loop 和 tracing 放在 loop 外面管理；其文档也建议需要自己控制 loop、tool dispatch、state handling 时直接使用 Responses API。
- Microsoft Agent Framework 明确区分 agent 和 workflow：agent 适合开放式或对话式任务，workflow 适合明确步骤、执行顺序控制、多 agent/function 协调，并提供 graph-based workflows、checkpointing 和 human-in-the-loop。
- LlamaIndex Workflows 和 CrewAI Flows 都把复杂 agent 应用放到 workflow/flow 层，用事件、步骤、状态、分支、循环和人类反馈来组织，而不是只暴露一个 ReAct loop。

因此 DroidLoom 的优化最高层不应定在 ReAct loop，而应定在 Agent Graph / ResponseGraph。ReAct 是一种可由 graph 表达的循环模式，不是全局优化的最高 IR，也不应作为 ResponseProgram 的内置字段。

## 决策

将工作流表示为中间表示：节点有明确类型，副作用显式声明，风险可标注，验证边可追踪。工作流定义先编译成可执行计划，再交给运行时调度。

优化最高层定为：

```text
WorkflowIR / AgentGraph
  -> ResponseGraph
      -> ResponseProgram / ResponseIR
          -> LocalResponseRequest
              -> LlmEngine / llama.h / MLC
```

定义：

- `WorkflowIR / AgentGraph` 是最高优化层，表达控制流、数据流、Android 能力、权限、风险、状态、verifier、retry、rollback 和跨节点 cache 生命周期。
- `ResponseGraph` 是 WorkflowIR 中 LLM/Agent reasoning 子图的 lower dialect，表达一组可执行的 response programs 以及它们之间的状态、tool output 和 cache 关系。
- `ResponseProgram / ResponseIR` 是单次模型交互的可执行程序对象，包含 input items、instructions、tools、tool visibility、guard binding、verifier binding、cache hints、trace metadata 和 structured output schema。
- `LocalResponseRequest` 是 ResponseProgram 的一次运行实例，是后端调用 ABI。
- ReAct/Plan-Act 等 agent loop 不进入 ResponseProgram；它们由 AgentGraph/ResponseGraph 的节点和边表达，例如 `ResponseProgram -> ToolDispatch -> ObserveScreen -> ResponseProgram`。

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
- `ResponseProgram`
- `ToolDispatch`
- `CachePlan`

初始编译优化 pass：

- schema validation；
- capability lowering；
- observation pruning；
- prompt constant folding；
- risk annotation；
- retry/verifier lowering；
- prompt segment 和 KV Cache 生命周期分析；
- ResponseGraph common prefix hoisting；
- model-visible/runtime-only/compiler-only tool partition；
- side-effect ordering；
- guard/verifier dominance check；
- cost planning。

## 理由

- 图结构让副作用可以在运行前被检查。
- Prompt segments 可以像值一样拥有 invalidation key 和生命周期。
- 静态工作流摘要是权限披露的必要基础。
- Verification 和 retry 可以一致地插入，而不是在每个工作流中临时拼接。
- 该设计在系统层面借鉴 TVM，而不是过早绑定 TVM 内部实现。
- ReAct loop 只覆盖“模型思考和动作交错”，无法单独承载跨节点 KV 生命周期、权限提升、verifier 支配关系、rollback、并行工具、安全确认和 trace replay。
- Agent Graph / ResponseGraph 能在 LLM 调用之前做全图优化，尤其适合 DroidLoom 的 prompt segment hoisting、tool schema 常量折叠、cache invalidation 和 Android action side-effect 排序。
- ResponseProgram 保留 OpenAI Responses-like 的执行形态，使上层 workflow 和底层 LLM backend 可以通过稳定的 lower dialect 并行开发。

## 影响

正向影响：

- 工作流成为可测试工件。
- 优化可以逐步添加。
- 运行时能在执行前拒绝不安全或不支持的工作流。

负向影响：

- 工作流作者必须接受受约束的模型，而不是任意代码。
- 一些动态 Agent 行为必须表示为显式、有界的节点。
- 工作流持久化后必须维护 IR 版本。
- ReAct-style 自由循环需要被展开成有界 graph pattern 或显式 loop edge，不能藏在 ResponseProgram 内部，否则无法静态估算成本和风险。
- compiler 需要维护 WorkflowIR、ResponseGraph、ResponseProgram、LocalResponseRequest 四层 lowering 关系，初期实现成本更高。

## 后续事项

- 从第一次实现开始就给 IR 加版本。
- 为图校验和 liveness 增加 property tests。
- 为每个工作流生成人类可读的编译说明。
- 增加 ResponseGraph lowering tests，验证 `workflowNodeId`、`responseProgramId`、`toolCallId`、`promptSegmentId`、`cachePlanId` 的双向映射。
- 增加 ReAct-as-Graph 示例，确保 ReAct 可以由 graph pattern 表达，但不能绕过 graph-level guard/verifier。

## 参考资料

- ReAct: https://arxiv.org/abs/2210.03629
- LangGraph overview: https://docs.langchain.com/oss/python/langgraph/overview
- OpenAI Agents SDK: https://openai.github.io/openai-agents-python/
- Microsoft Agent Framework overview: https://learn.microsoft.com/en-us/agent-framework/overview/
- LlamaIndex Workflows: https://docs.llamaindex.ai/en/stable/workflows/
- CrewAI Flows: https://docs.crewai.com/en/concepts/flows

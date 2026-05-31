# Graph 与多 Agent 编排

日期：2026-05-31

## 范围

本文描述 agent core 当前的 graph runtime。graph 的核心抽象是 message output log、output port、edge 和 input package。旧的 start node / activation condition / full inherited context runner 已经被替换。

TurnLoop 是会话级调度，不放在本文。TurnLoop、session tree、持久化、replay 见 [06-session](../06-session/README.md)。

## 核心模型

一次 graph run 从初始 messages 开始。runtime 把初始 messages 写到固定 output ref：

```text
("input", "messages")
```

每个 node 有一个 input package spec。package 由 required items 和 optional items 组成：

- required items 全满足后，node 可以解锁运行；
- optional items 不阻塞解锁，但会作为上下文传入 node；
- package version 变化后，runtime 会重新判断是否需要激活 node；
- 被 query 筛掉的 message 不进入 package，也不算新内容；
- selected fields 的 hash 不变时，同一 edge 不重复传输。

edge 的职责只有一件事：把某个 output ref 的新 message 按目标 package spec 过滤、投影、去重后，写入目标 package。

```text
(source_node, output_port) --edge--> (target_node, input_package)
```

因此“上下文继承”和“解锁条件”是统一的：下游要求的内容包就是它能继承的上下文；required 内容包是否满足就是解锁条件。

## Output 分流

output 分流分两层：

1. port 分流：executor 在 `NodeOutput` 里按 output port 写消息，例如 `tool_calls`、`results`、`final`。
2. package item 筛选：edge 把 port 上的 message 送到目标 package 后，`MessageQuery` 再按字段筛选并选择要传输的字段。

```rust
let output = NodeOutput::new()
    .with_message("tool_calls", assistant_tool_call_message)
    .with_message("final", assistant_final_message);

let graph = Graph::builder("react")
    .edge("agent_to_tool", ("agent", "tool_calls"), ("tap_tool", "calls"))
    .edge("agent_to_final", ("agent", "final"), ("final", "answer"))
    .build()?;
```

多个 edge 可以从同一个 output port fan-out；不同语义的输出应优先写到不同 port。

## Node

node 是一等执行单元。当前公开类型是 `graph_node::GraphNode`，它是 runtime `NodeSpec` 的别名。

node kind：

- `Transform { executor, config }`：普通计算或 adapter 节点；
- `Tool(ToolNodeSpec)`：单独工具节点；
- `Agent(AgentNodeSpec)`：单独 agent 节点；
- `Graph { graph_name }`：子图节点；
- `Final`：终止策略节点，不需要输出消息。

node executor 是 async trait：

```rust
pub trait NodeExecutor: Send + Sync {
    fn execute(
        &self,
        node: NodeSpec,
        input: NodeInput,
        ctx: NodeExecutionContext,
    ) -> BoxFuture<'static, AgentCoreResult<NodeOutput>>;
}
```

`GraphRuntime` 用 `FuturesUnordered` 同时驱动已解锁 node。`NodeConcurrency` 控制单个 node 的 activation 并发：

- `Serial`：同一 node 一次只跑一个 activation；
- `Parallel { max }`：同一 node 最多同时跑 `max` 个 activation；
- `ByKey { key, max_per_key }`：按 package item 的 selected hash 分组限流。

## Runner

`GraphRunner` 是同步门面，内部调用 async `GraphRuntime`。需要异步环境时可以直接用 `graph_runtime::GraphRuntime`。

runner 负责：

- 写入初始 messages；
- 扫描 edge 的未读 output log entries；
- 更新 package states；
- 生成 node activations；
- 按并发策略启动 node executor future；
- 提交 node output；
- 记录 transfer ledger、node attempt ledger 和 runtime events；
- 根据 `finish_at` 和 tick budget 结束 run。

`GraphRunResult.state` 是 debug/checkpoint 用的 runtime state，包含 output logs、edge cursors 和 package states。外部不要直接可变写入 state。

## 默认 ReAct Template

默认 ReAct graph 是一个普通内容包 graph：

```text
input.messages -> agent.context
agent.tool_calls -> tool.calls
tool.results -> agent.context
agent.final -> final.answer
```

agent node 通过 `context` package 同时接收用户 turn 和工具结果。tool node 单独接收 `tool_calls` port 上的工具调用。final node 接收 `agent.final` port 上的最终回答。

这不是特殊循环。ReAct loop 只是 graph 中的普通回边：

```text
tool.results -> agent.context
```

## 多 Agent 配合

多 agent 不靠 edge 执行。edge 只传内容。真正调用 child agent、工具、provider 或子图的逻辑都在 `NodeExecutor`。

父 graph 可以把一个 node 声明成 `NodeKind::Agent`，executor 根据 `AgentNodeSpec.agent_name` 路由到对应 child agent，并把 child agent 的产物写到明确 output port。父 graph 只继承被 edge 和 package query 选中的内容，不继承 child 的中间噪声。

## 停止条件

graph run 在以下情况停止：

- `finish_at` 指向的 final node 完成，状态为 `Completed`；
- 没有 activations、没有 running futures，状态为 `Drained`；
- `GraphRunInput.max_ticks` 超限，状态为 `BudgetExceeded`；
- node executor 返回错误，状态为 `Failed`；
- `GraphRunner` 收到启动前 stop request，状态为 `Cancelled`。

## 不变量

- edge 不执行 provider、tool、agent 或持久化副作用；
- 被筛选掉的内容不算新内容；
- package item 的 selected field hash 不变时不重复传输；
- node 是否解锁只由 input package 的 required items 决定；
- output port 是稳定分流边界；
- runtime 可以并行执行已解锁 node；
- session tree 是历史权威，graph runtime state 是单次 run 工作状态。

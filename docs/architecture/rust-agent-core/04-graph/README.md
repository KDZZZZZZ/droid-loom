# Graph 与多 Agent 编排

日期：2026-05-31

## 范围

本文描述 agent core 当前的 graph runtime。graph 的核心抽象是 node message log、edge 和 input package。旧的 start node、activation condition、全量继承上下文和端口分流都已经被替换。

TurnLoop 是会话级调度，不放在本文。TurnLoop、session tree、持久化、replay 见 [06-session](../06-session/README.md)。

## 核心模型

一次 graph run 从初始 messages 开始。runtime 把初始 messages 写入虚拟源节点：

```text
input
```

每个 node 有一个 input package spec。package 由 required items 和 optional items 组成：

- required items 全满足后，node 可以解锁运行；
- optional items 不阻塞解锁，但会作为上下文传入 node；
- package version 变化后，runtime 会重新判断是否需要激活 node；
- 同一条 message 同时匹配多个 package items 时，所有 matched content 写入后 package version 只递增一次；
- 被 query 筛掉的 message 不进入 package，也不算新内容；
- selected fields 的 hash 不变时，同一 edge 不重复传输；
- `content[*]` query 命中某个 block 时，select 只暴露匹配 blocks，不把同一 message 的其它 blocks 混入下游。

edge 的职责只有一件事：从上游 node message log 读取未传输的新 message，按目标 package spec 过滤、投影、去重后，写入目标 package。

```text
source_node --edge--> (target_node, input_package)
```

因此“上下文继承”和“解锁条件”是统一的：下游要求的内容包就是它能继承的上下文；required 内容包是否满足就是解锁条件。

## 消息筛选

graph 不再有端口分流。node executor 只返回一组 `RunMessage`，分流完全由下游 package 的 `MessageQuery` 表达。

```rust
let result = NodeResult::new()
    .with_message(assistant_tool_call_message)
    .with_message(assistant_final_message);

let graph = Graph::builder("react")
    .edge("agent_to_tool", "agent", ("tap_tool", "calls"))
    .edge("agent_to_final", "agent", ("final", "answer"))
    .build()?;
```

上例中，tool node 的 `calls` package 用 `content[*].type == "tool_call"` 过滤；final node 的 `answer` package 用 `content[*].text` 或业务 metadata 过滤。同一条上游消息是否进入某个下游，完全由下游 package query 决定。

## Node

node 是一等执行单元。当前公开类型是 `graph_node::GraphNode`，它是 runtime `NodeSpec` 的别名。

node kind：

- `Transform { executor, config }`：普通计算或 adapter 节点；
- `Tool(ToolNodeSpec)`：单独工具节点；
- `Agent(AgentNodeSpec)`：单独 agent 节点；
- `Graph { graph_name }`：子图节点；
- `Final`：终止策略节点。

node executor 是 async trait：

```rust
pub trait NodeExecutor: Send + Sync {
    fn execute(
        &self,
        node: NodeSpec,
        input: NodeInput,
        ctx: NodeExecutionContext,
    ) -> BoxFuture<'static, AgentCoreResult<NodeResult>>;
}
```

`GraphRuntime` 用 `FuturesUnordered` 同时驱动已解锁 node。`NodeConcurrency` 控制单个 node 的 activation 并发：

- `Serial`：同一 node 一次只跑一个 activation；
- `Parallel { max }`：同一 node 最多同时跑 `max` 个 activation；
- `ByKey { key, max_per_key }`：按 package item 的 selected hash 分组限流。

## 执行入口

普通外部调用方通过 `TurnLoop` 启动 graph。`TurnLoop` 负责追加/继承消息、构建本轮 graph context、调用内部 runner/runtime，并把 graph 新产生的消息追加回会话历史。

`GraphRuntime` 是下层 async 调度器，只有 async 宿主集成、runtime 级测试或特殊调度需求才需要直接使用。

runtime 负责：

- 写入初始 messages；
- 扫描 edge 对应的上游 node message log；
- 更新 package states；
- 生成 node activations；
- 按并发策略启动 node executor future；
- 提交 node result messages；
- 记录 transfer ledger、node attempt ledger 和 runtime events；
- 根据 `finish_at` 和 tick budget 结束 run。

`GraphRunResult.state` 是 debug/checkpoint 用的 runtime state，包含 `message_logs`、edge cursors 和 package states。外部不要直接可变写入 state，也不要把它当作会话历史；会话历史以 `TurnLoop::messages()` 为准。

## 默认 ReAct Template

默认 ReAct graph 是一个普通内容包 graph：

```text
input -> agent.context
agent -> tool.calls
tool -> agent.context
agent -> final.answer
```

agent node 通过 `context` package 同时接收用户 turn 和工具结果；用户 turn 应用 `role == "user"` query，避免被 tool result 覆盖。tool node 通过 `content[*].type == "tool_call"` 接收工具调用。final node 通过文本或业务 metadata query 接收最终回答。

这不是特殊循环。ReAct loop 只是 graph 中的普通回边：

```text
tool -> agent.context
```

## 多 Agent 配合

多 agent 不靠 edge 执行。edge 只传内容。真正调用 child agent、工具、provider 或子图的逻辑都在 `NodeExecutor`。

父 graph 可以把一个 node 声明成 `NodeKind::Agent`，executor 根据 `AgentNodeSpec.agent_name` 路由到对应 child agent，并把 child agent 产出的 `RunMessage` 写回该 node 的 message log。父 graph 只继承被 edge 和 package query 选中的内容，不继承 child 的中间噪声。

## 停止条件

graph run 在以下情况停止：

- `finish_at` 指向的 final node 完成，状态为 `Completed`；
- 没有 activations、没有 running futures，状态为 `Drained`；
- `GraphRunInput.max_ticks` 超限，状态为 `BudgetExceeded`；
- node executor 返回错误，状态为 `Failed`；如果 executor 明确返回 cancellation error，状态为 `Cancelled`；
- `TurnLoop` 收到 stop request 后不再启动新 turn；底层 runner 收到启动前 stop request 时状态为 `Cancelled`。

## 不变量

- edge 不执行 provider、tool、agent 或持久化副作用；
- 被筛选掉的内容不算新内容；
- package item 的 selected field hash 不变时不重复传输；
- node 是否解锁只由 input package 的 required items 决定；
- node 不声明输出分流，所有继承和分流都由下游 package query 完成；
- runtime 可以并行执行已解锁 node；
- session tree 是历史权威，graph runtime state 是单次 run 工作状态。

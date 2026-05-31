# Graph Runtime Reimplementation Notes

日期：2026-05-31

## 目标

`crates/agent-core` 的 graph runtime 已收敛为 message log 驱动的内容包调度器。它不是流程图触发器，也不是特殊 ReAct loop。

当前设计只保留四个核心概念：

- 上游 node 生产 `RunMessage`；
- edge 从 source node message log 读取下游尚未接收的新 message；
- 下游 node 声明自己的 input package；
- package 的 required content 满足时，node 解锁执行。

不再存在端口分流。分流和上下文继承都由下游 package 的 `MessageQuery` 表达。

## Eino 借鉴

Eino 的 Graph/Chain 重点不是给 message 外面再造一层通用 payload，而是让 ChatModel、ToolsNode 等路径直接围绕 message 流动；需要更细传递时再做字段级映射。

agent-core 采用同样方向：语义数据单位继续是 `RunMessage`，`ContentBlock` 承载 text、reasoning、tool call、tool result、diagnostic 等内容。Graph runtime 只增加日志、edge cursor、package state、node activation 这些执行元信息。

## 核心流转

```text
RunMessage -> NodeMessageLog -> EdgeState -> InputPackage -> NodeActivation
```

### Node Message Log

每个 node 有一个 append-only message log。node executor 返回 `NodeResult` 后，runtime 把其中的 messages commit 到该 node 的 log。

```rust
pub struct MessageLogEntry {
    pub seq: u64,
    pub node: NodeId,
    pub message: RunMessage,
    pub message_version: u64,
}
```

这里的类型名暂时仍叫 `MessageLogEntry`，但语义已经是 node message log entry；公开状态字段是 `GraphRuntimeState.message_logs`。

### Edge

edge 不带解锁条件，不带 selector，不带执行语义。edge 只声明 source node 的 message log 会进入哪个下游 package。

```rust
pub struct GraphEdge {
    pub id: EdgeId,
    pub from: NodeId,
    pub to: PackageRef,
}

pub struct PackageRef {
    pub node: NodeId,
    pub package: InputPackageName,
}
```

edge 每次从自己的 `EdgeState.next_seq` 开始扫描 source node message log。只有目标 package 的 item query 命中的内容才会写入 package；被筛掉的 message 不算新内容。

### Input Package

下游 node 用 `InputPackageSpec` 声明内容包：

```rust
InputPackageSpec::new("context")
    .required("turn", MessageQuery::where_eq("role", "user"), Cardinality::Latest)
    .optional(
        "tool_result",
        MessageQuery::where_eq("content[*].type", "tool_result"),
        Cardinality::Latest,
    );
```

required items 是解锁条件；optional items 是可继承上下文。两者本质上都是“下游要求的内容包”。

### Node Result

node executor 不声明分流。它只返回 messages：

```rust
let result = NodeResult::new()
    .with_message(assistant_tool_call_message)
    .with_message(assistant_final_message);
```

这些 messages 进入同一个 source node log。哪些下游会继承它们，由各个 edge 的目标 package query 决定。

## ReAct 表达

普通 ReAct 只是 graph 中的回边：

```text
input -> agent.context
agent -> tool.calls
tool -> agent.context
agent -> final.answer
```

tool node 的 package query 只接收 `content[*].type == "tool_call"`；agent 的 context package 接收 user turn 和 tool result；final package 接收最终文本或业务 metadata。没有 hardcoded loop，也没有 output 分流。

## 并行执行

runtime 用 Rust async 驱动已解锁 node：

- edge 扫描可以不断发现新的 package version；
- `FuturesUnordered` 同时推进 running node futures；
- `NodeConcurrency::Serial`、`Parallel`、`ByKey` 控制同一 node 的并发；
- node 一旦产出新 messages，下游 edge 下一轮扫描即可看到。

## 不变量

- message 是唯一语义载体；
- edge 只传内容，不执行工具、provider、agent 或 session 写入；
- 被筛掉的内容不算新内容；
- selected fields hash 不变时，不重复推进 package version；
- node 解锁条件和上下文继承统一由 input package 表达；
- 不继承中间噪声，除非下游 package query 明确要求。

# Graph Runtime Reimplementation Plan

日期：2026-05-31

## 目标

重新实现 `crates/agent-core` 的 graph runtime。
新的 graph 不应该是流程图触发器，也不应该是特殊 ReAct loop。
它应该是一个 message log 驱动的内容包调度器：

- 上游 node 只生产 `RunMessage`；
- edge 只负责从上游 output log 传输下游需要的新内容；
- 下游 node 声明自己需要的 input package；
- package 里的 required content 满足时，node 解锁；
- optional content 有就进入 package，没有也不阻塞；
- 普通 ReAct 是 self graph，不是 hardcoded loop。

## Eino 借鉴

Eino 的 Graph/Chain 重点不是给 message 外面再造一层通用数据结构，而是：

- 编排层是 node、edge 和上下游类型对齐；
- ChatModel、ToolsNode 这类 LLM/Tool 路径直接围绕 `schema.Message` 或 `AgenticMessage` 流动；
- `AgenticMessage` 用 `ContentBlock` 承载 reasoning、tool call、tool result 等事件；
- Workflow 在需要更细传递时做字段级映射。

参考：

- https://www.cloudwego.io/docs/eino/core_modules/chain_and_graph_orchestration/
- https://www.cloudwego.io/docs/eino/quick_start/chapter_01_chatmodel_and_message/
- https://www.cloudwego.io/zh/docs/eino/core_modules/chain_and_graph_orchestration/orchestration_design_principles/
- https://www.cloudwego.io/docs/eino/core_modules/chain_and_graph_orchestration/workflow_orchestration_framework/

结论：agent-core graph 的语义数据单位继续是 `RunMessage`。Graph runtime 只增加日志、路由、cursor、package state 这些执行元信息。

## 核心模型

整个 runtime 只围绕四段流转：

```text
RunMessage -> OutputLog -> EdgeState -> InputPackage -> NodeActivation
```

### Output Log

每个 node output port 是 append-only log。node 执行完成后，把输出 message commit 到对应 port 的 log。

```rust
pub struct OutputLogEntry {
    pub seq: u64,
    pub node: NodeId,
    pub port: OutputPort,
    pub message: RunMessage,
    pub message_version: u64,
}

pub struct OutputRef {
    pub node: NodeId,
    pub port: OutputPort,
}
```

`seq` 是 edge 扫描用的单调序号。`message_version` 用于审计和回放；是否触发下游不直接看 version，而看下游 query 选出来的内容是否是新内容。

### Edge

edge 不带解锁条件，不带 selector，不带执行语义。
edge 只声明一条上游 output log 会进入哪个下游 package。

```rust
pub struct GraphEdge {
    pub id: EdgeId,
    pub from: OutputRef,
    pub to: PackageRef,
}

pub struct PackageRef {
    pub node: NodeId,
    pub package: InputPackageName,
}
```

edge 的 runtime state 记录两件事：

- 上游 log 扫到哪里了；
- 哪些 matched content 已经传过了。

```rust
pub struct EdgeState {
    pub edge_id: EdgeId,
    pub next_seq: u64,
    pub delivered: BTreeSet<DeliveryKey>,
}

pub struct DeliveryKey {
    pub edge_id: EdgeId,
    pub package: InputPackageName,
    pub item: PackageItemName,
    pub message_id: Uuid,
    pub selected_hash: ContentHash,
}
```

注意：`DeliveryKey` 不把 `message_version` 放进去。
同一个 message 更新后，如果下游 query 选出来的字段没有变化，`selected_hash` 不变，就不算新内容。
如果是不同 message，即使选出来的字段相同，也会因为 `message_id` 不同而作为新内容传输。

### Input Package

下游 node 不声明“触发条件”，只声明自己需要的内容包。
解锁条件就是 required items 是否满足。

```rust
pub struct InputPackageSpec {
    pub name: InputPackageName,
    pub required: Vec<PackageItemSpec>,
    pub optional: Vec<PackageItemSpec>,
}

pub struct PackageItemSpec {
    pub name: PackageItemName,
    pub query: MessageQuery,
    pub cardinality: Cardinality,
}

pub struct MessageQuery {
    pub filters: Vec<FieldFilter>,
    pub select: FieldMask,
}

pub struct FieldFilter {
    pub path: FieldPath,
    pub op: FieldOp,
}

pub struct FieldMask {
    pub include: Vec<FieldPath>,
}

pub struct FieldPath(pub String);

pub enum FieldOp {
    Exists,
    Eq(Value),
    In(Vec<Value>),
}

pub enum Cardinality {
    Latest,
    One,
    AtLeast(usize),
}
```

`FieldPath` 是 `RunMessage` 字段路径，例如：

- `role`
- `status`
- `source_node_id`
- `metadata.kind`
- `content[*].type`
- `content[*].tool_name`
- `content[*].arguments`
- `content[*].is_error`

`select.include` 为空表示把完整 `RunMessage` 放进 package item。非空表示 node input 只看到这些字段。

### Package State

package state 保存已经匹配到的内容。

```rust
pub struct PackageState {
    pub node: NodeId,
    pub package: InputPackageName,
    pub version: u64,
    pub items: BTreeMap<PackageItemName, PackageItemState>,
    pub last_activated_version: Option<u64>,
}

pub struct PackageItemState {
    pub matches: Vec<MatchedContent>,
}

pub struct MatchedContent {
    pub source: OutputRef,
    pub message_id: Uuid,
    pub message_version: u64,
    pub selected: SelectedFields,
    pub selected_hash: ContentHash,
}
```

package 的 `version` 只在写入新的 matched content 时递增。
被 query 筛掉的 message 不写入 package，不推进 package version，不触发 node。

package ready 的规则：

```text
ready(package) =
  every required item satisfies its cardinality
```

optional item 不参与 ready 判断。node activation 的规则：

```text
activate(node) =
  ready(node.input_package)
  && package.version != package.last_activated_version
  && node concurrency allows a new run
```

这使三种情况统一成同一个模型：

- 上游任意更新就工作：required item 使用 `MessageQuery::any()` + `Cardinality::Latest`；
- 等一组内容：声明多个 required items；
- 等上游指令：声明一个 required item，query 要求 `metadata.kind == "instruction"` 或 `content[*].type == "instruction"`。

任意更新不是特殊 wake policy，它只是一个 required package item。

## Edge 投递算法

edge 每次处理自己的 source output log：

1. 从 `EdgeState.next_seq` 开始读取 source output log。
2. 对每条 `OutputLogEntry`，读取 target node 的 `InputPackageSpec`。
3. 用 package 的 required/optional items 逐个匹配该 message。
4. 如果没有任何 item 匹配，只推进 `next_seq`，不写 package，不写 transfer ledger。
5. 如果某个 item 匹配，按 `query.select` 取字段并计算 `selected_hash`。当 query 命中 `content[*]` 的某个 block 时，`content[*]` 选择只暴露匹配的 blocks。
6. 生成 `DeliveryKey(edge, package, item, message_id, selected_hash)`。
7. 如果 key 已经在 `EdgeState.delivered`，跳过。
8. 如果 key 没出现过，写入 `PackageState.items[item]`，记录 transfer ledger。
9. 同一个 output entry 的所有新 matched content 写完后，package version 只递增一次。
10. 处理完该 output entry 后推进 `next_seq`。

关键约束：

- 被筛选掉的内容不是新内容；
- 只有匹配 target package item 的内容才可能是新内容；
- message version 变化但 selected fields 不变，不触发下游；
- package version 变化才可能产生 node activation。

## Node

第一版每个 node 只有一个 input package。需要多个来源、多个条件、可选上下文时，都放进同一个 package 的 items 里表达。

```rust
pub struct NodeSpec {
    pub id: NodeId,
    pub kind: NodeKind,
    pub input: InputPackageSpec,
    pub outputs: Vec<OutputPort>,
    pub concurrency: NodeConcurrency,
}

pub enum NodeKind {
    Transform { executor: String, config: Value },
    Tool(ToolNodeSpec),
    Agent(AgentNodeSpec),
    Graph { graph_name: String },
    Final,
}

pub enum NodeConcurrency {
    Serial,
    Parallel { max: usize },
    ByKey { key: ConcurrencyKey, max_per_key: usize },
}
```

### Node Activation

node 可以随时解锁，不需要等待一个全局 graph round 结束。
任意 edge scan 只要写入了新的 matched content，导致 package version 变化，scheduler 就立刻重新判断 package ready。

```rust
pub struct NodeActivation {
    pub id: Uuid,
    pub node: NodeId,
    pub package: InputPackageName,
    pub package_version: u64,
    pub input: NodeInput,
}
```

activation 的生成规则只有一个：

```text
if ready(package)
   && package.version != package.last_activated_version
   && node.concurrency admits another activation
then create NodeActivation
```

这意味着：

- node 可在任何上游新内容到达后解锁；
- 多个 node 的 packages 同时 ready 时，可以同时进入 activation queue；
- 同一个 node 是否能并行执行由 `NodeConcurrency` 决定；
- self graph ReAct 的 agent node 默认应使用 `Serial`，避免同一个上下文上并发生成多个 assistant turn；
- 独立只读 tool node 可以用 `Parallel { max }`；
- 需要设备/action lock 的 mutating tool node 应使用 `Serial` 或 `ByKey`。

node executor 收到的是 package snapshot：

```rust
pub struct NodeInput {
    pub package: InputPackageName,
    pub version: u64,
    pub required: BTreeMap<PackageItemName, Vec<MatchedContent>>,
    pub optional: BTreeMap<PackageItemName, Vec<MatchedContent>>,
}

pub struct NodeOutput {
    pub messages: BTreeMap<OutputPort, Vec<RunMessage>>,
}
```

tool node 和 agent node 只是不一样的 executor，不是特殊 graph 机制。

```rust
pub struct ToolNodeSpec {
    pub tool_name: String,
    pub call_item: PackageItemName,
    pub result_port: OutputPort,
}

pub struct AgentNodeSpec {
    pub agent_name: String,
    pub graph: GraphRef,
}

pub enum GraphRef {
    SelfGraph,
    Named(String),
}
```

tool node 可以是单独工具 node，也可以是 dispatcher node：

```text
agent.tool_calls -> tap_tool_node -> agent.context
agent.tool_calls -> swipe_tool_node -> agent.context
agent.tool_calls -> tool_dispatcher -> agent.context
```

agent node 也是单独 node：

```text
manager_agent -> worker_agent -> manager_agent
phone_agent -> tool_node -> phone_agent
```

两者都通过 `InputPackageSpec` 接收内容，通过 `NodeOutput` 写回 output log。
Graph runtime 不需要知道“这是 ReAct 的第几步”，只需要执行 ready 的 node activation。

`GraphRef::SelfGraph` 表示普通 ReAct：agent 输出 tool call，tool node 输出 tool result，tool result 通过 edge 回到 agent 的 input package，直到 agent 输出 final。

## ReAct Graph

ReAct 不需要特殊 runner。

```rust
let graph = GraphSpec::builder("phone_react")
    .node(NodeSpec::agent(
        "agent",
        "phone_agent",
        InputPackageSpec::new("context")
            .required("turn", MessageQuery::where_eq("role", "user"), Cardinality::Latest)
            .optional("tool_result",
                MessageQuery::where_eq("content[*].type", "tool_result")
                    .select(["id", "role", "content[*]"]),
                Cardinality::Latest))
        .output("tool_calls")
        .output("final"))
    .node(NodeSpec::tool(
        "tools",
        "android_tool_dispatch",
        "tool_call",
        "results",
        InputPackageSpec::new("calls")
            .required("tool_call",
                MessageQuery::where_eq("content[*].type", "tool_call")
                    .select([
                        "id",
                        "content[*].call_id",
                        "content[*].tool_name",
                        "content[*].arguments",
                    ]),
                Cardinality::AtLeast(1))))
    .node(NodeSpec::final_node(
        "final",
        InputPackageSpec::new("answer")
            .required("final_answer",
                MessageQuery::where_exists("content[*].text")
                    .select(["id", "role", "content[*].text"]),
                Cardinality::Latest)))
    .edge("input_to_agent", ("input", "messages"), ("agent", "context"))
    .edge("agent_to_tools", ("agent", "tool_calls"), ("tools", "calls"))
    .edge("tools_to_agent", ("tools", "results"), ("agent", "context"))
    .edge("agent_to_final", ("agent", "final"), ("final", "answer"))
    .finish_at("final")
    .build()?;
```

这张图里没有 ReAct loop 特判。
loop 是 `tools_to_agent` 这条 edge 把 tool result 写回 agent 的 `context` package 后自然形成的。

## Runtime Loop

```text
1. commit initial RunMessage to graph input output log
2. scan edges from EdgeState.next_seq
3. match source messages against target package items
4. insert only new matched content into PackageState
5. if package ready and version changed, create NodeActivation
6. run activations subject to NodeConcurrency
7. commit NodeOutput messages to output logs
8. repeat edge scan until finish policy is satisfied or budget stops
```

runtime state：

```rust
pub struct GraphRuntimeState {
    pub output_logs: BTreeMap<OutputRef, Vec<OutputLogEntry>>,
    pub edge_states: BTreeMap<EdgeId, EdgeState>,
    pub package_states: BTreeMap<PackageRef, PackageState>,
}

pub struct GraphRunLedger {
    pub transfers: Vec<EdgeTransferRecord>,
    pub node_attempts: Vec<NodeAttemptRecord>,
    pub events: Vec<RuntimeEvent>,
}

pub struct EdgeTransferRecord {
    pub edge_id: EdgeId,
    pub from: OutputRef,
    pub to: PackageRef,
    pub item: PackageItemName,
    pub message_id: Uuid,
    pub message_version: u64,
    pub selected_hash: ContentHash,
}
```

## Async Runtime Model

这个 graph runtime 应该用 Rust async，但不是把所有逻辑都 async 化。

同步部分：

- output log append；
- edge cursor scan；
- message query/filter/select；
- package ready check；
- activation queue 更新；
- ledger 写入。

这些都是内存状态机，应该保持同步、确定、易测试。

异步部分：

- provider call；
- tool execution；
- agent node execution；
- graph/subgraph node execution；
- event sink；
- cancellation/deadline wait。

node executor 接口返回 `Future`。tool node 和 agent node 都通过 `NodeKind` 传给同一个 executor；运行时不做内联 dispatch。

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

scheduler 使用 `FuturesUnordered` 管理 running activations：

```rust
pub struct GraphRuntime {
    graph: GraphSpec,
    services: GraphRuntimeServices,
    state: GraphRuntimeState,
    activations: VecDeque<NodeActivation>,
    running: FuturesUnordered<RunningFuture>,
}
```

主循环是 async：

```rust
pub async fn run(mut self, input: GraphRunInput) -> AgentCoreResult<GraphRunOutput> {
    loop {
        self.scan_edges()?;
        self.enqueue_ready_activations()?;
        self.spawn_ready_activations()?;

        if let Some(result) = self.running.next().await {
            self.commit_running_result(result)?;
        }

        if self.finish_policy_satisfied() {
            return Ok(self.finish(GraphRunStatus::Completed, None));
        }
    }
}
```

`NodeConcurrency` 用 runtime 内部 running counts 控制：

- `Serial`：每个 node 一个 permit；
- `Parallel { max }`：每个 node 最多 `max` 个 running future；
- `ByKey`：每个 `(node, key)` 最多 `max_per_key` 个 running future。

这样多个 ready node 可以并行执行，单个 node 的并行度仍由 policy 控制。
edge scan 和 package matching 在 node future 完成后立刻继续运行，因此 graph 不需要全局轮次，也不需要阻塞等待某个慢工具。

## 实现阶段

### Phase 1: Model

新增最小模型：

- `OutputLogEntry`
- `GraphEdge`
- `EdgeState`
- `DeliveryKey`
- `InputPackageSpec`
- `PackageItemSpec`
- `MessageQuery`
- `PackageState`
- `NodeSpec`
- `NodeActivation`
- `NodeInput`
- `NodeOutput`

验收：

- edge 只传 matched content；
- filtered message 不推进 package version；
- required items 满足后 package ready；
- optional items 不阻塞 ready。

### Phase 2: Scheduler

新增：

- edge scan loop
- package ready check
- activation queue
- concurrency guard
- async running set

验收：

- 上游任意更新可由 required latest item 表达；
- 多 required items 全部满足后才执行；
- message version 变化但 selected fields 不变时不执行；
- 同一 matched content 不重复传输。
- 多个 ready node 可以并行进入 running；
- 单个 node 的并行度严格受 `NodeConcurrency` 控制。
- 慢 node future 不阻塞 edge scan 和其它 ready node。

### Phase 3: Tool And Agent Nodes

新增：

- `NodeExecutor`
- `GraphRef::SelfGraph`
- ReAct graph builder
- async executor interface

验收：

- `agent -> tool -> agent -> final` 完整运行；
- tool call/result 只是 `RunMessage.content` 中的 block；
- ReAct 不依赖 hardcoded loop。
- provider/tool/agent calls 通过 async future 并发执行。

### Phase 4: Runtime Services

新增：

- provider/tool registry 注入；
- event sink；
- cancellation/deadline；
- budget；
- execution ledger。

## 必须覆盖的测试

`crates/agent-core/src/graph/runtime.rs` 覆盖以下测试场景：

- Edge / Log：`edge_scans_only_unseen_entries`、`edge_ignores_unmatched_message`、`edge_delivers_matched_content_once`、`same_message_version_changed_but_selected_unchanged_no_activation`、`same_message_selected_field_changed_triggers_activation`、`different_message_same_selected_hash_still_delivered`。
- Input Package：`required_item_latest_materializes_only_latest`、`required_item_at_least_waits_until_enough_matches`、`multiple_required_items_all_must_be_ready`、`optional_item_does_not_block_ready`、`optional_item_included_when_available`、`filtered_content_does_not_increment_package_version`、`package_version_changes_once_per_new_matched_content_batch`。
- Source / Query：`agent_turn_query_does_not_match_tool_result`、`query_select_masks_unselected_fields_from_node_input`、`query_path_content_array_matches_nested_block`、`query_multiple_blocks_selects_only_matching_blocks`。
- Node Activation：`ready_package_creates_one_activation_per_version`、`new_package_version_creates_new_activation`、`node_not_activated_when_package_ready_but_version_unchanged`、`serial_node_prevents_concurrent_activations`、`parallel_node_allows_up_to_max`、`by_key_limits_concurrency_per_key`。
- Runtime Loop：`runtime_finishes_when_final_node_commits_output`、`runtime_does_not_finish_when_final_package_ready_but_node_not_run`、`runtime_returns_no_progress_when_no_running_no_activation_no_transfer`、`slow_node_does_not_block_independent_ready_node`、`edge_scan_continues_after_node_output_commit`。
- ReAct：`react_runs_agent_tool_agent_final_without_special_loop`、`agent_tool_call_routes_only_to_tool_node`、`agent_final_routes_only_to_final_node`、`tool_result_routes_back_to_agent_context`、`react_stops_on_final_even_if_previous_tool_results_exist`、`react_budget_stops_infinite_tool_loop`。
- Tool / Agent Executor：`tool_dispatcher_reads_call_item_and_emits_results_port`、`tool_node_preserves_call_id_in_result`、`tool_error_still_emits_tool_result_message`、`agent_node_receives_package_snapshot_not_live_state`。
- Cancel / Deadline / Error：`cancel_running_node_marks_graph_cancelled`、`deadline_exceeded_stops_new_activation`、`node_executor_error_records_attempt_and_policy_decides_status`、`recoverable_tool_error_can_continue_react`。
- Ledger / Replay：`ledger_records_edge_transfer`、`ledger_records_node_attempt_start_finish_status`、`replay_from_logs_reconstructs_package_state`、`deterministic_scan_order_produces_stable_activation_order`。

## 结论

统一抽象是内容包，不是 edge condition。

edge 只维护 cursor 和 delivery 去重；package 声明 required/optional content；package ready 就是解锁条件；node activation 只看 ready package 是否产生了新的 matched content。
这样任意更新、等待一组内容、等待上游指令都是同一套机制。

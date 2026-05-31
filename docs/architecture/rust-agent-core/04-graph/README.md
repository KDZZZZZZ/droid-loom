# Graph 与多 Agent 编排

日期：2026-05-30

## 范围

本文只描述单次 graph run、默认 ReAct graph template 和多 agent 编排。

TurnLoop 是会话级调度，不放在本文。TurnLoop、session tree、持久化、replay 见
[06-session](../06-session/README.md)。

## 使用指南

实现 `graph_*` 文件，或调整单次 agent run 编排时读本文。

分层原则：

- 用户输入入口只负责把输入转成 user message。
- TurnLoop 负责会话级 user message buffer、GenInput、preempt、stop、idle。
- graph 只负责单次 run 内的节点、edge、预算和状态推进。
- edge 是独立逻辑，不只是连线。edge 定义依赖关系、上下文继承规则和目标 node 启动条件。
- graph 内的 message/content block 见 [05-message](../05-message/README.md)。
- session 持久化和 replay 见 [06-session](../06-session/README.md)。

## 使用方案

graph 模块被 `Agent.run()` 调用，用来完成一次 agent run 的内部编排。

调用方：

- `Agent.run()`；
- 父 graph 中的 child agent node；
- 测试中的 graph runner harness。

输入：

- compiled graph template；
- 初始 `GraphState`；
- 本轮 user message；
- context/provider/tool/handler 等服务引用；
- run budget 和 cancellation token。

输出：

- graph result；
- graph run 内产生的 messages；
- runtime events；
- stop/error result。

典型调用流程：

1. `Agent.run()` 选择 graph template。
2. graph runner 创建 runnable set，从 start node 开始。
3. node 在 exec 中实时产生 message。
4. runner 写入 `GraphState`，递增该 node 的 message version。
5. runner 把 source message 交给对应 edge 判定。
6. edge 返回 `sleep` 或 `activate`。
7. `activate` 的 target node 进入 runnable set。
8. runner 继续推进，直到 end、stop、budget exceeded、error 或 cancellation。

不能这样用：

- 不要让 edge 执行 provider、tool 或持久化。
- 不要让单条 edge 管理循环次数。
- 不要把 TurnLoop、session tree、原始用户输入入口放进 graph。
- 不要把 summary/isolated/selected 做成 edge primitive；需要时让上游 node 产出显式字段，或让 target node 自己构造输入。

## 内部文件架构

### `src/agent/agent.rs` 与 graph runner

不单独设计 `loop.rs`。`Agent.run()` 直接负责单次 graph run 的入口工作：

- 选择 graph template；
- 创建初始 `GraphState`；
- 调用 `src/graph/runner.rs`；
- 规范化 graph result。

`Agent.run()` 不直接执行 provider、tool 或 child `Agent`。真正的节点执行、edge 评估和状态推进都在
`src/graph/runner.rs`。

当 graph 需要运行某个 agent 时，它调用对应的 `Agent`。graph 只关心 agent 的输入、事件和结果，
不展开 agent 内部 lifecycle。

### `src/graph/graph.rs`

定义可编译 graph：node、edge、入口、出口和编译期校验。校验 start node、edge 目标、
end node 和循环预算。

graph 不应把 edge 简化成 `from -> to`。node 负责产出结果，edge 负责解释这些结果如何流向下游。

### `src/graph/edge.rs`

定义 edge 契约。edge 只判断某个 source node 实时产生的一条 message 是否激活 target node。

edge 至少应包含：

- edge id；
- source node id；
- target node id；
- inherit policy：第一版只保留 `full`。被激活的 target node 继承 source node 到当前 message version 为止的完整消息流。
- activation condition；
- priority 或 ordering hint。

activation condition 是只读判断：输入是 source message 和 `GraphState` 只读视图。
它不能执行 provider、tool 或持久化副作用。

edge 的输出是 `EdgeDecision`：

- sleep：目标 node 本次不启动；
- activate：目标 node 可启动，并携带 full inherited context。

edge 不管理循环数。循环保护属于 graph runner。

### `src/graph/node.rs`

定义节点契约。采用 prep/exec/post，但 exec 可以持续产生 graph message：

- prep 只读 state，准备节点输入；
- exec 执行可重试 I/O 或计算，并在运行中实时 emit message；
- post 收尾，产出 finalized message。

message 是 graph runner 可以观察的结构化输出，不等同于 UI token delta。
runner 收到 message 后立即写入 `GraphState`，并触发该 node 出边的 edge 判定。

### `src/graph/state.rs`

单次 run 的工作内存。包含 run id、turn id、effective config、context snapshot、provider request snapshot、
assistant builder、tool calls/results、approval requests、child run registry、usage、budget、trace。

GraphState 还要保存 node message 流和 graph 执行计数：

- per-node message stream；
- per-node message version；
- fired edge records，防止同一个 edge 对同一个 source message version 重复激活；
- total node executions；
- per-node execution count；
- max node executions；
- max total node executions。

它不是 session tree，也不是 TurnLoop buffer。

### `src/graph/runner.rs`

执行 compiled graph。负责 node lifecycle、edge 判定触发、budget、cancellation、retry/fallback 和 subgraph。

最大循环数属于 graph runner。runner 根据全图执行轨迹检查 graph budget：

- total node executions 是否超过上限；
- 某个 node 是否超过 per-node 上限；
- runnable set 是否长期没有新进展。

超过上限或长期无进展时，runner 生成 graph stop/error result。这样循环保护集中在 runner，
不散落到 node、edge、Agent 或 TurnLoop。

runner 的基本循环：

1. 找出 runnable nodes。
2. 启动 node prep/exec。
3. node 运行中产生 message。
4. runner 写入 `GraphState`，递增该 node 的 message version。
5. runner 把该 node 的出边交给对应 edge 做判定。
6. edge 返回 `sleep` 或 `activate`。
7. `activate` 的 target node 进入 runnable set。
8. runner 记录 fired edge，避免同一个 edge 对同一版 source message 重复激活。
9. node post 只产出 finalized message；finalized message 也按同一规则触发 edge。

graph run 在以下情况停止：

- assistant 没有 tool call；
- provider stream error 或 aborted；
- hook handler 请求 stop；
- tool result 请求 terminate；
- graph 到达 end node；
- graph total node executions 或 per-node executions 超上限；
- graph runner 检测到长期无进展；
- child agent 超过 depth、concurrency 或 budget；
- turn、tool、time、token budget guard 触发。

### `src/graph/templates.rs`

保存 default ReAct graph 和后续可选 template。默认 graph：

start -> before_turn -> build_context -> provider_request -> collect_assistant，
再根据 action 进入 final、tool_calls、continue、approval_required、error 等路径。

这个模板里的每条箭头都应落成 edge：

- `collect_assistant -> execute_tools`：source message action=tool_calls，inherit tool call blocks；
- `execute_tools -> build_context`：source message tool batch completed，inherit tool messages；
- `collect_assistant -> finalize_turn`：source message action=final，inherit assistant final message；
- `provider_request -> handle_error`：source message is_error=true，触发 recoverable error 处理或失败路径。

## 多 Agent 配合

多 agent 通过两条路径接入：

- 静态 graph：`child_agent` node 显式调用另一个 `Agent` 或子图。
- 动态 delegation：模型请求 delegation capability，由 core 创建或取得 child `Agent`。

child `Agent` 只代表 child agent 的内部生命周期。它不天然拥有自己的 TurnLoop 或 session tree。
是否给 child agent 单独开 session，由外层 session/driver 决定。父 graph 默认只接收 child final output、
status、usage、artifacts 和 error summary。

## 后续拓展方案

第一版 graph runner 串行执行，支持 edge activation、full context inheritance、max total node executions、
per-node max executions、no-progress guard、cancellation。

第二版加入 child_agent node、foreground delegation、subgraph。

第三版加入并行 tool batch、并行 child agents、merge strategy、supervisor graph。

第四版加入持久化 checkpoint、background resume、graph visualization 和 debug UI。

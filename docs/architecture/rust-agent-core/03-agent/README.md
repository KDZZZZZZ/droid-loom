# Agent Definition、Factory 与 Agent

日期：2026-05-30

## 范围

本文描述 agent 的最小配置层、factory 装配层和可调用 agent 对象。`src/agent/definition.rs` 只保存配置声明，
不保存运行态，也不负责执行具体加载逻辑。单个 agent 生命周期状态直接放在 `src/agent/agent.rs` 内部，不再单独设计
`agent_runtime.rs` 文件。

## 使用指南

实现 agent public API、加载配置、多线程启动已定义 agent 的单次生命周期时读本文。

判断规则：

- 描述“这个 agent 暴露哪些能力”的声明放进 `AgentDefinition`。
- 描述“把 definition 和各层服务接起来”的装配放进 `AgentFactory`。
- 描述“单个 agent 生命周期内部正在发生什么”的状态放进 `src/agent/agent.rs` 的 `Agent` 内部。
- `src/agent/agent.rs` 可以承载 agent lifecycle，但不能承载 session、TurnLoop、message loading 或 tool loading。

## 使用方案

agent 相关代码提供“定义一个 agent”和“启动一次 agent 生命周期”两类能力。

调用方：

- CLI/config loader：创建 `AgentDefinition`。
- TurnLoop 或 graph driver：调用 `Agent.run()`。
- 父 graph：把 child agent 当作可调用节点使用。

输入：

- agent name；
- system prompt；
- tool visibility policy；
- provider registry、tool registry、context builder、hook handler registry 等依赖引用；
- 本轮 run input。

输出：

- `AgentDefinition`；
- 可运行的 `Agent`；
- agent run result、stream events、取消或失败状态。

典型调用流程：

1. 配置层创建 `AgentDefinition`，只声明 name、system prompt、tool visibility。
2. `AgentFactory` 校验 definition，并注入 core 服务依赖。
3. factory 创建 `Agent`。
4. TurnLoop 把本轮 user message 或 run input 交给 `Agent.run()`。
5. `Agent.run()` 创建 graph run，等待 graph runner 返回结果。
6. 调用方读取 result/events，不直接读取 agent 内部 mutable state。

不能这样用：

- 不要把默认模型、预算、handler、graph template 塞进 `AgentDefinition`。
- 不要让 factory 解析 tool schema 或拼 provider context。
- 不要让 `src/agent/agent.rs` 管理 session tree、TurnLoop buffer、message loading。
- 不要重新拆 `agent_runtime.rs`，除非 `src/agent/agent.rs` 已经无法维护。

## 内部文件架构

### `src/agent/agent.rs`

public facade 和可调用 agent 对象。它提供启动、恢复、取消一个 agent 生命周期的入口。

`src/agent/agent.rs` 可以持有单个 agent 生命周期内部状态：

- agent id；
- definition 引用或快照；
- lifecycle phase：initialized、starting、running、suspended、finishing、completed、failed、cancelled；
- 当前 graph run id 或 graph execution handle；
- 当前 streaming assistant view；
- 当前 pending tool calls / approval requests；
- 当前 child agent lifecycle summaries；
- 当前 usage、error、cancellation token。

对 graph 来说，`Agent` 就是可调用的 agent 单元。graph 不直接操作 definition，也不自己拼 agent 生命周期；
它只把准备好的 run input/context 交给 `Agent`，等待 `Agent` 返回 agent result 或 stream events。

`Agent` 对 graph 暴露的能力应保持很小：

- start/run：启动一次 agent 生命周期；
- resume：从 suspended lifecycle 继续；
- cancel：取消当前 agent 生命周期；
- result/events：返回最终结果或流式事件。

这些职责不属于 `src/agent/agent.rs`：

- TurnLoop buffer、GenInput、preempt、session stop；
- session tree、active leaf、replay、compaction；
- user message 构造；
- tool 可见性解析和 tool schema 加载；
- system prompt 转 provider instructions 和 context 拼装；
- session entry 写入；
- UI/TUI 订阅者管理。

### `src/agent/definition.rs`

不可变 agent 配置，只保留第一版真正需要的声明：

- agent name；
- system prompt；
- tool visibility policy；

tool visibility 只表达三类可见性：

- direct：常用工具，tool 层默认暴露给模型；
- searchable：不常用工具，默认不暴露 schema，只能通过 tool search 或显式选择后加载；
- hidden：不可见工具，tool 层不加载、不搜索、不暴露。

system prompt 是 agent 的基础行为配置，直接保存在 definition 中。definition 只保存 prompt 内容，
不负责把它转换成 provider request；转换成 OpenAI Responses 的 `instructions` 或其他 provider
等价字段，属于 context/provider request 层。

definition 冻结后只读。运行时如果需要临时变化，只能在 run config snapshot 中表达，不能回写 definition。

这些内容第一版不要放进 definition：默认模型、权限规则、graph template、hook handler 列表、budget、
context 裁剪策略。需要这些能力时先放在对应模块的默认配置或调用参数里，
不要把 `src/agent/definition.rs` 做成总配置仓库。

### `src/agent/factory.rs`

从用户配置或代码配置构建 `AgentDefinition`，并从 definition 创建 `Agent`。

factory 只负责装配，不负责具体业务逻辑：

- 可以做结构校验，例如 agent name 非空、system prompt 非空、tool visibility 枚举合法；
- 可以把 tool registry、context builder、provider registry 等服务引用传给 `Agent`；
- 不直接过滤 tool、不加载具体 tool schema；
- 不把 system prompt 转 provider instructions、不拼 context；
- 不决定 provider request 或 graph 行为。

tool 可见性由 tool 层根据 definition 的 visibility policy 解析。system prompt 由 context/provider
request 层读取并转换。

factory 也不要成为“第二个 core”。如果某段逻辑需要理解 tool schema，就放 tool 层；需要理解
system prompt 到 provider instructions 的转换，就放 context/provider request 层；需要理解 session
replay，就放 session 层。

## 并发模型

`AgentDefinition` 和 registry 是共享只读依赖。`Agent` 是每个 agent 生命周期独占的 mutable
对象。同一个 definition 可以创建多个 `Agent` 并发运行，agent 之间不能共享 pending calls、
streaming message、approval state 或 cancellation token。

## 后续拓展方案

后续可以在 factory 增加 preset、profile、organization policy、plugin bundle，但也只能做配置组合
和依赖装配，不能把 tool 解析、message 加载、context 拼装逻辑塞进 factory。

后续如果要做 TUI 状态树、远程 event subscriber、session metrics，应放在 `Agent` 外层的
session/driver/UI 层。不要重新拆出 `agent_runtime.rs`，除非 `src/agent/agent.rs` 已经大到无法维护。

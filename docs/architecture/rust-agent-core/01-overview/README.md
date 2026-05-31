# Rust Agent Core 总览

日期：2026-05-30

## 范围

这组文档描述一个受 PIagent、Eino TurnLoop/Graph、PocketFlow 和本地 Claude Code 快照启发的
Rust agent core。当前设计重点是把四个容易混在一起的层级拆开：

- 用户输入入口：把原始输入包装成 user message；
- 单轮 graph run：agent 在一次 graph 执行中的 message/content block；
- TurnLoop/session：会话级输入调度、持久化、branch、resume、replay；
- tool core：tool 抽象和生命周期，不包含具体工具实现。

第一版面向本地 CLI coding agent core。HTTP 服务、复杂 TUI、具体 coding tools、完整插件市场和后台任务
都先放在 core 第一版之外。

## 使用指南

阅读顺序：

1. 先读本文，确认整体分层。
2. 用户输入到 message 读 [02-user-message](../02-user-message/README.md)。
3. 配置、factory、agent 对象读 [03-agent](../03-agent/README.md)。
4. graph、多 agent 读 [04-graph](../04-graph/README.md)。
5. RunMessage 与 ContentBlock 读 [05-message](../05-message/README.md)。
6. TurnLoop、session、持久化、replay 读 [06-session](../06-session/README.md)。
7. hook、handler、extension 读 [07-hooks](../07-hooks/README.md)。
8. LLM provider 和 OpenAI Responses 读 [08-llm](../08-llm/README.md)。
9. tool core 读 [09-tool](../09-tool/README.md)。
10. 落地顺序读 [10-roadmap](../10-roadmap/README.md)。
11. 外部调用 core 的 API 契约读 [11-public-api](../11-public-api/README.md)。
12. Droid Loom backend 分支和 Android shell 集成读 [13-droid-loom-backend](../13-droid-loom-backend/README.md)。
13. core 内部 async runtime 优化读 [19-async-agent-core-optimization](../19-async-agent-core-optimization/README.md)。
14. graph runtime 重新实现读 [20-graph-runtime-refactor](../20-graph-runtime-refactor/README.md)。

这组文档是架构设计，不提供具体代码。

## 使用方案

这组文档对应的代码作为 `agent-core` 被 CLI、测试、后续 TUI/HTTP 层调用。

调用方：

- `agent-cli`：把用户输入交给 core，订阅输出事件，展示结果。
- 测试用例：直接构造 user message、agent definition、mock provider、mock tool，跑最小闭环。
- 后续产品层：通过 TurnLoop/session API 驱动一轮或多轮 agent run。

输入：

- 用户输入入口产出的 `RunMessage(role=user)`；
- 已冻结的 `AgentDefinition`；
- provider、tool、hook handler、session store 等服务依赖；
- 可选 run config，例如模型、预算、取消 token。

输出：

- graph run 产生的 finalized messages；
- session entry；
- runtime events；
- agent result 或错误结果。

典型调用流程：

1. 外层应用读取配置，创建 `AgentDefinition`。
2. `AgentFactory` 用 definition 和服务依赖创建 `Agent`。
3. 用户输入通过 `src/input/user_input.rs` 包装成 user message。
4. TurnLoop 接收 user message，准备一轮 run。
5. `Agent.run()` 选择 graph template 并启动 graph runner。
6. graph runner 驱动 provider、tool、handler、message builder。
7. run finalize 后，session 模块把 finalized messages 写成 session entries。
8. 下一轮请求从 session replay snapshot 和本轮 graph messages 构建 provider context。

不能这样用：

- 不要让 CLI 直接操作 graph node 或 edge。
- 不要让 `AgentDefinition` 承担 provider request、tool schema、session replay 等运行期逻辑。
- 不要把 session persistence 写进 message builder、provider adapter 或 tool executor。

## 核心分层

核心库建议先拆成两个 crate：

- `crates/agent-core`：user message、agent、TurnLoop、graph、run message、session、hook、LLM、tool core、context、error。
- `crates/agent-cli`：命令行参数、交互输入输出、配置读取和 smoke test。

`agent-core` 内部按职责拆成这些路径组：

- User input：`src/input/user_input.rs`，只负责原始用户输入到 user message 的转换。
- Definition/agent：`src/agent/agent.rs`、`src/agent/definition.rs`、`src/agent/factory.rs`。
- Graph：`src/graph/graph.rs`、`src/graph/node.rs`、`src/graph/edge.rs`、`src/graph/runner.rs`、`src/graph/state.rs`、
  `src/graph/templates.rs`。
- Run message/content block：`src/message/run_message.rs`、`src/message/content_block.rs`、`src/message/assistant_builder.rs`。
- TurnLoop/session/replay：`src/session/turn_loop.rs`、`src/session/entry.rs`、`src/session/tree.rs`、
  `src/session/store.rs`、`src/session/replay.rs`、`src/session/compaction.rs`。
- Hook/handler：`src/hook/hook.rs`、`src/hook/handler.rs`。
- LLM provider：`src/llm/context.rs`、`src/llm/llm.rs`、`src/llm/model.rs`、`src/llm/registry.rs`、
  `src/llm/provider.rs`、`src/llm/request.rs`、`src/llm/stream.rs`、`src/llm/openai_responses.rs`。
- Tool core：`src/tool/tool.rs`、`src/tool/registry.rs`、`src/tool/schema.rs`、`src/tool/permissions.rs`、
  `src/tool/executor.rs`、`src/tool/result.rs`、`src/tool/adapter.rs`。
- Public API facade：runtime、agent definition、session、run、event、handler、tool/provider registration。

## 关键设计原则

用户输入进入 core 后直接变成 `role=user` 的 `RunMessage`。`RunMessage` 和 `SessionEntry`
仍然必须分开：前者是单轮 graph run 的工作数据，后者是持久化历史。

配置和 agent 内部生命周期必须分离。第一版 `AgentDefinition` 只声明 agent name、system prompt 和
tool visibility。`Agent` 只保存单个 agent 生命周期内部状态，例如 phase、streaming message、
pending tool calls、approval state、usage、error 和 cancellation token。

definition 只声明，不执行。tool 可见性由 tool 层解析，system prompt 由 context/provider request
层转换成 provider instructions，factory 只负责装配依赖和创建 `Agent`。

TurnLoop、session tree、active leaf、replay、compaction、UI subscriber 都不属于 `src/agent/agent.rs`。

agent run graph 化。默认 ReAct 不是硬编码 while 循环，而是 graph template。`Agent.run()` 直接选择
graph template 并调用 graph runner。node 负责执行，edge 负责
依赖关系、上下文继承和目标 node 启动条件。模型调用、tool execution、人工确认、多 agent 子图和
错误处理都通过 node/edge 表达。

hook 是可决策生命周期点，handler 是被 hook 调用的处理函数。handler 分 point 和 wrapper 两种；
event 是观察流。PI 风格 extension handler 可适配成 point handler 或 wrapper handler，不能直接拿
runtime mutable state。

session tree 是历史权威。event stream 可以很细，trace log 可以很重，但 session 默认只保存 finalized
semantic entries。provider context 从 active leaf replay snapshot 和本轮 graph messages 构建。

第一版 tool 只做 core 抽象，不做具体工具实现。具体 coding tools 后续作为 tool pack 接入。

外部应用只依赖 public API。graph runtime state 和 session JSONL 不是第一版稳定可变写入 API。

## 参考来源

- PI SDK 和 agent loop：`https://pi.dev/docs/latest/sdk`
- PI session 和 JSONL tree：`https://pi.dev/docs/latest/sessions`
- PI extension 事件总线：`https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md`
- OpenAI Responses：`https://platform.openai.com/docs/api-reference/responses`
- Eino TurnLoop/Graph：`https://www.cloudwego.io/docs/eino/`
- PocketFlow Node/Flow：`https://the-pocket.github.io/PocketFlow/core_abstraction/node.html`
- 本地 Claude Code 快照：`C:\Users\Administrator\Desktop\claude-code-main`

## 后续拓展方案

第一阶段做 core 边界：user message conversion、run message/content block、session tree、OpenAI Responses、
context conversion、串行 graph runner、tool core 抽象和 CLI smoke test。

第二阶段做具体 tool pack：文件、搜索、编辑、shell、MCP 等都作为 core 外部模块接入。

第三阶段做多 agent：child_agent node、delegation、foreground spawn、fork context、深度限制、并发限制。

第四阶段做产品层：TUI/HTTP server、插件市场、远程 artifact store、后台任务、trace viewer。

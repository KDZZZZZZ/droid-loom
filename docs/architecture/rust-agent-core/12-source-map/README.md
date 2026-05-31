# 源码文件职责地图

日期：2026-05-30

## 范围

本文逐个说明 `crates/agent-core/src` 下每个文件的职责、内部逻辑、与其他文件的交互，以及未来谁应该使用它。本文用于代码评审和后续实现，不替代 API 契约文档。

目录组织规则：

- `src/core/` 对应错误、事件等跨模块基础类型。
- `src/input/` 对应 [02-user-message](../02-user-message/README.md)。
- `src/agent/` 对应 [03-agent](../03-agent/README.md)。
- `src/graph/` 对应 [04-graph](../04-graph/README.md)。
- `src/message/` 对应 [05-message](../05-message/README.md)。
- `src/session/` 对应 [06-session](../06-session/README.md)。
- `src/hook/` 对应 [07-hooks](../07-hooks/README.md)。
- `src/llm/` 对应 [08-llm](../08-llm/README.md)。
- `src/tool/` 对应 [09-tool](../09-tool/README.md)。

后续新增文件必须放进最接近的目录，并同步更新对应子文档的“使用方案”和“后续拓展方案”。如果某个变更需要跨目录移动职责，必须先改文档说明边界，再改代码。

## 使用方案

开发者改代码前先用本文定位文件，而不是靠搜索随手新增模块。

实际使用方式：

1. 先从需求判断所属边界：input、agent、message、graph、session、hook、llm、tool、core。
2. 在对应 `src/<domain>/` 目录内修改或新增文件。
3. 保持 `src/lib.rs` 的 public module 名称稳定；目录重组不应迫使外部调用方改 import。
4. 修改 public API、职责边界、目录路径或扩展点时，同步更新对应子文档。
5. 修改完成后跑 `cargo fmt --check`、`cargo check`、`cargo test`。

不能这样用：

- 不要为了方便把跨层逻辑放回 `src/lib.rs`。
- 不要新建和现有子文档不对应的顶层目录。
- 不要让某个文件同时承担两个子文档的职责。
- 不要只移动代码路径而不修正文档路径和使用方案。

## Facade 与错误

### `src/lib.rs`

职责：crate public facade，声明模块并 re-export 第一版稳定入口。

内部逻辑：只做模块导出，不放业务逻辑。

交互：被所有外部调用方依赖；内部模块之间仍优先使用 `crate::module::Type`，不要把 `src/lib.rs` 当依赖中心。

使用者：CLI、测试、后续 TUI/HTTP server、tool pack。

### `examples/public_api_smoke.rs`

职责：外部 API 契约的可运行 smoke example。

内部逻辑：定义一个最小 agent、注册 direct/searchable/hidden tool、执行一次 tool call、运行 point/wrapper handler、包装用户输入、构建 graph、读取 run events，并演示 stop requested。

交互：只通过 public API 接入 `agent-core`；如果该示例需要访问 private state，说明 API 契约缺口必须先补文档再改 core。

使用者：外部接入方、API 文档维护者、回归测试前的人工 smoke。

### `examples/deepseek_prepare_request.rs`

职责：DeepSeek provider 接入的可运行 request snapshot 示例。

内部逻辑：构造 agent definition、user message、visible tool schema 和 DeepSeek-specific metadata；从 `DEEPSEEK_API_KEY` 环境变量读取 key 并注入 provider；打印 provider、endpoint、model、message 数、tool 数和是否存在 auth header；如果存在 auth header，会发送一次非流式请求并打印归一化事件数量和首个文本 delta，不打印密钥。

交互：通过 `ContextBuilder`、`DeepSeekChatProvider` 和 public provider trait 准备请求；HTTP 发送只存在于 example，不进入 core adapter，不写 session。

使用者：外部 runtime setup、provider adapter 维护者、API 文档维护者。

### `example/mobilerun-agent-boundary/Cargo.toml`

职责：定义 Mobilerun-like core 边界示例 crate。

内部逻辑：只依赖 workspace `agent-core` 和 `serde_json`，用于证明示例没有绕过 core public API。

交互：作为根 workspace member 参与 `cargo check` 和 `cargo run -p mobilerun-agent-boundary`。

使用者：core API 维护者、移动端 adapter 作者、回归验证脚本。

### `example/mobilerun-agent-boundary/src/main.rs`

职责：示例入口。

内部逻辑：调用 `scripted_agent::run_boundary_probe()` 并打印能力矩阵。

交互：不放业务逻辑，不直接创建 tool、graph 或 provider request。

使用者：本地验证、CI smoke。

### `example/mobilerun-agent-boundary/CAPABILITY_MAP.md`

职责：Mobilerun agent 能力到当前 core API 的验收映射表。

内部逻辑：逐项列出 FastAgent、Manager/Executor、custom variables、credentials、custom tools、event stream、send user message、structured output、timeout、driver/state provider 等能力的 current mapping、示例证据、状态和后续拓展点。

交互：约束 `example/mobilerun-agent-boundary` 的扩展范围；新增 Mobilerun-like 能力前必须先更新本表，避免实现偏离。

使用者：core API 维护者、移动端 runtime 作者、reviewer、后续回归测试作者。

### `example/mobilerun-agent-boundary/src/prompt.rs`

职责：定义 Mobilerun-like prompt 配置。

内部逻辑：保存 fast/reasoning system prompt、user prompt、goal、custom variables、max steps、step timeout、vision flag 和 tool visibility；输出 `AgentDefinition`。

交互：只调用 `AgentDefinitionBuilder`，不注册 tool、不执行 provider、不写 session。

使用者：真实 runtime 的配置层、prompt preset 作者、测试。

### `example/mobilerun-agent-boundary/src/mobile_tools.rs`

职责：定义 Mobilerun-like mock tool pack。

内部逻辑：把 FastAgent 动作 `click`、`click_at`、`click_area`、`long_press`、`long_press_at`、`type`、`type_secret`、`swipe`、`system_button`、`wait`、`open_app`、`remember`、`complete` 映射成 `ToolSchema`；补充 `screenshot`、`ui_state`、`search_database` 和 hidden `raw_adb_shell`；tool invoke 只返回 mock observation。

交互：通过 `ToolRegistry` 注册，通过 `ToolExecutor` 执行；主路径使用 `execute_*_message(s)` 直接回到 message 层，`ToolResult` 只保留给审计和底层测试。

使用者：tool pack 作者、平台 adapter 作者、core tool API 回归测试。

### `example/mobilerun-agent-boundary/src/scripted_agent.rs`

职责：运行 Mobilerun-like 边界探针。

内部逻辑：构建 fast/reasoning `AgentDefinition`，包装用户 text/image input，构造 provider-neutral `LlmRequest`，用 `AssistantBuilder` 构造 tool-call turn，执行所有非 hidden tools，验证 hidden guard，把 user/assistant/tool result messages 写入 `InMemorySessionStore` 并通过 replay snapshot 生成轨迹样本，运行 point/wrapper handler，验证 recoverable error middleware，运行 `TurnLoop` 输入队列，运行 fast graph、manager/executor graph 和复杂 task graph，把 `CoreEvent` 投影成 Mobilerun-style event stream，验证取消、预算保护、并行 tool batch、轨迹概率图、预执行、冷热分层、task-bound context、cache-aware context ordering、key/account routing、app map memory、百步跨 App 任务和 token 优化效果，并输出能力缺口。

交互：只使用 `agent-core` public API；不发 HTTP，不调用 Android SDK；只写内存 session store 来验证 replay，不把 session 作为隐藏全局状态。

使用者：core API 维护者、移动端 runtime 作者、回归验证脚本。

### `example/mobilerun-agent-boundary/src/app_map_memory.rs`

职责：维护 GUI-Explorer 风格 App 地图记忆。

内部逻辑：`AppMapMemory` 从 `ui_state` JSON 抽取功能性 UI 元素，形成 `PageNode`；用 `PageTransition` 记录页面间动作、成功/失败次数和 stale 状态；`local_view()` 只返回当前位置附近几跳和任务相关入口；`semantic_search()` 支持按“订单详情”“账号安全”等目标搜页面；`forget()` 支持按 page/query/stale 删除地图；`build_candidate_actions()` 为当前页面预构造 click/type/back 等候选工具调用。

交互：输入是平台 adapter 的 accessibility tree 或 mock `ui_state` 输出；输出是 runtime 可放入 prompt 的局部地图视野和候选 `ToolCall`；不进入 `agent-core`，不持久化真实账号或截图。

使用者：移动端 agent shell、路径规划器、token optimizer、回归测试。

### `example/mobilerun-agent-boundary/src/cross_app_task.rs`

职责：验证地图记忆驱动的百步级跨 App 连续任务。

内部逻辑：构造 Shop -> Notes -> Calendar -> Mail 的模拟 App 地图，使用语义搜索和已知路径自动执行 100+ 个工具动作；每步通过 `ToolExecutor` 调用真实注册的 mock tool；记录 app switch、map reuse、每步局部地图 token 和整图 token，输出 token savings。

交互：依赖 `AppMapMemory`、`ToolExecutor` 和 `AgentDefinition`；不调用 provider；不使用隐藏工具；用于证明长任务 orchestration 可以在 core public API 外闭环。

使用者：长任务 runtime、token 成本评估、回归测试。

### `example/mobilerun-agent-boundary/src/execution_probability.rs`

职责：从 session replay 后的 message 轨迹统计执行概率图。

内部逻辑：读取 `ReplaySnapshot.messages` 或 finalized `Vec<RunMessage>`，抽取 `message:*`、`tool_call:*`、`tool_result:*:{ok|error}` 事件，统计 transition count、tool session probability 和 likely-next prediction；根据概率和 `ToolMetadata::can_preexecute()` 生成只读幂等工具预执行计划；根据工具使用概率生成 hot direct tools、cold searchable tools 和 hidden tools 分层。

交互：读取 message/session replay、查询 `ToolRegistry`、调用 `ToolExecutor::execute_batch_parallel_messages()`，把预执行结果直接交回 message 层；不直接调用 provider，不持久化 session，不实现具体工具。

使用者：runtime optimizer、移动端 agent shell、回归测试。

### `example/mobilerun-agent-boundary/src/context_stability.rs`

职责：为 provider context 排列稳定块，最大化 prompt cache 命中。

内部逻辑：用 `RunMessage.metadata["context.stability"]` 标记 `stable_prefix`、`task_package`、`dependency_result`、`volatile_observation`，按稳定性和原始顺序稳定排序；用 system prompt 和 hot direct tool names 生成 `stable_prefix_id`。

交互：输出仍是 `RunMessage` 列表，交给 `ContextBuilder` 构建 `LlmRequest`；`stable_prefix_id` 交给 key routing，不把 cache 策略塞进 message 或 provider adapter。

使用者：context assembler、provider runtime、cache/key routing 策略。

### `example/mobilerun-agent-boundary/src/task_context.rs`

职责：定义 Mobilerun subgoal 风格的 task-bound context DAG。

内部逻辑：`TaskContextPackage` 保存 task id、goal、inputs、artifacts 和 required state；`TaskDependencyGraph` 校验 task id、重复 id、依赖存在和循环；`context_for()` 只继承依赖任务的 artifacts，再追加当前 task package，不继承 tool call、失败尝试或临时屏幕状态。

交互：产出带稳定性标记的 `RunMessage`，交给 `context_stability` 排序和 `ContextBuilder` 使用；不执行 graph、不调 provider、不写 session。

使用者：manager/executor runtime、subgoal planner、移动端任务拆分器、测试。

### `example/mobilerun-agent-boundary/src/key_routing.rs`

职责：根据 stable prefix 把 provider request 路由到不同 key/account 槽位。

内部逻辑：`KeyRoutePlan` 根据 `stable_prefix_id` 选择稳定账号或通用账号；`SecretResolver` 抽象只按环境变量名取密钥；`apply()` 注入 Authorization header，并把 account/env/prefix 写入 request metadata 方便观测。

交互：输入是已经由 `ContextBuilder` 构建好的 `LlmRequest`；输出仍是 `LlmRequest`，供 provider adapter 发送；真实 API key 只存在运行时环境，不进入源码、文档、session 或测试输出。

使用者：provider runtime、账号隔离策略、prompt cache 策略、测试。

### `src/core/error.rs`

职责：统一 core 错误类型和 result alias。

内部逻辑：区分 invalid input、invalid config、not found、permission denied、recoverable、fatal、serialization。

交互：所有 public API 返回 `AgentCoreResult<T>`；provider/tool/hook/graph/session 都不能直接返回裸字符串错误。

使用者：所有模块和外部集成层。

### `src/core/event.rs`

职责：定义 core 观察事件和本地 event log。

内部逻辑：`CoreEvent` 表达 agent、graph、node、message、hook、error 等运行事件；`EventLog` 是轻量 Vec 包装。

交互：常规路径由 `TurnLoop` 运行 graph 并产出事件；低层 `Agent::run` 和 `GraphRunner` 仍可在高级适配器/测试中产出事件。UI/trace/test 读取事件但不改变行为。

使用者：CLI renderer、TUI/HTTP streaming、trace recorder、测试断言。

## Agent 定义与运行

### `src/agent/definition.rs`

职责：保存不可变 agent 配置。

内部逻辑：只包含 name、system prompt、tool visibility；builder 在 `build()` 时校验必填项。

交互：tool 层读取 visibility；context 层读取 system prompt；factory 读取并创建 `Agent`。

使用者：配置加载器、CLI preset、多 agent 编排器、测试构造器。

### `src/agent/factory.rs`

职责：把 definition 和 core 服务依赖装配成 `Agent`。

内部逻辑：校验 definition，并把 `AgentServices` 注入 `Agent`；不解析 tool，不构造 context，不写 session。

交互：向 `src/agent/agent.rs` 传入 runner 等高级服务依赖；常规外部执行由 `TurnLoop` 持有 runner 能力。

使用者：应用启动层、测试 harness、未来 runtime facade。

### `src/agent/agent.rs`

职责：单个 agent 生命周期的 public facade。

内部逻辑：持有 agent id、definition、services、cancellation token；`run()` 仍是高级/兼容路径，接收 graph 和初始消息，调用 runner，返回 result/events。

交互：不管理 TurnLoop、session tree、message loading；child-agent executor 可以通过 `Agent` 调用一个已定义 agent。

使用者：CLI/TUI/HTTP run endpoint、graph child-agent executor、测试。

## 用户输入、消息与内容块

### `src/input/user_input.rs`

职责：把原始用户输入按模态包装成 `RunMessage(role=user)`。

内部逻辑：text/file/image/audio 分别落成对应 `ContentBlock`；只做包装和基本校验。

交互：输出交给 TurnLoop、session 或 `AgentRunInput`；不排队、不写 session、不构造 provider request。

使用者：CLI input loop、TUI composer、HTTP message endpoint、测试。

### `src/message/run_message.rs`

职责：定义 graph run 内部流动的消息。

内部逻辑：保存 id、role、content、status、source node、provider response id、usage、metadata、created time；支持 streaming/finalized/aborted。

交互：graph runner 写 source node；session 只提交 finalized message；context builder 把它转换成 provider input。

使用者：graph node、assistant builder、tool result mapper、session、context、测试。

### `src/message/content_block.rs`

职责：定义 typed content block。

内部逻辑：支持 text、reasoning、tool call、tool result、file/image/audio reference、diagnostic、custom；text/reasoning 支持 delta append。

交互：user input、assistant builder、tool result、context builder 都只通过 typed block 交换内容。

使用者：所有 message producer 和 provider context converter。

### `src/message/assistant_builder.rs`

职责：把 provider stream 片段包装成 assistant message。

内部逻辑：合并连续 text/reasoning delta，追加 tool call/diagnostic block，finish 时校验非空并 finalize。

交互：provider adapter 或 graph node 使用它构造 assistant message；不执行 tool、不判断 edge、不写 session。

使用者：OpenAI Responses adapter、provider node、测试。

## Graph 编排

### `src/graph/graph.rs`

职责：定义 compiled graph template 和 builder。

内部逻辑：保存 nodes、edges、start/end nodes、budget；build 时校验 graph name、start/end、edge source/target。

交互：`TurnLoop::run_message` 接收 `Graph` 并推进一轮 turn；低层 runner 读取 graph 推进单次 run。

使用者：默认 graph template、多 agent 编排器、测试。

### `src/graph/node.rs`

职责：定义 node 契约和 runtime node spec。

内部逻辑：node kind 包括 transform、tool、agent、graph 和 final；node 执行由 `NodeExecutor` 完成，返回 `NodeResult` messages。

交互：`TurnLoop` 经由内部 runner 调用 async runtime；runtime 通过 `NodeExecutor` 执行 node 并消费 `NodeResult`。`NodeKind::Agent` 是 child agent executor 的声明，不由 edge 执行。

使用者：graph builder、graph runner、未来外部 node executor。

### `src/graph/edge.rs`

职责：定义 edge 内容传输逻辑。

内部逻辑：edge 从 source node message log 读取未传输的新 messages，按目标 input package 的 `MessageQuery` 过滤和投影，再写入目标 package。

交互：由 runtime 在扫描 edge 时调用；不执行 provider/tool/session，不管理循环数。

使用者：graph builder、graph runner、测试。

### `src/graph/state.rs`

职责：保存单次 graph run 的工作状态。

内部逻辑：记录 per-node message stream、全局 message log、message version、fired edges、node execution counts、budget、stop flag。

交互：runner 写入；edge 只拿只读 view；外部不要直接可变操作。

使用者：graph runner、debug/checkpoint、测试。

### `src/graph/runner.rs`

职责：执行 compiled graph。

内部逻辑：维护 runnable set；调用 node；message 产生后写 state、发 event、检查出边；管理 stop、budget、completed/failed status。

交互：被 `Agent::run` 调用；使用 graph/node/edge/state/event；不写 session，不直接读用户原始输入。

使用者：Agent、测试、未来 runtime facade。

### `src/graph/templates.rs`

职责：保存默认 graph template。

内部逻辑：当前提供 default ReAct skeleton 和 single-node graph helper；start node 使用 passthrough input。

交互：供 `AgentRunInput` 或应用层选择；后续可扩展更多 template。

使用者：CLI smoke、测试、默认 runtime。

## Session 与 TurnLoop

### `src/session/turn_loop.rs`

职责：会话级 push buffer、消息历史、GenInput、graph context 构建和 turn 状态机。

内部逻辑：`push` 接收 finalized message，`GenInput` 决定本轮 graph input/consumed/remaining，`PrepareGraph` 选择 graph，运行 graph 后通过 `OnTurnEvents` 消费事件并追加 graph 新消息。

交互：常规入口是 `append_messages`、`push`、`run_once`、`run_pending` 和 `run_message`；不直接写 session，不构造 provider request。

使用者：CLI/TUI/HTTP session driver、测试。

### `src/session/entry.rs`

职责：定义 append-only session tree entry。

内部逻辑：支持 header、message、compaction；message entry 只允许 finalized `RunMessage`。

交互：`SessionTree` 存储 entry；`src/session/replay.rs` 从 entry 构建 replay snapshot。

使用者：session store、driver、测试。

### `src/session/tree.rs`

职责：维护 append-only session tree 和 active leaf。

内部逻辑：按 parent_id 追加 entry，校验 parent 存在，支持 branch replay。

交互：store 持久化 tree；replay 读取 branch；不理解 provider 或 graph execution。

使用者：session store、session driver、测试。

### `src/session/store.rs`

职责：定义 session persistence 抽象和内存实现。

内部逻辑：`SessionStore` 提供 append/load；`InMemorySessionStore` 用 Vec 保存 entries 并重建 tree。

交互：外部持久化实现可以替换内存 store；不手写 JSONL 到其它模块。

使用者：CLI smoke、测试、未来 file/db session store。

### `src/session/replay.rs`

职责：从 session tree 构造 active branch replay snapshot。

内部逻辑：收集 summaries，并按最后一次 compaction 的边界选择需要保留的 messages。

交互：context builder 使用 replay 后得到的 `RunMessage` 列表；session compaction 写入 compaction entry。

使用者：TurnLoop driver、context builder、测试。

### `src/session/compaction.rs`

职责：定义 compaction plan。

内部逻辑：校验 summary 非空，生成 `SessionEntryKind::Compaction`。

交互：replay 根据 compaction 决定 provider context 的 summary 和 message 边界。

使用者：session maintenance、future summarizer、测试。

## Hook 与 Handler

### `src/hook/hook.rs`

职责：定义 hook 名称、payload、point decision、wrapper request/response/result、hook event request。

内部逻辑：区分 point hook 和 wrapper hook；数据统一用 typed wrapper 加 `serde_json::Value` payload。

交互：`src/hook/handler.rs` 负责注册和执行；runtime 节点后续在生命周期点调用它。

使用者：extension、permission/recovery/audit handler、测试。

### `src/hook/handler.rs`

职责：注册、排序、执行 handler。

内部逻辑：point handler 顺序执行并可 rewrite/block/stop/emit；wrapper layer 用 tower 风格包裹 terminal service。

交互：hook 定义来自 `src/hook/hook.rs`；provider/tool/node 执行点后续可通过 wrapper handler 包住。

使用者：extension host、runtime facade、测试。

## LLM Provider 与 Context

### `src/llm/context.rs`

职责：把 agent definition、messages、visible tools 转成 provider-neutral `LlmRequest`。

内部逻辑：system prompt 转 `instructions`；`RunMessage` 原样进入 `LlmRequest.messages`；diagnostic 是否进入上下文由参数控制。typed `ContentBlock` 只在 provider adapter 最后一跳序列化成具体 API 字段。

交互：读取 `AgentDefinition`、`RunMessage`、`ToolSchema`；输出给 `LlmRegistry/LlmProvider`。

使用者：provider request node、测试、future context policy。

### `src/llm/model.rs`

职责：定义 model id、model metadata 和 API kind。

内部逻辑：`ModelId` 是稳定 key；`LlmApi` 描述 provider API 类型，第一版包含 OpenAI Responses 和 DeepSeek Chat。

交互：registry 根据 model api 找 provider。

使用者：provider registration、runtime config、测试。

### `src/llm/request.rs`

职责：定义 provider-neutral LLM request。

内部逻辑：包含 model、api override、instructions、messages、tools、options、metadata、headers。

交互：context builder 产出 request；provider adapter 读取 request 并转具体 API。

使用者：context、provider、测试。

### `src/llm/stream.rs`

职责：定义 provider-neutral LLM stream event。

内部逻辑：用 iterator 返回 prepared request、text/reasoning delta、tool call、usage、completed、error。

交互：provider 返回 stream；assistant builder 消费 stream 构造 message。

使用者：provider adapter、graph provider node、测试。

### `src/llm/provider.rs`

职责：定义 provider trait 和 tower 风格 provider service/layer。

内部逻辑：provider 负责 prepare request 和 stream；默认 stream 产出 prepared request，便于 dry-run。

交互：`LlmRegistry` 管理 provider；OpenAI adapter 实现 trait。

使用者：provider adapter、middleware/wrapper、测试。

### `src/llm/registry.rs`

职责：统一管理 provider 和 model。

内部逻辑：按 API key 注册 provider，按 model id 注册 model，并根据 request 选择 provider。

交互：provider node 通过 registry 查找 adapter。

使用者：runtime setup、测试。

### `src/llm/llm.rs`

职责：提供 `LlmService` 统一调用入口。

内部逻辑：持有 registry，收到 `LlmRequest` 后解析 provider 并调用。

交互：上层 graph/provider node 不直接遍历 registry。

使用者：provider request node、runtime facade。

### `src/llm/openai_responses.rs`

职责：OpenAI Responses API adapter。

内部逻辑：把 `LlmRequest` 转 Responses request body，把 Responses SSE/value event 转 `LlmStreamEvent`。

交互：优先支持 OpenAI Responses；不把 OpenAI 专用结构泄漏给 tool API。

使用者：runtime setup、provider tests、后续真实 HTTP client。

### `src/llm/deepseek_chat.rs`

职责：DeepSeek Chat Completions API adapter。

内部逻辑：把 provider-neutral `LlmRequest` 转成 DeepSeek `/chat/completions` body；把 system prompt 转 system message；把 tools 转 Chat Completions function tool；把 assistant reasoning 映射到 `reasoning_content`；把 stream chunk 和非流式 response 映射成 `LlmStreamEvent`。

交互：`LlmRegistry` 按 `deepseek_chat` API key 选择它；context builder 不需要知道 DeepSeek 格式；API key 由外部环境变量或 header 注入，不保存在 agent definition。

使用者：runtime setup、provider tests、DeepSeek request 示例、后续真实 HTTP client。

## Tool Core

### `src/tool/schema.rs`

职责：定义 tool schema 和参数校验。

内部逻辑：保存 name、description、input/output schema、annotations；校验 metadata 和 required arguments。

交互：tool metadata 持有 schema；context builder 暴露 visible schema 给 provider。

使用者：tool pack、tool registry、tests。

### `src/tool/tool.rs`

职责：定义 tool trait、metadata、capability、invocation 和 output。

内部逻辑：tool 实现只需要提供 metadata 和 invoke；不决定 agent visibility。

交互：registry 持有 `Arc<dyn Tool>`；executor 调用 tool。

使用者：tool pack、MCP/plugin adapter、测试 mock。

### `src/tool/registry.rs`

职责：注册 tool，并根据 agent definition 解析 visibility。

内部逻辑：支持 direct schemas、searchable tools、hidden permission guard、简单搜索。

交互：context builder 读取 direct schemas；tool executor 从 registry 获取可执行 tool。

使用者：runtime setup、tool search node、tool executor、测试。

### `src/tool/permissions.rs`

职责：定义 tool permission policy。

内部逻辑：policy 返回 allow、passthrough、ask、deny；allow 可改写 arguments。

交互：executor 在真正调用 tool 前执行 permission policy。

使用者：permission extension、runtime setup、测试。

### `src/tool/executor.rs`

职责：执行 tool call。

内部逻辑：校验 visibility 和 arguments，执行 permission policy，调用 tool；主执行入口直接返回 tool `RunMessage`，原始 `ToolResult` 只用于审计、状态判断和底层测试。

交互：由 graph tool-execution node 调用；hook wrapper 后续可包住 execution。

使用者：tool node、runtime facade、测试。

### `src/tool/result.rs`

职责：定义 tool execution result 和到 core message 的映射。

内部逻辑：success/denied/failed 统一携带 output/error/recoverable；tool executor 的 message 入口会自动生成 typed `ContentBlock::ToolResult`。

交互：context builder 将 tool result message 转 provider function call output。

使用者：tool executor、graph runner、session、测试。

### `src/tool/adapter.rs`

职责：外部 tool adapter 注册点。

内部逻辑：adapter 提供 metadata 和 tools；registry 可以统一挂载外部 tool pack。

交互：不直接执行 tool，只把 tool 暴露给 `ToolRegistry`。

使用者：MCP adapter、plugin adapter、future tool marketplace。

## Droid Loom Android Shell

### `Cargo.toml`

职责：仓库根 workspace 配置。
内部逻辑：把 `agent-smoke` 与 `crates/agent-core` 放进同一个 workspace，并统一 `serde`、`serde_json`、`thiserror`、`uuid` 等依赖。
交互：`agent-smoke/Cargo.toml` 通过 workspace dependency 引用 `agent-core`。
使用者：Cargo、本地开发、CI、Android native lib 构建脚本。

### `agent-smoke/Cargo.toml`

职责：定义 Android-facing UniFFI crate。
内部逻辑：导出 `cdylib` 给 Android `.so`，导出 `rlib` 给桌面 smoke；依赖 shared `agent-core`。
交互：`build-android-libs.ps1` 编译它；UniFFI binding 从它生成 Kotlin API。
使用者：Android shell、desktop smoke、UniFFI 生成器。

### `agent-smoke/src/lib.rs`

职责：Android 平台 shell 和兼容 API。
内部逻辑：保存 shell 状态、注册工具 schema、调用 `ContextBuilder + DeepSeekChatProvider`、用 `AssistantBuilder` 汇总模型响应、通过 `PlatformToolHost` 回调 Kotlin 执行 Android 工具。当前默认 provider endpoint 使用 MiMo OpenAI-compatible Chat Completions，`DeepSeekChatProvider` 作为 Chat Completions request mapper 复用。
交互：调用 `crates/agent-core` 的 message/context/provider/tool schema；被 UniFFI Kotlin binding 调用；不把 Android SDK 类型传入 shared core。
使用者：Android Kotlin app、desktop smoke、后续移动端平台壳。

### `agent-smoke/src/main.rs`

职责：桌面 agent smoke CLI。
内部逻辑：从环境变量读取 provider 配置，优先使用 `MIMO_*`，`DEEPSEEK_*` 仅兼容 fallback，构造 `AgentCore`，发送一次 prompt。
交互：只依赖 `agent-smoke/src/lib.rs` 暴露的 UniFFI shell API，不直接绕进 `agent-core` 内部。
使用者：本地验证、CI smoke。

### `agent-smoke/scripts/build-android-libs.ps1`

职责：构建 Android ABI native libs。
内部逻辑：读取 `ANDROID_HOME`，定位 NDK toolchain，构建 `aarch64-linux-android` 和 `x86_64-linux-android`，复制 `.so` 到 Android `jniLibs`。
交互：必须使用 `--manifest-path Cargo.toml --target-dir target`，避免 workspace root target 影响复制路径。
使用者：本地 Android 构建、CI。

### `agent-smoke/android-shell/app/src/main/kotlin/com/example/agentsmoke/MainActivity.kt`

职责：Android demo UI、工具注册和平台能力 host。
内部逻辑：读取 BuildConfig provider 配置，创建 Rust `AgentCore`，注册工具 schema，执行 `promptWithTools()`，并在 `AndroidPlatformToolHost` 中分发 Android API。
交互：通过 UniFFI 调 Rust；通过 `PlatformToolHost` 被 Rust 回调；不实现 agent loop。
使用者：Android demo、模拟器验证。

### `agent-smoke/android-shell/app/build.gradle.kts`

职责：Android app 构建配置。
内部逻辑：注入 `MIMO_API_BASE`、`MIMO_API_KEY`、`MIMO_MODEL`、`MIMO_PROXY` 到 `BuildConfig`，并把 UniFFI binding 目录加入 Kotlin source set；`DEEPSEEK_*` 仅作为旧环境变量 fallback。
交互：打包 `jniLibs` 下的 Rust `.so` 和生成的 Kotlin binding。
使用者：Gradle、Android Studio、CI。

更多集成用法见 [13-droid-loom-backend](../13-droid-loom-backend/README.md)。

## 使用约束

- 新功能先判断属于 definition、factory、agent、graph、session、context、provider、tool、hook、event 哪一层。
- 不要把 session replay、tool visibility、provider request、graph loop 混进 `AgentDefinition`。
- 不要让 edge 产生副作用。
- 不要让 user input API 排队或写 session。
- 不要让 event subscriber 改变行为。
- 不要在 core 第一版实现具体 coding tools。

## 后续拓展方案

- 如果某个目录持续膨胀，优先在该目录内部拆子模块，例如 `src/graph/runner/`，不要直接新增并列顶层领域。
- 如果 public module 名称需要重命名，先新增兼容 re-export，再在 API 文档中标记迁移路径。
- 如果新增跨领域能力，先补 ADR 或对应子文档的“边界变更”说明，再改源码。
- 如果实现与设计目标发生偏移，必须回到对应子文档修正“使用方案”和“后续拓展方案”，否则不继续扩展。

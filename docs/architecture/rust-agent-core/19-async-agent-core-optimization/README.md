# Agent Core Async Optimization Plan

日期：2026-05-31

## 范围

本文只讨论 `crates/agent-core` 内部应该怎样优化异步执行能力。

不在本文范围内：

- `agent-smoke` 的 UniFFI、Android service、ADB 脚本或 provider key 注入。
- `example/mobilerun-agent-boundary` 的 deterministic probe。
- 真实移动端 adapter、credential vault、UI 悬浮球和 AccessibilityService 细节。

目标不是把所有 public trait 直接改成 `async fn`，而是在不破坏当前同步 API 的前提下，让 core 的真实 agent runtime 能利用异步执行、流式事件、取消、deadline、并发上下文准备和外部 node executor。

## 外部参考

Rig 的设计值得借鉴的点不是简单地把接口标成 async，而是把 async 用在 agent 运行时的关键路径：

- completion model 的 `completion` 和 `stream` 返回 future，streaming response 是一等能力；
- streaming 模块把 text delta、tool-call delta、final usage 等作为流式事件处理；
- agent request 构建阶段可以异步准备动态上下文；
- tool definition 和 tool call 可以异步执行；
- hook 可以观察 completion、tool call、tool result 和 streaming delta。

这些能力对应到本 core，应该落在 provider runtime、event stream、tool runtime、context prepare 和 node executor，而不是盲目重写全部数据模型。

参考入口：

- Rig `CompletionModel`：https://docs.rs/rig-core/latest/rig_core/completion/request/trait.CompletionModel.html
- Rig streaming module：https://docs.rs/rig-core/latest/rig/streaming/index.html
- Rig source repository：https://github.com/0xplaygrounds/rig

## 当前 Core 状态

### 同步执行边界

当前 core 的主要执行接口都是同步的：

- `Agent::run(AgentRunInput) -> AgentCoreResult<AgentRunResult>`
- `GraphRunner::run(&Graph, GraphRunInput) -> AgentCoreResult<GraphRunResult>`
- `GraphNode::execute(...) -> AgentCoreResult<NodeExecutionResult>`
- `Tool::invoke(ToolInvocation) -> AgentCoreResult<ToolOutput>`
- `ToolService::call(ToolExecutionRequest) -> AgentCoreResult<ToolResult>`
- `ProviderService::call(LlmRequest) -> AgentCoreResult<LlmStream>`
- `LlmStream = Box<dyn Iterator<Item = AgentCoreResult<LlmStreamEvent>> + Send>`
- `HandlerRegistry::run_point` / `run_wrapper`

这让当前 API 简单、对象安全，并且适合 `Arc<dyn Tool>`、`Arc<dyn LlmProvider>`、同步测试和 deterministic boundary probe。

### 真实限制

同步边界带来的限制已经影响真实 phone-using agent：

- provider streaming 只是同步 iterator 抽象，不能自然表达 SSE backpressure、token 级取消或实时 UI event。
- `Agent::cancel` 只在 run 开始和 node 间检查，不能打断正在进行的 provider/tool await。
- `ToolExecutionMetadata.timeout_ms` 只是 metadata，`ToolExecutor` 不强制 deadline。
- `NodeKind::Agent` 通过 `NodeExecutor` 执行；未提供能处理该 agent name 的 executor 时会显式失败。
- `HandlerRegistry` 可手动调用，但还没有作为 provider/tool/node 生命周期的内建 runtime pipeline。
- context 构建是同步拼装，不能并发准备 task package、session replay、tool schema、probability prediction、app map view 等上下文块。

## 优化原则

1. **保留同步 public API 兼容性。** 当前 API 文档和测试已经覆盖大量调用方式，不能因为 async 化推翻 `Arc<dyn Tool>` registry。
2. **新增 async runtime 层。** async 能力通过 additive APIs 引入，例如 `AsyncAgentRuntime`、`AsyncProviderService`、`AsyncToolService`、`AsyncNodeExecutor`。
3. **同步能力通过 adapter 进入 async runtime。** 现有 `Tool`、`LlmProvider` 和 `GraphRunner` 可以被包装成 async service。
4. **phone action 默认串行。** read-only、idempotent、provider/context 准备可以并发；真实设备 mutating action 必须经过 device/action lock。
5. **事件先流式，session 后提交。** event stream 可以记录 delta 和中间态；session 默认仍只提交 finalized semantic messages。
6. **deadline 和 cancellation 是 runtime 责任。** schema 和 metadata 只声明约束，执行层负责强制超时、取消和恢复。

## 需要优化的 Core 能力

### 1. Async Provider Runtime

优先级：P0

问题：

- 当前 `LlmStream` 是同步 `Iterator`，无法自然承载真实 provider SSE。
- provider 请求期间缺少可组合的 cancellation、timeout、retry、fallback 和 token delta event。

建议新增：

```rust
pub trait AsyncProviderService: Send + Sync {
    fn call_async(
        &self,
        request: LlmRequest,
        control: RunControl,
    ) -> BoxFuture<'static, AgentCoreResult<AsyncLlmStream>>;
}

pub type AsyncLlmStream =
    Pin<Box<dyn Stream<Item = AgentCoreResult<LlmStreamEvent>> + Send>>;
```

同步兼容：

- `LlmProvider` 保持不变。
- 新增 `SyncProviderAdapter<T>`，把当前同步 provider 包装成 async service。
- `LlmService` 可以保留同步 `stream()`，同时新增 `stream_async()`。

验收：

- mock async provider 能发出 `ResponseCreated -> TextDelta -> ToolCallDelta -> ToolCallCompleted -> Usage -> Completed`。
- 取消 token 在流未完成时能终止 run，并生成 cancelled event。
- provider wrapper 能按 deadline 返回 recoverable error。

### 2. Runtime Event Channel

优先级：P0

问题：

- 当前 `AgentRunResult.events` 和 `GraphRunResult.events` 是 run 结束后读取。
- 真实长任务需要实时暴露 token、tool call、tool result、node started、node ended、agent done。

建议新增：

```rust
pub enum RuntimeEvent {
    Core(CoreEvent),
    TokenDelta { run_id: Uuid, text: String },
    ToolCallDelta { run_id: Uuid, call_id: String, delta: String },
    ToolCallStarted { run_id: Uuid, call_id: String, tool_name: String },
    ToolCallFinished { run_id: Uuid, call_id: String, status: ToolResultStatus },
    ProviderUsage { run_id: Uuid, usage: LlmUsage },
}

pub trait EventSink: Send + Sync {
    fn emit(&self, event: RuntimeEvent) -> AgentCoreResult<()>;
}
```

同步兼容：

- `EventLog` 保留。
- 新增 `BufferedEventSink`，把实时事件收集回 `Vec<CoreEvent>` 或 `Vec<RuntimeEvent>`，供同步 `run()` 返回。

验收：

- provider streaming delta 在 run 未完成前可被 sink 观察。
- tool result event 顺序稳定：`ToolCallStarted` 先于 `ToolCallFinished`。
- session replay 不保存 token delta，只保存 finalized assistant/tool messages。

### 3. Cancellation 和 Deadline Enforcement

优先级：P0

问题：

- `Agent::cancel` 当前只是 atomic flag。
- `ToolExecutionMetadata.timeout_ms` 未被 executor 强制执行。
- graph budget 能限制节点执行次数，但不能限制正在运行的 provider/tool future。

建议新增：

```rust
#[derive(Clone)]
pub struct RunControl {
    pub cancellation: AgentCancellationToken,
    pub deadline: Option<Instant>,
}

impl RunControl {
    pub fn is_cancelled(&self) -> bool;
    pub fn remaining(&self) -> Option<Duration>;
}
```

执行规则：

- provider call、tool call、node executor 都接收 `RunControl`。
- wrapper handler 可以缩短 deadline，不能延长父 deadline。
- timeout 统一映射为 recoverable 或 fatal，由 policy 决定。

验收：

- cancelled provider stream 产生 `GraphRunStatus::Cancelled`。
- timeout tool call 返回 `ToolResultStatus::Error` 或 `Denied`，并带 timeout reason。
- no-progress budget 和 wall-clock deadline 同时存在时，谁先触发谁决定 run status。

### 4. Async Tool Runtime

优先级：P1

问题：

- `Tool::invoke` 同步，适合本地 mock 和简单工具。
- 远程 MCP、HTTP、数据库、文件索引、OCR、embedding、shell adapter 都更适合 async。
- 当前 `execute_batch_parallel` 用线程并发，不适合大量 IO 工具和 async cancellation。

建议新增：

```rust
pub trait AsyncToolService: Send + Sync {
    fn call_tool_async(
        &self,
        definition: AgentDefinition,
        call: ToolCall,
        control: RunControl,
    ) -> BoxFuture<'static, AgentCoreResult<ToolResult>>;
}

pub struct AsyncToolExecutor {
    registry: Arc<ToolRegistry>,
    permission_policy: Arc<dyn ToolPermissionPolicy>,
    scheduler: ToolScheduler,
}
```

调度规则：

- `read_only && idempotent` 工具可以并发和预执行。
- destructive 或 device-mutating 工具默认串行。
- 同一个 `device_scope` 内的 mutating action 必须按顺序执行。
- hidden 工具仍由 `ToolRegistry::get_for_agent` 拦截。

同步兼容：

- 现有 `Tool` 不改。
- 新增 `SyncToolAdapter`，把 `Tool::invoke` 包成 async future。
- 后续可新增 `AsyncTool` 或 `RemoteToolService`，但不替代 `Tool`。

验收：

- read-only 工具批量并发时结果顺序仍按输入 call 顺序返回。
- mutating phone tools 在同一 device lock 下不会并发执行。
- timeout/cancel 会停止等待，并产生 tool error message。

### 5. Async Node Executor

优先级：P1

问题：

- `NodeKind::Agent` 当前明确要求 external node executor。
- provider node、tool node、sub-agent node 都还不能由 `GraphRunner` 直接执行。

建议新增：

```rust
pub trait AsyncNodeExecutor: Send + Sync {
    fn execute_node(
        &self,
        node: GraphNode,
        input: NodeExecutionInput,
        ctx: NodeExecutionContext,
    ) -> BoxFuture<'static, AgentCoreResult<NodeExecutionResult>>;
}

pub struct NodeExecutionContext {
    pub run_id: Uuid,
    pub definition: Arc<AgentDefinition>,
    pub event_sink: Arc<dyn EventSink>,
    pub provider: Arc<dyn AsyncProviderService>,
    pub tools: Arc<dyn AsyncToolService>,
    pub control: RunControl,
}
```

执行规则：

- 内建 `Noop/PassthroughInput/EmitMessages` 可以继续同步执行。
- `NodeKind::Agent` 交给 executor，可调用 child agent 或 subgoal runtime。
- `NodeKind::Transform { executor, .. }` 交给 executor registry。
- edge 仍只负责消息路由，不执行副作用。

验收：

- `NodeKind::Agent` 在提供 executor 时能产出 child agent message。
- 未提供 executor 时仍保留当前 explicit error，避免静默成功。
- parallel graph branch 可以并发执行 read-only/provider/context node，但 device action node 受 scheduler 限制。

### 6. Async Context Preparation

优先级：P1

问题：

- `ContextBuilder` 当前只做同步转换。
- 真实 agent 每轮需要准备多类上下文：session replay、task package、dependency artifact、probability prediction、tool hot/cold layer、local app map view、动态检索。
- 这些上下文块有不同稳定性，应该并发准备后按稳定性排序。

建议新增：

```rust
pub trait ContextSource: Send + Sync {
    fn prepare(
        &self,
        request: ContextPrepareRequest,
        control: RunControl,
    ) -> BoxFuture<'static, AgentCoreResult<ContextBlock>>;
}

pub struct ContextBlock {
    pub stability: ContextStability,
    pub messages: Vec<RunMessage>,
    pub metadata: BTreeMap<String, Value>,
}
```

排序规则：

1. stable prefix
2. direct tool schemas
3. task package
4. dependency artifacts
5. session summaries
6. recent finalized messages
7. volatile observations
8. diagnostics

验收：

- 多个 context source 并发准备。
- 输出顺序仍按稳定性排列，最大化 prompt cache 命中。
- 失败的低优先级 source 可以降级为 diagnostic，不阻塞核心 task context。

### 7. Async Hook Pipeline

优先级：P2

问题：

- 当前 `HandlerRegistry` 可手动运行，但 provider/tool/node 还没有统一经过 wrapper handler。
- 远程审批、审计日志、rate limit、retry、fallback 更适合 async hook。

建议新增：

```rust
pub trait AsyncPointHandler: Send + Sync {
    fn handle(
        &self,
        payload: HookPayload,
        control: RunControl,
    ) -> BoxFuture<'static, AgentCoreResult<PointHookDecision>>;
}

pub trait AsyncWrapperService: Send + Sync {
    fn call(
        &self,
        request: WrapperRequest,
        control: RunControl,
    ) -> BoxFuture<'static, AgentCoreResult<WrapperResult>>;
}
```

同步兼容：

- 当前 `HandlerRegistry` 保留。
- 新增 adapter，把同步 point/wrapper handler 包成 async handler。

验收：

- provider/tool/node 执行都能经过 wrapper chain。
- async wrapper 可实现 timeout、retry、fallback 和 recover。
- hook emitted events 能进入 `RuntimeEvent` stream。

## 不建议优先做的事

### 不直接把 `Tool::invoke` 改成 async

原因：

- 会破坏当前 `Arc<dyn Tool>` registry 的简单对象安全模型。
- 大量本地工具和测试没有必要引入 executor。
- 远程和 IO 工具可以通过 `AsyncToolService` 支持。

### 不直接把 `GraphRunner::run` 替换成 async

原因：

- 当前内建 node 都是内存操作。
- 真正需要 await 的是 provider/tool/sub-agent/custom node。
- 更好的做法是新增 `AsyncGraphRunner` 或 `GraphRuntime`，让同步 runner 继续作为 deterministic facade。

### 不让 edge 执行副作用

原因：

- edge 只负责 activation 和 context inheritance。
- provider/tool/node 执行应该在 node executor 或 runtime service 中完成。

## 建议实施顺序

### Phase 1：Runtime Control 和 Event Sink

新增：

- `RunControl`
- `RuntimeEvent`
- `EventSink`
- `BufferedEventSink`

覆盖：

- cancellation token 贯穿 provider/tool/node runtime。
- run 结束后仍能把 buffered events 投影成现有 result events。

### Phase 2：Async Provider Service

新增：

- `AsyncProviderService`
- `AsyncLlmStream`
- sync provider adapter
- provider deadline wrapper

覆盖：

- streaming delta
- usage aggregation
- cancel provider request
- provider timeout

### Phase 3：Async Tool Service 和 Scheduler

新增：

- `AsyncToolService`
- `AsyncToolExecutor`
- `ToolScheduler`
- sync tool adapter

覆盖：

- read-only/idempotent 并发
- mutating phone action 串行
- tool timeout
- permission guard 复用

### Phase 4：Async Graph Runtime

新增：

- `AsyncNodeExecutor`
- `AsyncGraphRunner` 或 `GraphRuntime`
- external node executor registry

覆盖：

- provider node
- tool node
- child agent node
- custom node
- parallel branch scheduling

### Phase 5：Async Context Sources 和 Hook Pipeline

新增：

- `ContextSource`
- `ContextPrepareRuntime`
- async point/wrapper handler adapters

覆盖：

- dynamic context 并发准备
- 稳定性排序
- async approval/audit/retry/fallback

## 测试计划

Core 层测试应该覆盖以下综合行为：

- async provider stream 被逐步消费，event sink 在 run 未完成前收到 delta。
- provider stream 被 cancel 后不会继续 append finalized assistant message。
- async tool timeout 产生 tool error message，并触发 wrapper recover。
- 两个 read-only 工具并发执行，两个 mutating device tools 串行执行。
- `NodeKind::Agent` 在提供 async executor 时能产出 child agent result；未提供时继续显式失败。
- 多个 context source 并发准备后，最终 context 仍按稳定性排序。
- async wrapper handler 能包住 provider/tool/node，且 emitted events 进入 runtime stream。
- 同步 public API 的 existing contract tests 不需要改调用方式。

## 文档同步要求

实现任一 phase 后，必须同步更新：

- `docs/architecture/rust-agent-core/08-llm/README.md`
- `docs/architecture/rust-agent-core/09-tool/README.md`
- `docs/architecture/rust-agent-core/04-graph/README.md`
- `docs/architecture/rust-agent-core/07-hooks/README.md`
- `docs/architecture/rust-agent-core/11-public-api/README.md`
- `docs/architecture/rust-agent-core/18-public-api-meaning-audit/README.md`

如果新增的是 additive API，要在 public API 文档中明确：

- 同步入口是否仍是稳定 facade；
- async 入口的 runtime 依赖；
- cancellation/deadline 的具体语义；
- event stream 是否会进入 session；
- provider/tool/node 哪些行为由 core 保证，哪些仍由 adapter 保证。

## 结论

`agent-core` 应该优化异步能力，但优化点是 runtime 编排，而不是直接把所有现有接口改成 async。

推荐路线：

1. 保留同步 public API。
2. 新增 async provider runtime、runtime event channel 和 cancellation/deadline。
3. 新增 async tool service 与 tool scheduler。
4. 新增 async node executor，让 provider/tool/sub-agent node 真正进入 graph runtime。
5. 新增 async context sources 和 async hook pipeline。

这样可以吸收 Rig 式 async agent 的关键能力，同时不破坏当前已经跑通的 core contract、mobilerun-like boundary probe 和 phone-using agent 验收路径。

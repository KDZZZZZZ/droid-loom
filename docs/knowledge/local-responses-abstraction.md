# Local Responses 抽象设计

日期：2026-05-27

本文定义 DroidLoom 在 Agent Graph 和本地 LLM 后端之间的 Response lower dialect。它参考 OpenAI Responses API 的对象模型、流式事件、工具调用和会话状态语义，但不是 OpenAI SDK 的拷贝；DroidLoom 需要在这层加入 Android action、workflow trace、KV Cache 生命周期、上下文复用和 WorkflowIR lowering metadata。

## 1. 结论

DroidLoom 应引入一层 `ResponseGraph / ResponseProgram / LocalResponses`：

```text
WorkflowIR / AgentGraph
  -> ResponseGraph
      -> ResponseProgram / ResponseIR
          -> LocalResponseRequest
              -> LocalResponseEngine
                  -> LlamaResponseEngine
                      -> llama.h
```

这层的目标是把高层 workflow 中的 LLM/Agent reasoning 子图 lower 成可执行 response programs。Agent Graph 仍然是最高优化层；ResponseGraph 是 LLM-call lower dialect；LocalResponseRequest 是单个 ResponseProgram 的一次运行实例。

底层可以先用 `llama.h` 实现，后续再切换 MLC LLM、llama-server、远端 OpenAI-compatible backend 或测试 fake engine。

`llama.h` 不提供这层抽象。它只提供 model/context/batch/decode/sampler/memory/state API。OpenAI Responses API 和 llama.cpp server 的 `/v1/responses` 可以作为协议参考，但 DroidLoom 必须保留自己的 cache-aware 和 Android-aware 扩展。

## 1.1 与 Agent Graph 和循环控制的关系

优化最高层定为 `WorkflowIR / AgentGraph`，不是 ReAct loop。

```text
AgentGraph
  node: ObserveScreen
  node: ResponseProgram
  node: ToolDispatch
  node: Guard
  node: ExecuteAction
  node: Verify
  edge: continue / retry / rollback / loop
```

ResponseProgram 不预设 agent loop。ReAct、Plan-Act、反思重试、多 agent handoff 都应由 AgentGraph/ResponseGraph 的节点和边表达，而不是成为 ResponseProgram 的枚举字段。

ReAct 可以被表达为 graph pattern：

```text
ResponseProgram(reason_and_select_tool)
  -> ToolDispatch
  -> ObserveScreen
  -> ResponseProgram(next_reason_or_finish)
```

它不适合作为最高优化层，也不应藏在 ResponseProgram 内部，因为 DroidLoom 需要在循环边界外分析和插入：

- Android 权限和能力披露；
- high-risk action guard；
- verifier dominance；
- observation pruning；
- tool side-effect ordering；
- prompt segment hoisting；
- 跨节点 KV Cache 生命周期；
- retry、rollback、human takeover；
- trace replay 和 benchmark harness。

因此：

```text
单次模型决策:
  如果只需要一次模型决策，一个 ResponseProgram 可以足够。

多步 workflow:
  一组 ResponseProgram 和 tool/observe/verifier 节点组成 ResponseGraph。

全局编译优化:
  发生在 AgentGraph / ResponseGraph 上，而不是单个 ResponseProgram 内部。
```

## 2. 参考来源

优先参考：

- OpenAI Responses API：请求对象、响应对象、stream event、tool call 和状态模型。
- OpenAI Python SDK generated types：`ResponseCreateParams`、`Response`、`ResponseInputItem`、`ResponseOutputItem`、`ResponseStreamEvent`。
- llama.cpp `llama-server`：提供 OpenAI-compatible chat completions、responses、embeddings routes；但其 `/v1/responses` 目前是把 Responses request 转成 Chat Completions request 执行。
- [`llama.h`](./llama-h-usage.md)：DroidLoom 本地后端的最终 C API 接入面。

不要参考：

- OpenAI 云端内部实现。DroidLoom 只需要学习 API 语义。
- `llama-server` 的 HTTP server 形态。Android App 内部 runtime 不应依赖常驻 HTTP server。
- Agent 框架的完整 runtime。LangGraph 等框架可参考 state/checkpoint，但不应成为 DroidLoom 的核心依赖。

## 3. 抽象边界

| 层 | 负责 | 不负责 |
| --- | --- | --- |
| `runtime-agent` | planner loop、tool registry、动作解释、verifier 编排 | token decode、KV 物理操作 |
| `WorkflowIR / AgentGraph` | 控制流、数据流、权限、风险、状态、verifier、retry、rollback、跨节点 cache 生命周期 | 模型 token decode |
| `ResponseGraph` | LLM/Agent reasoning 子图、ResponseProgram 依赖、tool output 续接、cache plan 串联 | Android 原始 API 执行 |
| `LocalResponses` | 统一请求、输出 item、stream event、tool call、usage、cache hint、错误模型 | Android 权限执行、模型 kernel 优化 |
| `LlamaResponseEngine` | prompt flatten、chat template、grammar、decode loop、KV handle 映射 | workflow 策略、安全确认 |
| `llama.h` | tokenization、batch、decode、sampler、memory/state API | Responses 对象、tool call、trace 语义 |

这一层的核心价值是把“Agent Graph 怎么 lower 成可执行 LLM step”与“模型怎么生成 token 和复用 KV”解耦。Agent 和性能优化可以并行开发，只要双方遵守同一份 `ResponseProgram / LocalResponses` 契约。

## 3.1 ResponseProgram

`ResponseProgram` 是 WorkflowIR 中单次模型交互节点的 lower 结果。它比单个 OpenAI-style request 更丰富，因为它必须保留 compiler 需要的 sidecar metadata，但它不内置 agent loop。

```kotlin
data class ResponseProgram(
    val id: ResponseProgramId,
    val instructions: List<ResponseInputItem>,
    val inputBindings: List<InputBinding>,
    val tools: List<ToolSpec>,
    val constraints: ResponseConstraints,
    val cache: CacheHints,
    val guards: List<GuardBinding>,
    val verifiers: List<VerifierBinding>,
    val trace: TraceConfig,
    val lowering: ResponseLoweringManifest
)
```

`ResponseConstraints` 只描述单次模型交互的约束，例如最大输出 token、结构化输出 schema、tool choice、是否允许并行无副作用工具。它不能包含 loop 策略。

循环语义属于 ResponseGraph：

| 图结构 | 用途 |
| --- | --- |
| `ResponseProgram -> ToolDispatch -> ObserveScreen -> ResponseProgram` | ReAct-style 循环 |
| `ResponseProgram -> Guard -> ExecuteAction -> Verify` | 受控 action 执行 |
| `Verify -> ResponseProgram` | 失败重试或反思 |
| `ResponseProgram -> HumanApproval -> ExecuteAction` | 高风险动作确认 |

`ResponseProgram` lower 到 `LocalResponseRequest` 时，runtime 会绑定当前 screen state、tool output、session id 和 cache state。

```kotlin
data class ResponseLoweringManifest(
    val workflowId: WorkflowId,
    val workflowVersion: String,
    val workflowNodeId: WorkflowNodeId,
    val responseProgramId: ResponseProgramId,
    val segmentMap: List<PromptSegmentBinding>,
    val toolMap: List<ToolBinding>,
    val cachePlanId: CachePlanId,
    val verifierRefs: List<VerifierNodeId>,
    val guardPolicyRefs: List<GuardPolicyId>
)
```

## 4. 请求对象

`LocalResponseRequest` 应保留 OpenAI Responses 的关键字段，并增加 DroidLoom 专有字段。

```kotlin
data class LocalResponseRequest(
    val model: String,
    val instructions: List<ResponseInputItem> = emptyList(),
    val input: List<ResponseInputItem>,
    val tools: List<ToolSpec> = emptyList(),
    val toolChoice: ToolChoice = ToolChoice.Auto,
    val parallelToolCalls: Boolean = false,
    val text: TextConfig = TextConfig.Plain,
    val sampling: SamplingConfig = SamplingConfig(),
    val maxOutputTokens: Int,
    val previousResponseId: ResponseId? = null,
    val sessionId: SessionId,
    val stream: Boolean = true,
    val metadata: Map<String, String> = emptyMap(),
    val cache: CacheHints = CacheHints.None,
    val trace: TraceConfig = TraceConfig.Default,
    val lowering: ResponseLoweringManifest
)
```

字段语义：

| 字段 | OpenAI 对应 | DroidLoom 语义 |
| --- | --- | --- |
| `model` | `model` | 模型或本地 backend profile id |
| `instructions` | `instructions` / system/developer message | 系统策略、workflow 固定前缀、能力边界 |
| `input` | `input` | 用户目标、screen state、tool output、history item |
| `tools` | `tools` | Android API、Accessibility action、workflow tool、verifier tool |
| `toolChoice` | `tool_choice` | `auto`、`none`、指定工具、强制 action schema |
| `parallelToolCalls` | `parallel_tool_calls` | MVP 先禁用；后续只允许无副作用工具并行 |
| `text` | `text` | plain text 或 JSON schema structured output |
| `sampling` | `temperature` / `top_p` 等 | 映射到 llama sampler chain |
| `maxOutputTokens` | `max_output_tokens` | 包括可见输出和结构化 action JSON |
| `previousResponseId` | `previous_response_id` | 逻辑会话续接，不等于 KV 物理句柄 |
| `sessionId` | 无完全等价 | 本地 session、权限、trace、KV 生命周期作用域 |
| `cache` | `prompt_cache_key` / `prompt_cache_retention` | 本地 cache hint、segment id、KV 复用策略 |
| `trace` | `metadata` / dashboard trace | DroidLoom 本地可观测事件配置 |
| `lowering` | 无直接对应 | WorkflowIR、ResponseProgram、tool、segment、verifier、guard 的映射关系 |

### 4.1 `instructions` 和 `input`

OpenAI Responses 支持字符串输入和 item list。DroidLoom 应统一使用 item list，避免早期为了方便传字符串，后期再迁移。

```kotlin
sealed interface ResponseInputItem {
    val id: String?
    val type: String
}

data class MessageItem(
    override val id: String? = null,
    val role: Role,
    val content: List<ContentPart>,
    val cacheScope: CacheScope = CacheScope.Session,
    val invalidationKey: String? = null
) : ResponseInputItem {
    override val type = "message"
}

data class FunctionCallOutputItem(
    override val id: String? = null,
    val callId: String,
    val output: ToolOutput,
    val status: ItemStatus = ItemStatus.Completed
) : ResponseInputItem {
    override val type = "function_call_output"
}
```

`instructions` 不应自动从 `previousResponseId` 继承。OpenAI Responses 中也明确把新请求的 instructions 视为本轮插入的系统或 developer message。DroidLoom 应让 workflow compiler 每轮显式生成 instructions，这样系统前缀、工具 schema、能力边界和安全策略的失效条件才可分析。

## 5. 输出对象

`LocalResponse` 对应 OpenAI `Response` 对象，但增加本地 trace 和 cache 统计。

```kotlin
data class LocalResponse(
    val id: ResponseId,
    val createdAtMs: Long,
    val completedAtMs: Long?,
    val status: ResponseStatus,
    val model: String,
    val output: List<ResponseOutputItem>,
    val outputText: String,
    val previousResponseId: ResponseId?,
    val usage: ResponseUsage,
    val cache: CacheReport,
    val traceId: TraceId,
    val error: ResponseError? = null,
    val incompleteDetails: IncompleteDetails? = null
)
```

状态集合：

```kotlin
enum class ResponseStatus {
    Queued,
    InProgress,
    Completed,
    Failed,
    Cancelled,
    Incomplete
}
```

OpenAI Response 的 `output` 是 item array，不应假设第一个 item 一定是 assistant message。DroidLoom 也应保留这个设计，因为一次 planner 输出可能包含：

- 一段给用户看的 `message`;
- 一个或多个 `function_call`;
- 一个 `approval_request`;
- 一个 `reasoning` 或 debug summary；
- 一个内部 `compaction` 或 cache 管理 item。

## 6. 输出 item

MVP 只需要实现下列 output item：

```kotlin
sealed interface ResponseOutputItem {
    val id: String
    val type: String
    val status: ItemStatus
}

data class OutputMessageItem(
    override val id: String,
    val role: Role = Role.Assistant,
    val content: List<ContentPart>,
    override val status: ItemStatus = ItemStatus.Completed
) : ResponseOutputItem {
    override val type = "message"
}

data class FunctionCallItem(
    override val id: String,
    val callId: String,
    val name: String,
    val argumentsJson: String,
    val namespace: String? = null,
    override val status: ItemStatus = ItemStatus.Completed
) : ResponseOutputItem {
    override val type = "function_call"
}

data class ApprovalRequestItem(
    override val id: String,
    val callId: String,
    val risk: RiskLevel,
    val message: String,
    val actionPreview: ActionPreview,
    override val status: ItemStatus = ItemStatus.InProgress
) : ResponseOutputItem {
    override val type = "approval_request"
}
```

DroidLoom 的 Android action 不应被模型直接执行。模型只生成 `FunctionCallItem`；`runtime-agent` 解析后交给 Guard。高风险 action 先产生 `ApprovalRequestItem`，用户确认后再由 Executor 执行，并把执行结果作为 `FunctionCallOutputItem` 放入下一轮 `input`。

## 7. 工具调用闭环

工具调用采用 OpenAI Responses 的 `call_id` 模型：

```text
request(input + tools)
  -> response.output: function_call(call_id, name, arguments)
  -> runtime-agent 执行或请求确认
  -> next request.input: function_call_output(call_id, output)
  -> model 继续生成最终 message 或下一步 function_call
```

约束：

- `callId` 必须全局唯一，至少在 `sessionId` 内唯一。
- `function_call_output.callId` 必须匹配之前未完成的 `function_call.callId`。
- 有副作用工具默认不允许 `parallelToolCalls`。
- Android action 工具必须声明 `risk`、`requiredPermissions`、`targetPackagePolicy` 和 `verifier`。
- tool output 必须结构化，不能只回传自然语言成功或失败。

示例工具：

```kotlin
data class ToolSpec(
    val name: String,
    val namespace: String = "android",
    val description: String,
    val inputSchema: JsonSchema,
    val outputSchema: JsonSchema,
    val risk: RiskLevel,
    val sideEffect: SideEffect,
    val requiredCapabilities: Set<Capability>
)
```

## 8. 流式事件

OpenAI Responses streaming 使用语义事件，而不是裸 token 流。DroidLoom 应采用同类设计，并增加本地 cache 和 Android action 事件。

```kotlin
sealed interface LocalResponseEvent {
    val responseId: ResponseId
    val sequence: Long
    val timestampMs: Long
}
```

基础事件：

| 事件 | 语义 |
| --- | --- |
| `response.created` | request 已进入 engine |
| `response.in_progress` | 开始构造 prompt 或 prefill |
| `response.output_item.added` | 新 output item 开始生成 |
| `response.content_part.added` | message content part 开始生成 |
| `response.output_text.delta` | 文本增量 |
| `response.output_text.done` | 文本 part 结束 |
| `response.function_call_arguments.delta` | function call 参数增量 |
| `response.function_call_arguments.done` | function call 参数完整 |
| `response.output_item.done` | output item 完成 |
| `response.completed` | 整个 response 完成 |
| `response.failed` | 失败 |
| `response.incomplete` | 因 max token、安全或上下文限制未完成 |
| `response.cancelled` | 用户或系统取消 |

DroidLoom 扩展事件：

| 事件 | 语义 |
| --- | --- |
| `cache.lookup_started` | 开始匹配 prompt segment / KV |
| `cache.hit` | 命中可复用 prefix |
| `cache.miss` | 未命中，需重新 prefill |
| `cache.segment_materialized` | segment 已 tokenized/prefilled |
| `cache.snapshot_saved` | 保存 KV/state snapshot |
| `cache.snapshot_restored` | 恢复 KV/state snapshot |
| `android.tool_guard_required` | action 需要人工确认 |
| `android.tool_started` | action 开始执行 |
| `android.tool_completed` | action 执行完成 |
| `android.tool_failed` | action 执行失败 |
| `verifier.started` | 开始状态校验 |
| `verifier.completed` | 校验完成 |

事件必须进入 trace store。UI 可以只订阅用户可见事件，benchmark 可以订阅全部事件。

## 9. 缓存和 KV 抽象

OpenAI 的 `prompt_cache_key` 是服务端缓存命中提示；`prompt_cache_retention` 表示缓存保留策略。DroidLoom 不能只复制这两个字段，因为本地 workflow compiler 需要分析 prompt segment 的 live range、失效条件和 KV 生命周期。

推荐结构：

```kotlin
data class CacheHints(
    val promptCacheKey: String?,
    val retention: CacheRetention = CacheRetention.InMemory,
    val segments: List<PromptSegmentHint> = emptyList(),
    val reusePolicy: ReusePolicy = ReusePolicy.BestEffort
)

data class PromptSegmentHint(
    val segmentId: String,
    val role: Role,
    val cacheScope: CacheScope,
    val invalidationKey: String,
    val expectedTokenCount: Int? = null,
    val liveIn: Set<WorkflowNodeId> = emptySet(),
    val liveOut: Set<WorkflowNodeId> = emptySet()
)
```

`previousResponseId` 和 `promptCacheKey` 都不是 KV handle：

- `previousResponseId` 表示逻辑会话续接；
- `promptCacheKey` 表示相似请求的缓存分桶；
- `KvHandle` 是 DroidLoom 本地 engine 内部资源，不能暴露给 Agent 或 workflow author；
- workflow compiler 可以产生 `CacheHints`，但不能直接调用 `llama_memory_seq_*`。

底层映射：

| LocalResponses 概念 | `llama.h` 映射 |
| --- | --- |
| `PromptSegmentHint` | token range + seq id + pos range |
| cache hit | 跳过已存在 prefix 的 prefill |
| branch fork | `llama_memory_seq_cp` |
| rollback | `llama_memory_seq_rm` |
| active branch switch | `llama_memory_seq_keep` |
| context compaction | `llama_memory_seq_add` / `llama_memory_seq_div` 实验 |
| snapshot save | `llama_state_seq_save_file` 或完整 state save |
| snapshot restore | `llama_state_seq_load_file` 或完整 state load |

## 10. `LlamaResponseEngine` 实现路径

`LlamaResponseEngine` 的职责是把 Responses-like 请求转成 `llama.h` 可执行的 decode 任务。

```text
LocalResponseRequest
  -> validate model/session/tool/cache config
  -> build PromptSegments
  -> resolve cache plan
  -> apply chat template
  -> tokenize
  -> restore or prefill KV
  -> decode
  -> parse structured output / tool call
  -> emit LocalResponseEvent
  -> persist trace and cache report
```

实现原则：

- prompt builder 必须保留 segment metadata，不能只产出一段字符串。
- chat template 只在最后一步 flatten，不能吞掉 cache scope。
- function call MVP 可以用 JSON schema + grammar/constrained output；不要在 C++ 层隐式执行工具。
- streaming 事件在 Kotlin 层统一建模，native 层只返回 token、logits、timing、error 和 cache 操作结果。
- structured output 解析失败由 `runtime-agent` 决定 retry，不由 `llama.h` adapter 偷偷重试。

## 11. 错误模型

```kotlin
data class ResponseError(
    val type: ErrorType,
    val code: String,
    val message: String,
    val retryable: Boolean = false,
    val cause: Throwable? = null
)
```

错误分类：

| 类型 | 例子 |
| --- | --- |
| `InvalidRequest` | schema 错误、tool output 缺少 `callId`、上下文超限 |
| `ModelUnavailable` | 模型未加载、GGUF 不存在、profile 不支持 |
| `BackendError` | `llama_decode` 失败、sampler 构造失败 |
| `StructuredOutputError` | JSON/tool call 解析失败 |
| `PermissionDenied` | Android 能力未授权 |
| `GuardRejected` | 高风险 action 被策略或用户拒绝 |
| `Cancelled` | 用户取消或 session 被 takeover |
| `Timeout` | prefill/decode/tool/verifier 超时 |

## 12. Usage 和性能统计

OpenAI `usage` 提供 input/output/total token。DroidLoom 需要保留这部分，同时加入本地性能字段。

```kotlin
data class ResponseUsage(
    val inputTokens: Int,
    val cachedInputTokens: Int,
    val outputTokens: Int,
    val totalTokens: Int,
    val prefillMs: Long,
    val decodeMs: Long,
    val tokensPerSecond: Double,
    val peakNativeMemoryBytes: Long? = null
)
```

`cachedInputTokens` 是优化有效性的核心指标。它应来自 cache plan 和 `llama.h` 实际 prefill 结果，而不是只按字符串前缀估算。

## 13. MVP 范围

第一版只实现：

1. `LocalResponseRequest` / `LocalResponse` / `LocalResponseEvent` 数据模型。
2. text-only message input。
3. function tool call JSON 输出。
4. `function_call_output` 下一轮回填。
5. token streaming 到 `response.output_text.delta`。
6. `cache.hit` / `cache.miss` 事件先用 prompt segment metadata 模拟。
7. `usage` 记录 input/output token、prefill/decode latency。
8. fake engine 测试和 `LlamaResponseEngine` smoke test。

第一版不做：

- OpenAI 内置 web/file/code/computer tools 的完整兼容。
- 多模态 input。
- 真实 KV 跨 session 持久化。
- 多 tool 并行执行。
- background response。
- HTTP server 兼容层。

## 14. 与 llama-server 的关系

llama.cpp server 的 `/v1/responses` 很适合做兼容性参考和桌面调试工具，但不应成为 DroidLoom Android runtime 的内部主路径。

原因：

- 它当前通过转换到 Chat Completions 执行，不能表达 DroidLoom 的 workflow/KV 生命周期。
- HTTP server 增加 Android 内部部署复杂度。
- DroidLoom 需要控制 `llama_context`、`llama_batch`、`llama_memory` 和 state 文件策略。
- Agent trace、Android 权限和 verifier 都在 App runtime 内部，绕一层 HTTP 会让错误和取消语义变复杂。

可保留一个可选 desktop/dev adapter：

```text
LocalResponsesClient
  -> LlamaServerResponsesAdapter
      -> http://localhost:8080/v1/responses
```

这个 adapter 用于协议对齐测试，不用于最终 Android 端内推理。

## 15. 测试契约

必须有以下 contract tests：

| 测试 | 目标 |
| --- | --- |
| message text roundtrip | 输入文本，输出 message item 和 outputText |
| tool call parse | 结构化 JSON 生成 `FunctionCallItem` |
| tool output continuation | `function_call_output.callId` 续接上一轮 |
| stream ordering | event sequence 单调递增，done/completed 顺序正确 |
| cancellation | cancel 后不再发送 delta，只发送 cancelled |
| cache event | 相同 segment 产生 hit，不同 invalidationKey 产生 miss |
| context overflow | 超上下文时返回 `InvalidRequest` 或执行明确 truncation 策略 |
| fake engine parity | fake engine 和 llama engine 输出同形对象 |

## 16. 参考资料

- OpenAI Responses API: https://platform.openai.com/docs/api-reference/responses
- OpenAI streaming Responses: https://platform.openai.com/docs/api-reference/responses-streaming
- OpenAI function calling guide: https://platform.openai.com/docs/guides/function-calling
- OpenAI conversation state guide: https://platform.openai.com/docs/guides/conversation-state
- OpenAI prompt caching guide: https://platform.openai.com/docs/guides/prompt-caching
- OpenAI Python `ResponseCreateParams`: https://github.com/openai/openai-python/blob/main/src/openai/types/responses/response_create_params.py
- OpenAI Python `Response`: https://github.com/openai/openai-python/blob/main/src/openai/types/responses/response.py
- OpenAI Python `ResponseStreamEvent`: https://github.com/openai/openai-python/blob/main/src/openai/types/responses/response_stream_event.py
- llama.cpp server README: https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md
- ReAct: https://arxiv.org/abs/2210.03629
- LangGraph overview: https://docs.langchain.com/oss/python/langgraph/overview
- OpenAI Agents SDK: https://openai.github.io/openai-agents-python/
- Microsoft Agent Framework overview: https://learn.microsoft.com/en-us/agent-framework/overview/
- LlamaIndex Workflows: https://docs.llamaindex.ai/en/stable/workflows/
- CrewAI Flows: https://docs.crewai.com/en/concepts/flows
- DroidLoom [`llama.h` 使用文档](./llama-h-usage.md)

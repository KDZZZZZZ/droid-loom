# Local Responses 抽象设计

日期：2026-05-27

本文只定义 DroidLoom 的 `LocalResponses` 层。它参考 OpenAI Responses API 的请求、响应、item、工具调用和流式事件模型，但不是 OpenAI SDK 的拷贝。本文不定义工作流、Agent loop、多 Agent、调度策略、权限策略或 UI 交互；这些都是调用方或更上层 runtime 的责任。

`LocalResponses` 的目标是提供一个本地、可流式、可工具调用、可观测、cache-aware 的模型交互契约。任何上层系统只要能构造 `LocalResponseRequest`，并消费 `LocalResponseEvent` / `LocalResponse`，就可以替换底层实现。

## 1. 范围

`LocalResponses` 负责：

- 描述一次模型交互请求；
- 描述模型输出对象；
- 描述输入 item 和输出 item；
- 描述模型可见工具；
- 描述工具调用和工具结果的关联方式；
- 描述流式事件顺序；
- 描述本地 prompt/KV cache hint 和 cache report；
- 描述 token usage、latency、错误和取消语义；
- 为 `llama.h`、MLC LLM、llama-server 或 fake engine 提供统一 adapter 边界。

`LocalResponses` 不负责：

- 定义工作流图；
- 定义 Agent loop；
- 定义多 Agent 协作；
- 决定什么时候调用工具；
- 执行 Android API 或 Accessibility action；
- 做权限弹窗、人工确认或策略审批；
- 做 verifier、retry、rollback；
- 直接暴露 `llama_context`、KV memory 指针或 state 文件。

## 2. 分层边界

```text
caller
  -> LocalResponsesClient
      -> LocalResponseEngine
          -> backend adapter
              -> llama.h / MLC LLM / llama-server / fake engine
```

| 层 | 负责 | 不负责 |
| --- | --- | --- |
| `caller` | 构造请求、执行工具、处理最终结果 | 直接操作模型上下文 |
| `LocalResponsesClient` | 统一 API、stream 订阅、取消、超时 | 决定业务策略 |
| `LocalResponseEngine` | 请求校验、prompt 构造、cache plan、decode、事件输出 | 执行外部工具副作用 |
| backend adapter | tokenization、decode、sampling、KV/state 操作 | 理解上层工作流语义 |

工具进入 `LocalResponseRequest.tools` 后，即表示它对模型可见。runtime-only 或 compiler-only 能力不应出现在这个字段里。

## 3. 核心不变量

1. 一个 `LocalResponseRequest` 产生一个 `LocalResponse`。
2. 流式模式下，`LocalResponseEvent` 是运行中状态的事实来源。
3. `LocalResponse.output` 是 item array，调用方不能假设第一个 item 一定是文本消息。
4. 工具调用必须通过 `callId` 和工具结果关联。
5. `previousResponseId` 是逻辑续接标识，不是 KV handle。
6. `sessionId` 是本地会话作用域，不直接等价于模型上下文。
7. `CacheHints` 是请求级提示；实际是否命中由 engine 决定，并通过 `CacheReport` / cache event 返回。
8. backend 内部 KV 资源不能暴露给调用方。
9. 请求对象进入 engine 后应视为不可变。
10. 取消后不得继续发送 token delta，只能发送终止事件。

## 4. 请求对象

```kotlin
data class LocalResponseRequest(
    val idempotencyKey: String? = null,
    val model: String,
    val instructions: List<ResponseInputItem> = emptyList(),
    val input: List<ResponseInputItem>,
    val tools: List<ToolSpec> = emptyList(),
    val toolChoice: ToolChoice = ToolChoice.Auto,
    val parallelToolCalls: Boolean = false,
    val text: TextConfig = TextConfig.Plain,
    val sampling: SamplingConfig = SamplingConfig(),
    val maxOutputTokens: Int,
    val stop: List<String> = emptyList(),
    val previousResponseId: ResponseId? = null,
    val sessionId: SessionId? = null,
    val stream: Boolean = true,
    val metadata: Map<String, String> = emptyMap(),
    val cache: CacheHints = CacheHints.None,
    val trace: TraceConfig = TraceConfig.Default
)
```

字段语义：

| 字段 | 语义 |
| --- | --- |
| `idempotencyKey` | 调用方生成的幂等键，用于避免重试时重复生成 |
| `model` | 本地模型或 backend profile id |
| `instructions` | 本次请求插入的 system/developer 级输入 |
| `input` | 本次模型交互的输入 item |
| `tools` | 本次请求允许模型调用的工具 |
| `toolChoice` | 工具选择策略 |
| `parallelToolCalls` | 是否允许模型在一次响应中产生多个并行工具调用 |
| `text` | 输出文本或结构化输出配置 |
| `sampling` | 采样配置 |
| `maxOutputTokens` | 输出 token 上限 |
| `stop` | 额外停止序列 |
| `previousResponseId` | 逻辑续接的上一轮 response id |
| `sessionId` | 本地 session 作用域 |
| `stream` | 是否以事件流返回 |
| `metadata` | 透明元数据，engine 只记录不解释业务含义 |
| `cache` | prompt/cache hint |
| `trace` | 本层 trace 配置 |

`instructions` 不自动从 `previousResponseId` 继承。调用方必须显式传入本轮需要的 instructions。

## 5. 输入 item

```kotlin
sealed interface ResponseInputItem {
    val id: String?
    val type: String
}
```

### 5.1 Message

```kotlin
data class MessageItem(
    override val id: String? = null,
    val role: Role,
    val content: List<ContentPart>,
    val status: ItemStatus? = null
) : ResponseInputItem {
    override val type = "message"
}

enum class Role {
    System,
    Developer,
    User,
    Assistant
}
```

### 5.2 Function Call Output

工具执行结果作为下一轮输入回填：

```kotlin
data class FunctionCallOutputItem(
    override val id: String? = null,
    val callId: String,
    val output: ToolOutput,
    val status: ItemStatus = ItemStatus.Completed
) : ResponseInputItem {
    override val type = "function_call_output"
}
```

约束：

- `callId` 必须对应之前产生的 `FunctionCallItem.callId`。
- `output` 应为结构化数据或明确的错误对象。
- 工具失败也应回填为 tool output，而不是丢弃。

### 5.3 Item Reference

如果实现支持本地 response store，可以通过引用复用历史 item：

```kotlin
data class ItemReference(
    override val id: String? = null,
    val targetItemId: String
) : ResponseInputItem {
    override val type = "item_reference"
}
```

MVP 可以不实现 `ItemReference`。

## 6. Content Part

```kotlin
sealed interface ContentPart {
    val type: String
}

data class TextPart(
    val text: String
) : ContentPart {
    override val type = "input_text"
}

data class JsonPart(
    val value: JsonValue
) : ContentPart {
    override val type = "input_json"
}

data class ImagePart(
    val image: ImageRef,
    val detail: ImageDetail = ImageDetail.Auto
) : ContentPart {
    override val type = "input_image"
}

data class FilePart(
    val file: FileRef
) : ContentPart {
    override val type = "input_file"
}
```

MVP 只要求实现 `TextPart` 和 `JsonPart`。`ImagePart`、`FilePart` 留作多模态和附件扩展。

## 7. 工具定义

`tools` 中的工具全部是模型可见工具。

```kotlin
data class ToolSpec(
    val type: ToolType = ToolType.Function,
    val name: String,
    val namespace: String? = null,
    val description: String,
    val inputSchema: JsonSchema,
    val outputSchema: JsonSchema? = null,
    val annotations: ToolAnnotations = ToolAnnotations()
)

data class ToolAnnotations(
    val readOnly: Boolean = false,
    val idempotent: Boolean = false,
    val destructive: Boolean = false,
    val openWorld: Boolean = true
)
```

`ToolAnnotations` 只描述工具性质，不执行策略。调用方可以用这些信息做安全策略，但该策略不属于 `LocalResponses` 层。

### 7.1 Tool Choice

```kotlin
sealed interface ToolChoice {
    data object Auto : ToolChoice
    data object None : ToolChoice
    data object Required : ToolChoice
    data class Function(val name: String, val namespace: String? = null) : ToolChoice
}
```

语义：

| 选择 | 语义 |
| --- | --- |
| `Auto` | 模型自行决定是否调用工具 |
| `None` | 禁止工具调用 |
| `Required` | 至少产生一个工具调用 |
| `Function` | 必须调用指定工具 |

## 8. 输出对象

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
    val traceId: TraceId?,
    val metadata: Map<String, String> = emptyMap(),
    val error: ResponseError? = null,
    val incompleteDetails: IncompleteDetails? = null
)
```

状态：

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

`outputText` 是便利字段，由所有 `OutputTextPart` 拼接而来。调用方如果关心结构化输出或工具调用，必须读取 `output`。

## 9. 输出 item

```kotlin
sealed interface ResponseOutputItem {
    val id: String
    val type: String
    val status: ItemStatus
}
```

### 9.1 Message

```kotlin
data class OutputMessageItem(
    override val id: String,
    val role: Role = Role.Assistant,
    val content: List<OutputContentPart>,
    override val status: ItemStatus = ItemStatus.Completed
) : ResponseOutputItem {
    override val type = "message"
}
```

### 9.2 Function Call

```kotlin
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
```

约束：

- `callId` 必须在当前 session 内唯一。
- `argumentsJson` 必须能按对应 `ToolSpec.inputSchema` 校验。
- 如果 `parallelToolCalls = false`，一次 response 最多输出一个 `FunctionCallItem`。

### 9.3 Reasoning / Refusal

本层可以保留可选扩展 item，但 MVP 不要求实现：

```kotlin
data class ReasoningItem(...)
data class RefusalItem(...)
```

reasoning 内容的保留、脱敏和持久化由 engine 配置决定。

## 10. 输出 Content Part

```kotlin
sealed interface OutputContentPart {
    val type: String
}

data class OutputTextPart(
    val text: String,
    val annotations: List<TextAnnotation> = emptyList()
) : OutputContentPart {
    override val type = "output_text"
}

data class OutputJsonPart(
    val value: JsonValue
) : OutputContentPart {
    override val type = "output_json"
}
```

如果 `TextConfig` 要求 JSON schema，engine 应优先返回 `OutputJsonPart` 或可校验的 `OutputTextPart`。

## 11. 工具调用闭环

工具调用闭环采用 `callId`：

```text
LocalResponseRequest(input + tools)
  -> LocalResponse.output: FunctionCallItem(callId, name, argumentsJson)
  -> caller executes tool
  -> next LocalResponseRequest.input: FunctionCallOutputItem(callId, output)
  -> engine continues from tool result
```

`LocalResponses` 不执行外部工具。它只负责：

- 把工具 schema 渲染进模型上下文；
- 约束工具调用格式；
- 校验模型输出的工具调用；
- 通过 output item 和 stream event 返回工具调用；
- 接收下一轮请求中的工具结果。

## 12. 流式事件

```kotlin
sealed interface LocalResponseEvent {
    val responseId: ResponseId
    val sequence: Long
    val timestampMs: Long
}
```

事件顺序必须满足：

```text
response.created
  -> response.in_progress
  -> zero or more delta/item events
  -> one terminal event
```

terminal event 只能是：

- `response.completed`;
- `response.failed`;
- `response.incomplete`;
- `response.cancelled`。

基础事件：

| 事件 | 语义 |
| --- | --- |
| `response.created` | 请求已被 engine 接收 |
| `response.in_progress` | engine 开始处理请求 |
| `response.output_item.added` | 新 output item 开始 |
| `response.content_part.added` | 新 content part 开始 |
| `response.output_text.delta` | 文本增量 |
| `response.output_text.done` | 文本 part 结束 |
| `response.function_call_arguments.delta` | 工具参数 JSON 增量 |
| `response.function_call_arguments.done` | 工具参数 JSON 完整 |
| `response.output_item.done` | output item 完成 |
| `response.usage.updated` | usage 增量更新 |
| `response.completed` | response 完成 |
| `response.failed` | response 失败 |
| `response.incomplete` | response 未完整完成 |
| `response.cancelled` | response 被取消 |

本地 cache 事件：

| 事件 | 语义 |
| --- | --- |
| `cache.lookup_started` | 开始查找可复用 prefix |
| `cache.hit` | 命中可复用 prefix |
| `cache.miss` | 未命中 |
| `cache.segment_materialized` | segment 已完成 tokenization 或 prefill |
| `cache.snapshot_saved` | 保存 state/KV snapshot |
| `cache.snapshot_restored` | 恢复 state/KV snapshot |

事件的 `sequence` 必须在同一个 response 内单调递增。

## 13. CacheHints

OpenAI 的 `prompt_cache_key` 是服务端缓存命中提示；DroidLoom 本地实现需要更明确地描述 prompt segment。

```kotlin
data class CacheHints(
    val promptCacheKey: String? = null,
    val retention: CacheRetention = CacheRetention.InMemory,
    val segments: List<PromptSegmentHint> = emptyList(),
    val reusePolicy: ReusePolicy = ReusePolicy.BestEffort
) {
    companion object {
        val None = CacheHints(reusePolicy = ReusePolicy.Disabled)
    }
}

data class PromptSegmentHint(
    val segmentId: String,
    val role: Role,
    val textHash: String,
    val cacheScope: CacheScope,
    val invalidationKey: String,
    val expectedTokenCount: Int? = null
)
```

`CacheHints` 只表达本次请求的 cache 意图。engine 可以因为模型不匹配、上下文参数变化、segment hash 不匹配、内存不足或策略限制而拒绝复用。

### 13.1 CacheReport

```kotlin
data class CacheReport(
    val lookupCount: Int,
    val hitCount: Int,
    val missCount: Int,
    val cachedInputTokens: Int,
    val materializedInputTokens: Int,
    val restoredSnapshots: List<String> = emptyList(),
    val savedSnapshots: List<String> = emptyList()
)
```

`cachedInputTokens` 必须来自 engine 的实际执行结果，不能只靠字符串估算。

### 13.2 KV 资源边界

| LocalResponses 概念 | backend 内部映射 |
| --- | --- |
| `PromptSegmentHint` | token range、seq id、pos range |
| cache hit | 跳过已存在 prefix 的 prefill |
| cache miss | tokenization + prefill |
| snapshot save | backend state save |
| snapshot restore | backend state restore |

调用方不能拿到 `KvHandle`。如果未来需要调试，可以通过 trace 暴露只读 metadata。

## 14. SamplingConfig

```kotlin
data class SamplingConfig(
    val seed: Long? = null,
    val temperature: Float? = null,
    val topP: Float? = null,
    val topK: Int? = null,
    val minP: Float? = null,
    val repetitionPenalty: Float? = null,
    val logitBias: Map<Int, Float> = emptyMap()
)
```

structured output 或工具调用场景应允许 engine 覆盖为更保守的采样策略，例如低温度、grammar 或 JSON schema constrained decoding。

## 15. TextConfig

```kotlin
sealed interface TextConfig {
    data object Plain : TextConfig

    data class JsonSchema(
        val name: String,
        val schema: JsonSchema,
        val strict: Boolean = true
    ) : TextConfig
}
```

如果 `strict = true`，engine 必须在输出不符合 schema 时返回 `StructuredOutputError` 或 `Incomplete`，不能假装成功。

## 16. 错误模型

```kotlin
data class ResponseError(
    val type: ErrorType,
    val code: String,
    val message: String,
    val retryable: Boolean = false,
    val details: JsonValue? = null
)
```

错误类型：

| 类型 | 例子 |
| --- | --- |
| `InvalidRequest` | schema 错误、上下文超限、tool output 缺少 `callId` |
| `ModelUnavailable` | 模型未加载、模型文件不存在、profile 不支持 |
| `BackendError` | decode 失败、sampler 构造失败、native error |
| `StructuredOutputError` | JSON 或 tool call 解析失败 |
| `ToolCallValidationError` | 工具名不存在、参数不符合 schema |
| `Cancelled` | 调用方取消 |
| `Timeout` | prefill 或 decode 超时 |
| `ResourceExhausted` | 内存不足、上下文不足、cache 空间不足 |

## 17. Usage

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

约束：

- `totalTokens = inputTokens + outputTokens`。
- `cachedInputTokens <= inputTokens`。
- `prefillMs` 和 `decodeMs` 应尽量来自 backend 实测。
- cache 命中后仍应记录总 input token，便于比较优化效果。

## 18. Engine 实现要求

`LocalResponseEngine` 至少实现：

```text
LocalResponseRequest
  -> validate request
  -> render messages and tool schemas
  -> split/render prompt segments
  -> tokenize
  -> resolve cache hints
  -> prefill or restore cache
  -> decode
  -> parse output text / JSON / function_call
  -> emit stream events
  -> return LocalResponse
```

实现原则：

- engine 不执行外部工具；
- engine 不解释 `metadata` 的业务含义；
- engine 不把 `previousResponseId` 当成 KV handle；
- engine 可以拒绝不安全或不可执行的 sampling/cache 配置；
- engine 必须支持取消；
- engine 必须记录 usage；
- engine 必须能用 fake backend 做 contract tests。

## 19. 与 `llama.h` 的关系

`llama.h` 是一种 backend 实现，不是 `LocalResponses` 本身。

```text
LocalResponseEngine
  -> LlamaResponseEngine
      -> prompt render
      -> llama_tokenize
      -> llama_decode
      -> llama_sampler
      -> llama_memory / llama_state
```

`llama.h` adapter 应只暴露 `LocalResponses` 对象和事件，不暴露 native pointer。

## 20. 与 llama-server 的关系

llama.cpp server 的 `/v1/responses` 可作为兼容性参考和桌面调试 adapter。

```text
LocalResponsesClient
  -> LlamaServerResponsesAdapter
      -> http://localhost:8080/v1/responses
```

限制：

- llama-server 的实现细节不能决定 DroidLoom 的本地对象模型；
- HTTP adapter 不应成为 Android 端默认主路径；
- 本地 `llama.h` adapter 仍需要直接控制 context、batch、memory 和 state。

## 21. MVP 范围

第一版实现：

1. `LocalResponseRequest` / `LocalResponse` / `LocalResponseEvent`。
2. `MessageItem`、`TextPart`、`JsonPart`。
3. `OutputMessageItem`、`OutputTextPart`。
4. `FunctionCallItem` 和 `FunctionCallOutputItem`。
5. `ToolSpec` + JSON schema 参数校验。
6. token streaming 到 `response.output_text.delta`。
7. function call 参数 streaming 或一次性 done。
8. `CacheHints` metadata 记录和 cache hit/miss 事件。
9. usage 统计。
10. fake engine contract tests。

第一版不实现：

- 多模态输入；
- OpenAI 内置 web/file/code/computer tools 兼容；
- background response；
- HTTP server；
- 真实跨 session KV 持久化；
- backend 外部工具执行。

## 22. Contract Tests

| 测试 | 目标 |
| --- | --- |
| request validation | 缺少 model/input、schema 错误时返回 `InvalidRequest` |
| message text roundtrip | 输入文本，输出 message item 和 `outputText` |
| structured output | JSON schema 输出可校验 |
| tool call parse | 工具调用输出为 `FunctionCallItem` |
| tool output continuation | `FunctionCallOutputItem.callId` 能续接上一轮 |
| stream ordering | event sequence 单调递增，终止事件唯一 |
| cancellation | cancel 后不再发送 delta |
| cache report | cache hit/miss 和 usage 一致 |
| context overflow | 上下文超限时明确失败或 incomplete |
| fake/backend parity | fake engine 和真实 backend 返回同形对象 |

## 23. 参考资料

- OpenAI Responses API: https://platform.openai.com/docs/api-reference/responses
- OpenAI streaming Responses: https://platform.openai.com/docs/api-reference/responses-streaming
- OpenAI function calling guide: https://platform.openai.com/docs/guides/function-calling
- OpenAI conversation state guide: https://platform.openai.com/docs/guides/conversation-state
- OpenAI prompt caching guide: https://platform.openai.com/docs/guides/prompt-caching
- OpenAI Python `ResponseCreateParams`: https://github.com/openai/openai-python/blob/main/src/openai/types/responses/response_create_params.py
- OpenAI Python `Response`: https://github.com/openai/openai-python/blob/main/src/openai/types/responses/response.py
- OpenAI Python `ResponseStreamEvent`: https://github.com/openai/openai-python/blob/main/src/openai/types/responses/response_stream_event.py
- llama.cpp server README: https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md
- DroidLoom [`llama.h` 使用文档](./llama-h-usage.md)

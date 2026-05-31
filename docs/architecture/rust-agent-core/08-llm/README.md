# LLM Provider 与 Adapter

日期：2026-05-30

## 范围

本文描述多供应商 LLM provider 抽象、OpenAI Responses adapter 和 DeepSeek Chat adapter。agent loop 不直接关心厂商，
只面对 request snapshot 和 assistant stream。

## 使用指南

实现 context conversion、模型 registry、provider adapter、OpenAI Responses/DeepSeek 请求、stream event 归一化、
function calling 配对时读本文。

## 使用方案

LLM 模块被 graph 的 provider node 调用，用来把 core request snapshot 转成具体供应商请求。

调用方：

- graph 中的 provider_request node；
- context builder；
- provider wrapper handler；
- 测试中的 mock provider。

输入：

- session replay snapshot；
- 本轮 user message；
- graph run messages；
- system prompt；
- visible tool schemas；
- model id 和 provider config；
- API key、headers、metadata。

输出：

- immutable provider request snapshot；
- normalized stream events；
- assistant `RunMessage` update；
- provider error。

典型调用流程：

1. provider node 调用 `src/llm/context.rs` 构造 provider request snapshot。
2. `src/llm/registry.rs` 根据 model metadata 选择 adapter。
3. request 进入 provider wrapper handler chain。
4. adapter 把 request snapshot 映射成具体供应商请求。
5. provider stream 由 `src/llm/stream.rs` 归一化。
6. assistant builder 根据 stream 构造 assistant message。
7. function call 进入 tool executor；tool message 在下一轮 request 中映射成 `function_call_output`。

不能这样用：

- 不要让 adapter 读取 TurnLoop 或 session mutable state。
- 不要让 provider adapter 执行 tool。
- 不要在 LLM 层保存 session history。
- 不要让 provider-specific 格式泄漏到 tool 层或 user input 层。

## 内部文件架构

### `src/llm/llm.rs`

LLM facade。根据 model/api 找 adapter，调用 stream/complete，把 provider 失败转成统一 error event。

### `src/llm/model.rs`

模型元数据和能力描述。包含 model id、provider、api、base URL、模态、reasoning、function tools、
parallel tool calls、context window、成本和 compatibility flags。

### `src/llm/registry.rs`

管理 adapter 和 model metadata。按 `api` 查 adapter，不按厂商写死分支。

### `src/llm/provider.rs`

定义 provider adapter 契约。adapter 只接收 immutable request snapshot，不读取 runtime mutable state。

### `src/llm/request.rs`

定义 provider request snapshot。包含 model、instructions、input items、tool schemas、reasoning、
include fields、store/previous response/conversation 策略、metadata、headers、api key。

### `src/llm/context.rs`

把 session replay snapshot、本轮 user message、graph run messages、`AgentDefinition` 的 system prompt
和 visible tool schemas 转成 provider request snapshot。

它不读原始用户输入，不写 session tree，不执行 tool。system prompt 只在这里转换成 provider
instructions；factory 不转换 prompt，也不拼 provider context。

### `src/llm/stream.rs`

定义 provider streaming event 到 core run event 的统一协议，不暴露 provider 原始 SSE。

### `src/llm/openai_responses.rs`

OpenAI Responses adapter。负责 instructions、input items、function tools、function call、
`function_call_output`、response id、usage、reasoning metadata 和 stream event 映射。

### `src/llm/deepseek_chat.rs`

DeepSeek Chat adapter。负责把 provider-neutral request 映射到 DeepSeek 的 OpenAI-compatible
`/chat/completions` 请求，处理 `messages`、`tools`、tool call/tool result、`reasoning_content`、
usage 和 chat completion chunk。

API key 只允许从外部注入，例如 `DEEPSEEK_API_KEY` 环境变量或 HTTP 层 header，不写入代码和文档。

## OpenAI Responses 不变量

模型返回 function call 后，下一轮 input 必须包含对应 `function_call_output`。如果 call id 缺 result，
provider 可能拒绝请求或语义错乱。

## DeepSeek Chat 不变量

DeepSeek adapter 使用官方 `https://api.deepseek.com/chat/completions` endpoint，请求 header 使用
`Authorization: Bearer <api key>`。当前默认模型入口使用 `deepseek-v4-flash` 或 `deepseek-v4-pro`。

`ContextBuildInput.metadata` 中带 `deepseek.*` 前缀的字段可以透传到 provider body，例如：

- `deepseek.thinking` -> `thinking`
- `deepseek.reasoning_effort` -> `reasoning_effort`
- `deepseek.response_format` -> `response_format`
- `deepseek.tool_choice` -> `tool_choice`
- `deepseek.stream` -> `stream`
- `deepseek.stream_options` -> `stream_options`

可运行示例：

```bash
$env:DEEPSEEK_API_KEY="<your key>"
cargo run -p agent-core --example deepseek_prepare_request
```

示例先构造 request snapshot；如果进程环境中存在 `DEEPSEEK_API_KEY`，会用 example 内部的轻量 HTTP client
发送一次非流式请求。正式 runtime 的 provider transport 仍属于后续独立层，不放进 core adapter。

## 后续拓展方案

第一版实现 OpenAI Responses 和 DeepSeek Chat 的文本、reasoning 与 function calling request mapping。

第二版增加 image/file input、reasoning include、encrypted reasoning content、response id continuation。

第三版增加 Anthropic、Google、OpenRouter 和更多 OpenAI-compatible adapter。

第四版增加 provider fallback、model routing、cost estimator、rate limit backoff、prompt cache 策略。

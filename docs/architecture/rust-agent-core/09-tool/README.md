# Tool Core、Permission 与执行抽象

日期：2026-05-30

## 范围

本文只描述 tool core 抽象、registry、schema、permission、executor 和 result mapping。
第一版文档不设计具体工具文件，也不规定 `read_file`、`shell`、`edit_file` 等具体实现。

具体工具应作为后续 tool pack 或单独设计文档接入，不进入 core 架构第一版。

## 使用指南

实现 tool 抽象、权限管线、MCP adapter、function call output 配对、tool batch 执行时读本文。

边界判断：

- core 只知道 tool capability，不知道具体业务工具怎么实现。
- registry 根据 `AgentDefinition` 的 tool visibility 声明决定“哪些 tool 直接可见、哪些可搜索、哪些不加载”。
- permission 决定“这次调用能不能执行”。
- executor 负责统一生命周期。
- concrete tool 不应该反向依赖 runtime mutable state。

## 使用方案

tool 模块被 graph 的 execute_tools node 调用，用来执行模型请求的 tool call。

调用方：

- graph 中的 execute_tools node；
- tool search capability；
- provider context builder；
- tool wrapper handler；
- 测试中的 mock tool。

输入：

- assistant message 中的 tool call block；
- call id；
- tool name；
- raw arguments；
- active `AgentDefinition` 的 tool visibility policy；
- permission context；
- tool registry。

输出：

- `role=tool` 的 `RunMessage`；
- tool result block；
- validation error 或 permission decision；
- tool execution event。

典型调用流程：

1. execute_tools node 从 assistant message 中取出 tool call blocks。
2. `src/tool/executor.rs` 根据 tool name 查 registry。
3. `src/tool/schema.rs` 校验 arguments。
4. `tool_call` point hook 和 `src/tool/permissions.rs` 决定 allow、ask、deny 或 rewrite。
5. allow 后进入 `tool_execution` wrapper handler。
6. tool 执行完成后，`src/tool/result.rs` 生成 `role=tool` message。
7. `tool_result` point hook 可脱敏或改写结果。
8. graph 把 tool message 交给下一轮 provider context。

不能这样用：

- 不要在 tool registry 里做权限判断。
- 不要在 tool 层生成 OpenAI `function_call_output`。
- 不要让具体工具依赖 graph/session mutable state。
- 不要在 core 第一版实现具体 coding tools。

## 内部文件架构

### `src/tool/tool.rs`

定义 tool 核心抽象和 metadata。metadata 包括 name、description、input/output schema、capability flags、
loading policy、result policy、interrupt behavior。

`ToolCapabilities` 中的 `read_only + idempotent + !destructive` 组合表示工具可以被 runtime 做安全预执行。
core 只提供这个判定边界，不决定哪个任务一定要预执行；具体概率、阈值和候选参数由 runtime 或示例层根据轨迹统计决定。

### `src/tool/registry.rs`

管理 tool 集合。负责注册、命名空间、冲突检查、deterministic ordering 和 deferred exposure。

tool visibility 在这里解析，不在 factory 里解析：

- direct：常用工具，默认导出 schema 给 provider；
- searchable：不默认导出 schema，只让 tool search 返回候选，选中后再加载；
- hidden：不加载、不搜索、不暴露。

registry 只解析可见性和加载策略，不做权限判断。

### `src/tool/schema.rs`

负责 schema 导出、参数解析、schema validation、tool-local validation 和 structured validation error。

### `src/tool/permissions.rs`

作为 `tool_call` hook chain 里的 core permission handler。支持 allow、ask、deny、passthrough，
允许 decision 携带 updated input。

### `src/tool/executor.rs`

执行 tool batch。流程是定位 tool、schema/validation、触发 `tool_call` hook、处理 ask、
通过 `tool_execution` wrapper handler 执行 tool、触发 `tool_result` hook、交给 `src/tool/result.rs` 生成 tool message、
触发 `tool_execution_end`。

`ToolExecutor::execute_batch_parallel_messages()` 是并行 batch 的 message 主路径。它并行执行多个独立 tool call，并按输入顺序返回 tool result messages，
因此调用方可以直接把这些 messages 继续交给 provider。并行只改变调度，不改变 schema、visibility、
permission 或 result mapping 规则。

### `src/tool/result.rs`

把 tool 内部 result 映射成 `role=tool` 的 `RunMessage`，维护 call/result 配对不变量。

`src/tool/result.rs` 只负责生成 core message，不负责 provider-specific observation 格式。
OpenAI Responses 的 `function_call_output` 等具体格式由 context/LLM provider adapter 转换。

### `src/tool/adapter.rs`

定义外部 tool 来源的适配边界，例如 MCP、插件 tool、远程 tool service。adapter 只把外部能力转成
core tool metadata 和 invocation contract。

## Tool 生命周期

1. provider 返回 tool call block。
2. executor 查 registry。
3. schema 层解析和校验参数。
4. 进入 `tool_call` hook chain，包含 permission guard。
5. allow 后通过 `tool_execution` wrapper handler 执行 tool；ask 等待 TurnLoop 输入；deny/block 生成 error result。
6. 执行结果进入 `tool_result` hook。
7. `src/tool/result.rs` 生成 `role=tool` 的 `RunMessage`。
8. graph 把 tool message 交给下一轮 provider context。

## 后续拓展方案

第一版实现抽象 tool、registry、schema、permission、executor、result mapper，不实现具体业务工具。

第二版定义独立 tool pack 机制，让具体工具作为可插拔包接入。

第三版增加 MCP adapter、远程 tool service、tool sandbox、capability scope。

第四版增加 tool usage analytics、semantic result summarizer、tool marketplace。

Mobilerun-like 示例已经在 runtime 层演示了 tool usage analytics、只读幂等工具预执行和并行 batch；
这些能力仍通过本文件定义的 `ToolMetadata`、`ToolRegistry`、`ToolExecutor` 和 `ToolResult` 边界接入。

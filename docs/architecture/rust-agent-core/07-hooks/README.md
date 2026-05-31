# Hook、Handler 与事件订阅

日期：2026-05-30

## 范围

本文定义三个概念：

- hook：生命周期切点；
- handler：挂在 hook 上的处理函数；
- event subscriber：只观察事件，不改变行为。

第一版对外只暴露 hook + handler。handler 内部分两种形态：

- point handler：点式处理，输入 payload，返回 decision；
- wrapper handler：包住一段执行，拿到 `next`，可以重试、降级、恢复或替换执行。

可恢复错误、重试、降级、把错误转成给模型看的 message，都属于 wrapper handler。

## 使用指南

实现 extension、审计、权限、动态上下文注入、provider request 改写、错误恢复、session commit guard
时读本文。

PI 对外主要是 `pi.on(event, handler)`。多数 handler 是点式处理；其中 `tool_result` 这类事件按加载顺序
链式改写结果，表现得像 wrapper。`user_bash` 可以替换或包住执行 backend，bash tool 的 spawn hook
也属于执行前 wrapper。本文沿用 hook + handler 的外部模型，把 wrapper 作为 handler 的一种形态。

使用规则：

- 只观察、展示、打日志，用 event subscriber。
- 需要在某个生命周期点改写 payload，用 point handler。
- 需要包住 provider/tool/node 执行、捕获 error、重试或恢复，用 wrapper handler。
- extension handler 根据注册的 hook 类型，适配成 point handler 或 wrapper handler。

## 使用方案

hook 模块被 runtime 各层调用，用来开放扩展点、审计点和执行包装点。

调用方：

- `src/input/user_input.rs`：触发 input point hook。
- TurnLoop：触发 before_agent_start point hook。
- `src/llm/context.rs`：触发 context point hook。
- LLM provider：触发 provider point hook，并经过 provider wrapper hook。
- tool executor：触发 tool point hook，并经过 tool wrapper hook。
- graph runner：执行 node 时经过 node wrapper hook。
- session store：触发 before_session_commit point hook。

输入：

- hook name；
- hook payload；
- handler registry；
- handler execution context；
- wrapper hook 的 `next`。

输出：

- point decision：continue、rewrite、block、stop、emit；
- wrapper result：continue、rewrite、retry、recover、stop、fail；
- trace/event。

典型调用流程：

1. runtime 到达某个 hook 点。
2. `src/hook/hook.rs` 构造 payload。
3. `src/hook/handler.rs` 查找该 hook 注册的 handlers。
4. point hook 顺序执行 handlers，并合并 decision。
5. wrapper hook 构造 `next`，让 handlers 包住下游执行。
6. handler 返回 rewrite/retry/recover/stop/fail 后，runtime 按结果继续推进。
7. 纯观察逻辑只接收 event，不进入 decision chain。

不能这样用：

- 不要让 handler 直接持有 runtime mutable reference。
- 不要用 point handler 做 retry/recover；这属于 wrapper handler。
- 不要把 event subscriber 当成可决策 hook。
- 不要把 approval 的等待状态放进通用 hook decision。

## 内部文件架构

### `src/hook/hook.rs`

定义 hook 名称、payload、decision 和触发入口。

hook 本身不执行业务逻辑，只负责声明“这里可以挂 handler”。

### `src/hook/handler.rs`

定义 handler 注册、handler kind、执行顺序、短路规则、`next` 调用约定和 trace 记录。

handler kind：

- point：输入 hook payload，返回 decision。不拿 `next`。
- wrapper：输入 request 和 `next`，返回 result。可以不调用 `next`、调用一次或调用多次。

默认顺序：core pre-guard -> extension handlers -> definition handlers -> per-run handlers -> core post-guard。

## 第一版 Point Hook

### `input`

用户输入已经被包装成 `RunMessage` 后触发。可用于输入审计、改写或阻断。

### `before_agent_start`

TurnLoop 准备启动某个 `Agent` 前触发。可用于阻断本轮 run，或改写 run options。

### `context`

context builder 生成 provider context draft 时触发。可用于注入额外上下文、裁剪 message 或隐藏诊断内容。

### `before_provider_request`

provider request snapshot 发送前触发。可用于最终审计、改写 model/request 参数或阻断请求。

### `after_provider_response`

provider response 被 adapter 归一化成 core message 后触发。可用于安全过滤或结构修正。

### `tool_call`

tool 执行前触发。权限判断、参数改写和 deny 都放这里。

### `tool_result`

tool result 被包装成 `role=tool` 的 `RunMessage` 后触发。可用于脱敏、改写或终止本轮 graph。

### `before_session_commit`

finalized `RunMessage` 转成 `SessionEntry` 并 append 前触发。可用于脱敏、阻断持久化或改写 entry。

## 第一版 Wrapper Hook

### `provider_request`

包住 LLM provider 调用。用于超时、重试、fallback、限流处理、recoverable provider error 转 message。

### `tool_execution`

包住 tool invoke。用于 timeout、sandbox、retry、recoverable tool error 转 tool/diagnostic message。

### `node_execution`

包住 graph node exec。用于 node 级 retry、错误分类、recoverable node error 转 message。

### error recovery

不是独立 hook 名称，而是 wrapper handler 的常见行为。它挂在 `provider_request`、`tool_execution` 或
`node_execution` 上，根据 error 类型决定：

- retry；
- fallback；
- 返回给模型看的 `RunMessage`；
- stop/fail。

fatal error 不恢复，直接进入失败路径。

## 不作为 Hook 的事件

这些点第一版只作为 event subscriber 输入：

- `session_start`
- `session_shutdown`
- `agent_start`
- `agent_end`
- `turn_start`
- `turn_end`
- `node_start`
- `node_end`
- `message_update`
- `message_finalized`
- `tool_execution_start`
- `tool_execution_end`

`resources_discover` 不放在 TurnLoop hook 里，属于 extension/tool adapter 启动期注册流程。

## Point Decision

- continue：继续。
- rewrite：返回新 payload。
- block：阻断并给 reason。
- stop：结束当前 run 或 session。
- emit：追加 diagnostic/event 请求。

point handler 不负责 retry/recover。retry/recover 属于 wrapper handler。

## Wrapper Result

- continue：返回下游结果。
- rewrite：返回改写后的 request 或 response。
- retry：重新调用 `next`。
- recover：返回一个或多个 `RunMessage`，交给 graph 继续下一步。
- stop：停止当前 run。
- fail：返回 fatal error。

没有通用 `suspend` decision。需要用户确认的 tool 权限由 `src/tool/permissions.rs` 返回 permission decision，
不通过 hook handler 状态表达。

## 与模块的映射

- `src/input/user_input.rs` 构建 user message 后触发 `input` point handler。
- `src/session/turn_loop.rs` 启动 agent 前触发 `before_agent_start` point handler。
- `src/llm/context.rs` 生成 provider context draft 时触发 `context` point handler。
- `src/llm/request.rs` 发送前触发 `before_provider_request` point handler。
- `src/llm/provider.rs` 的 provider 调用由 `provider_request` wrapper handler 包住。
- `src/llm/provider.rs` 归一化 response 后触发 `after_provider_response` point handler。
- `src/tool/executor.rs` 执行前触发 `tool_call` point handler，实际 invoke 由 `tool_execution` wrapper handler 包住。
- `src/tool/executor.rs` 包装 result 后触发 `tool_result` point handler。
- `src/graph/runner.rs` 执行 node exec 时由 `node_execution` wrapper handler 包住。
- `src/session/store.rs` append 前触发 `before_session_commit` point handler。

## 后续拓展方案

第一版实现 hook 名称、payload、decision、handler 注册、handler kind、执行顺序和 trace。

第二版增加 extension manifest、资源发现、workspace 启用策略、handler priority。

第三版增加远程 extension、沙箱、capability scope、hook timeout、extension state persistence。

第四版增加 hook handler replay/debug UI。

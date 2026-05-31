# 实现路线与待定问题

日期：2026-05-30

## 范围

本文给出新的实现切片。切片顺序围绕四个边界展开：user message、run message/content block、session replay、
tool core 抽象。

## 使用指南

写代码前用本文确认当前迭代的闭环。不要先实现具体工具；先把 core tool lifecycle 和 function call
配对跑通。

## 使用方案

本文被项目维护者用来决定“下一步实现什么”，不是运行时代码入口。

调用方：

- 开发者；
- 测试规划；
- 后续代码审查。

输入：

- 当前已完成的模块；
- 当前 smoke test 是否通过；
- 新需求所在的模块边界。

输出：

- 下一批实现文件；
- 本阶段不做的能力列表；
- smoke test 闭环。

典型使用流程：

1. 开始实现前，先选择一个第一版切片。
2. 完成该切片的类型、最小行为和单元测试。
3. 跑第一个 smoke test：text user message 到 assistant session entry。
4. 跑第二个 smoke test：function call 到 mock tool result 再到 assistant final。
5. 新需求如果落在“第一版不做”，记录到后续阶段，不插入当前闭环。

不能这样用：

- 不要把 roadmap 当模块设计细节来源。
- 不要因为 roadmap 提到某个文件，就把其他模块逻辑塞进该文件。
- 不要在第一版 smoke test 前实现具体工具、复杂 TUI 或后台 agent。

## 第一版实现切片

1. 写 `src/message/content_block.rs`、`src/message/run_message.rs` 和 `src/input/user_input.rs`，把用户输入按模态包成 user message。
2. 写 `src/message/assistant_builder.rs` 和 `src/core/error.rs`。
3. 写 `src/session/turn_loop.rs`、`src/session/entry.rs`、`src/session/tree.rs`、`src/session/store.rs`、`src/session/replay.rs`，
   先支持会话级 user message buffer 和最小 JSONL tree。
4. 写 `src/hook/hook.rs` 和 `src/hook/handler.rs`，把 `input`、`context`、`tool_call` 等 point hook 插进生命周期，
   并让 provider/tool/node 执行经过 wrapper handler。
5. 写 `src/agent/definition.rs`、`src/agent/factory.rs`，只做 name、system prompt、tool visibility
   的配置声明和 `Agent` 装配。
6. 写 `src/llm/model.rs`、`src/llm/registry.rs`、`src/llm/provider.rs`、`src/llm/request.rs`、`src/llm/stream.rs`。
7. 写 `src/llm/openai_responses.rs`，支持 Responses 文本和 function calling。
8. 写 `src/llm/context.rs`，把 session replay snapshot 和 graph messages 转成 Responses input。
9. 写 `src/graph/graph.rs`、`src/graph/node.rs`、`src/graph/edge.rs`、`src/graph/state.rs`、`src/graph/runner.rs`、
   `src/graph/templates.rs`。
10. 在 `src/agent/agent.rs` 中实现 `Agent` lifecycle 和 `Agent.run()`，只管理单个 agent 生命周期内部状态；
    session entry 写入由 session 模块处理。
11. 写 `src/tool/tool.rs`、`src/tool/registry.rs`、`src/tool/schema.rs`、`src/tool/permissions.rs`、`src/tool/executor.rs`、
    `src/tool/result.rs`，由 tool 层解析 direct/searchable/hidden，只做抽象 tool core，不做具体工具。
12. 写 `agent-cli/main.rs` 和 `agent-cli/config.rs` 做 smoke test。

## 第一版不做

- 具体工具实现；
- subprocess shell；
- 文件编辑工具；
- 完整 MCP tool pack；
- 复杂 context compaction；
- 后台 multi-agent；
- provider fallback；
- 完整 TUI；
- 动态插件市场。

## Smoke Test 闭环

第一个闭环：

1. CLI 提交文本，`src/input/user_input.rs` 包成 text block 和 user message。
2. TurnLoop 消费 user message。
3. graph 构造 context。
4. OpenAI Responses 返回 assistant。
5. assistant run message finalize。
6. session 模块写 message entry。

第二个闭环：

1. 使用一个测试用 mock tool capability，不实现具体业务工具。
2. assistant 返回 function call。
3. tool executor 生成 mock result。
4. 下一轮 Responses input 包含 `function_call_output`。
5. assistant final message 写入 session entry。

## 待定问题

- `RunMessage` 和 `SessionEntry` 的转换是否需要从 `src/session/entry.rs` 拆成独立 writer。
- 对外 streaming API 用 channel、stream wrapper 还是 subscriber callback。
- tool schema validation 选 runtime JSON schema 库还是 typed helper。
- OpenAI Responses 是否启用 `previous_response_id`。
- child agent fork context 的权限确认粒度。

## 后续拓展方案

第二阶段增加具体 coding tool pack，但作为 core 外部模块接入。

第三阶段增加 foreground child agent、fork context、child session link、supervisor graph。

第四阶段增加 TUI/HTTP server、插件动态加载、trace viewer、remote session store、background resume。

# RunMessage 与 ContentBlock 规范

日期：2026-05-30

## 范围

本文只描述 graph run 内 message 和 content block 的定义、包装与构造规则。

本文不描述 node 调度、edge 判定、event stream、GraphState 写入、session 持久化、replay 或 compact。
finalized message 如何提交成 session entry，放在
[06-session](../06-session/README.md)。

## 使用指南

实现 `src/message/run_message.rs`、`src/message/content_block.rs`、`src/message/assistant_builder.rs` 时读本文。

边界判断：

- 原始用户输入到 user message 的转换见 [02-user-message](../02-user-message/README.md)。
- 本文只定义 user/assistant/tool/diagnostic message 的运行期形态。
- message builder 只负责包装 message，不写 session tree。
- tool result 如何包装成 tool message 见 [09-tool](../09-tool/README.md)。

## 使用方案

message 模块被 user input、provider adapter、tool result mapper 和 graph runner 共同使用。

调用方：

- `src/input/user_input.rs`：构造 user message。
- `src/message/assistant_builder.rs`：从 provider stream 构造 assistant message。
- `src/tool/result.rs`：构造 tool message。
- graph runner：消费 message 并触发 edge 判定。
- session 模块：把 finalized message 转成 session entry。

输入：

- role；
- typed content blocks；
- source node id；
- provider ids；
- streaming status；
- call id、usage refs、metadata。

输出：

- `RunMessage`；
- message update；
- append-friendly content block。

典型调用流程：

1. 调用方选择 role。
2. 调用方按内容模态构造 content blocks。
3. streaming 场景下，builder 保持 block id 稳定并追加 delta。
4. tool call block 生成后保留 call id。
5. tool result block 使用同一个 call id 配对。
6. message finalize 后，调用方交给 graph runner 或 session 模块。

不能这样用：

- 不要把 system prompt 放进 `RunMessage`。
- 不要让 `RunMessage` 持有 session parent_id。
- 不要在 builder 里执行 tool、判断 edge 或写 session。
- 不要用 untyped 字符串表示 tool call/result。

## 内部文件架构

### `src/message/run_message.rs`

定义单轮 graph run 内部消息模型。

`RunMessage` 至少表达：

- role：user、assistant、tool、diagnostic；
- content blocks；
- source node id；
- provider ids；
- streaming status；
- usage refs；
- trace refs；
- provider-specific metadata。

`RunMessage` 不带 session parent_id，不知道 active leaf，不负责持久化提交。

system prompt 不属于 run message。`AgentDefinition` 保存 system prompt，context/provider request
层负责把它转换成 provider instructions。

### `src/message/content_block.rs`

定义 typed content block：

- text；
- reasoning；
- tool call；
- tool result；
- file reference；
- image reference；
- audio reference；
- diagnostic；
- custom block。

block 必须 append-friendly。streaming 中还没完成的 block 可以持续追加 delta，但 block id 和 call id
必须稳定。

tool call 和 tool result 必须通过 call id 配对。具体 tool 执行和 result 包装归 tool 层。

### `src/message/assistant_builder.rs`

负责把 provider streaming 片段包装成 assistant `RunMessage`。

它维护：

- 当前 assistant message；
- 当前 text/reasoning/tool call block；
- block delta；
- provider ids；
- 完成状态。

builder 的输出仍然只是 `RunMessage` 或 message update。调用方负责消费这些 message。

## 包装规则

用户消息：

- 由 `src/input/user_input.rs` 按模态包装成 `role=user` 的 `RunMessage`。
- 文本、图片、文件、音频都落成对应 content block。

assistant 消息：

- 由 provider stream 经 `src/message/assistant_builder.rs` 包装。
- reasoning、text、tool call 分别落成独立 block。

tool 消息：

- 由 tool 层把 tool result 包装成 `role=tool` 的 `RunMessage`。
- `tool_result` block 必须保留对应 tool call id。

diagnostic 消息：

- 用于错误、警告、调试信息或 hook handler 产生的可见诊断。
- diagnostic message 是否进入 provider context，由 context 层决定。

## 后续拓展方案

第一版支持 text、reasoning、function tool call/result、diagnostic。

第二版支持 image/file/audio reference blocks 和 provider-specific metadata 降级。

第三版支持 partial message recovery：run 中断时补齐 aborted assistant message 和 cancelled tool message。

第四版增加 message block timeline 的 debug view。

# 用户输入到 Message

日期：2026-05-30

## 范围

本文只处理一件事：把用户输入包装成 `role=user` 的 message，并按输入模态选择对应的
`ContentBlock`。

这里不设计额外入口对象、输入队列或命令路由。控制指令、stop、approval 这些能力先走
CLI、driver 或 TurnLoop 的显式方法，不混进用户消息模型。

## 使用指南

实现用户发消息入口时读本文。入口逻辑应该尽量薄：

1. 接收原始用户输入。
2. 判断模态。
3. 生成 content blocks。
4. 包成 user message。
5. 交给 TurnLoop 启动一轮 graph run。

## 使用方案

`src/input/user_input.rs` 是外层应用进入 core 的第一道转换 API。

调用方：

- CLI 输入循环；
- HTTP 或测试入口；
- 后续 TUI 的消息提交动作。

输入：

- 原始文本；
- 文件、图片、音频等引用；
- 用户提交时的模态顺序。

输出：

- 一个 `RunMessage(role=user)`；
- 其中 content blocks 按用户提交顺序排列。

典型调用流程：

1. 外层应用收到用户提交。
2. `src/input/user_input.rs` 根据模态选择 `ContentBlock` 类型。
3. 文本直接包装成 text block。
4. 文件、图片、音频先包装成 reference block，不在这里读取、解析或上传。
5. 多模态输入合并成一个 user message。
6. 外层应用把 user message 交给 TurnLoop。

不能这样用：

- 不要在 `src/input/user_input.rs` 里做排队、抢占、stop、approval。
- 不要在这里写 session。
- 不要在这里构造 provider request。
- 不要新增 `InputItem` 一类入口对象。

## 内部文件架构

### `src/input/user_input.rs`

很薄的转换边界。它只负责把 CLI/API 收到的用户输入转成 user message。

它不排队、不抢占、不处理命令、不写 session、不构造 provider request。

### `src/message/run_message.rs`

定义 graph run 内使用的 message。用户输入转换后的结果就是一个 `role=user` 的 `RunMessage`。

### `src/message/content_block.rs`

按模态生成内容块：

- 文本输入 -> text block；
- 图片输入 -> image block 或 image reference block；
- 文件输入 -> file reference block；
- 音频输入 -> audio block 或 audio reference block；
- 多模态输入 -> 多个 blocks，按用户提交顺序排列。

## 生命周期

`用户输入 -> ContentBlock[] -> RunMessage(role=user) -> TurnLoop -> Graph`

TurnLoop 只需要拿到 user message。是否排队、是否立即运行、是否 stop，是 TurnLoop/driver 的职责，
不是用户输入转换层的职责。

## 后续拓展方案

第一版只支持文本输入。

第二版支持文件和图片引用。

第三版支持音频、多文件、多模态混合输入。

如果后续需要 slash command 或 approval，也优先作为 CLI/driver control API 设计，不默认塞回
user message 入口。

# TurnLoop、Session Tree、持久化与 Replay

日期：2026-05-30

## 范围

本文只描述会话级 TurnLoop、session tree、持久化、branch、resume、replay 和 compact。
它不定义用户输入入口，也不定义 graph run 内部 message 的 streaming 细节。

session tree 是历史权威。graph run 内的 message 只有 finalize 后才会被 session 模块转换并提交成
session entry。

## 使用指南

实现 `src/session/turn_loop.rs`、session persistence、resume、branch、compact、tree view、provider context replay
时读本文。

边界判断：

- session 不保存原始 input queue。
- session 不保存 provider delta。
- session 只保存 finalized semantic entries。
- replay 从 active leaf 重建 provider context snapshot，但不恢复某次 graph run 的临时 builder。

## 使用方案

session 模块被 TurnLoop 和 context builder 使用，用来保存历史和重建下一轮 provider context。

调用方：

- TurnLoop：管理会话级 user message buffer 和启动 agent run。
- graph/agent run 完成后的 session writer 流程。
- context builder：请求当前 active branch 的 replay snapshot。
- CLI/TUI：读取 tree view、branch、resume 信息。

输入：

- user message；
- finalized `RunMessage`；
- session metadata；
- branch/label/compact 请求；
- JSONL store path 或 in-memory store。

输出：

- append-only `SessionEntry`；
- active leaf；
- branch path；
- replay snapshot；
- session tree view。

典型调用流程：

1. TurnLoop 接收 user message，决定何时启动 agent run。
2. agent run finalize 后，session 模块接收 finalized messages。
3. `src/session/entry.rs` 把 finalized messages 转换成 message entries。
4. `src/session/store.rs` append JSONL。
5. `src/session/tree.rs` 更新 active leaf 和内存索引。
6. 下一轮 run 前，`src/session/replay.rs` 沿 active leaf 回放 branch。
7. context builder 使用 replay snapshot 构造 provider context。

不能这样用：

- 不要保存 provider delta。
- 不要保存 graph run 内部临时 builder。
- 不要在 session replay 中重新执行 tool 或 provider。
- 不要让 session 模块解析原始用户输入。

## TurnLoop 边界

### `src/session/turn_loop.rs`

`src/session/turn_loop.rs` 是会话级生命周期控制器。它接收 [02-user-message](../02-user-message/README.md)
产出的 user message，并负责：

- user message buffer；
- GenInput；
- prepare turn；
- preempt；
- stop；
- idle wakeup；
- late messages。

它不解析原始用户输入，不执行 provider，不执行 tool，不展开 graph 节点，也不保存 provider delta。
当需要启动 agent run 时，它把本轮 user message 交给 `Agent` 或 graph driver。

## 内部文件架构

### `src/session/entry.rs`

定义 append-only JSONL entry。每个非 header entry 有 id、parent_id、timestamp，通过 parent 指针成树。

entry 类型包括 header、message、model change、reasoning change、compaction、branch summary、label、
session info、custom entry、custom message entry。

`src/session/entry.rs` 还定义 finalized `RunMessage` 到 message entry 的转换形状。转换只发生在 run
边界，不在 message builder 内发生。

### `src/session/tree.rs`

维护内存树索引：by_id、children、active leaf、labels、latest session info。负责 append、branch、
reset leaf、get branch、get tree、tree filter。

### `src/session/store.rs`

负责 JSONL I/O。包括 create、append、flush、open、bad line recovery、version migration、in-memory
session、fork/clone、session list。

finalized message 的提交由 session 模块调用 `append` 完成。graph/message 层不直接写 store。

### `src/session/replay.rs`

沿 active leaf 的 parent_id 回放当前 branch，恢复 model、reasoning、labels、session info，并把
message/summary/custom message 转成 context builder 可消费的 replay snapshot。

### `src/session/compaction.rs`

定义 compact 策略和 compaction entry 的生成边界。compact 只处理 session 级历史，不直接改 graph run
内正在 streaming 的 message。

## Replay 逻辑

1. 从 active leaf 走到 root，得到 branch path。
2. 顺序扫描 path，恢复派生状态。
3. 遇到 compaction entry，生成 summary snapshot，再从 first_kept_entry 继续。
4. branch summary entry 转成 provider-visible summary。
5. custom entry 交给对应 handler 恢复状态，默认不进 provider context。
6. message entry 保留 tool call/result call id，保证下一轮 Responses input 可配对。

## 后续拓展方案

第一版实现 in-memory tree、JSONL append-only、message/model/reasoning entry、active branch replay。

第二版加入 label、session info、bad line recovery、fork/clone、tree view filter。

第三版加入 compaction、branch summary、custom entry、trace log 分离。

第四版加入 session diff、跨 session merge、远程 session store、加密 session 和 replay debugger。

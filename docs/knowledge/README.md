# 知识库

这个目录用于沉淀 DroidLoom 的论文、系统设计资料、产品能力边界和实验笔记。

当前入口：

- [论文索引](./paper-index.md)：按优先级和主题整理与 DroidLoom 相关的论文。
- [Repo 索引](./repo-index.md)：按优先级和模块整理值得参考的开源项目。
- [llama.h 使用文档](./llama-h-usage.md)：基于 `llama.cpp` C API 构建 DroidLoom adapter 的实现参考。
- [Local Responses 抽象设计](./local-responses-abstraction.md)：参考 OpenAI Responses API，为 DroidLoom 定义本地 Agent/LLM 契约。
- [论文笔记模板](./paper-note-template.md)：后续深入读单篇论文时使用。

## 分类标签

论文索引使用以下标签：

| 标签 | 含义 | DroidLoom 关联 |
| --- | --- | --- |
| `mobile-agent` | 手机或 GUI Agent | 屏幕观察、动作空间、Agent loop |
| `benchmark` | 可复现评测环境 | AndroidWorld-style harness、任务初始化、success check |
| `grounding` | GUI 元素定位 | 截图/OCR fallback、坐标动作 verifier |
| `workflow` | Agent 工作流和结构化程序 | Workflow IR、调度、工具调用 |
| `kv-cache` | KV Cache 和上下文复用 | Prompt segment、KV 生命周期、prefix cache |
| `compiler` | 编译器和优化 pass | IR、pass manager、cost model |
| `on-device` | 端侧推理 | MLC/llama.cpp、移动 SoC、内存规划 |
| `tool-use` | 工具调用 | Intent、Shortcut、Notification、Accessibility action |
| `safety` | 安全和确认机制 | Guard、Confirm、TakeOver、Trace |
| `repo` | 开源项目参考 | observation/action、benchmark、KV/runtime、workflow |
| `api-contract` | Agent/LLM 契约 | Local Responses、stream event、tool call、usage |

## 使用方式

1. 做架构设计时，先看 [论文索引](./paper-index.md) 和 [Repo 索引](./repo-index.md) 的 P0。
2. 实现某个模块前，按标签筛选相关论文和 repo。
3. 深读论文时，用 [论文笔记模板](./paper-note-template.md) 新建单篇笔记。
4. 如果论文或 repo 影响架构决策，补充 ADR，而不是只留在知识库里。

## 单篇笔记路径规范

未来单篇笔记放在：

```text
docs/knowledge/papers/YYYY-<short-slug>.md
```

示例：

```text
docs/knowledge/papers/2024-androidworld.md
docs/knowledge/papers/2023-pagedattention.md
docs/knowledge/papers/2018-tvm.md
```

# langchain-rs 使用文档

日期：2026-05-30

## 1. 命名说明

Rust 生态里有两个名字相近的 crate：

- `langchain-rust`：当前活跃版本为 `4.6.0`，Rust import 路径是 `langchain_rust`。
- `langchain_rs`：当前是很早期的 `0.0.x` 包，不建议作为项目主依赖。

本文中的 `langchain-rs` 指 `langchain-rust` crate。

参考来源：

- crates.io: <https://crates.io/crates/langchain-rust>
- docs.rs: <https://docs.rs/langchain-rust/latest/langchain_rust/>
- GitHub: <https://github.com/Abraxas-365/langchain-rust>

## 2. 适合场景

`langchain-rust` 适合快速验证 Rust LLM 应用：

- 调用 OpenAI、Azure OpenAI、Ollama、Anthropic Claude 等模型。
- 编写 prompt template 和 LLM chain。
- 做简单 conversation chain。
- 做 agent + tool 调用。
- 做 embedding、vector store、RAG。
- 接入 Qdrant、Postgres、Sqlite、SurrealDB 等向量存储。

如果你要做类 PIAgent 的 `agent.state`、生命周期事件、`steer/followUp`、跨平台 tool bridge，
不要直接把 `langchain-rust` 当最终 agent runtime。更稳妥的方式是：

```text
自研 agent-core
  -> 使用 langchain-rust 的 LLM、chain、tool、embedding、vector store 能力
```

## 3. 安装

创建 demo：

```bash
cargo new langchain-rs-demo
cd langchain-rs-demo
```

添加依赖：

```bash
cargo add langchain-rust
cargo add tokio --features full
cargo add serde_json
cargo add async-trait
cargo add futures
```

如果要用向量库，再按需打开 feature：

```bash
cargo add langchain-rust --features qdrant
cargo add langchain-rust --features postgres
cargo add langchain-rust --features sqlite
cargo add langchain-rust --features surrealdb
cargo add langchain-rust --features ollama
```

OpenAI 默认从环境变量读取 API key：

```powershell
$env:OPENAI_API_KEY="your_api_key"
```

Linux/macOS：

```bash
export OPENAI_API_KEY="your_api_key"
```

## 4. 最快搭建一个可运行 Agent

目标：先跑通“模型能调用工具”的 smoke test，不先做完整 `agent.state`。

创建项目：

```bash
cargo new agent-smoke
cd agent-smoke
cargo add langchain-rust
cargo add tokio --features full
cargo add async-trait serde_json
```

设置 API key：

```powershell
$env:OPENAI_API_KEY="your_api_key"
```

Linux/macOS：

```bash
export OPENAI_API_KEY="your_api_key"
```

写 `src/main.rs`：

```rust
use std::{error::Error, sync::Arc};

use async_trait::async_trait;
use langchain_rust::{
    agent::{AgentExecutor, OpenAiToolAgentBuilder},
    chain::{options::ChainCallOptions, Chain},
    llm::openai::OpenAI,
    memory::SimpleMemory,
    prompt_args,
    tools::Tool,
};
use serde_json::Value;

struct DateTool;

#[async_trait]
impl Tool for DateTool {
    fn name(&self) -> String {
        "Date".to_string()
    }

    fn description(&self) -> String {
        "Useful when you need to know today's date.".to_string()
    }

    async fn run(&self, _input: Value) -> Result<String, Box<dyn Error>> {
        Ok("2026-05-30".to_string())
    }
}

#[tokio::main]
async fn main() {
    let llm = OpenAI::default();
    let memory = SimpleMemory::new();

    let agent = OpenAiToolAgentBuilder::new()
        .tools(&[Arc::new(DateTool)])
        .options(ChainCallOptions::new().with_max_tokens(1000))
        .build(llm)
        .unwrap();

    let executor = AgentExecutor::from_agent(agent).with_memory(memory.into());

    let result = executor
        .invoke(prompt_args! {
            "input" => "What date is today? Use the available tool.",
        })
        .await
        .unwrap();

    println!("{}", result.replace('\n', " "));
}
```

运行：

```bash
cargo run
```

这个例子验证了三件事：

- OpenAI 模型调用可用。
- agent 能看到工具描述。
- agent 能调用 Rust tool，并把 tool result 整合进最终回答。

下一步再把 `DateTool` 换成自己的工具，例如：

```text
ObserveScreenTool
TapTool
TypeTextTool
OpenUrlTool
ReadFileTool
```

第一步不要接危险工具，比如 shell command。先用无副作用工具把 agent loop 跑通。

## 5. 最小模型调用

```rust
use langchain_rust::{
    language_models::llm::LLM,
    llm::openai::OpenAI,
};

#[tokio::main]
async fn main() {
    let open_ai = OpenAI::default();
    let response = open_ai.invoke("What is Rust?").await.unwrap();
    println!("{response}");
}
```

如果不想用环境变量，可以手动传配置：

```rust
use langchain_rust::{
    language_models::llm::LLM,
    llm::{openai::OpenAI, OpenAIConfig},
};

#[tokio::main]
async fn main() {
    let open_ai = OpenAI::default().with_config(
        OpenAIConfig::default()
            .with_api_base("https://api.openai.com/v1")
            .with_api_key("your_api_key"),
    );

    let response = open_ai.invoke("hola").await.unwrap();
    println!("{response}");
}
```

## 6. Prompt Template 和 LLMChain

`LLMChain` 用于把 prompt template 和模型组合起来。

```rust
use langchain_rust::{
    chain::{Chain, LLMChainBuilder},
    fmt_message, fmt_template,
    llm::openai::{OpenAI, OpenAIModel},
    message_formatter,
    prompt::HumanMessagePromptTemplate,
    prompt_args,
    schemas::messages::Message,
    template_fstring,
};

#[tokio::main]
async fn main() {
    let open_ai = OpenAI::default().with_model(OpenAIModel::Gpt4oMini.to_string());

    let prompt = message_formatter![
        fmt_message!(Message::new_system_message(
            "You are a concise technical documentation writer."
        )),
        fmt_template!(HumanMessagePromptTemplate::new(template_fstring!(
            "{input}",
            "input"
        )))
    ];

    let chain = LLMChainBuilder::new()
        .prompt(prompt)
        .llm(open_ai)
        .build()
        .unwrap();

    let result = chain
        .invoke(prompt_args! {
            "input" => "Explain ownership in Rust in one paragraph.",
        })
        .await
        .unwrap();

    println!("{result:?}");
}
```

## 7. 带历史消息的 Chain

`fmt_placeholder!("history")` 可以把历史消息插入 prompt。

```rust
use langchain_rust::{
    chain::{Chain, LLMChainBuilder},
    fmt_message, fmt_placeholder, fmt_template,
    llm::openai::OpenAI,
    message_formatter,
    prompt::HumanMessagePromptTemplate,
    prompt_args,
    schemas::messages::Message,
    template_fstring,
};

#[tokio::main]
async fn main() {
    let open_ai = OpenAI::default();

    let prompt = message_formatter![
        fmt_message!(Message::new_system_message(
            "You are a helpful assistant."
        )),
        fmt_placeholder!("history"),
        fmt_template!(HumanMessagePromptTemplate::new(template_fstring!(
            "{input}",
            "input"
        ))),
    ];

    let chain = LLMChainBuilder::new()
        .prompt(prompt)
        .llm(open_ai)
        .build()
        .unwrap();

    let result = chain
        .invoke(prompt_args! {
            "input" => "What is my name?",
            "history" => vec![
                Message::new_human_message("My name is Luis."),
                Message::new_ai_message("Hi Luis."),
            ],
        })
        .await
        .unwrap();

    println!("{result:?}");
}
```

## 8. Streaming

Chain 支持 streaming。每个 chunk 通过 stream 返回。

```rust
use futures::StreamExt;
use langchain_rust::{
    chain::{Chain, LLMChainBuilder},
    fmt_message, fmt_template,
    llm::openai::OpenAI,
    message_formatter,
    prompt::HumanMessagePromptTemplate,
    prompt_args,
    schemas::messages::Message,
    template_fstring,
};

#[tokio::main]
async fn main() {
    let open_ai = OpenAI::default();

    let prompt = message_formatter![
        fmt_message!(Message::new_system_message(
            "You are a concise assistant."
        )),
        fmt_template!(HumanMessagePromptTemplate::new(template_fstring!(
            "{input}",
            "input"
        )))
    ];

    let chain = LLMChainBuilder::new()
        .prompt(prompt)
        .llm(open_ai)
        .build()
        .unwrap();

    let mut stream = chain
        .stream(prompt_args! {
            "input" => "Write three bullets about Rust async.",
        })
        .await
        .unwrap();

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(value) => value.to_stdout().unwrap(),
            Err(err) => panic!("stream error: {err:?}"),
        }
    }
}
```

## 9. Agent 和 Tool

`langchain-rust` 提供 `OpenAiToolAgentBuilder` 和 `AgentExecutor`。
工具实现 `tools::Tool` trait。

```rust
use std::{error::Error, sync::Arc};

use async_trait::async_trait;
use langchain_rust::{
    agent::{AgentExecutor, OpenAiToolAgentBuilder},
    chain::{options::ChainCallOptions, Chain},
    llm::openai::OpenAI,
    memory::SimpleMemory,
    prompt_args,
    tools::Tool,
};
use serde_json::Value;

struct DateTool;

#[async_trait]
impl Tool for DateTool {
    fn name(&self) -> String {
        "Date".to_string()
    }

    fn description(&self) -> String {
        "Useful when you need to get the current date.".to_string()
    }

    async fn run(&self, _input: Value) -> Result<String, Box<dyn Error>> {
        Ok("2026-05-30".to_string())
    }
}

#[tokio::main]
async fn main() {
    let llm = OpenAI::default();
    let memory = SimpleMemory::new();

    let agent = OpenAiToolAgentBuilder::new()
        .tools(&[Arc::new(DateTool)])
        .options(ChainCallOptions::new().with_max_tokens(1000))
        .build(llm)
        .unwrap();

    let executor = AgentExecutor::from_agent(agent).with_memory(memory.into());

    let result = executor
        .invoke(prompt_args! {
            "input" => "What date is today?",
        })
        .await
        .unwrap();

    println!("{}", result.replace('\n', " "));
}
```

内置 tools 包含搜索、命令行、Wolfram、OpenAI 语音等能力。项目里要谨慎使用
`CommandExecutor`，它会执行系统命令，必须加权限边界。

## 10. Embedding

OpenAI embedding：

```rust
use langchain_rust::embedding::{
    embedder_trait::Embedder,
    openai::OpenAiEmbedder,
};

#[tokio::main]
async fn main() {
    let embedder = OpenAiEmbedder::default();
    let vector = embedder.embed_query("Why is the sky blue?").await.unwrap();
    println!("{vector:?}");
}
```

## 11. Qdrant Vector Store

启用 feature：

```bash
cargo add langchain-rust --features qdrant
```

启动 Qdrant：

```bash
docker run -p 6334:6334 qdrant/qdrant
```

示例：

```rust
use langchain_rust::{
    embedding::openai::openai_embedder::OpenAiEmbedder,
    schemas::Document,
    vectorstore::{
        qdrant::{Qdrant, StoreBuilder},
        VecStoreOptions, VectorStore,
    },
};

#[tokio::main]
async fn main() {
    let embedder = OpenAiEmbedder::default();

    let client = Qdrant::from_url("http://localhost:6334")
        .build()
        .unwrap();

    let store = StoreBuilder::new()
        .embedder(embedder)
        .client(client)
        .collection_name("langchain-rs")
        .build()
        .await
        .unwrap();

    let docs = vec![
        Document::new("Rust is a systems programming language."),
        Document::new("DroidLoom is an Android agent runtime project."),
    ];

    store
        .add_documents(&docs, &VecStoreOptions::default())
        .await
        .unwrap();

    let results = store
        .similarity_search("What is Rust?", 2, &VecStoreOptions::default())
        .await
        .unwrap();

    for doc in results {
        println!("{}", doc.page_content);
    }
}
```

## 12. 常用模块速查

| 模块 | 用途 |
| --- | --- |
| `llm::openai` | OpenAI LLM 调用 |
| `language_models::llm::LLM` | LLM trait，提供 `invoke` |
| `chain` | LLMChain、SequentialChain、QA Chain 等 |
| `prompt` | PromptTemplate、HumanMessagePromptTemplate |
| `schemas::messages` | system / human / ai message |
| `agent` | agent builder、executor |
| `tools` | tool trait 和内置工具 |
| `memory` | 简单 memory |
| `embedding` | embedding model |
| `vectorstore` | Qdrant、Postgres、Sqlite 等 |
| `document_loaders` | PDF、HTML、CSV、Git、source code 等 loader |

## 13. 常见坑

### crate 名和 import 名不同

`Cargo.toml` 里是：

```toml
langchain-rust = "4.6.0"
```

代码里是：

```rust
use langchain_rust::...;
```

### API 文档覆盖不完整

docs.rs 显示 `langchain-rust` 的文档覆盖率不高。实际开发时建议同时看：

- docs.rs item list
- GitHub examples
- 本地 cargo registry 里的 `examples/`

### Agent 不是 PIAgent 风格 stateful runtime

`AgentExecutor` 可以执行 agent，但它不是类似 PIAgent 的 `agent.state` 对象模型。
如果要做跨平台 agent core，建议自己保留：

```text
AgentState
AgentEvent
prompt / continue / abort / reset / waitForIdle
steering queue
follow-up queue
platform tool bridge
```

然后把 `langchain-rust` 放在 model/tool/RAG adapter 层。

### 命令行工具风险

`CommandExecutor` 很方便，但在真实 agent 里风险极高。至少要做：

- allowlist
- working directory 限制
- 超时
- 输出截断
- 用户确认
- trace 记录

### Android / macOS 跨平台

如果目标是 Android 和 macOS 复用 agent core：

- Rust agent core 可以跨平台。
- Android Accessibility、macOS Accessibility 不能直接塞进通用 core。
- 平台能力应通过 tool protocol 注入。
- `langchain-rust` 作为 Rust 侧模型/RAG层可以复用。

## 14. 推荐学习顺序

1. 先跑 `OpenAI::default().invoke(...)`。
2. 再跑 `LLMChainBuilder` + prompt template。
3. 再跑 streaming。
4. 再写一个自定义 `Tool`。
5. 最后再碰 embedding、vector store 和 RAG。

对 DroidLoom 这种 agent runtime 项目，优先掌握 agent/tool/memory 边界，不要一开始就把所有
LangChain 抽象接进核心状态机。

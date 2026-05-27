# llama.h 使用文档

日期：2026-05-27

本文面向 DroidLoom 的 `llama.cpp` adapter 设计，依据当前 `ggml-org/llama.cpp` 的 [`include/llama.h`](https://github.com/ggml-org/llama.cpp/blob/master/include/llama.h) 编写。`llama.cpp` API 仍在演进，真正落地时必须 pin 到一个具体 commit，并在代码里记录该 commit。

## 1. DroidLoom 中的定位

`llama.h` 不负责 Agent、workflow 或 Android 权限，它只提供本地模型推理能力。DroidLoom 应把它包装在 `runtime-llm` 的 `LlmEngine` 后面：

```text
Kotlin LlmEngine
  -> JNI bridge
  -> C++ LlamaEngine
      -> llama.h
      -> GGUF model
```

职责边界：

| 层 | 负责什么 | 不负责什么 |
| --- | --- | --- |
| Kotlin `LlmEngine` | Android lifecycle、配置、取消、stream 回调、错误映射 | 直接操作 KV 内存 |
| JNI bridge | 字符串/数组/句柄转换、线程边界、异常转错误码 | 业务策略 |
| C++ `LlamaEngine` | model/context/sampler/batch/state 管理 | Android 权限、workflow 调度 |
| `llama.h` | tokenization、decode、sampling、memory/state API | Agent loop、安全确认、trace |

## 2. 当前 API 主路径

不要使用已标记 deprecated 的旧函数。当前主路径是：

```text
llama_backend_init
  -> llama_model_default_params
  -> llama_model_load_from_file
  -> llama_context_default_params
  -> llama_init_from_model
  -> llama_model_get_vocab
  -> llama_tokenize
  -> llama_decode
  -> llama_sampler_chain_init / llama_sampler_chain_add
  -> llama_sampler_sample
  -> llama_sampler_accept
  -> llama_detokenize
  -> llama_free
  -> llama_model_free
  -> llama_backend_free
```

旧 API 对照：

| 不要用 | 改用 |
| --- | --- |
| `llama_load_model_from_file` | `llama_model_load_from_file` |
| `llama_new_context_with_model` | `llama_init_from_model` |
| `llama_free_model` | `llama_model_free` |
| `llama_token_bos` / `llama_token_eos` | `llama_vocab_bos` / `llama_vocab_eos` |
| `llama_get_state_size` | `llama_state_get_size` |
| `llama_copy_state_data` | `llama_state_get_data` |
| `llama_set_state_data` | `llama_state_set_data` |
| `llama_load_session_file` | `llama_state_load_file` |
| `llama_save_session_file` | `llama_state_save_file` |

## 3. 初始化和释放

进程启动时调用一次：

```cpp
llama_backend_init();
```

进程结束或 native runtime shutdown 时调用：

```cpp
llama_backend_free();
```

DroidLoom 不应该在每次请求里反复初始化 backend。推荐结构：

```cpp
class LlamaRuntime {
public:
    LlamaRuntime() {
        llama_backend_init();
    }

    ~LlamaRuntime() {
        llama_backend_free();
    }
};
```

Android 注意点：

- `llama_backend_init()` 放在 native runtime 生命周期，不放在 Activity 生命周期。
- `llama_backend_free()` 只在进程级释放时调用。
- `llama_context` 可以按 session 创建和释放。
- `llama_model` 应按模型缓存，避免每个 workflow 冷启动重新加载。

## 4. 加载模型

基础路径：

```cpp
llama_model_params mparams = llama_model_default_params();

// Android MVP 可以先 CPU-only；后续按设备 profile 调整。
mparams.n_gpu_layers = 0;
mparams.use_mmap = true;
mparams.use_mlock = false;

llama_model * model = llama_model_load_from_file(model_path.c_str(), mparams);
if (model == nullptr) {
    // 映射为 ModelLoadError
}
```

重要参数：

| 参数 | 作用 | DroidLoom 建议 |
| --- | --- | --- |
| `n_gpu_layers` | offload 到 GPU 的层数 | Android MVP 先 `0`，Vulkan/GPU profile 稳定后再调 |
| `use_mmap` | mmap 加载模型 | 默认开启，受文件系统和 Android 版本影响 |
| `use_mlock` | 锁定模型内存 | 移动端慎用，容易增加系统压力 |
| `check_tensors` | 校验模型 tensor | 调试或模型导入时开启，普通启动可关闭 |
| `progress_callback` | 加载进度 | 用于前台加载 UI 或 trace |
| `kv_overrides` | 覆盖 GGUF metadata | 只在明确需要时使用 |
| `no_alloc` | 只模拟 metadata/alloc | 可用于 profile 或 dry-run |

模型加载完成后可查询：

```cpp
const llama_vocab * vocab = llama_model_get_vocab(model);
int32_t n_ctx_train = llama_model_n_ctx_train(model);
int32_t n_layer = llama_model_n_layer(model);
```

## 5. 创建 context

```cpp
llama_context_params cparams = llama_context_default_params();

cparams.n_ctx = requested_context;       // 0 表示使用模型默认
cparams.n_batch = 512;                   // 逻辑 batch
cparams.n_ubatch = 128;                  // 物理 micro-batch
cparams.n_seq_max = 1;                   // MVP 单序列
cparams.n_threads = decode_threads;
cparams.n_threads_batch = prefill_threads;
cparams.embeddings = false;
cparams.offload_kqv = false;             // GPU 后端稳定后再评估

llama_context * ctx = llama_init_from_model(model, cparams);
if (ctx == nullptr) {
    // 映射为 ContextCreateError
}
```

创建后要查询实际值，不要只相信请求值：

```cpp
uint32_t actual_ctx = llama_n_ctx(ctx);
uint32_t actual_batch = llama_n_batch(ctx);
uint32_t actual_ubatch = llama_n_ubatch(ctx);
uint32_t actual_seq_max = llama_n_seq_max(ctx);
```

DroidLoom profile 里至少记录：

- requested/actual `n_ctx`;
- requested/actual `n_batch`;
- requested/actual `n_ubatch`;
- `n_threads` / `n_threads_batch`;
- model path、GGUF metadata 摘要、量化类型；
- cold load time、prefill time、decode tok/s；
- peak native memory 估算。

## 6. Tokenization

推荐用 model vocab 直接 tokenization：

```cpp
const llama_vocab * vocab = llama_model_get_vocab(model);

int32_t n = llama_tokenize(
    vocab,
    prompt.data(),
    static_cast<int32_t>(prompt.size()),
    nullptr,
    0,
    true,   // add_special
    true    // parse_special
);

if (n < 0) {
    n = -n;
}

std::vector<llama_token> tokens(n);

int32_t actual = llama_tokenize(
    vocab,
    prompt.data(),
    static_cast<int32_t>(prompt.size()),
    tokens.data(),
    static_cast<int32_t>(tokens.size()),
    true,
    true
);

if (actual < 0) {
    // buffer 仍不足或 tokenization 失败
}
```

注意：

- `add_special` 是否开启要跟模型 chat template 和 prompt builder 对齐。
- `parse_special` 允许解析控制 token；如果 prompt 是用户原文，应谨慎开启。
- DroidLoom 的 prompt builder 应输出明确的 segments：`system_prefix`、`workflow_prefix`、`tool_schema`、`goal`、`observation`、`scratchpad`。

## 7. Chat template

`llama.h` 提供 `llama_chat_apply_template`，可以把 `llama_chat_message` 转成模型 chat prompt。

DroidLoom 早期建议：

- 如果使用 known instruct model，可以先使用它的官方 template。
- workflow compiler 内部仍保留 prompt segment 结构。
- 最终传给 `llama.cpp` 前才 flatten 成文本。

不要让 chat template 吃掉 DroidLoom 的 cache metadata。应保留：

```text
PromptSegment {
  role,
  text,
  cacheScope,
  invalidationKey
}
```

## 8. Sampling chain

`llama.h` 当前使用 sampler chain：

```cpp
llama_sampler_chain_params sparams = llama_sampler_chain_default_params();
llama_sampler * sampler = llama_sampler_chain_init(sparams);

llama_sampler_chain_add(sampler, llama_sampler_init_top_k(50));
llama_sampler_chain_add(sampler, llama_sampler_init_top_p(0.9f, 1));
llama_sampler_chain_add(sampler, llama_sampler_init_temp(0.8f));
llama_sampler_chain_add(sampler, llama_sampler_init_dist(seed));
```

释放：

```cpp
llama_sampler_free(sampler);
```

注意：`llama_sampler_chain_add` 会接管 sampler 对象的所有权。加到 chain 后不要单独 free 子 sampler。

DroidLoom 参数建议：

| 场景 | sampler |
| --- | --- |
| tool/action JSON | 低温度或 greedy，优先 grammar/constrained output |
| 普通解释文本 | top-k + top-p + temperature |
| verifier/分类 | greedy 或 very low temperature |
| debug deterministic replay | 固定 seed + greedy/低温 |

## 9. Decode 循环

原型可以使用 `llama_batch_get_one` 快速跑通。生产路径建议用 `llama_batch_init` 管理 batch buffer。

原型示意：

```cpp
// prefill
llama_batch batch = llama_batch_get_one(tokens.data(), tokens.size());
int rc = llama_decode(ctx, batch);
if (rc != 0) {
    // DecodeError
}

std::vector<llama_token> output;

for (int i = 0; i < max_new_tokens; ++i) {
    llama_token id = llama_sampler_sample(sampler, ctx, -1);
    llama_sampler_accept(sampler, id);

    if (id == llama_vocab_eos(vocab)) {
        break;
    }

    output.push_back(id);

    llama_batch next = llama_batch_get_one(&id, 1);
    rc = llama_decode(ctx, next);
    if (rc != 0) {
        // DecodeError
        break;
    }
}
```

转回文本：

```cpp
std::string text;
text.resize(4096);

int32_t n = llama_detokenize(
    vocab,
    output.data(),
    static_cast<int32_t>(output.size()),
    text.data(),
    static_cast<int32_t>(text.size()),
    true,   // remove_special
    false   // unparse_special
);

if (n >= 0) {
    text.resize(n);
}
```

生产实现要求：

- 支持 streaming：每个 token detokenize 后回调 Kotlin。
- 支持 cancel：通过 `abort_callback` 或外部 session flag 终止 decode。
- 支持 timeout：session controller 层控制。
- 支持 structured parse failure retry：由 `runtime-agent` 处理，不在 C++ 隐式重试。
- 记录 prefill token 数、decode token 数、latency、tok/s。

## 10. Batch API 生产用法

`llama_batch_get_one` 头文件里标注为迁移 helper。DroidLoom 正式 adapter 应用：

```cpp
llama_batch batch = llama_batch_init(max_tokens, 0, n_seq_max);
```

每次提交前填：

```cpp
batch.n_tokens = n;
batch.token[i] = token;
batch.pos[i] = pos;
batch.n_seq_id[i] = 1;
batch.seq_id[i][0] = seq_id;
batch.logits[i] = should_output_logits;
```

释放：

```cpp
llama_batch_free(batch);
```

这样 DroidLoom 才能控制：

- prompt segment 对应的 position；
- 多 sequence / branch rollback；
- 哪些 token 需要 logits；
- prompt prefix 复用；
- verifier 或 tool-call 分支。

## 11. Memory API 与 KV 生命周期

`llama.h` 中的 memory API 是 DroidLoom 做 KV 生命周期实验的重点：

```cpp
llama_memory_t mem = llama_get_memory(ctx);
```

可用操作：

| API | 用途 | DroidLoom 映射 |
| --- | --- | --- |
| `llama_memory_clear` | 清空 memory/KV | session reset |
| `llama_memory_seq_rm` | 删除某序列某位置范围 | observation 失效、分支回滚 |
| `llama_memory_seq_cp` | 复制序列到另一个 seq | prefix reuse、branch fork |
| `llama_memory_seq_keep` | 只保留指定 seq | 切换 active workflow branch |
| `llama_memory_seq_add` | 平移 position | context sliding / compaction |
| `llama_memory_seq_div` | position 压缩 | 长上下文实验 |
| `llama_memory_seq_pos_min/max` | 查询 seq 位置范围 | cache metadata 校验 |

DroidLoom 的 compiler 不应直接暴露这些 API 给 workflow author。推荐内部抽象：

```text
KvSegment {
  seqId,
  posStart,
  posEnd,
  cacheScope,
  invalidationKey,
  liveIn,
  liveOut
}
```

映射策略：

| Prompt segment | cacheScope | 失效条件 |
| --- | --- | --- |
| `system_prefix` | model-global | 模型、sampler policy、system prompt 变化 |
| `workflow_prefix` | workflow | workflow version 变化 |
| `tool_schema` | tool-set | tool registry/schema 变化 |
| `goal` | session | 用户目标变化 |
| `observation` | screen-state | screen diff / package / activity 变化 |
| `scratchpad` | branch | retry、rollback、verifier failure |

## 12. State/session 文件

`llama.h` 提供完整 context state 和单 sequence state：

完整 state：

```cpp
size_t size = llama_state_get_size(ctx);
std::vector<uint8_t> buf(size);
size_t written = llama_state_get_data(ctx, buf.data(), buf.size());
size_t read = llama_state_set_data(ctx, buf.data(), written);
```

文件：

```cpp
bool ok = llama_state_save_file(ctx, path, tokens.data(), tokens.size());
bool ok2 = llama_state_load_file(ctx, path, tokens_out, cap, &n_tokens_out);
```

单 sequence：

```cpp
size_t seq_size = llama_state_seq_get_size(ctx, seq_id);
size_t n = llama_state_seq_get_data(ctx, dst, dst_size, seq_id);
size_t loaded = llama_state_seq_set_data(ctx, src, src_size, dest_seq_id);
```

DroidLoom 用法：

- MVP：先不持久化真实 KV，只记录 prompt segment metadata。
- POC：用 `llama_state_seq_save_file` 做 prefix snapshot 实验。
- 正式：由 `runtime-workflow` 决定哪些 prefix 可以保存，`runtime-llm` 只执行保存/恢复。

风险：

- state 文件可能很大，不适合频繁写 flash。
- state 和模型、上下文参数、sampler 策略强绑定。
- state 文件不能跨模型或不兼容版本盲目恢复。
- Android 上必须放在 app 私有目录，并受 trace retention policy 管控。

## 13. JNI 封装建议

Kotlin 侧接口不要暴露 `llama_context *`。用 opaque handle：

```kotlin
interface LlmEngine {
    suspend fun load(model: ModelHandle, profile: RuntimeProfile)
    fun generate(request: GenerateRequest): Flow<TokenEvent>
    suspend fun reset(sessionId: SessionId)
    suspend fun unload()
}
```

C++ 侧：

```cpp
struct LlamaHandle {
    llama_model * model = nullptr;
    llama_context * ctx = nullptr;
    llama_sampler * sampler = nullptr;
};
```

JNI 层只传 `jlong handle`：

```cpp
reinterpret_cast<LlamaHandle *>(handle);
```

线程规则：

- 一个 `llama_context` 同一时刻只给一个 generation loop 使用。
- 多 session 并发不要共享同一个 `llama_context`。
- 如果要共享模型，model cache 要有引用计数。
- Kotlin coroutine cancel 要映射到 native cancel flag。

## 14. Android 构建注意点

Android 上接 `llama.cpp` 推荐先做最小 NDK bridge：

```text
app/
  src/main/cpp/
    CMakeLists.txt
    llama_engine.cpp
    llama_jni.cpp
```

初期目标：

- CPU-only。
- 一种 ABI，优先 `arm64-v8a`。
- 一个小 GGUF 模型。
- fake/small prompt 测试。
- JNI smoke test，不跑大模型。

后续再评估：

- Vulkan/GPU backend。
- 多 ABI。
- 模型下载和校验。
- native crash dump。
- physical device performance profile。

## 15. DroidLoom adapter 错误模型

C++ 错误不要直接抛到 Kotlin。统一映射：

| 错误 | 触发点 |
| --- | --- |
| `BackendInitError` | `llama_backend_init` 或 native runtime 初始化失败 |
| `ModelLoadError` | `llama_model_load_from_file` 返回 null |
| `ContextCreateError` | `llama_init_from_model` 返回 null |
| `TokenizeError` | `llama_tokenize` 返回异常负值或 overflow |
| `DecodeError` | `llama_decode` 返回非 0 |
| `SamplerError` | sampler chain 构造失败 |
| `StateSaveError` | state/session 保存失败 |
| `StateLoadError` | state/session 恢复失败 |
| `Cancelled` | Kotlin session 取消或 native abort callback |

## 16. 最小实现顺序

1. C++ console POC：加载 GGUF，输入 prompt，输出文本。
2. Android JNI POC：Kotlin 调 native，返回整段文本。
3. Streaming：token-by-token 回调 Kotlin。
4. Cancellation：Kotlin cancel -> native abort。
5. `LlmEngine` 接口：替换临时 JNI API。
6. Batch API：从 `llama_batch_get_one` 切到 `llama_batch_init`。
7. State/session POC：保存和恢复 prompt prefix。
8. Memory seq POC：prefix fork、branch rollback、observation invalidation。
9. Trace：记录 token、latency、context、cache events。

## 17. 不做什么

MVP 不做：

- 修改 `llama.cpp` 内部 KV cache。
- 自己实现 sampler。
- 多模型并发。
- 长上下文压缩。
- 跨模型 state 复用。
- 在普通 PR CI 下载大模型。
- 把 state/session 文件作为用户可见数据导出。

## 18. 参考资料

- `llama.h`: https://github.com/ggml-org/llama.cpp/blob/master/include/llama.h
- Raw `llama.h`: https://raw.githubusercontent.com/ggml-org/llama.cpp/master/include/llama.h
- llama.cpp Android docs: https://github.com/ggml-org/llama.cpp/blob/master/docs/android.md
- llama.cpp repository: https://github.com/ggml-org/llama.cpp

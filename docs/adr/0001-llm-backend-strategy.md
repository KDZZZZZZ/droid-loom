# ADR 0001: LLM Backend Strategy

Date: 2026-05-27

## Status

Accepted.

## Context

DroidLoom needs local LLM inference on Android and eventually wants compiler-like optimization across workflows. The backend must support small on-device models, predictable memory profiles, and enough build-time/runtime control to study prompt and KV-cache behavior.

Candidate paths:

- MLC LLM.
- llama.cpp.
- MediaPipe/LiteRT LLM.
- Remote LLM provider.

## Decision

Use MLC LLM as the primary backend and llama.cpp as the secondary fallback behind a shared `LlmEngine` interface.

## Rationale

- MLC LLM has an Android SDK and a compile/package workflow that produces Android runtime artifacts.
- MLC is close to Apache TVM, which matches the project's goal of treating workflows like optimizable graphs.
- MLC exposes context and prefill related knobs that are directly relevant to memory planning.
- llama.cpp has a strong GGUF ecosystem, Android documentation, and a simpler path for CPU-first fallback.
- Keeping a backend interface prevents workflow/compiler logic from depending on either runtime too early.

## Consequences

Positive:

- Compiler research can target MLC first without blocking practical experiments.
- llama.cpp gives a robust escape hatch for unsupported models/devices.
- Model profiling can compare both engines using the same workload.

Negative:

- Two adapters increase integration and test cost.
- MLC Android setup is more complex than a pure Java/Kotlin dependency.
- Backend-specific prefix/KV behavior may not be portable.

## Follow-up

- Define `LlmEngine` capabilities before writing backend-specific code.
- Build a profiling harness before optimizing.
- Do not patch backend KV internals until workflow-level prompt segment reuse proves measurable value.

# Roadmap

## M0 - Repository and Design

Status: done in initial repository seed.

- Name, license, docs, ADRs.
- Android capability research.
- Backend selection research.
- Workflow compiler architecture.
- Permission and distribution boundary.

## M1 - Android Shell

Goal: build a minimal Android app with permission onboarding and a no-op session controller.

- Kotlin + Gradle Android project.
- Compose UI for permissions, sessions, and logs.
- AccessibilityService registration and disclosure screen.
- Foreground session notification.
- Local Room/DataStore setup.
- Unit test skeleton.

Exit criteria:

- App installs on a physical Android device.
- User can enable/disable the accessibility service.
- App records a local, non-sensitive session trace.

## M2 - Screen Observation and Action Executor

Goal: complete the first observe-act-verify loop without LLM.

- Accessibility tree snapshot.
- Node normalization and pruning.
- Accessibility action executor.
- Gesture fallback executor.
- Basic verifier predicates.
- UI Automator test harness.

Exit criteria:

- Deterministic workflow can open settings, find a visible UI element, click it, and verify state.
- Risky actions are blocked unless explicitly allowlisted.

## M3 - LLM Runtime Adapter

Goal: integrate local model inference through a stable `LlmEngine` interface.

- MLC LLM Android adapter proof of concept.
- llama.cpp adapter proof of concept.
- Model profile registry.
- Prompt segment builder.
- Structured action output parser.

Exit criteria:

- Same planner request can run on MLC or llama.cpp adapter.
- Basic model profile captures cold start, prefill latency, decode tok/s, and memory estimate.

## M4 - Workflow IR and Compiler Passes

Goal: make workflows analyzable and optimizable.

- Workflow schema and parser.
- IR graph model.
- Pass manager.
- Capability lowering.
- Prompt constant folding.
- Observation pruning.
- Risk annotation.
- Retry/verifier lowering.

Exit criteria:

- Example workflows compile to executable plans.
- Compiler emits a permission/action summary before execution.

## M5 - KV Lifetime and Context Optimizer

Goal: optimize LLM calls at the workflow layer before touching inference kernels.

- Prompt segment liveness analysis.
- Prefix invalidation keys.
- Session cache metadata.
- Context-window planner.
- MLC profile generation for context and prefill chunk choices.
- llama.cpp profile generation for context size choices.

Exit criteria:

- Repeated workflow runs reduce prefill tokens or prompt rebuild work.
- Optimizer can explain which prompt segments are reused or invalidated.

## M6 - Pixel/OCR Fallback

Goal: handle screens where accessibility tree is incomplete.

- MediaProjection lifecycle.
- Foreground service integration.
- OCR with ML Kit or pluggable OCR.
- Region-of-interest capture.
- Observation fusion between tree and OCR.

Exit criteria:

- Workflow can recover when target text is not available in accessibility nodes but visible on screen.
- Capture session starts only after explicit user action and stops after workflow completion.

## M7 - Benchmark Suite

Goal: measure reliability and cost.

- AndroidWorld-inspired tasks.
- UI Automator replay harness.
- Success/failure taxonomy.
- Latency, memory, battery, token metrics.
- Regression dashboard artifacts.

Exit criteria:

- Every release candidate runs a fixed task suite.
- Task failures include trace, screenshot policy result, and verifier output.

## Non-goals Until Later

- Root-only features.
- ADB-dependent user runtime.
- Marketplace for third-party workflows.
- Silent background operation.
- Autonomous high-risk actions.
- Custom fork of MLC or llama.cpp for KV internals before workflow-level cache planning proves value.

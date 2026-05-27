# ADR 0002: Workflow IR and Optimizer

Date: 2026-05-27

## Status

Accepted.

## Context

The project needs user-defined workflows and agent behavior, but free-form scripts are hard to secure, optimize, or verify. Treating a workflow as a graph gives the runtime a place to reason about observation cost, prompt construction, action risk, retry behavior, and KV-cache lifetime.

## Decision

Represent workflows as an intermediate representation with typed nodes, explicit side effects, risk annotations, and verifier edges. Compile workflow definitions into executable plans through a pass manager.

Initial IR operations:

- `ObserveScreen`
- `NormalizeState`
- `PromptBuild`
- `LlmCall`
- `ParseToolCall`
- `Guard`
- `ExecuteAction`
- `WaitForState`
- `Verify`
- `TraceWrite`

Initial compiler passes:

- schema validation;
- capability lowering;
- observation pruning;
- prompt constant folding;
- risk annotation;
- retry/verifier lowering;
- prompt segment and KV-cache lifetime analysis;
- cost planning.

## Rationale

- A graph makes side effects inspectable before runtime.
- Prompt segments can be analyzed like values with invalidation keys and lifetimes.
- Static workflow summaries are necessary for permission disclosure.
- Verification and retry can be added consistently instead of per-workflow ad hoc logic.
- The design mirrors TVM at the system level without prematurely coupling to TVM internals.

## Consequences

Positive:

- Workflows become testable artifacts.
- Optimizations can be added incrementally.
- The runtime can reject unsafe or unsupported workflows before execution.

Negative:

- Workflow authors must accept a constrained model instead of arbitrary code.
- Some dynamic agent behavior must be represented as explicit bounded nodes.
- IR versioning is required once workflows are persisted.

## Follow-up

- Version the IR from the first implementation.
- Add property tests for graph validation and liveness.
- Build a human-readable compiler explanation output for each workflow.

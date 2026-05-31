# Public API Meaning Audit

Date: 2026-05-31

## Scope

This audit checks every entry listed in `docs/architecture/rust-agent-core/11-public-api/README.md`.

For each API, the question is not only "does it compile", but also:

- what behavior it drives;
- where that behavior is implemented;
- whether tests assert an observable effect;
- whether the API is a runtime feature, a data model, or a guarded extension point.

## Result

All documented API entries are meaningful after this audit.

The graph API was refactored after this audit:

- old activation-condition graph state was removed from the public surface;
- `Graph` now aliases the package graph runtime spec;
- edge semantics are output-port routing plus input-package filtering;
- node execution is async through `NodeExecutor`;
- tick budget is configured on `GraphRunInput`.

No real API key is required or recorded by these checks.

## Verification

Run:

```powershell
cargo test -p agent-core -p mobilerun-agent-boundary
```

Expected coverage:

- core unit tests
- `crates/agent-core/tests/public_api_contract.rs`
- mobilerun boundary unit tests
- `example/mobilerun-agent-boundary/src/public_api_contract.rs`

## Meaning Categories

- Runtime behavior: the API changes execution state or output.
- Data model: the API carries typed information consumed by another component.
- Guarded extension point: the API is intentionally present, but unsupported paths fail explicitly or require a caller-owned policy.

Guarded extension points are still meaningful when the behavior is explicit and tested. They are not allowed to silently look successful.

## Core Agent APIs

| API | How it works | Meaning |
| --- | --- | --- |
| `AgentDefinitionBuilder` | Collects name, system prompt, and tool visibility before validation. | Runtime configuration builder. |
| `AgentDefinition` | Stores immutable agent identity, prompt, and per-tool visibility. | Runtime configuration read by registry and agent factory. |
| `ToolVisibility` | Controls direct, searchable, or hidden tool access. | Enforced by `ToolRegistry::get_for_agent`, `direct_schemas`, and search. |
| `AgentFactory` | Validates definition and attaches `AgentServices`. | Prevents invalid agents from running. |
| `Agent` | Owns an id, definition, services, and cancellation flag; delegates runs to `GraphRunner`. | Runtime execution entry. |
| `AgentRunResult` | Carries run id, messages, events, status, and error. | Observable result surface. |
| `AgentRunStatus` | Maps graph status into agent-level completed, cancelled, or failed. | Stable status contract. |

Evidence:

- `agent_public_api_contract`
- `agent::tests::run_delegates_to_graph_runner`
- `agent_factory::tests::*`

## Tool APIs

| API | How it works | Meaning |
| --- | --- | --- |
| `Tool` | Tool packs implement metadata and invoke. | Runtime tool boundary. |
| `ToolMetadata` | Stores schema, visibility, capabilities, and execution metadata. | Registry and probability graph consume it. |
| `ToolInvocation` | Carries call id, tool name, arguments, and metadata into a tool. | Typed tool input. |
| `ToolOutput` | Returns JSON output and metadata from a tool. | Typed tool output. |
| `ToolSchema` | Validates tool name, description, input object, and required arguments. | Prevents malformed tool calls. |
| `ToolRegistry` | Registers tools, resolves visibility, returns direct schemas, and searches searchable tools. | Enforces tool exposure. |
| `ToolExecutor` | Resolves tool, validates schema, applies permission policy, invokes tool, wraps result. | Main tool runtime. |
| `ToolCall` | Represents a model/planner-requested tool call. | Executor input. |
| `ToolResult` | Represents success, denied, or error output and converts to `RunMessage`. | Model/session-compatible tool output. |
| `ToolResultStatus` | Classifies tool execution outcome. | Used by long tasks and tests to decide success/failure. |

Evidence:

- `tool_public_api_contract`
- `tool_executor::tests::parallel_batch_preserves_call_order`
- `tool_registry::tests::resolves_visibility_from_agent_definition`
- `tool_schema::tests::validates_required_arguments`

Notes:

- `ToolExecutionMetadata.timeout_ms` and `ToolResultPolicy` are metadata today; the public API doc does not claim executor timeout or summary enforcement.
- `ToolMetadata::can_preexecute()` is active and consumed by the mobilerun probability graph.

## Graph APIs

| API | How it works | Meaning |
| --- | --- | --- |
| `Graph` | Validates node specs, package edges, graph input ref, and optional finish node. | Runtime graph template. |
| `GraphNode` | Public alias for `NodeSpec`; describes transform, tool, agent, graph, or final node. | Unit of graph execution. |
| `GraphEdge` | Routes `(node, output_port)` output log entries into `(node, input_package)`. | Message/content routing. |
| `InputPackageSpec` | Defines required and optional package items for a node. | Context inheritance and unlock condition. |
| `MessageQuery` | Filters messages and selected fields before package insertion. | Prevents noise inheritance and duplicate updates. |
| `NodeOutput` | Lets executor emit messages on named output ports. | Output fan-out and branch routing. |
| `NodeConcurrency` | Limits serial, parallel, or per-key node activations. | Runtime scheduling guard. |
| `NodeExecutor` | Async executor for transform/tool/agent/graph/final node kinds. | Runtime extension point. |
| `AgentRunInput` | Carries graph, initial messages, optional run id, and stop request into `Agent::run`. | Agent run input contract. |
| `GraphRunner` | Synchronous facade over async `GraphRuntime`; records state, ledger, events, and status. | Core graph runtime. |
| `GraphRunInput` | Carries graph run id, initial messages, stop request, and max ticks. | Direct runner input contract. |
| `GraphRunResult` | Carries graph messages, events, runtime state, ledger, status, and error. | Observable graph result. |
| `GraphRunStatus` | Reports completed, drained, cancelled, budget exceeded, or failed. | Stable graph status contract. |

Evidence:

- `graph_public_api_contract`
- `graph_runner::tests::*`
- `graph_runtime::tests::*`

## Hook APIs

| API | How it works | Meaning |
| --- | --- | --- |
| `HookName` | Distinguishes point hooks from wrapper hooks. | Registration guard. |
| `HookPayload` | Carries hook data and metadata through point handlers. | Point hook input/output. |
| `PointHookDecision` | Continues, rewrites, emits event, blocks, or stops. | Point hook control signal. |
| `WrapperRequest` | Carries wrapper hook data and metadata. | Wrapper input. |
| `WrapperResponse` | Carries wrapper data, metadata, and messages. | Wrapper output. |
| `WrapperResult` | Continues, rewrites, retries, recovers, stops, or fails. | Wrapper control signal. |
| `HandlerRegistry` | Registers ordered scoped handlers, runs point chains, and composes wrapper layers. | Manual hook runtime. |
| `WrapperNext` | Calls the inner wrapper service. | Middleware composition primitive. |

Evidence:

- `hook_public_api_contract`
- `hook_handler::tests::*`

Guarded extension point:

- `HandlerRegistry` is meaningful as an explicit registry. It is not yet automatically wired into `GraphRunner` or `AgentServices`; that remains a future integration point documented in the API doc.

## Event APIs

| API | How it works | Meaning |
| --- | --- | --- |
| `CoreEvent` | Represents agent, graph, node, message, hook, and error events. | Trace/event model. |
| `EventLog` | Accumulates and exposes event vectors. | Lightweight event collector. |
| `AgentRunResult.events` | Includes agent start/end plus graph events. | Agent-level trace output. |
| `GraphRunResult.events` | Includes graph start/end, node start/end, and message emitted events. | Graph-level trace output. |

Evidence:

- `message_input_event_and_context_public_api_contract`
- `agent_public_api_contract`
- `graph_public_api_contract`

## Input and Message APIs

| API | How it works | Meaning |
| --- | --- | --- |
| `user_input::text_message` | Validates non-empty text and wraps a user text block. | User input helper. |
| `user_input::content_blocks_message` | Validates non-empty block list and wraps user message. | Multimodal input helper. |
| `user_input::file_reference_message` | Validates URI and wraps file reference. | File input helper. |
| `user_input::image_reference_message` | Validates URI and wraps image reference. | Image input helper. |
| `user_input::audio_reference_message` | Validates URI and wraps audio reference. | Audio input helper. |
| `ContentBlock` | Represents text, reasoning, tool calls/results, references, diagnostics, or custom data. | Message content model. |
| `RunMessage` | Represents role, content, status, ids, usage, metadata, and source node. | Session/provider/graph message model. |
| `DiagnosticLevel` | Classifies diagnostic content severity. | Diagnostic metadata. |

Evidence:

- `message_input_event_and_context_public_api_contract`
- `user_input::tests::*`
- `run_message::tests::*`
- `content_block::tests::*`

## LLM Context APIs

| API | How it works | Meaning |
| --- | --- | --- |
| `AssistantBuilder` | Coalesces text/reasoning deltas and appends tool calls/diagnostics before finalizing. | Streaming assistant message builder. |
| `ContextBuildInput` | Carries model, replay messages, run messages, tool schemas, options, and metadata. | Context build input. |
| `ContextBuilder` | Converts core messages and tool schemas into provider-neutral `LlmRequest`. | Provider-neutral context compiler. |
| `LlmRequest` | Carries model, instructions, input items, tools, options, metadata, and headers. | Provider adapter input. |

Evidence:

- `message_input_event_and_context_public_api_contract`
- `llm_provider_and_registry_public_api_contract`
- `context::tests::*`

## Session and Turn APIs

| API | How it works | Meaning |
| --- | --- | --- |
| `TurnLoop` | Buffers user messages, prepares one active turn, finishes/aborts turns, and handles stop state. | Turn lifecycle guard. |
| `SessionEntry` | Stores session header, finalized message, or compaction entry. | Persistent session entry. |
| `InMemorySessionStore` | Appends entries and rebuilds a `SessionTree`. | Local store implementation. |
| `SessionStore` | Defines append, entries, and default `load_tree`. | Storage extension point. |
| `ReplaySnapshot` | Carries replayed entries, messages, and summaries. | Replay output model. |
| `replay_active_branch()` | Replays current active branch. | Session replay helper. |
| `replay_branch()` | Replays a specified leaf branch. | Branch replay helper. |

Evidence:

- `session_turn_and_error_public_api_contract`
- `session_store::tests::*`
- `session_replay::tests::*`
- `turn_loop::tests::*`

## Cancellation APIs

| API | How it works | Meaning |
| --- | --- | --- |
| `Agent::cancel` | Sets the agent cancellation flag. | Agent-level stop signal. |
| `Agent::cancellation_token` | Reads the cancellation flag without owning the agent. | Stop observation handle. |
| `AgentRunInput::with_stop_requested` | Requests cancellation for one run. | Per-run stop signal. |

Evidence:

- `agent_public_api_contract`
- `agent::tests::cancellation_token_tracks_agent_cancel`

## Error APIs

| API | How it works | Meaning |
| --- | --- | --- |
| `AgentCoreResult` | Common result alias for public API calls. | Error handling contract. |
| `AgentCoreError` | Typed invalid input/config, not found, permission, recoverable, fatal, and serialization errors. | Error taxonomy. |

Evidence:

- `session_turn_and_error_public_api_contract`
- error matching across graph, tool, input, session, and provider tests.

## Mobilerun-like Example APIs

| API | How it works | Meaning |
| --- | --- | --- |
| `scripted_agent::run_boundary_probe()` | Runs the whole boundary probe across definition, tools, context, session, probability, graph, map, and long task. | End-to-end integration entry. |
| `MobileRunLikeConfig::agent_definition()` | Builds default Mobilerun-like `AgentDefinition`. | Prompt/tool visibility bridge. |
| `MobileRunLikeConfig::agent_definition_with_tool_visibility()` | Builds an optimized definition from computed visibility. | Hot/cold tool layering bridge. |
| `MobileRunLikeConfig::render_user_prompt()` | Renders variables into task prompt. | User task prompt generation. |
| `MobileRunLikeConfig::render_system_prompt()` | Renders fast or reasoning system prompt. | System prompt generation. |
| `default_tool_visibility()` | Declares direct/searchable/hidden mobile tools. | Baseline tool layer. |
| `render_template()` | Replaces `{{key}}` and `{{ key }}` variables. | Prompt utility. |
| `register_mobilerun_tools()` | Registers all mock mobile tools into core registry. | Tool pack bridge. |
| `expected_action_count()` | Returns expected tool surface size. | Coverage guard. |
| `TrajectoryProbabilityGraph::from_sessions()` | Builds event transition and tool probability stats from message sessions. | Probability graph input. |
| `TrajectoryProbabilityGraph::from_replay_snapshots()` | Builds probability graph from session replay snapshots. | Replay-backed trajectory stats. |
| `TrajectoryProbabilityGraph::observe_session()` | Adds one session to the graph. | Incremental stats update. |
| `TrajectoryProbabilityGraph::likely_next()` | Returns next-event probabilities. | Prediction API. |
| `TrajectoryProbabilityGraph::tool_use_probability()` | Returns per-tool session probability. | Hot tool scoring. |
| `TrajectoryProbabilityGraph::plan_preexecution()` | Selects high-probability visible read-only idempotent calls. | Safe preexecution planner. |
| `TrajectoryProbabilityGraph::execute_preexecution_plan()` | Runs planned calls in parallel and converts results to messages. | Preexecution runtime. |
| `TrajectoryProbabilityGraph::plan_tool_layers()` | Converts probability stats into direct/searchable/hidden visibility. | Hot/cold tool layering. |
| `TaskContextPackage` | Holds task goal, inputs, artifacts, and required state. | Task-bound context unit. |
| `TaskNode` | Wraps a package and dependency ids. | DAG node. |
| `TaskDependencyGraph` | Stores tasks, detects dependency order, emits task messages. | Task context DAG. |
| `sample_mobile_task_dag()` | Builds open/search/finalize example DAG. | Test fixture with behavior. |
| `mark_stability()` | Writes context stability into message metadata. | Cache-aware context labeling. |
| `order_by_stability()` | Sorts stable prefix, task package, dependency result, volatile observation. | Cache-aware ordering. |
| `stability_order_labels()` | Reads stability labels for assertions/reporting. | Observability helper. |
| `stable_prefix_id()` | Hashes stable system prompt and direct tool names. | Cache/key route id. |
| `KeyRoutePlan::new()` | Defines stable and general account routes. | Key routing plan. |
| `KeyRoutePlan::route_for_prefix()` | Selects route for prefix id. | Routing decision. |
| `KeyRoutePlan::apply()` | Adds authorization header and route metadata to `LlmRequest`. | Request routing runtime. |
| `StaticSecretResolver::new()` | Provides fake/test secrets by env slot. | Test resolver. |
| `EnvSecretResolver` | Resolves env slot values at runtime. | Runtime resolver. |
| `run_hundred_step_cross_app_task()` | Executes 100+ meaningful cross-app tool steps through core `ToolExecutor`. | Long phone-using boundary task. |
| `CrossAppTaskReport::detail()` | Summarizes long task metrics. | Report helper. |
| `AppMapMemory::new()` | Creates empty GUI map memory. | Map memory state. |
| `AppMapMemory::observe_ui_state()` | Builds/refreshes page nodes and candidate actions from UI JSON. | Map update. |
| `AppMapMemory::local_view()` | Returns bounded local graph plus candidate actions. | Token-saving context view. |
| `AppMapMemory::semantic_search()` | Searches page summaries and elements. | Goal-to-page lookup. |
| `AppMapMemory::plan_path()` | Finds non-stale transition path between pages. | Map reuse path planning. |
| `AppMapMemory::forget()` | Deletes stale, query-matched, or explicit pages and dependent transitions. | Dynamic memory maintenance. |
| `sample_cross_app_map()` | Builds a multi-page map with transitions. | Long-task fixture with behavior. |

Evidence:

- `example/mobilerun-agent-boundary/src/public_api_contract.rs`
- `cross_app_task::tests::completes_hundred_step_cross_app_task_with_map_reuse`
- `app_map_memory::tests::*`
- `execution_probability::tests::*`
- `key_routing::tests::*`
- `task_context::tests::*`

## Remaining Boundaries

These are intentionally not claimed as active runtime features in the API doc:

- Automatic hook dispatch from `GraphRunner`.
- Production child-agent/provider/tool routing for `NodeKind::Agent` and `NodeKind::Tool`.
- Provider-independent tool timeout enforcement.
- Runtime interpretation of `ToolResultPolicy::ReturnSummary` or `Hidden`.

They remain extension points. The meaningful behavior today is explicit data modeling, explicit failure, or caller-owned policy execution.

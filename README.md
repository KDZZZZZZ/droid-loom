# DroidLoom

DroidLoom is an Android agent runtime and workflow compiler for operating a phone through user-approved screen understanding, Android APIs, and local LLM inference.

The name is intentional: Android workflows are "woven" from screen observations, API tools, guardrails, and model calls, then compiled into an executable plan that can be optimized like a graph.

## Goal

Build an Android agent app that can:

- observe the current screen through Android accessibility and optional screen-capture APIs;
- operate the UI through accessibility actions, gestures, intents, notifications, and app-specific Android APIs;
- expose a custom workflow and agent layer for user-defined automations;
- run local LLM backends through MLC LLM first, with llama.cpp as a portable GGUF fallback;
- compile workflows into an intermediate representation that enables scheduling, context reuse, KV-cache lifetime analysis, memory planning, and verification passes.

## Current Status

This repository currently contains the project definition, architecture, research notes, and decision records. It is intentionally docs-first because the Android permission model, Play policy boundary, local inference backend, and optimizer shape need to be stable before app scaffolding.

## Key Documents

- [Research Notes](./docs/research.md) - end-to-end technical research and references.
- [Architecture](./docs/architecture.md) - runtime components, data flow, and module boundaries.
- [Roadmap](./docs/roadmap.md) - phased build plan and validation gates.
- [ADR 0001](./docs/adr/0001-llm-backend-strategy.md) - MLC LLM primary backend with llama.cpp fallback.
- [ADR 0002](./docs/adr/0002-workflow-ir-and-optimizer.md) - workflow IR and compiler pass strategy.
- [ADR 0003](./docs/adr/0003-permission-and-distribution-boundary.md) - Android permissions and distribution boundary.

## Important Boundary

Android accessibility automation is a sensitive capability. Google Play policy permits AccessibilityService for many uses, but automation that lets an app autonomously initiate, plan, and execute actions is restricted unless the app is a qualifying accessibility tool. DroidLoom should start as a research/prototype and internal/sideloaded build until product scope, disclosures, and distribution rules are validated.

## License

Apache-2.0. See [LICENSE](./LICENSE).

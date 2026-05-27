# ADR 0003: Permission and Distribution Boundary

Date: 2026-05-27

## Status

Accepted.

## Context

DroidLoom needs capabilities that are sensitive on Android:

- AccessibilityService for screen semantics and UI actions.
- Optional MediaProjection for pixel capture.
- Optional NotificationListenerService for notification actions.
- Local traces that may include app names, visible text, and action history.

Google Play policy around AccessibilityService automation is strict. A general-purpose autonomous assistant that reads the screen and executes actions can conflict with Play policy unless scoped as a qualifying accessibility tool or constrained to narrow deterministic automation.

## Decision

Start DroidLoom as a research/prototype/internal/sideload project. Do not claim Google Play compatibility until product scope and policy review are completed.

Runtime rules:

- Every sensitive capability requires explicit user onboarding and a revoke path.
- The app must show a foreground notification during active agent sessions.
- Default operation is local-only.
- Screenshots are ephemeral by default.
- High-risk actions require user confirmation.
- Workflows must declare capabilities and side effects before execution.

## Rationale

- It avoids building the wrong product around a policy assumption.
- It keeps the first engineering milestone focused on a safe, observable runtime.
- It makes compliance requirements visible in the architecture instead of after implementation.

## Consequences

Positive:

- Lower risk of accidentally building silent or overbroad automation.
- Clearer docs for contributors.
- Easier to add policy gates to compiler and runtime.

Negative:

- Public distribution is delayed.
- Some desirable autonomous use cases must remain opt-in or unsupported.
- User testing needs sideload/internal distribution paths first.

## Follow-up

- Add a permission disclosure checklist before the first Android release.
- Add a risk taxonomy to workflow schema.
- Consult Play policy before any store listing work.

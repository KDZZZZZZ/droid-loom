# AGENTS.md

## Project Review Rules

DroidLoom is an Android Agent runtime and workflow compiler. Reviewers should prioritize correctness, Android permission boundaries, privacy, safety, and traceability over broad feature expansion.

## Human Review Requirement

Documentation review is a human approval process. Automated Codex review can provide comments, but it does not satisfy the human review requirement for `README.md`, `docs/**`, `AGENTS.md`, or `.github/**`.

Unreviewed documentation must not be merged into `main`.

## Codex Review Guidance

When reviewing PRs, focus on:

- Android sensitive capabilities: AccessibilityService, MediaProjection, NotificationListenerService, foreground services, permissions, screenshots, and logs.
- Agent safety: Guard, Confirm, TakeOver, StopSession, verifier behavior, and high-risk action handling.
- Workflow semantics: IR nodes, compiler passes, retry behavior, trace output, and cache invalidation rules.
- Local inference: MLC LLM, llama.cpp, JNI/native boundaries, model artifacts, and KV/prefix cache behavior.
- CI and device lab safety: public PRs must not run untrusted code on self-hosted physical-device runners.

For documentation-only PRs:

- Check whether the document changes project policy, architecture, product boundary, or implementation direction.
- Flag unsupported claims, missing risk notes, or references that do not match the proposed architecture.
- Do not treat spelling or style nits as blocking unless they obscure technical meaning.

For code PRs:

- Lead with correctness, security, privacy, crash risk, data loss, and missing tests.
- Prefer small, actionable findings tied to exact files or code paths.
- Call out missing emulator, unit, or physical-device validation when relevant.

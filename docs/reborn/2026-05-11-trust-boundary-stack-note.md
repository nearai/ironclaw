# Reborn Trust-Boundary Stack Note

**Date:** 2026-05-11
**Issue:** #3492 — Establish trust-boundary hardening baseline

This trust-boundary-stack note maps current open Reborn PRs to the trust-boundary baseline so reviewers and agents can distinguish slice-owned fixes from deferred follow-up work. It is not a claim that every PR below already satisfies every invariant.

## Baseline invariants

- trusted policy/evidence/snapshot values use host witnesses, sealed constructors, or crate-private minting paths;
- untrusted memory/skill/search/tool content enters prompts through an escaping envelope;
- hashes declare purpose, with trust/binding/authenticity using SHA-256/BLAKE3 or separate authenticity checks;
- driver/operator-visible errors map to `Transient`, `Permanent`, `Misconfigured`, `PolicyDenied`, or equivalent semantics;
- async queues, caches, buffers, byte budgets, and counters have explicit limits and checked arithmetic;
- sandbox/native/host names accurately describe the trust boundary;
- enum/status/policy changes include downstream match-site audit evidence;
- `serde(default)` on security/durability gates fails closed or has migration tests.

## Open stack ownership map

### PR #3488 — feat(reborn): audit memory significant events

URL: https://github.com/nearai/ironclaw/pull/3488

- owns: memory event redaction/no-raw-content posture for significant memory events.
- owns: event classification clarity where memory errors become durable event metadata.
- defers: prompt envelope integration for retrieved memory content until prompt assembly/memory context slices wire `UntrustedPromptContent`.
- defers: broad back-pressure helper adoption unless this PR changes event queue or byte accumulation behavior.
- follow-up: #3492 migration child issue/PR for memory prompt envelope wiring.

### PR #3487 — Project loop model milestones to durable events

URL: https://github.com/nearai/ironclaw/pull/3487

- owns: durable event projection metadata for model/reply milestones.
- owns: redacted milestone shape if driver/operator-visible failures are projected.
- defers: cross-crate `OperatorErrorClass` adoption unless this PR changes error mapping.
- defers: match-site audit harness beyond milestone-specific enum/status changes.
- follow-up: #3492 migration child issue/PR for event-facing error class mapping.

### PR #3471 — GitHub issue #3431: [Reborn] Add MemoryPromptContextService production adapter

URL: https://github.com/nearai/ironclaw/pull/3471

- owns: memory prompt context retrieval boundary and scoped read behavior.
- owns: ensuring retrieved memory content is marked as untrusted before prompt inclusion.
- defers: final prompt assembler rendering if this slice only returns context snippets rather than model messages.
- follow-up: #3492 migration child issue/PR should wire `UntrustedPromptContent` through memory prompt snippets and caller-level prompt assembly tests.

### PR #3470 — GitHub issue #3432: [Reborn] Add deterministic trust-aware SkillContextService

URL: https://github.com/nearai/ironclaw/pull/3470

- owns: trust-aware skill snapshot construction and version/fingerprint semantics.
- owns: skill content provenance needed for prompt envelope metadata.
- defers: replacing any trust-adjacent FNV/non-cryptographic fingerprint with SHA-256/BLAKE3 if not already done in this PR.
- follow-up: #3492 migration child issue/PR for cryptographic skill snapshot fingerprint and skill prompt envelope integration.

### PR #3469 — GitHub issue #3433: [Reborn] Complete HostManagedModelGateway budget, credential, and redaction tests

URL: https://github.com/nearai/ironclaw/pull/3469

- owns: model gateway budget, credential, and redaction caller-level tests.
- owns: driver/operator-safe summaries for gateway-visible failures touched by the slice.
- defers: shared `OperatorErrorClass` mapping unless this PR changes public error APIs.
- follow-up: #3492 migration child issue/PR for model-gateway error classification if current failures remain category-only.

### PR #3468 — GitHub issue #3451: [Reborn] Add direct DB operations for loop checkpoint mappings

URL: https://github.com/nearai/ironclaw/pull/3468

- owns: durable checkpoint mapping shape and persistence fail-closed behavior.
- owns: `serde(default)` scrutiny for checkpoint/durability fields touched by the PR.
- defers: broad match-site audit harness unless new status/exit/policy variants are added.
- follow-up: #3492 migration child issue/PR for checkpoint evidence witness construction if checkpoint mapping values become trust-bearing public API.

### PR #3462 — [Reborn] Add user-selectable model routes and provider pool

URL: https://github.com/nearai/ironclaw/pull/3462

- owns: model-route/provider-pool config boundaries and security-relevant defaults.
- owns: `serde(default)` fail-closed behavior for route/provider security or durability gates.
- owns: bounded provider pools/caches if this PR introduces admission or cache state.
- defers: shared error class adoption unless provider-pool errors cross driver/operator boundaries.
- follow-up: #3492 migration child issue/PR for route config default audit and operator error class mapping.

### PR #3460 — feat(reborn): add trusted LoopExitApplier

URL: https://github.com/nearai/ironclaw/pull/3460

- owns: `LoopExitValidationPolicy` trusted construction path and evidence verification semantics.
- owns: downstream match-site audit when loop exit/status/policy variants change.
- status in this baseline PR: `LoopExitValidationPolicy` fields are private, wire deserialization cannot mint host-verified evidence, fail-closed construction uses named constructors, and host-verified evidence bits are explicit at call sites.
- defers: global error classification helper unless runner/operator APIs change here.
- follow-up: #3492 migration child issue/PR should wire durable evidence ports into the host-side constructor calls where any remaining fake/test evidence exists.

### PR #3454 — Add Reborn loop capability host-runtime adapter slice

URL: https://github.com/nearai/ironclaw/pull/3454

- owns: capability-host to runtime-dispatch trust boundary for loop capability invocation.
- owns: stable redacted adapter error categories for dispatch/runtime failures touched by the slice.
- owns: output/admission limits when accumulating runtime output or byte counters.
- defers: shared `OperatorErrorClass` and `BoundedCounter` adoption if this PR already landed local equivalents.
- follow-up: #3492 migration child issue/PR for adapter error class normalization and checked output counters.

### PR #3428 — feat(reborn): add ProductWorkflow and InboundTurnService facade (#3280)

URL: https://github.com/nearai/ironclaw/pull/3428

- owns: inbound product workflow trusted envelope and host-auth evidence boundary.
- owns: sealed constructor patterns for verified protocol/auth state; existing `ProtocolAuthEvidence` is the reference pattern.
- defers: prompt-content envelope unless inbound payloads flow into prompt context in this slice.
- follow-up: #3492 migration child issue/PR for any remaining public trust-bearing inbound state.

### PR #3400 — Add Reborn text-only model reply driver

URL: https://github.com/nearai/ironclaw/pull/3400

- owns: driver/operator-visible model reply errors and text-only loop output behavior.
- owns: prompt assembly boundaries if retrieved context snippets are materialized into model messages.
- defers: full untrusted prompt envelope if this PR only rejects memory/instruction snippets.
- follow-up: #3492 migration child issue/PR for text-only prompt envelope support and fake-role/tag-injection tests.

### PR #3352 — feat(reborn): add product adapter host auth and egress primitives

URL: https://github.com/nearai/ironclaw/pull/3352

- owns: host-auth evidence sealed construction and host-mediated egress primitives.
- owns: credential/network policy default fail-closed behavior.
- owns: bounded request/response budgets if adapter egress accumulates bytes.
- defers: shared primitive adoption where older local helpers already exist.
- follow-up: #3492 migration child issue/PR for checked byte counters and operator-visible egress error classes.

## Mechanical audit command

For loop/status/policy changes, include at minimum:

```bash
rg "match.*TurnStatus|match.*LoopBlocked|match.*LoopExit" --type rust
```

For broader Reborn policy/status changes, include:

```bash
rg "match .*TurnStatus|match .*LoopBlocked|match .*LoopExit|match .*RuntimeKind|match .*AgentLoopHostErrorKind|match .*Trust|match .*Policy" --type rust
```

Record command output or audited sites in the PR body.

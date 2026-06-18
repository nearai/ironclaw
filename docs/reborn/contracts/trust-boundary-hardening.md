# Reborn Contract — Trust-Boundary Hardening Baseline

**Status:** Design draft for issue #3492
**Date:** 2026-05-11
**Target crates:** `ironclaw_trust`, `ironclaw_turns`, `ironclaw_memory`, `ironclaw_skills`, `ironclaw_host_runtime`, `ironclaw_architecture`, plus touched substrate crates
**Depends on:** [`kernel-boundary.md`](kernel-boundary.md), [`host-api.md`](host-api.md), [`memory.md`](memory.md), [`host-runtime.md`](host-runtime.md), [`runtime-selection.md`](runtime-selection.md), [`loop-exit.md`](loop-exit.md)

---

## 1. Purpose

Reborn already has good local trust-boundary patterns, but the patterns are not yet a baseline every slice must reuse. Issue #3492 turns repeated review findings into a shared contract, small code primitives, and PR/checklist requirements.

The baseline closes one repeated failure mode:

```text
trusted state exists in a design, but untrusted callers can still mint or mutate the values that prove trust
```

The fix is not one large subsystem. The fix is a cross-cutting rule set:

1. trust-bearing values are mint-only through host witnesses, sealed constructors, or crate-private construction paths;
2. untrusted retrieved content never enters prompts as raw instruction-shaped text;
3. hashes declare their purpose, and authenticity-adjacent comparisons use cryptographic digests or separate authenticity checks;
4. driver/operator-facing errors carry stable class semantics;
5. async entry points and accumulators declare admission, byte, count, and overflow limits;
6. names expose the security boundary they represent;
7. enum/status/policy changes come with downstream match-site audits;
8. `serde(default)` stays fail-closed for security/durability gates.

---

## 2. Non-goals for the foundation PR

The foundation PR should not migrate every open Reborn PR in one diff.

It should establish:

- contract docs and review checklist;
- shared primitive crates/modules where a future slice can import the pattern;
- tests proving the primitives fail closed;
- a stack tracking note naming which open PR owns or defers each invariant.

Follow-up PRs can then migrate concrete open slices such as loop-exit policy construction, trust-aware skill fingerprints, memory/skill prompt assembly, adapter error mapping, and output/admission limits.

---

## 3. Trusted minting contract

### 3.1 Rule

Any public type that represents policy, evidence, trust, validated snapshots, authority, or security-relevant state must answer this question in its docs and API shape:

```text
Who can construct this value, and what evidence did they verify first?
```

Untrusted callers must not be able to forge trusted state by:

- public struct fields;
- public `new(...)` constructors that accept raw claims without evidence;
- `Deserialize` into a trusted variant;
- `Default` producing a permissive trusted value;
- copying a witness from one value into another forged value.

### 3.2 Allowed patterns

Use one of these patterns:

1. **sealed host witness** — a private seal type proves construction happened inside the trusted module/crate;
2. **crate-private constructor** — public fields stay private and `pub(crate)` constructors mint trusted state after validation;
3. **wire claim + trusted decision split** — untrusted wire data deserializes to a claim; host validation returns a separate trusted decision;
4. **test-only minting** — `#[cfg(any(test, feature = "test-support"))]` constructors exist only for test fakes.

Existing examples to follow:

- `crates/ironclaw_product_adapters/src/auth.rs`: `ProtocolAuthEvidence` can serialize verified evidence but only failed evidence can deserialize from wire; verified evidence is host-minted through a private seal.
- `crates/ironclaw_trust`: privileged `EffectiveTrustClass` values are host-policy-only; manifest requests do not become grants.

### 3.3 Loop-exit policy implication

`LoopExitValidationPolicy` is security-relevant because it controls whether driver-provided exits become trusted run transitions. Its fields should not remain a freely constructable public bag for production callers.

Foundation path:

```text
LoopExitValidationPolicyClaim     # optional untrusted/config wire shape, if needed
LoopExitValidationPolicyWitness   # host/runner-only witness
LoopExitValidationPolicy          # private fields, host-minted constructors
```

The exact implementation may keep compatibility helpers temporarily, but production mapping must make the trusted construction path visible.

---

## 4. Untrusted prompt-content envelope

### 4.1 Rule

Retrieved memory, skill text, extension docs, catalog snippets, search results, and any other user/tool/provider supplied text must not be injected into prompts as raw instruction-shaped content.

Prompt assembly must wrap such content in a uniform envelope before it reaches model messages:

```xml
<untrusted-content source="memory|skill|extension|search|tool" trust="sandbox|installed|trusted|first_party|system|unknown" id="...">
  escaped content
</untrusted-content>
```

Attribute names and values are examples; exact enum names can follow crate vocabulary. Required semantics:

- content is escaped so it cannot close the envelope or inject sibling tags;
- source and trust are explicit metadata, not inferred from text;
- fake role turns such as `system:`, `assistant:`, `user:`, tool-call JSON, and instruction hijacks remain inert text;
- model-facing prompt builders have tests for tag injection, fake role/system/user turns, and instruction hijacks;
- prompt events/audit include metadata only, not raw envelope content.

### 4.2 Initial primitive

Add a small reusable primitive, likely in the crate that already owns prompt/context vocabulary for Reborn loop assembly:

```rust
UntrustedPromptContent {
    source: UntrustedPromptSource,
    trust: PromptContentTrust,
    id: Option<...>,
    body: String,
}

UntrustedPromptContent::render_envelope(...)
```

The primitive should be pure and unit-tested. Later slices wire it through `HostManagedLoopPromptPort`, memory prompt services, and skill context services.

---

## 5. Hash-purpose policy

### 5.1 Rule

Every hash use near trust, snapshots, surfaces, cache keys, or tamper checks must declare its purpose.

Allowed purpose classes:

| Purpose | Allowed algorithm | Examples |
| --- | --- | --- |
| Stable ID / partition / cache key | Fast non-cryptographic hash allowed if named as non-authenticating | deterministic local IDs, map shards, hints |
| Fingerprint / replay surface version | SHA-256 or BLAKE3 unless explicitly non-security and documented | capability surface version, run-profile fingerprint |
| Trust binding / tamper check / authenticity-adjacent comparison | SHA-256, BLAKE3, or a separate signature/MAC/authenticity check | skill snapshot trust binding, package manifest binding |

A type named `Fingerprint`, `TrustedSnapshot`, `TrustBinding`, `ContentHash`, or similar must not silently use FNV/`DefaultHasher` unless docs make clear it is non-authenticating and not used for trust.

### 5.2 Current migration target

Trust-aware skill snapshot versioning should not use FNV for a trust/authenticity-adjacent value. The foundation PR should add a policy helper and tests; the migration PR should move that fingerprint to SHA-256 or BLAKE3.

---

## 6. Cross-crate error classification

### 6.1 Rule

Driver/operator-visible errors crossing crate boundaries must expose both:

1. stable redacted error kind; and
2. broad action class.

Minimum action classes:

```rust
Transient       // retry may succeed without config/code changes
Permanent       // same input likely fails again
Misconfigured   // operator/config/deployment issue
PolicyDenied    // host policy, authorization, approval, trust, or resource rule denied it
```

Names can vary if semantics stay explicit.

### 6.2 Mapping guidance

- HTTP 429/503, temporary provider/network unavailable -> `Transient`.
- unsupported operation, invalid invocation, malformed checkpoint refs -> `Permanent`.
- missing durable store, missing configured policy sink, missing required runtime adapter -> `Misconfigured`.
- authorization denial, stale surface, scope mismatch, output/resource limit, prompt policy refusal -> `PolicyDenied`.

Existing `AgentLoopHostErrorKind` can remain the specific kind surface, but it should map to a shared class for runner/operator decisions. Raw backend/provider details stay behind diagnostic refs.

---

## 7. Admission, back-pressure, and checked limits

### 7.1 Rule

Async entry points, caches, buffers, byte accumulation, and counters must declare limits and overflow behavior before side effects.

Required checks where relevant:

- maximum queued items / in-flight work;
- maximum body/output/artifact bytes;
- maximum cache entries or total bytes;
- timeout/cancellation path;
- checked arithmetic for size accumulation;
- deterministic error when limit is exceeded;
- cleanup/release of reservations after failure.

### 7.2 Reusable primitive

Add one small reusable helper/pattern rather than each crate hand-rolling overflow checks. Candidate shape:

```rust
BoundedCounter::new(limit)
BoundedCounter::try_add(bytes) -> Result<(), LimitExceeded>
BoundedVecAdmission::try_push(item) -> Result<(), LimitExceeded>
```

The helper must use `checked_add` and return a stable limit-exceeded error instead of saturating into ambiguous state. Crates may keep domain-specific wrappers around the primitive.

---

## 8. Naming expectations

Names should make trust boundaries obvious.

Use:

- `Host*` for host-owned authority, policy, minting, or composition;
- `Runtime*` for code running inside a runtime lane after authorization;
- `SandboxBackend*` for containment implementation choices, not authority boundaries;
- `Native*` only for local host substrate implementation, not trusted-by-default authority;
- `System*` only for sealed host/kernel-only operations with mandatory audit.

Avoid names that imply sandboxing is authority or that native/local code is trusted. If a name is kept for compatibility, docs must state the real boundary.

---

## 9. Enum/status/policy downstream audit

When adding or changing variants for status, exit, policy, trust, runtime kind, or driver/operator-visible error enums, authors must audit downstream match sites.

Minimum mechanical command for loop-related changes:

```bash
rg "match.*TurnStatus|match.*LoopBlocked|match.*LoopExit" --type rust
```

Recommended broader audit when changing Reborn status/policy enums:

```bash
rg "match .*TurnStatus|match .*LoopBlocked|match .*LoopExit|match .*RuntimeKind|match .*AgentLoopHostErrorKind|match .*Trust|match .*Policy" --type rust
```

Each PR that changes such variants must list audited sites in the PR description or explain why the command is not relevant.

---

## 10. `serde(default)` fail-closed convention

`serde(default)` is allowed for compatibility fields that only add metadata or preserve old documents.

It is not allowed to silently enable or weaken security/durability gates. For gate-bearing fields:

- missing fields must deserialize to deny/disabled/untrusted/unknown/requires-validation;
- permissive defaults need a migration note and caller-level tests;
- `Default` for a trusted/policy type must be fail-closed;
- security-relevant deserialization should prefer `deny_unknown_fields` unless compatibility requires otherwise.

Examples of gate-bearing fields:

- trust level;
- approval requirement;
- credential injection requirement;
- durability requirement;
- network policy;
- sandbox/process policy;
- prompt-content trust classification;
- checkpoint/evidence validation policy.

---

## 11. Foundation PR shape

The first PR for issue #3492 should include:

1. this contract doc linked from `_contract-freeze-index.md`;
2. a Reborn trust-boundary checklist in `.github/pull_request_template.md`;
3. pure primitives and tests for at least:
   - untrusted prompt-content envelope;
   - hash-purpose classification or helper;
   - error action classification;
   - bounded counter/admission helper;
4. one representative sealed/witness constructor migration or contract test, preferably where the current code is already called out by review;
5. a stack tracking note under `docs/reborn/` listing open PRs and whether each owns or defers these invariants.

Do not mix this foundation with broad migrations across every open PR. Migrations should become follow-up PRs linked from the tracking note.

---

## 12. PR checklist text

Add this section to the PR template for Reborn changes:

```markdown
## Reborn Trust-Boundary Checklist

<!-- Required for Reborn/security/runtime/DB changes. Write "N/A" with reason if not relevant. -->

- [ ] Public policy/evidence/trust-bearing types: who can construct them?
- [ ] Untrusted content enters prompts only through an envelope/escaping primitive.
- [ ] Hashes declare purpose; trust/binding/authenticity uses SHA-256/BLAKE3 or separate authenticity check.
- [ ] New/changed status, exit, policy, runtime, or error variants: downstream match sites audited. Command/output:
- [ ] Security/durability `serde(default)` fields fail closed or have migration tests.
- [ ] Queues/maps/buffers/counters have bounds and overflow-safe arithmetic.
- [ ] Driver/operator-visible errors have stable class semantics (`Transient`, `Permanent`, `Misconfigured`, `PolicyDenied` or equivalent).
- [ ] Sandbox/native/host names accurately describe trust boundary.
```

---

## 13. Open-stack tracking note format

The stack note should list current open Reborn PRs and a compact ownership matrix:

```text
PR #3460 LoopExitApplier
  owns: loop-exit evidence/policy witness migration
  defers: global error classification helper
  follow-up: #...

PR #3470 SkillContextService
  owns: cryptographic skill snapshot fingerprint migration
  owns: skill prompt envelope integration
  defers: product prompt assembler wiring
```

The note is not a review comment and should not claim every PR is fixed. It is a handoff map so reviewers stop rediscovering the same invariant gaps.

---

## 14. Verification expectations

Foundation PR verification should include:

- `cargo fmt --all -- --check`;
- targeted unit tests for new primitives;
- targeted architecture/checklist test if a mechanical guard is added;
- `cargo test -p ironclaw_architecture` if boundary rules or audit harness tests change;
- targeted crate tests for any migrated trust-bearing type.

Later migration PRs must add caller-level tests when the primitive gates side effects such as prompt assembly, dispatch, persistence, runtime execution, network egress, approvals, resources, or events.

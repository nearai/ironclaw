# Hooks framework threat model

> Purpose: enumerate the adversaries, assets, and attack vectors against
> the `ironclaw_hooks` framework. For each vector: the mitigation, the
> test or invariant that proves it, and the residual risk.
>
> Status: draft v1 (2026-05-13). This is *not* a substitute for an
> external pentest. It is the design-time threat-modeling artifact a
> pentester would start from.
>
> Companion doc: [prior-art.md](./prior-art.md). When a mitigation
> matches a known pattern from another system, the prior-art row is
> cited; novel mitigations are flagged.

## Scope

In scope:
- The `ironclaw_hooks` crate and its public API
- The dispatcher (`HookDispatcher`, `HookDispatcherBuilder`)
- The registry (`HookRegistry`, `HookBinding`, scope enforcement)
- The predicate evaluator and its in-memory state
- The middleware ports (capability/prompt/model/transcript/checkpoint)
- The cross-crate seam to `ironclaw_turns` (milestone sink) and
  `ironclaw_events` (RuntimeEvent projection)
- The `ironclaw_prompt_envelope` leaf crate

Out of scope (separate threat models needed when these land):
- The WASM hook execution path (manifest validates but doesn't execute)
- The persistent predicate counter (no durable state yet)
- Event-triggered hooks (Phase 5; not in this PR)
- Self-authored hooks with durable ratification (#3567)
- The extension installation pipeline itself (#3492 covers this)

## Assets

Ranked by blast radius of compromise:

| Asset | Why it matters |
|---|---|
| **User capability invocations** | A subverted gate can let a malicious extension exfiltrate, mutate, or destroy user data via legitimate capabilities. |
| **The agent's prompt bundle** | A subverted mutator can inject instructions the model will follow as if they came from the user. Classical prompt-injection escalation surface. |
| **Hook telemetry / audit log** | A subverted observer or projector can silently drop, duplicate, or forge audit records, breaking the "LLM data is never deleted" guarantee for hook decisions. |
| **Approval gate-refs** | Forged or replayed gate-refs would let an attacker bypass human-in-the-loop approval, turning a `PauseApproval` decision into effective Allow. |
| **Predicate counter state** | Manipulating the counter (cross-tenant inference, exhaustion, or rollback) defeats rate-limit and value-cap predicates. |
| **Process availability** | Sticky poison + flood of bad hooks could DOS the dispatcher; failure-isolation policy mitigates but doesn't eliminate. |

## Adversaries

| ID | Adversary | Capabilities |
|---|---|---|
| **A1** | Malicious Installed extension | Author chooses hook bodies, manifest contents, version strings; cannot author privileged-tier hooks; cannot bypass tier-specific installers. |
| **A2** | Compromised Trusted extension | Started benign, attacker now controls hook bodies. Has access to privileged sink methods (`allow()`); cannot bypass type-level seal but can fully exploit Trusted-tier privilege. |
| **A3** | Confused-deputy internal caller | Future internal crate that calls `install_builtin_*` or builds contexts without provider info, bypassing tier or scope enforcement by accident, not by design. |
| **A4** | Co-tenant attacker | User B trying to read or influence User A's hook state via shared dispatcher / predicate counter. |
| **A5** | Network attacker on the audit boundary | Attacker between the `LoopHostMilestoneSink` and the `RuntimeEvent` projector, or on the event consumer side. |
| **A6** | Local user with shell access | Can set env vars (`SECRETS_MASTER_KEY`), read process memory, attach debugger. Mostly out of scope for the framework but worth noting. |
| **A7** | The agent itself (self-authored hook author) | The agent under attacker influence (prompt-injected from earlier turn data) tries to author a self-authored hook to lock in attacker capability. |

## STRIDE per asset / vector

### S — Spoofing

| # | Vector | Adversary | Mitigation | Test/invariant | Residual |
|---|---|---|---|---|---|
| S1 | Forge approval gate-ref to bypass `PauseApproval` | A1 | `UuidHookGateRefFactory` mints UUIDv4 gate-refs (122 random bits per ref, RFC 4122 §4.4); gate-ref namespace is `gate:hook-approval-<uuid>` vs `gate:hook-auth-<uuid>`; one-shot consumption is the approval gateway's responsibility, not the factory's | `gate_refs_are_v4_uuids`, `gate_refs_have_no_collisions_across_many_calls` (20k draws), `approval_and_auth_namespaces_do_not_overlap` | Low (factory side); approval gateway one-shot is its own threat model |
| S2 | Spoof `HookId` to impersonate another hook in audit log | A1/A2 | Content-addressed `HookId` (blake3 of length-prefixed fields including extension_id + hook_version); duplicate-id rejection at registry insert | `dispatch::tests::content_addressed_hook_id_test`; `registry::tests::duplicate_hook_id_rejected` | Low |
| S3 | Spoof `ExtensionId` in `BeforeCapabilityHookContext.provider` to bypass scope filter | A3 | `provider` is `Option<ExtensionId>` populated by the *middleware*, not the hook; hook reads ctx but doesn't write it | Hook trait only exposes `&BeforeCapabilityHookContext`; ctx fields are not mutable from hook code (rustc-enforced) | Low |
| S4 | Spoof envelope source in prompt patch (e.g., make hook-emitted patch appear as `Memory`) | A1/A2 | `wrap_untrusted(source, trust, body)` source is set by middleware based on patch origin; closed-vocabulary `EnvelopeSource` enum | `prompt_envelope::tests::source_is_caller_chosen` | Low |
| S5 | Forge milestone events to fake hook outcomes in audit log | A2 | Milestone sink is private to the dispatcher per build; hooks receive sink references via opaque trait, not the sink itself | Hook traits don't carry milestone-sink params | Low |

### T — Tampering

| # | Vector | Adversary | Mitigation | Test/invariant | Residual |
|---|---|---|---|---|---|
| T1 | Tamper with predicate counter to defeat rate limit | A1 | Counter is private (`PredicateEvaluator` internal state); tenant-keyed `HistoryKey` prevents one tenant from incrementing another's counter | `evaluator::tests::tenant_keyed_history`; per-build dispatcher means counter state is local to the run | Low (in-process; cross-process needs durable + signed storage) |
| T2 | Tamper with `HookPatch` after sink emission to inject content | A1 | `HookPatch` is sealed (no public constructor outside sink); patches stored by value, not by reference | `kinds::tests::patch_constructor_is_sealed` | Low |
| T3 | Tamper with manifest scope at install time (claim `Global` after install) | A1 | Manifest is part of the extension bundle; bundle integrity is the extension installer's concern (out of scope here); `HookBinding.{owning_extension, scope}` are set from manifest at registration, not mutable after | Registry doesn't expose mutation methods on `HookBinding` | Low (delegates to extension installer security) |
| T4 | Tamper with dispatch ordering to skip a gate (e.g., re-register at higher priority) | A1 | Phase → priority → hook-id ordering is stable and computed at dispatch time; duplicate-id rejected; re-registration not supported | `dispatch::tests::ordering_is_stable` | Low |
| T5 | Tamper with `EnvelopeTrust` to upgrade an Installed-authored snippet | A1/A2 | Trust is set by middleware from binding's trust class, not from hook input; closed-vocabulary enum | Trust derivation is in middleware, not hook | Low |
| T6 | Tamper with milestone-event projection to drop `HookFailed` events | A5 | Projector is in `ironclaw_reborn`; sits behind the same audit substrate as the rest of the loop; events are append-only | `milestone_events::tests::all_hook_milestones_projected` | Med (depends on `ironclaw_events` substrate integrity, which is its own threat model) |
| T7 | Tamper with `ironclaw_prompt_envelope` byte cap to exfiltrate larger payload | A1 | 4 KiB cap enforced in `wrap_untrusted`; cap is a const, not configurable per call | `prompt_envelope::tests::body_cap_enforced` | Low |

### R — Repudiation

| # | Vector | Adversary | Mitigation | Test/invariant | Residual |
|---|---|---|---|---|---|
| R1 | Hook fires but no `HookDispatched` is recorded | A1 | Dispatcher emits `HookDispatched` *before* invoking the hook body | L4 pairing-invariant matrix test: every dispatch outcome yields ≥1 `HookDispatched` | Low |
| R2 | Hook decision recorded but actual outcome differs | A1 | `HookDecisionEmitted` summary derived from the same `HookDecision` that the dispatcher acts on | Same matrix test pairs Dispatched with exactly one terminator | Low |
| R3 | Hook panic silently drops audit | A1 | `catch_unwind` produces a `HookFailed` milestone before returning; failure_policy still emits | `dispatch::tests::panic_emits_hook_failed`; observer panic test in `hooks_integration.rs` | Low |
| R4 | Operator claims "hook never fired" but it did | All | All hook dispatch projects into `RuntimeEvent::Hook*` (durable); content-addressed `HookId` ties event to exact hook version | L3 schema snapshots prevent silent field drift; `HookId` derivation is pinned | Low |
| R5 | Schema drift breaks audit log replay | A5 | L3 frozen-JSON snapshot tests for every milestone variant + RuntimeEvent variant | `run_profile::tests::milestone_l3_snapshots` | Low |

### I — Information disclosure

| # | Vector | Adversary | Mitigation | Test/invariant | Residual |
|---|---|---|---|---|---|
| I1 | Cross-tenant inference via shared predicate counter | A4 | Tenant-keyed `HistoryKey { tenant_id, capability, ... }`; per-build dispatcher (FU8) means counters don't survive across runs | `evaluator::tests::tenant_keyed_history`; per-build test in `hooks_integration.rs` | Low (in-process); High if a persistent counter ships without per-tenant partitioning |
| I2 | Hook reads `BeforeCapabilityHookContext` args to leak sensitive capability inputs | A1 | `SanitizedArguments` exposes **only** `is_resolved()` and `extract_numeric(field_path)` — no `as_json`, no iteration, no key listing. A hook can ask for a named numeric path one-at-a-time but cannot enumerate args. The current predicate path enforces this by construction because the predicate spec itself declares the field. Future WASM hooks must thread the manifest-declared field allowlist through the resolver. | Documented in `SanitizedArguments` rustdoc | Low (current path); reassess when Installed-WASM lands |
| I3 | Hook leaks state via milestone summary (e.g., embeds user data in decision reason) | A2 | `HookDecisionSummary` is closed-vocabulary enum, not free-text — Trusted hook can't smuggle data in reason | `run_profile::tests::decision_summary_is_closed_vocab` | Low |
| I4 | Hook timing side-channel to infer capability invocation patterns of other tenants | A4 | Per-build dispatcher; timing-based inference of in-process state requires high precision; rate-limit predicates aren't published cross-tenant | None (acknowledged residual) | Low-Med |
| I5 | Prompt envelope leaks instruction-marker bypass via clever encoding | A1 | `INSTRUCTION_LIKE_MARKERS` denylist in `wrap_untrusted`; envelope wraps with `Untrusted hook content: <body>` prefix | `prompt_envelope::tests::instruction_markers_denied` | Med (denylists are inherently incomplete vs determined attacker; mitigated by the fact that the LLM is also trained to be skeptical of `Untrusted hook content:` markers) |
| I6 | Audit log itself leaks user data to operators who shouldn't see it | A5 | Audit substrate access is governed by `ironclaw_events` policies (separate threat model) | N/A here | Defer |

### D — Denial of service

| # | Vector | Adversary | Mitigation | Test/invariant | Residual |
|---|---|---|---|---|---|
| D1 | Slow hook hangs the loop | A1 | `tokio::time::timeout` per hook (configurable); Gate timeout → FailClosed (deny), Observer timeout → FailIsolated | `dispatch::tests::timeout_fails_closed_for_gate` | Low |
| D2 | Panicking hook re-panics every call, exhausting log substrate | A1 | Poison sticks on first panic; subsequent calls short-circuit without invoking hook body | `registry::tests::poisoned_slot_skips`; same-dispatch poison re-check | Low |
| D3 | Extension installs N hooks to flood the dispatcher | A1 | Pre-flight cap at registrar boundary: `MAX_HOOKS_PER_EXTENSION = 32` total per install batch; rejection is whole-batch so no partial install can slip past | `install_rejects_when_total_exceeds_per_extension_cap`; cap value pinned in `registrar.rs` const | Low |
| D4 | Extension registers hooks at every attach point to slow every dispatch | A1 | Pre-flight cap: `MAX_HOOKS_PER_EXTENSION_PER_KIND = 8` per attach-point per extension; tighter than the total cap because fan-out at one dispatch point is the actual blast radius | `install_rejects_when_per_kind_cap_exceeded`; `install_accepts_at_per_extension_cap` pins the at-cap boundary | Low |
| D5 | Predicate evaluator unbounded memory growth (window state per tenant × capability × hook) | A1/A4 | Sliding-window eviction trims expired entries within a key; `MAX_HISTORY_KEYS = 8192` caps the *number of keys* per map; on overflow the LRU key is evicted and `evictions_observed()` advances for operator visibility | `lru_eviction_increments_counter_and_drops_oldest_key`; operator runbook §2 | Low |
| D5a | **Correctness-under-attack** variant of D5: A1 floods the dispatcher with high-cardinality `(tenant_id, hook_id, capability)` keys (e.g. a hostile extension cycling synthetic capability names) to push past `MAX_HISTORY_KEYS`, causing legitimate tenants' counters to be LRU-evicted — silently resetting rate-limit state and allowing a foreign tenant to re-consume their budget (henrypark133 security note on PR #3635) | A1/A4 | Same `MAX_HISTORY_KEYS` cap, but the eviction itself is now the attack: capacity for a malicious attacker to evict-and-reset another tenant's bucket scales with attacker fan-out, not memory. Mitigation today is the `evictions_observed()` counter (operator alerts on advance — runbook §2) plus per-extension hook caps (D3/D4) bounding extension fan-out. The durable backend (successor doc 03) shifts the bound from "8192 keys in memory" to "rows in a tenant-partitioned table" with `evict_older_than` as the reaper — eliminating the cross-tenant LRU-eviction surface entirely | Same as D5 (per-key tenant isolation pinned by `tenant_keyed_history`); add an explicit eviction-pressure test once a synthetic load harness exists | Med (in-memory backend); becomes Low once durable backend lands |
| D6 | Approval gate-ref accumulation (PauseApproval emitted but never resolved) | A1 | Approval gateway has its own TTL on outstanding refs (separate subsystem); hook side just mints | Out of scope (depends on approval gateway) | Defer |
| D7 | Audit-log flood from chatty observer hook | A1/A2 | Observer-failure-isolated means runaway observer doesn't fail the run; emission rate is bounded by dispatch rate | Low (bounded by user activity) | Low |

### E — Elevation of privilege

| # | Vector | Adversary | Mitigation | Test/invariant | Residual |
|---|---|---|---|---|---|
| **E1** | **Installed hook mints `Allow` (the load-bearing claim)** | A1 | (a) `RestrictedGateSink` trait has no `allow()` method; (b) `BeforeCapabilityHookImpl::{Privileged, Restricted}` variants are `pub(crate)`; only `install_builtin_*` / `install_trusted_*` can construct `Privileged`, and they always pair it with Builtin/Trusted binding | `dispatch::tests::compile_time_seal_test`; `installed_binding_cannot_be_paired_with_privileged_impl` | **Very low** (type-enforced) |
| E2 | Trust class is set wrong at the loader boundary (Installed-WASM routed through `install_builtin_*`) | A3 | Loader contract doc + manifest-derived trust class; tier-specific installers force callers to be explicit | Loader contract test pinned in FU2 | Low (depends on loader correctness) |
| E3 | Installed hook with `OwnCapabilities` scope denies foreign-provider capability anyway | A1 | Dispatch-time scope filter against `ctx.provider`; conservative default: unresolved provider + `OwnCapabilities` ⇒ don't fire | `hooks_integration::tests::installed_with_own_scope_does_not_fire_for_foreign_provider`; FU1 | Low |
| E4 | Self-authored hook adds `Allow` despite being monotonic-restriction only | A7 | `SelfAuthoredHookSink` exposes no `allow()` method; `SelfAuthoredHookSpec` predicate language has no Allow primitive; run-scoped only (no durable persistence path yet) | `self_authored::tests::sink_cannot_allow`; closed-vocabulary spec | Low |
| E5 | Hook patch escapes the envelope (e.g., raw injection without `Untrusted hook content:` prefix) | A1/A2 | Mutator middleware always passes through `wrap_untrusted`; raw patches never reach the bundle | `prompt_port::tests::all_patches_enveloped` | Low |
| E6 | Hook mutates capability args mid-flight to alter the invocation | A1 | `BeforeCapabilityHookContext` is read-only; hook returns a decision, not a mutated context; capability args flow through unchanged | Rust borrow checker on the trait signature | Very low |
| E7 | Trusted hook installs an Installed-tier hook at runtime to launder privilege | A2 | `HookRegistrar::install` is the only entry to the registry and is called from the extension-installation flow, not from running hooks; hooks receive read-only contexts | Hook trait API has no registrar handle | Low |

## Cross-cutting properties

These properties should hold across the framework. Each maps to one or more tests above; gaps are listed.

| Property | Holds? | Evidence |
|---|---|---|
| Installed cannot mint Allow | ✓ | E1 (type-enforced) |
| Every dispatch emits exactly one terminator | ✓ | R1+R2 (L4 matrix test) |
| Failed hooks emit `HookFailed` | ✓ | R3 (panic test) |
| Manifest scope enforced at dispatch | ✓ | E3 (FU1) |
| Predicate counter is tenant-isolated | ✓ in-memory | I1 (FU5 + FU8) |
| Audit log is replayable across versions | ✓ | R5 (L3 snapshots) |
| Hook IDs uniquely identify hook bytes | ✓ | S2 |
| Patches always carry the untrusted envelope | ✓ | E5 |
| Gate-refs are unguessable (factory side) | ✓ | S1 — `gate_refs_are_v4_uuids` + 20k no-collision test |
| Gate-refs are one-shot at consumption | Deferred | Approval gateway's threat model, not the factory's |
| Resolver can't leak undeclared fields | ✓ (current path) | I2 — narrow `SanitizedArguments` public API; reassess when Installed-WASM lands |
| Per-extension hook count is bounded | ✓ | D3 + D4 — `MAX_HOOKS_PER_EXTENSION` / `_PER_KIND` consts in `registrar.rs` |
| Per-evaluator counter state is bounded | ✓ | D5 — `MAX_HISTORY_KEYS` cap + LRU eviction + `evictions_observed()` |

## Open follow-ups (threat-model-driven)

Ranked by severity:

1. ~~**(High)** Per-extension cap on hook registrations (D3/D4).~~ **DONE** — `MAX_HOOKS_PER_EXTENSION` (32) + `_PER_KIND` (8) consts in `registrar.rs`, enforced pre-flight in `enforce_registration_caps`.
2. ~~**(High)** Gate-ref unguessability test (S1).~~ **DONE** — `gate_refs_are_v4_uuids` pins the v4 entropy source; 20k-draw no-collision test as statistical proxy. One-shot consumption deferred to the approval gateway's threat model.
3. ~~**(Med)** Resolver field-level scope (I2).~~ **DONE** — `SanitizedArguments` narrow public API (only `extract_numeric(field_path)`) makes the current predicate path field-scoped by construction. Documented in rustdoc; reassess when Installed-WASM lands.
4. ~~**(Med)** Per-evaluator state ceiling (D5).~~ **DONE** — `MAX_HISTORY_KEYS = 8192` per map, LRU eviction, `evictions_observed()` metric.
5. ~~**(Med)** Document poison-stickiness operator runbook.~~ **DONE** — see [`operator-runbook.md`](./operator-runbook.md) §1.
6. **(Low)** Acknowledge timing side-channel residual (I4) in CLAUDE.md; defer mitigation unless a use case forces it.
7. **(Low)** Strengthen instruction-marker denylist (I5) with a periodic review against published prompt-injection corpora.

## What this threat model does NOT cover

- The **extension installation pipeline** — where do extension bundles come from, who signs them, how is `trust_class` derived. This is #3492.
- The **WASM execution sandbox** — when Installed-WASM ships, it needs its own threat model covering the wasmtime surface, host-function attenuation, and the linear-memory boundary.
- The **approval gateway** — gate-ref lifecycle, TTL, user-facing approval UX. Owned by the channel layer (#3564).
- **Side channels at the model layer** — what if the LLM itself is the attack vector (jailbreak, prompt injection from user input). The prompt-envelope mitigates the *hook-injected* prompt-injection vector but doesn't address user-driven prompt injection.
- **Supply chain on `blake3`, `tokio`, `serde_json`, `wasmtime`** — out of scope here.

## Methodology notes

- Threats enumerated via STRIDE per asset, then cross-checked against the
  prior-art divergences in `prior-art.md` to ensure no novel design
  decision is unmodeled.
- Severity ratings are subjective and pre-pentest. An external review
  would update them.
- "Test/invariant" column points to tests that *currently exist*. Gaps
  are explicit so a pentester can prioritize.
- A second pass once Installed-WASM lands is mandatory — that path
  expands every section of this document.

# Hooks framework prior-art comparison

> Purpose: validate the IronClaw hooks design against well-established
> hook/policy/extension systems. For each axis where IronClaw diverges,
> articulate **why**. A divergence without a why is a design smell.

Status: draft v1 (2026-05-13). Reviewers: design-time check before
trusting the v1 framework end-to-end.

## Systems surveyed

| Tag | System | Domain |
|---|---|---|
| **LSM** | Linux Security Modules (SELinux/AppArmor backend) | Kernel syscall mediation |
| **EBPF** | eBPF + Tetragon | Kernel observability + enforcement |
| **ENVOY** | Envoy proxy-wasm filters | L7 HTTP middleware |
| **K8S** | Kubernetes admission webhooks (Validating + Mutating) | API-server admission control |
| **OPA** | Open Policy Agent / Gatekeeper | Policy-as-code engine |
| **CRX** | Chrome extension permissions + declarativeNetRequest | Browser extensions |
| **VSC** | VS Code extension API | IDE plugins |
| **TAURI** | Tauri v2 capability/permission model | Desktop app plugins |
| **ICLAW** | IronClaw `ironclaw_hooks` | LLM agent loop |

---

## Comparison matrix

### Axis 1 — Dispatch model (when do hooks fire, chain semantics, short-circuit)

| Sys | Where | Chain | Short-circuit | Multiple hooks at point |
|---|---|---|---|---|
| LSM | Inline at syscall boundary (security_*) | Stacked, ordered at init | First DENY wins, no continue past deny | Yes (with cross-LSM coordination) |
| EBPF | Kprobe/tracepoint/LSM hook (BPF_PROG_TYPE_LSM); kernel calls program list | Program list per attach point | Verdict combined; LSM-BPF: any DENY wins | Yes |
| ENVOY | HTTP filter chain per request | Linear filter chain configured in listener | Filter returns StopIteration to halt | Yes, ordered |
| K8S | Synchronous webhook call from kube-apiserver during admission | Validating webhooks run in parallel, all must pass; Mutating run serially | Any validating DENY rejects; mutating webhooks rewrite spec | Yes |
| OPA | Sidecar/library decision call; engine evaluates rule set | Rule set is a logic program, not a chain | `deny` rule set non-empty → reject | N/A — one engine, many rules |
| CRX | Browser invokes registered listeners at lifecycle events | Multiple extensions can listen; `webRequest` is opinionated about precedence | declarativeNetRequest: highest priority rule wins | Yes |
| VSC | Activation event triggers extension load; extension calls back via API | No chain — extensions react independently | N/A — no dispatcher | Yes (independent) |
| TAURI | Capability check at IPC boundary; permission set evaluated | Permission set is union/intersection | First deny in evaluation order | N/A — one check per command |
| **ICLAW** | **Inline at typed dispatch points (before_capability, before_prompt, after_*); dispatcher iterates registered bindings** | **Phase (Validation→Authorization→Policy→Telemetry) → priority → hook-id, stable** | **Gate hooks short-circuit on first non-Pass decision; Telemetry observers always run** | **Yes, ordered** |

**Observation:** ICLAW's *phase* layer is closer to OPA's rule ordering than to LSM's flat list. Phases let policy-class hooks defer to authorization-class hooks without each hook author needing to know the global order. This is **good** — it externalizes ordering concerns from hook authors.

**Divergence:** ICLAW runs Telemetry observers even after gate denial. LSM does not (denial aborts the syscall). Why we diverge: hook telemetry is the audit substrate; observability of a *denied* operation is at least as valuable as of an allowed one. K8S admission has the same property (audit events fire on rejected admission). ✓

---

### Axis 2 — Trust tiers (how is privilege differentiated)

| Sys | Tiers | Distinguishes? |
|---|---|---|
| LSM | Single tier — kernel module, fully trusted | No — LSMs are kernel code |
| EBPF | Single tier — kernel verifier enforces safety, but verified BPF is fully trusted post-verify | No — verifier is the trust boundary, not a tier |
| ENVOY | Two: native C++ filters (trusted) vs proxy-wasm (sandboxed); no graded privilege within wasm | Coarse (trusted/sandboxed) |
| K8S | Single tier — any webhook can deny/mutate any resource it's configured for | No (RBAC controls *who can install*, not *what installed hook can do*) |
| OPA | Single tier — policies all run in the same Rego engine | No |
| CRX | MV3 declares permissions in manifest; some are "automatic," some require user prompt at install, some require runtime grant | Permission-graded (not tier-graded); user is the trust granter |
| VSC | Single tier — extensions run with the user's full FS/process privilege; "trust" enforced socially via Marketplace and "Trust this workspace?" UX prompt | No real tiering |
| TAURI | Permission set per plugin declared in manifest; `core:*` are built-in, third-party plugins ship their own permission catalog | Capability-graded |
| **ICLAW** | **Four: Builtin, Trusted, Installed, SelfAuthored — each with default attenuation; tier-specific installers force trust-class ↔ impl pairing at compile time** | **Yes — explicit, type-enforced** |

**Observation:** Few systems do graded trust tiers; most do *binary* trusted/sandboxed (Envoy) or *capability-graded* (Tauri/CRX). The closest analog to ICLAW's tier model is **Microsoft Defender ATP custom detection rules** vs **device control policies** — different rule sources, different default capabilities. Even there it's mostly conventional, not type-enforced.

**Divergence:** ICLAW enforces tier↔impl at the type level (sealed `BeforeCapabilityHookImpl::{Privileged, Restricted}` variants + tier-specific installers). Why we diverge: every other system in this table has had a CVE caused by an Installed-tier hook gaining a privileged-tier capability. LSM had `commoncap` ordering bugs; K8S had webhook bypass via `--disable-admission-plugins`; CRX has had repeated permission-escalation flaws. **Type-level enforcement of "Installed cannot Allow" is the most defensible part of ICLAW's design and the part most underrepresented in prior art.** ✓✓

---

### Axis 3 — Attenuation (how privilege is restricted at registration)

| Sys | Mechanism |
|---|---|
| LSM | None at registration — module is loaded with full LSM API surface |
| EBPF | Verifier rejects unsafe programs; helper-function allowlist per program type |
| ENVOY | proxy-wasm: ABI surface limits what filter can do; no attenuation beyond ABI |
| K8S | Webhook URL + resource selector in MutatingWebhookConfiguration; no attenuation of decision power |
| OPA | Policy bundles can be partitioned, but policies have full Rego power |
| CRX | Manifest permissions declared at install; user can revoke; some permissions runtime-prompted |
| VSC | None — extension has user-level privilege |
| TAURI | Permission set + scope (allowlist/denylist patterns) attached to capability grant |
| **ICLAW** | **Per-tier default attenuation + manifest-declared scope (`Global`/`OwnCapabilities`/`SameTenant`) enforced at dispatch + capability ↔ hook binding** |

**Observation:** Tauri's permission+scope model is the closest analog. ICLAW adds the **tier-based default attenuation** layer on top — Installed hooks default to a smaller capability set than Trusted hooks, even before manifest-declared scope.

**Divergence:** Manifest scope (`OwnCapabilities`) is enforced at *dispatch time* by filtering bindings against `ctx.provider`, not just at install time. Why: install-time-only enforcement (Tauri, K8S) is bypassable if any caller can construct a context without provider info. Dispatch-time enforcement defends against future internal callers that might not have known about scope. This was specifically codex audit finding C3. ✓

---

### Axis 4 — Decision vocabulary (what can a hook return)

| Sys | Decisions |
|---|---|
| LSM | int return: 0=allow, -EPERM=deny; no mutate, no pause |
| EBPF | LSM hook return: 0=allow, negative errno=deny; tracing hooks return value ignored |
| ENVOY | StopIteration / Continue / SendLocalReply (synthesized response); can rewrite headers/body |
| K8S | Validating: Allowed/Denied + reason; Mutating: JSON Patch operations |
| OPA | `allow`/`deny` rules + violation messages; can return arbitrary structured decision |
| CRX | declarativeNetRequest: block/redirect/upgradeScheme/modifyHeaders/allowAllRequests |
| VSC | N/A — extensions act, they don't decide |
| TAURI | Allow / Deny via capability evaluation; no mutation |
| **ICLAW** | **`Allow` (Privileged only), `Deny`, `PauseApproval` (returns gate-ref for human approval), `PauseAuth` (returns gate-ref for auth), `Pass` (no opinion), `Patch` (mutators), `Effect` (observers, future)** |

**Observation:** Most systems are allow/deny only. K8S adds mutation. ICLAW's `PauseApproval`/`PauseAuth` (returning a gate-ref instead of a binary verdict) is closest to **OAuth step-up authentication** in spirit — the hook can require an out-of-band user action before deciding. No system in this table has this exact primitive.

**Divergence:** Pause-with-gate-ref is novel here. Why: agent loops have a human-on-the-side that synchronous syscall mediators (LSM) don't have. Routing a decision to the user is a real outcome, not an error. The risk is gate-ref forgery — addressed via `HookGateRefFactory`-minted UUIDs, but worth a property test that gate-refs are unguessable and one-shot. **TODO — add to threat model.** ⚠️

**Divergence:** `Pass` (no-opinion) as a first-class return distinct from `Allow`. LSM has no equivalent — every LSM either allows or denies. Why we diverge: with multiple hooks at a point, "I don't care" is genuinely different from "I bless this." OPA has the same shape (a deny rule that doesn't fire ≠ an allow rule that does fire). ✓

---

### Axis 5 — Failure semantics (panic, timeout, malformed return)

| Sys | Panic/crash | Timeout | Malformed |
|---|---|---|---|
| LSM | Kernel panic (module bugs are catastrophic) | N/A — synchronous, no timeout | Compile-time prevented |
| EBPF | Verifier rejects unsafe; runtime division-by-zero etc. terminates program (treated as deny for LSM hooks) | Instruction limit | Verifier rejects |
| ENVOY | proxy-wasm: trap → filter disabled for connection | Configurable per filter; trap on exceed | ABI mismatch → trap |
| K8S | Webhook crash → `failurePolicy: Fail` rejects admission or `Ignore` proceeds | Configurable timeout; same `failurePolicy` applies | Same |
| OPA | Engine error → fail open or closed (deployment choice) | Configurable | Eval error |
| CRX | Service worker crash → restarted; ongoing request may not complete | declarativeNetRequest is declarative; no runtime per-rule | N/A |
| VSC | Extension crash → reported to user; affected commands fail | N/A | N/A |
| TAURI | Plugin panic → IPC call returns error; app continues | Per-command | N/A |
| **ICLAW** | **`catch_unwind` per hook; failure_policy matrix: Gate=FailClosed, Observer=FailIsolated, Mutator=FailIsolated, Effect=FailClosed; poison scoped to the dispatcher's lifetime (per-host-build when `with_hook_dispatcher_factory` is used; shared across all builds for the legacy `with_hook_dispatcher` adapter)** | **`tokio::time::timeout` per hook; same policy matrix** | **AttenuationViolation = FailClosed** |

**Observation:** The `failure_policy` matrix — different defaults for different *kinds* of hooks at the same point — is unusual. K8S has a single `failurePolicy` per webhook config. Envoy has per-filter trap behavior but not differentiated by what the filter was doing.

**Divergence:** ICLAW's "Gate failures FailClosed, Observer failures FailIsolated" is the right call: a crashed gate is unsafe (you can't tell whether it would have allowed), but a crashed observer just loses telemetry for one event. LSM gets this wrong (panic on bug = no syscall mediation at all). K8S gets this right but only on operator say-so. ✓

**Divergence:** Poison sticks for the dispatcher's lifetime. With the recommended `with_hook_dispatcher_factory` path that scope is one host build — the next run starts with a fresh dispatcher and the poison is gone. With the legacy `with_hook_dispatcher` adapter the dispatcher is shared across every build the factory produces, so poison persists for the process lifetime. K8S retries failed webhooks per request; ICLAW does neither. Why ICLAW diverges: in an agent loop, a hook that's panicking repeatedly is more likely buggy than transiently faulty, and retrying it makes the loop unobservable. The cost is operator action (legacy: process restart or hook reinstall; factory: just wait for the next run) to recover. Worth documenting as a known property. ✓

---

### Axis 6 — Isolation unit (where does the hook execute)

| Sys | Unit |
|---|---|
| LSM | Same kernel address space — no isolation |
| EBPF | Same kernel, but verifier-bounded (no unbounded loops, no arbitrary memory) |
| ENVOY | proxy-wasm: per-filter wasm VM, sandboxed; native C++: same process |
| K8S | Out-of-process (separate webhook service), network-isolated |
| OPA | Sidecar process (typical) or in-process library (advanced) |
| CRX | Service worker (separate JS context); content scripts in page context with isolated world |
| VSC | Extension host process (separate Node.js process per workspace) |
| TAURI | In-process Rust plugin (trusted) or webview JS (sandboxed) |
| **ICLAW** | **In-process Rust** (Builtin/Trusted/Installed-predicate); **WASM sandbox** stubbed for Installed-WASM hooks |

**Observation:** Out-of-process isolation (K8S, OPA, VSC) is the gold standard for buggy/untrusted hooks but adds latency + operational complexity. In-process with type-level sealing (ICLAW for now) is acceptable while hook authors are trusted; becomes a problem when third-party Installed hooks ship.

**Divergence:** Installed-WASM execution is **stubbed** in v1; runtime Installed hooks are predicate-language only (no arbitrary code). Why: a typed predicate language is small enough to audit by hand (and is what we have); WASM execution adds wasmtime as a dependency surface and a new isolation boundary we'd want a separate threat model for. ✓ (deferred deliberately)

---

### Axis 7 — Manifest / declaration

| Sys | How hooks are declared |
|---|---|
| LSM | C registration call at kernel init |
| EBPF | BPF program loaded via syscall; attach point in syscall args |
| ENVOY | Static config or xDS; filter chain in listener YAML |
| K8S | `ValidatingWebhookConfiguration` / `MutatingWebhookConfiguration` CRDs |
| OPA | Policy bundles loaded from disk/HTTP/OCI |
| CRX | `manifest.json` at extension install |
| VSC | `package.json` `contributes` section |
| TAURI | `tauri.conf.json` permissions + per-capability `.toml` files |
| **ICLAW** | **`[[hooks]]` table in extension manifest with id, version, attach point, phase, priority, scope, body (Predicate \| Wasm), trust class derived from extension trust** |

**Observation:** ICLAW's manifest shape is closest to **K8S `ValidatingWebhookConfiguration`** in fields (id, scope/selector, failure policy) and closest to **Tauri permissions** in being shipped *with the extension* rather than installed by the operator. This is the right hybrid for an agent runtime where extensions are user-installed but operate on user data.

**Divergence:** `HookId` is content-addressed (blake3 of `extension_id || hook_local_id || hook_version || extension_version`). K8S uses operator-chosen names; CRX uses extension-id + listener-name. Why: content addressing makes duplicate-installation detection automatic, and it makes the milestone audit log uniquely identify the *exact bytes* of the hook that fired. Cost: hook IDs are 64-char hex strings, not human-friendly. ✓

---

### Axis 8 — Telemetry / audit

| Sys | Audit substrate |
|---|---|
| LSM | audit subsystem (auditd) for `LSM_AUDIT_*` events; per-LSM optional |
| EBPF | perf ring buffer / bpf_trace_printk; Tetragon emits structured events to userspace |
| ENVOY | Access logs + stats; per-filter metrics |
| K8S | API server audit log records admission outcome |
| OPA | Decision logs (structured) — opt-in |
| CRX | None standardized; chrome://extensions logs |
| VSC | None standardized; extension can write its own |
| TAURI | None standardized |
| **ICLAW** | **`HookDispatched` / `HookDecisionEmitted` / `HookFailed` milestones on `LoopHostMilestoneSink`; projected into `RuntimeEvent::Hook*` for durable audit; L3 schema snapshots + L4 pairing-invariant matrix tests** |

**Observation:** Most extension systems (CRX, VSC, Tauri) ship *no standard* audit substrate, which is one reason third-party extensions are hard to trust. The systems that *do* (OPA decision logs, K8S audit, Tetragon events) are the systems people trust for high-stakes deployments. ICLAW lining up with the *trusted-substrate* group is the right call.

**Divergence:** Pairing-invariant matrix test (every dispatch outcome ⇒ exactly one Dispatched + one terminator). No prior-art system documents this property as a test. Why ICLAW has it: "LLM data is never deleted" project rule means hook decisions must be reconstructable from the event log alone; dropped or duplicated emissions break that property silently. ✓✓

---

## Where IronClaw stands out (vs prior art)

1. **Type-level trust enforcement.** No survey system enforces "Installed cannot mint Allow" via sealed enum variants + tier-specific installers. The closest analog is Pony's reference capabilities — a different domain but the same insight that *unforgeable distinctions belong in the type system*.
2. **Phase-ordered dispatch with stable tiebreakers.** OPA has analogous ordering but only inside one engine; LSM has stacking but flat. ICLAW's phase layer externalizes ordering concerns from hook authors in a way few systems do.
3. **Dispatch-time manifest-scope enforcement** (not install-time only).
4. **Failure-kind matrix** (Gate=FailClosed vs Observer=FailIsolated at the same point).
5. **PauseApproval/PauseAuth as first-class decisions** with gate-ref minting.
6. **Pairing-invariant audit matrix** as a regression test, not just a design claim.
7. **Tenant-keyed predicate state** + per-build dispatcher (full tenant isolation for in-memory predicate counters).

## Where IronClaw is conventional (and should be)

1. **Phases (Validation→Authorization→Policy→Telemetry)** — same shape as Envoy filter phases and K8S admission stages.
2. **Allow/Deny + reason** — same as LSM, K8S, OPA.
3. **Mutator hooks emit patches** — same as K8S mutating admission.
4. **Manifest-declared attach point** — same as K8S, CRX, Tauri.
5. **Content-addressed identity** — same conceptual primitive as Git object IDs or OCI image digests.

## Where IronClaw is conventional but **shouldn't** be (open questions)

1. **In-process execution for Installed hooks.** K8S/OPA/VSC isolate untrusted code out-of-process; ICLAW keeps Installed hooks in-process and relies on predicate-language audit-by-hand for safety. This is fine while there's no Installed-WASM path. **Once Installed-WASM lands, revisit out-of-process or VM-per-extension isolation.**
2. **Sticky poison.** Scope depends on factory choice: per-host-build with `with_hook_dispatcher_factory` (recommended), process-lifetime with the legacy `with_hook_dispatcher` adapter. K8S retries; ICLAW does neither within a scope. Right call for now, but document the failure mode for operators.
3. **No formal model of the dispatch invariants.** OPA has Rego semantics; LSM-BPF has the verifier. ICLAW has tests. A short typed-state-machine spec for dispatch (states: Idle → Dispatching → DecisionEmitted/Failed → Quiescent) would close the loop. **TODO — add to design doc.**
4. **No per-tenant rate limit on hook *installation*.** If an Installed extension can register N hooks, a malicious extension can flood the dispatcher. Cap N somewhere reasonable. **TODO — add to manifest validator.**

## Where IronClaw diverges but the *why* is weak (review needed)

1. **`HookDispatchOutcome` is not retriable.** Once a gate denies, the loop has no native primitive to retry-with-context — the user has to re-issue. K8S admission has the same property and it's broadly considered correct, so probably fine, but worth confirming this is what we want for agent loops specifically.
2. **No "soft deny" / "advisory" decision.** OPA distinguishes deny (block) from warn (annotate, allow). ICLAW collapses both into Deny + reason. If we ever want to surface "the policy is uneasy but didn't block," we'd need a new outcome. Probably correct to defer, but log it.
3. **Telemetry-phase observers run even on Gate denial.** Defended above (audit value); confirm with operators that this matches the mental model of "what fired during this turn."

## Methodology notes

- Survey was constrained to systems with a public design doc / source. Closed systems (proprietary RASP/EDR products) likely have closer analogs but aren't useful as cite-able prior art.
- Axes were chosen *after* drafting IronClaw's design, so the table is unavoidably colored by ICLAW's vocabulary. A second pass with axes chosen from one of the survey systems (e.g., K8S's admission-controller checklist) would be a useful adversarial check.
- Each row in the matrix should be independently verified against current docs — kernel/Envoy/K8S/OPA APIs all evolve, and this snapshot is May 2026.

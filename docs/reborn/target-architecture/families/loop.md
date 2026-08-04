# `crates/loop/` — the loop-hosting tier

**Layer(s):** loops (all four crates) · **Crates:** 4 — `ironclaw_agent_loop`, `ironclaw_loop_host`, `ironclaw_turn_runner`, `ironclaw_hooks` · **Security posture:** owns no authority of its own — every privileged effect crosses a `loop_contracts` port into the kernel; this family's own decorators restrict or observe what the kernel has already granted, and never expand it.

*This document specifies the target architecture as designed. Dispositions, migration constraints, evidence, and open decisions live in [PROPOSAL.md](../PROPOSAL.md), [CHECKLIST.md](../CHECKLIST.md), and [PLAN.md](../PLAN.md).*

```text
crates/loop/
├── ironclaw_agent_loop    the canonical loop: executor & sealed strategies
├── ironclaw_loop_host     host-port adapters over kernel services
├── ironclaw_turn_runner   drivers & the agent-turn executor
└── ironclaw_hooks         trust-tiered hook middleware
```

## Role

`crates/loop/` hosts replaceable agent behavior and the adapters that connect it to the kernel. `ironclaw_agent_loop` is the sealed strategy-and-executor framework: it decides what a turn does next. `ironclaw_loop_host` implements every host-port contract that framework calls, over kernel services. `ironclaw_turn_runner` is the agent-turn executor, the driver registry, and the loop-host factory, registered as the executor the process supervisor invokes for turn-shaped work. `ironclaw_hooks` is the trust-tiered hook framework wrapping every port call with policy and audit. Together, the four let an agent's behavior be replaced or extended without any of the replacement code ever touching a privileged handle directly.

## Boundaries — what makes this family distinct

- **vs kernel** — userland vs authority. A shipped loop is not trusted merely because it ships with the product: the kernel secures the loop's environment and does not rely on the loop to preserve kernel invariants. Nothing in this family can construct a capability grant, an approval, a lease, or a secret; it can only request one through a port and receive the sealed outcome.
- **vs contracts (`loop_contracts`)** — ports are defined there, implemented here. `loop_contracts` names the shape of every capability, model, prompt, transcript, context, input, run-info, cancellation, compaction, progress, and checkpoint port; this family supplies every concrete implementation, directly or by decorating another implementation in the same family. No port is ever implemented inside `loop_contracts` itself.
- **vs product** — conversation and admission UX vs execution strategy. Product owns binding, idempotency, and command grammar; this family owns only how a claimed run's next step is decided and executed. Product reads a claimed run's projected outcome; it never reaches into this family for behavior.

## What belongs here / What never belongs here

**Belongs:** the sealed strategy-and-executor framework and its resumable state; host-port adapter implementations over kernel services; the claimed-work control plane — driver registry, executor registration; trust-tiered hook middleware wrapping the port set. **Never belongs:** any authority decision (trust, authorization, approval, resource reservation, dispatch); product workflow or binding; vendor protocol of any kind; a raw secret, filesystem handle, network client, or process handle; anything beyond a ref, cursor, counter, version, or safe summary in resumable state — never a raw prompt, raw model output, tool argument, secret, host path, or provider diagnostic.

## Dependency direction

`loop/` crates depend on `contracts/` (`host_api`, `common`, `loop_contracts`) and, for the host-adapter and control-plane crates, on the kernel services they wrap and the domain crates their adapters need. `ironclaw_agent_loop` alone depends on `contracts/` and nothing else. No crate in this family depends on `products/` or `app/`.

**The single sanctioned `Loop*Port` decorator chain, declared:** `ironclaw_loop_host` implements the base, kernel-facing adapter for every port directly over kernel services. `ironclaw_turn_runner` composes that base adapter into the concrete host it hands to each claimed run, adding its own driver-facing and capability-surface-tracking layers. `ironclaw_hooks` wraps the composed host outermost, so every port call is policy-checked and logged before it reaches `runner`'s composition and `loop_host`'s kernel-facing base beneath it. No other crate, inside or outside this family, implements a `Loop*Port`.

## Security & authority

A claimed run receives only the ports scoped to its own execution — never an ambient handle to any other run's state, and never a capability, secret, or dispatch handle directly. Every port call that requests a privileged effect is authorized, approved where required, and mediated by the kernel before it executes; the loop consumes only the sealed, redacted outcome. A loop's claimed exit is treated as a claim, not a fact: the outcome is validated against host-minted evidence before any durable transition commits, so a defective or adversarial loop cannot assert a successful result into existence. Inside the family, `ironclaw_hooks`' trust classes are fixed by source and never declarable: a hook cannot grant authority, cannot bypass authorization, approval, runtime policy, resource, secrets, filesystem, or network policy, and never receives an ambient secret, filesystem handle, network client, or process handle. Gate and mutator decisions fail closed; observer and effect decisions fail isolated with redacted audit; a hook that violates its own protocol is barred from acting for the remainder of that run.

## Crates

### `ironclaw_agent_loop`

- **Purpose:** the canonical, sealed loop-family and strategy-executor framework — the one artifact in the system meant to be replaced wholesale without touching anything privileged.
- **Owns:** loop-family identity and the strategy registry; the planner service and its built-in strategy composition; the canonical executor and its ordered lifecycle stages; resumable execution state limited to refs, cursors, counters, versions, and safe summaries.
- **Never contains:** any kernel, host-runtime, or product dependency of any kind; raw prompts, raw model output, tool arguments, secrets, host paths, or provider diagnostics anywhere in state; a strategy trait exposed outside the crate; a driver-facing type — the framework never sees a driver, only its own executor contract.
- **Public surface:** the canonical executor and the loop-family registry, consumed by `ironclaw_turn_runner`; consumes the full `loop_contracts` port set.
- **Depends on:** `common`, `host_api`, `loop_contracts` — contracts only.
- **Never depends on:** any substrate, domain, kernel, lanes, extensions, product, or app crate.
- **Security & authority role:** none. No privileged type is importable from this crate's dependency set, so "a shipped loop is not trusted" is a fact the compiler enforces rather than a review convention.
- **Why a separate crate:** it is the one crate whose entire dependency graph must stay swappable without touching authority; isolating it to contracts-only dependencies is what makes that guarantee mechanical.

### `ironclaw_loop_host`

- **Purpose:** the concrete implementation of every `loop_contracts` port over kernel services — the one crate licensed to hold both port types and kernel handles in the same module.
- **Owns:** the base capability-port adapter and its capability-surface-filtering decorators — including **progressive tool disclosure** (the catalog/selector, the deferring `LoopCapabilityPort` decorator, and its `REBORN_TOOL_DISCLOSURE` switch); the subagent-spawn port; the budget accountant; the checkpoint-state store; the input queue, cancellation port, and the **driver-host checkpoint/progress/no-extra-input port adapters**; the **model-gateway port adapter and the model-route policy vocabulary it resolves against**; identity, skill, and memory prompt-context builders that produce safe summaries and refs for the prompt port to assemble. ✎ **2026-08-03 (WS6):** also the **system-prompt content assets** — `prompts/{default_system,tool_disclosure_protocol,self_knowledge,benchmarking_mode}.md`, exported from `system_prompt_assets.rs`. They arrived from the composition root, whose charter (`families/app.md`: *"prompt content of any kind"* never belongs) already excluded them; what stays in the app family is assembly plus the boot-time seeding of the on-disk `SYSTEM.md`, which is `std::fs` work this crate does not do.
- **Never contains:** a provider client or dispatcher internal beyond what a single port adapter strictly needs — the model-gateway adapter is that single adapter, and no other module here may reach a provider client; product binding; a database migration; driver registration; the *ordering* of the decorator chain (the turn runner composes it — see "Dependency direction"); a decorator that performs a turn-lifecycle state transition — that belongs to the turn-admission crate's own runner-facing seam, never to a port adapter.
- **Public surface:** implementations of every `loop_contracts` port, consumed by `ironclaw_turn_runner` and, through its composition, by the executor.
- **Depends on:** `ironclaw_capabilities`, `ironclaw_host_runtime`, `ironclaw_resources`, `ironclaw_turns`, `ironclaw_safety`, `ironclaw_skills`, `ironclaw_memory`, `ironclaw_threads`, `loop_contracts`, and — for the model-gateway adapter alone — `ironclaw_llm` (`default-features = false`) and `ironclaw_common`.
- **Never depends on:** `ironclaw_assistant`, `ironclaw_extension_host`, `ironclaw_extension_manager`, or any package crate.
- **Security & authority role:** the concrete membrane implementation — every privileged effect a loop requests passes through an adapter defined here before it reaches the kernel. It must never bypass the capability host or the dispatcher.
- **Why a separate crate:** it is the only place kernel handles and `loop_contracts` types are permitted to coexist; keeping it apart from `ironclaw_agent_loop` is what keeps that crate's contracts-only rule true, and keeping it apart from `ironclaw_turn_runner` keeps port-adaptation concerns separate from driver-registry and claim-control-plane concerns.

### `ironclaw_turn_runner`

- **Purpose:** the agent-turn executor, the driver registry, and the loop-host factory — the trusted adapter between a kernel-claimed run and loop userland, registered as the executor the process supervisor invokes for turn-shaped work.
- **Owns:** the driver registry and its readiness validation; the two production drivers — a planned driver that adapts `ironclaw_agent_loop`'s families and executor to the driver contract, and a text-only driver for the smallest supported behavior; the loop-host factory assembly that composes a claimed run's port set; failure-lane and retry disposition; submitting the loop's claimed outcome to the turn kernel's exit applier, which validates it before anything durable commits.
- **Never contains:** a planner strategy slot exposed through a public API; a neutral vocabulary type duplicated from a contracts crate; a product-specific branch inside a driver; a silent fallback between drivers that is not an explicit profile or readiness decision.
- **Public surface:** the driver registry; the two production driver implementations; the turn-execution entry point the process supervisor invokes.
- **Depends on:** `ironclaw_agent_loop`, `ironclaw_loop_host`, `ironclaw_hooks`, `ironclaw_host_runtime`, `ironclaw_memory`, `ironclaw_threads`, and `ironclaw_turns`. **Not `ironclaw_llm`** — the provider cone left with the model gateway (WS3 runner sheds); re-adding that edge means provider behaviour came back with it.
- **Never depends on:** `ironclaw_assistant`, `ironclaw_extension_host`; never `ironclaw_composition` — the dependency runs the other way, composition depends on this crate, never the reverse.
- **Security & authority role:** the trusted control-plane adapter. It is handed a claimed run under a lease and hands that run only the ports scoped to it; it never decides durability itself — the turn kernel's exit validation does, and the runner only carries the claim to it.
- **Why a separate crate:** it is the one crate trusted to bridge a kernel-issued claim into loop userland; keeping its charter to exactly that job means the kernel's only reach into this family is a single, well-defined executor registration, not a same-tier dependency.

### `ironclaw_hooks`

- **Purpose:** the trust-tiered hook framework — four fixed-by-source trust classes, sealed decision sinks, and the outermost decorator layer wrapping every `loop_contracts` port call.
- **Owns:** the four trust classes — builtin, trusted, installed, and self-authored — each fixed by the hook's source and never declarable by the hook itself; the sealed decision sinks and decision kinds each class is restricted to; ordering and failure-policy rules; the declarative predicate language and its evaluator; a sandboxed execution engine for portable hook code; the decorator implementation of every `loop_contracts` port.
- **Never contains:** the driver contract itself — that stays in `loop_contracts`; a base, non-decorating implementation of any port — this crate only ever wraps another implementation; extension-bundle loading or installation authority — a hook executes once its code is resolved, but resolving and sourcing that code is someone else's job.
- **Public surface:** the hook dispatcher and its trust-tier-specific installers; the hook registry; decorator implementations of the full `loop_contracts` port set.
- **Depends on:** `ironclaw_event_log`, `host_api`, `prompt_envelope`, `loop_contracts`, `ironclaw_wasm_limiter`, and, under the family's documented persistence exception, the backends of its predicate store.
- **Never depends on:** `ironclaw_turns` — the dependency runs the other way, nothing in the turn-admission kernel crate depends on this crate; never `ironclaw_assistant`, `ironclaw_extension_host`, or the dispatcher.
- **Security & authority role:** a hook cannot grant authority or bypass any kernel-mediated policy stage, and never receives an ambient secret, filesystem handle, network client, or process handle. Gate and mutator decisions fail closed; observer and effect decisions fail isolated with redacted audit; a hook that violates its protocol is barred from acting for the rest of the run.
- **Why a separate crate:** an independent trust-tier contract with its own sandboxed execution engine, kept apart from `ironclaw_loop_host`'s non-sandboxed adapters so neither carries a dependency the other has no need for.

## Family AGENTS.md requirements

`crates/loop/AGENTS.md` must carry, verbatim or by direct reference:

- **The loop trust story** — a shipped loop is not trusted; ports are the membrane; nothing in this family constructs authority, it only requests it.
- **The sealed-strategy pattern as house style** — new loop behavior is a new strategy behind the sealed family-and-planner composition, never a branch inside the canonical executor.
- **The single declared `Loop*Port` decorator chain** — this document's "Dependency direction" section is the reference; a new port implementation anywhere in the workspace joins that declared chain in the same change, never as a parallel, undeclared decorator.
- **The crate-boundary-must-be-earned gate** — a fifth crate in this family must name its own role in the claim-to-execution path and its own trust seal or multi-implementation port; it is not warranted by convenience alone.
- **The dependency-direction rule, restated as a check** — this family depends on contracts and kernel; it never depends on products or app; nothing in the kernel depends back into this family except through a single registered executor.

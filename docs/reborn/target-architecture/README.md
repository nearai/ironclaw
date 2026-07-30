# IronClaw Reborn — Target Crate Architecture (Executive Overview)

**Status:** Proposal, under review · **Baseline:** `origin/main` @ `dde662d5a` (2026-07-29)
**Documents:** this overview · [PROPOSAL.md](PROPOSAL.md) (full evidence-backed specification) · [families/](families/) (one in-depth file per top-level family) · [CHECKLIST.md](CHECKLIST.md) (definition of done) · [PLAN.md](PLAN.md) (how to execute) · [explorer.html](explorer.html) (interactive map + dependency graph — self-contained, open in any browser)

This is the north star for the architecture cleanup: a concrete crate/folder map with explicit security and authority boundaries, so that agents and humans can answer "where does this go?" without archaeology, and so the refactor train has a fixed destination instead of a direction.

---

## What this proposes

Today `crates/` is a flat list of sixty-odd crates. The architecture underneath is actually in good shape — there are real dependency rules between the crates, and CI enforces them — but none of that is visible from the tree, and over the past months code has pooled into a handful of oversized crates that own far more than their names say. Figuring out where anything belongs takes archaeology, and every agent or new contributor pays that tax on every change.

The proposal is to reorganize the workspace into the shape the architecture already wants: **ten families, stacked in one direction.** User-facing code sits at the top, shared vocabulary at the bottom, and in the middle sits the **kernel** — the code that decides what is allowed to happen. Every crate sits on a seven-layer ladder — shared vocabulary at the bottom, the binary at the top — and may only depend on crates at or below its own layer. CI enforces exactly that matrix today; the ten families are those layers made visible on disk. That single ladder *is* the architecture, and once the tree mirrors it, the tree tells the story by itself: the app wires everything together, the product asks the kernel for privileged work, the kernel checks and mediates every privileged action, and everything below it is mechanism and vocabulary with no authority of its own.

Getting there is mostly reorganization, not rewrite:

- **Move** most crates into their family folder unchanged.
- **Add three small contracts crates** holding the shared interfaces for the loop, for extensions, and for the product surface — so lower layers stop reaching *upward* to borrow types, which is where most of today's dependency tangles come from.
- **Slim the four oversized crates** (composition, the extension host, the host runtime, the runner) by moving what they absorbed back to the crates that actually own it.
- **Delete code that is verifiably dead**, and fold a few single-purpose fragments into the crate they serve.
- **Give every installable extension one self-contained folder** — manifest, assets, and code together, with vendor-specific code allowed nowhere else.

The runtime security mechanisms are preserved — the same checks run in the same order — and the three deliberate boundary *tightenings* this proposal does make (evidence-mint consolidation, secrets narrowing, host/verifier colocation) are named and individually risk-tracked rather than slipped in silently. What changes is that ownership becomes obvious, the dependency rules become visible, and the exceptions to those rules that we currently tolerate get removed structurally instead of waived indefinitely.

## Why this shape

1. **The rules already exist — the layout just hides them.** Every dependency rule below is already enforced in CI today; the flat tree is what makes them invisible. Families make the enforced model legible, and the handful of currently-waived rule violations each become fixable instead of grandfathered.
2. **The problem is vocabulary in the wrong place, not missing concepts.** The audit found shared types and interfaces that migrated into whichever crate everything could already see, forcing upper and lower layers into each other. The fix is giving that vocabulary proper homes in the contracts tier — not inventing new layers.
3. **The oversized crates split along seams history already drew.** Each of the four grew by absorbing other owners' code in identifiable steps, so each split is a return to a known owner — not a judgment call made fresh.
4. **Moves are cheap; splits are few.** Family placement is a rename plus manifest edits. Only a handful of crates genuinely split — three true splits (host_api, turns, the extension host) plus the mass evictions from composition, the host runtime, and the runner — each along a seam with existing tests on both sides.

## The family map

Ten families under `crates/`, shown with the crates each one holds — enough to get a sense of what lives where. Crate names are shown without their `ironclaw_` prefix. The full contract for every crate (what it owns, what it must never contain, its dependencies) is in that family's spec file, linked below. Naming follows one written rule (PROPOSAL §5.1): a crate is named for what it *is*, never namespaced by its family — the directory does that job. Crates tagged `NEW` exist only in the target; the map shows the steady state (one transitional crate, `run_state`, remains under `kernel/` until its scheduled deletion — PROPOSAL §5).

```text
crates/
│
├── contracts/                 neutral vocabulary & ports — the leaf tier
│   ├── host_api               authority vocabulary & sealed witnesses
│   ├── common                 cross-domain primitives
│   ├── prompt_envelope        untrusted-snippet envelope
│   ├── loop_contracts         the loop ↔ kernel port set              NEW
│   ├── extension_contracts    extension surfaces, adapters, recipes   NEW
│   └── product_contracts      ProductSurface, DTOs & product ports    NEW
│
├── substrates/                privileged mechanisms the kernel mediates
│   ├── filesystem             storage fabric: mounts, containment, CAS
│   ├── secrets                secret custody & one-shot leases
│   ├── network                egress policy & hardened transport
│   ├── safety                 scanning & redaction primitives
│   └── observability          latency-trace macros
│
├── events/                    evidence → derived views → streams
│   ├── event_log              evidence vocabulary & log traits
│   ├── event_store            durable backends, fail-closed profiles
│   ├── event_projections      replay-derived read models
│   └── event_streams          admission-checked stream delivery
│
├── domains/                   typed record & service owners
│   ├── threads                canonical transcripts
│   ├── conversations          external ↔ canonical binding, idempotency
│   ├── triggers               schedules & trusted-fire minting
│   ├── memory                 provider-neutral memory contract
│   ├── skills                 skill parsing, selection, learning
│   ├── auth                   product auth & the recipe engine
│   ├── attachments            attachment landing & its ports
│   ├── extractors             pure bytes → text extraction
│   ├── identity               external identity → stable UserId · projects & ACL
│   ├── llm                    provider contract & reliability stack
│   ├── trace_commons          Trace Commons client & redaction
│   └── outbound               outbound authority: sealed grants
│
├── kernel/                    the authority perimeter — one crate per stage
│   ├── trust                  requested → effective trust ceilings
│   ├── authorization          default-deny grants & leases
│   ├── approvals              exact-invocation consent
│   ├── resources              reservation & quota accounting
│   ├── runtime_policy         pure policy resolution & lane planning
│   ├── capabilities           the CapabilityHost membrane
│   ├── processes              lifecycle journal & supervisor
│   ├── turns                  turn admission & exit validation
│   └── host_runtime           mediated services & the lane executor
│
├── lanes/                     execution for already-authorized work
│   ├── wit/                   component-model interface definitions
│   ├── wasm                   WASM component sandbox
│   ├── wasm_limiter           shared wasmtime resource limiter
│   ├── mcp                    MCP over host-mediated HTTP
│   └── sandbox                container process lane                  NEW
│
├── loop/                      agent behavior & its hosting
│   ├── agent_loop             the canonical loop, sealed strategies
│   ├── loop_host              port adapters over kernel services
│   ├── turn_runner            drivers & the agent-turn executor
│   └── hooks                  trust-tiered hook middleware
│
├── extensions/                everything "installable package"
│   ├── extension_registry     manifests & installation records
│   ├── extension_host         generic host: verify, bind, deliver
│   ├── extension_manager      product-side management                 NEW
│   ├── extension_support      shared native executors & the package inventory
│   └── packages/              one self-contained dir per package
│       ├── slack/             adapter crate + manifest + assets
│       ├── telegram/          adapter crate + manifest + assets
│       ├── github/            manifest + wasm + prompts (data only)
│       ├── memory-native/     native memory provider (crate)
│       ├── mem0/              mem0-backed memory provider (crate)
│       └── …                  gmail, google-*, web-access, notion-mcp, …
│
├── product/                   first-party userland
│   ├── assistant              the assistant: ProductSurface impl & delivery
│   ├── operator               deployment-operator control plane
│   ├── openai_compat          OpenAI-compatible ingress adapter
│   ├── webui                  web host: routes, auth, gateway, SPA
│   └── host_ingress           route-mount carriers
│
└── app/                       assembly & enforcement
    ├── composition            the assembly root: selection & wiring
    ├── cli                    the binary `ironclaw`
    ├── config                 boot contract & config.toml schema
    └── architecture_tests     mechanical enforcement tests

tools/                         developer diagnostics & excluded helpers
```

## The ten families

**[`contracts/`](families/contracts.md).** The shared vocabulary and ports every tier may see: identities and scopes, the capability/decision/approval vocabulary, the sealed `Authorized` witness and dispatch port, and the loop, extension, and product port sets. Nothing here executes, persists, or names a vendor — a contracts crate defines shapes and seals constructors; implementations always live above it.

**[`substrates/`](families/substrates.md).** The privileged mechanisms the kernel mediates: the storage fabric with mount containment and CAS, encrypted secrets with one-shot leases, hardened network policy and egress, safety scanning and redaction, and the tracing macros. Substrates enforce local invariants but never make authority decisions and never hold domain records or product behavior.

**[`events/`](families/events.md).** What already happened, kept in three deliberately separate contracts: canonical redacted evidence (vocabulary plus durable backends with fail-closed production profiles), rebuildable read models derived by replay, and transport-neutral streams with admission control. Projections and streams can never write state or become authority, and no transport framing lives here.

**[`domains/`](families/domains.md).** The typed record and service domains behind the kernel — threads, conversations, triggers, memory, skills, auth, attachments, identity (with its project records), llm, traces, outbound, and friends — each owning its record grammar and invariants over a `ScopedFilesystem`. Domains never select storage backends, never decide authority, and never expose HTTP; vendor code appears only under the two chartered exceptions (llm providers, auth recipes-as-data).

**[`kernel/`](families/kernel.md).** The security perimeter, one crate per stage of the mediated effect pipeline: trust ceilings, grant matching and leases, exact-invocation approvals, resource reservation, runtime policy, the `CapabilityHost` membrane, the process lifecycle authority, turn admission, and the mediated host services. The kernel decides and mediates; it contains no product UX, no loop strategy, no vendor code, and no lane mechanics.

**[`lanes/`](families/lanes.md).** How already-authorized work executes: the WASM sandbox, MCP, and the container sandbox, each receiving sealed invocations and mediated services and returning normalized outcomes. A lane never authorizes anything and never holds ambient network or secrets.

**[`loop/`](families/loop.md).** Replaceable agent behavior and the adapters that host it: the sealed executor and strategy families, the host-port implementations, the driver registry, and the hook middleware. A shipped loop is not trusted — everything privileged crosses ports into the kernel.

**[`extensions/`](families/extensions.md).** Everything "installable package," with the four responsibilities kept apart: the manifest registry and installation records, the generic host (ingress verification, binding, egress transport), the product-side manager, and the concrete packages themselves. Vendor names exist only under `packages/`; the registry never dispatches, and the host never carries product workflow.

**[`product/`](families/product.md).** The supported first-party experience: the `ProductSurface` implementation, admission/bindings/idempotency, delivery semantics, the operator control plane, and the transports (WebUI, OpenAI-compatible). Product asks the kernel for privileged work — it never decides authority, never touches lane mechanics, and never speaks a vendor protocol.

**[`app/`](families/app.md).** Assembly and enforcement: composition selects deployments and wires owners, the binary supplies the concrete binding tables, config owns the boot contract, and the architecture tests keep all of the above mechanical. Nothing else in the workspace may contain wiring, and app may contain nothing but wiring — no domain behavior, no policy content, no prompts, no vendor flows.

### Three that sound alike

`domains/`, `loop/`, and `lanes/` are the easiest to confuse, and the difference is the whole model:

- **`domains/` is what the system *knows*** — passive, typed owners of records (transcripts, triggers, memory, projects, delivery state). They store and serve; they never run agent logic, never execute anything, and — outside three narrow named authorities (outbound's sealed delivery grants, triggers' trusted-fire minting, projects' membership ACL) — never decide permissions.
- **`loop/` is what the agent *decides*** — assemble the prompt, call the model, pick a tool, retry, checkpoint. Deliberately untrusted: it can only *request* effects through typed ports; it cannot touch storage, network, or secrets itself.
- **`lanes/` is how an approved action *physically runs*** — the WASM sandbox, the MCP client, the container sandbox. A lane doesn't think and doesn't decide; it executes an already-authorized invocation in isolation and returns a normalized result.

One request stitches them together: a message arrives → the **loop** decides to call `github.search` → the **kernel** checks and authorizes it → a **lane** executes it in a sandbox → the outcome lands in **domains** (the transcript) and **events** (the record of what happened). *Loop chooses, kernel permits, lane runs, domains remember.*

### Why ten — and where reasonable people could disagree

The seven-layer ladder is the load-bearing choice; the ten families are how it reads on disk, and two of the groupings are judgment calls worth debating up front rather than discovering later:

- **`events/` could fold into `domains/`.** Both sit at the substrates layer. It stays separate because evidence has a different write model from records — append-only, replay-derived, never authority — and its four crates form one strict one-way pipeline. Merging down to nine families would not change a single dependency rule; it would only blur that write-model line on disk.
- **`extensions/` is the one deliberately *vertical* family.** Its crates span three layers (the registry at substrates, the host at loops, the manager and packages at products) because colocating everything "installable package" is what makes rules like "vendor names only under `packages/`" checkable in one place. It is the one spot where family and layer diverge — and the ladder, not the family, is what CI checks.

Every other cut is firm: substrate vs domains is mechanisms vs records; the kernel stays one family with one crate per pipeline stage (a family per stage would add directory depth and no new rule); loop vs lanes is untrusted strategy vs post-authorization mechanism — they sound alike and are enforced apart.

## The five decisions reviewers should weigh

1. **Three new contracts crates** — the load-bearing change. They exist because the dependency graph *proves* the need: `loop_contracts` (six crates consume the loop-port tier through the turn kernel today), `extension_contracts` (lanes/hosts consume adapter+manifest vocabulary through the registry or product today), `product_contracts` (ports defined in product force extension_host/operator/telegram *above* product today). Each is thin, allowlisted, and mass-ratcheted.
2. **Six layer reassignments, zero matrix changes** — `extensions` (becoming `extension_registry`)→substrates, `skills`→substrates, `extension_host`→loops, `runner`→loops, `hooks`→loops, `processes`→kernel. Together with the contracts crates, these make every current exception's `removes_in` condition true.
3. **Kernel = nine crates, on purpose.** Each pipeline stage (trust → authorization → approvals → resources → policy → capability membrane → lifecycle → admission → mediated services) is an independently consumed contract with fail-closed rules; merging them trades compiler-proven stage separation for module discipline.
4. **Packages are directories first, crates when earned** — a package crate exists iff it has a channel adapter (binary-only linking is already enforced), a provider surface, or a heavy isolated dependency; everything else is manifest+assets+modules. Slack is already the model citizen; Telegram reaches parity by merging its two crates and depending only on contracts. The memory providers follow the same rule: both ship as provider packages (`memory-native/`, `mem0/`) declaring a `[memory]` manifest surface, while the provider-neutral contract they implement stays in `domains/`.
5. **Deletion list is verified, not vibes** — every dead item was checked for zero production consumers at the baseline commit (full inventory in PROPOSAL.md §2.6), and each deletion still lands under the "removing a redundant layer un-masks behavior" review discipline.

## Security model in three sentences

Untrusted input becomes **validated** at the listener/verifier (webui middleware; manifest-recipe signature verification minting sealed verified-inbound evidence), **authorized** only inside the kernel membrane (`CapabilityHost`: trust ceiling → default-deny grants → exact-invocation approvals → obligations → the sealed `Authorized` witness), and **safe to persist/deliver** only after mediated execution (narrowed mounts, one-shot staged secrets, policy-scoped egress) produces redacted evidence and sealed outbound grants drive at-most-once delivery. Family placement changes none of these mechanisms — it relocates code ownership around them; the three deliberate security-adjacent changes (evidence-mint consolidation, secrets direct-consumer tightening, host/verifier colocation) are individually risk-tracked in PROPOSAL.md §12.1. The full transition-by-transition walkthrough with the mermaid diagram is PROPOSAL.md §7.

## What is explicitly *not* decided here

Open PRs stay open: #6691 (composition builders) is directionally identical but not presumed; #6696 (process-journal collapse) gates four mapping rows (`processes` widening, `run_state` deletion, `approvals` widening, `runner`'s scheduler/await-edge shed) — the target is valid whether or not it lands. Ten genuinely open decisions (prompt-envelope/safety unification, trust's inert signed-registry path, the three-OAuth-stacks question, openai-compat-as-extension, the `reborn_` rename batch, and more) are listed in PROPOSAL.md §12.10 and in each affected crate spec, not silently resolved. PR #6253 (the interactive architecture explorer) models the superseded 2026-07-17 design note; it should be regenerated against this target or closed, in coordination with its author.

## How to review

- Skim this file, then read [PROPOSAL.md](PROPOSAL.md) §1–§5 for the argument and §8.3 for the exception-elimination proof.
- Dive into your family via [families/](families/) — each file is a **forward-looking specification** (the architecture as designed, no migration talk): the hard line around the family, what distinguishes it from each neighbor, and the full contract for every crate in it (purpose / owns / never contains / public surface / dependency direction / security role / why-a-crate). Migration and evidence stay in PROPOSAL/CHECKLIST/PLAN.
- Challenge the [CHECKLIST.md](CHECKLIST.md): it is the definition of done — if you think something is missing, it goes there.
- Argue sequencing in [PLAN.md](PLAN.md): waves, gates, and PR-sizing rules; nothing there is sacred except the ordering constraints called out as load-bearing.

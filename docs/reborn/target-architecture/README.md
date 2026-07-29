# IronClaw Reborn — Target Crate Architecture (Executive Overview)

**Status:** Proposal, under review · **Baseline:** `origin/main` @ `dde662d5a` (2026-07-29)
**Documents:** this overview · [PROPOSAL.md](PROPOSAL.md) (full evidence-backed specification) · [families/](families/) (one in-depth file per top-level family) · [CHECKLIST.md](CHECKLIST.md) (definition of done) · [PLAN.md](PLAN.md) (how to execute)

This is the north star for the architecture cleanup: a concrete crate/folder map with explicit security and authority boundaries, so that agents and humans can answer "where does this go?" without archaeology, and so the refactor train has a fixed destination instead of a direction.

---

## The one-paragraph decision

Keep the capability-based microkernel model and the **mechanically enforced 7-layer ladder exactly as it exists today** (`[package.metadata.ironclaw] layer` + the matrix test in `ironclaw_architecture`). Physicalize ownership as **ten family directories** under `crates/` — discoverability groupings, **never** trust boundaries by themselves. Create exactly **three new contracts crates** (`ironclaw_loop_contracts`, `ironclaw_extension_contracts`, `ironclaw_product_contracts`) where today's dependency graph proves upper-layer vocabulary pooled in the wrong crate. **Narrow the four god crates** (`composition`, `extension_host`, `host_runtime`, `runner`) by moving inventoried behavior to named owners. **Delete the verified-dead surface** (two whole crates plus an itemized list). **Colocate every installable extension package** under `crates/extensions/packages/`. Net effect: 66 → 64 workspace packages, and **all 20 standing `LAYER_MATRIX_EXCEPTIONS` — the repo's own machine-tracked debt register — dissolve with zero new exceptions**.

## Why this shape

1. **The layer model already works; the filesystem doesn't.** The matrix is enforced and violated only through 20 dated exceptions the code itself labels with milestones (`W4.3`, `W6`, `W7`). A flat 65-directory `crates/` hides that model; families make it visible, and the new contracts crates make the exceptions deletable.
2. **The audit found vocabulary pooling, not missing layers.** `host_api` is 38% migrated product/channel vocabulary; `turns` is three crates wearing one name; `product` defines ~17 single-impl ports whose real impls live in composition/extension_host/operator. The fix is contract-layer homes, not new concepts.
3. **God crates decompose along seams the code already names** — extension_host's product-serve half arrived from composition in two identifiable PRs; host_runtime's builtin tools already use a registrar pattern the binary drives; composition's `local_dev/` is the mislabeled production path.
4. **Moves are cheap; splits are few.** Family placement is `git mv` + manifest edits. Only six crates genuinely split, each along a seam with existing tests on both sides.

## The family map

Ten families under `crates/`. The map stays at this altitude on purpose — the crates inside each family are defined in that family's spec file, linked from the table below.

```text
crates/
├── contracts/     neutral vocabulary & ports — the leaf tier every family may see
├── substrate/     privileged mechanism substrates the kernel mediates
├── events/        durable evidence → derived read models → transport streams
├── domains/       typed record/service domains behind the kernel
├── kernel/        the authority perimeter — every stage of the mediated effect pipeline
├── lanes/         execution mechanisms for already-authorized work
├── loop/          loop userland and its hosting adapters
├── extensions/    everything "installable package" — registry, host, manager, packages/
├── product/       first-party userland above the kernel
└── app/           assembly & enforcement — composition, the binary, config, architecture tests
tools/             developer diagnostics & excluded helpers
```

## Family roles and responsibilities (summary)

| Family (spec) | Role | Owns | Must never contain |
|---|---|---|---|
| [`contracts/`](families/contracts.md) — 6 crates | shared vocabulary + ports every tier may see | IDs/scopes, capability/decision/approval vocabulary, the sealed `Authorized` witness + dispatch port, loop/extension/product port sets, wire DTO homes | impls, frameworks, persistence, vendor names, behavior |
| [`substrate/`](families/substrate.md) — 5 crates | privileged mechanisms (storage fabric, secrets, network policy, safety scanning, tracing macros) | containment, CAS, leases, egress hardening, redaction primitives | authority decisions, domain records, product behavior |
| [`events/`](families/events.md) — 4 crates | durable evidence, derived read models, transport-neutral streams | canonical redacted events; backend selection + fail-closed profiles; rebuildable projections; admission-checked streams | transports, product views, any projection *writing* state |
| [`domains/`](families/domains.md) — 15 crates | typed domain owners over `ScopedFilesystem` | record grammar, domain services/factories, sealed domain trust types (outbound grants, trigger minting) | backend selection, authority decisions, HTTP surfaces, vendor branches (llm/auth vendor scope excepted by charter) |
| [`kernel/`](families/kernel.md) — 9 crates | the security perimeter: every stage of the mediated effect pipeline | trust ceilings, grants/leases, exact-invocation approvals, reservations, lane policy, `CapabilityHost`, process lifecycle, turn admission, mediated services | product UX, loop strategy, vendor code, lane mechanics |
| [`lanes/`](families/lanes.md) — 4 crates | how authorized work executes | runtime loading/isolation/metering, normalized outcomes | authorization, ambient network/secrets, product behavior, parallel lifecycles |
| [`loop/`](families/loop.md) — 4 crates | replaceable agent behavior + the adapters hosting it | executor/strategies (sealed), host-port impls, driver registry, hook middleware | authority (everything privileged crosses ports into the kernel) |
| [`extensions/`](families/extensions.md) — 3 crates + `packages/` | the four extension responsibilities, colocated | manifests/registry/records; generic hosting (ingress verify, binding, egress transport); product-side management; concrete packages | vendor names outside `packages/`; registry-as-dispatcher; host absorbing product workflow |
| [`product/`](families/product.md) — 5 crates | the supported first-party experience | ProductSurface impl, admission/bindings/idempotency, delivery semantics, operator control plane, transports (webui/openai) | authority decisions, lane mechanics, vendor protocol, assembly |
| [`app/`](families/app.md) — 4 crates | deployment selection, wiring, the binary, enforcement | builders-of-owners, binding tables, boot config, architecture tests | any domain behavior, policy content, prompts, vendor flows |

## The five decisions reviewers should weigh

1. **Three new contracts crates** — the load-bearing change. They exist because the dependency graph *proves* the need: `loop_contracts` (six crates consume the loop-port tier through the turn kernel today), `extension_contracts` (lanes/hosts consume adapter+manifest vocabulary through the registry or product today), `product_contracts` (ports defined in product force extension_host/operator/telegram *above* product today). Each is thin, allowlisted, and mass-ratcheted.
2. **Six layer reassignments, zero matrix changes** — `extensions`→substrates, `skills`→substrates, `extension_host`→loops, `runner`→loops, `hooks`→loops, `processes`→kernel. Together with the contracts crates, these make every current exception's `removes_in` condition true.
3. **Kernel = nine crates, on purpose.** Each pipeline stage (trust → authorization → approvals → resources → policy → capability membrane → lifecycle → admission → mediated services) is an independently consumed contract with fail-closed rules; merging them trades compiler-proven stage separation for module discipline.
4. **Packages are directories first, crates when earned** — a package crate exists iff it has a channel adapter (binary-only linking is already enforced) or a heavy isolated dependency; everything else is manifest+assets+modules. Slack is already the model citizen; Telegram reaches parity by merging its two crates and depending only on contracts.
5. **Deletion list is verified, not vibes** — every dead item was checked for zero production consumers at the baseline commit (full inventory in PROPOSAL.md §2.6), and each deletion still lands under the "removing a redundant layer un-masks behavior" review discipline.

## Security model in three sentences

Untrusted input becomes **validated** at the listener/verifier (webui middleware; manifest-recipe signature verification minting sealed verified-inbound evidence), **authorized** only inside the kernel membrane (`CapabilityHost`: trust ceiling → default-deny grants → exact-invocation approvals → obligations → the sealed `Authorized` witness), and **safe to persist/deliver** only after mediated execution (narrowed mounts, one-shot staged secrets, policy-scoped egress) produces redacted evidence and sealed outbound grants drive at-most-once delivery. Family placement changes none of these mechanisms — it relocates code ownership around them; the three deliberate security-adjacent changes (evidence-mint consolidation, secrets direct-consumer tightening, host/verifier colocation) are individually risk-tracked in PROPOSAL.md §12.1. The full transition-by-transition walkthrough with the mermaid diagram is PROPOSAL.md §7.

## What is explicitly *not* decided here

Open PRs stay open: #6691 (composition builders) is directionally identical but not presumed; #6696 (process-journal collapse) gates four mapping rows (`processes` widening, `run_state` deletion, `approvals` widening, `runner`'s scheduler/await-edge shed) — the target is valid whether or not it lands. Ten genuinely open decisions (prompt-envelope/safety unification, trust's inert signed-registry path, the three-OAuth-stacks question, openai-compat-as-extension, the `reborn_` rename batch, and more) are listed in PROPOSAL.md §12.10 and in each affected crate spec, not silently resolved.

## How to review

- Skim this file, then read [PROPOSAL.md](PROPOSAL.md) §1–§5 for the argument and §8.3 for the exception-elimination proof.
- Dive into your family via [families/](families/) — each file is a **forward-looking specification** (the architecture as designed, no migration talk): the hard line around the family, what distinguishes it from each neighbor, and the full contract for every crate in it (purpose / owns / never contains / public surface / dependency direction / security role / why-a-crate). Migration and evidence stay in PROPOSAL/CHECKLIST/PLAN.
- Challenge the [CHECKLIST.md](CHECKLIST.md): it is the definition of done — if you think something is missing, it goes there.
- Argue sequencing in [PLAN.md](PLAN.md): waves, gates, and PR-sizing rules; nothing there is sacred except the ordering constraints called out as load-bearing.

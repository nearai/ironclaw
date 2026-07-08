# Reborn Crate Restructure — From Internal Modules to the Target Crate List

**Status:** PROPOSED — extends `docs/plans/2026-07-02-reborn-internal-module-refactor.md`
(ACTIVE). This doc does not replace that plan's eval harness, testing strategy, or
composition dissection sequence — it reuses all three. It **amends** three of its
verdicts (gate S3, §6.1's "new crates only for channel adapters", §9's freeze on
composition-domain crates) once Wave 3 below is ratified. Until a human approves §5
of this doc, the 2026-07-02 plan remains the operative instruction and only Waves
0–2 may execute.

**Audience:** implementer + verifier subagents executing one wave-step per PR, and
the humans reviewing them. Each wave section is self-contained: prerequisites,
per-PR steps, gates, hazards, done-criteria. Follow the loop protocol of the
2026-07-02 plan §4.4 (implementer → fresh-context verifier → PASS/FAIL, two FAILs
escalate to a human; SOC 2 — nothing auto-merges).

---

## 1. Why crate-level now, when the ACTIVE plan chose internal modules

The 2026-07-02 decision ("redistribute mass inside crates, don't add/remove
crates") was correct **as a sequencing decision**, and its dissection series has
largely executed: n1–n8 are merged (outbound, support::fs, observability,
product_auth, projection, llm_admin, extension_host, slack), n9 (automation,
PR #5818) and n10 (webui, PR #5843) are open, n11 (root collapse) remains.
Composition's `lib.rs` is down from ~120 flat modules to 48 on `main`, heading
to ≤ 12.

The dissection changes the economics of extraction. Each domain is now:

- one cohesive internal module with a named boundary (`slack/`, `product_auth/`,
  `extension_host/`, `llm_admin/`, …);
- public-surface-frozen (the `composition-pubuse.snapshot` gate has held through
  eight moves);
- already verified against the cfg-matrix (the #1 hazard, per the plan §3).

Turning such a module into a crate is now close to the mechanical `git mv` the
dissection PRs already perform — plus a `Cargo.toml` and a boundary rule. What the
crate boundary buys over the internal module, and why it is worth that step for
*some* modules:

1. **Compiler-enforced edges.** An internal module can silently reach any other
   module in composition; `slack/` as a crate physically cannot import
   `product_auth/` internals. Boundary tests police cross-crate edges mechanically;
   intra-crate discipline is convention.
2. **The composition charter becomes checkable.** "Wiring only" is enforceable as
   a mass budget + import deny-list on a small crate; it is not enforceable on a
   167k-line crate (measured on `main` 2026-07-08, tests included) that owns whole
   products.
3. **Layer legibility.** 65+ Reborn crates in one flat directory, five of them
   v1-only, with an inconsistent `reborn_` prefix, is a real onboarding and
   agent-navigation cost. Directory-per-layer plus an allowlist rule fixes it
   without renaming high-fan-in crates.

What this doc does **not** revisit: the fan-in evidence. Merging backbone
vocabulary crates (fan-in ≥ 24) remains net-negative, and the target list below
honors that — several merges from earlier drafts of this effort are explicitly
dropped (§2.3).

## 2. Target crate map

### 2.1 Layer directories (end state)

Crate *names* mostly stay (renaming a fan-in-270 crate is churn without
enforcement value); crate *paths* move under layer directories. The layer is also
declared in each crate's manifest (§4.1) so the allowlist gate does not depend on
paths alone.

```
crates/
  contracts/    host_api, common, prompt_envelope, turn_contracts (NEW, JIT)
  kernel/       turns, capabilities, authorization, approvals, run_state,
                resources, runtime_policy, trust, host_runtime, dispatcher,
                runner (= today's ironclaw_reborn, renamed)
  substrates/   filesystem, events, reborn_event_store, event_projections,
                event_streams, threads, conversations, memory, memory_native,
                secrets, network, safety, auth (⊕oauth), hooks (+backends,
                +parity), attachments, extractors, observability, outbound,
                triggers, projects*, reborn_identity, reborn_config,
                reborn_traces, llm, embeddings*, product_context
  runtimes/     wasm (⊕wasm_sandbox_core), wasm_limiter, mcp, scripts,
                processes, process_sandbox, wasm_product_adapters
  loops/        agent_loop, loop_support, skills (⊕skill_learning),
                first_party_extensions, first_party_extension_ports, extensions
  products/     product_workflow (⊕storage), product_adapters,
                product_adapter_registry, reborn_openai_compat (⊕storage),
                webui_v2, webui_v2_static, reborn_webui_ingress (⊕composition
                webui module), slack_v2_adapter, telegram_v2_adapter,
                product_slack (NEW), product_auth (NEW), extension_host (NEW),
                llm_admin (NEW)
  app/          reborn_composition (shrunk to assembly), reborn_cli,
                reborn_migration, reborn_test_harness (NEW),
                architecture (tests-only), hooks_parity (tests-only)
  legacy/       engine, tui, gateway (v1-only; oauth and embeddings are
                consumed by reborn-side crates — verify reverse-deps before
                classing them legacy; oauth folds into auth in Wave 2)
```

`*` = placement to verify at execution time (see the wave that touches it).
`⊕` = absorbs a fold from Wave 2. `NEW` = created by Wave 3/5.

### 2.2 Net crate-count delta

| Change | Count |
| --- | ---: |
| Folds (Wave 2, all endorsed by the ACTIVE plan §2.4) | −6 |
| New product crates extracted from composition (Wave 3) | +4 |
| New app-layer crate (`reborn_test_harness`, Wave 3) | +1 |
| `turn_contracts` split (Wave 4, JIT — only if ripple hurts) | +1 |
| **Net** | **≈ 0 to +1** |

This is deliberately *not* a crate-count reduction program. The 2026-07-02
fan-in analysis showed count is not the pain; the pain is mass concentration
(composition = 15% of the workspace) and illegible layering. The restructure
targets those two.

### 2.3 Merges considered and rejected (do not resurrect without new evidence)

| Rejected merge | Why |
| --- | --- |
| `host_api + common + prompt_envelope → one contracts crate` | Pure churn: ~350 combined reverse-dep references repoint, zero enforcement gained. Directory placement delivers the orientation win. |
| `events + reborn_event_store (+projections +streams)` | `events` is 2.5k-line vocabulary with fan-in 70; welding storage backends and read models onto it makes every edit to a backend ripple through 70 consumers. The 4-way split is load-bearing. |
| `memory + memory_native` | On audit watch (architecture-review skill names it unjustified), but `memory` is fan-in-24 vocabulary and `memory_native` is 8k of behavior — same ripple argument. Revisit only when a second memory provider decision is made either way (add one, or commit to never adding one). |
| A single `ironclaw_kernel` mega-crate **today** | **Deferred, not rejected** — see Wave 7. Merging `turns` (fan-in 110) with capability/scheduler behavior *before* the W4.3 type split puts kernel churn into 110 consumers' rebuild path. After W4.3 moves the fan-in to `turn_contracts`, the behavior remainder has low fan-in and the consolidation becomes cheap; the single-crate kernel's auditability win (one reviewable security perimeter) then stands unopposed. Until then the kernel stays a *directory* of crates with allowlisted edges. |
| `webui_v2 + ingress + static → one product crate` | Ingress/CLAUDE.md contracts and the static-asset crate's build profile are deliberate boundaries; directory grouping suffices. |

### 2.4 The agent_loop / loop_support question — decision required

The ACTIVE plan (§2.3) verdicts `agent_loop → MERGE into loop_support` as "the one
clean substrate merge." This doc recommends the **opposite**: keep two crates,
because the split is what makes "loops are userland" compiler-enforced —
`agent_loop` depends only on neutral contracts (verifiable in its Cargo.toml),
while `loop_support` depends on threads/LLM/host services. Merging collapses that
to a module convention inside one crate.

Options for the ratifier (§5):

- **A (recommended):** keep both; optionally rename `loop_support` →
  `loop_host` in Wave 5 to say what it is (the host side of the loop ports).
- **B (ACTIVE plan):** merge as `reborn_loop`, and add a boundary test that the
  merged crate's `agent_loop`-derived modules stay import-clean — accepting that
  this is grep-level, not compiler-level, enforcement.

Whichever is chosen, record it by editing this section and §2.3 of the 2026-07-02
doc in the same PR.

## 3. Prerequisites and in-flight collision management

Do not start a wave whose inputs are still moving. As of 2026-07-08:

| In-flight work | Collides with | Rule |
| --- | --- | --- |
| Dissection n9 (#5818, automation), n10 (#5843, webui), n11 (root collapse, not yet opened) | Everything in Wave 3 | Waves 0–2 may run in parallel with the dissection. Wave 3 starts only after n11 merges. |
| NEA-25 stack (#5833→#5850: manifest v2 cutover, surface discovery, slack unification, frontend surfaces, zero-legacy gate) | `extensions`, `slack` domains | Wave 3's `extension_host` and `product_slack` extractions start only after the NEA-25 stack merges (it rewrites the very seams being extracted). Other Wave-3 extractions (`llm_admin`, `product_auth`) don't overlap it. |
| `codex/streaming-chat-webui` (#5821), design system (#5563), webui fixes | `webui_v2` | Wave 3's webui-module fold into ingress waits for these; directory moves in Wave 6 rebase over them trivially (`git mv` conflicts resolve as moves). |
| `release-plz` (#5598) | root `Cargo.toml` | Wave 6's members-list rewrite will conflict; coordinate the merge window. |

General rule for every wave-PR: **rebase on fresh `main` immediately before
starting**, and re-run the wave's gates after any rebase. The dissection series
proved intra-crate steps rebase cheaply; extraction steps (Wave 3) are the ones
that must not race a domain rewrite.

## 4. The waves

Summary (each wave = independently shippable, repo green after every PR):

| Wave | What | New/changed crates | Risk | Parallel with |
| --- | --- | --- | --- | --- |
| W0 | Allowlist layer gate | none (metadata + tests) | low | everything |
| W1 | Finish dissection n9–n11 | none | low | W0, W2 |
| W2 | Endorsed folds ×6 | −6 | low | W0, W1 |
| W3 | Product eviction from composition | +5 | **the real work** | — (serialized per domain) |
| W4 | Kernel co-location (+ JIT turn_contracts) | ±1, 1 rename | medium | W3 domains it doesn't touch |
| W5 | Loop boundary decision | 0 or −1 | low | W3/W4 |
| W6 | Directory re-layout + legacy move + docs/tooling sweep | paths only | low, high-conflict | nothing (atomic, quiet window) |
| W7 | Kernel consolidation (ratification-gated, after W4.3) | −3 to −5 | medium | — |

### Wave 0 — allowlist layer gate (1 PR, land first)

Today `ironclaw_architecture`'s dependency rules are blocklists (61 test fns,
~3.4k lines in `reborn_dependency_boundaries.rs`): a **new crate is unruled by
default**. Flip the default:

1. Add `[package.metadata.ironclaw] layer = "<contracts|kernel|substrates|runtimes|loops|products|app|legacy>"`
   to every workspace member's `Cargo.toml` (one mechanical commit; the §2.1 map
   is the source of truth — note the metadata is set now, while paths move only
   in Wave 6).
2. Add one generic test to `ironclaw_architecture` driven by `cargo metadata`:
   - every workspace member has a `layer` key (a member without one **fails the
     build** — this is the point);
   - normal dependencies respect the edge matrix:
     `app → products → loops → kernel → substrates/runtimes → contracts`
     (a layer may also depend on its own layer and anything lower;
     `loops/agent_loop` additionally restricted to `contracts` only — this
     encodes the userland rule; `legacy` may not be depended on by anything
     except the v1 root crate);
   - dev-dependencies are exempt from the matrix but not from the `legacy` rule.
3. Encode today's *known violations* (there will be some — e.g.
   `reborn_webui_ingress` depends on composition, product crates reach into
   kernel types) as an explicit, dated exception list in the test file, each
   entry naming the wave that removes it. The gate's job in W0 is to stop **new**
   violations, not to force a big-bang fix.
4. Do **not** delete the existing blocklist tests; they encode finer-grained
   rules (specific type leaks, v1-enclave bans) the matrix doesn't express.

Amends gate S3 of the 2026-07-02 plan: "no new crates" becomes "no new crate
without a layer entry and a matrix-legal position; expected member-count per wave
is stated in that wave's PR."

Done when: `cargo test -p ironclaw_architecture` fails on (a) a member without a
layer, (b) a new matrix-illegal edge, and the exception list is exactly the
pre-existing violations.

### Wave 1 — finish the dissection (owned by the existing series)

Not this doc's work; listed as a dependency. n9 (#5818) and n10 (#5843) are open
drafts; n11 (root collapse: `runtime.rs`/`factory.rs` dissolve, lib.rs ≤ 12 flat
mods, per plan §3 row 11) has not been opened. Wave 3 consumes n11's output —
the extraction cost model assumes `runtime.rs`/`factory.rs` no longer hold leaked
domain logic.

### Wave 2 — plan-endorsed folds (6 PRs, parallelizable, any order)

All six are already verdicted FOLD in the ACTIVE plan §2.4. Per fold: move the
source as modules into the absorber, move tests, delete the crate dir, update the
members list and all `Cargo.toml` references, keep the absorbed public items
exported from the absorber under a module named like the old crate (no root
glob re-exports — house rule).

| PR | Fold | Verified notes |
| --- | --- | --- |
| W2.1 | `ironclaw_oauth` → `ironclaw_auth` | oauth is 439 lines, v1-listed but consumed via auth — confirm reverse-deps at execution; if a v1-only consumer exists, the fold still works (v1 imports `ironclaw_auth::oauth`). |
| W2.2 | `ironclaw_skill_learning` → `ironclaw_skills` | Preserve the `SkillInferencePort` DIP shape: the port trait must stay in a module with no LLM/runtime deps — the fold must not let `skills`' deps leak into it. Prompt files move with the crate (`prompts/*.md` + `include_str!`). |
| W2.3 | `ironclaw_wasm_sandbox_core` → `ironclaw_wasm` | 357 lines. Check `wasm_product_adapters` imports repoint. |
| W2.4 | `ironclaw_reborn_openai_compat_storage` → `ironclaw_reborn_openai_compat` | Storage adapters become a `storage` module; keep the filesystem-backed idempotency tests intact. |
| W2.5 | `ironclaw_product_workflow_storage` → `ironclaw_product_workflow` | Dual-backend (libSQL/Postgres) adapters; run the parity/contract tests with both features. |
| W2.6 | `ironclaw_projects` → consumer | Only consumer is composition — folding *into composition* contradicts Wave 3's direction. Decision at execution: fold into `ironclaw_product_workflow` (if it's product-domain state) or leave standalone in `substrates/`. Do not fold into composition. |

Gates per PR: G1–G6 from the 2026-07-02 plan §4.1 (swap the `-p` targets for the
absorber + its consumers), S4-style public-surface check on the absorber, S5
move-purity, plus W0's layer gate. Expected member count: stated in each PR
(−1 each).

### Wave 3 — product eviction from composition (the centerpiece)

**Precondition:** n11 merged; NEA-25 merged for the two domains it touches; §5
ratified (this wave is where the 2026-07-02 §9 freeze is formally amended).

Target: composition ends as *assembly only* — `RebornServices`,
`build_reborn_services`, `RebornRuntime`, `build_reborn_runtime`, per-domain
`build_*`/`with_*` wiring, readiness, input/error DTOs, runtime-profile policy
(the §3-row-11 root set) — with a mass budget enforced by a new gate (§4.2 S6).
Consumers of composition today are exactly: `reborn_cli`, `reborn_webui_ingress`,
`reborn_migration` (deps), `product_workflow` (dev-dep), and the v1 root —
verified against `Cargo.toml`s on `main` 2026-07-08. That short list is what
makes extraction tractable: repoint them directly, **no `pub use` shim layers**
(house rule, plan §9).

Extraction order — smallest/cleanest first to re-prove the pattern, then by
descending readiness:

| PR-stack | Module → crate | Size | Notes and hazards |
| --- | --- | ---: | --- |
| W3.1 | `test_support` → `ironclaw_reborn_test_harness` (`app/`) | ~1.4k | Breaks the `product_workflow` dev-dep back-edge and shrinks the `test-support` feature surface. Harness keeps a *dep* on composition (it builds runtimes for tests) — that edge is legal app-layer. |
| W3.2 | `llm_admin` → `ironclaw_llm_admin` (`products/`) | ~5k | Carries the messiest cfg gates in the crate (`root-llm-provider`, `openai-compat-beta`, compound gate on `nearai_login_serve` — n6 PR #5709 documents the exact per-module gates). The features move to the new crate; composition re-exposes them as forwarding features so `reborn_cli` builds don't change. T2/T3 characterization tests (plan §5) must exist and stay green unmodified. |
| W3.3 | `product_auth` → `ironclaw_product_auth` (`products/`) | ~23k | The OAuth-provider-as-one-file seam (plan §6.5) moves with it — new providers then land in the product crate, not composition. `profile_approval_authorization` stays in composition root (profile policy, per n4's mapping). |
| W3.4 | `extension_host` → `ironclaw_extension_host` (`products/`) | ~11k | **After NEA-25.** Frozen constants (plan §3): the two OUT_DIR JSON names, the `.ironclaw-reborn-bundled.json` marker, owner string `ironclaw_reborn_composition_bundled_skill` — byte-identical even though the owning crate changes (it is a persisted marker; renaming it breaks install idempotency). The `include_str!` relative paths to `ironclaw_first_party_extensions/assets` get *shorter* again — re-verify all 11 (n7 PR #5783 documents the hazard). `build.rs` moves with `bundled_skills`. T4/T5 green unmodified. |
| W3.5 | `slack` → `ironclaw_product_slack` (`products/`) | ~35k | **After NEA-25 slack unification (#5845).** The `slack-v2-host-beta` feature moves; `PostSubmitDeliveryHook` OnceLock stays in composition root with the impl registered from the new crate (seam already shaped for this, plan §6.5). `#[path = "e2e_auth_challenge.rs"]` include preserved. T6/T7 green unmodified. `slack_v2_adapter` (protocol-pure) is untouched — the pair then matches the webui_ingress model exactly. |
| W3.6 | `webui` module → fold into `ironclaw_reborn_webui_ingress` | ~5k | Not a new crate — the host-side webui crate already exists; the middleware/serve glue joins it. Removes the ingress→composition exception from W0's list if the remaining edge allows (ingress still needs runtime handles; that edge is legal products→app? No — **invert it**: the runtime-handle types ingress needs move down or are passed in by composition at build time. Check `RebornWebuiBundle` consumers; this is the one extraction with a real design decision — budget time for it). T8 green unmodified. |
| W3.7+ (JIT) | `projection`, `automation`, `observability` module, `outbound` module, `support::fs` | ~27k | Default: **stay in composition** as internal modules — they are closer to wiring/read-model assembly than to products. Extract later only if a roadmap feature makes one of them a real product surface. Record the JIT trigger in this table when it fires. |

Per-extraction recipe (every W3 PR):

1. Rebase on fresh `main`; confirm the module's dissection PR merged unchanged.
2. `git mv crates/ironclaw_reborn_composition/src/<mod>` →
   `crates/<layer>/ironclaw_<name>/src/` (until Wave 6 lands, create the crate at
   `crates/ironclaw_<name>/` — flat — so W3 doesn't depend on W6).
3. Write the new `Cargo.toml`: copy only the deps the module actually uses
   (`cargo +nightly udeps` or compile-error-driven pruning); add the W0 layer key;
   add the crate to the members list.
4. Re-hang cfg gates: crate-level features replace the composition features for
   that domain; composition forwards (`feature = ["ironclaw_<name>/<feature>"]`)
   so downstream build commands stay valid.
5. Composition keeps only the domain's `build_*`/`with_*` functions, now calling
   into the new crate.
6. Repoint the ≤ 5 composition consumers where they used the domain's re-exports.
7. Add the new crate's boundary rule (blocklist tests) **in the same PR** — the
   skill checklist rule; W0's matrix covers the layer edge automatically.
8. Gates: G1–G6 (add `-p ironclaw_<name>` to G3/G5), S4 (composition snapshot
   shrinks — update with justification lines), S5, S6 ratchet (§4.2), layer gate.
9. Judge rubric: plan §4.3 unchanged, plus "no behavior change smuggled into the
   wiring left behind."

Done when: composition's non-test line count ≤ 25k (root + wiring + the W3.7
holdouts), every extracted domain's tests run under its own `-p`, and the W0
exception list has shrunk by the entries these waves named.

### Wave 4 — kernel co-location (2 small PRs + 1 JIT)

Fixes the "scheduler and executor live in different crates" scatter without a
mega-merge:

- **W4.1** Move `TurnRunScheduler` from `ironclaw_host_runtime`
  (`turn_scheduler.rs`) into `ironclaw_reborn`, next to `RebornTurnRunExecutor`
  — the claim/heartbeat/invoke/apply control plane becomes one crate. Direction
  chosen to *lower* fan-in exposure (host_runtime fan-in 24 → sheds a consumer;
  ironclaw_reborn fan-in 14). Update `crates/Architecture.md` §"Runner And Lease
  Flow" in the same PR.
- **W4.2** Rename `ironclaw_reborn` → `ironclaw_runner` (its name says nothing;
  after W4.1 it is precisely the runner). Mechanical: ~17 reverse-dep manifests
  (count on `main` 2026-07-08), no persisted strings carry the crate name —
  but grep for the literal (`rg 'ironclaw_reborn"' -g '*.rs'`) before asserting
  that at execution time; driver ids and checkpoint schema ids like
  `ironclaw_agent_loop.default_family.v1` are frozen literals and must not be
  "tidied" (they are persisted; see families/mod.rs const).
- **W4.3 (JIT, only if measured ripple hurts)** `turn_contracts` split: `turns`'
  type surface (ids, scopes, DTOs, state-machine contracts, `LoopExit`) into
  `contracts/ironclaw_turn_contracts`, behavior stays. The ACTIVE plan already
  verdicts this SPLIT-TYPES/JIT with fan-in 110. Trigger: a kernel-behavior PR
  visibly recompiling >50 downstream crates, or W6 wanting a clean contracts/
  directory. Do not do it speculatively.

### Wave 5 — loop boundary (1 PR, after §5 ratification)

Execute whichever of §2.4's options was ratified:

- Option A: rename `ironclaw_loop_support` → `ironclaw_loop_host`; add a
  boundary test asserting `agent_loop`'s dependency set ⊆ contracts layer
  (making the userland rule an explicit test, not just an observed fact).
- Option B: merge per the ACTIVE plan §2.3 (`reborn_loop`), with the grep-level
  import guard described there.

### Wave 6 — directory re-layout, legacy move, sweep (1 atomic PR, quiet window)

Everything before this wave used flat `crates/` paths, so W6 is pure `git mv` +
path bookkeeping. It is the highest-conflict, lowest-risk step; schedule it in a
lull (check open PRs touching `crates/` first; coordinate with release-plz).

1. `git mv` every crate to its §2.1 directory; rewrite the members list.
2. Move `engine`, `tui`, `gateway` to `crates/legacy/` (they stay workspace
   members until Tier B deletes them; the W0 gate already bans new deps on them).
   `embeddings` and the post-W2 `auth` stay out of `legacy/` — both have
   reborn-side consumers (verified for embeddings via the worked-examples table;
   re-verify at execution).
3. Sweep every path reference: CI workflows, `scripts/*.sh` (pre-commit safety,
   check-boundaries, codebase-graph), `.claude/skills/**` and `.claude/rules/**`,
   `CLAUDE.md`s, `crates/README.md` + `crates/Architecture.md` + `crates/AGENTS.md`,
   `docs/reborn/contracts/`, dependabot config (it names
   `crates/ironclaw_webui_v2/frontend`), release-plz config if it globs paths.
   ~128 non-crate files reference crate paths (measured 2026-07-08); grep, don't
   enumerate from memory.
4. `include_str!` relative paths that cross crate dirs (the
   first-party-extensions asset includes, W3.4) get re-verified by the
   full-feature build.
5. Reindex the knowledge graph (`scripts/codebase-graph.sh`) and regenerate
   openwiki via its workflow after merge.

Gates: full G1–G6 across the workspace, layer gate, and a zero-diff check that
`git diff --find-renames=100% --stat` shows only renames plus the bookkeeping
files.

### Wave 7 — kernel consolidation (ratification-gated; only after W4.3 has landed)

The end-state the original restructure proposal argued for and §2.3 defers: one
auditable kernel crate instead of authority spread across `turns`, `capabilities`,
`host_runtime`, and `runner`. The fan-in objection that blocks it today is
*removed by W4.3*: once `turn_contracts` carries the type surface (and with it
the fan-in ≈ 110), the behavior remainders are low-fan-in and merging them stops
rippling.

Preconditions (all hard):

1. W4.3 has actually executed (it is JIT — if its trigger never fired, fire it
   as W7.0 first) and the post-split fan-in of each candidate is measured and
   recorded here: `turns`-behavior, `capabilities`, `host_runtime`,
   `runner`. Proceed only if each is below ~25.
2. §5 item 6 ratified.

Shape:

- **W7.1** `ironclaw_kernel` = `turns`-behavior (coordinator, state machine,
  stores, `LoopExitApplier`) + `runner` (scheduler + executor + driver registry)
  — the turn-lifecycle control plane in one crate.
- **W7.2** fold `capabilities` (CapabilityHost) and the capability-hosting parts
  of `host_runtime` into `ironclaw_kernel`; `dispatcher` folds in as a
  `pub(crate)`-heavy module (it is already "composition-only contracts" —
  below-authorization routing belongs inside the perimeter it serves).
- Decision crates stay out: `authorization`, `approvals`, `trust`,
  `runtime_policy`, `resources` remain separate pure-decision crates — the
  kernel *calls* policy; merging policy engines in would recreate the
  everything-crate this plan exists to prevent.
- Visibility kit mandatory: `#![warn(unreachable_pub)]`, sealed traits on
  strategy slots, and a boundary test that the merged crate's public surface is
  ≤ the union of the pre-merge public surfaces (no accidental exposure widening).

What this buys: the security/recovery perimeter becomes one crate a reviewer or
auditor can hold; intra-kernel edges (e.g. "dispatcher is below authorization")
become module discipline, which is acceptable *inside* a mutually-trusting
perimeter — the edges that carry security weight (loops → kernel internals,
products → substrates) remain crate edges and W0-matrix rules.

If ratification declines W7, delete this section and move the §2.3 row back to
a plain rejection, so the doc never carries a permanently-pending wave.

## 5. Ratification checklist (a human answers these before W3)

1. Approve amending the 2026-07-02 plan's §6.1/§9 freeze: new crates for
   composition domains are now allowed **when extracted along dissection
   boundaries** (the freeze stays for speculative crates). Record by adding a
   pointer line to that doc's Status field.
2. Choose §2.4 option A or B (loop boundary).
3. Confirm the W3.7 holdouts (projection/automation/observability/outbound/
   support::fs stay internal) — or name which of them should extract and why.
4. Confirm `ironclaw_projects`' fold target (W2.6).
5. Confirm the composition mass budget number (S6 end-state; default 25k
   non-test lines) and the `ironclaw_runner` rename (W4.2 — it is the only
   rename with real churn).
6. Approve or decline Wave 7 (kernel consolidation). Declining is a valid
   end-state — record it by deleting the W7 section per its last paragraph.
   This item may be deferred until W4.3's fan-in measurements exist; it does
   not block Waves 0–6.

## 6. Amended and added gates

All 2026-07-02 gates stay in force. Deltas:

- **S3 (amended, W0):** "workspace member count unchanged" → "member count
  matches the number stated in the PR description; every member carries a layer
  key; no matrix-illegal edges beyond the dated exception list."
- **S6 (new, W3):** composition mass ratchet —
  `find crates/ironclaw_reborn_composition/src -name '*.rs' ! -path '*test*' | xargs wc -l`
  must be ≤ the ceiling stated in the wave table; each W3 PR lowers the recorded
  ceiling in this doc. End state ≤ 25k (pending §5.5).
- **S7 (new, W3):** extracted-crate dependency hygiene — the new crate's
  `Cargo.toml` contains no dep the compiler doesn't force (reviewer spot-check;
  copy-pasting composition's 96-dep manifest is the failure mode).
- **Frozen-literal gate (W3.4/W4.2):** the bundled-skills marker/owner/JSON
  names and the loop-family/checkpoint schema id prefixes are byte-frozen; add a
  const-pin test wherever one moves (T5 already covers bundled-skills).

## 7. One go, or in parts? — In parts. Non-negotiably.

The evidence is the repo's own history:

- The **abandoned 6-crate extraction** (#5135/#5137) was the one-go attempt at
  this exact problem; it died and was formally reversed by the 2026-07-02 plan.
- The **dissection series** (n1–n8 merged without a single behavior regression,
  public surface frozen throughout) is the in-parts pattern working, with gates
  that already exist and PR bodies that already read as an executable playbook.
- Two large in-flight stacks (NEA-25, streaming webui) touch the same domains;
  a big-bang branch would rot against them within days — rebasing a 65-crate
  restructure over a 7-PR extensions rewrite is not a realistic operation.
- SOC 2 review: every PR needs a human; a single 500-file PR is unreviewable,
  and unreviewable here means unmergeable.

The one deliberate exception: **Wave 6 is atomic** — a directory re-layout done
piecemeal leaves the tree in a mixed state that breaks path-referencing tooling
twice per crate instead of once; and because it is 100%-rename by construction,
its bigness is reviewable (the review is the gate output plus the sweep list,
not a line-by-line read).

Expected shape: W0 and W2 land this week alongside the dissection tail; W3 runs
as 6 serialized PR-stacks over several weeks, each gated; W4/W5 slot between W3
stacks; W6 is one scheduled afternoon.

## 8. PR conventions for implementing agents

- Branch names: `reborn/crate-restructure-w<wave>.<step>-<slug>` (mirrors the
  `reborn/composition-dissection-*` convention).
- PR body mirrors the dissection template: What (wave/step + this doc §), Files
  moved, cfg-gate table, public-API impact, Verification (each gate with its
  result), Risk. n8 (#5785) is the exemplar.
- One wave-step per PR. No "while I was here" changes (judge rubric enforces).
- Every PR updates *this doc*: tick the step in §4's tables (add a ✅ + PR
  number), adjust the S6 ceiling. The doc is the coordination ledger — the next
  agent must be able to determine remaining work from this file alone.
- On two consecutive verifier FAILs or any hazard not listed here: stop, comment
  findings on the PR, escalate to a human. Then add the hazard to this doc.

## 9. What we are explicitly NOT doing

- No merging of backbone vocabulary crates (fan-in ≥ 24) — unchanged from the
  ACTIVE plan; §2.3 lists the specific rejections with reasons. (Wave 7, if
  ratified, merges low-fan-in *behavior* remainders after the W4.3 type split;
  it does not touch vocabulary crates and its preconditions enforce that.)
- No renames beyond `ironclaw_reborn → ironclaw_runner` and (option A)
  `loop_support → loop_host`. The `reborn_` prefix inconsistency resolves itself
  when Tier B retires v1, not by renaming 60 crates.
- No `pub use` shim layers for extracted crates — consumers repoint directly
  (there are ≤ 5 of them; measured, not assumed).
- No touching v1 internals (`src/`, `legacy/` crates) — Tier B is a separate
  epic (ACTIVE plan §8).
- No speculative extraction of the W3.7 holdouts, and no new seams without the
  roadmap feature that needs them (ACTIVE plan §6.5 rule stands).

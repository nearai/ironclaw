# Reborn Internal-Module Refactor — Crate Map, Eval Harness, Testing Strategy

**Status:** ACTIVE — supersedes `docs/plans/2026-06-21-composition-crate-decomposition.md`
(the 6-crate extraction; PRs #5135/#5137 closed, direction reversed on fan-in evidence).

## 1. Decision

**We are NOT reducing abstraction by merging or extracting crates. We are redistributing
mass inside crates.** The evidence, in order:

1. **Roadmap shape (Jun–Aug 2026):** ~70% of planned features are cross-cutting
   (#1187-shaped): self-learning loops, long-term memory, multi-tenant collaboration,
   permission management, secrets×skills, admin-configurable skills/tools. These cross the
   *backbone* crates, which fan-in analysis proves un-mergeable. Only a minority are
   localized (#5502-shaped: one OAuth provider file + composition wiring) where crate
   granularity is already cheap.
2. **Fan-in × size analysis (all 76 crates):** ~25 backbone crates (fan-in ≥ 24) are
   load-bearing vocabulary/hubs — merging any ripples 24–270 recompiles. The ~40-crate
   tail is mostly deliberate DIP/ports/adapters/dual-backend boundaries. Crate *count* is
   not the pain; **mass concentration** is: `reborn_composition` alone is 15% of the
   workspace, top-6 crates ≈ 31%.
3. **Feature traces:** #5502 (Slack personal OAuth) = 4 crates, mostly composition —
   cheap. #1187 (learning system) = ~15–18 reborn crates vs 2 in v1 — expensive because
   of missing per-domain seams, not crate boundaries per se.
4. **Six-agent dissection of `reborn_composition` (153k lines, ~120 flat top-level
   modules):** the crate regroups into 10 cohesive internal modules; `runtime.rs` (10.2k)
   + `factory.rs` (6.8k) are ~80% leaked domain logic; the irreducible root is ~3.3k.

**Therefore:** decompose god crates into *internal modules* (zero cross-crate churn,
rebase-tolerant, behavior-preserving), split god *interfaces* by domain just-in-time,
add per-domain extension seams as roadmap features land, and do tail hygiene only where
genuinely dead/foldable.

## 2. Full crate map (76 crates)

Verdicts: **KEEP** (as-is) · **INTERNAL** (decompose/repair inside the crate) ·
**SPLIT-TYPES** (extract type surface only, keep behavior) · **FOLD→X** (merge into X) ·
**MERGE** (sibling merge) · **JIT** (act only when a feature demands it).

### 2.1 Backbone — KEEP, never merge (fan-in ≥ 24)

| Crate | Fan-in | Lines | Verdict |
|---|---:|---:|---|
| ironclaw_host_api | 270 | 6.3k | KEEP — universal vocabulary |
| ironclaw_filesystem | 155 | 9.8k | KEEP |
| ironclaw_turns | 110 | 23.9k | **SPLIT-TYPES (JIT):** carve `turns` type surface into a tiny stable crate if/when rebuild ripple hurts; behavior keeps low fan-in. Do not merge anything into it. |
| ironclaw_events | 70 | 2.5k | KEEP |
| ironclaw_common | 64 | 4.5k | KEEP |
| ironclaw_safety | 64 | 6.7k | KEEP |
| ironclaw_resources | 55 | 4.7k | KEEP |
| ironclaw_extensions | 54 | 4.7k | KEEP |
| ironclaw_product_adapters | 54 | 6.5k | KEEP |
| ironclaw_trust | 44 | 3.7k | KEEP |
| ironclaw_skills | 44 | 10.0k | KEEP |
| ironclaw_threads | 35 | 7.6k | KEEP |
| ironclaw_event_projections | 35 | 3.8k | KEEP |
| ironclaw_authorization | 34 | 1.8k | KEEP |
| ironclaw_run_state | 29 | 1.3k | KEEP |
| ironclaw_processes | 29 | 2.8k | KEEP |
| ironclaw_llm | 25 | 42.9k | KEEP boundary; **INTERNAL health OK** (36 top mods, biggest file 4.4k) — no action |
| ironclaw_hooks | 25 | 22.6k | KEEP; internal OK (21 mods, biggest 5.0k) |
| ironclaw_reborn_event_store | 25 | 3.0k | KEEP |
| ironclaw_host_runtime | 24 | 35.2k | KEEP boundary; internal OK (21 mods, biggest 2.9k) |
| ironclaw_loop_support | 24 | 29.6k | KEEP; **INTERNAL:** `capability_port.rs` (8.2k) is a god-file — split by capability family when next touched (JIT) |
| ironclaw_product_workflow | 24 | 23.8k | KEEP; **INTERNAL/JIT:** `reborn_services.rs` (6.0k) holds the 70-method `RebornServicesApi` god interface — split by domain (threads/turns/gates/extensions/llm/…) as features demand slices; storage fold-in from tail (below) |
| ironclaw_reborn_traces | 24 | 17.7k | KEEP; **INTERNAL:** `contribution.rs` = **14.5k lines in one file** — worst god-file in the workspace after composition; dissect into submodules |
| ironclaw_approvals | 24 | 3.2k | KEEP |
| ironclaw_memory | 24 | 1.7k | KEEP |

### 2.2 Mid fan-in (10–19) — KEEP all

triggers (19), secrets (19), auth (19), extractors (19), prompt_envelope (15),
dispatcher (15), attachments (15), outbound (15), product_context (15), reborn_config (14),
network (14), runtime_policy (14), first_party_extensions (14), ironclaw_reborn (14 —
internal OK: 21 mods, biggest 4.7k), mcp (13), memory_native (10), conversations (10),
capabilities (10), reborn_openai_compat (10), process_sandbox (10), scripts (10),
product_adapter_registry (10), wasm_limiter (10), wasm (9).

**ironclaw_reborn_composition (19, 153k) → INTERNAL — the centerpiece. See §3.**

### 2.3 Low fan-in tail (4–5) — mostly deliberate boundaries, KEEP

engine (38.9k — v1-adjacent; biggest file `runtime/mission.rs` 8.5k, fix JIT when touched),
tui, gateway, embeddings, webui_v2, webui_v2_static, reborn_webui_ingress,
slack_v2_adapter, telegram_v2_adapter, wasm_product_adapters,
first_party_extension_ports, event_streams, reborn_identity, hooks_postgres,
hooks_libsql (dual-backend rule — keep both).

**ironclaw_agent_loop (fan-in 5, 25.5k) → MERGE into `ironclaw_loop_support`** — the one
clean substrate merge: siblings (neither depends on the other), tiny blast radius.
Optional; schedule when loop work next opens both crates anyway.

### 2.4 Foldable tail — FOLD (~6 crates, low value, do opportunistically)

| Crate | Lines | Fold into |
|---|---:|---|
| ironclaw_oauth | 439 | its consumer (ironclaw_auth) |
| ironclaw_skill_learning | 356 | ironclaw_skills (or consumer) |
| ironclaw_wasm_sandbox_core | 357 | ironclaw_wasm |
| ironclaw_reborn_openai_compat_storage | 673 | ironclaw_reborn_openai_compat |
| ironclaw_projects | 842 | **KEEP in W2**; substrate entity/repository. If revisited later, the only acceptable consumer-side target is `ironclaw_product_workflow`, never composition. |
| ironclaw_product_workflow_storage | 946 | ironclaw_product_workflow |

### 2.5 Zero fan-in — KEEP all (correction: none are dead)

| Crate | What it actually is |
|---|---|
| ironclaw_architecture | **tests-only crate** — boundary tests (`reborn_dependency_boundaries.rs`, `reborn_composition_boundaries.rs`, 3.8k test lines). Load-bearing for this refactor's eval. |
| ironclaw_hooks_parity | **tests-only crate** — postgres/libsql parity oracle + adversarial matrix (2.1k test lines) |
| ironclaw_silk_decoder | standalone binary |
| ironclaw_reborn_cli | binary (12.4k + 3.7k tests) |

## 3. Composition dissection — 10 internal modules, 11 PRs

Target: `lib.rs` goes from ~120 flat `mod` decls to **≤ 12** (root + the 10 domain
modules of §3/§6.2, plus the `test_support` top-level mod); `runtime.rs`/`factory.rs`
dissolve; **one crate throughout — every existing `pub use` re-points, external
consumers compile unchanged.**

| # (PR order) | Module | Lines | Notes |
|---|---|---:|---|
| 1 | `outbound` | 2.6k | pure leaf, zero internal refs — proves the pattern |
| 2 | `support::fs` | 1.9k | filesystem readers, attachment_landing, project_service |
| 3 | `observability` | 9.5k | hooks/trace/budget/operator; + pull budget-evidence out of runtime.rs |
| 4 | `product_auth` | 23.4k | `{api,oauth,durable,serve,credentials}`; relocate `profile_approval_authorization`→root, `extension_activation_credentials`→extension_host first |
| 5 | `projection` | 9.4k | after #4 (renders auth previews); beware `hooks/projection.rs` name collision |
| 6 | `llm_admin` | 5.0k | + pull LLM-gateway & nearai-bootstrap blocks out of runtime.rs/factory.rs; `nearai_mcp` lands here |
| 7 | `extension_host` | 10.7k | + pull skill-selector wiring out; `lifecycle.rs`, `gsuite`, `mcp`/`mcp_discovery` land here; **freeze `bundled_skills` marker + OUT_DIR JSON names** |
| 8 | `slack` | 34.6k | `{host,serve,delivery,binding,routes,outbound,setup}`; relocate stranded helpers from lib.rs/factory.rs; repath `PostSubmitDeliveryHook` OnceLock in root |
| 9 | `automation` | 5.2k | `{facade,trigger_poller}`; + pull trigger-poller wiring out of factory.rs |
| 10 | `webui` | 4.8k | `middleware` submodule (= old http_kit surface, stays internal); + pull webui-auth-interaction out |
| 11 | `root` | ~3.3k | what remains: `RebornServices` + `build_reborn_services` + `RebornRuntime` + `build_reborn_runtime` + input/readiness/error DTOs + runtime-profile policy (`local_dev_authorization`, `runtime_profile_approval_policy`, `local_dev_capability_policy` STAY here — they are profile policy, not product auth) + folded glue (`product_live_adapters`, `communication_context`, `default_system_prompt`, `profile`) |

Known hazards (the complete risk surface):

- **cfg gates:** `slack-v2-host-beta`, `openai-compat-beta`, `root-llm-provider`,
  compound gate on `nearai_login_serve`. Re-hang per inner `mod`; a misplaced parent
  gate silently drops a domain from non-beta builds.
- **`bundled_skills`:** the following are behavior-load-bearing — move the file, but
  **never rename any of them** (verified against `build.rs` + `src/bundled_skills.rs`):
  - the two `include_str!(concat!(env!("OUT_DIR"), …))` inputs written by `build.rs`:
    `embedded_reborn_skill_summaries.json` and `embedded_reborn_skill_bundles.json`
  - the install marker file `.ironclaw-reborn-bundled.json` and its owner string
    `ironclaw_reborn_composition_bundled_skill` (compared on install for idempotency)
- **`#[path]`:** `slack_serve/e2e_tests.rs` uses `#[path = "e2e_auth_challenge.rs"]`.
- **Leaked-block extraction** (PRs 3,6,7,9,10) is code motion of private free functions
  out of runtime.rs/factory.rs — the only steps that are more than `mod`-path renames.

## 4. Eval harness — for subagents in a loop

Each PR-step is executed by a subagent; a separate verifier subagent scores it. A step
**passes** only if every machine gate is green AND the judge rubric has no FAIL.

### 4.1 Machine gates (scriptable; run in this order, all from repo root)

```bash
# G1 fmt
cargo fmt --check
# G2 lint, zero warnings
cargo clippy --all --benches --tests --examples --all-features
# G3 full-feature build+test of the crate and its consumers
cargo test -p ironclaw_reborn_composition --features "webui-v2-beta,slack-v2-host-beta,openai-compat-beta,root-llm-provider,test-support,postgres"
cargo test -p ironclaw_reborn_webui_ingress
cargo build -p ironclaw_reborn_cli
# G4 boundary tests (the load-bearing architectural contract)
cargo test -p ironclaw_architecture
# G5 cfg-matrix (catches mis-hung feature gates — the #1 hazard)
cargo check -p ironclaw_reborn_composition --no-default-features
cargo check -p ironclaw_reborn_composition            # default features
cargo check -p ironclaw_reborn_composition --all-features
# G6 workspace unit tier
cargo test
```

Flake discrimination: known cross-binary parallel flakes (budget_e2e `f1_happy_path`,
webui_v2_e2e `nearai_provider_save*`, runtime nearai env-var/keychain races) pass with
`--test-threads=1`; a failure that persists serially is a real regression. `--test-threads=1`
is only a *discriminator*, not the fix — the durable fix is serializing env-mutating tests
through the canonical `ironclaw_common::env_helpers::lock_env()`; see the "shipping speed"
lever on hermetic tests. Do not adopt `--test-threads=1` as the default runner.

### 4.2 Structural gates (assert the refactor is actually happening)

```bash
# S1 flat-module count in lib.rs must be ≤ the step's target
# (final target ≤ 12: root + 10 domain modules per §3/§6.2, plus test_support)
grep -cE '^\s*(pub )?mod [a-z_0-9]+;' crates/ironclaw_reborn_composition/src/lib.rs
# S2 god-file ceiling: no src file (tests excluded) over 4,000 lines by PR #11
find crates/ironclaw_reborn_composition/src -name '*.rs' ! -name '*tests*' -exec wc -l {} + | sort -rn | head -5
# S3 no new crates: workspace member count unchanged (76)
grep -c '^    "' Cargo.toml   # or: cargo metadata | jq '.workspace_members | length'
# S4 public-surface freeze: the pub-use snapshot diff must be EMPTY.
# Captures each `pub use` block AND any #[cfg(...)]/attributes immediately above it,
# so a silently dropped/changed feature gate on an export is caught, not missed.
awk '/^#\[/{attr=(attr ? attr "\n" : "") $0; next} /^pub use/{p=1; if(attr){print attr; attr=""}} p{print; if(/;/){p=0}} !p{attr=""}' \
  crates/ironclaw_reborn_composition/src/lib.rs > "$TMPDIR/pubuse.after"
diff docs/plans/composition-pubuse.snapshot "$TMPDIR/pubuse.after"
# S5 move-purity: diff must be ≥90% renames/moves (logic changes ≈ 0)
git diff --find-renames=90% --stat main...HEAD
```

(Generate `composition-pubuse.snapshot` once, before PR #1, and commit it next to this
doc; PRs that must legitimately re-point a `pub use` update the snapshot in the same
commit with a one-line justification.)

### 4.3 Judge rubric (verifier subagent, per step)

- **Behavior-preservation:** zero test deletions/weakenings; any test edit is path-only.
- **Boundary fidelity:** files landed in the module §3 assigns; straddlers relocated per
  the map, not dragged along.
- **cfg integrity:** every moved `#[cfg]` re-attached; G5 all three profiles green.
- **No smuggled refactors:** no renamed types, no signature changes, no "while I was
  here" cleanups (Rule 3 — surgical changes).
- **Frozen constants intact:** bundled-skills marker + JSON names byte-identical.
- **Verdict:** PASS / FAIL(+reason) / BLOCKED(+missing input). FAIL loops back to the
  implementer with the reason; two consecutive FAILs on a step escalate to a human.

### 4.4 Loop protocol

```
for step in 1..=11:
    implementer subagent: execute step per §3, run G1–G6 + S1–S5 locally
    verifier subagent (fresh context): re-run gates, apply §4.3 rubric
    if PASS → commit, open PR, next step
    if FAIL → implementer retries with verifier's reason (max 2)
    rebase each step on fresh main before starting (steps are intra-crate → conflicts stay local)
```

## 5. Testing strategy — coverage to add BEFORE the risky steps

Existing coverage is strong (composition ~705 tests, ingress 170, architecture 35), and
pure `mod`-path moves are compiler-verified — blanket new coverage is NOT needed. The
gap is the five **leaked-block extractions** (§3 PRs 3,6,7,9,10): today those blocks are
private free functions exercised only implicitly through full assembly. Per
`.claude/rules/testing.md` ("test through the caller"), pin them via their callers
before moving them. Write each test first, watch it pass on main, then move the block —
the test must stay green unmodified.

| # | New test (through the caller) | Pins | Before PR |
|---|---|---|---|
| T1 | `build_reborn_services` with budget settings → assert budget accountant/gate-evidence sinks wired (observable via gate outcomes on a scripted turn) | budget-evidence block in runtime.rs | 3 |
| T2 | `build_reborn_runtime` with `root-llm-provider` on/off → assert gateway kind (stub vs swappable vs production) + `apply_startup_stored_llm_key` effect via `llm_config_service` state | LLM-gateway + startup-key blocks | 6 |
| T3 | `build_reborn_services` with a nearai session token in env/keychain-stub → assert MCP bootstrap registered (extend the existing `runtime_nearai_mcp_bootstraps_*` tests to cover the factory path if not already) | nearai bootstrap block | 6 |
| T4 | `build_reborn_runtime` → assert local-dev skill selector config + `build_skill_learning_provider` produce the same selector given fixed workspace fixtures | skill-selector block | 7 |
| T5 | bundled-skills idempotency: install → re-run `ensure_bundled_reborn_skills_installed` → assert no re-install/orphan; plus a const-pin test `assert_eq!(BUNDLED_MARKER_OWNER, "ironclaw_reborn_composition_bundled_skill")` and the two OUT_DIR JSON filenames | marker/filename freeze | 7 |
| T6 | trigger-poller wiring: `build_reborn_services`+`spawn_trigger_poller` with a fake trigger → assert trusted-submit path fires and delivery hook is invoked (extends existing dynamic-Slack-trigger delivery test) | trigger-poller block in factory.rs | 9 |
| T7 | Slack delivery hook: after `build_reborn_runtime`, set `PostSubmitDeliveryHook` and drive a turn → assert hook called (pins the OnceLock repath) | slack OnceLock tie | 8 |
| T8 | webui auth-interaction: `build_webui_auth_interaction_service` via `webui_v2_app` request path → assert legacy-identity fold + audit append | webui-auth block | 10 |

Consolidation rule applies: where an existing test already drives the caller (several
`runtime::`/`factory::` tests do), **extend it** with the missing assertion instead of
adding a new test. T1–T8 name the behavior to pin, not necessarily eight new files.

Beyond composition (later, JIT): a characterization suite for
`ironclaw_reborn_traces::contribution` before dissecting the 14.5k-line file, and
snapshot tests on `RebornServicesApi` wire DTOs before any interface split.

## 6. Final architecture — the whole-reborn target map

Where the workspace should land once §3 (composition), the Tier-A god-file fixes,
the tail folds, and the JIT interface/seam work are done. This is the picture the
roadmap features get built against.

### 6.1 Target numbers

| Metric | Today | Target |
|---|---:|---:|
| Workspace crates | 76 | ~69 (6 folds + agent_loop merge; new crates ONLY for new channel/ingress adapters) |
| composition top-level `mod`s | ~120 | ≤ 12 |
| Largest non-test src file (reborn crates) | 14.5k (`reborn_traces/contribution.rs`) | ≤ 4k |
| `RebornServicesApi` | 1 trait, ~70 methods | ~7 domain port traits (§6.4) |
| Cross-cutting feature footprint | 9–18 crates | 4–8 crates/modules (§7) |

### 6.2 Layer map (final state)

Acyclic, higher depends on lower. Markers: `▣` internal decomposition,
`⊕` absorbs a folded crate, `⊘` retired in Tier B.

```
L6  COMPOSITION & BINARIES
    reborn_composition ▣ (11 modules: root, slack, webui, automation,
      extension_host, projection, llm_admin, product_auth, observability,
      outbound, support::fs)          reborn_cli
    [tests-only: ironclaw_architecture, ironclaw_hooks_parity]

L5  INGRESS / CHANNEL ADAPTERS        ← new crates allowed HERE ONLY
    webui_v2, webui_v2_static, reborn_webui_ingress,
    slack_v2_adapter, telegram_v2_adapter,
    (future: voice_adapter, native_app_ingress — roadmap)

L4  PRODUCT / WORKFLOW
    product_workflow ⊕storage ▣ (API split §6.4),
    product_adapters, product_adapter_registry, wasm_product_adapters,
    first_party_extensions, first_party_extension_ports,
    reborn_openai_compat ⊕storage

L3  RUNTIME
    host_runtime, reborn_loop (= loop_support ⊕ agent_loop,
      ▣ capability_port split), reborn, reborn_config

L2  DOMAIN SERVICES
    llm, safety, skills ⊕skill_learning, secrets, hooks (+postgres/libsql),
    reborn_event_store, event_projections, event_streams,
    reborn_traces ▣ (contribution.rs split), approvals, capabilities,
    processes, process_sandbox, extensions, extractors, network,
    runtime_policy, scripts, wasm ⊕wasm_sandbox_core, wasm_limiter,
    mcp, memory_native, triggers, outbound, dispatcher,
    auth ⊕oauth, embeddings

L1  DOMAIN VOCABULARY / STATE
    turns (type-split JIT), threads, conversations, run_state, resources,
    memory, product_context, prompt_envelope, authorization, trust,
    reborn_identity, attachments

L0  KERNEL
    host_api, common, events, filesystem

V1 (Tier B — retire as reborn absorbs): src/ monolith (335k),
    engine ⊘, gateway ⊘, tui ⊘
```

### 6.3 Composition internal target — see §3 (the 11-module dissection).

### 6.4 `RebornServicesApi` split target (JIT — each slice lands with the
feature that needs it)

The 70-method trait in `product_workflow` decomposes into domain ports;
`RebornServices` keeps aggregating them, HTTP crates depend only on the
slices they mount:

| Port trait | Methods (today's cluster) | First roadmap driver |
|---|---|---|
| `ThreadsApi` | create/list/get thread, timeline, cursors | — (stable) |
| `TurnsApi` | submit_turn, cancel, retry, drive outcomes | Missions |
| `GatesApi` | resolve_gate, approvals, auth challenges | Permission management |
| `ExtensionsApi` | lifecycle, install, capabilities, credentials | Admin-configurable skills/tools |
| `LlmAdminApi` | provider config/catalog/keys/reload | — (stable) |
| `MemoryApi` (new) | retrieval, write-back, profile | Long-term memory |
| `AutomationApi` | triggers, routines, poller control | Missions |

### 6.5 Seam registry — the extension points the roadmap needs

The actual lever for cross-cutting cost. Each seam is built JIT with its first
feature, then reused. New-feature rule: **land behind a seam, or add the seam.**

| Seam | Owner (crate::module) | Status | Roadmap features served |
|---|---|---|---|
| Channel adapter pattern | L5 adapter crate + `composition::<channel>` module | **exists** (slack, telegram) | Slack main channel, voice, native app |
| OAuth provider registry ("provider = one file") | `composition::product_auth::oauth` | **exists** (#5502 proved it) | onboarding, new integrations |
| `OutboundDeliveryTargetProvider` | `product_workflow` (trait) + per-channel impls | **exists** | any channel delivery |
| `PostSubmitDeliveryHook` | `composition::root` OnceLock → `composition::slack` impl | **exists** | triggered-run delivery |
| Memory retriever + prompt-assembly hook | `prompt_envelope` + `reborn_loop` | **JIT** — build with LTM | Long-term memory, user-voice model |
| Learning hook (turn-outcome → skill/memory write-back) | `reborn_loop` + `skills::learning` | **JIT** — build with self-learning | Self-learning loops |
| Permission policy port | `authorization` + `GatesApi` slice | **JIT** — build with permission mgmt | Permission management, multi-tenant |
| Credential injection surface | `secrets` + `capabilities` | partial (WASM injector exists) | Secrets usage with skills/tools |
| Tenancy/actor identity | `reborn_identity` + `trust` | partial | Multi-tenant collaboration |

## 7. Roadmap overlay — where each feature lands in the final architecture

Footprint = crates/modules edited (composition counts once; its module named).
"Now" figures are the traces measured in this exercise; "final" assumes §6.

| Roadmap feature | Lands in (final architecture) | Now → final footprint |
|---|---|---|
| Self-learning loops (#1187-shaped) | `skills::learning`, learning hook in `reborn_loop`, `memory_native`, `composition::extension_host`, `MemoryApi` | ~15–18 → **~5–6** |
| Long-term memory | `memory_native`, retriever seam (`prompt_envelope`+`reborn_loop`), `MemoryApi`, `composition::root` wiring, `webui_v2` | ~14 → **~5–6** |
| Multi-tenant collaboration | `reborn_identity`, `trust`, `authorization`, `threads`/`conversations`, `host_runtime`, `composition::root`, `webui_v2` | ~15 → **~7–8** (genuinely broad) |
| Permission management | `authorization` policy port, `approvals`, `capabilities`, `GatesApi`, `composition::projection`+`webui` | ~9 → **~5** |
| Secrets usage w/ skills+tools | `secrets`, `capabilities` injection surface, `skills`, `composition::extension_host` | ~8 → **~4** |
| Missions | `TurnsApi`+`AutomationApi`, `reborn_loop`, `composition::automation`, `webui_v2` | ~10 → **~5** |
| Admin-configurable skills/tools | `ExtensionsApi`, `skills`, `composition::extension_host`, `webui_v2` | ~8 → **~4** |
| Slack as main channel | `slack_v2_adapter`, `composition::slack` | ~3 → **~2** (already cheap) |
| Onboarding channel-first | `composition::product_auth`+`webui`, `webui_v2` | → **~3** |
| Custom build tools | `wasm`, `capabilities`, `composition::extension_host` | → **~3–4** |
| User-voice model | `llm`, retriever seam, new L5 `voice_adapter` | → **~4** |
| Native app (screen capture) | new L5 `native_app_ingress` + `composition` module | → **~3** |
| Clean up old architecture | Tier B (§8) | the epic itself |

The reduction comes from three things only: composition modularization (edits land
in one named module instead of grep-across-153k), the interface slices (features
extend one port, not the 70-method trait), and the seams (cross-cutting features
plug in instead of threading by hand). Crate merging contributes almost nothing —
which is why we aren't doing it (except `agent_loop`).

## 8. Tier B end-state — v1 retirement (separate epic, sequenced by product)

The `src/` monolith (335k lines, 411 files; god-files `extensions/manager.rs`
14.9k, `bridge/router.rs` 14.3k, `channels/wasm/wrapper.rs` 10.0k) plus v1-era
crates (`engine`, `gateway`, `tui`) retire as reborn reaches feature parity:
channels → L5 adapters, tools/extensions → `extension_host` + registry, engine
v1 → `reborn_loop`/`host_runtime`, web gateway → `webui_v2`+ingress. End state:
one stack, L0–L6 only. Do NOT restructure v1 internals meanwhile — code being
deleted doesn't get refactored; it gets migration-tested and removed.

## 9. What we are explicitly NOT doing

- No new crates for composition domains (http_kit/product_auth/slack_host/llm_admin/
  extension_host as crates: dead).
- No merging backbone crates (fan-in ≥ 24) — proven net-negative.
- No speculative extension seams — each seam lands with the roadmap feature that needs it.
- No `pub use` shim layers, no wrapper structs around config types.

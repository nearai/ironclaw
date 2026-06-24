# Decomposing `ironclaw_reborn_composition` — Fusion Design

**Status:** Accepted by council (Opus 4.8 + GPT‑5.5 xhigh). Being delivered as an **incremental series on fresh `main`** (branch `reborn/composition-decomposition`), one isolated crate per PR. Sections 1–12 below are the ratified design and remain current.
**Date:** 2026-06-21 (status updated 2026-06-22)
**Owners:** Reborn composition refactor

## Completion status (incremental, on fresh `main`)

A first attempt extracted all six crates at once on a branch taken earlier; during the work `main` advanced ~168 commits / +35k lines on this same crate (now ~132k) and **restructured the extracted modules** (e.g. `product_auth_durable.rs` → a 7-submodule `product_auth_durable/` directory; `product_auth_runtime_credentials/` and `extension_lifecycle/` likewise; new cross-cutting modules `credential_refresh_worker` / `extension_activation_credentials` / `product_auth_refresh_lock` now reach into the auth cluster). Reconciling that merge would have risked silently dropping shipped `main` features, so the big-bang branch was abandoned and the work is being re-applied incrementally on fresh `main`.

| # | Crate / move | Status |
|---|---|---|
| 1 | Step 1 mount-seam inversion + `ironclaw_reborn_http_kit` (6 middleware modules) | ✅ **PR #5137** (green) |
| 2 | `ironclaw_reborn_product_auth` (auth/OAuth) — re-scope for the `product_auth_durable/` split + integrate the 3 new cross-cutting auth modules | ⏳ next |
| 3 | `OutboundDeliveryTargetProvider` vocab → `ironclaw_product_workflow` (Slack-cycle prerequisite) | ⏳ |
| 4 | `ironclaw_reborn_slack_host` (concrete-handle `SlackHostRuntimeHandles<F>`) | ⏳ |
| 5 | `ironclaw_reborn_llm_admin` (`ResolvedRebornLlm` inverted out of `runtime_input.rs`) | ⏳ |
| 6 | `ironclaw_reborn_extension_host` (**partial** — see below) | ⏳ |

The per-crate line counts measured on the abandoned branch were roughly: http_kit ~2.6k, product_auth ~16.8k, slack_host ~17.5k, llm_admin ~4.1k, extension_host ~8.4k (those clusters have since grown on `main`; re-scope each before extracting).

**Why extension_host is partial:** `bundled_skills.rs` is tied to composition's `build.rs` OUT_DIR output and carries a behavior-sensitive marker string (`ironclaw_reborn_composition_bundled_skill`); `skill_listing` depends on it; `extension_lifecycle_command` needs `factory::RebornServices`. Moving them would force a build-script migration and a behavior-visible marker change, so they stay in composition (a defensible boundary — bundled skills are a composition build-time asset, the command is composition orchestration).

**Independent follow-ups (after the crate cuts):**
- Domain relocation: trigger creator-pairing hook + fire-time lookup → `ironclaw_triggers`; `builtin_first_party_trust_policy` + `*_allowed_effects` → first-party crates; one-time migrations → quarantined `composition::migrations` module.
- Internal `runtime.rs` / `factory.rs` decomposition (plans #4471/#4469).

**Verification per crate:** crate tests + `cargo test -p ironclaw_reborn_composition --features "webui-v2-beta,slack-v2-host-beta,openai-compat-beta,root-llm-provider,test-support"` + ingress + `cargo test -p ironclaw_architecture` + clippy, all green. **Pre-existing test-isolation flakes on `main`** (NOT caused by this refactor): several `runtime::` / nearai tests share process-global state (env var for the nearai session token, OS keychain, temp files) and fail *non-deterministically* under full-suite parallel load — they pass deterministically when run serially (`--test-threads=1`). Use serial runs to discriminate a real regression from this flakiness.

---

## 1. Problem statement

`ironclaw_reborn_composition` is a ~96.5k-line god crate. It is the legitimate Reborn composition root (owns `RebornRuntime`, `build_reborn_runtime`/`build_reborn_services`, `factory.rs`, `runtime.rs`, profiles, projection, readiness, local-dev), but it has accreted four product/service domains that do not belong in a root: a ~20k-line Slack product, a ~15k-line product-auth/OAuth stack, ~5k of LLM-admin services, and ~5k of extension/MCP/skills host code, plus a generic descriptor-driven HTTP middleware kit. The four feature flags (`webui-v2-beta`, `slack-v2-host-beta`, `openai-compat-beta`, `root-llm-provider`) are effectively four crates implemented as `cfg` gates inside one crate, so any change rebuilds all 96.5k lines plus the two downstream binaries and multiplies the test/feature matrix.

## 2. Goals and non-goals

**Goals.** Behavior-preserving decomposition into focused crates; the feature-cfg matrix collapses into ordinary dependency edges; incremental compile touches only the changed leaf crate; module boundaries become compile-enforced; each step independently shippable and green; minimal downstream blast radius.

**Non-goals.** No behavior change. No semver-stable public API for the new internal crates. No `runtime.rs`/`factory.rs` *internal* line-split until the crate cuts are done (separate, last). No new persistence model (existing dual libSQL+Postgres preserved).

## 3. Constraints and assumptions

- No existing test may fail; tests may be **added first** to lock behavior before a cut.
- `reborn_composition_boundaries.rs`: the 44 substrate crates must never depend on composition; composition's public API stays facade-shaped (no `RebornStorageInput`/raw stores/legacy bridge); installed-tier hooks route through `HookRegistrar::install()`.
- `reborn_dependency_boundaries.rs`: product/API crates may expose `Router`/`IngressRouteDescriptor` but must never bind a `TcpListener` or call `axum::serve`; CLI enters Reborn only via `build_reborn_runtime()`.
- New persistence supports both libSQL and Postgres.
- Prefer **deleting** cfg-complexity over rearranging it. New files < 800 lines.
- **Verified facts** (checked in code, not model assertion):
  - `OutboundDeliveryTargetProvider` + `OutboundDeliveryTargetEntry` are `pub(crate)` in `composition/src/outbound_preferences.rs:21,28`; Slack implements the trait at `slack_outbound_targets.rs:575`. The value vocabulary (`RebornOutboundDeliveryTargetId/Summary/Capabilities/...`) **already lives in** `ironclaw_product_workflow` (`lib.rs:171-175`).
  - `google_provider_spec()`/`notion_provider_spec()` return `HostOAuthProviderSpec` with `ExchangeScopePolicy` from `oauth_provider_client` (`google_oauth/mod.rs:3-10`, `notion_oauth.rs:8`) — host token-exchange policy, not extension packages.
  - `ironclaw_first_party_extensions` **already owns** the GSuite package specs (`src/gsuite/manifest.rs:16`, `gsuite_package_specs()`); composition's `gsuite.rs` is only a `FirstPartyCapabilityRegistry` registration adapter (`gsuite.rs:12,22,33,47`).

## 4. Final design — target crate graph

Six Reborn crates, all **above** substrate and **below** the two binaries. `ironclaw_reborn_composition` remains the only god-root the binaries name.

```
   ironclaw_reborn_cli          ironclaw_reborn_webui_ingress
            │                              │
            └──────────────┬───────────────┘
                           ▼
            ironclaw_reborn_composition   ── true root: RebornRuntime, factory.rs,
              │   │   │   │   │              runtime.rs, input/runtime_input, lifecycle,
              │   │   │   │   │              projection, readiness, profile, budget,
              │   │   │   │   │              local-dev, webui.rs (lowering wrappers),
              │   │   │   │   │              hooks/, + thin `pub use` re-export shims
   ┌──────────┘   │   │   │   └──────────────┐
   ▼              ▼   │   ▼                  ▼
 slack_host  product_auth │ llm_admin    extension_host
   │   │          │       │   │              │
   │   └──────────┤       │   │              │
   ▼              ▼       ▼   ▼              ▼
        ironclaw_reborn_http_kit  (mount vocabulary + 6 webui_* middleware modules)
                           │
                           ▼
   substrate: ironclaw_webui_v2(+static), ironclaw_auth, ironclaw_host_api,
   ironclaw_product_workflow, ironclaw_product_adapters, ironclaw_turns,
   ironclaw_threads, ironclaw_slack_v2_adapter, ironclaw_wasm_product_adapters,
   ironclaw_outbound, ironclaw_conversations, ironclaw_host_runtime,
   ironclaw_first_party_extensions(+_ports), ironclaw_extensions, ironclaw_mcp,
   ironclaw_skills, ironclaw_llm, ironclaw_secrets, ironclaw_reborn_config, …
```

**Crate contents:**

| Crate | Modules moved in | Cargo feature it owns |
|---|---|---|
| `ironclaw_reborn_http_kit` *(scaffold exists)* | `webui_serve`→`serve`, `webui_body_limit`→`body_limit`, `webui_rate_limit`→`rate_limit`, `webui_route_match`→`route_match`, `webui_ws_origin`→`ws_origin`, `webui_operator_auth`→`operator_auth`; mount vocabulary (`ProtectedRouteMount`/`PublicRouteMount`), `compose_webui_v2_app` | `openai-compat-beta` (auth-evidence injector for OpenAI-compat mounts) |
| `ironclaw_reborn_product_auth` | `auth`, `auth_prompt` (`AuthChallengeProvider`), `product_auth_durable`, `product_auth_serve/`, `product_auth_providers`, `product_auth_runtime_credentials`, `manual_token_flow`, `oauth_dcr(_protocol)`, `oauth_provider_client`, `oauth_gate`, `google_oauth/`, `notion_oauth` | — (built under composition's `webui-v2-beta`) |
| `ironclaw_reborn_slack_host` | all `slack_*` (~20k) | — (built under composition's `slack-v2-host-beta`) |
| `ironclaw_reborn_llm_admin` | `llm_catalog`, `llm_config_service`, `llm_key_store`, `llm_reload`, `provider_admin*`, `provider_repo`, `nearai_login_serve` | `root-llm-provider` |
| `ironclaw_reborn_extension_host` | `available_extensions`, `extension_installation_store`, `extension_lifecycle*`, `extension_lifecycle_command`, `mcp`, `mcp_discovery`, `nearai_mcp`, `bundled_skills`, `skill_listing`, the **GSuite host-registration adapter** (`gsuite.rs`) | — |
| `ironclaw_reborn_composition` | the true-root list above + `pub use` shims | declares `webui-v2-beta`/`slack-v2-host-beta`/`openai-compat-beta`/`root-llm-provider`/`libsql`/`postgres`, forwarding to crates |

**Inter-crate edges:** `slack_host → http_kit, product_auth, product_workflow, product_adapters, product_workflow_storage, slack_v2_adapter, wasm_product_adapters, outbound, conversations, host_runtime, filesystem, threads, turns`; `product_auth → http_kit` (WebUI mounts only) + `ironclaw_auth`; `llm_admin → ironclaw_llm, reborn_config, secrets, product_workflow`; `extension_host → first_party_extensions(+_ports), extensions, host_runtime, mcp, skills, product_workflow`.

## 5. Key decisions and alternatives rejected

**D1 — Slack upward cycle → concrete-handle injection (NOT a trait).** Both slots independently proposed the same shape. `slack_host` exposes:

```rust
pub struct SlackHostRuntimeHandles<F: RootFilesystem + 'static> {
    pub host_state_filesystem: Arc<ScopedFilesystem<F>>,
    pub host_runtime_http_egress: HostRuntimeHttpEgressPort,
    pub thread_service: Arc<dyn SessionThreadService>,      // ironclaw_threads
    pub turn_coordinator: Arc<dyn TurnCoordinator>,         // ironclaw_turns
    pub approval_interactions: Arc<dyn ApprovalInteractionService>,
    pub auth_interactions: Arc<dyn AuthInteractionService>,
    pub auth_challenges: Option<Arc<dyn AuthChallengeProvider>>, // from product_auth
}
pub fn build_slack_host_beta_mounts<F: RootFilesystem + 'static>(
    handles: SlackHostRuntimeHandles<F>, config: SlackHostBetaConfig,
) -> Result<SlackHostBetaMounts, SlackHostBetaBuildError>;
```

Composition's `webui.rs` constructs the struct from `RebornRuntime` accessors and keeps `build_slack_host_beta_mounts(&RebornRuntime, config)` as a shim. *Rationale:* every handle type already lives below composition; there is exactly one `RebornRuntime`, so a `WebuiRuntimeHandles` trait is `RebornRuntime` by another name — indirection that would invite scope creep. Concrete handles make required authority explicit, keep coherence trivial (Slack only impls foreign traits for its own types), and make Slack unit-testable with fakes. *Rejected:* trait-injection (only justified with multiple runtime implementers).

**D2 — Six separate crates; do NOT merge llm_admin + extension_host.** GPT‑5.5 blocked the merge; Opus conceded after verification. They are not the same surface: `extension_host` is exercised by `factory.rs` for **local-runtime extension activation and first-party capability registration** — it is *not* purely `webui-v2-beta`-gated — while `llm_admin` is `root-llm-provider`/`ironclaw_llm`/operator-secret code. They share no real plumbing (`ProviderRepo`/`LlmKeyStore` are LLM-only; extension credentials flow through product-auth runtime credentials). A merged `webui_services` crate would be "a compile blob with a nicer name," fusing two unrelated security-review surfaces. *Rejected:* the merged crate.

**D3 — OAuth exchange specs stay in product_auth; only GSuite registration glue moves.** `google_provider_spec()`/`notion_provider_spec()` are `HostOAuthProviderSpec` + `ExchangeScopePolicy` (token endpoints, DCR resource, secret-handle policy, scope fallback) — host token-exchange policy owned by product-auth, not extension implementations. They stay in `ironclaw_reborn_product_auth`. GSuite **package specs already live in** `ironclaw_first_party_extensions`; composition's `gsuite.rs` is only a `FirstPartyCapabilityRegistry` registration adapter, which moves to `extension_host`. *Rejected:* moving google/notion catalogs into first-party (crosses host-auth/extension-impl ownership and re-creates a split-brain).

**D4 — Relocate the outbound target-provider vocabulary into `ironclaw_product_workflow`.** Slack implements composition's `pub(crate) OutboundDeliveryTargetProvider`; an extracted Slack cannot name composition. The value IDs are already in `product_workflow`, so the move is: relocate the **`OutboundDeliveryTargetProvider` trait + `OutboundDeliveryTargetEntry`** down to `ironclaw_product_workflow`; keep `OutboundDeliveryTargetRegistry` and `RebornOutboundPreferencesFacade` composition-owned. (`OutboundDeliveryTargetEntry` embeds `ReplyTargetBindingRef` from `ironclaw_turns`; `product_workflow` already depends on `ironclaw_turns`, so the edge is free.) This is a prerequisite seam for the Slack cut (Step 4).

**D5 — Back-compat via thin `pub use` shims, not call-site updates.** Every symbol at `ironclaw_reborn_composition::X` keeps `pub use ironclaw_reborn_<crate>::X;`. `reborn_cli`, `reborn_webui_ingress`, and the `product_workflow` dev-dep compile **unchanged**, and `composition_public_api_is_facade_shaped` stays green (shape preserved). Internal call-site migration to direct crate paths is an optional later cleanup that never gates an extraction. *Rejected:* immediate downstream import rewrite (unnecessary blast radius).

## 6. Runtime / security / boundary implications

- **Feature-flag mapping:** public features stay declared on composition (binaries flip them there) and forward as optional-dep edges — `webui-v2-beta → dep:http_kit (+identity, wrappers)`, `slack-v2-host-beta → webui-v2-beta + dep:slack_host`, `openai-compat-beta → http_kit/openai-compat-beta` (composition-owned), `root-llm-provider → dep:llm_admin + ironclaw_reborn/root-llm-provider`. `root-llm-provider` must **not** pull `extension_host`. `libsql`/`postgres` forward only to crates that build backend-specific persistence.
- **Boundary tests:** add the five new crates to the product/API allowlist in `reborn_dependency_boundaries.rs` (they expose `Router`/descriptors, bind no listeners); never add them to `SUBSTRATE_CRATES` in `reborn_composition_boundaries.rs` — the test's structural rule (substrate ↛ composition) keeps holding because no new crate names composition.
- **Coherence/orphan:** all relocations move a trait *with* its owning concrete types or leave the impl in the crate that owns either side. No foreign-foreign impls created (`SlackHostRuntimeHandles` is a plain struct).

## 7. Domain-logic relocation (misplaced code leaving the root)

- Trigger creator-pairing hook + fire-time run lookup → `ironclaw_triggers`.
- `builtin_first_party_trust_policy` + `*_allowed_effects` tables → first-party crates (`ironclaw_first_party_extensions` / `extension_host`), **after** behavior is locked.
- One-time migrations (legacy skill backfill; pre-#4381 WebUI identity fold) → a quarantined `composition::migrations` module, each with a doc-comment naming its deletion criterion (the version after which no un-migrated rows exist).

## 8. Test and validation plan

Per step, all must stay green: `cargo test -p ironclaw_reborn_composition --features test-support,webui-v2-beta,slack-v2-host-beta,openai-compat-beta,root-llm-provider` (the 831-test suite), `ironclaw_reborn_webui_ingress` tests, `cargo test -p ironclaw_architecture` (boundary tests), `cargo clippy --all-features` (zero warnings). Feature-matrix smoke builds: `default`, `webui-v2-beta`, `slack-v2-host-beta`, `openai-compat-beta`, `root-llm-provider`, and **both** `--no-default-features --features postgres,webui-v2-beta` and the `libsql` twin. Add behavior-locking tests before any cut where a `pub(crate)`→`pub` visibility change or a trait relocation could alter observable behavior.

## 9. Rollout / migration plan (each step independently shippable & green)

1. **http_kit** *(Step 2; scaffold + Step 1 already done/green).* Add `ironclaw_reborn_http_kit` to workspace members; move the six `webui_*` module bodies + `compose_webui_v2_app`; composition depends on it under `webui-v2-beta`; add `pub use` shims (`webui_serve::*`, `RateLimitConfigError`).
2. **product_auth.** Extract auth/OAuth routes + provider clients + `auth_prompt`; keep `RebornProductAuthServices` re-exports. Google/Notion exchange specs stay here.
3. **outbound vocabulary.** Relocate `OutboundDeliveryTargetProvider` trait + `OutboundDeliveryTargetEntry` into `ironclaw_product_workflow`; composition facade behavior unchanged. (Prereq for Step 4.)
4. **slack_host** *(riskiest — ship alone).* Introduce `SlackHostRuntimeHandles<F>`; change `build_slack_host_beta_mounts` + helpers to take it; composition `webui.rs` constructs it; depends on `product_auth` for `AuthChallengeProvider`. Shim all `slack_*` symbols.
5. **llm_admin.** Extract `llm_*`/`provider_*`/`nearai_login_serve`; move `root-llm-provider` feature here; composition forwards; shim symbols.
6. **extension_host.** Extract `extension_lifecycle*`/`available_extensions`/`mcp*`/`bundled_skills`/`skill_listing` + GSuite host-registration adapter (`gsuite.rs`); depends on `first_party_extensions(+_ports)`, `host_runtime`. Shim symbols.
7. **Domain relocation + feature-edge cleanup** (Section 7); delete now-redundant intra-composition cfg-gates that became crate boundaries.
8. **(Independent) internal `runtime.rs`/`factory.rs` split** into composition-owned submodules — only after 1–7 are green.

## 10. Risks and mitigations

- **R1 orphan/coherence on extraction.** Mitigation: relocate traits with owning types; grep cross-cluster `impl`s before each cut; `SlackHostRuntimeHandles` is a plain struct.
- **R2 boundary-test regressions.** Mitigation: add new crates to product/API allowlist in the same PR; never to `SUBSTRATE_CRATES`; run `ironclaw_architecture` tests each step.
- **R3 feature-forwarding drift (dual backend).** Mitigation: each persisting crate declares its own `libsql`/`postgres` forwarding; CI smoke-builds both backends every step.
- **R4 visibility widening.** `pub(crate)`→`pub` on relocated items (`OutboundDeliveryTargetEntry`, `SlackPersonalBindingRouteState`, etc.) could over-expose. Mitigation: widen only what crosses the new crate line; keep facade-shape test green; lock behavior with a test before the cut.
- **R5 internal split slipping into a crate cut.** Mitigation: Step 8 is strictly last and separate; crate cuts are file-moves only.
- **R6 extension_host mis-gated as WebUI-only.** Mitigation (from GPT‑5.5): `extension_host` is plain (no feature of its own); `factory.rs` uses it for local-runtime activation; do not gate it behind `webui-v2-beta` or pull it via `root-llm-provider`.

## 11. Agreement ledger

| Decision | Opus 4.8 | GPT‑5.5 | Resolution |
|---|---|---|---|
| Slack cycle → concrete-handle `SlackHostRuntimeHandles` | proposed | proposed (same shape) | **Agree** |
| Six separate crates; split llm_admin & extension_host | proposed merge → **conceded split** | proposed split (blocked merge) | **Agree → split** |
| Google/Notion OAuth specs stay in product_auth | proposed move → **conceded stay** | proposed stay (verified) | **Agree → stay** |
| GSuite specs already in first_party; only registration glue → extension_host | conceded | proposed | **Agree** |
| Relocate `OutboundDeliveryTargetProvider` trait → product_workflow | adopted & sharpened (named the exact artifact) | proposed | **Agree** |
| Back-compat via `pub use` shims | proposed | proposed | **Agree** |
| Ordering: http_kit → product_auth → outbound → slack → llm → ext → relocate → internal split last | proposed | proposed | **Agree** |
| Migrations quarantined; trigger pairing → triggers; trust/effects → first-party | proposed | proposed | **Agree** |

Opus verdict on fused design: ACCEPT_WITH_CHANGES (changes folded in). GPT‑5.5 blocked Opus's *original* merge/OAuth-move positions; those are abandoned, so no live disagreement remains.

## 12. Unresolved blockers

None. Final signoff: **Opus 4.8 ACCEPT**, **GPT‑5.5 ACCEPT_WITH_NONBLOCKING_NOTES**, zero blockers from either.

Execution-time checks (not design blockers):
- **Step-4 interaction-service prereq is already satisfied (verified).** `ApprovalInteractionService` and `AuthInteractionService` are `pub` traits in substrate `ironclaw_product_workflow` (and `AuthInteractionService` also exists in `ironclaw_auth`), so `SlackHostRuntimeHandles` can name them today — the doc's "if private, move" branch is dead. The only genuine carry-through is moving `AuthChallengeProvider` (composition's `auth_prompt.rs`) into product_auth, which **Step 2 already covers**.
- **Trait-name collision to disambiguate.** Two same-named `AuthInteractionService` traits exist (`ironclaw_auth` and `ironclaw_product_workflow::auth_interaction`); `SlackHostRuntimeHandles`'s field doc-comment must name the fully-qualified path so the Step-4 implementer wires the right one.
- **Coherence grep before Step 4.** Grep for any `slack_*` `impl` of a `first_party_extensions`/`product_adapters` trait to avoid a surprise orphan-rule break during extraction.

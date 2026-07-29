# Target Architecture — Completion Checklist

**Definition of done:** when every box below is checked, the target architecture in [PROPOSAL.md](PROPOSAL.md) is 100% reached — code layout, dependency graph, enforcement, and agent-facing guidance all match the specification, with zero standing `LAYER_MATRIX_EXCEPTIONS` and the §2.6 dead surface gone.

Conventions: every code item lands with its tests and its guidance updates in the same PR (house rule). Items tagged **[#6696]** are gated on the process-journal collapse (or an equivalent approved design) landing — they are part of "done" but cannot start until that gate opens. Items tagged **[decision]** need an explicit Ben/Illia call first (listed in PROPOSAL §12.10). Verification commands are given where they are crisp; `cargo test -p ironclaw_architecture` is implied after every structural PR.

---

## WS0 — Prerequisites and baselines

- [ ] De-wildcard the `ironclaw_host_api` prelude: replace the 45 `pub use <mod>::*;` lines (`lib.rs:85-132`) with module-qualified exports; repoint all consumers. Behavior-free; unblocks every contracts split. Verify: `rg -c "pub use .*::\*" crates/ironclaw_host_api/src/lib.rs` → 0 (excluding the documented test-support gate).
- [ ] Add `ironclaw_first_party_extension_ports` to the root `members` array (make the implicit member explicit) so later tooling sees every package.
- [ ] Record baselines for the ratchets that must not regress during the restructure: composition mass, production-struct dead-code, integration coverage floor, `LAYER_MATRIX_EXCEPTIONS` count (=20), extension-specificity allowlist size.
- [ ] Confirm the team decision on Strategy B (family dirs + focused crates) and on which `reborn_` renames from the severable batch are in scope. **[decision]**

## WS1 — Contracts extraction (kills the 7 W4.3 exceptions + `auth→turns`)

- [ ] Complete the turn vocabulary in `host_api::turn`: move `TurnStatus` and the remaining turn DTOs that vocabulary-only consumers need; delete `turns/src/ids.rs` + `scope.rs` re-export shims; repoint the six vocabulary-only consumers (`auth`, `event_streams`, `outbound`, `telegram_extension`, `triggers`, `event_projections`) to `host_api`.
- [ ] Create `contracts/ironclaw_loop_contracts` from `turns::run_profile/**` + the `LoopExit` DTO half + `CheckpointStateStorePort`; repoint `agent_loop`, `loop_host`, `hooks`, `capabilities`, `extension_host`, `host_runtime`; `agent_loop`'s deps become contracts-only with zero exceptions.
- [ ] Create `contracts/ironclaw_extension_contracts`: `ChannelAdapter`/`ToolAdapter`/`Extension`/entrypoint vocabulary, channel manifest-surface descriptors, auth recipe schema, memory manifest surface, `InstallationState`/`LifecyclePublicState`/`AuthAccountState`, channel-identity hooks, `PreferenceTargetCodec` + `ReplyTargetBindingRef`; close the dual import path (forbid cross-crate `pub use` of these traits).
- [ ] Create `contracts/ironclaw_product_contracts`: `ProductSurface`/`BoundProductSurface`/caller + descriptor types, product wire DTOs (incl. `AppEvent` from `common::event`), `package_lifecycle`/`operator_llm` vocabulary, delivery-resolution/admission/operator/lifecycle-service ports, prompt-view DTOs.
- [ ] Consolidate sealed evidence minting: channel/webhook verified-inbound constructors into `extension_contracts` (mintable only by the generic verifier), bearer/session evidence into `host_api`; delete the `host-auth-mint` cargo feature everywhere; port the existing ingress/auth contract tests and add a refute-test that adapters/product cannot mint. **(security-sensitive — PROPOSAL §12.1a)**
- [ ] Evict behavior from `host_api` to product: `render_channel_auth_prompt`, `parse_product_slash_command`, `classify_channel_inbound_text`, `parse_interaction_resolution_text`, the `tracing::error!` call, the `tokio::sync::mpsc` projection type; move `UNGATED_LOOP_RUN_CAPABILITIES` behind the origin-gate ratchet's owner.
- [ ] Narrow `ironclaw_common`: `event.rs`→product_contracts; `llm_costs`/`provider_transcript`/`model_selection`→`llm`; `platform`→consumer; budget constants→`resources`; delete `trust_boundary` (dead); resolve the `AttachmentRef` name collision (rename the channel-facing one).
- [ ] Move failure-summary data tables (`runner::failure_summary`) into `host_api::failure`; sever `product→runner` and `product→loop_host` (prompt constant becomes a product asset).
- [ ] Verify: the 7 `*→turns` W4.3 exceptions + `auth→turns` are deleted from `LAYER_MATRIX_EXCEPTIONS` (count ≤ 12).

## WS2 — Extensions family (kills `extension_host→product`; packages colocated)

- [ ] Flip `extension_host`'s implemented ports to `product_contracts`/`extension_contracts` definitions (delivery resolver, reply-context source, admission, pairing sources, account-status) — **must land before the layer flip** (ordering constraint, PROPOSAL §12.1c).
- [ ] Split `ironclaw_extension_manager` out of `extension_host` (the #6616/#6669 arrival inventory: `product_lifecycle`, `available_extensions`+import, `extension_lifecycle_capabilities`+command+product service, `channel_config` product service, pairing workflow orchestration, `webui_extension_credentials`, admin/operator/skill capability handlers, `SharedCommandSurface`).
- [ ] Relocate extension_host strays: `channel_pairing_serve.rs` Axum routes → `webui`; `skill_learning.rs` seam → composition/skills; `bundled_skills.rs`+`build.rs` → CLI/composition asset step; delete the `RETIRED_SLACK_USER_EXTENSION_ID` live branch and `nearai_mcp` module in favor of package-owned data/migration steps.
- [ ] Kill the cross-crate `include_str!` reach-ins (gmail/github/nearai-mcp manifests): catalog/manifest data flows from package inventory via the binary; verify with the new §11.2.7 scan.
- [ ] Re-layer: `ironclaw_extensions` → substrates; `ironclaw_extension_host` → loops; re-charter `extensions` honestly (registry pure ∣ records stateful).
- [ ] Colocate packages under `crates/extensions/packages/`: move `first_party_extensions` to `extensions/first_party/`; every package (slack, telegram, github, gmail, google-*, web-access, notion-mcp, nearai-mcp, …) gets its own `packages/<ext>/` directory with manifest + assets + excluded `wasm-src` beside it; slack/telegram carry their adapter crates; update `scripts/build-wasm-extensions.sh`, `include_bytes!` paths, CI selectors.
- [ ] Merge `ironclaw_telegram_v2_adapter` into `ironclaw_telegram_extension`; telegram deps become `extension_contracts`-only (slack parity); fix the stale `ProductAdapter` naming (crate descriptions, AGENTS files, `ironclaw_wasm_product_adapters` references).
- [ ] Add a committed-`.wasm` freshness digest check (compare `wasm-src` source hash vs committed artifact).
- [ ] Verify: `rg -n "ironclaw_product" crates/extensions/ironclaw_extension_host/Cargo.toml` → nothing; extension-specificity scanner allowlist shrinks accordingly.

## WS3 — Kernel narrowing (kills the remaining W7 exceptions)

- [ ] Move `host_runtime/first_party_tools/**` (http, shell, time, json, echo, schemas, outbound-delivery, memory tools, trigger management, skill management/url-install, trace_commons, spawn-subagent stub) into `packages/first_party/` via the existing `FirstPartyHandlerRegistrar` pattern; host_runtime keeps only the registrar port.
- [ ] Create `lanes/ironclaw_sandbox` by merging `process_sandbox` (plan contract) + `host_runtime/sandbox_process/**` (Docker/broker/credential-firewall/CA) + the `scripts` Docker backend; delete `ironclaw_scripts` and `ironclaw_process_sandbox`; route all process spawning through the transport seam (fixing scripts' direct `std::process` bypass). No production behavior change (all pieces currently unwired/test-only — re-verify at land time).
- [ ] Split `host_runtime/obligations.rs` internally into its three chartered owners (obligation handling ∣ staged secret/network handoffs ∣ process-obligation store); shrink `services/builder.rs`+`production_wiring` toward composition-facing factories.
- [ ] `mcp` drops the registry dep (consume `extension_contracts`); confirm the estimate/usage vocabulary it needs lives in `host_api::resource`.
- [ ] Re-layer `processes` → kernel. Internal `capabilities/host.rs` split along its six workflows (module charter only).
- [ ] Fix `network`'s production use of `test_rewrite.rs` (`default_policy_http_egress` behind `test-support`; composition constructs the real transport).
- [ ] Tighten direct `secrets` consumers: remove the `webui` and `operator` edges via `product_contracts` ports; keep `auth` by charter; add the boundary rule. **(security-sensitive — PROPOSAL §12.1b; port replacements land first)**
- [ ] Verify: all `host_runtime→*`, `capabilities→extensions`, `mcp/scripts→*`, `processes→resources` W7 exceptions deleted; `rg "bollard|rcgen" crates/kernel/ironclaw_host_runtime/Cargo.toml` → nothing.

## WS4 — Loop tier

- [ ] Re-layer `runner` → loops and `hooks` → loops (clears `runner→agent_loop`, `runner→loop_host`, `hooks→wasm_limiter` exceptions).
- [ ] Runner sheds (non-gated part): `runtime.rs` `build_*` composition functions → composition; model gateway + port adapters → `loop_host`; tool-disclosure policy → loop_host/product per PROPOSAL §6.7.3; delete `production_readiness` (no production caller) or wire it.
- [ ] `loop_host` re-charter: absorb runner's model-gateway/port adapters; shed the `TurnRunTransitionPort` decorator; split `capability_port.rs` (11k lines) along its five roles; declare the sanctioned `Loop*Port` decorator chain in the family AGENTS.md.
- [ ] `agent_loop`: swap `turns` dep for `loop_contracts`; confirm contracts-only rule passes with zero exceptions.
- [ ] `hooks`: ADR-or-converge decision on its libSQL/Postgres predicate backends. **[decision]**
- [ ] `wit/` moves to `crates/lanes/wit/`; wasm bindgen path updated; §11.2.7 scan passes.

## WS5 — Product family

- [ ] `webui`: dep flips to `product_contracts`; the bearer-evidence mint import moves to `host_api`'s sealed home; gains pairing routes; verify its boundary rule updates.
- [ ] `operator`: implements `product_contracts` ports (ownership un-inverts; product dep dropped); route fragments move behind `host_ingress` carriers wired by composition; gains AGENTS/CLAUDE + a boundary rule (has neither today).
- [ ] `openai_compat`: rename from `reborn_openai_compat` **[decision — severable]**; dep flips to contracts; stale `storage`/`libsql`/`postgres` feature guidance corrected in all five audited places; collapse the LibSql/Postgres ref-store newtype wrappers onto the generic fabric form (same for product's ledger wrappers).
- [ ] `product` narrows: ports/DTOs out (WS1); `adapter_registry` manifest parsing → `extension_contracts`/`extensions` (resolving its guidance-vs-code contradiction); the ~120-symbol `host_api::product_adapter` re-export facade dissolved; slack/telegram token heuristics → packages; `external_tool_catalog` moves in from `turns`; `reborn_services` module-charter map committed (freeze ratchet stays).
- [ ] `conversations`/`threads` naming trap fixed: rename conversations' `SessionThreadService` (→ `InboundConversationService`) + its same-named DTO trio; unify `ExternalActorRef`/`ExternalConversationRef` with `host_api` (delete product's field-by-field translators).
- [ ] `attachments` widened: ports + composition impls move in; one home for size ceilings (webui/openai import them).

## WS6 — Composition, app, and domain evictions

- [ ] Composition behavior evictions (each its own PR, aligned with the #6691 direction): approval/authorization/trigger-fire policy → `approvals`/`authorization`/`runtime_policy`+`triggers`; automations panel + admin-user services → `product`; trace capture → `traces`+runner observer seam; system-prompt content → owning prompt asset; OpenAI-compat + NEAR-login route mounts → `openai_compat`/`operator` factories; project gating → `projects`; blocked-auth resume fan-out → `product`/`auth`; Google OAuth secret store → package/auth recipes.
- [ ] Retire the `local_dev` misnomer (production path renamed; deployment-mode naming ratchets extended to catch it).
- [ ] `RebornRuntime` slimmed: ~40 `_for_test` accessors behind `test-support`; re-export wall reduced to the documented snapshot (every survivor names consumer + enforcing test); delete the dead `product_live_adapters` export block (integration harness repointed).
- [ ] `ChannelExtensionBinding.extension_id` becomes typed `ExtensionId`; env reads consolidate behind `ironclaw_config`.
- [ ] `config` narrows: vendor sections (`SlackSection`/`TelegramSection`/`GoogleSection`, Google update pipeline, `update_slack_enabled`) and `capability_remediation.rs` move to package-owned admin-config/data; compatibility window: old sections parse into migration guidance for one release. **(compat constraint — PROPOSAL §12.2)**
- [ ] CLI sheds Google-OAuth resolution + `reject_legacy_slack_config` to package-owned steps behind generic seams; dir rename `ironclaw_reborn_cli`→`app/ironclaw_cli` (package name `ironclaw` unchanged) **[decision — severable]**.
- [ ] Renames executed (whichever are in scope): `composition`, `config`, `event_store`, `identity`, `traces`, `openai_compat` — no compatibility re-export shims; all consumers + docs repointed in the same PR. **[decision — severable]**
- [ ] Domain-internal cleanups: `traces` `contribution.rs` split + `ScopedFilesystem` + re-export modules dropped; `llm` `providers.json` becomes a crate asset/composition input + boundary rule added; `skills` stale v1 lib.rs doc rewritten; `triggers` SQL ADR-or-converge **[decision]**; `identity` absorbs `host_api::user_identity` ports + resolves the dual binding-store ambiguity **[decision]**; `projects` absorbs its composition service adapter.

## WS7 — Physical family moves

- [ ] Family directories created; every crate `git mv`'d to its §5 path **with** its narrowing milestone (retain-as-is crates may move in early batches); root `members` uses family paths; CI selectors/scripts/`Cargo.toml` path deps updated per batch.
- [ ] `tools/` unchanged (`stress` stays a member; optional `default-members` trim **[decision]**); root `fuzz/` deleted or re-pointed (currently unresolvable); `ironclaw_safety/fuzz` untouched.
- [ ] Verify after the last move: tree matches PROPOSAL §5 exactly (a script comparing `cargo metadata` paths to the documented tree).

## WS8 — Deletions and dead surface (each lands with the "removing a redundant layer un-masks behavior" discipline: full unfiltered suites; surfaced failures are candidate behaviors, not test edits)

- [ ] Crate: `ironclaw_dispatcher` (repoint 1 dev-dep + 3 test files).
- [ ] Crate: `ironclaw_embeddings` (remove unused root dev-dep; note revival path = memory-native port with a real consumer).
- [ ] Crate: `ironclaw_first_party_extension_ports` dissolved **after** WS3 removes the `host_runtime→first_party_extensions` edge (activation machinery → loop_host/skills; observer vocab → skills; bundle asset reader → package).
- [ ] Modules: `llm::reasoning`; `skills::{registry,catalog,v2,gating}` (or revive with named consumer **[decision]**); `auth::loopback_oauth` (+`urlencoding` dep); `auth::fakes` gated `test-support`; `event_projections::{EventStreamManager, PendingGateProjection, DurableMemoryAuditSink}`; `events::{parse_jsonl, replay_jsonl}`; `outbound::RouteCurrentRunFinalReply`; `approvals::ToolPermissionOverrideStorePort`; `memory_native::EmbeddingProvider` + its six path-shim modules; `common::trust_boundary`; `filesystem::HsmBackend` (or feature-gate); `secrets::placeholder` (until the egress proxy is built); `host_runtime` sandbox CA moves-or-deletes with WS3; `trust::{SignedRegistry, DevTrustOverride, BundledRegistry}` **[decision — commit or delete]**; `runner::production_readiness` (see WS4); composition `product_live_adapters` exports (see WS6).
- [ ] Unused dep edges: `composition→event_streams`, `memory_native→prompt_envelope`.
- [ ] `identity::{lookup, bind, adopt_migrated_identity}` — wire or trim per issue #5618. **[decision]**
- [ ] After the sweep: flip `unreachable_pub`+`dead_code` lints on for all `crates/**` (workspace lints already exist; the noise they'd flag is what this workstream deletes).

## WS9 — Process-journal-gated items **[#6696]**

- [ ] Gate: #6696 (or an equivalent approved design) lands on `main` with its import/rollback contract satisfied.
- [ ] `processes` widens to journal + `ProcessSupervisor`; runner's scheduler becomes the `ProcessKind::AgentTurn` executor registration; runner's `subagent/` await-edge machinery deleted; checkpoint payload + subagent-goal stores collapse per the slices.
- [ ] `approvals` absorbs approval-request + gate records; `capabilities` consumes the process-invocation port.
- [ ] `ironclaw_run_state` deleted; its freeze charter retired; turn stores become projections/adapters over `processes`.
- [ ] Verify: one lifecycle authority (no `RunRecord`/`ProcessRecord`/`TurnRunState` triplication); the §7 T4 walkthrough matches code.

## WS10 — Enforcement additions (each lands with or before the change it protects; all in `ironclaw_architecture` unless noted)

- [ ] §11.2.1 family⇄layer consistency test + no-stray-toplevel + explicit-members check.
- [ ] §11.2.2 exception ratchet (empty list; new entries require `removes_in` + owning issue).
- [ ] §11.2.3 contracts-purity allowlists (3 new crates + host_api/common/prompt_envelope; external framework denies).
- [ ] §11.2.4 port-location scan (adapter/surface/loop-port traits pinned to their owner; no cross-crate `pub use` of them).
- [ ] §11.2.5 sealed-evidence rule (mint visibility + feature-gone pin).
- [ ] §11.2.6 persistence-idiom rule (DB driver deps allowlist = {filesystem, event_store} + shrink-only {triggers, hooks}).
- [ ] §11.2.7 cross-crate `include_str!`/`include_bytes!` scan.
- [ ] §11.2.8 vendor-scope allowlist shrunk to the §8.1 rule-4 set (shrink-only mechanism exists).
- [ ] §11.2.10 channel-adapter conformance suite in `extension_contracts` + package-shape check; wasm digest check (WS2).
- [ ] §11.3 removal of the vestigial `legacy` layer variant; boundary-rule updates for every renamed/new crate (new-crate-adds-rule discipline); `skills` gets `publish = false`.
- [ ] Ratchet updates: composition-mass and struct ratchet baselines re-captured post-narrowing; coverage floor recaptured after test moves.

## WS11 — Guidance and agent-facing (the layer that keeps agents un-confused)

- [ ] Ten family-root `AGENTS.md` files written from the §6 family contracts (what belongs, what doesn't, allowed layer range + direction, named ports, the before-adding-a-crate gate) — content source: [families/](families/).
- [ ] Crate guides added where the audit found none: `extension_host`, `extension_manager` (new), `operator`, `attachments`, `traces`, `memory_mem0`, `prompt_envelope`, `sandbox` (new), plus `triggers` CLAUDE and `hooks` AGENTS; contracts crates get CLAUDE files with their admission tests.
- [ ] Stale guidance corrected (the audited drift list): root `CLAUDE.md` (v1 enclave, `ironclaw_skill_learning`, engine prompts path, `NetworkPolicyDecider`); `crates/AGENTS.md` → thin family index (drops the drifting 200-line table, the six nonexistent crates, the wrong Slack-host claim, duplicate telegram rows); `crates/Architecture.md` rewritten to the family model (drops `build_reborn_services`, v1 section; updates runner/lease + composition entry points); composition `CLAUDE.md`/`AGENTS.md` (drops `src/webui/`, `src/projection.rs`, `llm_admin::llm_catalog` claims); `host_runtime` AGENTS (planner ownership); `wasm` AGENTS (impl claims); `resources` AGENTS (governor inventory); `run_state` AGENTS (nonexistent in-memory stores; gate-record omission); `llm` CLAUDE (v1 paths, `testing` feature); `embeddings`/`skills` stale docs go with their deletions; `event_store`/`openai_compat`/`product` stale feature-gating text; slack/telegram AGENTS file lists; `reborn_identity` CONTRACT (binding-store resolution); `webui` AGENTS/CLAUDE contradiction + phantom features + duplicate line; `build.rs` doc drift.
- [ ] `.claude/skills` repointed: `ironclaw-reborn-orientation` (v1-enclave heuristic gone, new family map, prompt exemplar), `ironclaw-reborn-architecture-review` (+worked-examples referencing live seams), `reborn-extension-surfaces` (stale `ProductAdapter` paths), `reborn-feature`, `.claude/commands/{trace,triage-prs,deslop-reborn}`; `.claude/rules/type-placement.md` measured numbers refreshed (its fan-in figures inverted).
- [ ] `docs/reborn/` index + contracts updated where ownership moved (host-api contract's module list; extensions contract's dead `product_adapter_registry` reference; kernel-boundary's host_runtime naming note); `FEATURE_PARITY.md`/`CHANGELOG` entries; `openwiki` regenerates (verify the workflow picks up the new tree).
- [ ] PR template / testing playbook references still resolve after moves; `scripts/` (codebase-graph, build-wasm, check-* helpers) updated for family paths.
- [ ] This folder updated to match reality as waves land (PROPOSAL stays frozen as the decision record; README/families gain "landed" markers; CHECKLIST boxes get ticked in the PRs that land them).

## WS12 — Final verification (the 100% gate)

- [ ] `LAYER_MATRIX_EXCEPTIONS` is the empty list; the exception ratchet is active.
- [ ] `cargo metadata` package set == PROPOSAL §5 tree (64 workspace members steady-state; script-verified).
- [ ] Every §9 mapping row cross-checked as landed (74-row audit — a one-off script or manual table tick-through).
- [ ] Full gauntlet green: fmt, workspace clippy `-D warnings` (both feature lanes), workspace tests, architecture suite, integration lanes, recorded-fixture QA, frontend suites, e2e smoke.
- [ ] Backend parity suites green on libSQL + PostgreSQL for every fabric-routed domain (+ triggers/hooks per their ADR outcome).
- [ ] Extension journeys re-verified end-to-end: slack + telegram inbound→turn→delivery, gsuite tool call with credential injection, pairing, lifecycle install/config/activate/remove (fail-closed paths included).
- [ ] Security spot-audit of the three §12.1 changes (mint consolidation, secrets tightening, host/verifier colocation) signed off by a second reviewer.
- [ ] A fresh agent, given only the repo, correctly places three probe features (a new channel, a new product command, a new projection) using family AGENTS.md files alone — the "agents don't get giga confused" acceptance test.

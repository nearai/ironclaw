# Reborn Integration-Test — Shared-Persistence Group Architecture (Design)

**Date:** 2026-06-29
**Status:** REVISED after review round 1 (thermo-nuclear + approach + local-patterns + maintainability). All findings resolved below — see "Review-loop resolutions (round 1)". Re-review pending to confirm zero items before implementation.
**Builds on:** the landed #5392 in-process integration framework and the C1 approval primitives already committed on `reborn-itest-coverage`.

## Why (the new requirement)

Tests must exercise the **full e2e persistence experience**: within one *group*,
state written by one thread is visible to a later thread. Concretely:

> Thread A sets a tool to "approve always" → Thread B (later, same group, same
> tool) gets **no** approval prompt and continues. Same for auth (revoked /
> reauthorized credential) and every other persisted component (memory, projects,
> secrets, extensions, skills, conversation metadata).

Requirements:
1. Threads in one group **share the same filesystem/db + the same stores**.
2. Separate groups run **in parallel, fully isolated**.
3. After a group's tests run, **all persisted objects are deleted** (cleanup).
4. A test failing does **not** abort the others (unless they depend on it).

## Group model — Option A (chosen)

A **group = one subdirectory test binary** (`tests/reborn_group_<name>/main.rs`).
Cargo compiles `tests/foo/main.rs` as a test binary named `foo`; sibling files in
that dir are its modules. Separate subdirs = separate binaries → cargo runs them
**in parallel and isolated** (req 2). The binary runs **one** orchestrating
`#[tokio::test]` that constructs a `RebornIntegrationGroup` and drives scenario
modules **in sequence** over the shared group (req 1, deterministic ordering).
The group owns a `TempDir`; `Drop` deletes it (req 3). Failure isolation (req 4)
is explicit in the driver: independent scenarios run through a collector that
records + continues; dependent scenarios short-circuit with `?`.

Cargo does **not** guarantee order or sequential execution of multiple `#[test]`
fns in one binary, nor share an instance between them — so multiple `#[test]` fns
+ `serial_test` is rejected (non-deterministic order, global teardown, poisoned
statics). One orchestrating fn is the only design that satisfies all four reqs
without fragile machinery.

```
tests/reborn_group_approvals/
    main.rs                         // builds group, runs scenarios in order
    scenario_gate_then_resolve.rs   // pub async fn run(g:&RebornIntegrationGroup)->ScenarioResult
    scenario_approve_always_persists.rs
```
```rust
// main.rs
#[path = "../support/reborn/mod.rs"] mod reborn_support;
mod support;
mod scenario_gate_then_resolve;
mod scenario_approve_always_persists;

#[tokio::test]
async fn approvals_group_e2e() {
    let g = RebornIntegrationGroup::live_approvals().await.expect("group builds");
    let mut report = ScenarioReport::new();
    // dependent chain: approve-always must persist before the no-gate check reads it
    scenario_approve_always_persists::run(&g).await.expect("approve-always persists");
    // independent scenario: failure recorded, others continue
    report.record("gate_then_resolve", scenario_gate_then_resolve::run(&g).await);
    report.assert_all_passed();
}   // g drops -> TempDir deleted
```

## Core architecture: `RebornIntegrationGroup`

The single change that makes cross-thread persistence real: the group owns the
durable storage + stores **once**, and each `.thread(conv)` builds a per-thread
turn runtime over those shared pieces.

```rust
pub struct RebornIntegrationGroup {
    shared: Arc<GroupSharedStorage>,
}

struct GroupSharedStorage {
    storage: StorageMode,
    composite: Arc<CompositeRootFilesystem>,   // threads + turns + db-backed roots
    libsql_db_path: Option<PathBuf>,
    turn_root: Arc<TempDir>,                    // durable root; Drop deletes
    product_harness: RebornProductWorkflowHarness, // binding service + idempotency ledger (group-wide)
    capability: SharedCapabilityBackend,        // see below
    scope: ResourceScope,                       // (tenant,user)-keyed run scope
}
```

### Shared capability backend — the linchpin

The auto-approve toggle is keyed by `(tenant_id, user_id)` only
(`auto_approve.rs::AutoApproveSettingKey`). For "approve-always in A → no gate in
B", thread B must read the **same** `AutoApproveSettingStore` A wrote. That store
(plus the approval-request store, capability-lease store, credential store, and
the memory/projects/secrets filesystem) all live inside ONE
`HostRuntimeCapabilityHarness`. So the group holds `Arc<HostRuntimeCapabilityHarness>`
and every thread clones the Arc:

```rust
// per-thread:
let mode = HarnessCapabilityMode::HostRuntime(Arc::clone(&shared.capability_arc));
let parts = mode.into_parts(milestone_sink)?;  // into_parts already drives from &Arc
```

`HarnessCapabilityMode::into_parts` (harness.rs:1509) already consumes a
`HostRuntime(Arc<…>)` and its `capability_factory`/`capability_result_writer` take
`&self` (cloning inner Arcs). So N threads share one underlying harness → shared
approval/auto-approve/credential/memory/projects/secrets state + a group-wide
invocation log.

Echo backend (no stores) needs no sharing; groups that need it select a
`HostRuntime` backend. The group is parameterized by backend, mirroring the
builder:
- `RebornIntegrationGroup::live_approvals()` → file tools + approval stores (auto-approve disabled for the group scope once at build).
- `RebornIntegrationGroup::builtin_tools()` → memory/projects/secrets/http tools.
- `RebornIntegrationGroup::oauth_auth()` → C2 OAuth product-auth bundle.

### Per-thread build over shared storage

`build()` is refactored so its storage + capability + product harness can be
**supplied** (from a group) instead of constructed. Single-shot
`RebornIntegrationHarness::test_default()` becomes a degenerate one-thread group
(own storage) — the existing public API and all current tests keep working
unchanged.

```rust
impl RebornIntegrationGroup {
    pub fn thread(&self, conversation_id: impl Into<String>) -> RebornThreadBuilder<'_>;
}
// RebornThreadBuilder.script([...]).build().await -> RebornIntegrationHarness
//   (binds a fresh conversation/thread, builds its own scheduler/coordinator
//    over the SHARED composite + SHARED capability Arc + SHARED product harness)
```

Per-thread isolation that must NOT be shared: the scripted model gateway (each
thread scripts its own replies), the conversation_id/binding/thread_id/turn_scope,
the scheduler+coordinator (a fresh turn runtime per thread; the previous thread's
harness drops before the next builds, so only one scheduler polls a given thread
scope at a time — active-run exclusivity is keyed by thread scope anyway).

### Refactor shape (avoid sprawl — arch rules)

Extract the per-thread runtime assembly currently inline in `build()` into one
function `assemble_thread_runtime(shared: &GroupSharedStorage, conversation_id,
replies) -> RebornIntegrationHarness`. Both `RebornThreadBuilder::build` and the
single-shot `RebornIntegrationHarnessBuilder::build` call it. No `#[allow(
clippy::too_many_arguments)]` — pass the `GroupSharedStorage` bundle, not a
loose arg bag (arch rule #1). No `Option<Arc<…>>` "required in prod" fields
(arch rule #2): single-shot constructs its own `GroupSharedStorage` of one
thread, so the storage is always present, never optional.

## Per-subsystem persistence + read-back (from Wave-1 investigation)

> Filled from the 6 investigation agents. Each entry: write capability id + args,
> where it persists (must be inside the shared capability harness or shared
> composite so it crosses threads), read-back accessor, LibSql durability.

- **Approvals/auto-approve:** (CONFIRMED) `AutoApproveSettingStore` keyed by
  `(tenant,user)`; `builtin.write_file` gated; approve/deny + enable_auto_approve
  already implemented. Cross-thread: enable in A (persisted) → no gate in B.
- **Memory + profile:** caps `builtin.memory_{write,search,read,tree}` +
  `builtin.profile_set` (`memory.rs:28`, already in `core_builtin_tools`). Writes
  land in the **host runtime's own `/memory` `InMemoryBackend`** — a SEPARATE
  instance from the integration composite, and **always InMemory regardless of
  StorageMode** (`harness.rs:3112`). **This is the key validation of the group
  design:** threads sharing ONE `Arc<HostRuntimeCapabilityHarness>` share that
  `/memory` backend, so thread-A `memory_write` → thread-B `memory_read`/`search`
  sees it (the user's headline requirement) — read-back = a second scripted
  `memory_read`/`memory_search` turn asserted via `assert_tool_result_contains`.
  `memory_write` args `{target?, content, append?}`; `memory_read` `{path}`;
  `memory_search` `{query, limit?}`. PROFILE: `profile_set {timezone?,locale?,
  location?}` persists `context/profile.json` in same `/memory`; harness wires
  `EmptyUserProfileSource` so it's NOT read at next turn start — assert via a
  direct `MemoryBackedUserProfileSource::resolve_user_profile` on the shared
  backend OR a `memory_read` of `context/profile.json`. **Reopen-durability NOT
  available** for memory (host-runtime `/memory` is never libsql) — the user's
  requirement is cross-thread-LIVE sharing (shared Arc), which works; durability-
  across-restart is a secondary plan goal, deferred for memory.
- **Projects:** `builtin.project_create` (`PROJECT_CREATE_CAPABILITY_ID`,
  `crates/ironclaw_reborn_composition/src/runtime/local_dev/project_create.rs:20`)
  is a **synthetic** capability — NOT in `core_builtin_tools`; surfaced by
  `RefreshingCapabilityPort::build_inner` (`refreshing_capability_port.rs:181`)
  when a `project_service` is wired. Args `{name (req), description?}`. Persists to
  `/tenant-shared/reborn-projects/<tenant>/records/<id>.json` routed to the
  `/tenants` composite mount (`mount_local_dev_database_roots`, `factory.rs:2359`)
  → **LibSql on disk when libsql feature on (durable across reopen), else
  InMemory**. NOT approval-gated (synthetic caps bypass approval). Read-back: reuse
  the same `Arc<dyn ProjectService>` (`RebornProjectService`,
  `project_service.rs:69`) `.list_projects(...)`, or `FilesystemProjectRepository`
  directly. **ARCH NOTE:** the lightweight `core_builtin_tools` host runtime fs
  (only `/projects`+`/memory` mounts) is NOT the integration `/tenants` composite —
  so projects/memory/secrets read-back over a shared, durable, reopenable store
  pushes the group toward the **full `build_reborn_services` local-dev composition**
  as the shared capability backend (consistent fs + synthetic caps + approval +
  product-auth all on one composite), rather than the lightweight host runtime.
- **Secrets (+ LibSql wiring):** **NO first-party tool** — `SecretStore::put` is a
  trusted setup primitive; the model can't write secrets through a turn. Test writes
  **directly**: `FilesystemSecretStore::new(wrap_scoped(composite), SecretsCrypto)`
  (`filesystem_store.rs:211`; `wrap_scoped` pub at composition `lib.rs:844`,
  libsql-gated) → `put(scope, SecretHandle, SecretMaterial, None)` → reopen fresh
  libsql db over same file → `lease_once`+`consume` returns material (same
  master-key crypto). `/secrets` resolves under the already-mounted `/tenants`
  (`invocation_mount_view`). **The "one-liner under LibSql"** = a `test-support`
  accessor `build_local_dev_secret_store_for_test` delegating to the private
  `factory.rs:2419 build_local_dev_secret_store` (promote to `pub(crate)`), since
  the existing harness wires no-op `StaticSecretStore`. **Secrets group = pure
  store-durability/cross-instance test, NO turn / capability backend** — simplest
  slice; it does not need the group's per-thread runtime at all, only the shared
  composite + a write-then-reopen-read over it.
- **Extensions:** _TBD from agent a8e18f_
- **Skills (+ skill_context_source wiring):** caps `builtin.skill_{list,install,remove}`
  (`skill_management.rs:25`); install args `{name, content}` (inline SKILL.md).
  Persists `/projects/skills/<name>/SKILL.md` — composite needs a writable
  `/projects/skills` mount (NOT currently wired). Cross-turn injection requires
  setting `skill_context_source` (currently `None`) to
  `SkillBundleContextSource::new(FilesystemSkillBundleSource over ScopedFilesystem
  on the composite, [FilesystemSkillBundleRoot::user("/projects/skills")])`
  (`skill_bundle_context_source.rs:34`, `filesystem_skill_bundle_source.rs:59`).
  **Observable = the scripted `TraceLlm`'s received `CompletionRequest`** (skill
  system prompt lands in `system`/first system message) — needs a NEW
  `captured_completion_requests()` accessor (the scripted provider must record
  requests; analogous to `captured_egress_requests()`). Unique angle (no existing
  test): two-turn install→inject through the real `ironclaw_llm` chain. **Hardest
  C4 item** (mount + source wiring + new capture seam). Binary-E2E
  (`reborn_qa_smoke_scenarios_e2e.rs:384`) only asserts install/list/remove tool
  outputs, never cross-turn injection — no overlap.
- **Conversation metadata:** `FilesystemConversationStateStore`
  (`filesystem_store.rs:77`) → `/conversations/state.json`: `BindingRecord`
  (thread_id, source/reply binding refs `source:`/`reply:<uuid>`, route access) +
  `ThreadRecord` (participants) + monotone `revision`. **No** session
  start/end/timestamps — binding/thread-keyed, not session-keyed. Created during
  the turn the harness already drives (`resolve_binding`, `builder.rs:269`).
  Read-back: `RebornFilesystemConversationServices::new(scoped)` over the same
  composite → `lookup_binding(...)` → `ConversationBindingResolution`. Distinct
  assertions vs `backend_matrix` (which is thread+turn history): binding-ref
  mint+stability, idempotent re-lookup returns identical refs, thread_id matches
  across reopen. **Conversation group = lookup/binding-durability**, mostly
  store-level (the binding service is already group-shared via product harness).
- **C2 auth:** `ScriptedHttpResponse` (`http_matcher.rs:32`) + `RecordingRuntimeHttpEgress::execute`
  (`harness.rs:3527`, hardcodes 200) need `status: u16` (default 200) + `with_status`;
  `ScriptedOAuthTokenEgress` (`test_support.rs:292`, gated `test-support` +
  `any(libsql,postgres)`) needs additive `status`/`with_error_response`.
  **CONSTRAINT:** a `builtin.http` 401 is **model-visible only** — it does NOT
  produce `BlockedAuth` (`http.rs:213` returns Ok with `{"status":401}` to the
  model; `enrich_dispatch_error_credential_requirements` only fires when the
  *authorizer obligation* fails). Reaching a reauth gate needs a **credentialed
  capability** (authorizer emits `InjectCredentialAccountOnce`) against a
  `Revoked`/missing account — github-harness-style (`harness.rs:3443`), NOT
  `core_builtin_tools`. So C2 arms by difficulty: (a) **revoke**
  `update_status(Revoked)`→read-back and (b) **invalid_grant** sweep
  (slice-8 bundle + new egress error) are EASY/pure-store; (c) **live 401→reauth
  gate** + (d) **deny** need a credentialed backend (medium) — `deny_gate` already
  handles `BlockedAuth` (coordinator `AnyBlockedGate` + `Denied`); (e) add-credential
  `complete_selected_credential` then no-gate. resume_auth_gate
  (`auth_interaction/service.rs:163`) takes `Option<GateResumeDisposition>` (only
  `Denied`); integration tests use the harness `deny_gate`/coordinator handle, not
  the product API.

- **Extensions:** caps `builtin.extension_{search,install,activate,remove}`
  (`extension_lifecycle_capabilities.rs:22`, args `{extension_id}` / search
  `{query?}`). State persists `/system/extensions/.installations/state.json` on a
  **LocalFilesystem** mount of the host-runtime composite (`factory.rs:1269`).
  Backend = existing `HostRuntimeCapabilityHarness::extension_lifecycle_tools()`
  (auto-approve ON, seeds credentials). Read-back: `FilesystemExtensionInstallationStore`
  is `pub(crate)` → add test-support accessor `extension_installation_store_for_test`
  on `RebornServices` (mirror `local_dev_approval_test_parts`, `factory.rs:491`) →
  `list_installations()`/`get_installation(id)`; reopen via `load_at(fs, path)`.
  Unique angle vs binary-E2E (which asserts tool outputs): state.json persisted +
  reopen survives + activation transition persisted.

## Unified backend insight (drives the whole implementation)

Two filesystems exist and are **separate**:
- **Host-runtime composite** (capability `storage_root`): memory (`/memory`,
  always InMemory), projects (`/tenant-shared/...`→`/tenants`), extensions
  (`/system/extensions`, LocalFilesystem), skills (`/projects/skills`), secrets
  (`/secrets`→`/tenants`), approval/auto-approve stores.
- **Integration thread/turn composite** (`build_storage_composite`): thread
  history + turn state (LibSql/InMemory).

The group shares BOTH: one `Arc<HostRuntimeCapabilityHarness>` (→ all
capability-side stores common across threads = the user's cross-thread
requirement) + one thread/turn composite. The **existing binary-E2E constructors**
(`file_tools_requiring_approval`, `extension_lifecycle_tools`,
`skill_management_tools`, `core_builtin_tools`) are the right shared backends
— they ride the full `build_reborn_services` composition (synthetic caps,
approval, product-auth). Reuse them; do not build new lightweight backends.

What's genuinely NEW (small, additive, test-support-gated):
1. `RebornIntegrationGroup` + `build()` refactor to share storage/capability/product-harness.
2. pub(crate) on the reused constructors so the integration builder can call them.
3. test-support read-back accessors on `RebornServices`: `extension_installation_store_for_test`, `build_local_dev_secret_store_for_test`, (profile/projects via the already-exposed services).
4. `captured_completion_requests()` on the harness (scripted provider records requests) — only for the skills cross-turn-injection assertion.
5. `skill_context_source` wiring + `/projects/skills` mount (skills group only).
6. C2 status fields on the two scripted egresses.

## Realistic scope + sequencing (by tractability — all reuse the ONE group mechanism)

- **Tier 1 (cross-thread via shared Arc, existing constructors):** approvals
  (approve-always→no-gate), memory (write A→read B + profile), extensions
  (install A→present B + reopen). Strongest demonstration of the user requirement.
- **Tier 2 (store-level, no per-thread turn needed):** secrets (direct
  write→reopen→read), conversation (binding lookup stability/idempotency).
- **Tier 3 (medium new wiring):** auth — revoke + invalid_grant (store/sweep,
  easy) then live 401→reauth-gate + deny (needs a credentialed capability backend
  with a Revoked account, github-harness style).
- **Tier 4 (hardest):** projects (synthetic `project_create` needs `project_service`
  wired into the capability port), skills (`/projects/skills` mount +
  `skill_context_source` + `captured_completion_requests`).

Each group is its own subdir binary; tiers sequence the IMPLEMENTATION order, not
the runtime (all groups run parallel). If a tier proves out-of-budget it ships as
a clearly-scoped follow-up PR — but the group mechanism + Tier 1 prove the
end-to-end cross-thread-persistence contract the user asked for.

## C2 status-field changes (two surfaces — frame accurately)
- `ScriptedHttpResponse` (http_matcher.rs): add `status: u16` (default 200) — pure test-tree change.
- `ScriptedOAuthTokenEgress` (composition `test_support.rs`, behind `test-support`): additive `status`/error path — a **gated crate-API change**, backward compatible, NOT "test-only".

## Group test binaries (req: each its own subdir/binary)
`reborn_group_{approvals,auth,memory,projects,secrets,extensions,skills,conversation}`.
Each: a sequential driver + scenario modules. Each scenario asserting a *reaction*
gets a negative/control guard (mutation-testing lesson). Cross-thread persistence
is the headline scenario in each group.

## Verification discipline (unchanged from plan)
Test-first; run exact target with non-zero-count guard; `cargo fmt --check`; full
all-features clippy `-D warnings`; no-panics; mutation-test each reaction
assertion (break code → RED). Update `tests/support/reborn/CLAUDE.md` and delete
planning docs in the final cleanup.

## Review-loop resolutions (round 1) — THE IMPLEMENTATION CONTRACT

All four reviewers approved the core architecture (one-sequential-test-per-subdir
group, shared `Arc<HostRuntimeCapabilityHarness>`, `assemble_thread_runtime`
convergence avoiding `Option<Arc>`/`too_many_arguments`, TempDir-Drop cleanup,
tier sequencing). The following findings are ACCEPTED and binding:

**R1 (thermo BLOCKER1, local-patterns NIT): new file `tests/support/reborn/group.rs`.**
`RebornIntegrationGroup`, `GroupSharedStorage`, `RebornThreadBuilder<'_>`,
`ScenarioReport`, and `assemble_thread_runtime` live in `group.rs` (NOT builder.rs
— it is already ~969 lines; +340 would cross the 1500 arch threshold).
`builder.rs` keeps only single-shot `RebornIntegrationHarnessBuilder` and calls
`super::group::assemble_thread_runtime`. Add `pub(crate) mod group;` to `mod.rs`
+ a Files entry in CLAUDE.md + module-level `#![allow(dead_code)]`.

**R2 (thermo BLOCKER2, approach MED, maintainability MED — unanimous): per-thread
invocation baseline.** Shared `Arc<HostRuntimeCapabilityHarness>` means one
`invocations` vec across threads → `assert_tool_invoked` would pass on a *prior*
thread's entry and silently defeat mutation guards. FIX = option (a): each
`RebornIntegrationHarness` records `baseline_invocation_count` (=
`capability_recorder.invocations().len()`) at build; `assert_tool_invoked`,
`captured_egress_requests`, `captured_capability_results`, and
`captured_completion_requests` all read only the `[baseline..]` delta slice. One
field + one-line-per-assertion; zero production touch. (Chosen over a per-thread
fresh recorder — simpler, no restructuring of the capture path.) Single-shot
harness baseline = 0, behavior unchanged.

**R3 (thermo MAJOR3, local-patterns MED — unanimous): subdir module paths.** Group
`main.rs` MUST use BOTH `#[path]` forms, each with `#[allow(dead_code)]`:
```rust
#[allow(dead_code)] #[path = "../support/reborn/mod.rs"] mod reborn_support;
#[allow(dead_code)] #[path = "../support/mod.rs"] mod support;
```
Bare `mod support;` resolves to `tests/reborn_group_*/support.rs` (nonexistent) →
compile failure for all subdir binaries. Document the subdir pattern in CLAUDE.md.

**R4 (thermo MAJOR4): named two-composite accessors.** Expose on
`RebornIntegrationGroup`: `turn_composite() -> &Arc<CompositeRootFilesystem>`
(thread/turn history read-back) and `capability_harness() -> &Arc<HostRuntimeCapabilityHarness>`
(drives test-support accessors for memory/projects/extensions/secrets). Makes the
dual-composite reality explicit at the API boundary; documented in CLAUDE.md so an
author doesn't read the wrong (empty) composite.

**R5 (thermo MAJOR5, arch rule #3): single-source scope.** Do NOT store a parallel
`scope: ResourceScope` on `GroupSharedStorage`. Expose
`RebornProductWorkflowHarness::scope() -> &ResourceScope` (it already owns the
value it was constructed with) as the single source; group constructors call
`disable_auto_approve_for(self.product_harness.scope().clone())`. Also retire the
C1 `run_resource_scope` duplicate field on the single-shot harness in favor of the
same accessor.

**R6 (thermo MINOR6): thread-builder lifetime contract.** `RebornThreadBuilder<'g>`
borrows the group only for the builder's own lifetime; `build()` **Arc-clones**
every shared field from `GroupSharedStorage` into a `'static`
`RebornIntegrationHarness` (mirrors `into_parts` moving Arc-clones). The returned
harness does not borrow the group — independent scenarios may hold two harnesses,
and the sequential-drop contract is a usage convention, not a lifetime bound.

**R7 (thermo MINOR7): `ScenarioReport` hard API cap.** Exactly:
```rust
pub struct ScenarioReport(Vec<(String, HarnessResult<()>)>);
impl ScenarioReport {
    pub fn new() -> Self;
    pub fn record(&mut self, name: &str, result: HarnessResult<()>);
    pub fn assert_all_passed(self);   // panics listing every failed scenario
}
```
Nothing else — doc-comment "intentionally minimal; for richer per-scenario data,
enrich the scenario fn's return type." Lives in `group.rs`.

**R8 (approach LOW): secrets + conversation are FLAT tests, not group binaries.**
Neither exercises cross-thread sharing (secrets = direct write→reopen→read;
conversation = binding lookup durability/idempotency). Implement as flat
`tests/reborn_integration_secrets.rs` and `tests/reborn_integration_conversation.rs`
matching `reborn_integration_backend_matrix.rs`. Group subdir binaries are reserved
for genuine cross-thread sharing (approvals, memory, extensions; auth/projects/skills
in later tiers). Revised binary set: 6 group subdirs
(`reborn_group_{approvals,memory,extensions,auth,projects,skills}`) + 2 flat files.

**R9 (maintainability MED): CLAUDE.md update lands WITH the Tier-1 group PR**, not
deferred to final cleanup. Add an interim "Group tests" section (group = subdir
binary + one sequential test + shared stores; why multiple `#[test]` fns are
rejected; the `pub async fn run(g:&RebornIntegrationGroup)->HarnessResult<()>`
scenario shape; `?` vs `report.record()`; the subdir `#[path]` pattern; the
`turn_composite()`/`capability_harness()` accessors). The final cleanup still
rewrites CLAUDE.md slice-agnostic; this is the interim guard.

**R10 (maintainability LOW): shared driver helper.** Provide a minimal
`run_reborn_group!` macro (or a `run_group(setup_fut, body)` fn) in `group.rs` to
remove the `#[tokio::test]` + env + `ScenarioReport` boilerplate from the 6
`main.rs` drivers; scenario ordering stays explicit at the call site.

**R11 (local-patterns NIT): `captured_completion_requests()` is `pub(super)`**,
matching `captured_egress_requests`/`captured_capability_results` (builder.rs:680,726).

## Open design questions — RESOLVED in round 1
1. Group-wide invocation log → **R2** (per-thread `baseline_invocation_count` delta).
2. Per-thread schedulers over shared turn store → **R6** (harness Arc-clones shared
   fields, `'static`; sequential-drop is a usage convention. Active-run exclusivity
   is keyed by thread scope (`ironclaw_turns` CLAUDE.md), and each group thread is a
   distinct conversation/thread_id, so concurrent schedulers do not collide on a
   thread scope even if two harnesses are alive).
3. Single-shot `test_default()` regression risk → **R5/R1**: single-shot builds its
   own one-thread `GroupSharedStorage` and calls the SAME `assemble_thread_runtime`;
   baseline=0; no `Option<Arc>`; existing ~15 `reborn_integration_*.rs` behaviors
   are byte-identical. Verified by running the full existing suite green before/after
   the refactor (gate in the implementation task).

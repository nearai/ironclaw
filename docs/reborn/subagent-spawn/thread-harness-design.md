# Subagent Thread Harness — Delivery & Durability Design

**Status:** Accepted design — supersedes the 2026-06-08 durability spec and PR #5176's draft docs.
**Date:** 2026-07-07
**Parent:** [`README.md`](./README.md) (overarching design)
**Scope:** `crates/ironclaw_reborn`, `crates/ironclaw_loop_support`, `crates/ironclaw_turns`,
`crates/ironclaw_filesystem`, `crates/ironclaw_outbound`, `crates/ironclaw_reborn_composition`.

This is the canonical design for the subagent completion-delivery and durability
layer: how a parent thread is notified when a descendant subagent settles, how
that notification survives crashes and restarts, and how approval/auth gates
from any descendant reach a human. It **supersedes outright** the durability
design in the 2026-06-08 spec (`BoundedSubagentGateResolutionStore` +
SQL ledger/reconciler/idempotency-table approach) and the draft docs opened
under PR #5176 — no code from either survives beyond what §3 below marks
SURVIVES. This design was hardened through multiple rounds of adversarial
technical review against the live crates; every ruling cites the concrete
call sites, line numbers, and existing production patterns it verified
against.

Companion doc: `docs/reborn/subagent-spawn/README.md` is the overarching
subagent-spawn design (spawn mechanism, security model, blocking lifecycle).
This document is the canonical source for its background-mode delivery and
durability layer (README §7 "Background delivery" / §13 follow-ups — deferred
there pending this design, now settled here).

**Standing ruling — governs the whole document:** `spawn_subagent` is
deny-filtered in production today — no live behavior to preserve.
`SubagentSpawnDeps.gate_store` is **required**, not `Option`
(`crates/ironclaw_loop_support/src/subagent_spawn_port.rs:378`). **No
`subagent.v2_enabled` flag anywhere.** `disabled_capability_ids` is the sole
on/off gate.

## 0. Requirements

| # | Requirement | Satisfied by |
|---|---|---|
| 1 | Parent inspects a child's work anytime | `subagent_inspect`, §1, §7 (PR3) |
| 2 | Human opens child thread, keeps conversing | Console `activate(Human)`, §7 (PR5a/5b), §11 |
| 3 | Parent re-activates a finished child, no cold start | `subagent_extend` = `activate(ParentAgent)`, §6, §7 (PR4) |
| 4 | Best-in-class harness (Claude Code/Codex/LangGraph/Temporal/Devin/Assistants) | Whole design; §1 |
| 5 | Any descendant gate (approval/auth) reaches a human deterministically, any depth | §9 Gate propagation |
| 6 | Flavors configurable (prompt/capabilities/budget/model), no loop fork | §10 Per-flavor configurability |

## 1. Inherited core

- Child = normal Turn thread + lineage header (`parent_run_id`, `tree_root_run_id`, `depth`). Inspect/human-attach/re-activate are ordinary thread operations.
- One await-edge file per parent-awaits-child; CAS `open → settled → drained`, `open → abandoned`.
- Terminal is per-run; threads park, never die. `activate(thread, input, provenance)` is the single re-activation primitive; `ThreadBusy` if a run is live.
- Consent-to-wake: an agent wakes only its own direct live child; orphaned descendants are human-wake-only.
- Parent resume via `TurnStatus::BlockedDependentRun` + `gate:subagent-await-<child_run_id>` + `resume_turn` with `ResumeTurnPrecondition::BlockedDependentRunGate` (verified: `crates/ironclaw_turns/src/request.rs:33,48` maps this to `required_status() == Some(BlockedDependentRun)`).
- Child run record is the result source of truth (no `CapabilityResultStore`).
- Safety: child→parent framed-as-untrusted + byte-capped via `wrap_untrusted_subagent_text`; child→human raw; gates bubble up lineage (generalized in §9).
- **No flag.** `disabled_capability_ids` gates the capability.

## 2. Closed edges are deleted

CLAUDE.md: *"LLM data is never deleted... 'Cleanup' means evicting from in-memory caches, never deleting database rows."* Scoped to **model I/O** — transcripts, run records, events. The await-edge is control-plane delivery bookkeeping, not a record of what any LLM said or did. The child's run record, its lifecycle events, and the framed content the parent consumed (written at drain) are the durable facts — the edge is scaffolding for one-time delivery; once delivered it has no further job.

**Ruling: on CAS to `drained`, and on `abandoned` finalization, the edge file is deleted — via the new CAS-guarded `delete_if_version` (§4.0), never a blind delete.** `RootFilesystem::delete(path)` has **no CAS precondition** today (`crates/ironclaw_filesystem/src/root.rs:158` — no `CasExpectation` param, unlike `put`); §4.0 adds the missing primitive, closing the silent blind-delete gap this design corrects.

**Crash-ordering and self-healing.** Edge close is two durable steps: (1) CAS the edge's own state to its terminal value, (2) `delete_if_version(edge_path, Version(post-transition))`. **Order is fixed — state transition first, delete second** (deleting before the state CAS would make a live-looking edge vanish with no record of why). A crash between (1) and (2) leaves a **terminal-but-undeleted edge** on disk — **recovery (boot pass §4.3/§5.3, and the lazy backstop) must enumerate `drained`/`abandoned` edges the same way it enumerates open ones**, and re-run step (2); `NotFound`/`VersionMismatch` on that retry is itself benign (someone else finished it, or it's mid-handling) and reported `already_closed`. **This makes the orphan window self-healing by construction, not merely invisible** — recovery cost stays bounded, now also covering the terminal-but-undeleted window, not just true open/settled state.

**Consequences:** the per-parent live-index approach from earlier drafts is **deleted outright** (deletion gets there for free). No closed-root/archive language anywhere. Settled-but-undrained edges are the pending-delivery queue and are never deleted — only terminal transitions delete. `NotFound` on any edge read is benign — treat it like a completed settle-then-drain.

**Abandon is mode-scoped, not a blanket "parent terminal → abandoned" rule.** §1's inherited open→abandoned transition applies only to:
- **(a) Blocking edges whose resume became impossible** — the parent run went `Cancelled`/`Failed` while parked on the gate/dependent-run wait, and was never resumed. This is the failure case `abandoned` exists to capture.
- **(b) Explicit tree teardown** — an operator/human tears the tree down (`subagent_cancel`, §7 PR6) while the edge is still open.

**For background edges, parent-run completion with the edge open is the *normal* delivery case, not an abandonment trigger** — see §8. Threads park, never die (§1); a live parent thread finishing its run is not "parent gone," it's exactly the state §8's `System`-provenance activate exists to wake. The edge stays `open` (then `settled`) and drains the ordinary way, whether the parent run that opened it is still live or has already gone terminal.

## 3. What this replaces

New code lives in `crates/ironclaw_reborn/src/subagent/await_edge/` (`mod.rs`, `store.rs`, `resolver.rs`, `roster.rs`). The new `AwaitEdgeResolver` owns settle/resume/drain/recovery end-to-end.

| DIES | Why |
|---|---|
| `BoundedSubagentGateResolutionStore` (`.../subagent/gate_resolution.rs`, **1,532 lines**) | Its job — in-memory gate bookkeeping for delivery — is what the CAS'd, durable edge file now does. |
| `DefaultPlannedRuntimeParts.subagent_gate_store` (`crates/ironclaw_reborn/src/runtime.rs:288`, reads `:524,618,657,668`) + construction/threading at `crates/ironclaw_reborn_composition/src/runtime.rs:3002` (`:3134,:3449`) | Replaced 1:1 by `Arc<dyn AwaitEdgeWriter>`, same required-not-`Option` shape (per the standing no-flag ruling above). |
| `completion_observer.rs` gate-store call sites (production cluster before test helpers at ~1077) | Split below by function. |
| `prompt_material.rs`'s gate read (`GateBackedSubagentPromptMaterialSource::material_for_run`, `.gate_store.subagent_kind_for_child(...)`, line 79) | **Dies outright.** Existing fallback (lines 82-90) already covers every case. |
| `BoundedSubagentResultTombstoneStore` (`tombstone_store.rs`, 119 lines) | **Verified already unwired** — dead code before this design. |
| `production_readiness.rs`'s `subagent_result_tombstone_store: RebornComponentReadiness` field (`crates/ironclaw_reborn/src/production_readiness.rs:362`, read `:384,455,761`) + regression test `production_readiness_rejects_non_durable_subagent_tombstone_store` (`tests/production_readiness.rs:152-156`) | Named explicitly — a readiness-graph field for a deleted component is not a partial deletion. |
| `AwaitedChildState`'s release-claim tri-state (`descendant_reservation_released`/`_release_claimed` fields + `claim_descendant_reservation_release`/`mark_descendant_reservation_released`/`release_descendant_reservation_claim`, `gate_resolution.rs:334-379`) | Dies with the store; **moves** to the `AwaitEdge` payload — §5.5. |
| `AwaitedChildSetRecord.{gate_ref, result_ref}`, `TurnRunRecord.gate_ref` | Path identity + `child_run_id` locator replace both (see §1). |

| SURVIVES | Reason |
|---|---|
| Spawn port + `SpawnTreeReservation` + flavors + goal store | Capacity admission, prompt flavor, goal text — no gate/edge coupling. |
| `completion_observer.rs`'s `TurnCommittedEventObserver` impl | **Non-delivery duty kept**: detects child terminal, calls `AwaitEdgeResolver::on_child_terminal(...)`. |
| `release_descendant_reservation` (line 425) | **Non-delivery duty kept**: capacity bookkeeping. Tri-state guard moves per §5.5. |
| `resume_parent`, `write_terminal_result`, `mark_child_deliveries`, `child_terminal_output`, `update_parent_result_reference`, `recover_missing_gate_record`, `reconstruct_record` | **Move into `AwaitEdgeResolver`** — re-homed onto the edge file. |

## 4.0 CAS-guarded delete primitive

PR1 merge blocker: **P1.0b**.

**Gap:** `RootFilesystem::delete` (`root.rs:158`) takes no `CasExpectation`, unlike `put`. Every backend's delete is unconditional: `in_memory.rs:142` (`state.entries.remove(path)`, no version compare), `libsql.rs:951` (`DELETE ... WHERE path = ?1 OR path LIKE ?2`, no version predicate), `postgres.rs:590`/`postgres_delete_with_client` (same shape). §2's edge-close and §4.5's roster-prune both need TOCTOU-safe delete; neither exists today.

**Ruling: add `delete_if_version(&self, path, expected: CasExpectation) -> Result<(), FilesystemError>` as a new, additive `RootFilesystem` method — not a signature change to `delete`.** `RootFilesystem::delete(path)` has ~20+ production call sites across unrelated subsystems (verified: `ironclaw_secrets`, `ironclaw_skills`, `ironclaw_outbound/src/filesystem_store.rs`, `ironclaw_threads/src/filesystem_service.rs`, `ironclaw_product_workflow/src/filesystem_ledger.rs`, `ironclaw_reborn_composition/{slack_setup,extension_lifecycle,bundled_skills,llm_key_store,product_auth_durable/*}.rs`, `ironclaw_resources/src/cas_snapshot.rs`, `ironclaw_reborn/src/subagent/goal_store.rs`, `ironclaw_approvals/src/capability_permission.rs`) — none need CAS, none should have their blind-delete semantics touched. Forcing a `CasExpectation` onto all of them is exactly the blast-radius churn this design avoids (`P1.0b` is scoped to await-edges/roster only).

**Error taxonomy is NOT inherited from `put` — it's a new, small diagnosis branch.** `put`'s `Version` arm never emits `NotFound` for an absent row: `postgres.rs`'s `diagnose_put_failure` (`postgres.rs:1362-1379`) and `libsql.rs`'s `Version(expected)` arm (`libsql.rs:299-306` — not the `Absent`-arm conflict branch at :262-267) both collapse "row absent" and "row present at wrong version" into the same `VersionMismatch{found: None}`; the in-memory `check_cas` helper (`in_memory.rs:487-506`) does the same — its `(CasExpectation::Version(expected), found)` catch-all arm returns `VersionMismatch{expected: Some(expected), found}` regardless of whether `found` is `None` (absent) or `Some` (wrong version). That collapse is correct for `put` (an absent row on write just means "create it"), but wrong for `delete_if_version`, where the caller needs to tell "already gone" (`NotFound`, benign per §2) apart from "gone stale, don't touch it" (`VersionMismatch`). **`delete_if_version` therefore implements its own two-branch diagnosis on the 0-rows/absent case, per backend: read current state first (or follow up a failed conditional delete with a read) — row absent → `NotFound`; row present at a different version → `VersionMismatch`.** This is new logic sized to the one new method, not a reuse of `put`'s diagnosis.

**Shape, per backend:**
- Default trait impl: `Unsupported`, same pattern as `put`'s default.
- `in_memory.rs` (new fn beside :134, the `delete` impl): look up the current version before removing — `None` → `NotFound`; `Some(v)` where `v != expected` → `VersionMismatch{expected, found: Some(v)}`; matching version → remove, no mutation on either error branch. Cannot reuse `check_cas` as-is (its catch-all arm doesn't distinguish absent from wrong-version, per above) — the two-branch check is written directly in the new fn.
- `libsql.rs` (new fn beside :951): `DELETE ... WHERE path = ?1 AND version = ?2` for `Version(v)`; 0 rows → a follow-up `SELECT version` distinguishes `NotFound` (no row) from `VersionMismatch` (row exists at a different version) — a new two-step diagnosis for this method, not the reused `put` shape. `Any` → today's unconditional predicate.
- `postgres.rs` (new fn beside :590): same `AND version = $2` addition, with its own follow-up `SELECT version` on 0-rows to distinguish `NotFound` from `VersionMismatch` — `version` column already exists, read by `put`/`query` (`postgres.rs:352,388-395,483-491`), but the diagnosis branch itself is new.
- `ScopedFilesystem` (new wrapper beside `delete` at `scoped.rs:526`): pure passthrough to `self.root.delete_if_version(...)`.
- `CompositeRootFilesystem` (new fn beside `delete` at `catalog.rs:360`): `self.matching_mount(path)?.backend.delete_if_version(path, expected)` — pure dispatch.
- **Out of scope:** `LocalFilesystem::delete` (`local.rs:354`) — `LocalFilesystem::put` already rejects `CasExpectation::Version` as `Unsupported` (`local.rs:201-206`, no per-path versioning), so it was never viable for CAS'd edges. `HsmBackend` (`hsm.rs:111`) is an in-tree placeholder, not production. `StorageTxn::delete` (`backend.rs:41`) is the separate multi-key-transaction mechanism; edges are single-key CAS-only ("Stores must always work with CAS... as the floor," `root.rs:33`) and never open a `StorageTxn` — unaffected.
- `CasExpectation::Absent` is not meaningful for delete (nothing to compare) — every edge/roster call site uses `Version` or `Any`.

**Consequences:** (a) **Edge close** = CAS state-to-terminal, then `delete_if_version(edge_path, Version(post-transition))` — ordering in §2. (b) **Roster prune** (§4.5) = boot reads the roster marker's current version at the instant it observes the scope's open-edge dir empty, then `delete_if_version(roster_path, Version(that_version))`. A concurrent spawn writing a new edge for that scope bumps the marker's version via its own idempotent-but-version-bumping upsert, so the prune's CAS fails with `VersionMismatch` and aborts — no TOCTOU loss of a roster entry newly needed.

**Residual race, closed by two symmetric compensating checks.** §4.5's roster-before-edge order is never violated by this race — the roster marker for a scope is always written before that scope's first edge. The residual race is purely about **boot's read-then-delete window landing inside the gap between a scope's roster upsert and its first edge write**, for a scope currently in the middle of its very first spawn: (1) spawn upserts the roster marker (version bump) — roster-before-edge holds; (2) boot's recovery pass lists that scope's open-edge dir and finds it still empty (the edge hasn't been written yet); (3) boot reads the roster marker's *current*, already-bumped version at that same moment; (4) spawn now writes its first edge for the scope; (5) boot's `delete_if_version(roster_path, Version(read in step 3))` **succeeds** — nothing bumped the version again between steps 3 and 5, so the version boot read is still current. The result: a real edge now exists for the scope, but its roster marker is gone. Two compensating checks close this, symmetric in time: **(i) spawn self-heal** — after writing the first edge for a scope, spawn re-reads the roster marker; `NotFound` triggers an idempotent re-put (`CasExpectation::Absent`; a `VersionMismatch` on that re-put means someone else already restored it, which is fine). **(ii) boot prune self-check** — immediately after a successful `delete_if_version` of a roster marker, boot re-lists that scope's open-edge dir; if it is now non-empty, boot restores the marker via the same idempotent re-put. The two checks are complementary in time — whichever of them runs later than the actual edge write is the one that observes and heals the stranded state, so every interleaving converges to a live roster entry; a subsequent boot pass sees a non-empty dir and never attempts the delete at all.

**Leaf-key note.** Edge and roster files are leaf keys with no children, so `delete`'s cascade semantics (the `... OR path LIKE ...` subtree sweep visible in `libsql.rs:951` and mirrored in the in-memory/postgres backends) are immaterial here — `delete_if_version` is pinned to single-key semantics only; it never needs subtree-delete behavior.

**Required tests (P1.0b) — crate-tier** (`RootFilesystem`/backend primitive, no production caller to drive through yet): dual-backend parity: (1) correct-version delete succeeds; (2) stale version → `VersionMismatch`, no mutation; (3) missing path → `NotFound`; (4) roster-prune-vs-concurrent-spawn race, split into (4a) and (4b) per the residual-race ruling above — (4a) version-mismatch abort: a concurrent version bump lands before prune's delete call → prune's CAS fails, roster entry survives untouched; (4b) CAS-delete-succeeds-but-edge-lands-anyway: for a scope's very first spawn, the roster upsert lands first (roster-before-edge preserved), then boot's `list_dir` observes the still-empty open-edge dir and reads the already-bumped marker version, then the spawn writes its first edge — prune's `delete_if_version(read version)` succeeds because nothing bumped the version again in between, stranding a roster marker a now-real edge needs; the test asserts convergence to a restored roster entry via either compensating path (integration-tier, since (4b) exercises both spawn and boot in the same race window). **Until P1.0b lands, nothing else in PR1 merges** (sibling gate alongside P1.0, §4.4).

## 4. Blocker resolutions

**4.1 Crate placement — permanent seam.** `AwaitEdgeWriter`/`AwaitEdgeSettler` traits defined in `ironclaw_loop_support` (owns `SubagentSpawnDeps`); `FilesystemAwaitEdgeStore`/`AwaitEdgeResolver` in `ironclaw_reborn` implement both — dependency inversion, category 2 of `.claude/rules/type-placement.md`. `ironclaw_loop_support` cannot depend on `ironclaw_reborn`. Permanent, no `arch-exempt`.

**4.2 Canonical path.** `/turns/subagent-await-edges/agents/{some/<agent>|none}/projects/{some/<project>|none}/<parent_run_id>/<child_run_id>.json` — one constant **alias, scope-relative** (§4.5a: the physical location this resolves to is per-scope-partitioned by the mount, but the mount only carries tenant/user — §4.5a explains why the agent/project axes must additionally be encoded in-path). The optional-axis segments mirror the `{some/<v>|none}` encoding already used by the scope roster key (§4.5) and by the existing `local_trigger_access` precedent (`crates/ironclaw_reborn/src/local_trigger_access/filesystem.rs:403-419`, `optional_axis_path`); the goal store (`crates/ironclaw_reborn/src/subagent/goal_store.rs:213-228`) independently establishes the precedent of inserting `agents/<agent_id>`/`projects/<project_id>` segments in-path for these same two optional `TurnScope` axes (its variant omits the segment entirely when absent rather than encoding `none` — the roster/edge paths here use the `{some|none}` form instead, so every path has a fixed, unambiguous segment count a scope-prefixed `list_dir` can enumerate without branching on presence). Deleted (via `delete_if_version`, §4.0) on drain/abandon-final (§2).

**4.3 Boot-recovery driver: roster- and store-driven.** Enumerate the scope roster (§4.5), then per scope `list_parents_with_unclosed_edges(scope)` — a plain bounded, **scope-isolated** `list_dir` under that scope's **full-axis prefix** (§4.2's `agents/{some/<agent>|none}/projects/{some/<project>|none}` segments, not just the tenant/user-mounted `/turns/subagent-await-edges` root) (§2, §4.5a — this only ever lists that one scope's mounted-**and**-axis-qualified subtree), **plus** the terminal-but-undeleted sweep from §2's crash-ordering ruling. No `TurnStateStore` active-run query.

**Recovery concurrency is bounded, not one-scope-at-a-time nor unbounded fan-out.** Boot processes roster scopes with a small fixed concurrency limit, `BOOT_RECOVERY_MAX_CONCURRENT_SCOPES = 4` — a worker pool of at most 4 concurrent scope-resolves drains the roster, never one task per scope. This bounds backend/connection load during a cold-boot recovery sweep regardless of how many scopes are pending.

**4.4 #5466 owned, not deferred.** `docs/plans/2026-07-04-w6-cas-contention-plan.md` (verified) explicitly does **not** chase root cause: excludes `StorageMode::LibSql` "unconditionally until #5466's libsql diagnosis lands a real fix" (line 85); allow-list captioned "already-*observed* (NOT root-cause-diagnosed)" (line 125). Its own text (line 70) locates the defect in `FilesystemTurnStateStore::apply_with_retry`'s (now `cas_update`, `crates/ironclaw_turns/src/filesystem_store.rs:44,441`) lock-free CAS retry over a `RootFilesystem` — `SIGABRT`/`SQLITE_MISUSE` under concurrent CAS retries against libsql. **Ruling: PR1 includes P1.0, "root-cause + fix #5466 at the RootFilesystem/libsql layer," with the w6 plan's repro (16 children CAS-settling concurrently, one scope, 100 iterations, both backends) as falsifier + acceptance. PR1's merge gate is P1.0 (and P1.0b, §4.0) done** — not "upstream fixes it eventually."

**4.5 Scope roster + scope-key encoding.** Boot/lazy recovery needs which `(tenant, user, agent, project)` scopes have unclosed edges without a global walk (why a global walk isn't available at all: §4.5a). At first edge write for a scope, spawn idempotently writes a roster marker under `ResourceScope::system()` (`crates/ironclaw_host_api/src/resource.rs:28,112`; used this way by `ironclaw_conversations/src/filesystem_store.rs:203,237`, `ironclaw_reborn_composition/src/llm_key_store.rs:116`).

**Encoding reuses an existing precedent** — `crates/ironclaw_reborn/src/local_trigger_access/filesystem.rs:403-419` turns `(agent_id, project_id, user_id)` into a nested path-safe key. Roster reuses it: `/turns/subagent-await-scopes/tenants/<tenant>/users/<user>/agents/{some/<agent>|none}/projects/{some/<project>|none}.json` — `tenant_id` in-path since the roster lives under the system sentinel, which the same resolver rewrites to the constant `__system__`/`__system__` target rather than a tenant-specific one — one predictable global tree, hence tenant must be encoded in the path itself (verified `ironclaw_reborn_composition/src/lib.rs:718-760`). **The project axis is added here to match §4.2's edge path** — both the roster key and the edge path partition on the same four axes (`tenant`, `user`, `agent`, `project`); a roster entry that only encoded `agent` while the edge path also varied by `project` would let two different-project scopes for the same agent collide onto one roster marker.

**Write ordering:** the roster marker is written **before** the first edge file for that scope, never after. A crash between the two leaves a roster **superset** — harmless, since boot finds an empty open-edge dir and the CAS'd prune (§4.0(b)) removes the stale entry next pass. The unsafe ordering (edge first, roster after) would let a crash hide a real open edge from boot recovery entirely — never chosen.

Boot enumerates the roster (`list_dir` on `/turns/subagent-await-scopes/`); lazy per-scope backstop (§5.3) stays as roster-miss safety net. **Dual-backend:** `list_dir` verified on all three backends — `postgres.rs:545`, `libsql.rs:910`, `in_memory.rs:170`.

**Roster cardinality.** One marker per `(tenant, user, agent, project)` scope with at least one unclosed edge — cardinality tracks **active-subagent scopes**, not edge count (a scope with 16 open edges is still one roster entry). Roster `list_dir` enumeration may paginate per the backend's ordinary `list_dir` semantics — no full-listing-in-one-call assumption is baked into this design. Tenant sharding (§12 tripwire 2) remains the escape hatch if a single tenant's roster subtree itself grows pathological; the §4.3 concurrency bound above is committed now, independently of that tripwire.

**4.5a Scope-aware construction — no fixed-view.** The await-edge store and the scope roster are both built on the **same single shared `ScopedFilesystem` handle**, constructed *once* via `crate::wrap_scoped(root)` → `ScopedFilesystem::new(root, invocation_mount_view)` (`crates/ironclaw_reborn_composition/src/lib.rs:848-852`; resolver `invocation_mount_view` at `:753-763`, alias table `PER_USER_ALIASES` — including `/turns` — at `:718-733`) — **never** via `ScopedFilesystem::with_fixed_view`. Every op (`put`/`get`/`list_dir`/`delete_if_version`) takes the caller's live `ResourceScope` as an explicit argument (`scoped.rs:216-233,487,526`); the resolver recomputes the `MountView` for *that* scope on *that* call. This is the same pattern already load-bearing in production for `ironclaw_conversations::filesystem_store` (`.get(&scope, &path)` / `.put`, re-evaluating a fresh scope per call at `filesystem_store.rs:205,238`) and `ironclaw_reborn_composition::llm_key_store` (`store.put(scope(), ...)`, `scope()` re-evaluated per call at `llm_key_store.rs:39,115-116`) — neither constructs a filesystem baked to one scope.

**Anti-pattern, named explicitly:** `owner_turn_state_filesystem` (`crates/ironclaw_reborn_composition/src/factory.rs:1927-1938`, used in production at `:2079,:4314`) builds a **`ScopedFilesystem::with_fixed_view`** baked to a single `owner_scope` at composition time — a single-boot-owner view. That is the exact bug class behind #5720/#5721 (per-user `/turns` state collapses onto one owner on multi-user boxes). **New await-edge/roster code must not copy this constructor** — it must be built the `wrap_scoped`/`invocation_mount_view` way: one process-wide handle, scope threaded per call, never per-scope-baked at construction. (The only other `with_fixed_view` hit under `subagent/` — `factory.rs:5011`'s `subagent_goal_filesystem` — is test-only fixture code inside `mod tests` opening at line 4647; production's goal store at `factory.rs:2232` is correctly built via `wrap_scoped`.)

**Consequence — §4.2's canonical path is scope-relative, not scope-agnostic — and now axis-relative too.** `ResourceScope` (`crates/ironclaw_host_api/src/resource.rs:31`) carries only `tenant_id`/`user_id` — it has no `agent_id`/`project_id` field at all — and `invocation_mount_view` (`ironclaw_reborn_composition/src/lib.rs:718-763`) rewrites `PER_USER_ALIASES` (including `/turns`) using only those two fields. **The mount alone cannot isolate two agents (or two projects) of the same user** — that isolation has to come from the path itself, which is exactly what §4.2's `agents/{some/<agent>|none}/projects/{some/<project>|none}` in-path segments provide. `/turns/subagent-await-edges/agents/{some/<agent>|none}/projects/{some/<project>|none}/<parent_run_id>/<child_run_id>.json` is a `ScopedPath` alias; the physical `VirtualPath` it resolves to is `/tenants/<tenant>/users/<user>/turns/subagent-await-edges/agents/{some/<agent>|none}/projects/{some/<project>|none}/<parent_run_id>/<child_run_id>.json` (`/turns` is tenant/user-rewritten — it is one of the `PER_USER_ALIASES`; the agent/project segments are ordinary path segments beneath the alias, untouched by that rewrite). Two different `TurnScope`s' edges for the *same* `parent_run_id`/`child_run_id` pair therefore land in physically distinct trees on the backend whenever **any** of tenant, user, agent, or project differs — not just tenant/user. That is what makes `list_parents_with_unclosed_edges(scope)` (§4.3) a plain, bounded, **scope-isolated** `list_dir` under `/turns/subagent-await-edges/agents/{...}/projects/{...}` for that scope's own agent/project axis values: it only ever lists that one scope's mounted-**and**-axis-qualified subtree, never a global one and never a sibling agent's or project's edges under the same user. The scope roster (§4.5) exists precisely because that isolation means boot cannot discover *which* scopes have edges without a scope-agnostic index — the roster marker (itself written under the system sentinel scope, so its own listing genuinely *is* global, and now carrying the same agent/project axes per §4.5) is what tells boot which real `(tenant, user, agent, project)` scopes to mount-and-walk one at a time.

**Acceptance tests (fold into P1.6a/P1.6c) — integration-tier** (drive the real composed `ScopedFilesystem`/`invocation_mount_view` stack): (a) **two-users-distinct-paths** — write an await-edge for two different `ResourceScope`s (different `user_id`; same or different `tenant_id`) using the same `parent_run_id`/`child_run_id`, then assert — read back at the filesystem layer, not just via two `ScopedFilesystem` calls that could coincidentally agree — that the two writes landed at two distinct physical paths (`/tenants/<A>/users/<A>/turns/...` vs `/tenants/<B>/users/<B>/turns/...`); (b) **two-agents-same-user-distinct-paths** — same tenant, same user, two different `agent_id` values (or one `Some` and one `None`) on the `TurnScope`, same `parent_run_id`/`child_run_id` — assert the two writes land at two distinct physical paths differing only in the `agents/{...}` segment, and that `list_parents_with_unclosed_edges(scope)` for one agent's scope never enumerates the other agent's edge.

**4.6 Module placement.** `crates/ironclaw_reborn/src/subagent/await_edge/` — not appended to `completion_observer.rs` (**4,685 lines**, already over the 1,500-line budget in `.claude/rules/architecture.md` §5). Only edit to that file beyond §3's extraction: `wrap_untrusted_subagent_text` → `pub(crate)`.

## 5. Core delivery mechanics

The following are carried forward unchanged from the prior durability-design lineage (restated here so this doc is self-contained; §5.5 and §5.6 are new).

**5.1 Capacity: reservation is the cap, listing advisory.** Unchanged. `SpawnTreeReservation` (depth ≤ 1, ≤ 4 spawns/turn, ≤ 16 descendants/tree) is the sole admission mechanism.

**Made explicit: the 16-descendant cap counts non-terminal children — deliberately, including gate-parked ones.** A child sitting on `BlockedApproval`/`BlockedAuth` still holds its reservation slot. The cap bounds **outstanding side-effect obligations** — 16 approval-parked children is 16 pending sensitive actions paging one human — not compute. Compute concurrency is excluded on a separate, already-existing axis: the scheduler deschedules a blocked run (no worker slot held), so a tree full of gate-parked children costs zero live workers, only zero remaining reservation headroom. These are deliberately different budgets; this design does not collapse them.

**Never release-on-park; release only at terminal (§5.5); never re-claim on ordinary resume.** A child resuming past its own gate (approval granted, auth completed) is not a re-admission event — it already holds its slot from spawn, uninterrupted. **Ruling: resumption must never require re-admission.** An approved child that can't resume because the tree filled up while it waited would be a deadlock-by-design (the human who just unblocked it discovers the unblock didn't work) — never chosen.

**Relief valves, when a tree does fill:** `subagent_inspect` (§7 PR3) shows which children are blocked and why; `subagent_cancel` (§7 PR6) reclaims slots by driving a child to terminal early (terminal → release, §5.5); a human resolving the gate queue directly (§9.2) drains blocked children the ordinary way, which also releases their slots. `ironclaw-reborn subagent edges` (§5.4) reports per-scope open-reservation counts alongside edge counts, so an operator sees a tree approaching the cap before it deadlocks.

**`subagent_extend` is the one re-claim path — by design, not by exception.** Unlike ordinary resume, `subagent_extend` (§6, §7 PR4) re-activates an *already-terminal* child (§0 requirement 3, "no cold start"). A terminal child already released its slot (§5.5: `Claimed → Released` on terminal). Re-activating it is therefore a **fresh outstanding obligation**, not a continuation — `subagent_extend`'s `activate()` call re-claims a descendant-reservation slot as a normal admission check, and fails with the existing capacity error (the same one spawn uses) if the tree is already full. This is the only place in the design where activating a child re-runs admission.

Required tests (**P4.4**): extend on a full tree → capacity rejection (integration-tier — drives extend through the real `activate()`/reservation admission path); extend releases its slot at the new run's terminal (crate-tier is sufficient for the reservation-accounting half).

**5.2 Two-layer exactly-once, `InvalidTransition` discriminator.** Layer 1: CAS single-winner per transition. Layer 2: `resume_turn`'s durable idempotency replay (`crates/ironclaw_turns/src/memory/mod.rs:187,1031-1046,2248-2269`, CAS'd in the same snapshot as every run-state transition). No `resume_dispatched_at` field needed.

**Boot/drain contract, matched on `from` — no wildcard.** `resume_turn_once` (`memory/mod.rs:2363-2384`) sets `from: record.status.get()` when `BlockedDependentRunGate`'s required status (`BlockedDependentRun`, `request.rs:48`) mismatches, raising `InvalidTransition{from, to: Queued}`.

- **Benign `already_closed`:** `from ∈ {Queued, Running, Completed}`.
- **`ResolveReport.failed`, surfaced:** everything else. No wildcard match.

Required tests (**P1.3**), crate-tier (`InvalidTransition` discriminator + CAS layer are pure state-machine logic, no external seam to drive through): (a) double-settle race → one resume; (b) crash-after-settle-before-resume → one resume via durable idempotency key; (c) crash-after-resume-before-drain → `InvalidTransition{from: Queued|Running}`-as-`already_closed` lets drain complete without a second resume.

**5.3 Boot enumeration: roster-driven + lazy backstop.** Eager: background boot task walks roster (§4.5), resolves unclosed edges per scope, never blocks foreground. Lazy: a spawn/activate against an unbooted scope triggers a one-shot scoped resolve first (`in_progress` guard).

**5.4 Observability.** `ResolveReport { resumed, drained, abandoned, already_closed, failed }` per scope, `debug!` + counters. Under §2, `drained`/`abandoned` now mean "resumed/finalized **and the edge file deleted**." `failed > 0` is actionable. `ironclaw-reborn subagent edges [--scope …]` lists unclosed edges off the roster. **Terminal byte-length accounting:** today's `AwaitedChildState.terminal_byte_len` (`gate_resolution.rs:27`) is set by `record_terminal_byte_len` (`:186-198`) as a best-effort **second** write *after* `mark_terminal_result_written` in `write_terminal_result` (`completion_observer.rs:497-518`) — **moves onto the edge as an additive `terminal_byte_len: Option<u64>` field, set in the same CAS write that transitions the edge to terminal.** Strictly better than today (folds two mutations that could drift into one atomic write). Still feeds `ByteCapStrategy` observability only, never durability — a missing value is logged at `debug!` and never blocks delivery.

## 5.5 Descendant-reservation release tri-state on the edge

**Gap:** the single-winner release guard for `SpawnTreeReservation` capacity lives entirely on the dying store — `descendant_reservation_released` / `descendant_reservation_release_claimed` fields plus `claim_descendant_reservation_release`/`mark_descendant_reservation_released`/`release_descendant_reservation_claim` (`gate_resolution.rs:334-379`, exact range). §3's replacement inventory lists this as "surviving" logic without saying where its state lives once the store dies — this section closes that gap.

**Ruling: the tri-state moves onto the `AwaitEdge`'s own state payload, CAS'd like every other transition — no separate store, no new file.** Add `reservation_release: ReservationReleaseState` (`Unclaimed | Claimed | Released`):
- `Unclaimed → Claimed`: CAS'd (`CasExpectation::Version(edge_version)`) exactly like today's in-memory check-then-set; only the CAS winner releases `SpawnTreeReservation` capacity.
- `Claimed → Released`: CAS'd after the capacity-release call succeeds, mirroring `mark_descendant_reservation_released`.
- A failed release attempt CAS's back `Claimed → Unclaimed`, mirroring `release_descendant_reservation_claim`'s retry-unlock, so a transient failure doesn't permanently strand the reservation.

Lives on the **same edge file** as settle/drain state — one more field, one more CAS'd path, not a second file — so it's deleted with the edge on drain/abandon (§2).

**Boot recovery also owns the claim-but-not-yet-released window.** An edge that crashed between the `Unclaimed → Claimed` CAS and the capacity-release call — i.e. scanned during recovery and found sitting in `Claimed` — is retried by the same boot/lazy recovery pass (§4.3/§5.3) that resolves every other unclosed edge; the `Claimed → Released` (or its failure-path retry-unlock back to `Unclaimed`) is just another CAS attempt the recovery walk makes, no special-cased recovery path needed.

**Required test (P1.9 extension), crate-tier:** (a) double-release race — two concurrent resolver instances (crash-retry racing a fresh recovery pass) both attempt to release the same child's reservation; assert capacity is released **exactly once** (the loser's `Unclaimed → Claimed` CAS fails, observes `Claimed`/`Released` already, no-ops); (b) boot-recovery-retries-`Claimed` — seed an edge already at `Claimed` (simulating the crash window above), run a recovery pass, assert it drives the edge to `Released` (or retry-unlocks to `Unclaimed` on a simulated release failure) rather than leaving it stuck.

## 5.6 `AwaitEdge` payload — assembled

One struct, one file per parent↔child, fields previously scattered across §2/§4.2/§5.4/§5.5:

```
AwaitEdge {
  child_scope: TurnScope,                          // §1
  child_thread_id: ThreadId,
  mode: SubagentSpawnMode,                         // Blocking | Background
  state: AwaitEdgeState,                           // Open | Settled | Drained | Abandoned (§2)
  terminal_kind: Option<EdgeTerminalKind>,         // set in the settle CAS
  terminal_byte_len: Option<u64>,                  // §5.4 — same settle CAS, observability only
  reservation_release: ReservationReleaseState,    // §5.5 — Unclaimed | Claimed | Released
  created_at, settled_at: Option<...>,             // stamps
}
```

Identity (`parent_run_id`, `child_run_id`) lives in the path, not the payload (§4.2). `Drained`/`Abandoned`-final edges are deleted (§2), so terminal states are transient on disk — the durable terminal facts live on the child's run record.

## 6. Run-budget floor (derived, not stored)

Cap: **8 consecutive `ParentAgent`-provenance activations per thread.** `Human` resets to 0, never capped. 9th consecutive `ParentAgent` → `subagent_extend_budget_exhausted`, sanitized, no internal identifiers cross the model-visible boundary.

**Counter — derived at `activate()`, not stored.** `SessionThreadRecord.metadata_json` (`crates/ironclaw_threads/src/contract.rs:184`) has **no mutation path**. Instead: `TurnRunRecord` gets one additive field, `subagent_activation_provenance: Option<ActivationProvenance>` (`ActivationProvenance { Human, ParentAgent, System }` — `System` is §8's background-wake provenance) set once at run creation, immutable thereafter.

`activate()` derives the cap via a **bounded** query, not an open-ended walk: fetch at most the newest **8** non-`System`-provenance run records for the thread (`K = cap = 8`, one indexed/ordered query, `LIMIT K`) — enough to decide the cap outright for the **pending** activation being admitted, without ever materializing a 9th `ParentAgent` record: if a `Human` run appears anywhere in that 8-record window, or history has fewer than 8 non-`System` records before its start, the pending activation is admitted (the streak resets or hasn't reached the cap); if all `K` fetched records are `ParentAgent` with no `Human` among them, the pending activation would be the 9th consecutive `ParentAgent` and is refused. `System` runs are excluded from the fetch itself (not filtered post-fetch), so they never count against the `K`-record budget. The constant is tied to the cap by construction (`K = cap`, not `cap + 1` — the cap is enforced at admission time on the *next* activation, so history itself never holds more than `cap` consecutive `ParentAgent` records to begin with) — raising the 8-activation cap means raising `K` in lockstep, never decoupling them (distinct from §12 tripwire 4, which is about the separate depth cap).

Unchanged: per-child SUBAGENT budget (`iteration_limit` 16, made per-flavor-overridable in §10); `SpawnTreeReservation` cap (16), depth cap (1, depth-agnostic per §9/§10).

## 6a. Human input priority — no content queue

Two mechanisms give a human priority over `ParentAgent`/`System` activation traffic on a thread. **The no-queue non-goal stands**: neither mechanism stores any payload, so none of a content queue's ordering, injection, or budget questions apply — a reservation is a marker, not a mailbox.

**(i) Next-slot reservation.** A human whose `activate()` hits `ThreadBusy` may set a `human_waiting` reservation marker on the thread. While set, `activate()` admission refuses `ParentAgent` and `System` provenance with a new `ThreadReserved` outcome — callers treat it exactly like `ThreadBusy` (retry/backoff, no special-casing needed at call sites). The marker clears when the human activates (their `activate()` succeeds) or when they explicitly abandon the reservation.

**The reservation is a lease, not an indefinite hold.** A disconnected client that never activates and never abandons would otherwise block `ParentAgent` **and** `System` (§8's settle-wake delivery) forever — a starvation bug, not a feature. **Ruling:** the marker carries `owner` + `expires_at` (default **15 minutes**, `HUMAN_RESERVATION_LEASE_TTL`); admission treats an expired lease as absent via a lazy, read-time check (`now > expires_at` at the point `activate()` reads the marker) — no timer task, no dedicated background reaper. No sweep over this per-thread marker is needed for correctness: a live `activate()` call re-checking an expired marker is sufficient on its own, so a stale marker left after a client disconnects is inert — it simply never blocks anything again once its `expires_at` has passed, whether or not anything ever cleans up the file itself. Human re-activation or explicit abandon still clears the marker immediately, ahead of expiry; renewing the reservation is a re-`put` with a fresh `expires_at`.

**This does not contradict §12's "no gate expiry timers" non-goal.** That non-goal is about **security-decision gates** (approval/auth) — they guard a sensitive action and must park forever until a human decides, never auto-resolve on a clock. The `human_waiting` reservation carries no security content at all — it is a UX courtesy hold on scheduling priority, and letting it lapse only returns the thread to ordinary `ParentAgent`/`System` admission, exactly the state it would be in had the human never reserved it. Lapsing a courtesy hold is safe by construction; auto-resolving a gate would not be.

Deferred `System` settle-wakes are not lost while a reservation holds `ParentAgent`/`System` off the thread — the existing run-start sweep and boot pass (§8.2, triggers 2 and 3) already heal any settle-wake that loses to `ThreadBusy`/`ThreadReserved`; the settled edge just waits its turn, same as any other `ThreadBusy` deferral.

**Marker storage:** a small marker file, scoped and CAS'd the same way as every other piece of state in this design (§4.0/§4.5's pattern) — not a mutation onto `SessionThreadRecord.metadata_json`, which §6 already establishes has **no mutation path**. Reusing the filesystem-marker pattern this design already relies on for edges and the scope roster is simpler than opening a first mutation path onto `metadata_json` for a feature that doesn't need anything else `metadata_json` carries. Payload: `{ owner: UserId, expires_at: Timestamp }` — `owner` identifies the reserving human for audit/UX, `expires_at` backs the lease check above.

Ships with the extend PR (**P4.3**) — `activate()` admission is where `ThreadBusy` already lives, so `ThreadReserved` is a sibling admission outcome in the same place. **Required tests — integration-tier**: (a) reservation set → parent `subagent_extend` refused with `ThreadReserved` → human `activate()` succeeds and clears the marker → a subsequent parent extend is admitted normally; (b) reservation set with an expired `expires_at` → a `ParentAgent` `activate()` is admitted (expired lease treated as absent), with no timer task or reaper involved.

**(ii) Interrupt & take over.** Console-only compose of two existing primitives — the underlying run-cancel mechanism (the same one `subagent_cancel`, §7 PR6, later wraps as a model-facing tool) and `activate(Human)` (§1) — no new state, no new admission outcome, and no dependency on PR6's tool-level capability landing first. A human on a child (or parent) console cancels the live run and activates in its place. Ships with the console UI PR (**P5.4**).

## 7. Functionality staging

| PR | Ships | Gate |
|----|-------|------|
| 1 | edge store + resolver + scope roster + boot/lazy recovery + **CAS-guarded delete (P1.0b, §4.0)** + depth floor + `wrap_untrusted_subagent_text` promotion + **P1.0: #5466 fix** + exactly-once + scope-isolation + **descendant-reservation tri-state (§5.5)** tests — **replaces** gate-store delivery outright (blocking only) | P1.0 + P1.0b done; all tests green both backends |
| 2 | background mode + activate-on-settle for parked/completed parents (§8) + `ResolveReport` counters + operator `edges` command + **gate-propagation escalation walk, moved up from PR6 (§9, P2.5/P2.6), including resolution-from-any-owner-surface (§9.2)** | PR1 soaked; integration matrix green; gate-walk tests green |
| 3 | `subagent_inspect` (metadata-only) + per-flavor budget plumbing (P3.2, §10c) | — |
| 4 | `subagent_extend` (`activate` + `ParentAgent` + consent-to-wake + budget, §6) + reservation re-claim at admission (P4.4, §5.1) + next-slot reservation / `ThreadReserved` (P4.3, §6a) | — |
| 5a | `GET /api/webchat/v2/threads/{thread_id}/children` (lineage projection, no new store) | ~0.5 day |
| 5b | `ThreadTree` sidebar + raw-vs-framed display rule (§11) + interrupt & take over (P5.4, §6a) | ~1.5-2 days |
| 6 | `subagent_cancel` | security review |

**Gate-coverage window.** §9's escalation walk is now a **prod-enable gate**, not a PR6 afterthought — it ships with PR2, before `spawn_subagent` is ever cleared from `disabled_capability_ids`. During PR1 alone (blocking-only), a descendant's `BlockedApproval`/`BlockedAuth` has **no escalation** yet — the harness-only-phase inherited behavior: per §1, "child = normal Turn thread... ordinary thread operations," so the gate surfaces exactly the way **any** blocked thread's gate does today — via the existing origin-agnostic gate projection (`ironclaw_event_projections::PendingGateProjection`, `crates/ironclaw_event_projections/src/pending_gate_projection.rs`) and per-thread approval service (`crates/ironclaw_product_workflow/src/approval_interaction/service.rs`), neither new for subagents. The gap during this window is **discoverability, not resolvability**: a human must directly inspect/attach to the child thread (available from PR1 via §1, ahead of PR3 formalizing `subagent_inspect`) rather than being paged at the root automatically. Since `spawn_subagent` stays deny-filtered through PR1 regardless (the standing no-flag ruling), this window is unreachable in production — it only constrains what PR1's own tests may assume.

Prod enable: clear `builtin.spawn_subagent` from `disabled_capability_ids` after PR2 (now including the gate-escalation walk), e2e un-ignored, matrix green. No flag. **The e2e un-ignore task itself is integration-tier by definition** (`tests/e2e/`) — it is the end-to-end prod-enable gate, not a substitute for the crate/integration tests above.

## 8. Background delivery = activate

`PostCapabilityStage::drain_settled` (`crates/ironclaw_agent_loop/src/executor/post_capability.rs:36-37`) is **a permanent no-op stub today, not existing behavior to preserve** — it unconditionally `Vec::new()`s. Its doc comment names `LoopBackgroundChildPort` as the replacement; that type **does not exist anywhere in `crates/`** — verified, it is prose only in `docs/reborn/2026-06-04-subagent-compaction-design.md` (lines 66,83-84,180,275-287,322) plus a doc-comment pointer (#4474). Also verified: `PostCapabilityStage::process` runs on **every** `TurnCompletedStep::Continue` (`post_capability.rs:60-99`) — every non-exiting loop iteration, including a freshly-activated run's first (AssistantReply turns "reach here with an empty map" per the existing comment) — a fact §8.2 relies on.

**Ruling: `drain_settled` is NEWLY IMPLEMENTED by an owned PR2 task (P2.4) wiring it to `AwaitEdgeResolver` — list settled edges for the live parent, drain each. This supersedes the #4474 stub contract; no `LoopBackgroundChildPort` is ever built.**
- **Live parent:** `drain_settled` drains each iteration (P2.4).
- **Parked/completed parent:** on child settle, resolver calls `activate(parent_thread, input, provenance=System)`. `ThreadBusy` is a benign no-op — edge stays `settled`; §8.2's retry set picks it back up. `System` activations are exempt from §6's cap, bounded to **one** attempt per settled child — the edge's `settled` state is the dedupe.

**Batched drain, not per-edge rescan.** When `drain_settled` (or a recovery/boot pass) finds **multiple** settled edges for the same parent, it must batch all their `result_ref`/`safe_summary` transcript updates into **one** thread-snapshot read + **one** CAS write pass — never one rescan-and-CAS-retry cycle per edge. `update_parent_result_reference` → `update_tool_result_reference` (§8.1) is the right primitive for a *single* settled edge's CAS-retry closure; the batch caller accumulates the full set of settled `(result_ref, safe_summary)` pairs for the parent first, then invokes the transcript-update helper **once** over that whole set — one snapshot read, every matched message rewritten in place, one CAS write — rather than calling it `E` times against a thread with `M` messages. This keeps a multi-edge drain O(E+M) instead of O(E×M); the O(E×M) shape is exactly what a naive per-edge loop over `update_parent_result_reference` would produce on a run-start sweep or boot pass that finds several children settled at once.

**8.1 Drain-append idempotency.** The framed-content write into the parent — `update_parent_result_reference` → `update_tool_result_reference` (`crates/ironclaw_threads/src/filesystem_service.rs:1965-2013`) — is a **CAS-guarded in-place field update on an already-existing message**, not an append: it rescans for the tool-result-reference message (`matches_tool_result_reference`) then rewrites its `content` via `apply_message_update`'s CAS-retry closure. Calling it twice with the same `result_ref`/`safe_summary` reproduces the same `content` — **already idempotent by construction**, verified not asserted. The payload write it follows (`update_capability_result`, from `write_terminal_result`, `completion_observer.rs:494-518`) is guarded today by the in-memory `terminal_result_written` flag; under the edge design that guard **becomes the edge's own CAS state** (write payload once per `settled`→`drained` transition — a crash before that CAS just retries the whole flow, safe precisely because the transcript write is an overwrite, not an append). **Required test — integration-tier** (`tests/integration/`; drives the drain path through the real `AwaitEdgeResolver` + thread service, asserting at the transcript-message seam per `.claude/rules/testing.md`, part of P2.4's acceptance): crash after the transcript field-write but before the edge's CAS to `drained` → recovery replays the equivalent write → assert exactly one tool-result-reference message for that `result_ref`, content unchanged by the replay.

**Required test (P2.4 extension) — integration-tier:** drain of N (≥2) settled edges for the same parent in one pass performs exactly one thread-snapshot read and one CAS write — not N of each — asserted via a write-count seam (e.g. a counting wrapper around the thread service's snapshot-read/CAS-write calls, or an equivalent call-count assertion).

**8.2 System-activate retry set.** A settled edge for a parked/live parent is drained by exactly three independent triggers:
1. **Settle-time** `activate(..., System)` attempt (above) — may lose to `ThreadBusy` mid-run.
2. **Every run-start sweep** — `PostCapabilityStage::process` runs on every `Continue`, including a fresh run's first iteration, so the *next* time this thread runs for **any** reason, `drain_settled` (P2.4) picks up every still-settled edge as a side effect of that run happening at all.
3. **Boot pass** (§4.3/§5.3) — roster-driven, independent of any thread activating.

**Invariant:** a `ThreadBusy` at settle-time (trigger 1) is always healed by trigger 2 or 3 — a settled edge can never go permanently undrained while its parent thread ever runs again or a boot pass ever occurs. **Required test — integration-tier** (`tests/integration/`, same P2.4 acceptance as §8.1, driven through the live parent thread + boot-pass harness, not the resolver in isolation): settle a child mid-parent-run (forcing `ThreadBusy` on trigger 1); assert no further `System` attempt (dedup by `settled` state); then either (a) let the live run's next iteration drain it (trigger 2), or (b) run a boot pass and assert the roster sweep does (trigger 3). **Make the parent-completed precondition explicit** (§2's mode-scoping): the parent run reaching its own terminal state (`Completed`) while the background child is still running and its edge is `open` is not a special case of the above — the test asserts the edge stays `open` (never `abandoned`, per §2) across the parent's own terminal transition, and that the child's later settle still wakes the parked parent thread via trigger 1 (or trigger 3, if the thread never runs again first) and drains normally.

## 9. Gate propagation — approval and auth, always to the tree root

Any gate from any descendant, any depth, bubbles to the **tree root's** originating human surface. Covers **auth/credential gates** (`BlockedAuth`) too, not just approvals. **Surfacing is always the tree root's originating surface** — one deterministic paging point, unchanged. **Resolution is not the same thing as surfacing**: it is accepted from any surface authenticated as the tree owner, including a child thread's console a human has open directly — §9.2 lifts the old blanket restriction for owner-authenticated humans specifically; it still holds, without exception, for any LLM in the chain.

**Both kinds, one shape — no new gate representation.** `TurnBlockedGateMetadata` (`crates/ironclaw_turns/src/events.rs:59-66`) already carries `gate_kind: TurnBlockedGateKind` (`Approval`, `Auth`, `Resource`, `AwaitDependentRun`, `ExternalTool`) plus `credential_requirements`. The walk is generic over `gate_kind`.

**9.1 Root-delivery-surface resolution, field-by-field.** The root's `source_binding_ref` is **not** unconditionally the delivery surface — this breaks for triggered origins. Resolution chain, reusing existing production machinery instead of inventing a fallback: `ironclaw_outbound`'s `resolve_run_notification_context` (`crates/ironclaw_outbound/src/resolution_engine.rs:59-95`) already dispatches on `RunNotificationOrigin` (`.../delivery_resolution.rs:137-151`):
- **`LiveSourceRoute { source_route }`** → `source_route.reply_target_binding_ref` verbatim (`:73-75`). Covers **interactive chat and live Slack**: the root's `TurnRunRecord.reply_target_binding_ref` (`ironclaw_turns/src/store.rs:160,171`) genuinely identifies a real conversation.
- **`Triggered { trigger }`** → `resolve_triggered_target(scope, actor, kind)` (`:76,119-137`), which for `ApprovalPrompt` calls `load_preference_target(scope, actor, PreferenceTargetKind::ApprovalPrompt)` — resolved from `TurnScope.explicit_owner_user_id()` (the trigger creator) via the owner's outbound preference, **bypassing `reply_target_binding_ref` entirely.** Every trigger fire's binding is synthesized against the canonical `TRIGGER_TRUSTED_ADAPTER_KIND = "trigger"` adapter (`crates/ironclaw_triggers/src/trusted_submit.rs:3,33-41`; predicate `is_trusted_trigger_adapter_kind`, "the trigger-owned authority," lines 7-13) — never a live channel — so a triggered root's binding refs are unconditionally non-actionable, not "sometimes." (Note: `SourceBindingRef::new("trusted-trigger-outcome-source")` at `ironclaw_conversations/src/inbound.rs:1053` is a **unit-test fixture** inside `#[cfg(test)] mod tests` opening at line 529, not a production value — the real non-actionable value is the `"trigger"` adapter binding above.)
- **`TriggeredFromSourceRoute { trigger, source_route }`** → hybrid path for a triggered fire with a genuine source route too (`:78-86`).
- **`SystemEvent { reason }`** → `NoDelivery` (`:87-89`).

**Origin classification** reuses the existing predicate `is_automation_trigger_thread` (`crates/ironclaw_product_workflow/src/reborn_services.rs:4727-4737`, parses `SessionThreadRecord.metadata_json`) — no new classifier.

**Ruling:** the walk's root-delivery step builds a `RunNotificationContext`/`CommunicationDeliveryIntent::RunNotification` for the tree root (root's `TurnScope` + classification above) and delegates to this **existing** engine — it does not read `source_binding_ref` directly, and invents no second fallback.

**Named integration gap (not hand-waved):** today's triggered-run delivery poller (`SlackFinalReplyDeliveryServices`/`deliver_triggered_run`, `crates/ironclaw_reborn_composition/src/slack_delivery.rs:2033`, feature `slack-v2-host-beta`) watches the **root run's own** status for `BlockedApproval`/`BlockedAuth` (`notification_for_actionable_state`, `:395-460`). Under this design a gate parks the **descendant**, not the root — the root sits at `BlockedDependentRun` (§5.2), so this poller would miss the gate entirely. **P2.5/P2.6 (§7) own extending it (or a successor) to accept the walk's resolved gating-run id/kind/gate_ref, not just observe the root run it already tracks** — a real required wiring change, not a pre-existing behavior to lean on unmodified.

**Auth identity — no new resolver.** `RuntimeCredentialAuthRequirement.requester_extension: ExtensionId` (`crates/ironclaw_host_api/src/decision.rs:106-113`) is already typed on the gate metadata the walk forwards verbatim.

**Required tests (P2.5/P2.6) — integration-tier** (`tests/integration/`; the walk, root-routing, and the Slack poller extension above all cross the descendant→root→delivery-surface seam, not a single-function unit): the depth-agnostic walk resolving a descendant gate to the tree root's delivery surface for each `RunNotificationOrigin` branch (§9.1), and the extended triggered-run delivery poller (`deliver_triggered_run`, above) picking up the walk's resolved gating-run id/kind/gate_ref instead of only the root run's own status. **Add (§9.2):** resolution submitted from a child-console surface resolves the gating run exactly once — the second resolution attempt observes the already-resolved CAS state and no-ops.

**Mechanics — depth-agnostic:**
1. The gating run parks. No ancestor run touched.
2. Resolver walks `parent_run_id` upward: skip terminal runs, stop at the run whose `parent_run_id` is `None` — the root.
3. §9.1's resolution engine picks the delivery target from the root's scope + origin.
4. Resolution flows back down: the surface's decision resolves the **gating run itself** — not the root run.
5. An `abandoned` (deleted, §2) edge along the chain doesn't break the walk — lineage lives on the immutable run record.

**9.2 Resolution accepted from any owner-authenticated surface.** Surfacing and resolution are decoupled. Surfacing (§9's opening ruling) stays exactly as designed — the tree root's originating surface is the sole paging point, unconditionally. What changes: the gate **resolution** itself — the actual approval/auth decision — is accepted from *any* surface authenticated as the tree owner, not only the root's originating surface. This includes a child thread's console a human has directly opened (§1, `activate(Human)`; PR5a/5b once shipped, but the underlying approval-interaction service has no console dependency — it is exercisable as soon as any owner-authenticated surface can reach a `gate_ref`, ahead of full ThreadTree UI).

Why this is safe, not a hole in the never-auto-approve floor:
- **The approver must be a human, from any surface.** No LLM anywhere in the chain resolves a gate — a subagent (at any depth) referencing its own or a sibling's pending gate still never resolves it. Only lifting this for owner-authenticated humans, never for automation, is what makes it safe.
- **The approval service was never root-scoped, only root-paged.** `crates/ironclaw_product_workflow/src/approval_interaction/service.rs` resolves against `gate_ref`/`run_id` with no thread affinity baked into the decision path — root-surfacing was a paging convention layered on top, not a constraint the service itself enforced.
- **Two doors, one CAS'd lock.** Gate resolution is CAS-idempotent, single-winner, the same pattern as every other transition in this design (§5.2, §5.5). Whichever surface's resolution attempt wins the CAS resolves the gate; the other observes the already-resolved state and no-ops. Two resolution paths onto one lock is exactly the shape CAS'd resolution already handles safely.

**Audit:** the resolution record captures *which* surface actually resolved the gate — root or a specific non-root owner surface — not just that it was resolved, so a human reviewing the tree afterward can see where the call was made.

## 10. Per-flavor configurability — flavors are data, the loop is shared

All 4 launch flavors (`SubagentFlavorId::{General, Explorer, Coder, Planner}`, `crates/ironclaw_reborn/src/subagent/flavors.rs:9-16`) run the **same** loop machinery — one `LoopFamily` (`LoopFamilyId::SUBAGENT`, `crates/ironclaw_agent_loop/src/families/subagent.rs`). **A flavor is config, not a fork.**

**(a) Prompt — already file-sourced.** `.../subagent/directions/{general,explorer,coder,planner}.md` via `include_str!()` (`directions/mod.rs:22-25`). No change.

**(b) Capability allow/deny — reuses existing machinery.** `SubagentCapabilitySurfaceResolver::resolve` (`.../subagent/capability_surface.rs:30-46`) computes `intersect_allow_sets(base, flavor_allowlist)`; deny layered outermost by `CapabilitySurfaceDenyFilter` (`ironclaw_loop_support/src/capability_surface_filter.rs:174-195`). Nothing new.

**(c) Budget — the one real gap, currently family-wide.** `SUBAGENT_ITERATION_LIMIT = 16` (`.../families/subagent.rs:9`) baked into `DefaultBudgetStrategy` (`:37-41`). **Ruling:** add `iteration_limit: u32` to `SubagentFlavor` (default 16), read at `material_for_run` — family default stays fallback (**P3.2**).

**(d) Model override — same pattern as (c).** `resolved_model_route` already overrides `DefaultModelStrategy` per run. `SubagentFlavor.model_override: Option<ModelRouteId>` feeds the same resolution (**P3.3**).

**Flavor schema (additive for future custom flavors):**
```
SubagentFlavor {
  id: SubagentFlavorId,
  direction: DirectionId,              // (a) unchanged
  tool_allowlist: &[SubagentToolId],   // (b) unchanged
  iteration_limit: u32,                // (c) NEW, default 16
  model_override: Option<ModelRouteId>,// (d) NEW, default None
  allow_nesting: bool,                 // unchanged
  summary: &str,                       // unchanged
}
```

## 11. WebUI scoping & extensibility

**PR5 WebUI.** `GET /api/webchat/v2/threads/{thread_id}/children` (lineage read projection, no new store) + `ThreadTree` sidebar. **Authorization:** the endpoint resolves the parent thread through the caller's authenticated scope, using the same owner-bound `TurnScope` resolution every other webchat v2 thread-read endpoint uses; an unauthorized or unknown `thread_id` collapses to the same not-found response as those reads (no existence oracle), and the returned children are filtered to that same resolved scope. Display rule: opening a child directly shows the raw transcript (§1); a "what did my subagent do" parent-agent-framed view shows only the framed + byte-capped edge content. **Required test (P5.2) — integration-tier** (`tests/integration/`; the endpoint is a real HTTP seam over live lineage data): the children endpoint returns the correct lineage projection for a thread with settled/drained/parked descendants; a cross-user request for another caller's thread returns not-found, with no lineage leaked.

Extensibility (unchanged): fork-on-extend, structured child→parent output schema, cross-agent shared memory, per-token budgets, tool attenuation at depth, `ARCHIVED` state + GC, file-discovered custom flavors — named deferrals, no code today.

## 12. What this design deliberately does NOT do

- No SQL tables, ledger, reconciler phases, dual-dialect DDL.
- **No semantic injection scanning of child output** — the equivalent-control posture is untrusted framing (`wrap_untrusted_subagent_text`) + the byte cap (§5.4) + gate-driven approval-bubbling to a human (§9) as the catastrophe backstop, not a scan. The unscanned-ingress gap this implies is platform-wide and pre-existing — Reborn's `submit_turn` ingress has no `SafetyLayer` wiring today for any caller, not something this design introduces for subagents specifically. When Reborn-wide `SafetyLayer` ingress wiring lands, the child→parent drain write adopts that same scan point like every other model-bound input — no bespoke subagent scanner — and that follow-up's scope must explicitly list the drain path as one of the ingress points it covers.
- No auto-cancel of orphans.
- No new crate; no feature flag.
- **No separate live-index/archive layer for closed edges (§2).**
- **No gate expiry timers.**
- **No multi-node/HA correctness claims.**
- **No signature change to `RootFilesystem::delete`** — the CAS primitive is additive (`delete_if_version`, §4.0), avoiding a ~20-call-site blast radius.
- **No content queue for human-priority activation (§6a)** — the next-slot reservation stores a marker, never a payload; no ordering, injection, or budget semantics to design for.
- **Tripwires (flip these rulings if hit):** (1) #5466 turns out structural to CAS-over-libsql, not fixable at the RootFilesystem layer; (2) a scope's roster directory becomes pathological to `list_dir` — shard by tenant; (3) any scope's *open* edge count grows large enough that admission-time `list_dir` is measurably slow — one CAS'd count file; (4) depth cap raised above 1 and §8/§6 need more than the limits knob.

## 13. Design→Plan cross-map

**Self-contained.** Every task id below has its files-touched + acceptance defined inline in the referenced section — this table is a locator, not the source of truth. The implementation plan this design feeds (tracked under `docs/plans/`, not committed to source) is retasked from this design end-to-end as its first deliverable, **P0.0**: drop any flag-centric framing carried over from earlier drafts, retask against P1.0-P6.x below, delete any leftover flag-setup tasks. No task ID depends on a prior plan draft surviving.

| Design item | Task ID(s) | Note |
|---|---|---|
| §0 requirements | — | Traceability only |
| §2 closed-edge deletion | P1.2 (delete-on-drain/abandon via `delete_if_version`), P1.10 (NotFound-as-benign), **P1.0b's crash-ordering recovery sweep** | Supersedes the prior per-parent live-index approach entirely; abandon is now mode-scoped to (a) unresumable blocking edges, (b) explicit teardown — background parent-completion is not an abandon trigger (§8) |
| §3 replacement inventory | P1.1, P1.9 | Tombstone-store deletion now includes its `production_readiness.rs` field + test |
| §4.0 CAS-guarded delete | **P1.0b** (new, PR1 merge blocker alongside P1.0) | `delete_if_version` on `RootFilesystem` + 3 backends + `ScopedFilesystem`/`CompositeRootFilesystem` passthrough; residual-race text and test (4b) now stated roster-before-edge-consistent with §4.5 |
| §4.1 crate placement, DIP | P1.1, P1.4 | Permanent per type-placement.md cat. 2 |
| §4.2 canonical path | P1.1, P1.2 | Path now also encodes optional agent/project axes in-path (§4.5a) — the mount alone is tenant/user-only |
| §4.3 boot-recovery driver | P1.4, P1.6a | Now also enumerates terminal-but-undeleted edges; `list_dir` is now axis-qualified (agent/project), not just tenant/user-mounted; recovery concurrency bounded (`BOOT_RECOVERY_MAX_CONCURRENT_SCOPES = 4`) |
| §4.4 #5466 fix | **P1.0** | PR1 merge gate, alongside P1.0b |
| §4.5 roster + scope-key | **P1.6a, P1.6b (dual-backend), P1.6c (scope-aware construction, §4.5a, #5721 cross-check)** | Write-before-first-edge ordering is part of P1.6a; roster key now also carries a project axis alongside agent, matching the edge path; §4.5a's two-users-distinct-paths and two-agents-same-user-distinct-paths tests — integration-tier — are part of P1.6c |
| §4.6 module placement | P1.1, P1.2, P1.4, P1.6a, P2.1-P2.3 | — |
| §5.1 capacity | Already shipped; **P4.4** (extend re-claims a reservation slot at admission + capacity tests) | Non-terminal cap now explicit as counting gate-parked children deliberately (side-effect-obligation bound, not compute); never release-on-park; `subagent_extend` is the sole re-claim path |
| §5.2 exactly-once, discriminator | P1.3 (3 tests), P1.9 (contract) | — |
| §5.3 boot/lazy enumeration | P1.6a, P1.9 | — |
| §5.4 observability | P2.1, P2.2 | `terminal_byte_len` moves onto the edge |
| §5.5 reservation-release tri-state | **P1.9's extension** (new test), state lives on `AwaitEdge` | Release-at-terminal-only is the mechanism §5.1's re-claim ruling depends on |
| §6 run-budget, derived | **P4.2** | Adds provenance field; cap derived via a bounded `LIMIT`-8 query (`K = cap`), not an open-ended walk |
| §6a human input priority, no queue | **P4.3** (next-slot reservation + `ThreadReserved` admission, ships with extend PR), **P5.4** (interrupt & take over, ships with console PR) | No content queue (§12); marker stored via scoped-filesystem CAS pattern, not `metadata_json` (§6 already has no mutation path there); reservation is a lease (`owner` + `expires_at`, default 15min, lazy expiry, no reaper) — not an indefinite hold — and carries no security content, distinct from the gate-expiry non-goal (§12) |
| §7 staging | Governs PR1-6 | Gate-escalation walk moved PR6→PR2; PR4 gains P4.3/P4.4; PR5b gains P5.4 |
| §8 background = activate | **P2.4** | Drain-append idempotency and the system-activate retry set are part of P2.4's acceptance; parent-completed-while-open precondition now explicit (§2's mode-scoped abandon); multi-edge drain batches into one snapshot/CAS pass, not one per edge (O(E+M), not O(E×M)) |
| §9 gate propagation | **P2.5 (walk + root routing via `ironclaw_outbound` resolution engine), P2.6 (auth rendering, `requester_extension` reuse)** | Moved from P6.1/P6.2; P2.5 also extends the Slack triggered-delivery poller to accept the walk's gating-run id (the named integration gap in §9.1); §9.2's resolution-from-any-owner-surface + child-console resolution test fold into the same P2.5/P2.6 |
| §10 per-flavor | (a)/(b) no task, live; **(c) P3.2, (d) P3.3** | — |
| §11 PR5 WebUI | **P5.2 (5a), P5.3 (5b)** | — |
| §12 non-goals/tripwires | — | — |
| Companion plan | **P0.0** (first deliverable) | Retasks the `docs/plans/` implementation plan end-to-end (see intro above) |
| No-flag ruling | Deletes any legacy flag-setup task; folds into P1.8 | — |

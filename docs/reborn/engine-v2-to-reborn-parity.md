# Engine v2 → Reborn Capability Parity

**Status:** Validation record — parity evidence for removing engine v2.
**Date:** 2026-07-02
**Purpose:** Prove that every capability the interim **engine v2** architecture
(`crates/ironclaw_engine` + `src/bridge/`, gated behind `ENGINE_V2`, default
off) provided has a home in the **Reborn** architecture
(`crates/ironclaw_reborn*` + the neutral contract crates, shipped as the
standalone `ironclaw-reborn` binary). This document is the justification for
deleting engine v2 from the codebase.

## Source of Evidence

Engine v2 (`crates/ironclaw_engine`, `src/bridge/`) and the Reborn crates
(`crates/ironclaw_reborn*` plus the neutral contract crates) **coexist on
`main`**, so every path cited below — contract docs under
`docs/reborn/contracts/`, Reborn crates under `crates/`, and the
`tests/reborn_*.rs` / crate-local `tests/` suites — is present and verifiable on
the same tree this document is committed to. All paths are repo-relative. This
parity record is the justification attached to the engine-v2 removal change.

## Summary

Engine v2 was an **interim** unification step: it collapsed ~10 legacy
abstractions (Session, Job, Routine, Channel, Tool, Skill, Hook, Observer,
Extension, LoopDelegate) into five primitives (Thread, Step, Capability,
MemoryDoc, Project) behind three host-implemented traits (`LlmBackend`,
`Store`, `EffectExecutor`). It was never the default runtime and never reached
product completeness. Reborn is the **target** architecture and is *fully
independent* of engine v2 — it does not depend on `ironclaw_engine`, and its
kernel/host-runtime boundary re-derives the same primitives from first
principles as narrow, individually-tested contract crates. Because engine v2 is
gated off (`ENGINE_V2` default false) and Reborn does not import it, removing
engine v2 changes no shipping behavior; the only question this document answers
is whether Reborn is a *superset* of engine v2's capability surface. The matrix
below shows every engine-v2 capability maps to a Reborn contract + crate +
tests, with two capabilities honestly marked **Partial** (the in-loop
Monty/RLM CodeAct orchestrator, and per-action reliability/estimation
tracking) — both of which were themselves incomplete or non-production in
engine v2 and are intentionally reshaped or deferred in Reborn.

## Capability Parity Matrix

Legend: **Covered** = Reborn has an equivalent contract, crate, and test
evidence; **Partial** = an equivalent exists but with a scoped delta noted;
**Gap** = no direct equivalent.

| Engine v2 capability | Reborn equivalent | Contract doc | Crate(s) | Test evidence | Status |
| --- | --- | --- | --- | --- | --- |
| **Thread** — unit of work; lifecycle state machine (`ThreadState`), parent-child tree, capability leases (`types/thread.rs`, `runtime/manager.rs::ThreadManager`) | Typed run/thread/turn state with a durable lifecycle answer ("what is this invocation doing/waiting on"); host-coordinated `TurnCoordinator` | `contracts/run-state.md`, `contracts/turns-agent-loop.md`, `contracts/turn-persistence.md` | `ironclaw_threads`, `ironclaw_run_state`, `ironclaw_turns` | `tests/reborn_thread_binding_isolation_parity.rs`, `tests/reborn_turn_state_lock_free_submit_parity.rs`, `crates/ironclaw_conversations/tests/inbound_contract.rs` | Covered |
| **Step** — unit of execution (one LLM call + its action executions); `executor/loop_engine.rs::ExecutionLoop` | Turn-scoped agent-loop step with a validated `LoopExit` handshake; pluggable loop families over the kernel surface | `contracts/turns-agent-loop.md`, `contracts/loop-exit.md`, `contracts/agent-loop-protocol.md`, `contracts/lightweight-agent-loop.md` | `ironclaw_agent_loop`, `ironclaw_loop_support`, `ironclaw_reborn` (loop drivers) | `tests/reborn_response_order_parity.rs`, `crates/ironclaw_reborn/tests/loop_driver_host.rs`, `crates/ironclaw_reborn/tests/planned_driver_e2e.rs`, `crates/ironclaw_reborn/src/loop_exit_applier/tests/mod.rs` | Covered |
| **Capability** — unit of effect (actions + knowledge + policies), replaces Tool/Skill/Hook/Extension; `types/capability.rs`, `capability/registry.rs` | Host-mediated capability invocation: descriptor lookup, trust-aware authorization, dispatch, spawn, obligations | `contracts/capabilities.md`, `contracts/capability-access.md`, `contracts/dispatcher.md` | `ironclaw_capabilities`, `ironclaw_authorization`, `ironclaw_dispatcher` | `tests/reborn_minimal_dispatch_parity.rs`, `crates/ironclaw_dispatcher/tests/vertical_slice_contract.rs`, `crates/ironclaw_host_runtime/tests/reborn_invoke_vertical_slice.rs` | Covered |
| **Capability leases** — scoped/time-limited/use-limited grants; `capability/lease.rs::LeaseManager`, `LeasePlanner` | Grant- and lease-backed dispatch/spawn gates; async lease stores in the control plane | `contracts/capability-access.md`, `contracts/approvals.md` | `ironclaw_authorization`, `ironclaw_approvals` | `tests/reborn_agent_scope_isolation_parity.rs`, `tests/reborn_wrong_scope_access_isolation_parity.rs`, `crates/ironclaw_reborn_composition/tests/budget_e2e.rs` | Covered |
| **PolicyEngine** — deterministic effect-level allow/deny/approve (`Deny > RequireApproval > Allow`) + provenance taint; `capability/policy.rs`, `types/provenance.rs` | `CapabilityDispatchAuthorizer` returning `Decision::Allow{obligations}/Deny/RequireApproval`; trust-class enforcement and taint at the kernel boundary | `contracts/capability-access.md`, `contracts/kernel-boundary.md`, `contracts/trust-boundary-hardening.md` | `ironclaw_authorization`, `ironclaw_trust` | `tests/reborn_agent_scope_isolation_parity.rs`, `tests/reborn_tenant_binding_scope_isolation_parity.rs`, `crates/ironclaw_reborn_composition/src/factory/local_dev_host_tests/approval_gates.rs` | Covered |
| **EffectType** taxonomy — `ReadLocal/ReadExternal/WriteLocal/WriteExternal/CredentialedNetwork/Compute/Financial`; `types/capability.rs` | Effect surface decomposed into typed resource/network/secrets/filesystem contracts feeding the authorizer + `ResourceEstimate` | `contracts/resources.md`, `contracts/network.md`, `contracts/secrets.md`, `contracts/filesystem.md` | `ironclaw_resources`, `ironclaw_network`, `ironclaw_secrets`, `ironclaw_filesystem` | `tests/reborn_http_network_scope_isolation_parity.rs`, `crates/ironclaw_reborn/tests/secrets.rs`, `tests/integration/secrets.rs` | Covered |
| **MemoryDoc** — durable knowledge (Summary/Lesson/Skill/Issue/Spec/Note); `types/memory.rs`, `memory/store.rs::MemoryStore`, `RetrievalEngine` | Provider-neutral `MemoryService` contract + native provider (chunking/indexing/embeddings/search, prompt-context assembly) | `contracts/memory.md`, `contracts/memory-profiles.md`, `contracts/storage-placement.md` | `ironclaw_memory`, `ironclaw_memory_native` | `tests/integration/group_memory/` (suite), `tests/reborn_qa_doc_grounding.rs` | Covered (note 1) |
| **Project** — unit of context (scopes memory/threads/missions); `types/project.rs` | First-class `ProjectRecord` + membership ACL (`Owner>Editor>Viewer`) over the scoped filesystem substrate | `contracts/storage-placement.md`; plan `docs/plans/2026-06-17-reborn-projects.md` | `ironclaw_projects` | `crates/ironclaw_projects/tests/repository_contract.rs`, `tests/reborn_project_scope_isolation_parity.rs`, `tests/reborn_identity_project_scope_isolation_parity.rs` | Covered |
| **Missions** — long-running goals that spawn threads on cadence; `runtime/mission.rs::MissionManager`, budget/rate gates | Scheduled trigger intake → synthetic inbound turn on the normal turn pipeline; budgets via authorization/approvals | `contracts/triggers.md`, `contracts/approvals.md` | `ironclaw_triggers` | `tests/reborn_qa_routines.rs`, `crates/ironclaw_reborn_composition/tests/trigger_poller_e2e.rs`, `crates/ironclaw_reborn_composition/tests/trigger_webui_timeline_e2e.rs`, `crates/ironclaw_reborn_composition/tests/budget_approval_e2e.rs` | Partial (note 2) |
| **Learning missions** — error diagnosis, skill repair, skill extraction, conversation insights (`MissionManager::ensure_learning_missions`) | Skill distillation/refinement pipeline (extract + repair) over trace input | plan `docs/plans/2026-06-16-reborn-skill-evolution.md`; `contracts/skills-extension.md` | `ironclaw_skills::learning` | `crates/ironclaw_skills/src/learning.rs` (`distill_skill_runs_inference_then_validates`, `parses_a_valid_skill_and_extracts_the_name`, `parse_refinement_accepts_a_refined_skill`) | Partial (note 2) |
| **Gates / Approvals** — `gate/` (`ExecutionGate`, `GatePipeline`, `LeaseGate`, `GateResolution`, `ResumeKind`), auth/approval resume | Durable approval requests resolved into bounded scoped leases; typed gate/resume with exact invocation identity; deny-continue flow | `contracts/approvals.md`, `contracts/capability-access.md`, `contracts/run-state.md`; plan `docs/plans/2026-06-15-reborn-approval-deny-continue.md` | `ironclaw_approvals`, `ironclaw_run_state` | `tests/reborn_approval_traces_parity.rs`, `tests/integration/auth_failure.rs`, `crates/ironclaw_reborn_composition/tests/budget_approval_e2e.rs`, `crates/ironclaw_reborn_composition/src/factory/local_dev_host_tests/approval_gates.rs` | Covered (note 3) |
| **CodeAct / Tier 1** — embedded Python via Monty (RLM): context-as-variables, `llm_query()` recursive subagent, compact output metadata; `executor/scripting.rs` | Two parts: (a) native script/software execution lane (`RuntimeKind::Script`); (b) CodeAct is an *allowed* pluggable parent loop family, and recursive subagents exist as `spawn_subagent` | `contracts/scripts.md`, `contracts/agent-loop-protocol.md` (CodeAct as parent protocol) | `ironclaw_scripts`, `ironclaw_agent_loop`, `ironclaw_process_sandbox` | `tests/integration/process_port.rs`, `tests/reborn_subagent_spawn_e2e.rs`, `crates/ironclaw_reborn_composition/tests/subagent_runtime_wiring.rs` | Partial (note 4) |
| **Self-modify** — prompt overlays / orchestrator patches applied by the self-improvement mission; skill versioning/rollback (`memory/skill_tracker.rs::SkillTracker`) | Versioned skill evolution (distill/refine with validation); overlay/orchestrator self-patching intentionally not carried over (Reborn has no Monty orchestrator to patch) | plan `docs/plans/2026-06-16-reborn-skill-evolution.md`; `contracts/skills-extension.md` | `ironclaw_skills::learning`, `ironclaw_skills` | `crates/ironclaw_skills/src/learning.rs` (refine/distill tests) | Partial (note 4) |
| **Per-project Docker sandbox** — filesystem/shell tools routed through a per-project container (`SANDBOX_ENABLED`; `crates/Dockerfile.sandbox`) | Typed process plans, backend-neutral sandbox backends, hardened Docker command construction, fail-closed network-host validation, timeout/cancel cleanup | `contracts/processes.md`, `contracts/scripts.md`; design `docs/reborn/2026-05-26-docker-process-sandbox-mvp.md` | `ironclaw_process_sandbox`, `ironclaw_processes`, `ironclaw_wasm` | `tests/integration/process_port.rs`, `crates/ironclaw_reborn_composition/tests/production_runtime_automations.rs` | Covered (note 5) |
| **OpenAI-compatible Responses API** — engine v2's OpenAI-compatible ingress | Contract-first, ProductWorkflow-backed Chat Completions + Responses (create/retrieve/cancel), idempotency/opaque-ref, projection-backed SSE streaming | `contracts/openai-compatible-api.md` | `ironclaw_reborn_openai_compat` | `crates/ironclaw_reborn_openai_compat/tests/responses_workflow_handlers_contract.rs`, `.../chat_workflow_handlers_contract.rs`, `.../streaming_handlers_contract.rs`, `.../error_contract.rs`, `.../ref_store_contract.rs` | Covered (note 6) |
| **Skills** — trusted/installed skills, activation criteria, `skill_*` tools | First-party in-process skills extension: portable `SKILL.md` bundles; kernel owns trust/visibility/leases/context injection; catalog-first model-selected activation | `contracts/skills-extension.md` | `ironclaw_skills` | `tests/integration/group_extensions/` (suite), `crates/ironclaw_reborn_cli/tests/smoke.rs` | Covered |
| **Hooks** — lifecycle hooks (6 points); `Hook` trait | Host-mediated hook execution with multi-backend persistence and an adversarial parity oracle | (hooks are exercised through `contracts/extensions.md` + capability dispatch) | `ironclaw_hooks`, `ironclaw_hooks_libsql`, `ironclaw_hooks_postgres`, `ironclaw_hooks_parity` | `crates/ironclaw_reborn/tests/hooks_integration.rs`, `crates/ironclaw_hooks_parity/tests/parity_matrix.rs`, `crates/ironclaw_hooks_parity/tests/multi_host_adversarial.rs`, `crates/ironclaw_reborn_composition/tests/third_party_hook_projection.rs` | Covered |
| **Extensions** — installed extension/channel lifecycle | Host-mediated extension registry + first-party extension ports; product adapters (Telegram/Slack v2, GSuite, MCP-hosted) | `contracts/extensions.md`, `contracts/product-adapters.md`, `contracts/mcp.md` | `ironclaw_extensions`, `ironclaw_first_party_extensions`, `ironclaw_product_adapters`, `ironclaw_mcp` | `tests/reborn_adapter_installation_scope_isolation_parity.rs`, `tests/integration/mcp.rs`, `crates/ironclaw_reborn_cli/tests/extension.rs`, `crates/ironclaw_reborn_composition/tests/gsuite.rs` | Covered |
| **Effect execution / tool dispatch** — `traits/effect.rs::EffectExecutor`, `ThreadExecutionContext` | Composition-only `RuntimeDispatcher::dispatch_json` selecting a `RuntimeAdapter` per `RuntimeKind`, normalized results, fail-closed | `contracts/dispatcher.md`, `contracts/capabilities.md` | `ironclaw_dispatcher`, `ironclaw_capabilities` | `tests/reborn_minimal_dispatch_parity.rs`, `tests/reborn_tool_param_coercion_parity.rs`, `tests/reborn_trace_core_builtin_tools_parity.rs`, `tests/reborn_trace_file_tools_parity.rs`, `tests/reborn_trace_wasm_github_fixture_parity.rs` | Covered |
| **Events + Projections** — `ThreadEvent`, `EventKind` (18 variants), event sourcing from day one; `types/event.rs` | Explicit boundary between realtime delivery, durable audit/history, transcript milestones, and derived projections; durable event store | `contracts/events.md`, `contracts/events-projections.md` | `ironclaw_events`, `ironclaw_event_projections`, `ironclaw_event_streams`, `ironclaw_reborn_event_store` | `crates/ironclaw_reborn_event_store/tests/durable_event_store_contract.rs`, `.../filesystem_event_log_contract.rs`, `.../coalescing_sink_contract.rs`, `crates/ironclaw_reborn/tests/loop_milestone_event_projection.rs` | Covered |
| **`LlmBackend` trait** — `complete(messages, actions, config)`; host wraps `LlmProvider` | Host-managed model requests / tool-capable model gateway; provider-safe tool projection (dotted `CapabilityId` ↔ provider names) | `contracts/host-api.md`, `contracts/runtime-workflows.md` | `ironclaw_llm`, `ironclaw_reborn` (model routes/gateway) | `crates/ironclaw_reborn/tests/model_routes.rs`, `crates/ironclaw_reborn/tests/llm_gateway.rs` | Covered |
| **`Store` trait** — 20-method Thread/Step/Event/Project/Doc/Lease/Mission CRUD; host wraps `Database` (PG + libSQL) | Durable stores over the scoped-filesystem substrate + PostgreSQL/libSQL, with backend-parity readiness diagnostics | `contracts/storage-placement.md`, `contracts/turn-persistence.md`, `contracts/run-state.md` | `ironclaw_run_state`, `ironclaw_reborn_event_store`, `ironclaw_memory_native`, `ironclaw_conversations` | `crates/ironclaw_reborn_composition/tests/postgres_substrate.rs`, `.../libsql_substrate.rs`, `crates/ironclaw_conversations/tests/filesystem_store_contract.rs` | Covered |
| **`WorkspaceReader` trait** — read-side workspace access; `traits/workspace.rs`, `workspace/` mounts (`MountBackend`, `ProjectMounts`) | Scoped filesystem substrate + memory read side; per-project mounts realized by the process sandbox bind-mount path | `contracts/filesystem.md`, `contracts/memory.md` | `ironclaw_filesystem`, `ironclaw_memory` | `crates/ironclaw_reborn_composition/tests/libsql_substrate.rs`, `tests/reborn_trace_file_tools_parity.rs` | Covered |
| **ConversationManager / ConversationSurface** — routes UI messages to threads; `runtime/conversation.rs` | Conversation binding: inbound routing to the correct run/thread with scope isolation | `contracts/conversation-binding.md`, `contracts/communication-delivery-resolution.md` | `ironclaw_conversations`, `ironclaw_outbound` | `tests/reborn_direct_chat_user_scope_isolation_parity.rs`, `tests/reborn_outbound_reply_target_scope_isolation_parity.rs`, `crates/ironclaw_conversations/tests/inbound_contract.rs` | Covered |
| **Context builder + compaction** — `executor/context.rs`, `executor/compaction.rs` (context assembly, compaction near model limit) | Loop context strategies + context-compaction design; prompt envelope assembly | design `docs/reborn/2026-05-26-context-compaction.md`; `contracts/turns-agent-loop.md` | `ironclaw_agent_loop` (`strategies/context.rs`), `ironclaw_prompt_envelope` | `crates/ironclaw_agent_loop/src/executor/tests.rs`, `crates/ironclaw_reborn_composition/src/runtime/tests/default_system_prompt.rs` | Covered |
| **ThreadTree / sub-agents** — parent-child relationships; `runtime/tree.rs` | Subagent spawn through capability authorization/dispatch, wired into the runtime | `contracts/agent-loop-protocol.md` (`spawn_subagent`) | `ironclaw_agent_loop`, `ironclaw_reborn_composition` | `tests/reborn_subagent_spawn_e2e.rs`, `crates/ironclaw_reborn_composition/tests/subagent_runtime_wiring.rs` | Covered |
| **ReliabilityTracker** — per-action success-rate + latency via EMA; `reliability.rs` | No direct EMA reliability/estimation service; `ironclaw_observability` is a minimal event/metric sink, not a per-action learning tracker | (none dedicated) | `ironclaw_observability` (partial) | — | Gap (note 7) |

## Notes

1. **Memory (Covered).** The `MemoryService` contract and native provider carry
   the full document lifecycle and hybrid retrieval that engine v2's
   `MemoryStore`/`RetrievalEngine` provided. The
   legacy-vs-reborn comparison (`docs/internal/2026-06-26-legacy-vs-reborn-feature-comparison.md`,
   "Memory/workspace" row) notes that not every *legacy product* memory UX is
   carried over — that is a legacy-product delta, not an engine-v2 primitive
   delta. The `MemoryDoc` primitive itself (typed durable docs incl. skills) is
   fully represented.

2. **Missions / learning missions (Partial).** Engine v2's `MissionManager`
   bundled two concerns: (a) cadence-driven goal execution and (b) four
   event-driven learning missions that auto-fire on thread completion. Reborn
   splits these cleanly — `ironclaw_triggers` owns scheduled cadence (routed
   through the normal turn pipeline), and `ironclaw_skills::learning` owns
   skill extraction/repair. Marked **Partial** because: the automatic
   post-completion firing of *all four* learning missions is not yet a single
   wired-up manager, and the comparison doc's "Automation/routines" and
   "Notable Gaps → Automation production readiness" rows flag external result
   delivery, readiness policy, active-run retention, and jitter as open
   follow-ups. The underlying primitives (scheduled fire, learning
   distill/refine) exist and are tested.

3. **Approvals (Covered, with a tracked epic).** The gate/approval/resume
   architecture is fully present and tested. The comparison doc marks product
   approval *parity* as an ongoing epic (#4539), and the cutover closeout
   (`docs/reborn/production-cutover-readiness-closeout.md`) records that #3026
   covers production wiring for approval request/lease stores with fail-closed
   readiness. The engine-v2 *capability* (typed gates, lease resume, deny) is
   covered; residual work is product-UX parity, not a missing primitive.

4. **CodeAct / self-modify (Partial).** This is the most substantive delta.
   Engine v2 Tier 1 embedded a Python interpreter (Monty) inside the loop with
   the RLM pattern: context-as-variables, `llm_query()` recursive subagent
   calls, and compact inter-step output metadata. Reborn does **not** ship an
   equivalent in-loop Monty orchestrator. Instead it provides (a) a native
   script/software execution lane (`ironclaw_scripts`, `RuntimeKind::Script`)
   sandboxed via `ironclaw_process_sandbox`, and (b) an architecture where
   CodeAct is an explicitly *allowed pluggable parent loop family*
   (`contracts/agent-loop-protocol.md`) rather than a hardcoded tier. The most
   valuable RLM sub-feature — recursive subagents — is realized as
   `spawn_subagent` and is tested (`reborn_subagent_spawn_e2e.rs`).
   Correspondingly, engine v2's *self-modify* that patched the Python
   orchestrator/prompt overlays has no analog because there is no Monty
   orchestrator to patch; versioned skill evolution
   (`ironclaw_skills::learning`) carries the durable-knowledge half. Net: the
   engine-v2 CodeAct primitive is reshaped, not reproduced byte-for-byte. This
   is acceptable for removal because engine-v2 CodeAct was itself experimental
   (Tier 1, RLM), never a production default, and the `.claude/rules/tool-evidence.md`
   note records that even its side-effect gating was only a soft nudge, never a
   hard gate.

5. **Sandbox (Covered).** The per-project Docker sandbox is reproduced as typed
   process plans over backend-neutral sandbox backends with hardened command
   construction and fail-closed network-host validation. The comparison doc
   marks production MITM/product wiring as still partial, but the engine-v2
   capability (route filesystem/shell effects through a per-project container)
   is present and tested.

6. **OpenAI-compatible API (Covered for the engine-v2 surface).** Reborn ships
   contract-first, ProductWorkflow-backed Chat Completions and the Responses
   API (create/retrieve/cancel) with opaque-ref/idempotency and
   projection-backed SSE streaming — deliberately *not* reusing the v1
   stateless proxy path. The comparison doc's "OpenAI-compatible API" row
   labels the *broader legacy gateway API* as Missing/Partial in Reborn; that
   refers to the full control-plane API surface, not the Responses/Chat
   ingress that engine v2 exposed, which is present.

7. **ReliabilityTracker (Gap).** Engine v2's per-action EMA success-rate and
   latency tracker has no dedicated Reborn equivalent; `ironclaw_observability`
   is currently a minimal single-file event/metric sink. This is a low-impact
   gap: the tracker was an internal heuristic feeding capability selection, not
   a user-facing capability, and it has no persisted-contract or wire surface
   that removal of engine v2 would strand. If per-action reliability weighting
   is desired in Reborn it should be a new observability/estimation slice, not
   a blocker for engine-v2 removal.

## Known Gaps / Follow-ups

The only items that are not **Covered** are:

- **CodeAct in-loop Monty/RLM orchestrator** — Partial (note 4). Reshaped into
  a pluggable loop family + native script lane + `spawn_subagent`. No durable
  contract stranded by removal.
- **Unified learning-mission auto-firing** — Partial (note 2). Primitives exist
  (`ironclaw_triggers`, `ironclaw_skills::learning`); orchestration + automation
  production readiness are tracked in the comparison doc's "Notable Gaps →
  Automation production readiness".
- **Per-action ReliabilityTracker (EMA)** — Gap (note 7). Internal heuristic
  with no wire/contract surface; safe to drop or re-introduce as a new
  observability slice.

These correspond to entries in the "Notable Gaps Before Reborn Can Replace
Legacy" section of
`docs/internal/2026-06-26-legacy-vs-reborn-feature-comparison.md` (Automation
production readiness; Model/provider feature parity) and to the residual epics
noted in `docs/reborn/production-cutover-readiness-closeout.md` (#4539
approvals parity, #3029 migration). Critically, **none of these gaps are
engine-v2-specific**: they are Reborn-vs-legacy-product deltas that exist
independently of whether engine v2 is present, because engine v2 is gated off
and no shipping path depends on it.

## Conclusion

Every one of engine v2's five primitives (Thread, Step, Capability, MemoryDoc,
Project) and every runtime concern it layered on top (missions,
gates/approvals, sandbox, OpenAI-compatible Responses API, skills/hooks/
extensions, effect dispatch, events/projections, the `LlmBackend`/`Store`/
`EffectExecutor`/`WorkspaceReader` trait boundaries) maps to a named Reborn
contract, one or more dedicated crates, and passing test evidence. Two
capabilities are honestly **Partial** and one is a **Gap** — all three are
internal/experimental engine-v2 features (in-loop Monty CodeAct, unified
learning-mission firing, EMA reliability tracking) with no durable contract,
wire format, or shipping code path that engine-v2 removal would strand.

Because engine v2 is default-off (`ENGINE_V2` false) and Reborn is fully
independent of `ironclaw_engine`, deleting engine v2 removes only dead-by-default
interim code. **Engine v2 can be safely removed.** The recommended follow-ups
(CodeAct loop family productionization, unified learning-mission wiring, and an
optional reliability/estimation observability slice) are net-new Reborn work,
not regressions caused by the removal.

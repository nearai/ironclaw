# ironclaw_loop_host

The concrete implementation of every `ironclaw_loop_contracts` port over kernel
services — the one crate licensed to hold both `Loop*Port` types and kernel
handles in the same module. Since the WS3 sheds it also owns the model-gateway
adapter (the family's single sanctioned provider client), the model-route
policy vocabulary, the driver-host port adapters, progressive tool disclosure,
and the loop tier's system-prompt content assets.

- **Family / layer:** `loop` / `loops` · **Package:** `ironclaw_loop_host` · **Manifest:** `crates/loop/ironclaw_loop_host/Cargo.toml`
- **Use this when:** implementing or decorating a loop port over a kernel or
  domain service; adding prompt-safe context builders; changing tool
  disclosure or model routing.
- **Don't use this when:** deciding what a turn does next → `ironclaw_agent_loop`;
  composing the decorator chain for a claimed run or registering drivers →
  `ironclaw_turn_runner` (the runner orders the pieces; this crate supplies
  them); policy/audit middleware → `ironclaw_hooks`.

## Public surface

- Base `Loop*Port` adapters: capability port + surface filtering
  (`capability_port.rs`, `capability_surface_filter.rs`,
  `capability_allow_set.rs`), input queue, cancellation, compaction,
  checkpoint store, budget accountant, subagent-spawn port.
- Model gateway (`model_gateway.rs`, `thread_resolving_model_gateway.rs`) and
  the `model_routes.rs` policy vocabulary (`ModelRoute`, `ModelSlot`,
  `ModelRouteResolver`, …).
- Driver-host port adapters (`driver_host_port_adapters.rs`):
  `HostManagedLoopCheckpointPort`, `HostManagedLoopProgressPort`,
  `NoExtraLoopInputPort`.
- Progressive tool disclosure (`tool_disclosure*.rs`): catalog/selector, the
  deferring `LoopCapabilityPort` decorator, the `REBORN_TOOL_DISCLOSURE`
  switch.
- Tool retrieval (`tool_search.rs`): the crate-private
  `NativeBm25fToolRetrieval`, the host-bundled bounded BM25F ranker and the
  default binding of the `ToolRetrievalProvider` port in
  `ironclaw_loop_contracts`. A deployment binds another through
  `ToolDisclosureCapabilityDecorator::with_retrieval_provider`, which the
  runtime build calls when `DefaultPlannedRuntimeParts::tool_retrieval_provider`
  (composition: `RebornRuntimeInput::tool_retrieval_provider`) is set; the
  production alternatives are the opt-in dense ranker in
  `crates/extensions/packages/tool-retrieval` (`REBORN_TOOL_RETRIEVAL=dense`)
  and `HybridToolRetrieval` (`hybrid_tool_retrieval.rs`,
  `REBORN_TOOL_RETRIEVAL=hybrid`), which fuses BM25F with that dense ranker by
  reciprocal rank and returns the unchanged BM25F result for any search whose
  dense side is absent, failing, or past its time bound. The
  disclosure port never fits or searches under its turn-state lock, repairs
  provider output that breaks the port contract before using it, and turns a
  ranker failure into a model-visible `tool_search` failure.
- Turn-start tool selection (`tool_prefetch.rs`, `REBORN_TOOL_PREFETCH`):
  ranks the whole authorized catalog, core tools included, against a
  conversation's opening request and advertises the best-ranked tools up to
  `max_tools` plus an always-on floor (the three bridges and `result_read`).
  The local classifier ranks each user message separately (a long one in
  segments, `ConversationContext::segments`) with one `search_many` call on
  one fitted index, and merges the rankings round-robin by rank, so every
  topic gets its best tools; score thresholds are optional and off by default.
  The result is the `initial` entry of the conversation's append-only
  selection history in `ironclaw_threads`; while the provider's prompt cache
  could be warm, every later call rebuilds the byte-identical `tools` array
  from the entry in force and never ranks again, and nothing is promoted.
  With re-selection on (`ToolReselectionConfig`, `tool_prefetch/reselect.rs`)
  a turn boundary chooses again from a window of the conversation after an
  idle gap past the cache lifetime (`cache_cold`) or a model change
  (`model_change`), keeping every tool the conversation called; a revocation
  re-selects too (`revoked`), and without re-selection only removes the
  tool. The idle clock and last model are read from the conversation's
  durable `ToolSelectionActivity`, which `ThreadBackedLoopModelPort` writes
  after each call (`with_model_call_recording`); the run's model and cache
  lifetime come from a `PromptCacheProfileSource`
  (`GatewayPromptCacheProfiles` over the model gateway). Bound through
  `ToolDisclosureCapabilityDecorator::with_tool_prefetch`. The choice itself
  goes through the `ToolSelectionClassifier` port: the ranker and merge
  are the bundled `RankingToolClassifier` (`tool_prefetch/local_classifier.rs`);
  `ToolPrefetchConfig::with_classifier` binds another one instead, whose
  answer the host checks against the candidates and caps before adding the
  floor. A local failure keeps the ordinary surface unrecorded; a bound
  classifier's failure at the first selection records the core tool set
  (authorized `CORE_TOOL_NAMES` plus the floor) with the failure's label.
- Prompt-context builders (`identity_context.rs`, `skill_context.rs`) and
  `skill_activation/` (the dissolved `ironclaw_first_party_extension_ports`
  crate, WS8).
- `system_prompt_assets.rs` + `prompts/*.md` — the system-prompt *content*
  (composition keeps assembly and on-disk `SYSTEM.md` seeding, never the
  text).

## Depends on / consumed by

- **Normal workspace deps (17):** contracts (`ironclaw_common`,
  `ironclaw_host_api`, `ironclaw_loop_contracts`), kernel services the
  adapters wrap (`ironclaw_capabilities`, `ironclaw_host_runtime`,
  `ironclaw_turns`, `ironclaw_processes`, `ironclaw_approvals`,
  `ironclaw_resources`), domains/substrates the context builders need
  (`ironclaw_filesystem`, `ironclaw_memory`, `ironclaw_observability`,
  `ironclaw_outbound`, `ironclaw_safety`, `ironclaw_skills`,
  `ironclaw_threads`) — and `ironclaw_llm` (`default-features = false`) **for
  the model-gateway adapter alone**, an exception by charter (PROPOSAL
  §6.7.2), not drift.
- **Consumed by (4):** `ironclaw_turn_runner` (composes the base adapters into
  each claimed run's host), `ironclaw_composition` (assembly),
  `ironclaw_extension_host`, and `ironclaw_assistant` — the last is the
  measured `products → loops` debt edge PROPOSAL §6.10.1 scopes (6 files /
  4 seams), not a pattern to extend.

## Invariants

- **No other module may reach a provider client** — the gateway is the single
  adapter; `ironclaw_turn_runner` must not regain `ironclaw_llm`
  (`--test reborn_dependency_boundaries reborn_runner_llm_wiring_is_isolated`).
- **The WS3 shed items are defined here and absent from the runner**
  (`--test reborn_runner_sheds`).
- **`skill_activation/` keeps its old crate boundary as a module-import
  equality** (`--test reborn_dependency_boundaries dissolved_ports_module_keeps_its_crate_boundary`).
- **Prompt content stays out of the composition root**
  (`--test reborn_composition_boundaries composition_root_embeds_no_prompt_content`).
- No decorator performs a turn-lifecycle state transition, and nothing here
  bypasses `CapabilityHost` or dispatcher authority paths.
- Message-backed runs retain their exact accepted user task across both
  message-count and token-budget selection. If that task alone exceeds the
  prompt budget, prompt construction fails instead of silently dropping it.

## Tests

```bash
cargo test -p ironclaw_loop_host
cargo test -p ironclaw_turns -p ironclaw_turn_runner   # when host-port contracts change
cargo test -p ironclaw_architecture_tests              # after dependency/API changes
```

## See also

Family rules: `crates/loop/AGENTS.md` · working rules: `AGENTS.md` beside this
file · design record: `docs/internal/reborn/target-architecture/families/loop.md`
(§6.7.2).

# Agent Map — ironclaw_loop_host

Working rules for the host-port adapter crate. Orientation lives in
`README.md`; family rules in `crates/loop/AGENTS.md`.

## Start Here

- Read `README.md` for what this crate is; read `Cargo.toml` for the
  dependency shape.
- Use these neighboring contracts before changing behavior:
  - `crates/contracts/ironclaw_loop_contracts/` — the ports this crate
    implements.
  - `crates/kernel/ironclaw_turns/AGENTS.md`
  - `crates/kernel/ironclaw_capabilities/AGENTS.md`
  - `crates/domains/ironclaw_skills/AGENTS.md`
  - `crates/loop/ironclaw_turn_runner/AGENTS.md`

## What This Crate Owns

Base implementations of every `ironclaw_loop_contracts` port over host-owned
kernel services — adapter glue, not the executor, runner, product workflow, or
low-level runtime. `src/` has outgrown a hand-maintained file inventory (50+
files, plus 20 `prompts/*.md`); re-derive the current one with
`ls crates/loop/ironclaw_loop_host/src/` and
`ls crates/loop/ironclaw_loop_host/prompts/` rather than trusting a bullet
list. Stable anchors, by category:

- `lib.rs` — crate root; port implementations are wired here.
- `capability_port.rs` / `capability_surface_filter.rs` /
  `capability_surface_policy.rs` — capability-surface adapters, filtering, and
  profile-to-policy resolution (neutral policy vocabulary stays in
  `ironclaw_host_api`).
- `model_gateway.rs` / `model_routes.rs` / `thread_resolving_model_gateway.rs`
  — the model-gateway adapter over `ironclaw_llm::LlmProvider`, the one
  sanctioned provider-client exception in this family (PROPOSAL §6.7.2).
- `tool_disclosure.rs` / `tool_disclosure_port.rs` / `tool_disclosure_mode.rs`
  — progressive tool disclosure and the `REBORN_TOOL_DISCLOSURE` switch.
  `tool_prefetch.rs` — turn-start tool selection. Its `tools` array is part
  of the prompt-cache prefix: while the cache could be warm, never re-rank,
  promote, or add to it; only a revocation may change it. Re-selection
  (`tool_prefetch/reselect.rs`) runs only at a turn boundary after an idle
  gap past the cache lifetime, a model change, or a revocation — never
  mid-turn or on compaction — and every unknown leans towards "warm".
  The choice goes through the `ToolSelectionClassifier` port; a classifier's
  answer is untrusted and the host, not the classifier, adds the floor.
- `skill_activation/` — skill activation selection. **Arrived 2026-08-05
  (CHECKLIST WS8) as the whole of the dissolved
  `ironclaw_first_party_extension_ports` crate** (PROPOSAL §9 row 55); its old
  crate boundary survives as an equality over the module's imports —
  `cargo test -p ironclaw_architecture_tests --test reborn_dependency_boundaries dissolved_ports_module_keeps_its_crate_boundary`
  — so this module may reach only `host_api`, `loop_contracts`, `filesystem`,
  `skills`, `turns`, and this crate, even though `loop_host` itself may reach
  far more.
- `system_prompt_assets.rs` + `prompts/*.md` — the system-prompt *content*
  (`DEFAULT_SYSTEM_PROMPT` seed text plus the protocol appendices resolved at
  runtime). Text lives in `prompts/*.md`, never inline in Rust. Evicted from
  the composition root, which owns assembly and boot-time seeding of the
  on-disk `SYSTEM.md`, never the text (PROPOSAL §6.10.1). Prompt text that
  names a tool follows the run's advertised tools (#7836): the context and
  model ports hand each run's `AdvertisedTools` (from the host-build surface)
  to the identity and skill sources, and `tool_naming_sections` picks the
  system-prompt sections. Gating applies only to a turn-start selected
  surface, so prompts with selection off stay byte-identical. Find the gates
  with `rg -n "may_name\(" crates`.

Everything else in `src/` (budget/compaction accounting, subagent prompt/spawn
ports, tool search, context-window caching, model-visible output scrubbing,
skill-bundle sources, input/cancellation ports, …) is real and current but not
enumerated above — trust `ls src/`, not a stale list.

## Do Not Move In Here

- Core loop strategy or runner state transitions — an adapter must not
  perform a turn-lifecycle transition; that belongs to the turn-admission
  seam.
- The runner's decorator *composition* — `families/loop.md` assigns "composes
  that base adapter into the concrete host it hands to each claimed run" to
  `ironclaw_turn_runner`, not here. This crate supplies the pieces; the runner
  orders them.
- Product workflow composition, runtime lane execution, or Reborn app wiring —
  no dispatcher internals, product binding, DB migrations, or driver
  registration.
- Bypasses around `CapabilityHost` or dispatcher authority paths.
- Full prompt content where safe summaries/refs are required.
- A second provider client. The model gateway is the one sanctioned
  exception, by charter rather than drift — PROPOSAL §6.7.2 assigns the
  model-gateway adapter here explicitly ("a host-port adapter by charter"),
  and `ironclaw_llm` (`default-features = false`) is a normal dependency for
  that adapter alone. No other module may reach a provider client.

## Adding code

- Add one file per host adapter or context source.
- Add decorators (e.g. profile filters) as named types with a single policy
  responsibility.
- Put capability-surface filtering behavior in
  `capability_surface_filter.rs`; put profile-to-policy resolution in
  `capability_surface_policy.rs`. Neutral policy vocabulary belongs to
  `ironclaw_host_api`.
- Add traits here only for host-owned inputs to existing loop ports.
- Do not fold unrelated ports into `lib.rs`.

## Validation

- Fast local check: `cargo test -p ironclaw_loop_host`
- Boundary check after dependency/API changes:
  `cargo test -p ironclaw_architecture_tests`
- Run `cargo test -p ironclaw_turns` and `cargo test -p ironclaw_turn_runner`
  when host-port contracts change.
- `cargo test -p ironclaw_architecture_tests --test reborn_runner_sheds` pins
  what moved here in WS3 and what may not come back.
- `cargo test -p ironclaw_architecture_tests --test reborn_composition_boundaries composition_root_embeds_no_prompt_content`
  pins prompt content out of the composition root; moving a prompt asset back
  there fails it.

# Agent Map — ironclaw_loop_host

## Start Here

- Read `CLAUDE.md` first; it is the crate-local guardrail file.
- Read `Cargo.toml` for actual dependencies and feature shape.
- Use these neighboring contracts before changing behavior:
  - `crates/ironclaw_turns/AGENTS.md`
  - `crates/ironclaw_capabilities/AGENTS.md`
  - `crates/ironclaw_skills/AGENTS.md`
  - `crates/ironclaw_runner/CLAUDE.md`

## What This Crate Owns

- Loop host support services for `AgentLoopHost` / `ironclaw_turns` loop ports.
- `skill_context.rs` and `identity_context.rs` prompt-safe instruction/context builders.
- `capability_port.rs`, `capability_surface_filter.rs`, and
  `capability_surface_policy.rs` capability-surface adapters.
- `input_queue.rs` / `input_port.rs` steering and followup queues.
- `cancellation_port.rs` cancellation observation adapter.
- `model_gateway.rs` / `model_routes.rs` / `thread_resolving_model_gateway.rs` — the model-gateway adapter over `ironclaw_llm` and the route policy it resolves (absorbed from `ironclaw_runner`, PROPOSAL §6.7.2).
- `driver_host_port_adapters.rs` — checkpoint/progress/no-extra-input port adapters for a claimed run.
- `tool_disclosure.rs` / `tool_disclosure_port.rs` / `tool_disclosure_mode.rs` — progressive tool disclosure: catalog, `LoopCapabilityPort` decorator, and the `REBORN_TOOL_DISCLOSURE` switch.
- `skill_bundle_source.rs` / `filesystem_skill_bundle_source.rs` skill-bundle source ports (`SkillBundleSource`, `FilesystemSkillBundleSource`, `SkillBundleDescriptor`/`SkillBundleId`/`SkillBundleProvenance`).
- `system_prompt_assets.rs` + `prompts/{default_system,tool_disclosure_protocol,self_knowledge,benchmarking_mode}.md` — the system-prompt *content* (seed text for the user-editable `SYSTEM.md`, plus the three protocols appended at resolve time). Evicted from the composition root, which owns assembly and the boot-time seeding, not prompt text (PROPOSAL §6.10.1).

## Do Not Move In Here

- Core loop strategy or runner state transitions.
- The runner's decorator *composition* — `families/loop.md` assigns "composes that base adapter into the concrete host it hands to each claimed run" to the turn runner, not here. This crate supplies the pieces; the runner orders them.
- Product workflow composition, runtime lane execution, or Reborn app wiring.
- Bypasses around `CapabilityHost` or dispatcher authority paths.
- Full prompt content where safe summaries/refs are required.

## Validation

- Fast local check: `cargo test -p ironclaw_loop_host`
- Boundary check after dependency/API changes: `cargo test -p ironclaw_architecture`
- Run `cargo test -p ironclaw_turns` and `cargo test -p ironclaw_runner` when host-port contracts change.
- `cargo test -p ironclaw_architecture --test reborn_runner_sheds` pins what moved here in WS3 and what may not come back.
- `cargo test -p ironclaw_architecture --test reborn_composition_boundaries composition_root_embeds_no_prompt_content` pins prompt content out of the composition root; moving a prompt asset back there fails it.

## Agent Notes

- Add one file per host adapter or context source.
- Put capability filtering behavior in `capability_surface_filter.rs` and
  profile-to-policy resolution in `capability_surface_policy.rs`; the neutral
  policy vocabulary belongs to `ironclaw_host_api`.
- Add traits here only for host-owned inputs to existing loop ports.
- Do not fold unrelated ports into `lib.rs`.

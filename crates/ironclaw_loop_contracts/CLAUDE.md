# ironclaw_loop_contracts guardrails

The loop-tier contract: how any loop, hook, or host adapter talks to the turn
kernel **without importing it**. Layer `contracts`. Specified by PROPOSAL
§6.1.4 and `docs/reborn/target-architecture/families/contracts.md`.

This crate exists so `ironclaw_agent_loop` — the one artifact meant to be
replaced wholesale without touching anything privileged — can satisfy its
"contracts-layer dependencies only" rule with **zero** layer-matrix exceptions.
Every rule below serves that.

## What this crate owns

- The eleven `Loop*Port` traits — capability, model, prompt, transcript,
  context, input, run-info, cancellation, compaction, progress, checkpoint —
  plus the blanket `AgentLoopDriverHost` that exposes them together.
- `AgentLoopDriver` and the run-profile vocabulary: resolved profile shape,
  capability surface, context and checkpoint policy, prompt/model contract
  types, instruction/memory/skill context refs.
- `LoopExit` and its evidence-reference DTOs — the **claim** a loop makes about
  how its turn ended.
- `RedactedCheckpointPayload` and its ceiling; loop-side error and safe-summary
  vocabulary.

## Boundaries

- **The direction inverts, always.** Nothing here may depend on
  `ironclaw_turns`. The turn kernel implements and validates *against* these
  contracts; a dependency the other way would make the whole crate pointless.
  Pinned by the `ironclaw_loop_contracts` allowlist in
  `reborn_dependency_boundaries.rs` (§11.2.3).
- **Internal dependencies:** `ironclaw_host_api`, `ironclaw_common`,
  `ironclaw_prompt_envelope`. Nothing else, ever — the allowlist forbids every
  other workspace crate rather than naming today's offenders.
- **No framework, driver, or runtime client** — no axum, reqwest, wasmtime,
  libsql, deadpool, tokio-postgres, sqlx. Pinned by
  `reborn_contracts_crates_hold_no_framework_dependencies`. `tokio` is present
  with the `rt` feature only, for the one `JoinHandle` in
  `CommunicationContextFetch::Spawned`; that carve-out is documented in the
  manifest and in the test, and widening it is a deliberate edit.
- **No implementation of a port declared here.** `LoopExit` validation, the
  exit applier, the coordinator, and the state store are turn-kernel authority.
  `Loop*Port` implementations live in `ironclaw_loop_host`, `ironclaw_runner`,
  and `ironclaw_hooks` — the single declared decorator chain
  (`families/loop.md`).
- **No prompt content, no model-gateway implementation.** (One inherited
  exception, called out under "Known debt" below.)
- Public/wire DTOs carry refs, bounded safe summaries, typed ids, versions,
  cursors, and sanitized errors. Transient prompt-construction types may carry
  bounded, host-approved model-visible content, but must not be serde
  wire/public DTOs. Raw prompt text, raw assistant content, tool input JSON,
  secrets, host paths, and backend errors stay behind host implementations.

## Turn vocabulary is not ours

`TurnId`/`TurnRunId`/`TurnScope`/`TurnActor`, the gate/message/result refs,
`TurnStatus`, `RunProfileId`/`RunProfileVersion`/`RunProfileRequest`,
`ProductTurnContext`, `GateResumeDisposition` — all of these live in
`ironclaw_host_api::turn`. Import them from there. This crate deliberately does
**not** re-export them: one type, one import path.

## Adding code

- Add a field only when every host implementation can honor the neutral
  contract; keep defaults fail-closed when a concrete host has not implemented
  a new capability yet.
- Add a port trait when the loop needs a new host-owned capability — and add
  its row to `LOOP_PORT_OWNERS` in
  `crates/ironclaw_architecture/tests/reborn_loop_port_location_scan.rs` in the
  same change. The scan fails on an unowned port by design.
- Add a new file when a contract has a separate lifecycle or validation model.
  No `common`, `misc`, or `helpers` modules.
- Ports are **not** sealed: they exist to be implemented by crates above this
  one. The sealed-trait pattern belongs to `ironclaw_agent_loop`'s strategy
  slots (`crates/ironclaw_agent_loop/src/planner.rs`), which stay there.

## Common mistakes

- Importing a lower runtime crate to make a contract convenient.
- Putting production adapter wiring in profile resolution.
- Re-exporting a `Loop*Port` from another crate — that is the §11.2.4
  re-export-path trap, and `reborn_loop_port_location_scan` fails on it.
- Making a safe-summary field carry raw content by convention.

## Known debt (recorded, not accepted)

- `instruction_bundle.rs` embeds one prompt asset
  (`prompts/capability_surface_usage_policy.md`) through `include_str!`.
  PROPOSAL §6.1.4 says a contracts crate holds no prompt content; the module
  moved here anyway because `ironclaw_hooks` consumes
  `InstructionMaterializationStore` and leaving it in the turn kernel would
  have kept the `hooks → turns` exception alive. The resolution §6.7.2 points
  at is hoisting `InstructionBundleBuilder` (the behavior) to
  `ironclaw_loop_host` and leaving the bundle/store *types* here — a WS4 item,
  not a WS1.2 one.

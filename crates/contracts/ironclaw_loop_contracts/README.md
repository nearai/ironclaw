# ironclaw_loop_contracts

The loop-tier contract: how any loop, hook, or host adapter talks to the turn
kernel **without importing it**. It exists so `ironclaw_agent_loop` — the one
artifact meant to be replaced wholesale without touching anything privileged —
satisfies its "contracts-layer dependencies only" rule with zero layer-matrix
exceptions, and so the turn kernel can evolve its internal state machinery
without touching the loop-side contract. The direction always inverts: the
kernel implements and validates *against* these contracts, never the reverse.

- **Family / layer:** `crates/contracts/` / `contracts` · **Package:** `ironclaw_loop_contracts` · **Manifest:** `crates/contracts/ironclaw_loop_contracts/Cargo.toml`
- **Use this when:** a loop, hook framework, or host adapter needs the port,
  profile, or exit vocabulary of the turn kernel.
- **Don't use this when:** you need turn *identity* vocabulary (`TurnId`,
  `TurnStatus`, refs — → `ironclaw_host_api::turn`), the coordinator/state
  store/exit applier (→ `ironclaw_turns`), or a port *implementation* (→
  `ironclaw_loop_host` / `ironclaw_turn_runner` / `ironclaw_hooks`).

## Public surface

Almost purely trait + DTO, re-exported flat from `src/lib.rs`:

- The eleven `Loop*Port` traits — capability, model, prompt, transcript,
  context, input, run-info, cancellation, compaction, progress, checkpoint —
  plus the blanket `AgentLoopDriverHost`.
- `AgentLoopDriver` and the run-profile vocabulary: `ResolvedRunProfile`,
  resolver/capability-surface/context/checkpoint-policy shapes,
  prompt/model/skill/instruction/milestone contract types.
- `LoopExit` and its evidence-reference DTOs — the *claim* a loop makes about
  how its turn ended; only the kernel validates it into a durable transition.
  Compiler-checked variant guards live beside the `#[non_exhaustive]` enums.
- `RedactedCheckpointPayload` + `MAX_CHECKPOINT_STATE_PAYLOAD_BYTES`;
  loop-side error and safe-summary vocabulary (`AgentLoopHostError*`,
  `LoopSafeSummary`).

## Depends on / consumed by

- **Internal deps (manifest, measured):** `ironclaw_host_api` and
  `ironclaw_extension_contracts` (the latter because `LoopRuntimeContext`
  carries `Option<ChannelPresentation>`). The enforced allowlist
  (`reborn_crate_dependency_boundaries_hold::loop_contracts_allowed`) also
  *permits* `ironclaw_common` and `ironclaw_prompt_envelope`, which the crate
  does not currently use. Never `ironclaw_turns`.
- **External carve-out:** `tokio` with the `rt` feature only, for the single
  `JoinHandle` in `CommunicationContextFetch::Spawned` — documented in the
  manifest, pinned by the framework-deny gate.
- **Consumed by 13 manifests** (reproduce:
  `grep -rl '^ironclaw_loop_contracts = ' --include=Cargo.toml crates Cargo.toml | wc -l`)
  — the loop tier (`agent_loop`, `loop_host`, `turn_runner`, `hooks`), the
  kernel (`turns`, `capabilities`, `host_runtime`), the extension tier
  (`extension_host`, `extension_manager`, `extension_support`), product,
  composition, and the integration-test root.

## Invariants

- **No dependency on the turn kernel, ever** — allowlist in
  `reborn_dependency_boundaries.rs`; the whole crate is pointless otherwise.
- **No implementation of a port declared here** — implementations live in the
  loop-hosting tier; `reborn_loop_port_location_scan.rs` additionally pins one
  definition and one import path per port, and fails on an unowned port (add
  the `LOOP_PORT_OWNERS` row with the trait).
- **Ports are open, not sealed** — they exist to be implemented above; the
  sealed-strategy pattern belongs to `ironclaw_agent_loop::planner`.
- **Known debt (recorded, not accepted):** `instruction_bundle.rs` embeds one
  prompt asset via `include_str!` against the family's no-prompt-content rule;
  the resolution (hoist `InstructionBundleBuilder` to `ironclaw_loop_host`) is
  a WS4 item — PROPOSAL §6.1.4, CHECKLIST `loop_host` re-charter row.
- **Size ceiling:** `reborn_contracts_crates_carry_a_checked_size_ceiling`.

## Tests

```bash
cargo test -p ironclaw_loop_contracts
cargo test -p ironclaw_architecture_tests       # allowlist, port-location scan, ceilings
```

## See also

- Working rules and traps: [`AGENTS.md`](./AGENTS.md) (canonical crate
  guidance; `CLAUDE.md` points here).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: PROPOSAL §6.1.4;
  `docs/reborn/target-architecture/families/contracts.md`; the port
  implementation chain in `docs/reborn/target-architecture/families/loop.md`.

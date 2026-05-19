# ironclaw_turns::run_profile

Owns neutral run-profile and agent-loop host contracts.

## Files

- `driver.rs` owns the runner-facing `AgentLoopDriver` trait, descriptors, run
  and resume requests, and driver errors.
- `host.rs` owns `AgentLoopDriverHost` and `LoopXxxPort` traits plus neutral
  DTOs passed over those ports.
- `resolver.rs` owns profile resolution policy and profile defaults.
- `snapshot.rs`, `refs.rs`, and profile submodules own typed ids, snapshots,
  prompt/model/policy metadata, and instruction/memory/skill context refs.

## Boundaries

- This directory defines contracts only. It must not construct concrete
  capability hosts, dispatchers, host runtime services, workspace readers, DB
  backends, product adapters, or provider clients.
- Port DTOs carry refs, bounded safe summaries, typed ids, versions, cursors,
  and sanitized errors.
- Raw prompt text, raw assistant content, tool input JSON, secrets, host paths,
  and backend errors must stay behind host implementations.

## Adding code

- Add fields here only when every host implementation can honor the neutral
  contract.
- Add a new port trait when the loop needs a new host-owned capability.
- Add a new file when the contract has a separate lifecycle or validation
  model.
- Keep defaults fail-closed when a concrete host has not implemented a new
  capability yet.

## Common mistakes

- Do not import lower runtime crates to make a contract convenient.
- Do not put production adapter wiring in profile resolution.
- Do not add public runner transition shortcuts for product callers.
- Do not make safe-summary fields carry raw content by convention.

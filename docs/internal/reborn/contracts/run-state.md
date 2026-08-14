# Retired run-state contract

The standalone run-state subsystem was retired on 2026-07-26.

- Process and capability-invocation lifecycle state is owned by
  `ironclaw_processes` and projected from the process journal.
- Durable approval requests, decisions, and model-visible gate records are
  owned by `ironclaw_approvals`.
- Turn status is a conversation-facing projection; it is not an independent
  lifecycle authority.

See `docs/internal/reborn/contracts/processes.md` and
`docs/internal/reborn/contracts/approvals.md` for the active contracts.

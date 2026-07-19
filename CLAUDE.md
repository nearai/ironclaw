# IronClaw Development Guide

The supported product runtime is Reborn. The canonical `ironclaw` binary is
owned by `crates/ironclaw_reborn_cli`; the retired root v1 runtime and its
`src/` monolith are intentionally absent.

Start with `AGENTS.md`, then read `crates/AGENTS.md` and the owning crate's
`AGENTS.md`, `CLAUDE.md`, `CONTRACT.md`, or `README.md`. Cross-crate contracts
live under `docs/reborn/contracts/`. Do not hand-edit generated `openwiki/`
content.

## Discovery

Run `bash scripts/codebase-graph.sh status` once for structural questions. Use
the graph when it is fresh and available; otherwise fall back immediately to
the Reborn orientation skill, crate guidance, and targeted `rg`. Always verify
graph results against live code.

The canonical product flow is:

`ironclaw_webui` / product adapters -> `ironclaw_product_workflow` ->
threads and turns -> `ironclaw_runner` -> `ironclaw_agent_loop` ->
`ironclaw_capabilities` -> mediated runtime lanes.

Transports normalize requests and render projections; they do not create
authoritative conversation state or bypass authorization, approvals, or host
mediation.

## Build And Test

```bash
cargo fmt --all -- --check
cargo build -p ironclaw_reborn_cli --bin ironclaw
cargo test -p ironclaw_architecture
cargo test -p ironclaw_reborn_cli
cargo clippy --all --tests --examples --all-features -- -D warnings
```

For behavior changes, follow the test progression and integration-test rules in
`AGENTS.md`, `.claude/skills/ironclaw-reborn-testing/SKILL.md`, and
`.claude/rules/review-discipline.md`.

## Ownership Rules

- Product UI and HTTP transport belong in `crates/ironclaw_webui`.
- Product-facing facades belong in `crates/ironclaw_product_workflow`.
- Composition wires typed ports; it does not own domain policy.
- Capability execution crosses authorization, approvals, obligations, host
  mediation, and the selected runtime lane.
- Persistence uses `RootFilesystem` / `ScopedFilesystem` and bounded CAS where
  a backend transaction is unavailable.
- Prompt templates live in the owning crate's `prompts/` directory and are
  loaded with `include_str!`.
- Shared types live with the contract owner; do not create mirror DTOs or use
  `ironclaw_common` as a general dumping ground.

## Safety Rules

- No `.unwrap()` or `.expect()` in production code.
- Keep clippy at zero warnings.
- Treat every ingress, runtime lane, extension, and external service as
  untrusted until a typed boundary establishes otherwise.
- Do not weaken auth, origin checks, limits, allowlists, secret mediation,
  approval leases, redaction, or authoritative side-effect evidence.
- LLM and durable event data is never silently discarded.
- New behavior must test through the production caller at a meaningful seam.

The architecture guardrails intentionally fail if the root `ironclaw` package,
legacy package layers, retired v1 crates, or root runtime sources are
reintroduced.

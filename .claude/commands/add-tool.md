---
description: Add a mediated Reborn capability or extension tool
allowed-tools: Read, Edit, Write, Glob, Grep, Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*)
argument-hint: <tool_name> [description]
model: opus
---

Add `$ARGUMENTS` through the supported Reborn capability path.

1. Decide ownership before coding. Declarative extension metadata belongs in
   `ironclaw_extensions`; execution belongs in a runtime lane behind host
   mediation. Core host behavior uses a typed built-in capability through the
   same `CapabilityHost` surface.
2. Reuse an existing extension/runtime library and manifest pattern. External
   HTTP must cross `ironclaw_network`; credentials remain host-side.
3. Keep discovery side-effect free. Installation, credential binding,
   activation, execution, deactivation, and removal are explicit transitions.
4. Route execution through authorization, approvals, obligations, dispatch,
   and the selected runtime lane. Never add a direct side-effect path.
5. Add a production-seam integration test, plus manifest/schema tests where
   applicable. Side-effecting success needs durable/provider evidence and
   read-back verification.
6. Run owning-crate tests, architecture tests for dependency changes, formatting,
   and clippy with warnings denied.

Start from `.claude/skills/reborn-feature/SKILL.md` and verify live ownership
with `crates/AGENTS.md` and targeted `rg` before editing.

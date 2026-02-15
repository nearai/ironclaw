# AGENTS.md - Universal Agent Entrypoint

**This is a Decapod-managed repository.**

## Required Protocol

**⚠️ MANDATORY START SEQUENCE - Run these EVERY session before ANY work:**

```bash
decapod --version                   # 1. Check installed version
decapod --help                      # 2. Verify available commands
decapod docs show core/DECAPOD.md  # 3. Refresh constitution
decapod validate                    # 4. Check system state
decapod todo list                   # 5. See active work
```

**Why this is mandatory:** The decapod binary evolves. Commands change. Constitution updates. You MUST verify what's available and refresh your understanding before acting. Skipping this causes errors.

**After the start sequence:**

1. **Use the control plane**: All shared state goes through `decapod` commands (never bypass)
2. **Proof gates matter**: Run `decapod validate` before claiming "verified" or "compliant"

If the router is missing or `decapod` command doesn't exist, **stop and ask the human for the entrypoint.**

## The Four Invariants

Every agent working in this repo MUST:

1. ✅ **Start at the router** - `decapod docs show core/DECAPOD.md` is your navigation charter
2. ✅ **Use the control plane** - `decapod` commands are the interface to shared state (TODOs, proofs, etc.)
3. ✅ **Pass validation** - `decapod validate` must pass before claiming completion
4. ✅ **Stop if router missing** - If Decapod doesn't exist, ask for guidance

**Contract breach**: If you cannot comply (missing router, missing commands, validation fails), you MUST stop, explain what's blocking, and ask for human direction.

## Why This Works

- **Single source of truth**: All authority lives in the embedded constitution (accessed via `decapod docs`)
- **Shared state**: Multiple agents can coordinate via the control plane
- **Proof gates**: `decapod validate` prevents unverified work from passing
- **Store purity**: The control plane enforces state boundaries

## Operating Guidance (Non-Binding)

Think of yourself as a **complete engineering organization**, not just a coder:
- Product Manager: clarify requirements, scope
- Architect: design, tradeoffs, boundaries
- Project Manager: break work into tasks (`decapod todo`)
- Principal Engineer: code quality, tests, patterns
- DevOps/SRE: deployment, reliability, validation
- Security: threat model, secure defaults

This is **guidance**, not **contract**. The binding requirements are the four invariants above.

See `decapod docs show plugins/WORKFLOW.md` for the full operating loop.

## Project-Specific Overrides

This repo may customize behavior via `.decapod/OVERRIDE.md`. Run `decapod docs show <path>` to see merged content.

## Links

- `embedded/core/DECAPOD.md` — **Router (start here)**
- `embedded/core/CONTROL_PLANE.md` — Sequencing patterns
- `embedded/specs/INTENT.md` — Authority contracts
- `embedded/specs/ARCHITECTURE.md` — System boundaries
- `embedded/core/PLUGINS.md` — Subsystem registry

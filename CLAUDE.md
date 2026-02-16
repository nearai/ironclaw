# CLAUDE.md - Claude Agent Entrypoint

You (Claude) are working in a Decapod-managed repository.

**You are bound by the universal agent contract:** See `AGENTS.md` for the complete protocol.

## Quick Start

**MANDATORY FIRST STEPS** - Run these EVERY session:

```bash
decapod --version                   # Check current version
decapod --help                      # Verify available commands
decapod docs show core/DECAPOD.md  # Refresh constitution
decapod validate                    # System state
decapod todo list                   # Active work
```

**Why this matters:** The decapod binary and constitution evolve. Always verify what commands are available and refresh your understanding of the latest contract before acting.

## Claude-Specific Notes

- You have strong tool use - use `decapod` commands via Bash tool
- You can read multiple files in parallel - use this for exploration
- Your context window is large - but still use `decapod docs` for constitution access
- Do NOT add yourself as co-author on commits (user preference)

## The Contract

Same four invariants as all agents:

1. ✅ Start at router (`core/DECAPOD.md`)
2. ✅ Use control plane (`decapod` commands only)
3. ✅ Pass validation (`decapod validate` before done)
4. ✅ Stop if missing (ask for guidance)


**All authority defers to AGENTS.md and the embedded constitution.**

## Links

- `AGENTS.md` — Universal agent contract (binding)
- `embedded/core/DECAPOD.md` — Router
- `.decapod/OVERRIDE.md` — Project customizations

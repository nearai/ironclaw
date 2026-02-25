# Migration: IronClaw/OpenClaw to cLawyer

This guide covers local migration to `cLawyer` with legal defaults.

## Path Changes

- Old settings path: `~/.ironclaw/settings.json`
- New settings path: `~/.clawyer/settings.json`
- New default TOML: `~/.clawyer/config.toml`
- New bootstrap env: `~/.clawyer/.env`

`cLawyer` keeps legacy read fallbacks for existing `~/.ironclaw/*` settings/env files.

## Binary and Commands

- Old command: `ironclaw`
- New command: `clawyer`

Examples:

```bash
clawyer onboard
clawyer run
clawyer config list
```

## New Legal Config Blocks

Add/update:

```toml
[legal]
enabled = true
jurisdiction = "us-general"
hardening = "max_lockdown"
require_matter_context = true
citation_required = true
matter_root = "matters"

[legal.network]
deny_by_default = true
allowed_domains = []

[legal.audit]
enabled = true
path = "logs/legal_audit.jsonl"
hash_chain = true
```

## Skill Trust Changes

- Bundled legal skills are trusted.
- Non-bundled skills are not auto-trusted in `max_lockdown`.

## Post-Migration Validation

1. Start with a matter:
   - `clawyer --matter demo-matter`
2. Confirm domain deny-by-default:
   - call `http` tool to non-allowlisted host and verify block.
3. Confirm matter-scoped writes:
   - attempt `write_file` outside `matters/demo-matter` and verify block.
4. Confirm audit log:
   - check `logs/legal_audit.jsonl` for hash-linked entries.


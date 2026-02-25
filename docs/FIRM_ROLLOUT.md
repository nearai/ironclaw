# cLawyer Firm Rollout (Single-Tenant Local)

This rollout is designed for one firm deployment per host (single tenant).

## 1. Install

```bash
cargo build --release
./target/release/clawyer onboard
```

During onboarding, keep:

- legal profile enabled
- hardening: `max_lockdown`
- network: `deny_by_default`
- audit logging enabled

## 2. Baseline Policy

Use:

- `matters/<matter_id>/` for all matter artifacts
- `conflicts.json` for local conflict checks
- `matter.yaml` in each matter directory

Review seeded docs:

- `AGENTS.md`
- `legal/CITATION_STYLE_GUIDE.md`
- `legal/CONFIDENTIALITY_NOTES.md`

## 3. Tooling Controls

- Keep tool approvals enabled.
- Only add allowlisted domains with explicit need.
- Require matter context in user workflows.

## 4. Operational Checks

Daily:

- verify `logs/legal_audit.jsonl` is writable
- verify blocked-action and redaction events are recorded

Weekly:

- review allowed domains
- review bundled and installed skills
- test a known block case (out-of-scope write or unallowlisted HTTP host)

## 5. Incident Response (Local)

If suspicious behavior occurs:

1. Pause agent usage.
2. Preserve `logs/legal_audit.jsonl`.
3. Export current settings (`clawyer config list`).
4. Rotate secrets and remove untrusted installed skills.
5. Re-run with strict `--legal-profile max-lockdown`.


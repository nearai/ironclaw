# cLawyer Legal Profile

`cLawyer` ships with legal-mode enabled by default for U.S.-general workflows.

## Defaults

- `legal.enabled = true`
- `legal.jurisdiction = "us-general"`
- `legal.hardening = "max_lockdown"`
- `legal.require_matter_context = true`
- `legal.citation_required = true`
- `legal.matter_root = "matters"`
- `legal.network.deny_by_default = true`
- `legal.audit.enabled = true`
- `legal.audit.path = "logs/legal_audit.jsonl"`
- `legal.audit.hash_chain = true`

## CLI Controls

- `--matter <matter_id>`
- `--jurisdiction <code>`
- `--legal-profile <max-lockdown|standard>`
- `--allow-domain <domain>` (repeatable)

## Runtime Flow

1. Request enters preflight.
2. cLawyer checks: active matter (for non-trivial legal requests), conflict list, tool approval policy, and domain allowlist.
3. Sensitive tool calls are approval-gated in `max_lockdown`.
4. Memory/file writes are scoped to `matters/<matter_id>/...` when matter context is required.
5. Output is scanned for leakage and citation markers.
6. Audit events are appended to JSONL with hash-chain links.

## Matter Model

Use:

```text
matters/<matter_id>/matter.yaml
```

Required metadata fields:

- `matter_id`
- `client`
- `confidentiality`
- `retention`

If metadata is missing or invalid, legal task execution is blocked with guidance.

## Bundled Legal Skills

Trusted bundled skills:

- `legal-intake`
- `legal-chronology`
- `legal-contract-review`
- `legal-litigation-support`
- `legal-research-synthesis`

Each bundled legal skill is expected to include:

- `domain: legal`
- `requires_matter: true`
- `citation_mode: required`

## Audit and Metrics

Audit log events include:

- prompts
- approvals
- blocked operations
- redaction events
- skill activations

Counters tracked in audit state:

- `blocked_actions`
- `approval_required`
- `redaction_events`


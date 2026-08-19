# ironclaw_prompt_envelope

The prompt-injection fence: one primitive — `wrap_untrusted` — that wraps
untrusted, model-visible content with an explicit, closed-vocabulary trust
marker before it reaches a model, rejects instruction-hijack phrasing outright,
and caps the result at a byte budget. It is a separate crate because its
leaf-ness *is* the guarantee: it depends on nothing (internal deps: zero;
external: `thiserror` only), so wrapping a snippet never hands a consumer a
detection/redaction dependency cone, and no dependency of this crate can
influence what the fence lets through.

- **Family / layer:** `crates/contracts/` / `contracts` · **Package:** `ironclaw_prompt_envelope` · **Manifest:** `crates/contracts/ironclaw_prompt_envelope/Cargo.toml`
- **Use this when:** untrusted, source-attributed text (a memory snippet, a
  hook patch, a skill contribution) is about to become model-visible.
- **Don't use this when:** you need scanning/sanitizing of arbitrary content
  or severity policy → use `ironclaw_safety` instead; or the content is a
  host-authored safe summary that never left trusted code (no envelope
  needed).

## Public surface

Pure functions over closed enumerations — no traits (measured:
`rg -c '^pub trait' src/lib.rs` → 0):

- `wrap_untrusted(source, trust, body)` and
  `wrap_untrusted_with_limit(..., max_bytes)` — the only two entry points.
- `EnvelopeSource` — **closed**: `Memory`, `Hook`, `Skill`. Adding a variant
  is a security-relevant, reviewed API change, never a routine one — sources
  are deliberately not free-form strings.
- `EnvelopeTrust` — `Trusted` / `Untrusted`. Trusted content is still wrapped
  and still passes marker checks; only the prefix word differs.
- `EnvelopedContent` (prefix + body, `as_str`/`into_string`/`byte_len`) and
  `EnvelopeError` (`EmptyBody`, `HijackMarker`, byte-budget exceeded).
- `DEFAULT_MAX_ENVELOPE_BYTES` = 4 KiB, matched to the host-runtime safe
  summary and hooks snippet budgets.
- The instruction-hijack marker denylist ("ignore previous instructions",
  `<|im_start|>`, …). Content containing any marker is **rejected** — never
  silently passed through.

## Depends on / consumed by

- **Internal deps: none**, by design. External: `thiserror` only.
- **Consumed by 2 workspace crates** (measured:
  `grep -rl '^ironclaw_prompt_envelope = ' --include=Cargo.toml crates`):
  `ironclaw_hooks` (loop) and `ironclaw_host_runtime` (kernel). The
  `ironclaw_memory` contract's dependency allowlist *permits* this crate
  (`reborn_dependency_boundaries.rs::memory_contract_allowed`) though it holds
  no edge today; the design record's "its 3 consumers are exactly its 3
  sources" predates the memory/skill paths routing through the host runtime.

## Invariants

- **Leaf, forever:** an internal dep would fail the layer matrix / same-layer
  inventory; a framework dep fails
  `reborn_contracts_crates_hold_no_framework_dependencies`.
- **Closed vocabulary:** a new `EnvelopeSource` or a free-form source label is
  a contract change (crate doc, `#![deny(missing_docs)]`, and the family
  admission test all apply).
- **Reject, don't launder:** a hijack marker is an error, not a strip-and-pass.
- **Size ceiling:** `reborn_contracts_crates_carry_a_checked_size_ceiling`
  (432 production lines at writing; ceiling 832).
- The overlap with `ironclaw_safety::wrap_external_content` (two wrapping
  pipelines, two denylists) is a recorded open decision — PROPOSAL §12.10 —
  not an invitation to merge the crates.

## Tests

```bash
cargo test -p ironclaw_prompt_envelope
cargo test -p ironclaw_architecture_tests
```

## See also

- Family boundary: [`../AGENTS.md`](../AGENTS.md) (this crate has no separate
  `AGENTS.md`; the invariants above are the working rules).
- Design record: PROPOSAL §6.1.6;
  `docs/internal/reborn/target-architecture/families/contracts.md`.

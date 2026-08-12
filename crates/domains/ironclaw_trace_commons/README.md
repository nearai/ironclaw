# ironclaw_trace_commons

The Trace Commons / TraceDAO **client**: it turns a finished turn into a
consent-bearing, deterministically redacted envelope, queues it on disk, and
submits it to the external Trace Commons service — plus credits, device-key
onboarding, and the autonomous capture pipeline. It is the last thing that
runs before user data leaves the machine, so redaction is this crate's
security-critical obligation, not a feature. It is **not** the
tracing/observability subsystem — that is `ironclaw_observability`
(`crates/substrates/`).

- **Family / layer:** `domains` / `substrates` · **Package:** `ironclaw_trace_commons` · **Manifest:** `crates/domains/ironclaw_trace_commons/Cargo.toml`
- **Use this when:** anything about contributing traces — envelope schema,
  redaction rules, queue/holds, claims/credits, device-key enrollment, or the
  turn-end capture pipeline.
- **Don't use this when:** you want runtime telemetry/logging →
  `ironclaw_observability`; the model-callable trace-submission *tool* → the
  first-party extension package (a caller of this crate's client); recording
  model calls for replay → `ironclaw_llm::recording` (this crate only reuses
  that vocabulary).

## Public surface

- `contribution` — the whole contribution pipeline behind a glob re-export
  wall: `contribution::X` is the only public path; 13 private submodules whose
  charter lives in `src/contribution/mod.rs` (read it before adding code
  there).
- `capture` — the autonomous turn-end capture pipeline: standing-policy gate,
  envelope build, queue, immediate flush, periodic flush worker; keyed on a
  scope string + `ConversationMessage`, so it names no turn or runtime type.
- `client` — the host-facing trace client; `onboarding` — device-key
  enrollment, invites, protocol; `redaction` — `redact_sensitive_json`, the
  shared JSON scrubber; `conversation_message` — the shared message type.

## Depends on / consumed by

- **Normal deps (measured):** `ironclaw_common`, `ironclaw_host_api`,
  `ironclaw_llm` (recording vocabulary — the inventoried same-layer edge),
  `ironclaw_safety`. HTTP is chartered here for Trace Commons submission.
- **Consumed by (5):** `ironclaw` (CLI), `ironclaw_assistant`,
  `ironclaw_composition`, `ironclaw_host_runtime` (those four consume
  `contribution::…`), and `ironclaw_turn_runner` (hands turn-end data to
  `capture`).

## Invariants

- **Deterministic redaction before anything leaves the process** — a change
  that widens what an envelope carries needs a test pinning the *absence* of
  the raw value, not just the presence of the redacted one (see
  [`AGENTS.md`](./AGENTS.md)).
- The vendor **denylists** in `contribution/tool_payloads.rs` and
  `contribution/classification.rs` are deliberate carve-outs of
  `reborn_extension_specificity.rs` and must stay supersets of the bundled
  package inventory — sourcing them from the inventory would weaken redaction.
- `contribution`'s submodules stay private (`pub` there is a promise to four
  consumer crates); cross-submodule items are `pub(crate)`.
- Known, tracked gap (PROPOSAL §6.4.14): the crate still resolves its own
  paths via raw `std::fs`/`dirs`/env (39 production call sites measured
  2026-08-04) instead of taking a `ScopedFilesystem`; the device-key half
  additionally carries 0700-permission logic the mount plane cannot express
  yet. Do not add new raw-filesystem call sites.

## Tests

```bash
cargo test -p ironclaw_trace_commons
cargo test -p ironclaw_trace_commons --test trace_channel_wire_compat
```

`src/contribution/tests/` mirrors the production charter one module per owner
— put a new test in the module whose production owner it pins (two named
exceptions in `AGENTS.md`).

## See also

- Working rules: [`AGENTS.md`](./AGENTS.md) (canonical crate guidance —
  module map, test-placement rules, gap ledger).
- Family boundary: [`../AGENTS.md`](../AGENTS.md).
- Design record: `families/domains.md`, PROPOSAL §6.4.14.

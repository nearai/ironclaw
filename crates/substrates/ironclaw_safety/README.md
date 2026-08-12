# ironclaw_safety

Dependency-light detection and redaction primitives: prompt-injection
detection, input validation, secret-leak scanning, credential detection,
sensitive-path classification, and display redaction. This crate turns "does
this text look like an attack, a leaked secret, or a sensitive path" into a
typed, testable answer that kernel obligations, filesystem, memory, and hooks
all act on — it detects and redacts, and never enforces containment or holds
material itself.

- **Family / layer:** `substrates` / `substrates` · **Package:**
  `ironclaw_safety` · **Manifest:**
  `crates/substrates/ironclaw_safety/Cargo.toml`
- **Use this when:** untrusted text needs scanning before storage or LLM
  injection, or a value needs a safe-to-show form before display.
- **Don't use this when:** you want to *block* or *contain* → enforcement is
  the caller's job (kernel obligations, the sandbox lane); you want to *store*
  a secret → `ironclaw_secrets`; you want network policy → `ironclaw_network`.

## Public surface

- `SafetyLayer` — composes sanitizer + validator + policy engine + leak
  detector into one call.
- Individually consumable scanners: `Sanitizer` (injection patterns over
  untrusted text), `Validator` (structural/size validation for provider-bound
  content), `LeakDetector` (credential material about to leave a trust
  boundary), plus `credential_detect`, `sensitive_paths`, and
  `display_redaction`/`redaction` (modules: `sanitizer`, `validator`,
  `leak_detector`, `policy`, `prompt_validation`, `provider_validation`,
  `credential_detect`, `sensitive_paths`, `display_redaction`, `redaction`).
- `redact_model_input_text` — an infallible, source-independent model-view
  transform that combines known credential formats with labeled weak values
  such as `password: letmein`, including complete single-, double-, and
  backtick-quoted values; it also detects offset-prefixed character dumps that
  reconstruct a labeled value, preventing shell output from bypassing the model
  boundary by inserting whitespace between every character. Provider-visible
  host paths are replaced with `[REDACTED_HOST_PATH]`, and encoded JSON that
  exceeds the bounded decoder fails closed.
- `redact_model_input_url` — URL-aware model-view redaction for userinfo and
  credential query parameters; inline `data:` image payloads remain byte-for-byte
  unchanged.

## Depends on / consumed by

- **Depends on:** nothing in the workspace at the normal tier (one documented
  dev-dep on `ironclaw_secrets` pins the placeholder prefix). External: the
  pattern-matching cone this crate isolates — `regex`, `aho-corasick`, plus
  `url`/`urlencoding`/`serde_json`.
- **Consumed by (measured 2026-08-05):** 17 normal consumers across domains,
  kernel, lanes (`ironclaw_sandbox`), loops, products, and app — detection is
  needed from nearly every layer, and none of those callers needs to know how
  it works.

## Invariants

- **Detection is data, not authority** — rules here classify; they never decide
  who may call.
- **Bounded, linear-time matching on untrusted input** — no backtracking-regex
  behavior (crate rule in [`AGENTS.md`](./AGENTS.md)); the fuzz harness under
  `fuzz/` guards the parsers (see `fuzz/README.md`).
- **No raw secret values in findings** — a safety finding must never log or
  return the material it detected.
- **Model-input findings redact, never reject** — callers apply the transform
  at model-input boundaries: memory admission of model-visible content and
  immediately before provider dispatch. Structural validation and injection
  containment remain separate policies.
- Consumption boundaries are enforced from the consumer side (e.g. the
  `ironclaw_webui` `BoundaryRule` forbids direct `ironclaw_safety` use);
  the same-layer edges into this crate are inventoried in
  `reborn_same_layer_edge_inventory.rs`.

## Tests

```bash
cargo test -p ironclaw_safety
# fuzz targets after parser/pattern changes: see fuzz/README.md
```

## See also

Working rules: [`AGENTS.md`](./AGENTS.md) (canonical). Family boundary:
[`crates/substrates/AGENTS.md`](../AGENTS.md). Security rules:
`.claude/rules/safety-and-sandbox.md`. Design record: PROPOSAL §6.2.4
(including the §12.10 duplicate-pipeline cleanup note).

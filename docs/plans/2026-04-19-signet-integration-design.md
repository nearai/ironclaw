# Signet Integration Design

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Integrate `signet-core` into ironclaw's tool dispatch pipeline to cryptographically sign every tool call with Ed25519 and maintain a tamper-evident hash-chained audit log.

**Architecture:** Add a `SigningService` that wraps `signet-core`, invoked by `ToolDispatcher::dispatch()` after building the `ActionRecord` but before database persistence. Signing keys and audit logs live in signet's default directories (`~/.signet/`). A configurable skiplist allows excluding low-value tools from signing.

**Tech Stack:** `signet-core` 0.9 (Ed25519 via ed25519-dalek, SHA-256 hash chain, JSONL audit log)

---

## Decisions

| Dimension | Decision | Rationale |
|-----------|----------|-----------|
| Signing scope | Configurable: default sign-all, skiplist to exclude | Balance completeness with performance |
| Audit storage | signet JSONL hash chain (`~/.signet/audit/`) | Tamper-evident; DB rows are mutable, defeating hash chain purpose |
| Key management | signet keystore (`~/.signet/keys/`), auto-generate on startup | signet-cli compatible; zero-config offline verification |
| Audit directory | `~/.signet/audit/` (signet default) | Keys already at `~/.signet/keys/`; `signet-cli verify-chain` works out of box |
| DB schema | No changes | Signature lives in JSONL, not ActionRecord; zero migration |

## Integration Point

```
ToolDispatcher::dispatch()
    │
    ├─ 1. Resolve tool from registry
    ├─ 2. Safety validation (SafetyLayer)
    ├─ 3. Execute tool with timeout
    ├─ 4. Build ActionRecord (redacted params, sanitized output)
    ├─ 5. ★ Sign & append to audit chain (NEW)
    │     ├─ Check skiplist: tool_name in skip_tools? → skip
    │     ├─ Build signet::Action from ActionRecord fields
    │     ├─ signet::sign(key, action, "ironclaw", owner) → Receipt
    │     └─ Append Receipt to hash-chained JSONL
    └─ 6. Persist ActionRecord to DB (unchanged)
```

## New Modules

### `src/signing/mod.rs` — SigningService

```rust
use signet_core::{SigningKey, Receipt, Action};
use std::collections::HashSet;

pub struct SigningService {
    signing_key: SigningKey,
    skip_tools: HashSet<String>,
}

impl SigningService {
    /// Load or generate the "ironclaw" signing key.
    /// Called once at startup from app.rs.
    pub fn init(skip_tools: HashSet<String>) -> Result<Self, SigningError>;

    /// Sign a tool call and append to the audit chain.
    /// Returns None if the tool is in the skiplist.
    pub fn sign_action(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        output_summary: &str,
        success: bool,
        user_id: &str,
    ) -> Option<Receipt>;

    /// Verify the integrity of the full audit chain.
    pub fn verify_chain(&self) -> Result<ChainStatus, SigningError>;
}
```

### `src/config/signing.rs` — SigningConfig

```rust
pub struct SigningConfig {
    /// Master switch. Env: SIGNING_ENABLED (default: true)
    pub enabled: bool,

    /// Tools to skip signing. Env: SIGNING_SKIP_TOOLS (comma-separated)
    /// Example: "echo,time,json_parse"
    pub skip_tools: Vec<String>,
}
```

## Files Changed

| File | Change |
|------|--------|
| `Cargo.toml` | Add `signet-core = "0.9"` dependency |
| `src/signing/mod.rs` | **New** — `SigningService`, `SigningError` |
| `src/config/signing.rs` | **New** — `SigningConfig` |
| `src/config/mod.rs` | Add `SigningConfig` to top-level `Config` |
| `src/tools/dispatch.rs` | Insert `signing_service.sign_action()` between steps 4-5 |
| `src/app.rs` | Initialize `SigningService` on startup, pass to `ToolDispatcher` |
| `src/lib.rs` | Declare `pub mod signing;` |

## Files NOT Changed

- `ActionRecord` struct — signature lives in JSONL, not DB
- Database schema — zero migrations
- Hook system — signing is not a hook (it needs post-execution data)
- Observer system — Phase 2 can add `ActionSigned` event

## Error Handling

Signing failures must NOT block tool execution. The dispatch pipeline treats signing as best-effort:

```rust
if let Err(e) = signing_service.sign_action(...) {
    tracing::warn!(tool = %tool_name, error = %e, "Failed to sign tool call");
    // Continue — tool result is still returned to caller
}
```

This matches the existing pattern for `save_action` failures in dispatch.rs.

## Configuration

Environment variables (following ironclaw's existing config pattern):

```bash
SIGNING_ENABLED=true           # default: true
SIGNING_SKIP_TOOLS=echo,time   # default: empty (sign everything)
```

## Testing Strategy

1. **Unit tests** (`src/signing/mod.rs`):
   - `test_sign_action_produces_valid_receipt` — sign + verify roundtrip
   - `test_skiplist_excludes_tool` — skipped tool returns None
   - `test_auto_generate_key_on_first_run` — key created if missing
   - `test_chain_integrity_after_multiple_signs` — verify_chain passes

2. **Integration test** (`tests/signing_integration.rs`):
   - Full dispatch pipeline with signing enabled
   - Verify JSONL file written with correct hash chain
   - Verify `signet-core::verify_chain()` passes on generated audit

## Future Work (Out of Scope)

- **Phase 2:** Add `ObserverEvent::ActionSigned` for real-time signing notifications
- **Phase 2:** Add `signature: Option<String>` to `ActionRecord` for DB queryability
- **Phase 3:** CLI subcommand `ironclaw audit verify` wrapping `signet-cli verify-chain`
- **Phase 3:** Bilateral signing (agent + server) for MCP tool calls
- **Phase 3:** Policy attestation via signet's YAML policy engine

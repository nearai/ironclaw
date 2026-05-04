# Signet Integration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Integrate `signet-core` into ironclaw for Ed25519 tool call signing with hash-chained audit log.

**Architecture:** `SigningService` wraps `signet-core`, invoked by `ToolDispatcher::dispatch()` after ActionRecord construction. Keys and audit at `~/.signet/`.

**Tech Stack:** signet-core 0.9, ed25519-dalek, SHA-256 hash chain

**Design doc:** `docs/plans/2026-04-19-signet-integration-design.md`

---

### Task 1: Add signet-core dependency

**Files:**
- Modify: `Cargo.toml`

**Step 1: Add dependency**

Add to `[dependencies]` section:
```toml
signet-core = "0.9"
```

**Step 2: Verify it compiles**

Run: `cargo check`
Expected: compiles without errors

**Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: add signet-core dependency"
```

---

### Task 2: Add SigningConfig

**Files:**
- Create: `src/config/signing.rs`
- Modify: `src/config/mod.rs`
- Test: `src/config/signing.rs` (inline tests)

**Step 1: Write the failing test**

Create `src/config/signing.rs` with test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_signs_everything() {
        let config = SigningConfig::default();
        assert!(config.enabled);
        assert!(config.skip_tools.is_empty());
    }

    #[test]
    fn test_skip_tools_parsing() {
        // Simulate SIGNING_SKIP_TOOLS=echo,time
        let config = SigningConfig {
            enabled: true,
            skip_tools: vec!["echo".to_string(), "time".to_string()],
        };
        assert_eq!(config.skip_tools.len(), 2);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p ironclaw --lib config::signing`
Expected: FAIL — module does not exist

**Step 3: Write SigningConfig**

In `src/config/signing.rs`:

```rust
use std::env;

/// Configuration for cryptographic tool call signing.
#[derive(Debug, Clone)]
pub struct SigningConfig {
    /// Master switch. Env: SIGNING_ENABLED (default: true)
    pub enabled: bool,

    /// Tools to skip signing. Env: SIGNING_SKIP_TOOLS (comma-separated)
    pub skip_tools: Vec<String>,
}

impl Default for SigningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            skip_tools: Vec::new(),
        }
    }
}

impl SigningConfig {
    pub fn from_env() -> Self {
        let enabled = env::var("SIGNING_ENABLED")
            .map(|v| v != "false" && v != "0")
            .unwrap_or(true);

        let skip_tools = env::var("SIGNING_SKIP_TOOLS")
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        Self {
            enabled,
            skip_tools,
        }
    }
}
```

**Step 4: Register in config/mod.rs**

Add `pub mod signing;` and include `SigningConfig` in the top-level `Config` struct.

**Step 5: Run tests**

Run: `cargo test -p ironclaw --lib config::signing`
Expected: PASS

**Step 6: Commit**

```bash
git add src/config/signing.rs src/config/mod.rs
git commit -m "feat(config): add SigningConfig for signet integration"
```

---

### Task 3: Implement SigningService

**Files:**
- Create: `src/signing/mod.rs`
- Modify: `src/lib.rs`
- Test: `src/signing/mod.rs` (inline tests)

**Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_sign_action_produces_valid_receipt() {
        let service = SigningService::init(HashSet::new())
            .expect("should init with auto-generated key");
        let receipt = service.sign_action(
            "shell",
            &serde_json::json!({"command": "ls"}),
            "file list output",
            true,
            "user-1",
        );
        assert!(receipt.is_some(), "non-skipped tool should produce receipt");
    }

    #[test]
    fn test_skiplist_excludes_tool() {
        let skip = HashSet::from(["echo".to_string()]);
        let service = SigningService::init(skip)
            .expect("should init");
        let receipt = service.sign_action(
            "echo",
            &serde_json::json!({"text": "hello"}),
            "hello",
            true,
            "user-1",
        );
        assert!(receipt.is_none(), "skipped tool should return None");
    }

    #[test]
    fn test_chain_integrity_after_multiple_signs() {
        let service = SigningService::init(HashSet::new())
            .expect("should init");
        for i in 0..5 {
            service.sign_action(
                "shell",
                &serde_json::json!({"command": format!("cmd-{i}")}),
                "ok",
                true,
                "user-1",
            );
        }
        let status = signet_core::verify_chain()
            .expect("chain verification should succeed");
        assert!(
            matches!(status, signet_core::ChainStatus::Valid { .. }),
            "chain should be valid after sequential signs"
        );
    }
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p ironclaw --lib signing`
Expected: FAIL — module does not exist

**Step 3: Implement SigningService**

Create `src/signing/mod.rs`:

```rust
use std::collections::HashSet;
use signet_core::{self, SigningKey, Action, Receipt};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SigningError {
    #[error("Failed to initialize signing key: {0}")]
    KeyInit(String),

    #[error("Failed to sign action: {0}")]
    SignFailed(String),
}

pub struct SigningService {
    signing_key: SigningKey,
    skip_tools: HashSet<String>,
}

impl SigningService {
    /// Load or auto-generate the "ironclaw" signing key.
    pub fn init(skip_tools: HashSet<String>) -> Result<Self, SigningError> {
        let signing_key = match signet_core::load_signing_key("ironclaw") {
            Ok(key) => key,
            Err(_) => {
                tracing::info!("No signing key found, generating new Ed25519 keypair");
                signet_core::generate_and_save("ironclaw")
                    .map_err(|e| SigningError::KeyInit(e.to_string()))?
                    .signing_key()
            }
        };

        Ok(Self {
            signing_key,
            skip_tools,
        })
    }

    /// Sign a tool call and append to the audit chain.
    /// Returns None if the tool is in the skiplist.
    pub fn sign_action(
        &self,
        tool_name: &str,
        input: &serde_json::Value,
        output_summary: &str,
        success: bool,
        user_id: &str,
    ) -> Option<Receipt> {
        if self.skip_tools.contains(tool_name) {
            return None;
        }

        let action = Action {
            tool: tool_name.to_string(),
            input: input.clone(),
            output: Some(serde_json::json!({
                "summary": output_summary,
                "success": success,
            })),
        };

        match signet_core::sign(
            &self.signing_key,
            action,
            "ironclaw",
            user_id,
        ) {
            Ok(receipt) => Some(receipt),
            Err(e) => {
                tracing::warn!(
                    tool = %tool_name,
                    error = %e,
                    "Failed to sign tool call"
                );
                None
            }
        }
    }
}
```

**Step 4: Declare module in lib.rs**

Add `pub mod signing;` to `src/lib.rs`.

**Step 5: Run tests**

Run: `cargo test -p ironclaw --lib signing`
Expected: PASS

**Step 6: Run clippy**

Run: `cargo clippy --all --all-features -p ironclaw -- -D warnings`
Expected: zero warnings

**Step 7: Commit**

```bash
git add src/signing/mod.rs src/lib.rs
git commit -m "feat(signing): add SigningService wrapping signet-core"
```

---

### Task 4: Wire into ToolDispatcher

**Files:**
- Modify: `src/tools/dispatch.rs`
- Modify: `src/app.rs`

**Step 1: Add SigningService to ToolDispatcher**

In `src/tools/dispatch.rs`, add `signing: Option<Arc<SigningService>>` field to `ToolDispatcher` struct. Update the constructor to accept it.

**Step 2: Insert signing call in dispatch()**

After building `ActionRecord` (step 4) and before `save_action` (step 6):

```rust
// Sign the action (best-effort, never blocks tool result)
if let Some(ref signing) = self.signing {
    let output_summary = match &final_result {
        Ok(output) => output.result.to_string(),
        Err(e) => e.to_string(),
    };
    // Truncate output_summary to avoid signing multi-MB payloads
    let summary = if output_summary.len() > 1024 {
        &output_summary[..output_summary.floor_char_boundary(1024)]
    } else {
        &output_summary
    };
    if let Err(e) = signing.sign_action(
        &resolved_name,
        &safe_params,
        summary,
        final_result.is_ok(),
        user_id,
    ) {
        tracing::warn!(tool = %resolved_name, error = %e, "Failed to sign tool call");
    }
}
```

**Step 3: Initialize in app.rs**

In the startup sequence, after config loading:

```rust
let signing_service = if config.signing.enabled {
    match SigningService::init(config.signing.skip_tools.iter().cloned().collect()) {
        Ok(s) => {
            tracing::debug!("Signing service initialized");
            Some(Arc::new(s))
        }
        Err(e) => {
            tracing::warn!(error = %e, "Signing service failed to initialize, continuing without signing");
            None
        }
    }
} else {
    None
};
```

Pass `signing_service` to `ToolDispatcher::new()`.

**Step 4: Verify compilation**

Run: `cargo check`
Expected: compiles

**Step 5: Commit**

```bash
git add src/tools/dispatch.rs src/app.rs
git commit -m "feat(signing): wire SigningService into ToolDispatcher pipeline"
```

---

### Task 5: Integration test

**Files:**
- Create: `tests/signing_integration.rs`

**Step 1: Write integration test**

```rust
//! Integration test: verify the signing pipeline produces a valid audit chain.

use std::collections::HashSet;

#[test]
fn test_dispatch_with_signing_creates_valid_audit_chain() {
    // 1. Init SigningService with temp SIGNET_DIR
    // 2. Call sign_action for 3 different tools
    // 3. Call signet_core::verify_chain()
    // 4. Assert chain is valid and contains 3 entries
}

#[test]
fn test_signing_disabled_produces_no_audit() {
    // 1. Don't init SigningService (None)
    // 2. Dispatch a tool call
    // 3. Assert no JSONL files created
}
```

**Step 2: Run integration test**

Run: `cargo test --test signing_integration`
Expected: PASS

**Step 3: Commit**

```bash
git add tests/signing_integration.rs
git commit -m "test(signing): add integration tests for audit chain"
```

---

### Task 6: Final verification and cleanup

**Step 1: Run full clippy**

Run: `cargo clippy --all --benches --tests --examples --all-features -- -D warnings`
Expected: zero warnings

**Step 2: Run full test suite**

Run: `cargo test`
Expected: all pass

**Step 3: Run pre-commit safety checks**

Run: `scripts/pre-commit-safety.sh`
Expected: pass

**Step 4: Commit any fixes**

**Step 5: Create PR**

```bash
git push fork feat/signet-integration -u
gh pr create --base staging --title "feat(signing): integrate signet-core for cryptographic tool call audit" --body "..."
```

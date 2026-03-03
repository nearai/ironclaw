# Testing Batch 2: Gateway Helpers, Security Tests, Search Edge Cases

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extract shared gateway test helpers to eliminate duplication, add regression tests for installer security controls, and expand RRF search edge case coverage.

**Architecture:** Gateway helpers go in `src/channels/web/test_helpers.rs` (new file, `#[cfg(test)]` gated) with a `TestGatewayBuilder` that eliminates the 19-field struct duplication. Security tests go inline in `src/tools/builtin/skill_tools.rs`. Search edge case tests go inline in `src/workspace/search.rs`.

**Tech Stack:** Rust, axum, tokio, existing test infrastructure

---

### Task 1: Extract TestGatewayBuilder

**Files:**
- Create: `src/channels/web/test_helpers.rs`
- Modify: `src/channels/web/mod.rs` — add `#[cfg(test)] pub mod test_helpers;`
- Modify: `tests/ws_gateway_integration.rs` — replace manual GatewayState construction
- Modify: `tests/openai_compat_integration.rs` — replace manual GatewayState construction

**Context:** Both `ws_gateway_integration.rs` and `openai_compat_integration.rs` manually construct `GatewayState` with 19 fields, differing only in `msg_tx` and `llm_provider`. The builder should provide sensible defaults and let tests override what they need.

**Step 1: Create `src/channels/web/test_helpers.rs`**

```rust
//! Shared test helpers for the web gateway.
//!
//! Provides [`TestGatewayBuilder`] to eliminate boilerplate when constructing
//! `GatewayState` for tests. Both unit tests and integration tests can use this.

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::channels::IncomingMessage;
use crate::channels::web::server::{GatewayState, RateLimiter, start_server};
use crate::channels::web::sse::SseManager;
use crate::channels::web::ws::WsConnectionTracker;
use crate::llm::LlmProvider;

/// Default auth token for test servers.
pub const TEST_AUTH_TOKEN: &str = "test-token-12345";

/// Builder for constructing a test gateway server with sensible defaults.
///
/// # Usage
///
/// ```rust,no_run
/// let (addr, state, agent_rx) = TestGatewayBuilder::new()
///     .with_agent_channel()
///     .start()
///     .await;
/// ```
pub struct TestGatewayBuilder {
    auth_token: String,
    agent_channel: bool,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    ws_tracker: bool,
}

/// Result of starting a test gateway.
pub struct TestGateway {
    pub addr: SocketAddr,
    pub state: Arc<GatewayState>,
    pub agent_rx: Option<mpsc::Receiver<IncomingMessage>>,
}

impl TestGatewayBuilder {
    pub fn new() -> Self {
        Self {
            auth_token: TEST_AUTH_TOKEN.to_string(),
            agent_channel: false,
            llm_provider: None,
            ws_tracker: true,
        }
    }

    /// Enable the agent message channel (returns the receiver in TestGateway).
    pub fn with_agent_channel(mut self) -> Self {
        self.agent_channel = true;
        self
    }

    /// Set a custom LLM provider (for OpenAI-compat endpoint tests).
    pub fn with_llm(mut self, provider: Arc<dyn LlmProvider>) -> Self {
        self.llm_provider = Some(provider);
        self
    }

    /// Set a custom auth token.
    pub fn with_auth_token(mut self, token: impl Into<String>) -> Self {
        self.auth_token = token.into();
        self
    }

    /// Start the test server on a random port.
    pub async fn start(self) -> TestGateway {
        let (msg_tx, agent_rx) = if self.agent_channel {
            let (tx, rx) = mpsc::channel(64);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };

        let state = Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(msg_tx),
            sse: SseManager::new(),
            workspace: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: None,
            job_manager: None,
            prompt_queue: None,
            user_id: "test-user".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: if self.ws_tracker {
                Some(Arc::new(WsConnectionTracker::new()))
            } else {
                None
            },
            llm_provider: self.llm_provider,
            skill_registry: None,
            skill_catalog: None,
            chat_rate_limiter: RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            startup_time: std::time::Instant::now(),
            restart_requested: std::sync::atomic::AtomicBool::new(false),
        });

        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let bound_addr = start_server(addr, state.clone(), self.auth_token)
            .await
            .expect("Failed to start test server");

        TestGateway {
            addr: bound_addr,
            state,
            agent_rx,
        }
    }
}

impl Default for TestGatewayBuilder {
    fn default() -> Self {
        Self::new()
    }
}
```

**Step 2: Register the module in `src/channels/web/mod.rs`**

Add: `#[cfg(test)] pub mod test_helpers;`

**Step 3: Refactor `tests/ws_gateway_integration.rs`**

Replace the manual `start_test_server()` function with:

```rust
use ironclaw::channels::web::test_helpers::{TestGatewayBuilder, TEST_AUTH_TOKEN};

async fn start_test_server() -> (SocketAddr, Arc<GatewayState>, mpsc::Receiver<IncomingMessage>) {
    let gw = TestGatewayBuilder::new()
        .with_agent_channel()
        .start()
        .await;
    (gw.addr, gw.state, gw.agent_rx.unwrap())
}
```

Update `AUTH_TOKEN` references to use `TEST_AUTH_TOKEN` or keep the local const and pass it via `.with_auth_token()`.

**Step 4: Refactor `tests/openai_compat_integration.rs`**

Replace the manual `start_test_server_with_provider()` with:

```rust
use ironclaw::channels::web::test_helpers::TestGatewayBuilder;

async fn start_test_server_with_provider(
    llm_provider: Arc<dyn LlmProvider>,
) -> (SocketAddr, Arc<GatewayState>) {
    let gw = TestGatewayBuilder::new()
        .with_llm(llm_provider)
        .start()
        .await;
    (gw.addr, gw.state)
}
```

**Step 5: Verify all gateway tests still pass**

Run: `cargo test ws_gateway_integration -- --nocapture`
Run: `cargo test openai_compat_integration -- --nocapture`

**Step 6: Commit**

```bash
git add src/channels/web/test_helpers.rs src/channels/web/mod.rs tests/ws_gateway_integration.rs tests/openai_compat_integration.rs
git commit -m "refactor(testing): extract TestGatewayBuilder to eliminate gateway test duplication"
```

---

### Task 2: Add installer security regression tests

**Files:**
- Modify: `src/tools/builtin/skill_tools.rs` — add tests to existing `mod tests` block

**Context:** The skill installer has strong security controls (ZIP bomb protection, SSRF prevention, path traversal prevention) but no tests proving they work. These are regression tests to prevent future breakage. The `extract_skill_from_zip()` function (lines 522-616) parses ZIP archives manually and only matches `SKILL.md` exactly. The `is_private_ip()` and `is_restricted_host()` functions handle SSRF.

**Step 1: Add security regression tests**

Add to the `mod tests` block in `src/tools/builtin/skill_tools.rs`:

Tests to add:
- `test_zip_extraction_only_matches_skill_md` — ZIP with `SKILL.md` + other files extracts only SKILL.md
- `test_zip_extraction_rejects_path_traversal` — ZIP entry named `../SKILL.md` is not matched
- `test_zip_extraction_rejects_oversized` — ZIP with declared uncompressed size > 1MB is rejected
- `test_ssrf_blocks_private_ips` — `is_private_ip()` blocks 127.0.0.1, 10.x, 172.16.x, 192.168.x, 169.254.x
- `test_ssrf_blocks_ipv4_mapped_ipv6` — `is_private_ip()` blocks `::ffff:127.0.0.1`
- `test_ssrf_blocks_restricted_hosts` — `is_restricted_host()` blocks localhost, .internal, .local, metadata IPs

These test the actual functions, not HTTP endpoints — they're pure unit tests.

**Step 2: Run tests**

Run: `cargo test tools::builtin::skill_tools::tests -- --nocapture`

**Step 3: Commit**

```bash
git add src/tools/builtin/skill_tools.rs
git commit -m "test(security): add regression tests for skill installer ZIP and SSRF protections"
```

---

### Task 3: Add RRF search edge case tests

**Files:**
- Modify: `src/workspace/search.rs` — add tests to existing `mod tests` block

**Context:** The RRF algorithm (`merge_rrf()` in search.rs) has 7 tests but misses edge cases. The function takes `fts_results` and `vector_results` as `Vec<ScoredChunk>` and merges them using Reciprocal Rank Fusion with configurable `k` parameter, `min_score`, and `limit`.

**Step 1: Add edge case tests**

Tests to add:
- `test_rrf_both_empty` — empty FTS + empty vector = empty results
- `test_rrf_one_empty_one_populated` — only FTS results, no vector results (and vice versa)
- `test_rrf_duplicate_chunks_across_methods` — same chunk_id in both FTS and vector results gets merged/boosted
- `test_rrf_limit_zero` — limit=0 returns empty
- `test_rrf_min_score_filters_all` — min_score=1.0 filters everything
- `test_search_config_fts_only_disables_vector` — `fts_only()` config works correctly
- `test_search_config_vector_only_disables_fts` — `vector_only()` config works correctly

**Step 2: Run tests**

Run: `cargo test workspace::search::tests -- --nocapture`

**Step 3: Commit**

```bash
git add src/workspace/search.rs
git commit -m "test(search): add RRF edge case tests for empty inputs, limits, and config modes"
```

---

### Task 4: Add architecture boundary check script

**Files:**
- Create: `scripts/check-boundaries.sh`

**Context:** OpenClaw uses custom Oxlint rules to enforce architectural boundaries. We can start simpler with grep-based checks in a shell script that CI can run. The key boundaries: no direct DB calls outside `src/db/`, no direct LLM calls outside `src/llm/`, no raw secret access outside `src/secrets/`.

**Step 1: Create the script**

The script should check for common boundary violations using grep patterns:
- Direct `tokio_postgres::` or `libsql::` usage outside `src/db/` and `src/workspace/repository.rs`
- Direct `reqwest::Client::new()` in channel code (should use shared client)
- Usage of `std::env::var` for secrets (should use config/secrets module)

Keep it simple — just the most valuable checks. Output violations with file:line format.

**Step 2: Run and verify**

Run: `bash scripts/check-boundaries.sh`
Expected: Either clean or with documented exceptions

**Step 3: Commit**

```bash
git add scripts/check-boundaries.sh
git commit -m "ci: add architecture boundary check script"
```

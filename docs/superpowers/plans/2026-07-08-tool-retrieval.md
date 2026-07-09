# Tool-Retrieval Implementation Plan (Phase 1 of Adaptive Routing)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Inject only the tools relevant to each turn (top-K by semantic similarity) instead of all ~200, so the agent stays fully capable while the per-turn context (and TTFT) drops sharply.

**Architecture:** A pure-Rust `ToolRetriever` embeds each tool's `"name: description"` once (via the existing nomic `EmbeddingProvider`), embeds the incoming user message per turn, cosine-ranks tools, and returns `core ∪ top-K` tool names. The worker turn loop uses `ToolRegistry::tool_definitions_for(&names)` to assemble `available_tools` from that set instead of `tool_definitions()`. Feature-flagged; falls back to all tools on any error.

**Tech Stack:** Rust, tokio, `ironclaw_embeddings` (nomic), `ironclaw_llm::ToolDefinition`, `ToolRegistry`.

## Global Constraints
- Rust edition/toolchain as in repo; build: `~/.cargo/bin/cargo build --release`.
- No `.unwrap()`/`.expect()` in production paths; error types via `thiserror` (`crate::error`).
- Every behavior change ships a regression test (commit-msg hook enforces).
- Import extracted-crate types directly (`use ironclaw_llm::ToolDefinition;`, `use ironclaw_embeddings::EmbeddingProvider;`).
- Feature is **off = exact current behavior**; flag `TOOL_RETRIEVAL_ENABLED` (default `true`, but a single point disables it).
- Never silently drop tools: any retrieval error → fall back to ALL tools + `tracing::warn!`.
- Deploy out-of-band: build `target/release/ironclaw`, test the fresh binary manually, keep `/usr/local/bin/ironclaw` for rollback, cut over + restart, verify.

---

### Task 1: Cosine similarity + top-K ranking (pure function)

**Files:**
- Create: `src/tools/retrieval.rs`
- Modify: `src/tools/mod.rs` (add `pub mod retrieval;`)
- Test: inline `#[cfg(test)] mod tests` in `src/tools/retrieval.rs`

**Interfaces:**
- Produces: `pub fn cosine(a: &[f32], b: &[f32]) -> f32`; `pub fn rank_top_k(query: &[f32], items: &[(String, Vec<f32>)], k: usize, min_score: f32) -> Vec<String>` (returns item keys whose score ≥ `min_score`, highest first, capped at `k`).

- [ ] **Step 1: Write the failing test**
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cosine_and_ranking() {
        // orthogonal -> 0, identical -> 1
        assert!((cosine(&[1.0, 0.0], &[0.0, 1.0])).abs() < 1e-6);
        assert!((cosine(&[1.0, 1.0], &[1.0, 1.0]) - 1.0).abs() < 1e-6);
        let items = vec![
            ("trip".into(), vec![1.0, 0.0]),
            ("ocr".into(),  vec![0.0, 1.0]),
            ("place".into(), vec![0.9, 0.1]),
        ];
        // query aligned with "trip"/"place" axis; k=2, floor 0.5
        let got = rank_top_k(&[1.0, 0.0], &items, 2, 0.5);
        assert_eq!(got, vec!["trip".to_string(), "place".to_string()]);
    }
    #[test]
    fn min_score_floor_excludes_weak_and_k_caps() {
        let items = vec![("a".into(), vec![1.0,0.0]), ("b".into(), vec![0.2,0.98])];
        assert_eq!(rank_top_k(&[1.0,0.0], &items, 5, 0.5), vec!["a".to_string()]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `~/.cargo/bin/cargo test -p ironclaw retrieval::tests -- --nocapture`
Expected: FAIL — `cosine`/`rank_top_k` not found.

- [ ] **Step 3: Write minimal implementation**
```rust
//! Per-turn semantic tool retrieval: rank tools by similarity to the message.

/// Cosine similarity. Returns 0.0 if either vector is zero-length or empty.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Rank `items` (key, vector) by cosine to `query`; keep score >= `min_score`,
/// highest first, capped at `k`. Returns the keys.
pub fn rank_top_k(
    query: &[f32],
    items: &[(String, Vec<f32>)],
    k: usize,
    min_score: f32,
) -> Vec<String> {
    let mut scored: Vec<(&String, f32)> = items
        .iter()
        .map(|(key, vec)| (key, cosine(query, vec)))
        .filter(|(_, s)| *s >= min_score)
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().take(k).map(|(key, _)| key.clone()).collect()
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `~/.cargo/bin/cargo test -p ironclaw retrieval::tests`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**
```bash
git add src/tools/retrieval.rs src/tools/mod.rs
git commit -m "feat(tools): cosine + top-k ranking for tool retrieval"
```

---

### Task 2: ToolRetriever — build index + select per turn

**Files:**
- Modify: `src/tools/retrieval.rs`
- Test: inline tests (use the `KeywordEmbeddings` stub below — NOT `MockEmbeddings`)

**Interfaces:**
- Consumes: `ironclaw_llm::ToolDefinition { name, description }`; `ironclaw_embeddings::EmbeddingProvider::{embed, embed_batch}`.
- Produces:
  - `pub struct ToolRetriever { index: Vec<(String, Vec<f32>)>, core: Vec<String>, top_k: usize, min_score: f32 }`
  - `pub async fn ToolRetriever::build(defs: &[ToolDefinition], core: Vec<String>, top_k: usize, min_score: f32, embed: &dyn EmbeddingProvider) -> Result<Self, EmbeddingError>`
  - `pub async fn select(&self, message: &str, embed: &dyn EmbeddingProvider) -> Result<Vec<String>, EmbeddingError>` → `core ∪ top-K names` (deduped).

> **Test provider — read this first.** `ironclaw_embeddings::MockEmbeddings` hashes the
> *whole* input string (confirmed in `crates/ironclaw_embeddings/src/mock.rs`), so
> semantically-similar texts get UNRELATED vectors — it cannot demonstrate retrieval
> ranking. Tests in Tasks 2 and 4 therefore use a tiny in-test `KeywordEmbeddings` provider
> that maps a keyword to a fixed axis (deterministic AND similarity-reflecting). Real nomic
> semantic behavior is validated at Task 5 (deploy-time measurement), not in unit tests.
> Define this stub once in the test module:
> ```rust
> use async_trait::async_trait;
> use ironclaw_embeddings::{EmbeddingError, EmbeddingProvider};
> struct KeywordEmbeddings;
> #[async_trait]
> impl EmbeddingProvider for KeywordEmbeddings {
>     fn dimension(&self) -> usize { 3 }
>     fn model_name(&self) -> &str { "keyword-test" }
>     fn max_input_length(&self) -> usize { 10_000 }
>     async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
>         let t = text.to_lowercase();
>         // axis 0 = travel, 1 = image/ocr, 2 = everything else (incl. memory)
>         let v = if t.contains("trip") || t.contains("travel") { [1.0, 0.0, 0.0] }
>             else if t.contains("image") || t.contains("ocr") { [0.0, 1.0, 0.0] }
>             else { [0.0, 0.0, 1.0] };
>         Ok(v.to_vec())
>     }
>     // embed_batch uses the trait default (calls embed per item).
> }
> ```

- [ ] **Step 1: Write the failing test** (define `KeywordEmbeddings` above in the test module, then:)
```rust
#[tokio::test]
async fn retriever_selects_relevant_plus_core() {
    use ironclaw_llm::ToolDefinition;
    let embed = KeywordEmbeddings;
    let defs = vec![
        ToolDefinition { name: "create_trip".into(), description: "plan a trip / travel itinerary".into(), parameters: serde_json::json!({}) },
        ToolDefinition { name: "ocr_image".into(), description: "read text from an image".into(), parameters: serde_json::json!({}) },
        ToolDefinition { name: "memory_search".into(), description: "search memory".into(), parameters: serde_json::json!({}) },
    ];
    // top_k=1, floor=-1.0 => exactly the single best-ranked tool, plus the core set.
    let r = ToolRetriever::build(&defs, vec!["memory_search".into()], 1, -1.0, &embed).await.unwrap();
    let picked = r.select("plan a trip to Tokyo", &embed).await.unwrap();
    assert!(picked.contains(&"memory_search".to_string())); // core always present
    assert!(picked.contains(&"create_trip".to_string()));   // relevant retrieved (axis 0)
    assert!(!picked.contains(&"ocr_image".to_string()));    // irrelevant excluded (k=1)
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `~/.cargo/bin/cargo test -p ironclaw retriever_selects_relevant_plus_core`
Expected: FAIL — `ToolRetriever` not found.

- [ ] **Step 3: Write minimal implementation** (append to `src/tools/retrieval.rs`)
```rust
use ironclaw_embeddings::{EmbeddingError, EmbeddingProvider};
use ironclaw_llm::ToolDefinition;

pub struct ToolRetriever {
    index: Vec<(String, Vec<f32>)>, // (tool name, embedding of "name: description")
    core: Vec<String>,
    top_k: usize,
    min_score: f32,
}

impl ToolRetriever {
    pub async fn build(
        defs: &[ToolDefinition],
        core: Vec<String>,
        top_k: usize,
        min_score: f32,
        embed: &dyn EmbeddingProvider,
    ) -> Result<Self, EmbeddingError> {
        let texts: Vec<String> = defs
            .iter()
            .map(|d| format!("{}: {}", d.name, d.description))
            .collect();
        let vectors = embed.embed_batch(&texts).await?;
        let index = defs
            .iter()
            .map(|d| d.name.clone())
            .zip(vectors.into_iter())
            .collect();
        Ok(Self { index, core, top_k, min_score })
    }

    pub async fn select(
        &self,
        message: &str,
        embed: &dyn EmbeddingProvider,
    ) -> Result<Vec<String>, EmbeddingError> {
        let q = embed.embed(message).await?;
        let mut names = self.core.clone();
        for name in rank_top_k(&q, &self.index, self.top_k, self.min_score) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        Ok(names)
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `~/.cargo/bin/cargo test -p ironclaw retriever_selects_relevant_plus_core`
Expected: PASS.

- [ ] **Step 5: Commit**
```bash
git add src/tools/retrieval.rs
git commit -m "feat(tools): ToolRetriever build-index + per-turn select"
```

---

### Task 3: Config keys for tool retrieval

**Files:**
- Modify: `src/config/skills.rs` sibling — create `src/config/retrieval.rs`; register in `src/config/mod.rs`
- Test: inline test in `src/config/retrieval.rs`

**Interfaces:**
- Produces: `pub struct RetrievalConfig { pub enabled: bool, pub top_k: usize, pub min_score: f32, pub core_set: Vec<String> }` with `RetrievalConfig::from_env_and_db(ss: &RetrievalSettings, defaults: &RetrievalSettings) -> Result<Self, ConfigError>` following the existing `db_first_*` pattern (mirror `src/config/skills.rs`).
- Env keys (verbatim): `TOOL_RETRIEVAL_ENABLED` (default `true`), `TOOL_RETRIEVAL_TOP_K` (default `10`), `TOOL_RETRIEVAL_MIN_SCORE` (default `0.2`), `TOOL_CORE_SET` (default `memory_search,memory_write,memory_tree,memory_read,message`).

- [ ] **Step 1: Write the failing test**
```rust
#[test]
fn retrieval_config_defaults_and_core_parse() {
    // With nothing set, defaults apply.
    let c = RetrievalConfig::defaults();
    assert!(c.enabled);
    assert_eq!(c.top_k, 10);
    assert!((c.min_score - 0.2).abs() < 1e-6);
    assert!(c.core_set.contains(&"memory_tree".to_string()));
    // Comma parse
    assert_eq!(parse_core_set("a, b ,c"), vec!["a","b","c"]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `~/.cargo/bin/cargo test -p ironclaw retrieval_config_defaults_and_core_parse`
Expected: FAIL — module/functions absent.

- [ ] **Step 3: Write minimal implementation** — model `src/config/retrieval.rs` on `src/config/skills.rs`: a `RetrievalConfig` with a `defaults()` constructor, a `from_settings(...)` using `db_first_bool`/`db_first_or_default`/`optional_env` for each key, and `pub fn parse_core_set(s: &str) -> Vec<String>` splitting on `,`, trimming, dropping empties. Add `pub mod retrieval;` to `src/config/mod.rs` and a `pub retrieval: RetrievalConfig` field to the top-level `Config` populated in its builder (mirror how `skills` is wired in `src/config/mod.rs`/`builder.rs`).

- [ ] **Step 4: Run test to verify it passes**

Run: `~/.cargo/bin/cargo test -p ironclaw retrieval_config`
Expected: PASS.

- [ ] **Step 5: Commit**
```bash
git add src/config/retrieval.rs src/config/mod.rs src/config/builder.rs
git commit -m "feat(config): TOOL_RETRIEVAL_* settings"
```

---

### Task 4: Wire retrieval into the turn loop

**Files:**
- Modify: `src/worker/job.rs` (around 356–358 and 1453–1501 where `reason_ctx.available_tools` is assigned) and `src/worker/container.rs:174,429`
- Modify: worker deps to hold an `Option<Arc<ToolRetriever>>` + `Arc<dyn EmbeddingProvider>` + `RetrievalConfig` (confirm the exact deps struct; embeddings already flow to the gateway via `components.embeddings`).
- Test: `tests/tool_retrieval_integration.rs` (integration tier)

**Interfaces:**
- Consumes: `ToolRetriever::select`, `ToolRegistry::tool_definitions_for(&[&str])`, `RetrievalConfig`.
- Produces: turn-loop behavior — when `retrieval.enabled` and a user message is present, `available_tools = registry.tool_definitions_for(&selected_names)`; on `select` error or disabled, `available_tools = registry.tool_definitions[_visible_under]` (unchanged).

- [ ] **Step 1: Write the failing integration test**
This task wires into real worker internals — the implementer MUST first read `src/worker/job.rs`,
`src/worker/container.rs`, and how `components.embeddings` reaches the worker, then confirm the deps
struct that will hold `Option<Arc<ToolRetriever>>`. The test uses the same `KeywordEmbeddings` stub
from Task 2 (NOT `MockEmbeddings`). Because standing up a full worker turn is heavy, the preferred
test asserts the **selection→definition** seam directly rather than a full turn:
```rust
// tests/tool_retrieval_integration.rs  (run with --features integration)
// Reuse the KeywordEmbeddings stub (dimension=3, keyword→axis) from Task 2.
#[tokio::test]
async fn retrieval_narrows_available_tools() {
    // 1. Build a ToolRegistry containing create_trip / ocr_image / memory_search
    //    (register three minimal Tool impls, or the project's existing test-tool helper —
    //     confirm the helper in src/tools/registry.rs tests before writing).
    // 2. defs = registry.tool_definitions().await;  assert defs.len() == 3 (baseline = all).
    // 3. let r = ToolRetriever::build(&defs, vec!["memory_search".into()], 1, -1.0, &KeywordEmbeddings).await.unwrap();
    // 4. let names = r.select("plan a trip to Tokyo", &KeywordEmbeddings).await.unwrap();
    //    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    //    let narrowed = registry.tool_definitions_for(&refs).await;
    // 5. assert narrowed.len() == 2; names contain create_trip + memory_search, NOT ocr_image.
    //    This proves retrieval narrows the injected set from all-3 to core+relevant.
}
```
Fill each numbered comment with real code against the confirmed registry test-construction API.

- [ ] **Step 2: Run test to verify it fails**

Run: `~/.cargo/bin/cargo test --features integration turn_injects_only_retrieved_plus_core`
Expected: FAIL — retrieval not wired; all 3 tools returned.

- [ ] **Step 3: Write minimal implementation** — at each `reason_ctx.available_tools = ...` assignment in `job.rs`/`container.rs`, insert:
```rust
reason_ctx.available_tools = match (&self.retriever, self.retrieval.enabled, latest_user_message.as_deref()) {
    (Some(retriever), true, Some(msg)) => match retriever.select(msg, self.embeddings.as_ref()).await {
        Ok(names) => {
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            self.tools().tool_definitions_for(&refs).await
        }
        Err(e) => {
            tracing::warn!("tool retrieval failed ({e}); using all tools");
            self.tools().tool_definitions().await // fail toward capability
        }
    },
    _ => self.tools().tool_definitions().await, // disabled / no message: unchanged
};
```
Build the `ToolRetriever` once at worker/gateway init from `registry.tool_definitions()` + `RetrievalConfig` (Task 2's `build`), store `Option<Arc<ToolRetriever>>` in deps; rebuild it when the tool set changes (MCP toggle/add/remove) — for v1, rebuild lazily if the tool count differs from the index length.

- [ ] **Step 4: Run test to verify it passes**

Run: `~/.cargo/bin/cargo test --features integration turn_injects_only_retrieved_plus_core`
Expected: PASS (count == 2).

- [ ] **Step 5: Commit**
```bash
git add src/worker/job.rs src/worker/container.rs tests/tool_retrieval_integration.rs
git commit -m "feat(engine): inject only retrieved+core tools per turn"
```

---

### Task 5: Build, measure, deploy (out-of-band, reversible)

**Files:** none (build/deploy).

- [ ] **Step 1: Full build + lint**

Run: `~/.cargo/bin/cargo build --release && ~/.cargo/bin/cargo clippy --all-features 2>&1 | tail -5`
Expected: builds; zero new clippy warnings.

- [ ] **Step 2: Re-enable the heavy MCP servers (retrieval makes them cheap again)**

Run: `sudo runuser -u ironclaw -- bash -c 'set -a; . /home/ironclaw/.ironclaw/.env; set +a; ironclaw mcp toggle trek --enable; ironclaw mcp toggle vibetrading --enable; ironclaw mcp toggle worldmonitor --enable'`
Expected: all enabled (so retrieval selects from the full ~200 again).

- [ ] **Step 3: Test the fresh binary out-of-band, measure input_tokens**

Copy `target/release/ironclaw` → `/usr/local/bin/ironclaw-next`; stop service; run it briefly (repl mode) hitting `/v1/responses` with "plan a trip to Tokyo" and with "hi"; assert `input_tokens` is far below the ~59K all-tools baseline (target ≤ ~12K) and that trip tools appear for the trip prompt.

- [ ] **Step 4: Cut over with rollback in hand**

Back up `/usr/local/bin/ironclaw` → `ironclaw.prev`; install the new binary; `systemctl restart ironclaw`; verify `/v1/responses`. If broken: `cp ironclaw.prev /usr/local/bin/ironclaw; systemctl restart ironclaw`.

- [ ] **Step 5: Commit + tag**
```bash
git add -A && git commit -m "chore: tool-retrieval phase 1 measured + deployed"
```

---

### Task 6: Keep-warm — consistent low latency across new chats

**Goal:** a new chat / idle-then-message should not pay the full cold reprocess. Model is
already pinned (`OLLAMA_KEEP_ALIVE=-1`); this addresses KV-cache eviction + prompt-prefix
stability so the static prefix stays warm.

**Files:**
- Create: `/usr/local/bin/ironclaw-keepwarm.sh`, `/etc/systemd/system/ironclaw-keepwarm.{service,timer}` (deploy-time, via a `sudo bash` install script under `~/tool-integrations/`)
- Modify: `/etc/systemd/system/ollama.service.d/override.conf` (add `OLLAMA_NUM_PARALLEL=2`)

- [ ] **Step 1: Isolate background LLM from the user slot**

Add `Environment="OLLAMA_NUM_PARALLEL=2"` to the Ollama override so the heartbeat/routines
use a second slot and don't evict the interactive slot's KV cache. NOTE the tradeoff:
context window splits across slots (65536/2 = 32768 per slot) — acceptable now that
tool-retrieval keeps per-turn context ≪ 32K. `systemctl daemon-reload && systemctl restart ollama`.
Verify the model still loads 100% GPU: `docker exec … || nvidia-smi` + `ollama ps`.

- [ ] **Step 2: Keep-warm timer**

`ironclaw-keepwarm.sh`: every run, POST a trivial message to `/v1/responses` (reads the
token from `.env`) so the interactive slot re-primes the static prefix. Timer fires every
**4 minutes** (under any practical eviction window, over the 5-min prompt-cache TTL concerns):
```ini
# ironclaw-keepwarm.timer
[Timer]
OnBootSec=2min
OnUnitActiveSec=4min
[Install]
WantedBy=timers.target
```
`systemctl enable --now ironclaw-keepwarm.timer`.

- [ ] **Step 3: Measure**

After enabling: send a message in a **brand-new chat** → assert TTFT is in the low-single
seconds (warm), not ~16s. Re-run after 10 min idle to confirm the timer held the cache.

- [ ] **Step 4: (Engine, optional) stable prefix ordering**

Confirm the system prompt places STATIC content (identity, core instructions) FIRST and
DYNAMIC content (retrieved tools, memory, timestamp) AFTER, so the cacheable prefix is
maximal. If a per-request timestamp sits early in the prompt (breaks all caching), move it
after the static block. Locate via `crates/ironclaw_engine/prompts/` + the prompt assembler.

- [ ] **Step 5: Commit + document**
```bash
git add ~/tool-integrations/install-keepwarm.sh
git commit -m "feat(ops): keep-warm timer + NUM_PARALLEL to hold the interactive KV cache"
```

---

## Deferred to Phase 2 (separate plan)
- Fast/deep **model routing** (`qwen3:0.6b` vs `qwen3.6:27b`, escalation).
- Index rebuild on MCP registry mutation (event-driven vs the v1 lazy count-check).
- Retrieval granularity by MCP *group* instead of per-tool, if per-tool proves noisy.

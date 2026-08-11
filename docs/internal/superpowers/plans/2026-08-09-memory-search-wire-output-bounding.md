# Memory Search Wire-Output Bounding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Apply the existing query-aware 8 KiB memory-search excerpt at the shared conventional tool-output boundary instead of inside the native provider.

**Architecture:** Providers that implement the optional conventional search operation return raw `MemoryServiceSearchResponse` values. The provider-neutral `ironclaw_memory::search_response_output` helper consumes those responses and bounds each result's raw content before serializing the model-visible conventional tool JSON. Both native and mem0 handler paths already call this helper, while context retrieval and direct provider search callers bypass it.

**Tech Stack:** Rust, Tokio tests, Serde JSON, Cargo workspace checks.

---

## File Map

- Modify `crates/domains/ironclaw_memory/src/service.rs`: own the conventional output budget, excerpt algorithm, output serialization, and algorithm tests.
- Modify `crates/extensions/packages/memory-native/src/service.rs`: return raw backend snippets and remove native-only output policy and tests.
- Modify `crates/extensions/packages/memory-native/tests/memory_service_contract.rs`: pin raw provider-service behavior.
- Modify `crates/kernel/ironclaw_host_runtime/src/first_party_tools/memory.rs`: extend the existing production-shaped handler test to pin bounded model-visible output.
- Keep `crates/app/ironclaw_composition/src/memory_provider_factory.rs` unchanged: its mem0 handler already calls `search_response_output`.
- Keep both memory search JSON schemas unchanged: the wire fields do not change.

### Task 1: Pin Both Sides of the Boundary

**Files:**
- Modify: `crates/extensions/packages/memory-native/tests/memory_service_contract.rs:104-136`
- Modify: `crates/domains/ironclaw_memory/src/service.rs:822-948`

- [ ] **Step 1: Change the native provider regression to require raw content**

Replace `native_search_bounds_oversized_results_around_exact_query` with:

```rust
#[tokio::test]
async fn native_search_preserves_oversized_provider_result() {
    const QUERY: &str = "needle";
    const RESULT_BOUND: usize = 8 * 1024;
    let position = RESULT_BOUND + 512;
    let mut oversized = "a".repeat(position + QUERY.len() + RESULT_BOUND);
    oversized.replace_range(position..position + QUERY.len(), QUERY);
    let service = NativeMemoryService::new(Arc::new(MockSearchBackend {
        results: vec![search_result(
            "tenant-native-memory",
            "user-native-memory",
            "oversized.md",
            1.0,
            &oversized,
        )],
        fail: false,
    }));

    let response = service
        .search(
            invocation(),
            MemoryServiceSearchRequest {
                query: QUERY.to_string(),
                limit: 5,
            },
        )
        .await
        .expect("search through native memory service");

    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].content, oversized);
}
```

- [ ] **Step 2: Add a shared wire-output regression**

Append this test to `crates/domains/ironclaw_memory/src/service.rs`'s existing `tests` module:

```rust
#[test]
fn search_output_bounds_oversized_content_around_exact_query() {
    const QUERY: &str = "needle";
    const RESULT_BOUND: usize = 8 * 1024;
    let position = RESULT_BOUND + 512;
    let mut oversized = "a".repeat(position + QUERY.len() + RESULT_BOUND);
    oversized.replace_range(position..position + QUERY.len(), QUERY);

    let output = search_response_output(MemoryServiceSearchResponse {
        query: QUERY.to_string(),
        results: vec![MemoryServiceSearchResult {
            content: oversized,
            score: 1.0,
            path: "oversized.md".to_string(),
            is_hybrid_match: false,
        }],
    });
    let content = output["results"][0]["content"]
        .as_str()
        .expect("search content is a string");

    assert!(content.len() <= RESULT_BOUND);
    assert!(content.contains(QUERY));
}
```

- [ ] **Step 3: Run both tests and verify they fail for the intended reasons**

Run:

```bash
cargo test -p ironclaw_memory search_output_bounds_oversized_content_around_exact_query
cargo test -p ironclaw_memory_native native_search_preserves_oversized_provider_result
```

Expected: the shared output test reports content larger than 8192 bytes; the native contract test reports that actual content is the existing bounded excerpt rather than `oversized`.

### Task 2: Move Excerpting into the Shared Output Helper

**Files:**
- Modify: `crates/domains/ironclaw_memory/src/service.rs:660-688,822-948`
- Modify: `crates/extensions/packages/memory-native/src/service.rs:48-54,120-127,343-481,974-1220`

- [ ] **Step 1: Make `search_response_output` consume raw results and bound content**

Replace its body with this ownership-preserving shape:

```rust
pub fn search_response_output(response: MemoryServiceSearchResponse) -> Value {
    let MemoryServiceSearchResponse { query, results } = response;
    let results = results
        .into_iter()
        .map(|result| {
            json!({
                "content": bound_search_result_content(result.content, &query),
                "score": result.score,
                "path": result.path,
                "is_hybrid_match": result.is_hybrid_match,
            })
        })
        .collect::<Vec<_>>();
    let result_count = results.len();
    json!({
        "query": query,
        "results": results,
        "result_count": result_count,
        "search_scope": MEMORY_SEARCH_SCOPE,
        "external_services_searched": false,
    })
}
```

This returns short owned strings without cloning and only allocates a replacement for oversized excerpts.

- [ ] **Step 2: Move the excerpt policy and algorithm into `ironclaw_memory`**

Place these private items immediately after `search_response_output`:

```rust
const MAX_SEARCH_RESULT_CONTENT_BYTES: usize = 8 * 1024;
const SEARCH_EXCERPT_PRE_BYTES: usize = 128;
const SEARCH_EXCERPT_POST_BYTES: usize = 256;
const SEARCH_EXCERPT_DELIMITER: &str = "\n…\n";

fn bound_search_result_content(content: String, query: &str) -> String {
    if content.len() <= MAX_SEARCH_RESULT_CONTENT_BYTES {
        return content;
    }
    if query.is_empty() {
        // The empty query matches everywhere; excerpting would degenerate to
        // the whole content. Keep the plain head.
        return bounded_search_head(content, MAX_SEARCH_RESULT_CONTENT_BYTES);
    }
    bounded_search_excerpts(&content, query)
        .unwrap_or_else(|| bounded_search_head(content, MAX_SEARCH_RESULT_CONTENT_BYTES))
}

fn bounded_search_excerpts(content: &str, query: &str) -> Option<String> {
    let mut out = String::new();
    let mut search_from = 0usize;
    let mut previous_end = 0usize;
    while let Some(relative) = content[search_from..].find(query) {
        let position = search_from + relative;
        let query_end = position + query.len();
        let mut desired_start = position.saturating_sub(SEARCH_EXCERPT_PRE_BYTES);
        while desired_start < position && !content.is_char_boundary(desired_start) {
            desired_start += 1;
        }
        let mut desired_end = (query_end + SEARCH_EXCERPT_POST_BYTES).min(content.len());
        while !content.is_char_boundary(desired_end) {
            desired_end -= 1;
        }
        if desired_start < previous_end && desired_end <= previous_end {
            // Fully covered by the previous excerpt; the query context
            // is already present.
            search_from = query_end;
            continue;
        }

        let contiguous = !out.is_empty() && desired_start <= previous_end;
        let delimiter_len = if out.is_empty() || contiguous {
            0
        } else {
            SEARCH_EXCERPT_DELIMITER.len()
        };
        let Some(available) = MAX_SEARCH_RESULT_CONTENT_BYTES
            .checked_sub(out.len())
            .and_then(|remaining| remaining.checked_sub(delimiter_len))
        else {
            break;
        };
        let mut start = if contiguous {
            previous_end
        } else {
            let Some(max_pre) = available.checked_sub(query.len()) else {
                break;
            };
            desired_start.max(position.saturating_sub(max_pre))
        };
        while start < position && !content.is_char_boundary(start) {
            start += 1;
        }
        if query_end.saturating_sub(start) > available {
            break;
        }

        let mut end = desired_end.min(start + available);
        while end > query_end && !content.is_char_boundary(end) {
            end -= 1;
        }
        if delimiter_len > 0 {
            out.push_str(SEARCH_EXCERPT_DELIMITER);
        }
        out.push_str(&content[start..end]);
        previous_end = end;
        search_from = query_end;
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

fn bounded_search_head(content: String, bound: usize) -> String {
    let mut content = content;
    let mut end = bound.min(content.len());
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    content.truncate(end);
    content
}
```

Retain the existing detailed rustdoc from the native implementation, updating names and responsibility wording from “provider” to “conventional tool output.”

- [ ] **Step 3: Move the algorithm tests without weakening them**

Move these existing native unit tests and their `QUERY`, `MIB`, marker constants, `mib_body`, and `mib_body_with` fixtures into `ironclaw_memory::service::tests`, renaming calls to `bound_search_result_content` and constants to the shared names:

```text
small_snippet_is_returned_unchanged
empty_snippet_is_returned_unchanged
snippet_exactly_at_bound_is_returned_unchanged
snippet_one_byte_over_bound_is_cut_to_exact_cap
oversized_body_with_exact_literal_query_retains_head_match
no_exact_occurrence_falls_back_to_bounded_head
exact_literal_occurrence_beyond_head_is_retained
exact_query_near_cap_keeps_the_full_match_beyond_head
overlapping_excerpt_windows_preserve_contiguous_source
matching_is_exact_literal_not_case_folded
oversized_1mib_multiple_matches_retain_each_query_context
many_occurrences_stop_at_cap_with_complete_prefix
multibyte_excerpt_windows_never_split_chars
empty_query_keeps_bounded_head
query_longer_than_cap_falls_back_to_head
multibyte_head_cut_never_splits_chars
truncated_preview_is_deterministic
```

Preserve every existing assertion. The move changes ownership, not behavior.

- [ ] **Step 4: Restore raw native provider mapping**

Change the native search result mapping to:

```rust
.map(|result| MemoryServiceSearchResult {
    is_hybrid_match: result.is_hybrid(),
    content: result.snippet,
    score: result.score,
    path: result.path.relative_path().to_string(),
})
```

Delete `MAX_SEARCH_SNIPPET_BYTES`, `EXCERPT_PRE_BYTES`, `EXCERPT_POST_BYTES`, `EXCERPT_DELIMITER`, `bound_search_snippet`, `bounded_excerpts`, `bounded_head`, and the moved native unit-test block.

- [ ] **Step 5: Run the focused red tests and algorithm suite**

Run:

```bash
cargo test -p ironclaw_memory search_output_bounds_oversized_content_around_exact_query
cargo test -p ironclaw_memory_native native_search_preserves_oversized_provider_result
cargo test -p ironclaw_memory service::tests
```

Expected: all pass. The shared output contains `needle` within 8192 bytes; the direct native response equals the complete original body; all moved algorithm edge cases remain green.

- [ ] **Step 6: Commit the boundary move**

```bash
git add crates/domains/ironclaw_memory/src/service.rs \
  crates/extensions/packages/memory-native/src/service.rs \
  crates/extensions/packages/memory-native/tests/memory_service_contract.rs
git commit -m "fix(memory): bound conventional search tool output"
```

### Task 3: Pin the Production Handler Path

**Files:**
- Modify: `crates/kernel/ironclaw_host_runtime/src/first_party_tools/memory.rs:610-647`

- [ ] **Step 1: Strengthen the existing write-then-search handler test**

In `handler_serves_write_then_search_over_the_request_filesystem`, build content larger than the per-result bound with the query beyond the head:

```rust
const QUERY: &str = "handler marker heron";
const RESULT_BOUND: usize = 8 * 1024;
let position = RESULT_BOUND + 512;
let mut content = "a".repeat(position + QUERY.len() + RESULT_BOUND);
content.replace_range(position..position + QUERY.len(), QUERY);
```

Pass `content` to the existing write request and search for `QUERY`. After the existing output metadata assertions, add:

```rust
let result_content = search.output["results"][0]["content"]
    .as_str()
    .expect("handler search content is a string");
assert!(result_content.len() <= RESULT_BOUND);
assert!(result_content.contains(QUERY));
```

This exercises the production-shaped guard → native service → shared wire-output helper path. Do not add a second handler test for the same scenario.

- [ ] **Step 2: Run the handler regression**

Run:

```bash
cargo test -p ironclaw_host_runtime handler_serves_write_then_search_over_the_request_filesystem
```

Expected: PASS with one bounded search result retaining the exact query beyond the first 8192 bytes.

- [ ] **Step 3: Commit caller-level coverage**

```bash
git add crates/kernel/ironclaw_host_runtime/src/first_party_tools/memory.rs
git commit -m "test(memory): pin bounded search handler output"
```

### Task 4: Verify Compatibility and Architecture

**Files:**
- Verify unchanged: `crates/app/ironclaw_composition/src/memory_provider_factory.rs:384-393`
- Verify unchanged: `crates/extensions/packages/memory-native/schemas/memory/search.output.v1.json`
- Verify unchanged: mem0 conventional search schema and handler call path
- Include: `docs/internal/plans/2026-08-09-memory-search-wire-output-bounding-design.md`
- Include: `docs/internal/superpowers/plans/2026-08-09-memory-search-wire-output-bounding.md`

- [ ] **Step 1: Format**

Run:

```bash
cargo fmt --all
```

Expected: exit 0.

- [ ] **Step 2: Run affected package tests**

Run:

```bash
cargo test -p ironclaw_memory \
  -p ironclaw_memory_native \
  -p ironclaw_memory_mem0 \
  -p ironclaw_host_runtime
```

Expected: all tests pass.

- [ ] **Step 3: Run zero-warning clippy**

Run:

```bash
cargo clippy \
  -p ironclaw_memory \
  -p ironclaw_memory_native \
  -p ironclaw_memory_mem0 \
  -p ironclaw_host_runtime \
  --all-targets --all-features -- -D warnings
```

Expected: exit 0 with no warnings.

- [ ] **Step 4: Run the architecture gate**

Run:

```bash
cargo test -p ironclaw_architecture_tests
```

Expected: all tests pass; no forbidden dependency or ownership edge was introduced.

- [ ] **Step 5: Validate documentation changes**

Run the publication-boundary check:

```bash
python3 scripts/ci/docs_publication_boundary.py
```

Expected: exit 0; both design artifacts remain under the internal documentation fence.

Test the documentation with Mintlify:

```bash
cd docs && mint dev
cd docs && mint broken-links
```

Expected: the site builds locally and all internal links resolve.

- [ ] **Step 6: Commit design and plan artifacts**

```bash
git add docs/internal/plans/2026-08-09-memory-search-wire-output-bounding-design.md \
  docs/internal/superpowers/plans/2026-08-09-memory-search-wire-output-bounding.md
git commit -m "docs(memory): record search output boundary"
```

- [ ] **Step 7: Inspect the scoped diff and push**

```bash
git diff origin/main...HEAD -- \
  crates/domains/ironclaw_memory/src/service.rs \
  crates/extensions/packages/memory-native/src/service.rs \
  crates/extensions/packages/memory-native/tests/memory_service_contract.rs \
  crates/kernel/ironclaw_host_runtime/src/first_party_tools/memory.rs \
  docs/internal/plans/2026-08-09-memory-search-wire-output-bounding-design.md \
  docs/internal/superpowers/plans/2026-08-09-memory-search-wire-output-bounding.md
git push fork issue-7360-memory-search-snippets
```

Expected: only the responsibility move, preserved algorithm/tests, raw native contract, handler regression, and internal design artifacts are present; the push updates PR #7436.

## Review Follow-up

Reply to Ben Kurrek's thread with the concrete boundary: provider conventional search responses are raw `MemoryServiceSearchResponse` values; `ironclaw_memory::search_response_output` applies the 8 KiB query-aware raw-content bound before JSON serialization for both native and mem0 conventional tool handlers. Include the focused test and CI evidence. Do not claim cursor-based large-artifact navigation; that remains #7441.

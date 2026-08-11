# Memory Search Wire-Output Bounding Design

## Problem

The conventional `ironclaw.memory.search` tool can receive complete stored document bodies from a memory provider. Returning those bodies unchanged can exceed the model-tool output budget. PR #7436 currently bounds results inside the native provider, but output size is a property of the model-callable tool contract, not native-memory storage or search semantics.

Provider-level bounding also creates inconsistent behavior: native search is truncated while another provider can return the same conventional tool shape unbounded. It also changes direct provider search callers that do not cross the model-tool boundary.

## Decision

Bound conventional memory-search result content in `ironclaw_memory::search_response_output`.

This provider-neutral helper already converts `MemoryServiceSearchResponse` into the conventional model-visible JSON shape. Both production handler paths call it, and both native and mem0 providers use those paths. The helper will apply the existing query-aware, UTF-8-safe 8 KiB per-result excerpt before serializing each result.

Provider implementations will continue returning their raw search results. Direct provider search callers and internal context-retrieval paths therefore retain their current semantics.

```text
native provider ─┐
                 ├─ raw MemoryServiceSearchResponse
mem0 provider ───┘              │
                                ▼
             ironclaw_memory::search_response_output
             - exact-query-aware excerpt
             - UTF-8-safe 8 KiB raw content/result
                                │
                                ▼
                    model-visible tool JSON
```

## Scope

### Move into `ironclaw_memory`

- The 8 KiB conventional search-result content budget.
- The query-aware excerpt-selection helper.
- Deterministic merging of overlapping or touching excerpts.
- UTF-8-safe truncation and fallback behavior.
- Unit coverage for exact query preservation, repeated matches, merged windows, UTF-8 boundaries, and no-match fallback.

### Remove from the native provider

- Native-only search excerpt constants and helper functions.
- Native-only output bounding inside `NativeMemoryService::search`.
- Tests that incorrectly define bounded content as a native provider service contract.

### Preserve

- Stored document content.
- Backend search, ranking, indexing, scores, paths, and hybrid-match flags.
- Internal context retrieval.
- Direct non-tool provider search operation responses.
- The existing conventional search JSON fields and schema.

## Data Flow

1. A conventional memory-search handler parses model input into `MemoryServiceSearchRequest`.
2. The selected provider executes its conventional search operation and returns a raw `MemoryServiceSearchResponse`.
3. `search_response_output` receives the response.
4. For each result, it computes an excerpt using the resolved query and the 8 KiB content budget.
5. It serializes the bounded content with the provider's unchanged score, path, and hybrid-match flag.
6. Existing host output guards remain the final general-purpose safety net; they are not the primary search-result contract.

## Edge Cases and Errors

- If content is already within 8 KiB, return it byte-for-byte without allocation beyond existing serialization needs.
- If an exact query occurrence fits, preserve the occurrence and as much surrounding context as the budget allows.
- If several occurrences fit, include deterministic non-overlapping excerpts; merge touching or overlapping windows before rendering separators.
- If no complete exact occurrence can fit, return a bounded UTF-8-safe document head.
- Never split a UTF-8 code point.
- Empty provider content remains empty.
- Excerpting is infallible and introduces no new model-visible error variant.

## Verification

- Move the excerpt algorithm's behavioral tests to `ironclaw_memory`.
- Add or extend a model-callable handler regression so a large native document with an exact match beyond the head produces a bounded result that retains the match.
- Keep provider conformance tests asserting provider-neutral search semantics rather than model-output truncation.
- Run narrow tests for `ironclaw_memory`, `ironclaw_memory_native`, `ironclaw_memory_mem0`, and the handler-owning crate.
- Run formatting and zero-warning clippy for affected crates.
- Run `ironclaw_architecture_tests` because the responsibility moves across crate-family boundaries.
- Run `cd docs && mint dev` and `cd docs && mint broken-links` to validate the documentation changes.

## Compatibility and Rollback

The conventional tool JSON schema is unchanged. Native model-tool output remains bounded; mem0 gains the same bound. Direct native service callers regain raw result content, matching the provider-neutral contract.

Rollback is a code-only revert of the shared helper change and provider cleanup. No persistence schema, stored data, configuration, or migration changes are involved.

## Not Covered

This change does not add document pagination, cursors, byte-range reads, path-scoped search, continuation tokens, or exhaustive large-artifact navigation. Those remain tracked separately in #7441.

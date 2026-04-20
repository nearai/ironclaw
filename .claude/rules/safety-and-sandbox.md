---
paths:
  - "src/safety/**"
  - "src/sandbox/**"
  - "src/secrets/**"
  - "src/tools/wasm/**"
  - "src/bridge/**"
  - "src/channels/**"
  - "src/workspace/**"
  - "src/agent/**"
  - "crates/ironclaw_engine/**"
---
# Safety Layer & Sandbox Rules

## Safety Layer

All external tool output passes through `SafetyLayer`:
1. **Sanitizer** - Detects injection patterns, escapes dangerous content
2. **Validator** - Checks length, encoding, forbidden patterns
3. **Policy** - Rules with severity (Critical/High/Medium/Low) and actions (Block/Warn/Review/Sanitize)
4. **Leak Detector** - Scans for 15+ secret patterns at two points: tool output before LLM, and LLM responses before user

Tool outputs are wrapped in `<tool_output>` XML before reaching the LLM.

## Shell Environment Scrubbing

The shell tool scrubs sensitive env vars before executing commands. The sanitizer detects command injection patterns (chained commands, subshells, path traversal).

## Sandbox Policies

| Policy | Filesystem | Network |
|--------|-----------|---------|
| ReadOnly | Read-only workspace | Allowlisted domains |
| WorkspaceWrite | Read-write workspace | Allowlisted domains |
| FullAccess | Full filesystem | Unrestricted |

## Zero-Exposure Credential Model

Secrets are stored encrypted on the host and injected into HTTP requests by the proxy at transit time. Container processes never see raw credential values.

## Every New Ingress Scans Before Storage or LLM

Mirror of CLAUDE.md's "Everything Goes Through Tools" rule: every new surface that accepts external data into the system — user messages, webhook payloads, memory writes, URL fetches, file ingestion — must run the matching safety scan on the **pre-transform, pre-injection** payload before the data reaches the LLM or the database.

Recurring bug shape: a new code path is added, and the safety scan is skipped, applied post-injection (too late), or applied to the wrong stage. References: #2491 Engine v2 inbound, #2676 WASM URL post-injection, #2470 memory write layer.

Rules:

- **Inbound user text** → `safety_layer.scan_inbound_for_secrets()` before engine dispatch.
- **Tool output** (existing) → sanitize + leak detector before LLM, wrapped in `<tool_output>` XML.
- **LLM response** → leak detector before user delivery.
- **Memory / workspace writes** → injection scan on the pre-storage value. Never on the transformed/rendered value.
- **URL fetches** → leak-pattern scan on the resolved URL **before** credential injection; not on the post-injection URL.

A newly added ingress handler (HTTP route, webhook receiver, `Channel::send_message` impl) that reaches an LLM call or DB write without calling a `safety_layer.*` function on the payload is a review-blocker.

## Bounded Resources

User-controlled inputs must not grow unbounded. Apply caps at the boundary:

- **Interners, caches, accumulators** — hard size limit (entries + total bytes), eviction policy documented. PR #2673 model-name interner: 256-byte value cap, 1024-entry cap.
- **File reads in HTTP handlers** — stream with `tokio::fs::File::open` + `ReaderStream`; never `tokio::fs::read`, which buffers the whole file. Reference: #2633 item 3.
- **Fan-out scans (portfolio addresses, batch tool calls)** — position cap + O(n) algorithm required, not O(n²). Tool-specific fuel limits, not global raises. Reference: #2710 portfolio tool.
- **Tokio task fan-out** — in-flight dedup or bounded semaphore on spawns driven by user input (PR #2702).

## Cache Keys Must Be Complete

A cache whose stored value depends on input X must include a stable representation of X in its key. `WorkspacePool` keyed on `user_id` but applying token-specific `workspace_read_scopes` before caching (#2633 item 1) froze the first token's scopes for every subsequent request — canonical example. Rule: if `get_or_create(a, b)` inserts using only `a` but `b` affects the stored value, that is a bug.

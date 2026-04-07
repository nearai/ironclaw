---
title: "fix: Multi-tenant security hardening — credential isolation, URL rewrite removal, skill ownership"
type: fix
status: active
date: 2026-04-07
origin: "GitHub issues #2056, #2068, #2069, #2070-2074, #2085"
---

# Multi-Tenant Security Hardening

## Overview

IronClaw has a cluster of related security and multi-tenant isolation issues that must be fixed before any production multi-tenant deployment. These span credential leakage (#2068, #2069), a production security hole (#2056), WASM channel secret ownership (#2070), MCP/extension state partitioning (#2071, #2072), config fallback warnings (#2073), dynamic tool ownership (#2074), and skill isolation (#2085).

This plan groups them into 6 implementation units ordered by severity: security-critical first, then ownership model fixes, then feature work.

## Problem Frame

In a multi-tenant IronClaw deployment (e.g., behind a web gateway with multiple users):
1. **Credential cross-contamination** — WASM tools silently fall back to "default" user's secrets; sandbox jobs always use the owner's credentials regardless of who triggered them
2. **URL hijacking vulnerability** — Production binaries contain env-var-controlled API URL rewriters for Telegram/Slack, allowing credential exfiltration
3. **Shared mutable state** — Skills, MCP sessions, extensions, and dynamic tools are not partitioned by user; one user's actions affect all others

## Requirements Trace

- R1. [#2056] Production binaries must not contain test URL rewrite functions or read test env vars
- R2. [#2069] WASM tool credential lookup must fail with a clear error when user lacks credentials, never fall back to "default"
- R3. [#2068] Sandbox job credential lookups must use the job creator's identity, not a global owner
- R4. [#2070] WASM channel secret ownership must use the channel's actual owner, not hardcoded "default"
- R5. [#2071, #2072] MCP sessions and ExtensionManager client state must be partitioned by user
- R6. [#2073] Implicit fallback to owner_id "default" must log a warning in all remaining sites
- R7. [#2074] Dynamic tools table must include user_id column for per-user ownership
- R8. [#2085] Skill install/remove must be scoped to user's project; admin skills visible to all but not deletable by non-admins

## Scope Boundaries

- Not refactoring the broader channel runtime architecture beyond transport injection
- Not changing the E2E test fixture setup in `tests/e2e/`
- Not addressing `pub fn for_testing()` constructors elsewhere (separate issue)
- Not implementing full RBAC — just per-user isolation with admin/non-admin distinction
- Engine v2 orchestrator changes out of scope (only v1 orchestrator state)

## Context & Research

### Relevant Code

- `src/channels/wasm/wrapper.rs` — WASM channel HTTP host function with URL rewriters (lines ~68, ~386, ~3130, ~3976)
- `src/extensions/manager.rs:263` — Duplicate test URL env var constant
- `src/tools/wasm/wrapper.rs:1540-1568` — "default" credential fallback in WASM tool execution
- `src/orchestrator/api.rs:39-57` — `OrchestratorState` with global `user_id: String`
- `src/orchestrator/api.rs:446+` — `get_credentials_handler` using `state.user_id`
- `src/tools/builtin/skill_tools.rs` — skill_install / skill_remove tools
- `src/tools/wasm/wrapper.rs:1049` — Broadcast metadata "default" fallback
- `src/tools/wasm/wrapper.rs:3130` — `resolve_websocket_identify_message` hardcoded "default"
- `src/tools/wasm/http_security.rs` — SSRF builder pattern (model for transport injection)
- `src/llm/provider.rs::LlmProvider` — Clean trait-injection pattern

### Existing Patterns

- `LlmProvider` trait: clean dependency injection, tests substitute with mock
- `SecretsStore` trait: `get_decrypted(owner_id, secret_name)` already supports per-user lookup
- `job_owner_cache` in `OrchestratorState`: already exists but not used for credential lookups
- `resolve_user_project()`: per-user project already implemented but skill install/remove doesn't use it

## Key Technical Decisions

- **#2056: `#[cfg(test)]` gate as immediate fix, transport trait as follow-up** — The immediate Layer 1 fix (cfg-gating) can ship in hours and is sufficient to close the security hole. The architectural Layer 2 (transport injection) is better engineering but can follow as a separate PR
- **#2069: Fail-hard, no fallback** — When a user lacks credentials, return a clear error rather than silently using someone else's keys. This may break existing single-tenant setups that rely on "default" — mitigate by logging a deprecation warning first for one release, then hard-fail
- **#2068: Use `job_owner_cache` for credential routing** — The cache already maps job_id → user_id. Use it in `get_credentials_handler` instead of the global `state.user_id`
- **#2085: MemoryDoc-scoped skills** — Skill install/remove should write `DocType::Skill` MemoryDocs scoped to the user's project, not the shared filesystem

## Implementation Units

- [ ] **Unit 1: [CRITICAL] Remove test URL rewriters from production code (#2056)**

**Goal:** Production release binaries contain zero test URL rewrite functions or env var lookups

**Requirements:** R1

**Dependencies:** None — do this first

**Files:**
- Modify: `src/channels/wasm/wrapper.rs`
- Modify: `src/extensions/manager.rs`
- Test: existing `tests/telegram_auth_integration.rs`, `tests/slack_auth_integration.rs`

**Approach:**
- Wrap `rewrite_telegram_api_url_for_testing()`, `rewrite_slack_api_url_for_testing()`, and their constants with `#[cfg(any(test, debug_assertions))]`
- Wrap the call site at `wrapper.rs:386` with `#[cfg(any(test, debug_assertions))]` — in release builds, `logical_url` is used directly with no rewrite
- Remove the duplicate `TELEGRAM_TEST_API_BASE_ENV` const in `extensions/manager.rs` or gate it the same way
- Verify: `cargo build --release && nm target/release/ironclaw | grep -i rewrite_.*_for_testing` returns zero

**Test scenarios:**
- Happy path: All existing E2E telegram/slack tests pass (they run in test/debug mode, so cfg-gated code is included)
- Security: Release binary does not contain rewrite symbols (nm check)
- Edge case: Setting `IRONCLAW_TEST_TELEGRAM_API_BASE_URL` in a release build has no effect

**Verification:**
- `nm target/release/ironclaw | grep -i rewrite_.*_for_testing` returns 0 results
- `grep -rn "IRONCLAW_TEST_" src/` only shows `#[cfg(test)]` or `#[cfg(debug_assertions)]` gated hits

---

- [ ] **Unit 2: [CRITICAL] Remove "default" credential fallback in WASM tools (#2069)**

**Goal:** WASM tool credential lookups fail with a clear error instead of falling back to "default" user

**Requirements:** R2

**Dependencies:** None

**Files:**
- Modify: `src/tools/wasm/wrapper.rs` (lines ~1540-1568, ~1049, ~3130)
- Test: `src/tools/wasm/wrapper.rs` (tests module)

**Approach:**
- Remove the `if user_id != "default" { store.get_decrypted("default", ...) }` fallback block at lines 1558-1561
- When credential lookup fails, return an error: `"credential '{name}' not found for user '{user_id}'; configure via ironclaw secrets set"`
- Fix broadcast metadata fallback at line ~1049: use `owner_scope_id` instead of "default"
- Fix `resolve_websocket_identify_message` at line ~3130: use channel's owner from context, not hardcoded "default"
- Grep for remaining `"default"` fallbacks in `src/tools/wasm/` and fix them

**Test scenarios:**
- Happy path: WASM tool with valid user credential succeeds
- Error path: WASM tool with missing user credential returns clear error, does NOT fall back
- Edge case: Single-tenant setup with owner_id "default" still works (credential is looked up for "default" user normally)

**Verification:**
- `grep -n '"default"' src/tools/wasm/wrapper.rs` returns zero non-test hits for credential lookups

---

- [ ] **Unit 3: [CRITICAL] Thread job creator identity through sandbox credentials (#2068)**

**Goal:** Sandbox job credential lookups use the job creator's user_id, not the global owner

**Requirements:** R3

**Dependencies:** None

**Files:**
- Modify: `src/orchestrator/api.rs` — `get_credentials_handler`, `OrchestratorState`
- Modify: `src/orchestrator/mod.rs` — state construction
- Test: `src/orchestrator/api.rs` (tests module)

**Approach:**
- In `get_credentials_handler`: resolve the requesting job's owner from `state.job_owner_cache` (already populated at job creation), use that user_id for `secrets.get_decrypted()`
- If job_id not found in cache, fall back to `state.user_id` with a warning log (graceful degradation for in-flight jobs)
- Consider removing `user_id` from `OrchestratorState` entirely if it's no longer needed as the default — or rename to `default_owner_id` to make intent explicit

**Test scenarios:**
- Happy path: User A creates sandbox job → credential lookup uses user A's secrets
- Happy path: User B creates sandbox job → credential lookup uses user B's secrets, not user A's
- Error path: Job not in cache → falls back to default owner with warning log
- Integration: Two concurrent sandbox jobs from different users → each gets own credentials

**Verification:**
- `get_credentials_handler` resolves user_id from job context, not global state

---

- [ ] **Unit 4: Fix WASM channel secret ownership (#2070)**

**Goal:** WASM channel secret lookups use the channel's owner, not hardcoded "default"

**Requirements:** R4

**Dependencies:** Unit 2 (same file, avoid merge conflicts)

**Files:**
- Modify: `src/channels/wasm/wrapper.rs` — credential injection paths

**Approach:**
- Identify all `get_decrypted("default", ...)` calls in `src/channels/wasm/wrapper.rs`
- Replace with the channel's actual owner scope (available from `ChannelStoreData.owner_id` or equivalent context)
- Add a `#[cfg(test)]` test that verifies channel credential lookup uses the owner from config, not "default"

**Test scenarios:**
- Happy path: WASM channel owned by user X looks up credentials under user X
- Error path: Credential not found for channel owner → clear error
- Edge case: Legacy single-tenant config with owner "default" still works

**Verification:**
- `grep -n 'get_decrypted.*"default"' src/channels/wasm/wrapper.rs` returns zero non-test hits

---

- [ ] **Unit 5: MCP session partitioning + dynamic tool ownership (#2071-2074)**

**Goal:** MCP sessions, extensions, and dynamic tools are partitioned by user identity

**Requirements:** R5, R6, R7

**Dependencies:** Unit 3 (user identity threading pattern established)

**Files:**
- Modify: `src/extensions/manager.rs` — partition MCP client state by user
- Modify: `src/tools/registry.rs` — dynamic tool user_id support
- Modify: DB migration — add `user_id` column to dynamic tools table
- Modify: config resolution paths — add deprecation warnings for "default" fallback

**Approach:**
- **#2071/#2072**: ExtensionManager MCP client state keyed by `(user_id, server_name)` instead of just `server_name`. When a user connects an MCP server, their session is isolated
- **#2073**: In all remaining config resolution paths that fall back to "default", add `tracing::warn!("implicit owner_id fallback to 'default' — configure OWNER_ID explicitly")`
- **#2074**: Add `user_id` column to dynamic_tools table (nullable for backwards compat), filter queries by user_id when present

**Test scenarios:**
- Happy path: Two users each connect a different MCP server → sessions are independent
- Happy path: Dynamic tool created by user A is not visible to user B
- Edge case: Legacy tools without user_id remain visible to all (nullable column)
- Edge case: "default" fallback logs a warning but does not fail

**Verification:**
- MCP sessions isolated per user
- Dynamic tools filtered by user_id
- `grep -rn '"default"' src/ | grep -v cfg.test | grep -v warn` shows no silent fallbacks

---

- [ ] **Unit 6: Per-user skill isolation (#2085)**

**Goal:** Skill install/remove scoped to user's project; admin skills visible to all but not deletable by non-admins

**Requirements:** R8

**Dependencies:** Unit 5 (user identity in tool registry established)

**Files:**
- Modify: `src/tools/builtin/skill_tools.rs` — skill_install, skill_remove
- Modify: `src/bridge/router.rs` — resolve_user_project integration
- Test: skill tool tests

**Approach:**
- `skill_install`: Write `DocType::Skill` MemoryDoc to user's project scope (from `resolve_user_project`), not shared `~/.ironclaw/installed_skills/`
- `skill_remove`: Only remove skills from user's own project. If skill belongs to admin/shared scope, return error "cannot remove admin skill"
- Skill visibility: When building the skill context for a user, merge admin-project skills + user-project skills. Admin skills take precedence on name collision
- Prerequisite: #2084 fix must be in place (shared skills visible across projects)

**Test scenarios:**
- Happy path: User installs skill → visible only to that user
- Happy path: Admin skill visible to all users
- Error path: Non-admin tries to delete admin skill → error
- Edge case: User installs skill with same name as admin skill → user's version used for that user, admin version for others

**Verification:**
- Skill install/remove only affects user's own project
- Admin skills visible across all user projects

---

## System-Wide Impact

- **Credential flow**: After Units 2-4, all credential lookups are scoped by user identity — no more silent cross-user fallback
- **Backwards compatibility**: Single-tenant deployments with owner "default" continue to work because credentials are stored under "default" and looked up under "default" — the fix removes the fallback, not the normal path
- **MCP sessions**: After Unit 5, MCP connections are per-user. Users connecting the same MCP server get independent sessions
- **Release binary size**: Unit 1 removes test code from release builds — binary may be slightly smaller
- **Migration required**: Unit 5 adds a DB column (dynamic_tools.user_id) — requires migration

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Removing "default" fallback breaks single-tenant setups | Normal single-tenant lookup for "default" user still works; only cross-user fallback is removed |
| MCP session partitioning increases memory | Per-user sessions only created on demand; idle users have no sessions |
| #2084 fix is a prerequisite for Unit 6 | #2084 already has PR #2086 — verify it's merged before starting Unit 6 |
| Dynamic tools migration on large DBs | Nullable column addition is fast; backfill user_id separately |

## Sources & References

- #2056: [SECURITY issue with full analysis](https://github.com/nearai/ironclaw/issues/2056)
- #2068: [Orchestrator credential threading](https://github.com/nearai/ironclaw/issues/2068)
- #2069: [WASM default credential fallback](https://github.com/nearai/ironclaw/issues/2069)
- #2070-2074: [Ownership model issues](https://github.com/nearai/ironclaw/issues/2070)
- #2085: [Skill isolation](https://github.com/nearai/ironclaw/issues/2085)
- Ownership model design spec: `docs/superpowers/specs/2026-04-01-ownership-model-design.md`
- SSRF builder pattern: `src/tools/wasm/http_security.rs`
- LlmProvider injection pattern: `src/llm/provider.rs`

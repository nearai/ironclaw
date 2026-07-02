# Persistent User Context (Agent-Context Profile) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give IronClaw Reborn a per-user, always-injected agent-context profile (timezone, locale, location) the model can read every turn and update via a typed capability, plus the freeform `USER.md` prose already injected.

**Architecture:** Three changes across clear ownership boundaries. (1) `ironclaw_turns` gains a plain `UserProfileContext` struct + a `user_profile` field on `LoopRuntimeContext`, rendered into the model prompt. (2) A `HostUserProfileSource` trait (in `ironclaw_loop_support`, impl in `ironclaw_host_runtime`) reads `context/profile.json` and is wired into the loop-driver host factory to fill `user_timezone` + `user_profile` at loop start — mirroring the existing `HostIdentityContextSource` pattern so `ironclaw_reborn` never imports `ironclaw_memory`. (3) A new `builtin.profile_set` first-party capability does typed-validated, field-merge writes to `context/profile.json`.

**Tech Stack:** Rust, tokio, `chrono` / `chrono-tz`, `serde_json`, the Reborn memory backend (`ironclaw_memory`), first-party capability framework (`ironclaw_host_runtime`).

**Spec:** `docs/superpowers/specs/2026-06-16-persistent-user-context-profile-design.md`

---

## Design invariants (apply to every task)

- **Never guess a timezone.** A missing/invalid value → `None` → "unknown", never a host default (mirrors the existing `user_timezone` doc comment, `runtime_context.rs:16`).
- **Profile scope is `(tenant, user, agent=None, project=None)`** regardless of the run's scope. All path construction goes through one helper (Task 4) so the decision lives in one place.
- **Logging:** background/producer diagnostics use `debug!`, never `info!`/`warn!` (REPL/TUI corruption rule).
- **`ironclaw_turns` must not import `ironclaw_memory`** (forbidden, `reborn_dependency_boundaries.rs:2568`). `UserProfileContext` carries primitives only.
- **System config is excluded.** `profile_set`'s field enum cannot name provider/model/approval (spec §9).
- After any cross-crate API/boundary change, run `cargo test -p ironclaw_architecture`.

---

## File map

| File | Change | Responsibility |
|---|---|---|
| `crates/ironclaw_turns/src/run_profile/runtime_context.rs` | modify | `UserProfileContext`, `Locale`, `LoopRuntimeContext.user_profile`, render line + elicitation hint |
| `crates/ironclaw_loop_support/src/user_profile_context.rs` | create | `HostUserProfileSource` trait + `EmptyUserProfileSource` default |
| `crates/ironclaw_loop_support/src/lib.rs` | modify | module decl + re-export |
| `crates/ironclaw_host_runtime/src/user_profile_source.rs` | create | production `HostUserProfileSource` impl (reads `context/profile.json`) |
| `crates/ironclaw_host_runtime/src/lib.rs` | modify | module decl + export |
| `crates/ironclaw_host_runtime/src/first_party_tools/profile_set.rs` | create | `builtin.profile_set` capability |
| `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs` | modify | manifest + handler + dispatch wiring |
| `crates/ironclaw_host_runtime/src/first_party_tools/schemas.rs` | modify | `profile_set.input.v1.json` schema arm |
| `src/workspace/reborn_identity_context.rs` | modify | drop `context/profile.json` from prose identity allow-list (Task 3B) |
| `crates/ironclaw_reborn/src/loop_driver_host.rs` | modify | `with_user_profile_source` builder + fill at construction site (~1563) |
| `crates/ironclaw_reborn_composition/src/...` | modify | wire production source into the factory |
| `crates/ironclaw_reborn/tests/` or composition tests | create | caller-level integration test (both DB backends) |

---

## Task 1: `UserProfileContext` type + render in `ironclaw_turns`

**Files:**
- Modify: `crates/ironclaw_turns/src/run_profile/runtime_context.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block (after `renders_unknown_timezone_fallback`, ~line 490). Uses existing `stamp()` helper.

```rust
fn profile(locale: Option<&str>, location: Option<&str>) -> UserProfileContext {
    UserProfileContext {
        locale: locale.and_then(|s| Locale::new(s).ok()),
        location: location.map(str::to_string),
    }
}

#[test]
fn renders_user_profile_line_when_present() {
    let ctx = LoopRuntimeContext {
        loop_started_at_utc: stamp(),
        user_timezone: None,
        communication: None,
        product_context: None,
        user_profile: Some(profile(Some("ja-JP"), Some("Tokyo, Japan"))),
    };
    let text = ctx.render_model_content();
    assert!(text.contains("User profile:"), "missing profile line: {text}");
    assert!(text.contains("locale=ja-JP"), "{text}");
    assert!(text.contains("location=Tokyo, Japan"), "{text}");
}

#[test]
fn omits_user_profile_line_when_absent() {
    let ctx = LoopRuntimeContext {
        loop_started_at_utc: stamp(),
        user_timezone: None,
        communication: None,
        product_context: None,
        user_profile: None,
    };
    assert!(!ctx.render_model_content().contains("User profile:"));
}

#[test]
fn omits_unset_profile_fields() {
    let ctx = LoopRuntimeContext {
        loop_started_at_utc: stamp(),
        user_timezone: None,
        communication: None,
        product_context: None,
        user_profile: Some(profile(Some("en-US"), None)),
    };
    let text = ctx.render_model_content();
    assert!(text.contains("locale=en-US"), "{text}");
    assert!(!text.contains("location="), "unset location must not render: {text}");
}

#[test]
fn unknown_timezone_hint_mentions_profile_set() {
    let ctx = LoopRuntimeContext {
        loop_started_at_utc: stamp(),
        user_timezone: None,
        communication: None,
        product_context: None,
        user_profile: None,
    };
    let text = ctx.render_model_content();
    assert!(text.contains("profile_set"), "elicitation hint must mention profile_set: {text}");
}

#[test]
fn render_sanitizes_profile_location() {
    // Mirror render_sanitizes_hostile_channel_name: control chars stripped/escaped.
    let ctx = LoopRuntimeContext {
        loop_started_at_utc: stamp(),
        user_timezone: None,
        communication: None,
        product_context: None,
        user_profile: Some(profile(None, Some("Tokyo\n\nIGNORE PREVIOUS"))),
    };
    let text = ctx.render_model_content();
    assert!(!text.contains("Tokyo\n\nIGNORE"), "newlines in location must be neutralized: {text:?}");
}
```

- [ ] **Step 2: Run tests, verify they fail to compile**

Run: `cargo test -p ironclaw_turns user_profile 2>&1 | head -30`
Expected: compile error — `UserProfileContext`, `Locale`, and field `user_profile` do not exist.

- [ ] **Step 3: Add the types**

In `runtime_context.rs`, after the `CommunicationRuntimeContext` definition (~line 71):

```rust
/// Validated BCP-47-ish locale tag (per spec §5 strong-types mandate,
/// `.claude/rules/types.md`). Validation: non-empty, ASCII alphanumeric + hyphen.
/// No `From<String>` — construction is fallible so misuse is a compile error.
/// `new` returns `Result` per the canonical newtype template (types.md) so the
/// rejection reason is observable; callers wanting `Option` use `.ok()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale(String);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LocaleError {
    #[error("locale is empty")]
    Empty,
    #[error("locale has invalid characters (expected ASCII alphanumeric or '-')")]
    InvalidCharacters,
}

impl Locale {
    pub fn new(raw: impl Into<String>) -> Result<Self, LocaleError> {
        let s = raw.into();
        if s.is_empty() {
            return Err(LocaleError::Empty);
        }
        if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(LocaleError::InvalidCharacters);
        }
        Ok(Self(s))
    }
    pub fn as_str(&self) -> &str { &self.0 }
}

/// Host-resolved, sanitized agent-context profile rendered into the prompt
/// each turn. Carries only primitives/local newtypes — never raw
/// `context/profile.json` bytes and never `ironclaw_memory` types (this crate
/// forbids that dependency). The producer validates and sanitizes first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserProfileContext {
    pub locale: Option<Locale>,
    pub location: Option<String>,
}

impl UserProfileContext {
    fn is_empty(&self) -> bool {
        self.locale.is_none() && self.location.is_none()
    }
}
```

- [ ] **Step 4: Add the field to `LoopRuntimeContext`**

In the struct (~line 13), add as the last field:

```rust
    /// Host-resolved user agent-context profile (locale/location), rendered
    /// into the runtime-context prompt section. `None` when no profile is set.
    pub user_profile: Option<UserProfileContext>,
```

Update the in-crate test helper `time_only_ctx` (~line 445) and every existing test constructor in this file to add `user_profile: None,` (the compiler lists them).

- [ ] **Step 5: Render the profile line + elicitation hint**

In `render_model_content`, change the `None` arm of `time_line` (~line 84) to mention `profile_set`:

```rust
            None => format!(
                "Current date/time at loop start: {utc}. The user's timezone is \
                 unknown - if local time matters, ask the user and offer to save \
                 it with the profile_set capability, or use the time capability if \
                 it is visible."
            ),
```

After `let mut parts = vec![time_line];` (~line 93), insert:

```rust
        if let Some(profile) = &self.user_profile {
            if !profile.is_empty() {
                let mut fields = Vec::new();
                if let Some(locale) = &profile.locale {
                    // Locale is already validated (ascii-alnum/hyphen) — no sanitize needed.
                    fields.push(format!("locale={}", locale.as_str()));
                }
                if let Some(location) = &profile.location {
                    // location is free text — sanitize via the existing helper.
                    fields.push(format!("location={}", sanitize_prompt_string(location)));
                }
                parts.push(format!("User profile: {}.", fields.join(", ")));
            }
        }
```

Reuse the **existing** helper `sanitize_prompt_string` (`runtime_context.rs:245`, used by the channel-name/delivery-target render path) — do not invent a new `sanitize_inline`. This keeps behavior identical to `render_sanitizes_hostile_channel_name`.

- [ ] **Step 6: Run tests, verify pass**

Run: `cargo test -p ironclaw_turns 2>&1 | tail -20`
Expected: PASS (new tests + existing `renders_*` tests).

- [ ] **Step 7: Update the contract test**

`crates/ironclaw_turns/tests/agent_loop_host_contract.rs:501` (`instruction_bundle_renders_runtime_context_section`) constructs a `LoopRuntimeContext` — add `user_profile: None,` and, if useful, assert the profile line is absent when `None`.

Run: `cargo test -p ironclaw_turns --test agent_loop_host_contract 2>&1 | tail -15`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/ironclaw_turns/src/run_profile/runtime_context.rs crates/ironclaw_turns/tests/agent_loop_host_contract.rs
git commit -m "feat(turns): add UserProfileContext to LoopRuntimeContext render"
```

---

## Task 2: `HostUserProfileSource` trait + resolved DTO

**Files:**
- Create: `crates/ironclaw_loop_support/src/user_profile_context.rs` (trait **and** `ResolvedUserProfile`)
- Modify: `crates/ironclaw_loop_support/src/lib.rs`

> **Design update (timezone folds into the profile):** `user_timezone` is no longer a standalone `LoopRuntimeContext` field — it lives in `UserProfileContext.timezone` (done in Wave 1). So there is **no `ResolvedUserProfile` DTO** — the trait returns `Option<UserProfileContext>` directly (timezone + locale + location all in one). Simpler: one type, no split.
>
> **Review fix (approach + local-patterns):** the trait takes `&LoopRunContext` — exactly mirroring the sibling `HostIdentityContextSource::load_identity_candidates` (`identity_context.rs:17-20`) — not decomposed `(TurnScope, TurnActor)`. This keeps a uniform call shape at `loop_driver_host.rs` and preserves access to `resolved_run_profile` for a future privacy gate.

- [ ] **Step 1: Write the trait + failing default test**

Create `crates/ironclaw_loop_support/src/user_profile_context.rs`:

```rust
//! Host source for the per-user agent-context profile, read at loop start.
//!
//! Mirrors `HostIdentityContextSource`: the trait lives here so the loop driver
//! depends only on a neutral port, while the concrete implementation (which
//! reads `context/profile.json` from the memory backend) lives in
//! `ironclaw_host_runtime`, keeping `ironclaw_memory` out of `ironclaw_reborn`.

use async_trait::async_trait;
use ironclaw_turns::run_profile::host::LoopRunContext;     // match identity_context.rs import
use ironclaw_turns::run_profile::runtime_context::UserProfileContext;

/// Resolves the per-user agent-context profile for a run. Returns the validated
/// `UserProfileContext` (timezone/locale/location), or `None` when no profile is
/// set or it cannot be resolved. Implementations must never fabricate values
/// (e.g. a guessed timezone) — fail to `None` instead.
#[async_trait]
pub trait HostUserProfileSource: Send + Sync {
    async fn resolve_user_profile(
        &self,
        run_context: &LoopRunContext,
    ) -> Option<UserProfileContext>;
}

/// Default no-op source: always `None`. Used when no profile source is wired.
#[derive(Debug, Default, Clone)]
pub struct EmptyUserProfileSource;

#[async_trait]
impl HostUserProfileSource for EmptyUserProfileSource {
    async fn resolve_user_profile(
        &self,
        _run_context: &LoopRunContext,
    ) -> Option<UserProfileContext> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Build a sample LoopRunContext the same way the identity_context tests do —
    // grep `LoopRunContext` in this crate's tests for the existing constructor.
    use crate::tests_support::sample_loop_run_context;

    #[tokio::test]
    async fn empty_source_returns_none() {
        let run_context = sample_loop_run_context();
        assert!(EmptyUserProfileSource.resolve_user_profile(&run_context).await.is_none());
    }
}
```

> Note: confirm the exact import path for `LoopRunContext` (match the `use` line in `identity_context.rs`) and the existing test constructor for it. Match how `UserProfileContext` is re-exported.

- [ ] **Step 2: Declare the module (`pub mod`)**

In `crates/ironclaw_loop_support/src/lib.rs`, mirror the `pub mod identity_context;` declaration (`lib.rs:28`). The crate sets `#![warn(unreachable_pub)]`, so a private `mod` + `pub use` would warn — use `pub mod`:

```rust
pub mod user_profile_context;
pub use user_profile_context::{EmptyUserProfileSource, HostUserProfileSource};
```

Confirm `async-trait` is a dependency of `ironclaw_loop_support` (used by sibling host-source traits). No `chrono-tz` needed here now — the trait returns `UserProfileContext` (which owns the `Tz`) from `ironclaw_turns`, an existing dependency.

- [ ] **Step 3: Run the test**

Run: `cargo test -p ironclaw_loop_support user_profile 2>&1 | tail -20`
Expected: PASS (after fixing import paths per the Step 1 note).

- [ ] **Step 4: Architecture + commit**

```bash
cargo test -p ironclaw_architecture 2>&1 | tail -15   # expect PASS — no new forbidden edges
git add crates/ironclaw_loop_support/src/user_profile_context.rs crates/ironclaw_loop_support/src/lib.rs
git commit -m "feat(loop-support): add HostUserProfileSource port + ResolvedUserProfile DTO"
```

---

## Task 3: Production `HostUserProfileSource` impl (reads `context/profile.json`)

**Files:**
- Create: `crates/ironclaw_host_runtime/src/user_profile_source.rs`
- Modify: `crates/ironclaw_host_runtime/src/lib.rs`

This impl owns the `ironclaw_memory` read and all parsing/validation, then returns a fully-resolved `ResolvedUserProfile`.

- [ ] **Step 1: Write the failing test (in-memory filesystem)**

Create `crates/ironclaw_host_runtime/src/user_profile_source.rs` with a test that writes a `context/profile.json` via the memory backend and asserts the source parses it. Mirror how `first_party_tools/memory.rs` tests build a `RootFilesystem` (grep for `InMemoryBackend`/`InMemory` in `ironclaw_memory` tests and reuse that constructor).

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resolves_timezone_locale_location_from_profile_doc() {
        let fs = in_memory_root_filesystem();           // helper per memory tests
        let scope = sample_turn_scope_with_user("tenant-a", "user-1");
        write_profile_json(&fs, &scope, r#"{"timezone":"Asia/Tokyo","locale":"ja-JP","location":"Tokyo, Japan"}"#).await;

        let source = MemoryBackedUserProfileSource::new(fs);
        let resolved = source.resolve_user_profile(&run_context_for(&scope, "user-1")).await.unwrap();

        assert_eq!(resolved.timezone.map(|tz| tz.name()), Some("Asia/Tokyo"));
        assert_eq!(resolved.locale.map(|l| l.as_str().to_string()).as_deref(), Some("ja-JP"));
        assert_eq!(resolved.location.as_deref(), Some("Tokyo, Japan"));
    }

    #[tokio::test]
    async fn invalid_timezone_resolves_to_none_not_guess() {
        let fs = in_memory_root_filesystem();
        let scope = sample_turn_scope_with_user("tenant-a", "user-1");
        write_profile_json(&fs, &scope, r#"{"timezone":"Pacific Time","locale":"en-US"}"#).await;

        let source = MemoryBackedUserProfileSource::new(fs);
        let resolved = source.resolve_user_profile(&run_context_for(&scope, "user-1")).await.unwrap();

        assert!(resolved.timezone.is_none(), "invalid IANA name must not be guessed");
        assert_eq!(resolved.locale.map(|l| l.as_str().to_string()).as_deref(), Some("en-US"));
    }

    #[tokio::test]
    async fn missing_doc_resolves_to_none() {
        let fs = in_memory_root_filesystem();
        let scope = sample_turn_scope_with_user("tenant-a", "user-1");
        let source = MemoryBackedUserProfileSource::new(fs);
        assert!(source.resolve_user_profile(&run_context_for(&scope, "user-1")).await.is_none());
    }
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cargo test -p ironclaw_host_runtime user_profile_source 2>&1 | head -30`
Expected: compile error — `MemoryBackedUserProfileSource` undefined.

- [ ] **Step 3: Implement the source**

```rust
use std::sync::Arc;

use async_trait::async_trait;
use chrono_tz::Tz;
use ironclaw_loop_support::HostUserProfileSource;
use ironclaw_memory::{
    FilesystemMemoryDocumentRepository, MemoryContext, MemoryDocumentPath, MemoryDocumentScope,
    RepositoryMemoryBackend, MemoryBackend,
};
use ironclaw_filesystem::RootFilesystem;
use ironclaw_turns::run_profile::host::LoopRunContext;
use ironclaw_turns::run_profile::runtime_context::{Locale, UserProfileContext};
use serde::Deserialize;

// This module is a loop-start PRODUCER (host service boundary), not a capability
// handler — that is why it lives at top-level `src/` rather than under
// `first_party_tools/` (per crate CLAUDE.md, runtime services get their own module).

/// Relative path of the per-user agent-context profile document.
pub const PROFILE_DOCUMENT_PATH: &str = "context/profile.json";

/// Single home for the profile scope decision: keyed to the human user at
/// `agent=None, project=None` (spec §10) regardless of run scope. BOTH the
/// producer (read) and `profile_merge_write` (write) call this so the scope
/// narrowing — and any future project-override — lives in exactly one place.
pub(crate) fn profile_scope_and_path(
    tenant_id: &str,
    user_id: &str,
) -> Result<(MemoryDocumentScope, MemoryDocumentPath), ()> {
    let scope = MemoryDocumentScope::new_with_agent(tenant_id, user_id, None, None).map_err(|_| ())?;
    let path = MemoryDocumentPath::new_with_agent(
        tenant_id, user_id, None, None, PROFILE_DOCUMENT_PATH,
    ).map_err(|_| ())?;
    Ok((scope, path))
}

/// Reads `context/profile.json` for the run owner and resolves it into a
/// validated `ResolvedUserProfile`. Owns the `ironclaw_memory` dependency so the
/// loop driver and `ironclaw_reborn` never import it.
pub struct MemoryBackedUserProfileSource {
    filesystem: Arc<dyn RootFilesystem>,
}

impl MemoryBackedUserProfileSource {
    pub fn new(filesystem: Arc<dyn RootFilesystem>) -> Self {
        Self { filesystem }
    }
}

#[derive(Debug, Deserialize, Default)]
struct ProfileJson {
    #[serde(default)]
    timezone: Option<String>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    location: Option<String>,
}

#[async_trait]
impl HostUserProfileSource for MemoryBackedUserProfileSource {
    async fn resolve_user_profile(
        &self,
        run_context: &LoopRunContext,
    ) -> Option<UserProfileContext> {
        // Profile is keyed to the human user at agent=None, project=None
        // (spec §10) regardless of the run's agent/project scope.
        let scope = &run_context.scope;
        let user_id = run_context.actor.as_ref().map(|a| a.user_id.as_str())?;
        // Shared scope helper — same keying as the writer (no duplicated decision).
        let (doc_scope, path) = profile_scope_and_path(scope.tenant_id.as_str(), user_id).ok()?;
        let context = MemoryContext::new(doc_scope);

        let repository = Arc::new(FilesystemMemoryDocumentRepository::new(Arc::clone(&self.filesystem)));
        let backend = RepositoryMemoryBackend::new(repository);

        let bytes = match backend.read_document(&context, &path).await {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return None,
            Err(error) => {
                tracing::debug!(error = %error, "user profile read failed; continuing without profile");
                return None;
            }
        };

        let parsed: ProfileJson = match serde_json::from_slice(&bytes) {
            Ok(parsed) => parsed,
            Err(error) => {
                tracing::debug!(error = %error, "user profile JSON parse failed; continuing without profile");
                return None;
            }
        };

        // Never guess: invalid IANA name → None. Timezone lives in the profile.
        let timezone = parsed
            .timezone
            .as_deref()
            .and_then(|name| name.trim().parse::<Tz>().ok());
        let profile = UserProfileContext {
            timezone,
            // validated newtype; invalid → None, with a debug trail per types.md
            locale: parsed.locale.and_then(|s| match Locale::new(s) {
                Ok(l) => Some(l),
                Err(error) => {
                    tracing::debug!(%error, "locale in profile rejected; dropping field");
                    None
                }
            }),
            location: parsed.location.filter(|s| !s.trim().is_empty()),
        };

        if profile == UserProfileContext::default() {
            return None;
        }
        Some(profile)
    }
}
```

> Confirm exact re-export names (`FilesystemMemoryDocumentRepository`, `RepositoryMemoryBackend`, `MemoryDocumentPath::new_with_agent`, `MemoryContext::new`) against `ironclaw_memory`'s public surface; the explorer cited all of these. If `new_with_agent` rejects `agent=None` here, that's caught by Task 6's integration test.

- [ ] **Step 4: Declare the module + export**

In `crates/ironclaw_host_runtime/src/lib.rs`:

```rust
mod user_profile_source;
pub use user_profile_source::{MemoryBackedUserProfileSource, PROFILE_DOCUMENT_PATH};
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p ironclaw_host_runtime user_profile_source 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/ironclaw_host_runtime/src/user_profile_source.rs crates/ironclaw_host_runtime/src/lib.rs
git commit -m "feat(host-runtime): MemoryBackedUserProfileSource reads context/profile.json"
```

---

## Task 3B: Stop prose-injecting `context/profile.json`

**Review fix (approach, intent-mismatch 80):** spec §4a says the structured profile must **not** also be dumped as a raw-JSON identity message once the typed render exists — otherwise every turn injects the blob twice (raw JSON via the identity path + the `User profile:` line), giving the model two sources that can disagree. `context/profile.json` is currently in `DEFAULT_PROMPT_PROTECTED_PATHS` (`crates/ironclaw_memory/src/safety.rs:164`) and surfaces through the identity allow-list (`src/workspace/reborn_identity_context.rs` `stable_identity_paths()` / `STABLE_IDENTITY_PATHS`).

**Files:**
- Modify: `src/workspace/reborn_identity_context.rs` (the identity-path allow-list)

> **Important:** do NOT remove `context/profile.json` from `DEFAULT_PROMPT_PROTECTED_PATHS` in `ironclaw_memory` — that list also governs prompt-**write** safety (injection scanning on write), which we still want. Only drop it from the **identity injection** allow-list so it stops being rendered as prose; keep it protected on write.

- [ ] **Step 1: Write the failing test**

In the test module for `reborn_identity_context.rs`, assert `context/profile.json` is NOT among the stable identity paths:

```rust
#[test]
fn profile_json_is_not_prose_injected() {
    let paths = stable_identity_paths();   // or the equivalent accessor
    assert!(!paths.iter().any(|p| p == "context/profile.json"),
        "context/profile.json must be consumed by the typed producer, not prose-injected");
}
```

- [ ] **Step 2: Run, verify fail**

Run: `cargo test -p <crate-of-reborn_identity_context> profile_json_is_not_prose 2>&1 | head -20`
Expected: FAIL — the path is currently included.

- [ ] **Step 3: Remove it from the identity allow-list**

In `src/workspace/reborn_identity_context.rs`, omit `"context/profile.json"` from `STABLE_IDENTITY_PATHS` (the allow-list filtered against `DEFAULT_PROMPT_PROTECTED_PATHS`). Add a comment: `// context/profile.json is rendered via LoopRuntimeContext (typed), not prose-injected — see profile design §4a`.

- [ ] **Step 4: Run, verify pass + commit**

```bash
cargo test -p <crate> profile_json 2>&1 | tail -15
git add src/workspace/reborn_identity_context.rs
git commit -m "fix(workspace): stop prose-injecting context/profile.json (typed render owns it)"
```

---

## Task 4: `builtin.profile_set` capability

**Files:**
- Create: `crates/ironclaw_host_runtime/src/first_party_tools/profile_set.rs`
- Modify: `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs`
- Modify: `crates/ironclaw_host_runtime/src/first_party_tools/schemas.rs`

The handler does typed validation of each field (authoritative), then read-modify-write field-merge of `context/profile.json` using the shared profile-path helper.

- [ ] **Step 1: Write the failing handler test**

In `profile_set.rs`, mirror the memory dispatch tests. Build a `FirstPartyCapabilityRequest` with `input = {"timezone":"Asia/Tokyo"}`, dispatch, then read the doc back and assert it contains the field; then a second call with `{"locale":"ja-JP"}` and assert **both** fields persist (merge, no clobber); then an invalid `{"timezone":"Pacific Time"}` returns an error.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sets_then_merges_fields_without_clobber() {
        let state = MemoryCapabilityState::default();
        let req1 = profile_set_request(json!({"timezone": "Asia/Tokyo"}));
        dispatch(&state, &req1).await.unwrap();
        let req2 = profile_set_request(json!({"locale": "ja-JP"}));
        dispatch(&state, &req2).await.unwrap();

        let doc = read_profile_doc(&req2).await;     // helper reads context/profile.json
        assert_eq!(doc["timezone"], "Asia/Tokyo");   // preserved across the second write
        assert_eq!(doc["locale"], "ja-JP");
    }

    #[tokio::test]
    async fn rejects_invalid_timezone() {
        let state = MemoryCapabilityState::default();
        let req = profile_set_request(json!({"timezone": "Pacific Time"}));
        let err = dispatch(&state, &req).await.unwrap_err();
        assert!(format!("{err:?}").to_lowercase().contains("timezone"));
    }

    #[tokio::test]
    async fn rejects_unknown_field() {
        let state = MemoryCapabilityState::default();
        // System-config-style field must be refused by the closed surface.
        let req = profile_set_request(json!({"always_approve": true}));
        assert!(dispatch(&state, &req).await.is_err());
    }
}
```

> The trait takes one arg (`&LoopRunContext`). Add a test helper `run_context_for(scope: &..., user_id: &str) -> LoopRunContext` that builds a `LoopRunContext` from the scope + a `TurnActor` for `user_id`, mirroring how `identity_context.rs` tests construct `LoopRunContext` (grep `LoopRunContext::new` in that crate's tests). Reuse the request/filesystem builders the existing `memory.rs` tests use (grep `FirstPartyCapabilityRequest` test constructors in `first_party_tools/`).

- [ ] **Step 2: Run, verify fail**

Run: `cargo test -p ironclaw_host_runtime profile_set 2>&1 | head -30`
Expected: compile error — module/functions undefined.

- [ ] **Step 3: Implement the capability**

```rust
use chrono_tz::Tz;
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode};
use serde_json::{json, Map, Value};

use crate::first_party::{FirstPartyCapabilityRequest, FirstPartyCapabilityResult};
use crate::FirstPartyCapabilityError;
use super::memory::{self, MemoryCapabilityState};
use super::{first_party_capability_manifest, input_error, resource_profile};

pub const PROFILE_SET_CAPABILITY_ID: &str = "builtin.profile_set";

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        PROFILE_SET_CAPABILITY_ID,
        "Record a known structured fact about the user: timezone (IANA name), \
         locale (BCP-47), or location (free label). Use \
         whenever the user states one of these so future answers stay correct.",
        vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
        PermissionMode::Allow,
        resource_profile(),
    )
}

/// Validate the closed field set into a JSON object to merge. Unknown fields and
/// invalid values are rejected here — this typed boundary is the authoritative
/// enforcement (the doc JSON-schema is defense-in-depth).
fn validated_fields(input: &Value) -> Result<Map<String, Value>, FirstPartyCapabilityError> {
    let obj = input.as_object().ok_or_else(input_error)?;
    let mut out = Map::new();
    for (key, value) in obj {
        match key.as_str() {
            "timezone" => {
                let s = value.as_str().ok_or_else(input_error)?;
                s.trim().parse::<Tz>().map_err(|_| input_error())?; // reject non-IANA
                out.insert("timezone".into(), json!(s.trim()));
            }
            "locale" => {
                let s = value.as_str().ok_or_else(input_error)?;
                // light BCP-47 shape check: non-empty, ascii-alnum/hyphen
                if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                    return Err(input_error());
                }
                out.insert("locale".into(), json!(s));
            }
            "location" => {
                let s = value.as_str().ok_or_else(input_error)?;
                if s.is_empty() || s.chars().count() > 200 {
                    return Err(input_error());
                }
                out.insert("location".into(), json!(s));
            }
            _ => return Err(input_error()), // closed surface: refuse unknown/system-config fields
        }
    }
    if out.is_empty() {
        return Err(input_error());
    }
    Ok(out)
}

pub(super) async fn dispatch(
    state: &MemoryCapabilityState,
    request: &FirstPartyCapabilityRequest,
) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
    let fields = validated_fields(&request.input)?;
    // Reuse the memory services/backend resolution; profile_merge_write keys the
    // doc via the shared profile_scope_and_path helper (agent=None, project=None).
    memory::profile_merge_write(state, request, fields).await
}
```

- [ ] **Step 4: Add `profile_merge_write` to `memory.rs`**

This read-modify-writes the profile doc at the user-only scope. Add to `crates/ironclaw_host_runtime/src/first_party_tools/memory.rs`:

```rust
/// Compare-and-write field-merge for the structured user profile doc. Keys the
/// doc via the shared `profile_scope_and_path` helper (single scope-decision
/// home). Validated field map is merged over existing JSON under a CAS retry
/// loop so concurrent `profile_set` calls cannot lose a field.
pub(super) async fn profile_merge_write(
    state: &MemoryCapabilityState,
    request: &FirstPartyCapabilityRequest,
    fields: serde_json::Map<String, serde_json::Value>,
) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError> {
    use crate::user_profile_source::profile_scope_and_path;

    ensure_memory_mount(request, /* write */ true)?;
    let (scope, path) = profile_scope_and_path(
        request.scope.tenant_id.as_str(),
        request.scope.user_id.as_str(),
    ).map_err(|_| input_error())?;
    let context = MemoryContext::new(scope)
        .with_audit_context(request.scope.clone(), ironclaw_host_api::CorrelationId::new());
    let backend = state.backend_for(request)?;
    let options = MemoryBackendWriteOptions::default();

    // CAS retry loop — mirrors `patch_document`'s MAX_MEMORY_PATCH_RETRIES pattern.
    // Read current bytes + hash, merge fields, compare-and-write; retry on hash
    // mismatch (a concurrent writer raced in). `validated_fields()` is the sole
    // authoritative validator (no schema overlay — see review-fix note).
    for _ in 0..MAX_MEMORY_PATCH_RETRIES {
        let current = backend.read_document(&context, &path).await.map_err(|_| operation_error())?;
        let expected_hash = current.as_deref().map(content_bytes_sha256);   // existing helper in memory.rs
        let mut doc: serde_json::Map<String, serde_json::Value> = match &current {
            Some(bytes) => serde_json::from_slice(bytes).unwrap_or_default(),
            None => serde_json::Map::new(),
        };
        for (k, v) in &fields { doc.insert(k.clone(), v.clone()); }
        let bytes = serde_json::to_vec(&serde_json::Value::Object(doc)).map_err(|_| operation_error())?;

        let outcome = backend.compare_and_write_document_with_backend_options(
            &context, &path, expected_hash.as_deref(), &bytes, &options,
        ).await.map_err(|_| operation_error())?;
        if outcome.committed() {                  // match the real MemoryWriteOutcome API
            return Ok(FirstPartyCapabilityResult::new(json!({ "status": "ok" }), ResourceUsage::default()));
        }
        // else: hash moved under us — loop and re-merge onto the newer doc.
    }
    Err(operation_error())
}
```

> `operation_error()`/`input_error()`/`ensure_memory_mount`/`backend_for` already exist in `memory.rs` (explorer-confirmed). Reuse them. Confirm the `ResourceUsage`/`FirstPartyCapabilityResult::new` shape against the other dispatch fns in this file.

- [ ] **Step 5: (removed by review — duplicate truth)**

**Review fix (maintainability, duplicate-truth 85):** the original plan added a `profile_write_options()` metadata schema overlay duplicating `validated_fields()` and the `schemas.rs` arm — a third hand-edited copy of the field set with no sync mechanism. **Dropped.** `validated_fields()` (Step 3) is the single authoritative enforcement: any write path other than `profile_set` is already blocked because the capability is the only writer of `context/profile.json`. The `schemas.rs` arm (Step 7) remains solely as the **LLM-visible tool description** (a different concern — what the model sees), not a second validator. Two artifacts, two distinct purposes; no duplicated truth.

- [ ] **Step 6: Register the capability in `mod.rs`**

1. Add `PROFILE_SET_CAPABILITY_ID` to the manifest vec in `builtin_first_party_package()` (~`mod.rs:162`): `profile_set::manifest()?,`.
2. Add the handler in `builtin_first_party_base_registry()` (~`mod.rs:247`): `.with_handler(CapabilityId::new(profile_set::PROFILE_SET_CAPABILITY_ID)?, handler.clone())`.
3. Add a dispatch arm in `BuiltinFirstPartyTools::dispatch` (~`mod.rs:347`), early-return like memory (it manages its own `ResourceUsage`):

```rust
        profile_set::PROFILE_SET_CAPABILITY_ID => {
            let mut result = profile_set::dispatch(&self.memory_state, request).await?;
            result.usage.output_bytes = bounded_output_bytes(&result.output, FIRST_PARTY_MAX_OUTPUT_BYTES)?;
            return Ok(result);
        }
```

4. Add `mod profile_set;` to the module list at the top of `mod.rs`.

- [ ] **Step 7: Add the input schema arm in `schemas.rs`**

In the `resolve_builtin_input_schema_ref` match (~`schemas.rs:3`):

```rust
        "schemas/builtin/profile_set.input.v1.json" => json!({
            "type": "object",
            "properties": {
                "timezone": { "type": "string", "description": "IANA timezone name, e.g. America/Los_Angeles" },
                "locale":   { "type": "string", "description": "BCP-47 tag, e.g. en-US" },
                "location": { "type": "string", "description": "Free-text location label" }
            },
            "additionalProperties": false
        }),
```

- [ ] **Step 8: Run tests**

Run: `cargo test -p ironclaw_host_runtime profile_set 2>&1 | tail -25`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/ironclaw_host_runtime/src/first_party_tools/
git commit -m "feat(host-runtime): add builtin.profile_set capability with field-merge write"
```

---

## Task 5: Wire the producer into the loop-driver host factory

**Files:**
- Modify: `crates/ironclaw_reborn/src/loop_driver_host.rs`
- Modify: `crates/ironclaw_reborn_composition/src/...` (the factory construction site that builds `RebornLoopDriverHostFactory`)

- [ ] **Step 1: Add the optional source field + builder to the factory**

In `loop_driver_host.rs`, on `RebornLoopDriverHostFactory` (struct ~line 885), mirror `identity_context_source`:

```rust
    user_profile_source: Option<Arc<dyn HostUserProfileSource>>,
```

Add the builder (mirror `with_identity_context_source`, ~line 1273):

```rust
    pub fn with_user_profile_source(mut self, source: Arc<dyn HostUserProfileSource>) -> Self {
        self.user_profile_source = Some(source);
        self
    }
```

Initialize the field to `None` in the constructor/`Default` path. Add `use ironclaw_loop_support::HostUserProfileSource;`.

> Per `.claude/rules/architecture.md` smell #2 (`Option<Arc<…>>` + `with_*`): production wires this every time, so guard against the "optional but always-set" lint by either (a) wiring it unconditionally in composition (Step 4) and keeping `Option` only for the test/no-profile path, or (b) defaulting to `Arc::new(EmptyUserProfileSource)`. Prefer (b): make the field non-optional `Arc<dyn HostUserProfileSource>` defaulting to `EmptyUserProfileSource`, and rename the builder to set it. This avoids the optional-Arc smell entirely.

Revised field:

```rust
    user_profile_source: Arc<dyn HostUserProfileSource>,   // defaults to EmptyUserProfileSource
```

- [ ] **Step 2: Fill the runtime context at the construction site**

At the `LoopRuntimeContext` build (~line 1563), before constructing it, resolve the profile:

```rust
        // Timezone lives inside the profile — no split. `None` when unset.
        let user_profile = self
            .user_profile_source
            .resolve_user_profile(&run_context)
            .await;
```

Then change the struct literal (note: `user_timezone` field no longer exists on `LoopRuntimeContext` — Wave 1 removed it):

```rust
        .with_runtime_context(LoopRuntimeContext {
            loop_started_at_utc: chrono::Utc::now(),
            communication,
            product_context: run_context.product_context.clone(),
            user_profile,
        });
```

> Performance: this awaits a memory read on the loop-start path. v1 accepts a single bounded read here (the existing `communication` slice already does backend fetches at loop start). If profiling shows it matters, a follow-up can move it onto the same concurrent fetch budget as `CommunicationContextProvider`. Note this tradeoff inline; do not pre-optimize.

- [ ] **Step 3: Add a host-level test with a fake source**

In `loop_driver_host.rs` tests (or the crate's test module), add a fake `HostUserProfileSource` returning a known `ResolvedUserProfile`, build the host, and assert the rendered runtime context contains the local-time line + profile line. If the host-build path is hard to exercise in a unit test, defer this assertion to Task 6's integration test and keep a smaller unit test that the factory stores and calls the source.

- [ ] **Step 4: Wire the production source in composition**

In `ironclaw_reborn_composition` where `RebornLoopDriverHostFactory` is built, construct `MemoryBackedUserProfileSource::new(root_filesystem)` from the same `RootFilesystem` the host runtime already uses, and pass `.with_user_profile_source(Arc::new(source))`. Match how `with_identity_context_source` is wired in the same composition file.

- [ ] **Step 5: Run + architecture check**

```bash
cargo test -p ironclaw_reborn 2>&1 | tail -20
cargo test -p ironclaw_architecture 2>&1 | tail -15   # confirm ironclaw_reborn did NOT gain an ironclaw_memory edge
```
Expected: PASS, and `ironclaw_reborn/Cargo.toml` has **no** `ironclaw_memory` dependency (the read lives behind the trait in `ironclaw_host_runtime`).

- [ ] **Step 6: Commit**

```bash
git add crates/ironclaw_reborn/src/loop_driver_host.rs crates/ironclaw_reborn_composition/
git commit -m "feat(reborn): fill LoopRuntimeContext user profile via HostUserProfileSource"
```

---

## Task 6: Caller-level integration test (both DB backends)

**Files:**
- Create: `crates/ironclaw_reborn/tests/user_profile_roundtrip.rs` (or extend an existing composition integration test)

Per `.claude/rules/testing.md` — drive the real callers end-to-end, across PostgreSQL + libSQL.

- [ ] **Step 1: Write the round-trip test**

Drive: (1) dispatch `builtin.profile_set` with `{"timezone":"Asia/Tokyo","locale":"ja-JP","location":"Tokyo, Japan"}` through the real capability dispatch path for a run scoped with a non-None agent/project; (2) build the loop-driver host for the same user; (3) assert the rendered runtime context contains `Asia/Tokyo` local time and `User profile: locale=ja-JP, location=Tokyo, Japan`. This proves the **scope-narrowing** (write at agent/project-scoped run, read back at user-only `(tenant,user,None,None)`) actually works through the backend's scope isolation.

```rust
#[tokio::test]
async fn profile_set_then_runtime_context_renders_local_time() {
    // ... build harness with InMemory or test DB backend ...
    // 1. profile_set
    // 2. start a run, capture rendered runtime context
    // 3. assert contains "Asia/Tokyo" and "User profile:"
}
```

- [ ] **Step 2: Run on both backends**

```bash
cargo test -p ironclaw_reborn --test user_profile_roundtrip 2>&1 | tail -20
cargo test --features integration user_profile_roundtrip 2>&1 | tail -20   # PostgreSQL
```
Expected: PASS on both. **If the user-only-scope read fails under an agent-scoped run**, this is the scope-narrowing risk surfacing — fix in the single `profile_merge_write` / source path (the only two places that build the profile path), e.g. by confirming the memory backend authorizes a same-user narrower context, or adjust the documented scope.

- [ ] **Step 3: Commit**

```bash
git add crates/ironclaw_reborn/tests/user_profile_roundtrip.rs
git commit -m "test(reborn): integration round-trip profile_set -> runtime context render"
```

---

## Task 7: Full quality gate + docs

- [ ] **Step 1: Format, lint, test**

```bash
cargo fmt
cargo clippy --all --benches --tests --examples --all-features 2>&1 | tail -20   # zero warnings
cargo test 2>&1 | tail -20
```

- [ ] **Step 2: Update the explainer doc**

Add a short note to `docs/reborn/2026-06-16-memory-how-it-works.md` §7 table: `builtin.profile_set` now exists; `user_timezone` now has a producer. Keep it factual.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "docs: record profile_set capability and user_timezone producer"
```

---

## Review revisions (round 1 applied)

Three plan-mode reviewers (approach / local-patterns / maintainability) produced 9 findings; resolutions:

| # | Finding (reviewer, conf) | Resolution |
|---|---|---|
| 1 | Trait takes decomposed `(TurnScope, TurnActor)` not `&LoopRunContext` like sibling (approach 75, local 75) | **Fixed** — trait + impl + call site now take `&LoopRunContext` |
| 2 | `ResolvedUserProfile` in wrong crate `ironclaw_turns` (maint 78) | **Fixed** — moved to `ironclaw_loop_support` next to the trait |
| 3 | Profile field schema in 3 unsynchronized places (maint 85) | **Fixed** — dropped the metadata schema overlay (Step 5); `validated_fields()` is sole enforcement |
| 4 | `Option<String>` locale violates spec §5 strong-types (approach 75) | **Fixed** — added `Locale` newtype; `UserProfileContext.locale: Option<Locale>` |
| 5 | `mod` vs `pub mod` under `#![warn(unreachable_pub)]` (local 90) | **Fixed** — `pub mod user_profile_context;` |
| 6 | Invented `sanitize_inline` vs existing `sanitize_prompt_string` (local 80) | **Fixed** — reuse `sanitize_prompt_string` (`runtime_context.rs:245`) |
| 7 | `%error` vs `error = %error` log form (local 75) | **Fixed** — `error = %error` |
| 8 | Test helper could shadow `dispatch` (local 85) | Note added — tests call `super::dispatch` via `use super::*`; rename local builders to `profile_set_request`/`call_profile_set` if any collision arises |
| 9 | `user_profile_source.rs` top-level placement (local 65) | **Fixed** — added a placement comment explaining it's a producer, not a capability |

Speculative-generality on the trait (single real impl) was **considered and not flagged** by the maintainability reviewer: `EmptyUserProfileSource` is a real test double and the trait is the established boundary pattern (`HostIdentityContextSource`), so it passes the GOOD-abstraction test.

## Review revisions (round 2 applied)

Second plan-mode pass produced 6 findings; resolutions:

| # | Finding (reviewer, conf) | Resolution |
|---|---|---|
| 1 | `context/profile.json` double-injected (prose + typed render) (approach 80) | **Fixed** — new **Task 3B** drops it from the identity allow-list (keeps it write-protected) |
| 2 | RMW merge ignores available CAS → lost writes (maint 75) | **Fixed** — `profile_merge_write` now uses `compare_and_write_*` in a `MAX_MEMORY_PATCH_RETRIES` loop |
| 3 | Scope key duplicated in reader + writer (maint 75) | **Fixed** — single `profile_scope_and_path` helper; both producer and writer call it |
| 4 | Task 3 test snippet used stale 2-arg signature (local 75) | **Fixed** — tests build `LoopRunContext` via `run_context_for` helper |
| 5 | `Locale::new` returns `Option`, template wants `Result` (local 65) | **Fixed** — returns `Result<Self, LocaleError>`; producer logs + `.ok()` |
| 6 | Carry `Option<String>` not `Tz` to avoid chrono-tz on loop_support (approach 55) | **Rejected** — keeping `Tz` avoids a stringly-typed re-parse (types.md favors it) and the reviewer's fix would instead push `chrono-tz` onto `ironclaw_reborn` (worse). `chrono-tz` on `loop_support` is a small, honest dep already transitive. |

## Self-review notes (addressed)

- **Spec coverage:** structured store (Task 4 `context/profile.json`), `profile_set` typed capability (Task 4), producer→runtime (Tasks 2/3/5), render + elicitation §6 (Task 1), field set v1 §5 (Task 4 `validated_fields`), scope §10 (single `profile_merge_write`/source helper, agent=None/project=None), security §9 (closed enum rejects unknown fields — Task 4 `rejects_unknown_field`), testing §11 (Tasks 1–6, both backends Task 6), error handling §7 (typed errors + `debug!` fail-soft Task 3).
- **Prose container (USER.md)** needs no code — already injected; documented in spec.
- **Out of scope** (geo→tz derivation, project override, system-config migration): not implemented; `location` is a label only (Task 4).
- **Optional-Arc smell:** resolved by defaulting `user_profile_source` to `EmptyUserProfileSource` (Task 5 Step 1) rather than `Option<Arc<…>>`.

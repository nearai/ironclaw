//! Trusted-internal-write scope marker.
//!
//! Some Store implementations gate `save_memory_doc` for security-sensitive
//! documents (orchestrator code, prompt overlays). The gate exempts
//! *system-internal* writes — e.g. seeding the compiled-in orchestrator v0
//! during project bootstrap — from the LLM-write rules.
//!
//! Distinguishing "system internal" from "LLM-authored" purely by document
//! contents is unsafe: an LLM tool call could craft a payload with whatever
//! marker the gate checks. Instead, the trusted-write callsite enters this
//! task-local scope before calling `save_memory_doc`, and the Store
//! implementation reads `is_trusted_internal_write_active()`. The flag is
//! scoped to the current async task and does **not** propagate across
//! `tokio::spawn`, so untrusted code cannot inherit it.
//!
//! ## Usage
//!
//! ```ignore
//! use ironclaw_engine::runtime::internal_write::with_trusted_internal_writes;
//!
//! with_trusted_internal_writes(async {
//!     store.save_memory_doc(&seed_doc).await?;
//!     Ok(())
//! }).await
//! ```
//!
//! Inside the closure, the store's gate sees `is_trusted_internal_write_active() == true`
//! and allows the write even when self-modification is otherwise disabled.

use std::future::Future;

tokio::task_local! {
    static TRUSTED_INTERNAL_WRITE: bool;
}

/// Run `fut` with the trusted-internal-write flag set.
pub async fn with_trusted_internal_writes<F: Future>(fut: F) -> F::Output {
    TRUSTED_INTERNAL_WRITE.scope(true, fut).await
}

/// True iff the current async task is inside a `with_trusted_internal_writes` scope.
pub fn is_trusted_internal_write_active() -> bool {
    TRUSTED_INTERNAL_WRITE.try_with(|v| *v).unwrap_or(false)
}

/// Snapshot of `ORCHESTRATOR_SELF_MODIFY` read once per process at first call.
///
/// Reading the env var on every gate check is fragile: env vars are global
/// mutable state, and a future sandbox escape (or a misbehaving in-process
/// caller) could flip a security gate mid-execution. This OnceLock captures
/// the value the first time it's queried and returns the same answer for
/// the rest of the process lifetime, so all callers — engine loop,
/// self-improvement mission, and the host store — see a consistent flag.
///
/// In dev/test builds (`debug_assertions` enabled), tests may override the
/// snapshot via `set_self_modify_for_test()` to exercise both code paths.
/// The override path is **compiled out of release builds**, so production
/// code cannot flip the gate at runtime under any circumstances.
pub fn self_modify_enabled() -> bool {
    #[cfg(debug_assertions)]
    if let Some(v) = test_override_value() {
        return v;
    }

    static SELF_MODIFY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SELF_MODIFY.get_or_init(|| {
        std::env::var("ORCHESTRATOR_SELF_MODIFY")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false)
    })
}

#[cfg(debug_assertions)]
static SELF_MODIFY_TEST_OVERRIDE: std::sync::RwLock<Option<bool>> = std::sync::RwLock::new(None);

#[cfg(debug_assertions)]
fn test_override_value() -> Option<bool> {
    SELF_MODIFY_TEST_OVERRIDE.read().ok().and_then(|g| *g)
}

/// Test-only override for `self_modify_enabled()`.
///
/// Production code reads the value from a process-wide OnceLock seeded from
/// `ORCHESTRATOR_SELF_MODIFY`. Tests would otherwise be unable to flip the
/// flag (the OnceLock locks the first reader's view forever), so this
/// helper provides a separate override layer that takes precedence. Pass
/// `None` to clear.
///
/// **Compiled out of release builds.** Calling this in a release build is
/// a no-op — `cfg(debug_assertions)` gates both the override storage and
/// the read path inside `self_modify_enabled()`.
#[cfg(debug_assertions)]
pub fn set_self_modify_for_test(value: Option<bool>) {
    if let Ok(mut guard) = SELF_MODIFY_TEST_OVERRIDE.write() {
        *guard = value;
    }
}

#[cfg(not(debug_assertions))]
pub fn set_self_modify_for_test(_value: Option<bool>) {
    // No-op in release builds — the override layer is compiled out so the
    // production OnceLock is the only path.
}

/// Scoped override for `self_modify_enabled()` that restores the previous
/// value on drop, even when the test panics. Strongly preferred over the
/// bare setter.
///
/// **Test serialization**: cargo runs unit tests in parallel by default,
/// and this guard mutates a process-wide `RwLock`. Without serialization,
/// two tests with conflicting overrides race and the loser sees the
/// other's value. The guard holds a `parking-lot`-style `Mutex` for its
/// lifetime so concurrent test threads queue up instead of racing. Tests
/// that don't touch self-modify pay no cost; tests that do touch it run
/// one at a time, which is acceptable because there are very few of them.
pub struct SelfModifyTestGuard {
    #[cfg(debug_assertions)]
    previous: Option<bool>,
    // Held for the guard's entire lifetime to serialize concurrent tests.
    // Stored as `Option` so `Drop` can release before clearing the override.
    #[cfg(debug_assertions)]
    _serializer: Option<std::sync::MutexGuard<'static, ()>>,
}

#[cfg(debug_assertions)]
static SELF_MODIFY_TEST_SERIALIZER: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl SelfModifyTestGuard {
    pub fn enable() -> Self {
        Self::new(true)
    }

    pub fn disable() -> Self {
        Self::new(false)
    }

    #[cfg(debug_assertions)]
    fn new(value: bool) -> Self {
        let serializer = SELF_MODIFY_TEST_SERIALIZER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = test_override_value();
        set_self_modify_for_test(Some(value));
        Self {
            previous,
            _serializer: Some(serializer),
        }
    }

    #[cfg(not(debug_assertions))]
    fn new(_value: bool) -> Self {
        Self {}
    }
}

impl Drop for SelfModifyTestGuard {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        set_self_modify_for_test(self.previous);
        // _serializer drops here, releasing the test mutex.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn defaults_to_false_outside_scope() {
        assert!(!is_trusted_internal_write_active());
    }

    #[tokio::test]
    async fn flag_is_true_inside_scope() {
        let inside =
            with_trusted_internal_writes(async { is_trusted_internal_write_active() }).await;
        assert!(inside);
        // And clears outside.
        assert!(!is_trusted_internal_write_active());
    }

    #[tokio::test]
    async fn flag_does_not_propagate_across_spawn() {
        let outer = with_trusted_internal_writes(async {
            // Spawning a new task starts a fresh task-local context.
            let handle = tokio::spawn(async { is_trusted_internal_write_active() });
            handle.await.unwrap()
        })
        .await;
        assert!(
            !outer,
            "trusted scope must NOT propagate across tokio::spawn"
        );
    }
}

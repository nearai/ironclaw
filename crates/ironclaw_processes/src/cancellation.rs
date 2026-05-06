//! Cooperative cancellation primitives for in-flight processes.
//!
//! Tokens are cheap-to-clone handles backed by an `AtomicBool` plus a `Notify`.
//! The registry keeps one token per running process so the host can cancel
//! across detached executor tasks without holding direct task handles.

use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
};

use ironclaw_host_api::{ProcessId, ResourceScope};
use tokio::sync::Notify;

use crate::types::ProcessKey;

#[derive(Clone)]
pub struct ProcessCancellationToken {
    inner: Arc<ProcessCancellationState>,
}

struct ProcessCancellationState {
    cancelled: AtomicBool,
    notify: Notify,
}

impl Default for ProcessCancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessCancellationToken {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ProcessCancellationState {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }

    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::SeqCst) {
            self.inner.notify.notify_waiters();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// Resolve when the token has been cancelled.
    ///
    /// The notify-then-check ordering matters: we *create* the `notified()`
    /// future before re-checking the atomic flag, so any `notify_waiters` that
    /// fires between the check and the `.await` is captured (rather than lost).
    /// Spurious wakeups loop back to the flag check; we only return once the
    /// cancelled flag is observably true.
    pub async fn cancelled(&self) {
        if self.is_cancelled() {
            return;
        }
        loop {
            let notified = self.inner.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
            if self.is_cancelled() {
                return;
            }
        }
    }
}

impl fmt::Debug for ProcessCancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProcessCancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl PartialEq for ProcessCancellationToken {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

#[derive(Debug, Default)]
pub struct ProcessCancellationRegistry {
    tokens: Mutex<HashMap<ProcessKey, ProcessCancellationToken>>,
}

impl ProcessCancellationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        scope: &ResourceScope,
        process_id: ProcessId,
    ) -> ProcessCancellationToken {
        let token = ProcessCancellationToken::new();
        self.tokens_guard()
            .insert(ProcessKey::new(scope, process_id), token.clone());
        token
    }

    pub fn cancel(&self, scope: &ResourceScope, process_id: ProcessId) -> bool {
        let token = self
            .tokens_guard()
            .remove(&ProcessKey::new(scope, process_id));
        if let Some(token) = token {
            token.cancel();
            true
        } else {
            false
        }
    }

    pub fn unregister(&self, scope: &ResourceScope, process_id: ProcessId) {
        self.tokens_guard()
            .remove(&ProcessKey::new(scope, process_id));
    }

    fn tokens_guard(&self) -> MutexGuard<'_, HashMap<ProcessKey, ProcessCancellationToken>> {
        self.tokens
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

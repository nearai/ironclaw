//! Per-thread catalog of caller-provided external tools.
//!
//! The Responses API (`/v1/responses`) lets clients declare their own
//! `function`-typed tools alongside the agent's internal action surface.
//! Those tools are not registered through `ToolRegistry` — they are
//! caller-executed: the LLM emits a structured tool call, the engine
//! pauses with `ResumeKind::External`, and the caller posts the result
//! back as a `function_call_output` item.
//!
//! This catalog is the per-thread mapping the bridge consults to
//! distinguish caller tools from internal actions:
//!
//! - `register(thread_id, actions)` — called by the Responses API
//!   handler before the request reaches the agent loop.
//! - `list(thread_id)` — `EffectBridgeAdapter::available_actions`
//!   merges these into the LLM-visible action surface so the model
//!   sees the caller tools as callable.
//! - `contains(thread_id, name)` — `EffectBridgeAdapter::execute_action`
//!   short-circuits to a `GatePaused { resume_kind: External { ... } }`
//!   error for any name in the catalog.
//! - `clear(thread_id)` — invoked when a thread reaches a terminal
//!   state so the entry doesn't leak.
//! - `sweep_older_than(max_age)` — backstop TTL eviction for threads
//!   that get stuck in `Waiting` because the caller never POSTed back.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use ironclaw_engine::{ActionDef, ThreadId};
use tokio::sync::RwLock;

/// Callback-id prefix used for external-tool pause gates. The bridge
/// router's projection uses this prefix to distinguish caller-tool
/// pauses (which surface as `AppEvent::ExternalToolCall` for the
/// Responses API) from OAuth/pairing pauses (which keep going through
/// the existing `AppEvent::GateRequired` channel).
pub const EXTERNAL_TOOL_CALLBACK_PREFIX: &str = "ext_tool:";

/// Build a fully-qualified callback id for an external tool pause.
///
/// The `call_id` is the LLM-emitted tool call identifier (e.g.
/// `call_AbCd123…`); we stamp it onto the prefix so the resume payload
/// can be matched back to the originating action call without a
/// secondary lookup.
pub fn external_tool_callback_id(call_id: &str) -> String {
    format!("{EXTERNAL_TOOL_CALLBACK_PREFIX}{call_id}")
}

/// Returns true when a callback id was produced by `external_tool_callback_id`.
pub fn is_external_tool_callback_id(callback_id: &str) -> bool {
    callback_id.starts_with(EXTERNAL_TOOL_CALLBACK_PREFIX)
}

/// Strip the external-tool prefix from a callback id, returning the
/// embedded `call_id`. Returns `None` if the callback id was not
/// produced by `external_tool_callback_id`.
pub fn call_id_from_external_callback(callback_id: &str) -> Option<&str> {
    callback_id.strip_prefix(EXTERNAL_TOOL_CALLBACK_PREFIX)
}

/// One catalog entry: the caller-provided action defs plus when they
/// were registered (used by the TTL sweep).
#[derive(Debug, Clone)]
pub struct ExternalToolEntry {
    pub actions: Vec<ActionDef>,
    pub registered_at: DateTime<Utc>,
}

/// Per-thread registry of caller-provided external tools.
///
/// Single instance lives on the bridge, shared via `Arc` between the
/// Responses API handler (writer) and the `EffectBridgeAdapter`
/// (reader).
#[derive(Debug, Default)]
pub struct ExternalToolCatalog {
    inner: RwLock<HashMap<ThreadId, ExternalToolEntry>>,
}

impl ExternalToolCatalog {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Replace the catalog entry for `thread_id` with `actions`.
    /// Updating instead of merging matches the Responses API contract:
    /// each request restates the full `tools[]` list.
    pub async fn register(&self, thread_id: ThreadId, actions: Vec<ActionDef>) {
        let mut map = self.inner.write().await;
        map.insert(
            thread_id,
            ExternalToolEntry {
                actions,
                registered_at: Utc::now(),
            },
        );
    }

    /// Snapshot the registered action defs for a thread. Empty vec if
    /// nothing is registered.
    pub async fn list(&self, thread_id: ThreadId) -> Vec<ActionDef> {
        let map = self.inner.read().await;
        map.get(&thread_id)
            .map(|entry| entry.actions.clone())
            .unwrap_or_default()
    }

    /// Whether `action_name` is in this thread's catalog.
    pub async fn contains(&self, thread_id: ThreadId, action_name: &str) -> bool {
        let map = self.inner.read().await;
        map.get(&thread_id)
            .map(|entry| entry.actions.iter().any(|a| a.name == action_name))
            .unwrap_or(false)
    }

    /// Drop the entry for `thread_id`. Called when a thread reaches a
    /// terminal state, or when the caller explicitly cancels.
    pub async fn clear(&self, thread_id: ThreadId) {
        let mut map = self.inner.write().await;
        map.remove(&thread_id);
    }

    /// Evict entries older than `max_age`. Returns the thread ids that
    /// were dropped. Backstop for callers that abandon a paused thread.
    pub async fn sweep_older_than(&self, max_age: Duration) -> Vec<ThreadId> {
        let cutoff = Utc::now() - max_age;
        let mut map = self.inner.write().await;
        let stale: Vec<ThreadId> = map
            .iter()
            .filter(|(_, entry)| entry.registered_at < cutoff)
            .map(|(id, _)| *id)
            .collect();
        for id in &stale {
            map.remove(id);
        }
        stale
    }

    /// Number of registered threads. For diagnostics / metrics.
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_engine::{EffectType, ModelToolSurface};

    fn action(name: &str) -> ActionDef {
        ActionDef {
            name: name.to_string(),
            description: format!("test {name}"),
            parameters_schema: serde_json::json!({"type": "object"}),
            effects: vec![EffectType::Compute],
            requires_approval: false,
            model_tool_surface: ModelToolSurface::FullSchema,
            discovery: None,
        }
    }

    #[tokio::test]
    async fn register_then_list_returns_actions() {
        let catalog = ExternalToolCatalog::new();
        let thread_id = ThreadId::new();
        catalog
            .register(thread_id, vec![action("lookup"), action("convert")])
            .await;
        let listed = catalog.list(thread_id).await;
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].name, "lookup");
        assert_eq!(listed[1].name, "convert");
    }

    #[tokio::test]
    async fn contains_matches_registered_name() {
        let catalog = ExternalToolCatalog::new();
        let thread_id = ThreadId::new();
        catalog.register(thread_id, vec![action("lookup")]).await;
        assert!(catalog.contains(thread_id, "lookup").await);
        assert!(!catalog.contains(thread_id, "other").await);
        let other = ThreadId::new();
        assert!(!catalog.contains(other, "lookup").await);
    }

    #[tokio::test]
    async fn register_replaces_existing_entry() {
        let catalog = ExternalToolCatalog::new();
        let thread_id = ThreadId::new();
        catalog.register(thread_id, vec![action("a")]).await;
        catalog
            .register(thread_id, vec![action("b"), action("c")])
            .await;
        let listed = catalog.list(thread_id).await;
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|a| a.name == "b"));
        assert!(!listed.iter().any(|a| a.name == "a"));
    }

    #[tokio::test]
    async fn clear_removes_entry() {
        let catalog = ExternalToolCatalog::new();
        let thread_id = ThreadId::new();
        catalog.register(thread_id, vec![action("a")]).await;
        assert_eq!(catalog.len().await, 1);
        catalog.clear(thread_id).await;
        assert!(catalog.is_empty().await);
    }

    #[tokio::test]
    async fn sweep_evicts_old_entries_only() {
        let catalog = ExternalToolCatalog::new();
        let fresh = ThreadId::new();
        let stale = ThreadId::new();
        catalog.register(fresh, vec![action("a")]).await;
        catalog.register(stale, vec![action("b")]).await;
        // Backdate the stale entry by mutating the inner map directly.
        // Tests own the RwLock, so this is fine.
        {
            let mut map = catalog.inner.write().await;
            if let Some(entry) = map.get_mut(&stale) {
                entry.registered_at = Utc::now() - Duration::hours(2);
            }
        }
        let evicted = catalog.sweep_older_than(Duration::hours(1)).await;
        assert_eq!(evicted, vec![stale]);
        assert!(!catalog.contains(stale, "b").await);
        assert!(catalog.contains(fresh, "a").await);
    }

    #[test]
    fn callback_id_round_trip() {
        let cb = external_tool_callback_id("call_abc123");
        assert!(is_external_tool_callback_id(&cb));
        assert_eq!(call_id_from_external_callback(&cb), Some("call_abc123"));
        assert!(!is_external_tool_callback_id("pairing:telegram"));
        assert_eq!(call_id_from_external_callback("pairing:telegram"), None);
    }
}

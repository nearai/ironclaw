//! Per-user MCP client registry.
//!
//! Separates MCP client ownership from the global `ToolRegistry`. The
//! `ToolRegistry` is keyed by tool name only and is shared across users;
//! prior to this module, `McpToolWrapper` embedded the activating user's
//! `Arc<McpClient>` directly, so the second user's activation silently
//! overwrote the first user's wrapper — both users ended up dispatching
//! through whichever client got registered last. See
//! `.claude/rules/safety-and-sandbox.md` "Cache Keys Must Be Complete".
//!
//! `McpClientStore` holds the `(user_id, server_name) -> Arc<McpClient>`
//! mapping and is the source of truth at tool-dispatch time. Each
//! `McpToolWrapper` holds an `Arc<McpClientStore>` + `server_name` and
//! resolves the right client from `JobContext.user_id` on every call.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use super::client::McpClient;

/// Composite key identifying an MCP client instance: the authenticating
/// user plus the server name. Both fields participate in `Hash` / `Eq` so
/// two users can hold active clients against the same server
/// simultaneously without key collision.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct McpClientKey {
    pub user_id: String,
    pub server_name: String,
}

impl McpClientKey {
    pub fn new(user_id: &str, server_name: &str) -> Self {
        Self {
            user_id: user_id.to_string(),
            server_name: server_name.to_string(),
        }
    }
}

/// Per-user MCP client registry. Typically held as `Arc<McpClientStore>`
/// by both `ExtensionManager` (for lifecycle) and every `McpToolWrapper`
/// (for dispatch-time lookup).
#[derive(Default)]
pub struct McpClientStore {
    clients: RwLock<HashMap<McpClientKey, Arc<McpClient>>>,
}

impl McpClientStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace the client for `(user_id, server_name)`. Replacing
    /// is only intended for the same user re-activating the same server.
    pub async fn insert(&self, user_id: &str, server_name: &str, client: Arc<McpClient>) {
        self.clients
            .write()
            .await
            .insert(McpClientKey::new(user_id, server_name), client);
    }

    /// Remove and return the client for `(user_id, server_name)`, if any.
    pub async fn remove(&self, user_id: &str, server_name: &str) -> Option<Arc<McpClient>> {
        self.clients
            .write()
            .await
            .remove(&McpClientKey::new(user_id, server_name))
    }

    /// Atomically remove `(user_id, server_name)` and report whether the
    /// server has zero remaining users after the removal. Holds the write
    /// lock across both the `remove` and the emptiness check so a
    /// concurrent `insert` (user C activating) or `remove` (user B) can't
    /// slip between the two and produce a stale "last user out" decision.
    ///
    /// Callers use the returned boolean to decide whether the server's
    /// global tool wrappers should be unregistered from the
    /// `ToolRegistry`. That decision is still racy against a concurrent
    /// activation that *starts after* this call returns — the
    /// extension-manager-level per-server lifecycle lock is what
    /// serialises activate and remove end-to-end.
    pub async fn remove_and_check_empty(&self, user_id: &str, server_name: &str) -> bool {
        let mut clients = self.clients.write().await;
        clients.remove(&McpClientKey::new(user_id, server_name));
        !clients.keys().any(|key| key.server_name == server_name)
    }

    /// Look up the client for `(user_id, server_name)`. Returns `None` if
    /// the user hasn't activated the server.
    pub async fn get(&self, user_id: &str, server_name: &str) -> Option<Arc<McpClient>> {
        self.clients
            .read()
            .await
            .get(&McpClientKey::new(user_id, server_name))
            .cloned()
    }

    /// Whether `(user_id, server_name)` has an active client.
    pub async fn contains(&self, user_id: &str, server_name: &str) -> bool {
        self.clients
            .read()
            .await
            .contains_key(&McpClientKey::new(user_id, server_name))
    }

    /// Whether ANY user still has this server active. Used by the remove
    /// path to decide whether the server's global tool wrappers can be
    /// unregistered — they must survive as long as some user is still
    /// holding the server active.
    pub async fn any_active_for_server(&self, server_name: &str) -> bool {
        self.clients
            .read()
            .await
            .keys()
            .any(|key| key.server_name == server_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::mcp::McpClient;

    #[tokio::test]
    async fn insert_and_get_are_per_user() {
        let store = McpClientStore::new();
        let client_a = Arc::new(McpClient::new_with_name("notion", "http://a.invalid"));
        let client_b = Arc::new(McpClient::new_with_name("notion", "http://b.invalid"));

        store.insert("user-a", "notion", client_a.clone()).await;
        store.insert("user-b", "notion", client_b.clone()).await;

        assert!(Arc::ptr_eq(
            &store.get("user-a", "notion").await.expect("a"),
            &client_a
        ));
        assert!(Arc::ptr_eq(
            &store.get("user-b", "notion").await.expect("b"),
            &client_b
        ));
    }

    #[tokio::test]
    async fn remove_and_check_empty_reports_last_user_out() {
        let store = McpClientStore::new();
        let client_a = Arc::new(McpClient::new_with_name("notion", "http://a.invalid"));
        let client_b = Arc::new(McpClient::new_with_name("notion", "http://b.invalid"));

        store.insert("user-a", "notion", client_a).await;
        store.insert("user-b", "notion", client_b).await;

        assert!(
            !store.remove_and_check_empty("user-a", "notion").await,
            "removing user-a while user-b still holds notion must not report empty"
        );
        assert!(
            store.remove_and_check_empty("user-b", "notion").await,
            "removing user-b (last user) must report empty"
        );
        assert!(
            !store.contains("user-b", "notion").await,
            "removal must have actually taken effect"
        );
    }

    #[tokio::test]
    async fn remove_and_check_empty_is_idempotent_on_missing_user() {
        let store = McpClientStore::new();
        let client = Arc::new(McpClient::new_with_name("notion", "http://a.invalid"));
        store.insert("user-a", "notion", client).await;

        assert!(
            !store
                .remove_and_check_empty("user-never-activated", "notion")
                .await,
            "removing a user who never activated must leave the existing user's client in place"
        );
        assert!(store.contains("user-a", "notion").await);
    }

    #[tokio::test]
    async fn any_active_for_server_tracks_multi_tenancy() {
        let store = McpClientStore::new();
        let client = Arc::new(McpClient::new_with_name("notion", "http://a.invalid"));

        assert!(!store.any_active_for_server("notion").await);
        store.insert("user-a", "notion", client.clone()).await;
        assert!(store.any_active_for_server("notion").await);
        store.insert("user-b", "notion", client).await;

        assert!(store.remove("user-a", "notion").await.is_some());
        assert!(
            store.any_active_for_server("notion").await,
            "user-b still holds the server; global wrappers must stay registered"
        );
        assert!(store.remove("user-b", "notion").await.is_some());
        assert!(!store.any_active_for_server("notion").await);
    }
}

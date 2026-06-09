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

use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use uuid::Uuid;

use super::client::McpClient;
use super::protocol::McpTool;

/// Render a `serde_json::Value` as a stable, order-insensitive
/// canonical JSON string: object keys are sorted recursively. Used
/// by `surface_signature` so two schemas that are semantically
/// equivalent but differ only in JSON key order produce the same
/// fingerprint. Without this, a backend that emits `{"a":1,"b":2}`
/// on one call and `{"b":2,"a":1}` on the next — both legal JSON —
/// would falsely trip the cross-tenant conflict check.
fn canonicalize_json(value: &serde_json::Value) -> String {
    fn recurse(value: &serde_json::Value, out: &mut String) {
        match value {
            serde_json::Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push('{');
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    // `serde_json::to_string` on a String handles the
                    // escape rules correctly.
                    out.push_str(&serde_json::to_string(k).unwrap_or_default());
                    out.push(':');
                    recurse(&map[*k], out);
                }
                out.push('}');
            }
            serde_json::Value::Array(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    recurse(v, out);
                }
                out.push(']');
            }
            other => {
                // Null / bool / number / string: serde_json's default
                // serialization is already canonical.
                out.push_str(&serde_json::to_string(other).unwrap_or_default());
            }
        }
    }
    let mut buf = String::new();
    recurse(value, &mut buf);
    buf
}

/// Compute a deterministic fingerprint of an MCP server's reported tool
/// surface. Used by `McpClientStore::check_surface_conflict` to detect
/// when two users activate the same `server_name` but the backend
/// returns a different set of tools, different parameter schemas, or
/// different behavioral annotations — the global `ToolRegistry` is
/// keyed by tool name only, so the second activation would silently
/// shadow the first and leak whichever dimension differed across
/// tenants.
///
/// The fingerprint covers every dimension of the tool surface that
/// affects runtime behavior visible to the LLM or the approval
/// pipeline:
/// - `name` + `description` (schema advertised to the LLM)
/// - `input_schema` (parameter validation shape)
/// - `annotations` (approval gating — `destructive_hint` drives
///   `McpTool::requires_approval`, and `ToolRegistry` treats the
///   globally-registered wrapper's approval policy as authoritative
///   for every caller. Two backends returning the same schema but
///   different `destructive_hint` must therefore be treated as
///   conflicting surfaces, else one user's approval semantics leak
///   to the other.)
///
/// JSON values (`input_schema`, `annotations`) are canonicalized
/// (object keys sorted recursively) so that semantically equivalent
/// payloads with different key order produce identical fingerprints.
/// Tool list is sorted by name so server-side ordering doesn't
/// influence the hash either.
pub fn surface_signature(tools: &[McpTool]) -> String {
    let mut entries: Vec<(String, String, String, String)> = tools
        .iter()
        .map(|t| {
            (
                t.name.clone(),
                t.description.clone(),
                canonicalize_json(&t.input_schema),
                t.annotations
                    .as_ref()
                    .map(|a| {
                        canonicalize_json(
                            &serde_json::to_value(a).unwrap_or(serde_json::Value::Null),
                        )
                    })
                    .unwrap_or_default(),
            )
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (name, description, schema, annotations) in &entries {
        hasher.update(name.as_bytes());
        hasher.update(b"\x00");
        hasher.update(description.as_bytes());
        hasher.update(b"\x00");
        hasher.update(schema.as_bytes());
        hasher.update(b"\x00");
        hasher.update(annotations.as_bytes());
        hasher.update(b"\x01");
    }
    format!("{:x}", hasher.finalize())
}

/// Composite key identifying an MCP client instance: the authenticating
/// user plus the server name plus an optional Thread id. All fields
/// participate in `Hash` / `Eq` so:
/// - Two users can hold active clients against the same server simultaneously.
/// - Two concurrent IronClaw Threads under one user activating the same server
///   get distinct `Arc<McpClient>` instances (and thus distinct MCP protocol
///   sessions), so any per-session state the upstream MCP server holds is
///   partitioned per Thread, not shared across all of a user's Threads.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct McpClientKey {
    pub user_id: String,
    pub server_name: String,
    /// `None` = user-scoped (legacy); `Some(uuid)` = thread-scoped.
    pub thread_id: Option<Uuid>,
}

impl McpClientKey {
    /// Create a user-scoped key (no Thread axis).
    pub fn new(user_id: &str, server_name: &str) -> Self {
        Self {
            user_id: user_id.to_string(),
            server_name: server_name.to_string(),
            thread_id: None,
        }
    }

    /// Create a thread-scoped key.
    ///
    /// Two Threads under the same user activating the same server receive
    /// distinct `Arc<McpClient>` instances because `thread_id` participates in
    /// the `Hash`/`Eq` key.
    pub fn new_for_thread(user_id: &str, server_name: &str, thread_id: Uuid) -> Self {
        Self {
            user_id: user_id.to_string(),
            server_name: server_name.to_string(),
            thread_id: Some(thread_id),
        }
    }
}

/// Per-user MCP client entry: the active client plus the fingerprint
/// of the tool surface it exposes. The signature is captured at
/// activation time and is what `check_surface_conflict` compares
/// across users.
#[derive(Clone)]
struct McpClientEntry {
    client: Arc<McpClient>,
    surface: String,
}

/// Per-user MCP client registry. Typically held as `Arc<McpClientStore>`
/// by both `ExtensionManager` (for lifecycle) and every `McpToolWrapper`
/// (for dispatch-time lookup).
#[derive(Default)]
pub struct McpClientStore {
    clients: RwLock<HashMap<McpClientKey, McpClientEntry>>,
}

impl McpClientStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace the client for `(user_id, server_name)`. The
    /// signature is the fingerprint of the tool surface this client
    /// reported at activation time (see `surface_signature`). Replacing
    /// is only intended for the same user re-activating the same server.
    pub async fn insert(
        &self,
        user_id: &str,
        server_name: &str,
        client: Arc<McpClient>,
        surface: String,
    ) {
        self.clients.write().await.insert(
            McpClientKey::new(user_id, server_name),
            McpClientEntry { client, surface },
        );
    }

    /// Insert or replace the client for the given `McpClientKey`.
    ///
    /// Allows callers to provide a fully-constructed key (including optional
    /// `thread_id`) without going through the `(user_id, server_name)` string pair.
    pub async fn insert_with_key(
        &self,
        key: McpClientKey,
        client: Arc<McpClient>,
        surface: String,
    ) {
        self.clients
            .write()
            .await
            .insert(key, McpClientEntry { client, surface });
    }

    /// Look up the client for the given `McpClientKey`.
    pub async fn get_with_key(&self, key: &McpClientKey) -> Option<Arc<McpClient>> {
        self.clients
            .read()
            .await
            .get(key)
            .map(|entry| entry.client.clone())
    }

    /// Remove and return the client for `(user_id, server_name)`, if any.
    pub async fn remove(&self, user_id: &str, server_name: &str) -> Option<Arc<McpClient>> {
        self.clients
            .write()
            .await
            .remove(&McpClientKey::new(user_id, server_name))
            .map(|entry| entry.client)
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
            .map(|entry| entry.client.clone())
    }

    /// Resolve the right `McpClient` for the `(user, server, thread)` triple.
    ///
    /// - `thread_id == None` → returns the user-scoped base client (unchanged
    ///   behaviour; CLI and boot paths take this route).
    /// - `thread_id == Some(tid)` → returns the thread-scoped client if already
    ///   cached; otherwise lazily materialises one by calling
    ///   [`McpClient::for_thread`] on the user-scoped base. The materialised
    ///   client is inserted into the store and returned as an `Arc<McpClient>`.
    ///
    /// Returns `None` only when the user-scoped base client for this server
    /// has never been activated (i.e. `get(user, server)` would also be
    /// `None`).
    pub async fn resolve_for_thread(
        &self,
        user_id: &str,
        server_name: &str,
        thread_id: Option<Uuid>,
    ) -> Option<Arc<McpClient>> {
        let Some(tid) = thread_id else {
            // No thread context → user-scoped lookup (backward compat).
            return self.get(user_id, server_name).await;
        };

        let thread_key = McpClientKey::new_for_thread(user_id, server_name, tid);

        // Fast path: already materialised.
        {
            let guard = self.clients.read().await;
            if let Some(entry) = guard.get(&thread_key) {
                return Some(entry.client.clone());
            }
        }

        // Slow path: materialise from the user-scoped base.
        // Acquire the write lock first so we atomically read the base,
        // materialise the fork, and insert it — preventing a race where two
        // concurrent callers both miss the fast-path and double-insert.
        let mut guard = self.clients.write().await;

        // Double-check under write lock in case another caller beat us.
        if let Some(entry) = guard.get(&thread_key) {
            return Some(entry.client.clone());
        }

        // Look up the base client under the write lock (still present).
        let base = guard.get(&McpClientKey::new(user_id, server_name))?;
        let surface = base.surface.clone();
        let forked = Arc::new(base.client.for_thread(tid));
        tracing::debug!(
            user_id = %user_id,
            server = %server_name,
            thread_id = %tid,
            "McpClientStore: materialised thread-scoped fork"
        );
        guard.insert(
            thread_key,
            McpClientEntry {
                client: forked.clone(),
                surface,
            },
        );
        Some(forked)
    }

    /// Whether `(user_id, server_name)` has an active client.
    /// Evict all thread-scoped client entries for the given thread UUID.
    ///
    /// Removes every `(user, server, thread_id)` entry that was lazily
    /// materialised for `thread_id` via [`resolve_for_thread`], while leaving
    /// user-scoped entries (`thread_id == None`) intact.
    ///
    /// # Wiring note
    /// No production caller yet — the agent runtime does not currently emit
    /// a "Thread terminated" event the store can subscribe to. The method
    /// is provided so the memory-growth guard is in place once the
    /// lifecycle hook exists; until then, thread-scoped entries
    /// accumulate for the lifetime of the process.
    pub async fn evict_thread(&self, thread_id: Uuid) {
        self.clients
            .write()
            .await
            .retain(|key, _| key.thread_id != Some(thread_id));
    }

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

    /// Check whether the tool surface `incoming` — fingerprint of the
    /// tools reported by the activating client — is compatible with any
    /// OTHER user who already has `server_name` active.
    ///
    /// Returns `Some(other_user_id)` if a conflicting entry exists: a
    /// different user has the same `server_name` active with a DIFFERENT
    /// surface fingerprint. Same-user re-activations are ignored
    /// because they're expected to replace the old entry.
    ///
    /// The `ToolRegistry` is keyed by tool name only, so two users on
    /// the "same" server name with different URLs or different
    /// credentials can produce different schemas. Without this check
    /// the second user's registration would silently shadow the first's
    /// — see the reviewer's concern that one user's `list_tools()`
    /// result becomes the shared wrapper surface for everyone.
    pub async fn check_surface_conflict(
        &self,
        user_id: &str,
        server_name: &str,
        incoming: &str,
    ) -> Option<String> {
        let clients = self.clients.read().await;
        for (key, entry) in clients.iter() {
            if key.server_name == server_name && key.user_id != user_id && entry.surface != incoming
            {
                return Some(key.user_id.clone());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::mcp::McpClient;
    use crate::tools::mcp::protocol::{McpTool, McpToolAnnotations};

    fn tool_with_annotations(name: &str, annotations: Option<McpToolAnnotations>) -> McpTool {
        McpTool {
            name: name.to_string(),
            description: "shared-desc".to_string(),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
            annotations,
        }
    }

    #[test]
    fn surface_signature_diverges_when_only_annotations_differ() {
        // Same name / description / schema; only `destructive_hint`
        // differs. McpTool::requires_approval reads that field, and
        // ToolRegistry keys wrappers by tool name — without this
        // dimension in the fingerprint, the second user's activation
        // would be accepted and the globally-registered wrapper's
        // approval policy would leak to the first user's dispatches.
        let safe = tool_with_annotations(
            "do_thing",
            Some(McpToolAnnotations {
                destructive_hint: false,
                ..Default::default()
            }),
        );
        let destructive = tool_with_annotations(
            "do_thing",
            Some(McpToolAnnotations {
                destructive_hint: true,
                ..Default::default()
            }),
        );

        let sig_safe = surface_signature(std::slice::from_ref(&safe));
        let sig_destructive = surface_signature(std::slice::from_ref(&destructive));
        assert_ne!(
            sig_safe, sig_destructive,
            "annotation-only divergence must produce distinct fingerprints so \
             cross-user activations with different approval policies are \
             rejected instead of sharing one registered wrapper",
        );

        // And make the round-trip obvious: identical annotations must
        // still fingerprint identically.
        let also_safe = tool_with_annotations(
            "do_thing",
            Some(McpToolAnnotations {
                destructive_hint: false,
                ..Default::default()
            }),
        );
        assert_eq!(
            sig_safe,
            surface_signature(std::slice::from_ref(&also_safe)),
            "matching annotations must fingerprint identically",
        );
    }

    #[test]
    fn surface_signature_is_object_key_order_insensitive() {
        // JSON object key ordering is not semantically meaningful, and
        // a server is free to emit the same schema with different key
        // order across calls. Without canonicalization, two equivalent
        // schemas would produce different fingerprints and incorrectly
        // trip the cross-tenant conflict check, blocking legitimate
        // multi-user activation.
        let t1 = McpTool {
            name: "do_thing".into(),
            description: "d".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"a": {"type": "string"}, "b": {"type": "integer"}},
                "required": ["a", "b"]
            }),
            annotations: None,
        };
        let t2 = McpTool {
            name: "do_thing".into(),
            description: "d".into(),
            input_schema: serde_json::json!({
                "required": ["a", "b"],
                "properties": {"b": {"type": "integer"}, "a": {"type": "string"}},
                "type": "object"
            }),
            annotations: None,
        };
        assert_eq!(
            surface_signature(std::slice::from_ref(&t1)),
            surface_signature(std::slice::from_ref(&t2)),
            "equivalent schemas with reordered keys must fingerprint identically",
        );
    }

    #[test]
    fn surface_signature_treats_missing_vs_default_annotations_distinctly() {
        // `None` vs `Some(default)` are different wire shapes (the
        // server either omitted `annotations` entirely or returned an
        // explicit empty object). The fingerprint should reflect the
        // actual bytes the server sent so two backends that disagree
        // on whether to emit the field are not merged into one
        // wrapper surface.
        let none = tool_with_annotations("do_thing", None);
        let default_some = tool_with_annotations("do_thing", Some(McpToolAnnotations::default()));
        assert_ne!(
            surface_signature(std::slice::from_ref(&none)),
            surface_signature(std::slice::from_ref(&default_some)),
        );
    }

    #[tokio::test]
    async fn insert_and_get_are_per_user() {
        let store = McpClientStore::new();
        let client_a = Arc::new(McpClient::new_with_name("notion", "http://a.invalid"));
        let client_b = Arc::new(McpClient::new_with_name("notion", "http://b.invalid"));

        store
            .insert("user-a", "notion", client_a.clone(), "sig-a".into())
            .await;
        store
            .insert("user-b", "notion", client_b.clone(), "sig-b".into())
            .await;

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

        store
            .insert("user-a", "notion", client_a, "sig".into())
            .await;
        store
            .insert("user-b", "notion", client_b, "sig".into())
            .await;

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
        store.insert("user-a", "notion", client, "sig".into()).await;

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
        store
            .insert("user-a", "notion", client.clone(), "sig".into())
            .await;
        assert!(store.any_active_for_server("notion").await);
        store.insert("user-b", "notion", client, "sig".into()).await;

        assert!(store.remove("user-a", "notion").await.is_some());
        assert!(
            store.any_active_for_server("notion").await,
            "user-b still holds the server; global wrappers must stay registered"
        );
        assert!(store.remove("user-b", "notion").await.is_some());
        assert!(!store.any_active_for_server("notion").await);
    }

    #[tokio::test]
    async fn check_surface_conflict_flags_divergent_surface_for_same_server() {
        let store = McpClientStore::new();
        let client = Arc::new(McpClient::new_with_name("notion", "http://a.invalid"));
        store
            .insert("user-a", "notion", client, "surface-v1".into())
            .await;

        assert_eq!(
            store
                .check_surface_conflict("user-b", "notion", "surface-v2")
                .await,
            Some("user-a".to_string()),
            "user-b activating notion with a different surface than user-a must flag user-a as the conflict source",
        );
        assert!(
            store
                .check_surface_conflict("user-b", "notion", "surface-v1")
                .await
                .is_none(),
            "identical surface fingerprint means no conflict — both users get the same wrapper shape",
        );
        assert!(
            store
                .check_surface_conflict("user-a", "notion", "surface-v2")
                .await
                .is_none(),
            "same-user re-activation with a new surface is allowed (caller replaces their own entry)",
        );
    }
    // ── Thread-axis partition tests ──────────────────────────────────────

    /// Two distinct Threads under the same user activating the same server
    /// must get distinct `Arc<McpClient>` instances. Without the `thread_id`
    /// axis, the second Thread's activation would overwrite the first's entry,
    /// sharing one `McpClient` and one MCP protocol session — defeating the
    /// session-isolation guarantee `McpSessionKey.thread_id` provides.
    #[tokio::test]
    async fn distinct_threads_get_distinct_client_instances() {
        use uuid::Uuid;
        let store = McpClientStore::new();
        let client_t1 = Arc::new(McpClient::new_with_name("notion", "http://a.invalid"));
        let client_t2 = Arc::new(McpClient::new_with_name("notion", "http://b.invalid"));

        let thread_1 = Uuid::from_u128(1);
        let thread_2 = Uuid::from_u128(2);
        let key1 = McpClientKey::new_for_thread("user-a", "notion", thread_1);
        let key2 = McpClientKey::new_for_thread("user-a", "notion", thread_2);

        store
            .insert_with_key(key1.clone(), client_t1.clone(), "sig".into())
            .await;
        store
            .insert_with_key(key2.clone(), client_t2.clone(), "sig".into())
            .await;

        let got1 = store.get_with_key(&key1).await.expect("thread 1 client");
        let got2 = store.get_with_key(&key2).await.expect("thread 2 client");

        assert!(
            Arc::ptr_eq(&got1, &client_t1),
            "Thread 1 must return its own McpClient"
        );
        assert!(
            Arc::ptr_eq(&got2, &client_t2),
            "Thread 2 must return its own McpClient"
        );
        assert!(
            !Arc::ptr_eq(&got1, &got2),
            "Thread 1 and Thread 2 must NOT share a McpClient instance"
        );
    }

    /// A thread-scoped key and a user-scoped key with the same `(user, server)`
    /// must coexist as distinct entries — the `None` / `Some` distinction on
    /// `thread_id` must participate in the hash.
    #[tokio::test]
    async fn thread_scoped_and_user_scoped_keys_coexist() {
        use uuid::Uuid;
        let store = McpClientStore::new();
        let client_user = Arc::new(McpClient::new_with_name("notion", "http://user.invalid"));
        let client_thread = Arc::new(McpClient::new_with_name("notion", "http://thread.invalid"));

        let thread_a = Uuid::from_u128(42);
        let key_user = McpClientKey::new("user-a", "notion");
        let key_thread = McpClientKey::new_for_thread("user-a", "notion", thread_a);

        store
            .insert_with_key(key_user.clone(), client_user.clone(), "sig".into())
            .await;
        store
            .insert_with_key(key_thread.clone(), client_thread.clone(), "sig".into())
            .await;

        let got_user = store
            .get_with_key(&key_user)
            .await
            .expect("user-scoped client");
        let got_thread = store
            .get_with_key(&key_thread)
            .await
            .expect("thread-scoped client");

        assert!(
            Arc::ptr_eq(&got_user, &client_user),
            "user-scoped key must return the user-scoped client"
        );
        assert!(
            Arc::ptr_eq(&got_thread, &client_thread),
            "thread-scoped key must return the thread-scoped client"
        );
        assert!(
            !Arc::ptr_eq(&got_user, &got_thread),
            "user-scoped and thread-scoped clients must be distinct instances"
        );
        assert_eq!(
            store.clients.read().await.len(),
            2,
            "both keys must exist as separate entries in the store"
        );
    }

    // ── resolve_for_thread tests ───────────────────────────────────────────

    /// `resolve_for_thread` with `None` returns the same `Arc<McpClient>` as
    /// the plain `get` — backward-compatible user-scoped path unchanged.
    #[tokio::test]
    async fn resolve_for_thread_none_returns_user_scoped_client() {
        let store = McpClientStore::new();
        let base = Arc::new(McpClient::new_with_name("svc", "http://base.invalid"));
        store
            .insert("user-a", "svc", base.clone(), "sig".into())
            .await;

        let got = store
            .resolve_for_thread("user-a", "svc", None)
            .await
            .expect("must return base client for None thread_id");
        assert!(
            Arc::ptr_eq(&got, &base),
            "None thread_id must return the user-scoped base client unchanged"
        );
    }

    /// `resolve_for_thread` with two distinct thread UUIDs returns two distinct
    /// `Arc<McpClient>` instances (different pointer identity).
    #[tokio::test]
    async fn resolve_for_thread_distinct_uuids_yield_distinct_clients() {
        use uuid::Uuid;
        let store = McpClientStore::new();
        let base = Arc::new(McpClient::new_with_name("svc", "http://base.invalid"));
        store
            .insert("user-a", "svc", base.clone(), "sig".into())
            .await;

        let tid_a = Uuid::from_u128(0xAAAA);
        let tid_b = Uuid::from_u128(0xBBBB);

        let got_a = store
            .resolve_for_thread("user-a", "svc", Some(tid_a))
            .await
            .expect("thread A client");
        let got_b = store
            .resolve_for_thread("user-a", "svc", Some(tid_b))
            .await
            .expect("thread B client");

        assert!(
            !Arc::ptr_eq(&got_a, &got_b),
            "distinct thread UUIDs must yield distinct client instances"
        );
        assert!(
            !Arc::ptr_eq(&got_a, &base),
            "thread-scoped client must be distinct from the user-scoped base"
        );
    }

    /// The materialised thread-scoped client has `thread_id = Some(uuid)` and
    /// fresh init state (the `initialized` OnceCell is not yet set).
    #[tokio::test]
    async fn resolve_for_thread_materialised_client_has_correct_thread_id() {
        use uuid::Uuid;
        let store = McpClientStore::new();
        let base = Arc::new(McpClient::new_with_name("svc", "http://base.invalid"));
        store
            .insert("user-a", "svc", base.clone(), "sig".into())
            .await;

        let tid = Uuid::from_u128(0xDEAD_BEEF);
        let got = store
            .resolve_for_thread("user-a", "svc", Some(tid))
            .await
            .expect("materialised client");

        assert_eq!(
            got.thread_id(),
            Some(tid),
            "materialised client must carry thread_id = Some(tid)"
        );
    }

    /// A second call with the same thread UUID returns the *same* cached Arc —
    /// we don't materialise a new client on every call.
    #[tokio::test]
    async fn resolve_for_thread_caches_materialised_client() {
        use uuid::Uuid;
        let store = McpClientStore::new();
        let base = Arc::new(McpClient::new_with_name("svc", "http://base.invalid"));
        store
            .insert("user-a", "svc", base.clone(), "sig".into())
            .await;

        let tid = Uuid::from_u128(0xCAFE);
        let first = store
            .resolve_for_thread("user-a", "svc", Some(tid))
            .await
            .expect("first call");
        let second = store
            .resolve_for_thread("user-a", "svc", Some(tid))
            .await
            .expect("second call");

        assert!(
            Arc::ptr_eq(&first, &second),
            "repeated calls with the same thread UUID must return the cached client"
        );
    }

    // ── evict_thread tests ─────────────────────────────────────────────────

    /// `evict_thread` removes all thread-scoped entries for `thread_id` while
    /// leaving user-scoped and other-thread entries intact.
    #[tokio::test]
    async fn evict_thread_removes_only_matching_thread_entries() {
        use crate::tools::mcp::McpClient;
        let store = McpClientStore::new();
        let base = Arc::new(McpClient::new("http://localhost:9099"));

        let tid_a = Uuid::from_u128(0xAAAA);
        let tid_b = Uuid::from_u128(0xBBBB);

        // Insert a user-scoped entry and two thread-scoped forks.
        store
            .insert("alice", "svc", Arc::clone(&base), "sig-base".to_string())
            .await;
        let key_a = McpClientKey::new_for_thread("alice", "svc", tid_a);
        let key_b = McpClientKey::new_for_thread("alice", "svc", tid_b);
        store
            .insert_with_key(key_a.clone(), Arc::clone(&base), "sig-a".to_string())
            .await;
        store
            .insert_with_key(key_b.clone(), Arc::clone(&base), "sig-b".to_string())
            .await;

        assert!(
            store.get_with_key(&key_a).await.is_some(),
            "pre: tid_a present"
        );
        assert!(
            store.get_with_key(&key_b).await.is_some(),
            "pre: tid_b present"
        );
        assert!(
            store.contains("alice", "svc").await,
            "pre: user-scoped present"
        );

        store.evict_thread(tid_a).await;

        assert!(
            store.get_with_key(&key_a).await.is_none(),
            "tid_a entry evicted"
        );
        assert!(
            store.get_with_key(&key_b).await.is_some(),
            "tid_b entry survives"
        );
        assert!(
            store.contains("alice", "svc").await,
            "user-scoped entry survives"
        );
    }
}

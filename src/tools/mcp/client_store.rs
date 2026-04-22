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
}

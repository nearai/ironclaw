//! MCP session management.
//!
//! Manages Mcp-Session-Id headers for stateful connections to MCP servers.
//! Each `(user, server)` or `(user, server, thread)` pair has its own
//! session that persists across requests.
//!
//! Sessions are partitioned by `(user_id, server_name, thread_id)` so:
//! - Two users activating the same server get distinct sessions (prevents
//!   cross-tenant `Mcp-Session-Id` sharing).
//! - Two concurrent IronClaw Threads under one user activating the same
//!   server get distinct sessions, so an MCP server that binds state to
//!   its `Mcp-Session-Id` (per-conversation rate limits, persistent
//!   sessions, tenant scoping, etc.) sees one session per Thread, not
//!   one session shared across all of a user's Threads.
//!
//! See `.claude/rules/safety-and-sandbox.md` "Cache Keys Must Be Complete"
//! for the broader cache-key-completeness rationale.

use std::collections::HashMap;
use std::time::Instant;

use ironclaw_common::McpServerName;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Composite key for an MCP session.
///
/// A given user holds one session per server per optional Thread:
/// - `thread_id = None`: user-scoped session — the legacy default, used when
///   the call is not rooted in an IronClaw Thread (test fixtures, CLI paths).
/// - `thread_id = Some(uuid)`: thread-scoped session — issued when a
///   dispatching Thread is known, ensuring that two concurrent Threads under
///   one user never share a `Mcp-Session-Id` for the same server.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct McpSessionKey {
    user_id: String,
    server_name: McpServerName,
    /// `None` = user-scoped (legacy); `Some(uuid)` = thread-scoped.
    thread_id: Option<Uuid>,
}

impl McpSessionKey {
    /// Create a user-scoped session key (no Thread axis).
    ///
    /// Use this for call-paths that are not rooted in a specific IronClaw
    /// Thread — e.g. CLI invocations, test fixtures, or shutdown cleanup.
    /// Two calls with the same `(user, server)` will share a session.
    pub fn new(user_id: impl Into<String>, server_name: McpServerName) -> Self {
        Self {
            user_id: user_id.into(),
            server_name,
            thread_id: None,
        }
    }

    /// Create a thread-scoped session key.
    ///
    /// Use this when the caller is a specific IronClaw Thread. Two Threads
    /// under the same user activating the same server will receive distinct
    /// sessions because `thread_id` participates in the `Hash`/`Eq` key.
    pub fn new_for_thread(
        user_id: impl Into<String>,
        server_name: McpServerName,
        thread_id: Uuid,
    ) -> Self {
        Self {
            user_id: user_id.into(),
            server_name,
            thread_id: Some(thread_id),
        }
    }
}

/// Session state for a single `(user, server[, thread])` MCP connection.
#[derive(Debug, Clone)]
pub struct McpSession {
    /// Session ID returned by the server (via Mcp-Session-Id header).
    pub session_id: Option<String>,

    /// Last activity timestamp for this session.
    pub last_activity: Instant,

    /// Server URL this session is connected to.
    pub server_url: String,

    /// Whether initialization has completed.
    pub initialized: bool,
}

impl McpSession {
    /// Create a new session for a server.
    pub fn new(server_url: impl Into<String>) -> Self {
        Self {
            session_id: None,
            last_activity: Instant::now(),
            server_url: server_url.into(),
            initialized: false,
        }
    }

    /// Update the session ID (from server response).
    pub fn update_session_id(&mut self, session_id: Option<String>) {
        if session_id.is_some() {
            self.session_id = session_id;
        }
        self.last_activity = Instant::now();
    }

    /// Mark the session as initialized.
    pub fn mark_initialized(&mut self) {
        self.initialized = true;
        self.last_activity = Instant::now();
    }

    /// Check if the session has been idle for too long.
    pub fn is_stale(&self, max_idle_secs: u64) -> bool {
        self.last_activity.elapsed().as_secs() > max_idle_secs
    }

    /// Touch the session to update last activity.
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }
}

/// Manages MCP sessions across multiple `(user, server[, thread])` triples.
///
/// Server names are typed via [`McpServerName`] so a free-form string can't
/// bypass allowlist validation at the boundary. Callers convert raw strings
/// via `McpServerName::new` (validating) or `McpServerName::from_trusted`
/// (for names the caller already validated). This makes identity-confusion
/// bugs — matching the shape described in `.claude/rules/types.md` — a
/// compile error rather than a runtime surprise.
pub struct McpSessionManager {
    /// Active sessions keyed by `(user_id, server_name, thread_id)`.
    sessions: RwLock<HashMap<McpSessionKey, McpSession>>,

    /// Maximum idle time before a session is considered stale (in seconds).
    max_idle_secs: u64,
}

impl McpSessionManager {
    /// Create a new session manager with default idle timeout (30 minutes).
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            max_idle_secs: 1800, // 30 minutes
        }
    }

    /// Create a new session manager with custom idle timeout.
    pub fn with_idle_timeout(max_idle_secs: u64) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            max_idle_secs,
        }
    }

    fn key(user_id: &str, server_name: &McpServerName, thread_id: Option<Uuid>) -> McpSessionKey {
        match thread_id {
            Some(tid) => McpSessionKey::new_for_thread(user_id, server_name.clone(), tid),
            None => McpSessionKey::new(user_id, server_name.clone()),
        }
    }

    /// Get or create a session for `(user, server[, thread])`.
    pub async fn get_or_create(
        &self,
        user_id: &str,
        server_name: &McpServerName,
        server_url: &str,
        thread_id: Option<Uuid>,
    ) -> McpSession {
        let key = Self::key(user_id, server_name, thread_id);
        let mut sessions = self.sessions.write().await;

        if let Some(session) = sessions.get(&key) {
            // Check if session is stale
            if session.is_stale(self.max_idle_secs) {
                // Create a fresh session
                let new_session = McpSession::new(server_url);
                sessions.insert(key, new_session.clone());
                return new_session;
            }
            return session.clone();
        }

        // Create new session
        let session = McpSession::new(server_url);
        sessions.insert(key, session.clone());
        session
    }

    /// Get the current session ID for `(user, server[, thread])`, if any.
    pub async fn get_session_id(
        &self,
        user_id: &str,
        server_name: &McpServerName,
        thread_id: Option<Uuid>,
    ) -> Option<String> {
        let sessions = self.sessions.read().await;
        sessions
            .get(&Self::key(user_id, server_name, thread_id))
            .and_then(|s| s.session_id.clone())
    }

    /// Update the session ID from a server response.
    pub async fn update_session_id(
        &self,
        user_id: &str,
        server_name: &McpServerName,
        session_id: Option<String>,
        thread_id: Option<Uuid>,
    ) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&Self::key(user_id, server_name, thread_id)) {
            session.update_session_id(session_id);
        }
    }

    /// Mark a session as initialized.
    pub async fn mark_initialized(
        &self,
        user_id: &str,
        server_name: &McpServerName,
        thread_id: Option<Uuid>,
    ) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&Self::key(user_id, server_name, thread_id)) {
            session.mark_initialized();
        }
    }

    /// Check if a session is initialized.
    pub async fn is_initialized(
        &self,
        user_id: &str,
        server_name: &McpServerName,
        thread_id: Option<Uuid>,
    ) -> bool {
        let sessions = self.sessions.read().await;
        sessions
            .get(&Self::key(user_id, server_name, thread_id))
            .map(|s| s.initialized)
            .unwrap_or(false)
    }

    /// Touch a session to update its activity timestamp.
    pub async fn touch(&self, user_id: &str, server_name: &McpServerName, thread_id: Option<Uuid>) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&Self::key(user_id, server_name, thread_id)) {
            session.touch();
        }
    }

    /// Terminate a session (e.g., on error or explicit disconnect).
    pub async fn terminate(
        &self,
        user_id: &str,
        server_name: &McpServerName,
        thread_id: Option<Uuid>,
    ) {
        let mut sessions = self.sessions.write().await;
        sessions.remove(&Self::key(user_id, server_name, thread_id));
    }

    /// Snapshot the active `(user, server, thread_id)` triples.
    pub async fn active_sessions(&self) -> Vec<(String, McpServerName, Option<Uuid>)> {
        let sessions = self.sessions.read().await;
        sessions
            .keys()
            .map(|k| (k.user_id.clone(), k.server_name.clone(), k.thread_id))
            .collect()
    }

    /// Clean up stale sessions.
    pub async fn cleanup_stale(&self) -> usize {
        let mut sessions = self.sessions.write().await;
        let before_len = sessions.len();
        sessions.retain(|_, session| !session.is_stale(self.max_idle_secs));
        before_len - sessions.len()
    }
}

impl Default for McpSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_A: &str = "user-a";
    const USER_B: &str = "user-b";

    fn sn(s: &str) -> McpServerName {
        McpServerName::new(s).expect("test name")
    }

    fn tid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn test_session_creation() {
        let session = McpSession::new("https://mcp.example.com");
        assert!(session.session_id.is_none());
        assert!(!session.initialized);
        assert_eq!(session.server_url, "https://mcp.example.com");
    }

    #[test]
    fn test_session_update() {
        let mut session = McpSession::new("https://mcp.example.com");

        session.update_session_id(Some("session-123".to_string()));
        assert_eq!(session.session_id, Some("session-123".to_string()));

        session.mark_initialized();
        assert!(session.initialized);
    }

    #[test]
    fn test_session_staleness() {
        let mut session = McpSession::new("https://mcp.example.com");

        // Fresh session should not be stale with reasonable timeout
        assert!(!session.is_stale(1800));

        // Manually set last_activity to the past to simulate staleness
        session.last_activity = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(10))
            .expect("System uptime is too low to run staleness test");
        assert!(session.is_stale(5));
        assert!(!session.is_stale(15));
    }

    #[tokio::test]
    async fn test_session_manager_get_or_create() {
        let manager = McpSessionManager::new();
        let notion = sn("notion");

        // First call creates a new session
        let session1 = manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", None)
            .await;
        assert!(session1.session_id.is_none());

        // Update the session ID
        manager
            .update_session_id(USER_A, &notion, Some("session-abc".to_string()), None)
            .await;

        // Second call returns existing session with the ID
        let session2 = manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", None)
            .await;
        assert_eq!(session2.session_id, Some("session-abc".to_string()));
    }

    #[tokio::test]
    async fn test_session_manager_terminate() {
        let manager = McpSessionManager::new();
        let notion = sn("notion");

        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", None)
            .await;
        manager
            .update_session_id(USER_A, &notion, Some("session-123".to_string()), None)
            .await;

        // Terminate the session
        manager.terminate(USER_A, &notion, None).await;

        // Should create a fresh session now
        let session = manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", None)
            .await;
        assert!(session.session_id.is_none());
    }

    #[tokio::test]
    async fn test_session_manager_initialization() {
        let manager = McpSessionManager::new();
        let notion = sn("notion");

        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", None)
            .await;

        assert!(!manager.is_initialized(USER_A, &notion, None).await);

        manager.mark_initialized(USER_A, &notion, None).await;

        assert!(manager.is_initialized(USER_A, &notion, None).await);
    }

    #[tokio::test]
    async fn test_active_sessions_tracks_user_server_pairs() {
        let manager = McpSessionManager::new();
        let notion = sn("notion");
        let github = sn("github");

        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", None)
            .await;
        manager
            .get_or_create(USER_A, &github, "https://mcp.github.com", None)
            .await;
        manager
            .get_or_create(USER_B, &notion, "https://mcp.notion.com", None)
            .await;

        let pairs = manager.active_sessions().await;
        assert_eq!(pairs.len(), 3);
        assert!(pairs.contains(&(USER_A.to_string(), notion.clone(), None)));
        assert!(pairs.contains(&(USER_A.to_string(), github.clone(), None)));
        assert!(pairs.contains(&(USER_B.to_string(), notion.clone(), None)));
    }

    /// Regression for the cross-tenant session-ID collision called out in
    /// review of the `McpClientStore` PR: two users activating the same
    /// server MUST hold distinct session IDs. If the map were keyed by
    /// server name alone, user-B's `update_session_id` would overwrite
    /// user-A's slot and user-A's next request would send user-B's
    /// `Mcp-Session-Id` — potential cross-tenant access to server-side
    /// session state.
    #[tokio::test]
    async fn test_session_id_is_partitioned_per_user() {
        let manager = McpSessionManager::new();
        let notion = sn("notion");

        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", None)
            .await;
        manager
            .get_or_create(USER_B, &notion, "https://mcp.notion.com", None)
            .await;

        manager
            .update_session_id(USER_A, &notion, Some("session-a".to_string()), None)
            .await;
        manager
            .update_session_id(USER_B, &notion, Some("session-b".to_string()), None)
            .await;

        assert_eq!(
            manager.get_session_id(USER_A, &notion, None).await,
            Some("session-a".to_string())
        );
        assert_eq!(
            manager.get_session_id(USER_B, &notion, None).await,
            Some("session-b".to_string())
        );

        manager.terminate(USER_A, &notion, None).await;
        assert!(
            manager
                .get_session_id(USER_A, &notion, None)
                .await
                .is_none()
        );
        assert_eq!(
            manager.get_session_id(USER_B, &notion, None).await,
            Some("session-b".to_string()),
            "terminating user-A must not affect user-B's session"
        );
    }

    #[test]
    fn test_update_session_id_none_leaves_id_unchanged() {
        let mut session = McpSession::new("https://mcp.example.com");
        session.session_id = Some("existing-id".to_string());

        session.update_session_id(None);

        assert_eq!(session.session_id, Some("existing-id".to_string()));
    }

    #[test]
    fn test_touch_updates_last_activity() {
        let mut session = McpSession::new("https://mcp.example.com");
        // Push last_activity into the past so we can observe the change.
        session.last_activity = std::time::Instant::now() - std::time::Duration::from_secs(60);
        let before = session.last_activity;

        session.touch();

        assert!(session.last_activity > before);
    }

    #[test]
    fn test_with_idle_timeout() {
        let manager = McpSessionManager::with_idle_timeout(42);
        assert_eq!(manager.max_idle_secs, 42);
    }

    #[tokio::test]
    async fn test_get_session_id_nonexistent_returns_none() {
        let manager = McpSessionManager::new();
        assert!(
            manager
                .get_session_id(USER_A, &sn("ghost"), None)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_update_session_id_nonexistent_is_noop() {
        let manager = McpSessionManager::new();
        // Should not panic or create a session.
        manager
            .update_session_id(USER_A, &sn("ghost"), Some("id".to_string()), None)
            .await;
        assert!(manager.active_sessions().await.is_empty());
    }

    #[tokio::test]
    async fn test_mark_initialized_nonexistent_is_noop() {
        let manager = McpSessionManager::new();
        manager.mark_initialized(USER_A, &sn("ghost"), None).await;
        assert!(manager.active_sessions().await.is_empty());
    }

    #[tokio::test]
    async fn test_touch_nonexistent_is_noop() {
        let manager = McpSessionManager::new();
        manager.touch(USER_A, &sn("ghost"), None).await;
        assert!(manager.active_sessions().await.is_empty());
    }

    #[tokio::test]
    async fn test_cleanup_stale_removes_only_stale() {
        // Use a 5-second idle timeout so we can fake staleness easily.
        let manager = McpSessionManager::with_idle_timeout(5);
        let fresh = sn("fresh");
        let stale1 = sn("stale1");
        let stale2 = sn("stale2");

        manager
            .get_or_create(USER_A, &fresh, "https://fresh.example.com", None)
            .await;
        manager
            .get_or_create(USER_A, &stale1, "https://stale1.example.com", None)
            .await;
        manager
            .get_or_create(USER_A, &stale2, "https://stale2.example.com", None)
            .await;

        // Push the two stale sessions into the past.
        {
            let mut sessions = manager.sessions.write().await;
            let past = std::time::Instant::now() - std::time::Duration::from_secs(60);
            sessions
                .get_mut(&McpSessionManager::key(USER_A, &stale1, None))
                .unwrap()
                .last_activity = past;
            sessions
                .get_mut(&McpSessionManager::key(USER_A, &stale2, None))
                .unwrap()
                .last_activity = past;
        }

        let removed = manager.cleanup_stale().await;
        assert_eq!(removed, 2);

        let remaining = manager.active_sessions().await;
        assert_eq!(remaining.len(), 1);
        assert!(remaining.contains(&(USER_A.to_string(), fresh.clone(), None)));
    }

    #[tokio::test]
    async fn test_terminate_nonexistent_is_noop() {
        let manager = McpSessionManager::new();
        // Should not panic.
        manager.terminate(USER_A, &sn("ghost"), None).await;
        assert!(manager.active_sessions().await.is_empty());
    }

    #[test]
    fn test_default_trait_impl() {
        let manager = McpSessionManager::default();
        // Default should match new(), which uses 1800s idle timeout.
        assert_eq!(manager.max_idle_secs, 1800);
    }

    // ── Thread-axis partition tests ───────────────────────────────────────

    /// Two `new_for_thread` keys with distinct `thread_id` must not collide:
    /// Thread A and Thread B under the same user and same server get
    /// independent MCP sessions.
    #[tokio::test]
    async fn test_distinct_thread_ids_do_not_collide() {
        let manager = McpSessionManager::new();
        let notion = sn("notion");
        let thread_a = tid(1);
        let thread_b = tid(2);

        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", Some(thread_a))
            .await;
        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", Some(thread_b))
            .await;

        manager
            .update_session_id(
                USER_A,
                &notion,
                Some("sess-thread-a".to_string()),
                Some(thread_a),
            )
            .await;
        manager
            .update_session_id(
                USER_A,
                &notion,
                Some("sess-thread-b".to_string()),
                Some(thread_b),
            )
            .await;

        assert_eq!(
            manager
                .get_session_id(USER_A, &notion, Some(thread_a))
                .await,
            Some("sess-thread-a".to_string()),
            "Thread A's session must not be overwritten by Thread B's update"
        );
        assert_eq!(
            manager
                .get_session_id(USER_A, &notion, Some(thread_b))
                .await,
            Some("sess-thread-b".to_string()),
            "Thread B's session must be independent from Thread A"
        );
    }

    /// `new` (None) and `new_for_thread` with the same `(user, server)` must
    /// not collide: the user-scoped slot and a thread-scoped slot coexist.
    #[tokio::test]
    async fn test_none_and_some_thread_id_are_distinct_slots() {
        let manager = McpSessionManager::new();
        let notion = sn("notion");
        let thread_a = tid(42);

        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", None)
            .await;
        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", Some(thread_a))
            .await;

        manager
            .update_session_id(USER_A, &notion, Some("sess-user-scoped".to_string()), None)
            .await;
        manager
            .update_session_id(
                USER_A,
                &notion,
                Some("sess-thread-scoped".to_string()),
                Some(thread_a),
            )
            .await;

        assert_eq!(
            manager.get_session_id(USER_A, &notion, None).await,
            Some("sess-user-scoped".to_string()),
            "user-scoped slot must be independent from the thread-scoped slot"
        );
        assert_eq!(
            manager
                .get_session_id(USER_A, &notion, Some(thread_a))
                .await,
            Some("sess-thread-scoped".to_string()),
            "thread-scoped slot must be independent from the user-scoped slot"
        );
        // Confirm two distinct entries exist
        assert_eq!(manager.active_sessions().await.len(), 2);
    }

    /// Same `(user, server, thread_id)` triple must collide — a round-trip
    /// verify that the thread axis is keyed correctly.
    #[tokio::test]
    async fn test_same_triple_collides() {
        let manager = McpSessionManager::new();
        let notion = sn("notion");
        let thread_a = tid(7);

        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", Some(thread_a))
            .await;
        manager
            .update_session_id(USER_A, &notion, Some("sess-v1".to_string()), Some(thread_a))
            .await;

        // Second get_or_create with the same triple must return the existing session.
        let sess = manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", Some(thread_a))
            .await;
        assert_eq!(
            sess.session_id,
            Some("sess-v1".to_string()),
            "identical (user, server, thread_id) must hit the same session slot"
        );
        assert_eq!(manager.active_sessions().await.len(), 1);
    }

    /// Terminating a thread-scoped session must not affect the user-scoped
    /// session for the same `(user, server)`.
    #[tokio::test]
    async fn test_thread_terminate_does_not_affect_user_scoped() {
        let manager = McpSessionManager::new();
        let notion = sn("notion");
        let thread_a = tid(99);

        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", None)
            .await;
        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com", Some(thread_a))
            .await;
        manager
            .update_session_id(USER_A, &notion, Some("sess-user".to_string()), None)
            .await;
        manager
            .update_session_id(
                USER_A,
                &notion,
                Some("sess-thread".to_string()),
                Some(thread_a),
            )
            .await;

        manager.terminate(USER_A, &notion, Some(thread_a)).await;

        assert!(
            manager
                .get_session_id(USER_A, &notion, Some(thread_a))
                .await
                .is_none(),
            "thread-scoped session must be removed"
        );
        assert_eq!(
            manager.get_session_id(USER_A, &notion, None).await,
            Some("sess-user".to_string()),
            "user-scoped session must survive thread termination"
        );
    }
}

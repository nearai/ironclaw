//! MCP session management.
//!
//! Manages Mcp-Session-Id headers for stateful connections to MCP servers.
//! Each `(user, server)` pair has its own session that persists across
//! requests.
//!
//! Sessions are partitioned by `(user_id, server_name)` — **not** by server
//! name alone. An MCP server issues a distinct `Mcp-Session-Id` for every
//! authenticated client. If two users activate the same MCP server and the
//! manager were keyed on server name only, the second user's session ID
//! would overwrite the first user's; the first user's next request would
//! then send the second user's `Mcp-Session-Id`, potentially accessing
//! cross-tenant server-side state. Same shape as the MCP client-isolation
//! bug in `McpClientStore` — see `.claude/rules/safety-and-sandbox.md`
//! "Cache Keys Must Be Complete".

use std::collections::HashMap;
use std::time::Instant;

use ironclaw_common::McpServerName;
use tokio::sync::RwLock;

/// Composite key for an MCP session. A given user holds one session per
/// server; the same user across two different servers gets two distinct
/// sessions; the same server across two different users also gets two
/// distinct sessions.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct McpSessionKey {
    user_id: String,
    server_name: McpServerName,
}

impl McpSessionKey {
    pub fn new(user_id: impl Into<String>, server_name: McpServerName) -> Self {
        Self {
            user_id: user_id.into(),
            server_name,
        }
    }
}

/// Session state for a single `(user, server)` MCP connection.
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

/// Manages MCP sessions across multiple `(user, server)` pairs.
///
/// Server names are typed via [`McpServerName`] so a free-form string can't
/// bypass allowlist validation at the boundary. Callers convert raw strings
/// via `McpServerName::new` (validating) or `McpServerName::from_trusted`
/// (for names the caller already validated). This makes identity-confusion
/// bugs — matching the shape described in `.claude/rules/types.md` — a
/// compile error rather than a runtime surprise.
pub struct McpSessionManager {
    /// Active sessions keyed by `(user_id, server_name)`.
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

    fn key(user_id: &str, server_name: &McpServerName) -> McpSessionKey {
        McpSessionKey::new(user_id, server_name.clone())
    }

    /// Get or create a session for `(user, server)`.
    pub async fn get_or_create(
        &self,
        user_id: &str,
        server_name: &McpServerName,
        server_url: &str,
    ) -> McpSession {
        let key = Self::key(user_id, server_name);
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

    /// Get the current session ID for `(user, server)`, if any.
    pub async fn get_session_id(
        &self,
        user_id: &str,
        server_name: &McpServerName,
    ) -> Option<String> {
        let sessions = self.sessions.read().await;
        sessions
            .get(&Self::key(user_id, server_name))
            .and_then(|s| s.session_id.clone())
    }

    /// Update the session ID from a server response.
    pub async fn update_session_id(
        &self,
        user_id: &str,
        server_name: &McpServerName,
        session_id: Option<String>,
    ) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&Self::key(user_id, server_name)) {
            session.update_session_id(session_id);
        }
    }

    /// Mark a session as initialized.
    pub async fn mark_initialized(&self, user_id: &str, server_name: &McpServerName) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&Self::key(user_id, server_name)) {
            session.mark_initialized();
        }
    }

    /// Check if a session is initialized.
    pub async fn is_initialized(&self, user_id: &str, server_name: &McpServerName) -> bool {
        let sessions = self.sessions.read().await;
        sessions
            .get(&Self::key(user_id, server_name))
            .map(|s| s.initialized)
            .unwrap_or(false)
    }

    /// Touch a session to update its activity timestamp.
    pub async fn touch(&self, user_id: &str, server_name: &McpServerName) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&Self::key(user_id, server_name)) {
            session.touch();
        }
    }

    /// Terminate a session (e.g., on error or explicit disconnect).
    pub async fn terminate(&self, user_id: &str, server_name: &McpServerName) {
        let mut sessions = self.sessions.write().await;
        sessions.remove(&Self::key(user_id, server_name));
    }

    /// Snapshot the active `(user, server)` pairs.
    pub async fn active_sessions(&self) -> Vec<(String, McpServerName)> {
        let sessions = self.sessions.read().await;
        sessions
            .keys()
            .map(|k| (k.user_id.clone(), k.server_name.clone()))
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
            .get_or_create(USER_A, &notion, "https://mcp.notion.com")
            .await;
        assert!(session1.session_id.is_none());

        // Update the session ID
        manager
            .update_session_id(USER_A, &notion, Some("session-abc".to_string()))
            .await;

        // Second call returns existing session with the ID
        let session2 = manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com")
            .await;
        assert_eq!(session2.session_id, Some("session-abc".to_string()));
    }

    #[tokio::test]
    async fn test_session_manager_terminate() {
        let manager = McpSessionManager::new();
        let notion = sn("notion");

        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com")
            .await;
        manager
            .update_session_id(USER_A, &notion, Some("session-123".to_string()))
            .await;

        // Terminate the session
        manager.terminate(USER_A, &notion).await;

        // Should create a fresh session now
        let session = manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com")
            .await;
        assert!(session.session_id.is_none());
    }

    #[tokio::test]
    async fn test_session_manager_initialization() {
        let manager = McpSessionManager::new();
        let notion = sn("notion");

        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com")
            .await;

        assert!(!manager.is_initialized(USER_A, &notion).await);

        manager.mark_initialized(USER_A, &notion).await;

        assert!(manager.is_initialized(USER_A, &notion).await);
    }

    #[tokio::test]
    async fn test_active_sessions_tracks_user_server_pairs() {
        let manager = McpSessionManager::new();
        let notion = sn("notion");
        let github = sn("github");

        manager
            .get_or_create(USER_A, &notion, "https://mcp.notion.com")
            .await;
        manager
            .get_or_create(USER_A, &github, "https://mcp.github.com")
            .await;
        manager
            .get_or_create(USER_B, &notion, "https://mcp.notion.com")
            .await;

        let pairs = manager.active_sessions().await;
        assert_eq!(pairs.len(), 3);
        assert!(pairs.contains(&(USER_A.to_string(), notion.clone())));
        assert!(pairs.contains(&(USER_A.to_string(), github.clone())));
        assert!(pairs.contains(&(USER_B.to_string(), notion.clone())));
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
            .get_or_create(USER_A, &notion, "https://mcp.notion.com")
            .await;
        manager
            .get_or_create(USER_B, &notion, "https://mcp.notion.com")
            .await;

        manager
            .update_session_id(USER_A, &notion, Some("session-a".to_string()))
            .await;
        manager
            .update_session_id(USER_B, &notion, Some("session-b".to_string()))
            .await;

        assert_eq!(
            manager.get_session_id(USER_A, &notion).await,
            Some("session-a".to_string())
        );
        assert_eq!(
            manager.get_session_id(USER_B, &notion).await,
            Some("session-b".to_string())
        );

        manager.terminate(USER_A, &notion).await;
        assert!(manager.get_session_id(USER_A, &notion).await.is_none());
        assert_eq!(
            manager.get_session_id(USER_B, &notion).await,
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
        assert!(manager.get_session_id(USER_A, &sn("ghost")).await.is_none());
    }

    #[tokio::test]
    async fn test_update_session_id_nonexistent_is_noop() {
        let manager = McpSessionManager::new();
        // Should not panic or create a session.
        manager
            .update_session_id(USER_A, &sn("ghost"), Some("id".to_string()))
            .await;
        assert!(manager.active_sessions().await.is_empty());
    }

    #[tokio::test]
    async fn test_mark_initialized_nonexistent_is_noop() {
        let manager = McpSessionManager::new();
        manager.mark_initialized(USER_A, &sn("ghost")).await;
        assert!(manager.active_sessions().await.is_empty());
    }

    #[tokio::test]
    async fn test_touch_nonexistent_is_noop() {
        let manager = McpSessionManager::new();
        manager.touch(USER_A, &sn("ghost")).await;
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
            .get_or_create(USER_A, &fresh, "https://fresh.example.com")
            .await;
        manager
            .get_or_create(USER_A, &stale1, "https://stale1.example.com")
            .await;
        manager
            .get_or_create(USER_A, &stale2, "https://stale2.example.com")
            .await;

        // Push the two stale sessions into the past.
        {
            let mut sessions = manager.sessions.write().await;
            let past = std::time::Instant::now() - std::time::Duration::from_secs(60);
            sessions
                .get_mut(&McpSessionManager::key(USER_A, &stale1))
                .unwrap()
                .last_activity = past;
            sessions
                .get_mut(&McpSessionManager::key(USER_A, &stale2))
                .unwrap()
                .last_activity = past;
        }

        let removed = manager.cleanup_stale().await;
        assert_eq!(removed, 2);

        let remaining = manager.active_sessions().await;
        assert_eq!(remaining.len(), 1);
        assert!(remaining.contains(&(USER_A.to_string(), fresh.clone())));
    }

    #[tokio::test]
    async fn test_terminate_nonexistent_is_noop() {
        let manager = McpSessionManager::new();
        // Should not panic.
        manager.terminate(USER_A, &sn("ghost")).await;
        assert!(manager.active_sessions().await.is_empty());
    }

    #[test]
    fn test_default_trait_impl() {
        let manager = McpSessionManager::default();
        // Default should match new(), which uses 1800s idle timeout.
        assert_eq!(manager.max_idle_secs, 1800);
    }
}

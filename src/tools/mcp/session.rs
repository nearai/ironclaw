//! MCP session management.
//!
//! Manages Mcp-Session-Id headers for stateful connections to MCP servers.
//! Each server can have an active session that persists across requests.

use std::collections::HashMap;
use std::time::Instant;

use tokio::sync::RwLock;

const DEFAULT_MAX_SESSIONS: usize = 1024;

/// Session state for a single MCP server connection.
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

/// Manages MCP sessions for multiple servers.
pub struct McpSessionManager {
    /// Active sessions keyed by (user, server).
    sessions: RwLock<HashMap<McpSessionKey, McpSession>>,

    /// Maximum idle time before a session is considered stale (in seconds).
    max_idle_secs: u64,

    /// Hard cap on active sessions to bound `(user_id, server_name)` growth.
    max_sessions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct McpSessionKey {
    user_id: String,
    server_name: String,
}

impl McpSessionKey {
    fn new(user_id: &str, server_name: &str) -> Self {
        Self {
            user_id: user_id.to_string(),
            server_name: server_name.to_string(),
        }
    }
}

impl McpSessionManager {
    /// Create a new session manager with default idle timeout (30 minutes).
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            max_idle_secs: 1800, // 30 minutes
            max_sessions: DEFAULT_MAX_SESSIONS,
        }
    }

    /// Create a new session manager with custom idle timeout.
    pub fn with_idle_timeout(max_idle_secs: u64) -> Self {
        Self::with_limits(max_idle_secs, DEFAULT_MAX_SESSIONS)
    }

    /// Create a new session manager with custom idle timeout and capacity.
    pub fn with_limits(max_idle_secs: u64, max_sessions: usize) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            max_idle_secs,
            max_sessions: max_sessions.max(1),
        }
    }

    /// Get or create a session for a server.
    pub async fn get_or_create(
        &self,
        user_id: &str,
        server_name: &str,
        server_url: &str,
    ) -> McpSession {
        let mut sessions = self.sessions.write().await;
        let key = McpSessionKey::new(user_id, server_name);

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

        Self::cleanup_stale_locked(&mut sessions, self.max_idle_secs);
        if sessions.len() >= self.max_sessions
            && let Some(oldest_key) = sessions
                .iter()
                .min_by_key(|(_, session)| session.last_activity)
                .map(|(key, _)| key.clone())
        {
            sessions.remove(&oldest_key);
        }

        // Create new session
        let session = McpSession::new(server_url);
        sessions.insert(key, session.clone());
        session
    }

    /// Get the current session ID for a server (if any).
    pub async fn get_session_id(&self, user_id: &str, server_name: &str) -> Option<String> {
        let sessions = self.sessions.read().await;
        sessions
            .get(&McpSessionKey::new(user_id, server_name))
            .and_then(|s| s.session_id.clone())
    }

    /// Update the session ID from a server response.
    pub async fn update_session_id(
        &self,
        user_id: &str,
        server_name: &str,
        session_id: Option<String>,
    ) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&McpSessionKey::new(user_id, server_name)) {
            session.update_session_id(session_id);
        }
    }

    /// Mark a session as initialized.
    pub async fn mark_initialized(&self, user_id: &str, server_name: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&McpSessionKey::new(user_id, server_name)) {
            session.mark_initialized();
        }
    }

    /// Check if a session is initialized.
    pub async fn is_initialized(&self, user_id: &str, server_name: &str) -> bool {
        let sessions = self.sessions.read().await;
        sessions
            .get(&McpSessionKey::new(user_id, server_name))
            .map(|s| s.initialized)
            .unwrap_or(false)
    }

    /// Touch a session to update its activity timestamp.
    pub async fn touch(&self, user_id: &str, server_name: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(&McpSessionKey::new(user_id, server_name)) {
            session.touch();
        }
    }

    /// Terminate a session (e.g., on error or explicit disconnect).
    pub async fn terminate(&self, user_id: &str, server_name: &str) {
        let mut sessions = self.sessions.write().await;
        sessions.remove(&McpSessionKey::new(user_id, server_name));
    }

    /// Get all active server names for a single user.
    pub async fn active_servers(&self, user_id: &str) -> Vec<String> {
        let sessions = self.sessions.read().await;
        sessions
            .keys()
            .filter(|key| key.user_id == user_id)
            .map(|key| key.server_name.clone())
            .collect()
    }

    /// Clean up stale sessions.
    pub async fn cleanup_stale(&self) -> usize {
        let mut sessions = self.sessions.write().await;
        Self::cleanup_stale_locked(&mut sessions, self.max_idle_secs)
    }

    fn cleanup_stale_locked(
        sessions: &mut HashMap<McpSessionKey, McpSession>,
        max_idle_secs: u64,
    ) -> usize {
        let before_len = sessions.len();
        sessions.retain(|_, session| !session.is_stale(max_idle_secs));
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

        // First call creates a new session
        let session1 = manager
            .get_or_create("user-a", "notion", "https://mcp.notion.com")
            .await;
        assert!(session1.session_id.is_none());

        // Update the session ID
        manager
            .update_session_id("user-a", "notion", Some("session-abc".to_string()))
            .await;

        // Second call returns existing session with the ID
        let session2 = manager
            .get_or_create("user-a", "notion", "https://mcp.notion.com")
            .await;
        assert_eq!(session2.session_id, Some("session-abc".to_string()));
    }

    #[tokio::test]
    async fn test_session_manager_terminate() {
        let manager = McpSessionManager::new();

        manager
            .get_or_create("user-a", "notion", "https://mcp.notion.com")
            .await;
        manager
            .update_session_id("user-a", "notion", Some("session-123".to_string()))
            .await;

        // Terminate the session
        manager.terminate("user-a", "notion").await;

        // Should create a fresh session now
        let session = manager
            .get_or_create("user-a", "notion", "https://mcp.notion.com")
            .await;
        assert!(session.session_id.is_none());
    }

    #[tokio::test]
    async fn test_session_manager_initialization() {
        let manager = McpSessionManager::new();

        manager
            .get_or_create("user-a", "notion", "https://mcp.notion.com")
            .await;

        assert!(!manager.is_initialized("user-a", "notion").await);

        manager.mark_initialized("user-a", "notion").await;

        assert!(manager.is_initialized("user-a", "notion").await);
    }

    #[tokio::test]
    async fn test_active_servers() {
        let manager = McpSessionManager::new();

        manager
            .get_or_create("user-a", "notion", "https://mcp.notion.com")
            .await;
        manager
            .get_or_create("user-a", "github", "https://mcp.github.com")
            .await;

        let servers = manager.active_servers("user-a").await;
        assert_eq!(servers.len(), 2);
        assert!(servers.contains(&"notion".to_string()));
        assert!(servers.contains(&"github".to_string()));
    }

    #[tokio::test]
    async fn test_sessions_are_partitioned_by_user() {
        let manager = McpSessionManager::new();

        manager
            .get_or_create("user-a", "notion", "https://mcp.notion.com")
            .await;
        manager
            .update_session_id("user-a", "notion", Some("session-a".to_string()))
            .await;

        manager
            .get_or_create("user-b", "notion", "https://mcp.notion.com")
            .await;
        manager
            .update_session_id("user-b", "notion", Some("session-b".to_string()))
            .await;

        assert_eq!(
            manager.get_session_id("user-a", "notion").await.as_deref(),
            Some("session-a")
        );
        assert_eq!(
            manager.get_session_id("user-b", "notion").await.as_deref(),
            Some("session-b")
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
        assert!(manager.get_session_id("user-a", "ghost").await.is_none());
    }

    #[tokio::test]
    async fn test_update_session_id_nonexistent_is_noop() {
        let manager = McpSessionManager::new();
        // Should not panic or create a session.
        manager
            .update_session_id("user-a", "ghost", Some("id".to_string()))
            .await;
        assert!(manager.active_servers("user-a").await.is_empty());
    }

    #[tokio::test]
    async fn test_mark_initialized_nonexistent_is_noop() {
        let manager = McpSessionManager::new();
        manager.mark_initialized("user-a", "ghost").await;
        assert!(manager.active_servers("user-a").await.is_empty());
    }

    #[tokio::test]
    async fn test_touch_nonexistent_is_noop() {
        let manager = McpSessionManager::new();
        manager.touch("user-a", "ghost").await;
        assert!(manager.active_servers("user-a").await.is_empty());
    }

    #[tokio::test]
    async fn test_cleanup_stale_removes_only_stale() {
        // Use a 5-second idle timeout so we can fake staleness easily.
        let manager = McpSessionManager::with_idle_timeout(5);

        manager
            .get_or_create("user-a", "fresh", "https://fresh.example.com")
            .await;
        manager
            .get_or_create("user-a", "stale1", "https://stale1.example.com")
            .await;
        manager
            .get_or_create("user-a", "stale2", "https://stale2.example.com")
            .await;

        // Push the two stale sessions into the past.
        {
            let mut sessions = manager.sessions.write().await;
            let past = std::time::Instant::now() - std::time::Duration::from_secs(60);
            sessions
                .get_mut(&McpSessionKey::new("user-a", "stale1"))
                .unwrap()
                .last_activity = past;
            sessions
                .get_mut(&McpSessionKey::new("user-a", "stale2"))
                .unwrap()
                .last_activity = past;
        }

        let removed = manager.cleanup_stale().await;
        assert_eq!(removed, 2);

        let remaining = manager.active_servers("user-a").await;
        assert_eq!(remaining.len(), 1);
        assert!(remaining.contains(&"fresh".to_string()));
    }

    #[tokio::test]
    async fn test_session_manager_evicts_oldest_when_capacity_is_reached() {
        let manager = McpSessionManager::with_limits(300, 2);

        manager
            .get_or_create("user-a", "oldest", "https://oldest.example.com")
            .await;
        {
            let mut sessions = manager.sessions.write().await;
            sessions
                .get_mut(&McpSessionKey::new("user-a", "oldest"))
                .expect("oldest session")
                .last_activity = std::time::Instant::now() - std::time::Duration::from_secs(10);
        }
        manager
            .get_or_create("user-b", "newer", "https://newer.example.com")
            .await;
        manager
            .get_or_create("user-c", "newest", "https://newest.example.com")
            .await;

        assert!(
            manager.get_session_id("user-a", "oldest").await.is_none(),
            "oldest session should be evicted when capacity is reached"
        );
        let sessions = manager.sessions.read().await;
        assert_eq!(sessions.len(), 2);
        assert!(sessions.contains_key(&McpSessionKey::new("user-b", "newer")));
        assert!(sessions.contains_key(&McpSessionKey::new("user-c", "newest")));
    }

    #[tokio::test]
    async fn test_terminate_nonexistent_is_noop() {
        let manager = McpSessionManager::new();
        // Should not panic.
        manager.terminate("user-a", "ghost").await;
        assert!(manager.active_servers("user-a").await.is_empty());
    }

    #[test]
    fn test_default_trait_impl() {
        let manager = McpSessionManager::default();
        // Default should match new(), which uses 1800s idle timeout.
        assert_eq!(manager.max_idle_secs, 1800);
    }
}

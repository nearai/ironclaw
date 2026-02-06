//! MCP session management.
//!
//! Manages Mcp-Session-Id headers for stateful connections to MCP servers.
//! Each server can have an active session that persists across requests.

use std::collections::HashMap;
use std::time::Instant;

use tokio::sync::RwLock;

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
    /// Active sessions by server name.
    sessions: RwLock<HashMap<String, McpSession>>,

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

    /// Get or create a session for a server.
    pub async fn get_or_create(&self, server_name: &str, server_url: &str) -> McpSession {
        let mut sessions = self.sessions.write().await;

        if let Some(session) = sessions.get(server_name) {
            // Check if session is stale
            if session.is_stale(self.max_idle_secs) {
                // Create a fresh session
                let new_session = McpSession::new(server_url);
                sessions.insert(server_name.to_string(), new_session.clone());
                return new_session;
            }
            return session.clone();
        }

        // Create new session
        let session = McpSession::new(server_url);
        sessions.insert(server_name.to_string(), session.clone());
        session
    }

    /// Get the current session ID for a server (if any).
    pub async fn get_session_id(&self, server_name: &str) -> Option<String> {
        let sessions = self.sessions.read().await;
        sessions.get(server_name).and_then(|s| s.session_id.clone())
    }

    /// Update the session ID from a server response.
    pub async fn update_session_id(&self, server_name: &str, session_id: Option<String>) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(server_name) {
            session.update_session_id(session_id);
        }
    }

    /// Mark a session as initialized.
    pub async fn mark_initialized(&self, server_name: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(server_name) {
            session.mark_initialized();
        }
    }

    /// Check if a session is initialized.
    pub async fn is_initialized(&self, server_name: &str) -> bool {
        let sessions = self.sessions.read().await;
        sessions
            .get(server_name)
            .map(|s| s.initialized)
            .unwrap_or(false)
    }

    /// Touch a session to update its activity timestamp.
    pub async fn touch(&self, server_name: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(server_name) {
            session.touch();
        }
    }

    /// Terminate a session (e.g., on error or explicit disconnect).
    pub async fn terminate(&self, server_name: &str) {
        let mut sessions = self.sessions.write().await;
        sessions.remove(server_name);
    }

    /// Get all active server names.
    pub async fn active_servers(&self) -> Vec<String> {
        let sessions = self.sessions.read().await;
        sessions.keys().cloned().collect()
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
        session.last_activity = std::time::Instant::now() - std::time::Duration::from_secs(10);
        assert!(session.is_stale(5));
        assert!(!session.is_stale(15));
    }

    #[tokio::test]
    async fn test_session_manager_get_or_create() {
        let manager = McpSessionManager::new();

        // First call creates a new session
        let session1 = manager
            .get_or_create("notion", "https://mcp.notion.com")
            .await;
        assert!(session1.session_id.is_none());

        // Update the session ID
        manager
            .update_session_id("notion", Some("session-abc".to_string()))
            .await;

        // Second call returns existing session with the ID
        let session2 = manager
            .get_or_create("notion", "https://mcp.notion.com")
            .await;
        assert_eq!(session2.session_id, Some("session-abc".to_string()));
    }

    #[tokio::test]
    async fn test_session_manager_terminate() {
        let manager = McpSessionManager::new();

        manager
            .get_or_create("notion", "https://mcp.notion.com")
            .await;
        manager
            .update_session_id("notion", Some("session-123".to_string()))
            .await;

        // Terminate the session
        manager.terminate("notion").await;

        // Should create a fresh session now
        let session = manager
            .get_or_create("notion", "https://mcp.notion.com")
            .await;
        assert!(session.session_id.is_none());
    }

    #[tokio::test]
    async fn test_session_manager_initialization() {
        let manager = McpSessionManager::new();

        manager
            .get_or_create("notion", "https://mcp.notion.com")
            .await;

        assert!(!manager.is_initialized("notion").await);

        manager.mark_initialized("notion").await;

        assert!(manager.is_initialized("notion").await);
    }

    #[tokio::test]
    async fn test_active_servers() {
        let manager = McpSessionManager::new();

        manager
            .get_or_create("notion", "https://mcp.notion.com")
            .await;
        manager
            .get_or_create("github", "https://mcp.github.com")
            .await;

        let servers = manager.active_servers().await;
        assert_eq!(servers.len(), 2);
        assert!(servers.contains(&"notion".to_string()));
        assert!(servers.contains(&"github".to_string()));
    }
}

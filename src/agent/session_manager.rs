//! Session manager for multi-user, multi-thread conversation handling.
//!
//! Maps external channel thread IDs to internal UUIDs and manages undo state
//! for each thread.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::agent::session::Session;
use crate::agent::undo::UndoManager;

/// Key for mapping external thread IDs to internal ones.
#[derive(Clone, Hash, Eq, PartialEq)]
struct ThreadKey {
    user_id: String,
    channel: String,
    external_thread_id: Option<String>,
}

/// Manages sessions, threads, and undo state for all users.
pub struct SessionManager {
    sessions: RwLock<HashMap<String, Arc<Mutex<Session>>>>,
    thread_map: RwLock<HashMap<ThreadKey, Uuid>>,
    undo_managers: RwLock<HashMap<Uuid, Arc<Mutex<UndoManager>>>>,
}

impl SessionManager {
    /// Create a new session manager.
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            thread_map: RwLock::new(HashMap::new()),
            undo_managers: RwLock::new(HashMap::new()),
        }
    }

    /// Get or create a session for a user.
    pub async fn get_or_create_session(&self, user_id: &str) -> Arc<Mutex<Session>> {
        // Fast path: check if session exists
        {
            let sessions = self.sessions.read().await;
            if let Some(session) = sessions.get(user_id) {
                return Arc::clone(session);
            }
        }

        // Slow path: create new session
        let mut sessions = self.sessions.write().await;
        // Double-check after acquiring write lock
        if let Some(session) = sessions.get(user_id) {
            return Arc::clone(session);
        }

        let session = Arc::new(Mutex::new(Session::new(user_id)));
        sessions.insert(user_id.to_string(), Arc::clone(&session));
        session
    }

    /// Resolve an external thread ID to an internal thread.
    ///
    /// Returns the session and thread ID. Creates both if they don't exist.
    pub async fn resolve_thread(
        &self,
        user_id: &str,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        let session = self.get_or_create_session(user_id).await;

        let key = ThreadKey {
            user_id: user_id.to_string(),
            channel: channel.to_string(),
            external_thread_id: external_thread_id.map(String::from),
        };

        // Check if we have a mapping
        {
            let thread_map = self.thread_map.read().await;
            if let Some(&thread_id) = thread_map.get(&key) {
                // Verify thread still exists in session
                let sess = session.lock().await;
                if sess.threads.contains_key(&thread_id) {
                    return (Arc::clone(&session), thread_id);
                }
            }
        }

        // Create new thread (always create a new one for a new key)
        let thread_id = {
            let mut sess = session.lock().await;
            let thread = sess.create_thread();
            thread.id
        };

        // Store mapping
        {
            let mut thread_map = self.thread_map.write().await;
            thread_map.insert(key, thread_id);
        }

        // Create undo manager for thread
        {
            let mut undo_managers = self.undo_managers.write().await;
            undo_managers.insert(thread_id, Arc::new(Mutex::new(UndoManager::new())));
        }

        (session, thread_id)
    }

    /// Register a hydrated thread so subsequent `resolve_thread` calls find it.
    ///
    /// Inserts into the thread_map and creates an undo manager for the thread.
    pub async fn register_thread(
        &self,
        user_id: &str,
        channel: &str,
        thread_id: Uuid,
        session: Arc<Mutex<Session>>,
    ) {
        let key = ThreadKey {
            user_id: user_id.to_string(),
            channel: channel.to_string(),
            external_thread_id: Some(thread_id.to_string()),
        };

        {
            let mut thread_map = self.thread_map.write().await;
            thread_map.insert(key, thread_id);
        }

        {
            let mut undo_managers = self.undo_managers.write().await;
            undo_managers
                .entry(thread_id)
                .or_insert_with(|| Arc::new(Mutex::new(UndoManager::new())));
        }

        // Ensure the session is tracked
        {
            let mut sessions = self.sessions.write().await;
            sessions.entry(user_id.to_string()).or_insert(session);
        }
    }

    /// Get undo manager for a thread.
    pub async fn get_undo_manager(&self, thread_id: Uuid) -> Arc<Mutex<UndoManager>> {
        // Fast path
        {
            let managers = self.undo_managers.read().await;
            if let Some(mgr) = managers.get(&thread_id) {
                return Arc::clone(mgr);
            }
        }

        // Create if missing
        let mut managers = self.undo_managers.write().await;
        // Double-check
        if let Some(mgr) = managers.get(&thread_id) {
            return Arc::clone(mgr);
        }

        let mgr = Arc::new(Mutex::new(UndoManager::new()));
        managers.insert(thread_id, Arc::clone(&mgr));
        mgr
    }

    /// Remove sessions that have been idle for longer than the given duration.
    ///
    /// Returns the number of sessions pruned.
    pub async fn prune_stale_sessions(&self, max_idle: std::time::Duration) -> usize {
        let cutoff = chrono::Utc::now() - chrono::TimeDelta::seconds(max_idle.as_secs() as i64);

        // Find stale session user_ids
        let stale_users: Vec<String> = {
            let sessions = self.sessions.read().await;
            sessions
                .iter()
                .filter_map(|(user_id, session)| {
                    // Try to lock; skip if contended (someone is actively using it)
                    let sess = session.try_lock().ok()?;
                    if sess.last_active_at < cutoff {
                        Some(user_id.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };

        if stale_users.is_empty() {
            return 0;
        }

        // Collect thread IDs from stale sessions for cleanup
        let mut stale_thread_ids: Vec<Uuid> = Vec::new();
        {
            let sessions = self.sessions.read().await;
            for user_id in &stale_users {
                if let Some(session) = sessions.get(user_id)
                    && let Ok(sess) = session.try_lock()
                {
                    stale_thread_ids.extend(sess.threads.keys());
                }
            }
        }

        // Remove sessions
        let count = {
            let mut sessions = self.sessions.write().await;
            let before = sessions.len();
            for user_id in &stale_users {
                sessions.remove(user_id);
            }
            before - sessions.len()
        };

        // Clean up thread mappings that point to stale sessions
        {
            let mut thread_map = self.thread_map.write().await;
            thread_map.retain(|key, _| !stale_users.contains(&key.user_id));
        }

        // Clean up undo managers for stale threads
        {
            let mut undo_managers = self.undo_managers.write().await;
            for thread_id in &stale_thread_ids {
                undo_managers.remove(thread_id);
            }
        }

        if count > 0 {
            tracing::info!(
                "Pruned {} stale session(s) (idle > {}s)",
                count,
                max_idle.as_secs()
            );
        }

        count
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_or_create_session() {
        let manager = SessionManager::new();

        let session1 = manager.get_or_create_session("user-1").await;
        let session2 = manager.get_or_create_session("user-1").await;

        // Same user should get same session
        assert!(Arc::ptr_eq(&session1, &session2));

        let session3 = manager.get_or_create_session("user-2").await;
        assert!(!Arc::ptr_eq(&session1, &session3));
    }

    #[tokio::test]
    async fn test_resolve_thread() {
        let manager = SessionManager::new();

        let (session1, thread1) = manager.resolve_thread("user-1", "cli", None).await;
        let (session2, thread2) = manager.resolve_thread("user-1", "cli", None).await;

        // Same channel+user should get same thread
        assert!(Arc::ptr_eq(&session1, &session2));
        assert_eq!(thread1, thread2);

        // Different channel should get different thread
        let (_, thread3) = manager.resolve_thread("user-1", "http", None).await;
        assert_ne!(thread1, thread3);
    }

    #[tokio::test]
    async fn test_undo_manager() {
        let manager = SessionManager::new();
        let (_, thread_id) = manager.resolve_thread("user-1", "cli", None).await;

        let undo1 = manager.get_undo_manager(thread_id).await;
        let undo2 = manager.get_undo_manager(thread_id).await;

        assert!(Arc::ptr_eq(&undo1, &undo2));
    }

    #[tokio::test]
    async fn test_prune_stale_sessions() {
        let manager = SessionManager::new();

        // Create two sessions and resolve threads (which updates last_active_at)
        let (_, _thread_id) = manager.resolve_thread("user-active", "cli", None).await;
        let (s2, _thread_id) = manager.resolve_thread("user-stale", "cli", None).await;

        // Backdate the stale session's last_active_at AFTER thread creation
        {
            let mut sess = s2.lock().await;
            sess.last_active_at = chrono::Utc::now() - chrono::TimeDelta::seconds(86400 * 10); // 10 days ago
        }

        // Prune with 7-day timeout
        let pruned = manager
            .prune_stale_sessions(std::time::Duration::from_secs(86400 * 7))
            .await;
        assert_eq!(pruned, 1);

        // Active session should still exist
        let sessions = manager.sessions.read().await;
        assert!(sessions.contains_key("user-active"));
        assert!(!sessions.contains_key("user-stale"));
    }

    #[tokio::test]
    async fn test_prune_no_stale_sessions() {
        let manager = SessionManager::new();
        let _s1 = manager.get_or_create_session("user-1").await;

        // Nothing should be pruned when timeout is long
        let pruned = manager
            .prune_stale_sessions(std::time::Duration::from_secs(86400 * 365))
            .await;
        assert_eq!(pruned, 0);
    }

    #[tokio::test]
    async fn test_register_thread() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let thread_id = Uuid::new_v4();

        // Create a session with a hydrated thread
        let session = Arc::new(Mutex::new(Session::new("user-hydrate")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(thread_id, sess.id);
            sess.threads.insert(thread_id, thread);
            sess.active_thread = Some(thread_id);
        }

        // Register the thread
        manager
            .register_thread("user-hydrate", "gateway", thread_id, Arc::clone(&session))
            .await;

        // resolve_thread should find it (using the UUID as external_thread_id)
        let (resolved_session, resolved_tid) = manager
            .resolve_thread("user-hydrate", "gateway", Some(&thread_id.to_string()))
            .await;
        assert_eq!(resolved_tid, thread_id);

        // Should be the same session object
        let sess = resolved_session.lock().await;
        assert!(sess.threads.contains_key(&thread_id));
    }

    #[tokio::test]
    async fn test_resolve_thread_with_explicit_external_id() {
        let manager = SessionManager::new();

        // Two calls with the same explicit external thread ID should resolve
        // to the same internal thread.
        let (_, t1) = manager
            .resolve_thread("user-1", "gateway", Some("ext-abc"))
            .await;
        let (_, t2) = manager
            .resolve_thread("user-1", "gateway", Some("ext-abc"))
            .await;
        assert_eq!(t1, t2);

        // A different external ID on the same channel/user gets a new thread.
        let (_, t3) = manager
            .resolve_thread("user-1", "gateway", Some("ext-xyz"))
            .await;
        assert_ne!(t1, t3);
    }

    #[tokio::test]
    async fn test_resolve_thread_none_vs_some_external_id() {
        let manager = SessionManager::new();

        // None external_thread_id is a distinct key from Some("ext-1").
        let (_, t_none) = manager.resolve_thread("user-1", "cli", None).await;
        let (_, t_some) = manager.resolve_thread("user-1", "cli", Some("ext-1")).await;
        assert_ne!(t_none, t_some);
    }

    #[tokio::test]
    async fn test_resolve_thread_different_users_isolated() {
        let manager = SessionManager::new();

        let (_, t1) = manager
            .resolve_thread("user-a", "gateway", Some("same-ext"))
            .await;
        let (_, t2) = manager
            .resolve_thread("user-b", "gateway", Some("same-ext"))
            .await;

        // Same channel + same external ID but different users = different threads
        assert_ne!(t1, t2);
    }

    #[tokio::test]
    async fn test_resolve_thread_different_channels_isolated() {
        let manager = SessionManager::new();

        let (_, t1) = manager
            .resolve_thread("user-1", "gateway", Some("thread-x"))
            .await;
        let (_, t2) = manager
            .resolve_thread("user-1", "telegram", Some("thread-x"))
            .await;

        // Same user + same external ID but different channels = different threads
        assert_ne!(t1, t2);
    }

    #[tokio::test]
    async fn test_resolve_thread_stale_mapping_creates_new_thread() {
        let manager = SessionManager::new();

        // Create a thread normally
        let (session, original_tid) = manager
            .resolve_thread("user-1", "gateway", Some("ext-1"))
            .await;

        // Simulate the thread being removed from the session (e.g. pruned)
        {
            let mut sess = session.lock().await;
            sess.threads.remove(&original_tid);
        }

        // Next resolve should detect the stale mapping and create a fresh thread
        let (_, new_tid) = manager
            .resolve_thread("user-1", "gateway", Some("ext-1"))
            .await;
        assert_ne!(original_tid, new_tid);

        // The new thread should actually exist in the session
        let sess = session.lock().await;
        assert!(sess.threads.contains_key(&new_tid));
    }

    #[tokio::test]
    async fn test_register_thread_preserves_uuid_on_resolve() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let known_uuid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("user-web")));
        let session_id = {
            let sess = session.lock().await;
            sess.id
        };

        // Simulate hydration: create thread with a known UUID
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(known_uuid, session_id);
            sess.threads.insert(known_uuid, thread);
        }

        // Register it
        manager
            .register_thread("user-web", "gateway", known_uuid, Arc::clone(&session))
            .await;

        // resolve_thread with UUID as external_thread_id MUST return the same UUID,
        // not mint a new one (this was the root cause of the "wrong conversation" bug)
        let (_, resolved) = manager
            .resolve_thread("user-web", "gateway", Some(&known_uuid.to_string()))
            .await;
        assert_eq!(resolved, known_uuid);
    }

    #[tokio::test]
    async fn test_register_thread_idempotent() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let tid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("user-idem")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(tid, sess.id);
            sess.threads.insert(tid, thread);
        }

        // Register twice
        manager
            .register_thread("user-idem", "gateway", tid, Arc::clone(&session))
            .await;
        manager
            .register_thread("user-idem", "gateway", tid, Arc::clone(&session))
            .await;

        // Should still resolve to the same thread
        let (_, resolved) = manager
            .resolve_thread("user-idem", "gateway", Some(&tid.to_string()))
            .await;
        assert_eq!(resolved, tid);
    }

    #[tokio::test]
    async fn test_register_thread_creates_undo_manager() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let tid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("user-undo")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(tid, sess.id);
            sess.threads.insert(tid, thread);
        }

        manager
            .register_thread("user-undo", "gateway", tid, Arc::clone(&session))
            .await;

        // Undo manager should exist for the registered thread
        let undo = manager.get_undo_manager(tid).await;
        let undo2 = manager.get_undo_manager(tid).await;
        assert!(Arc::ptr_eq(&undo, &undo2));
    }

    #[tokio::test]
    async fn test_register_thread_stores_session() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let tid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("user-new")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(tid, sess.id);
            sess.threads.insert(tid, thread);
        }

        // The user has no session yet in the manager
        {
            let sessions = manager.sessions.read().await;
            assert!(!sessions.contains_key("user-new"));
        }

        manager
            .register_thread("user-new", "gateway", tid, Arc::clone(&session))
            .await;

        // Now the session should be tracked
        {
            let sessions = manager.sessions.read().await;
            assert!(sessions.contains_key("user-new"));
        }
    }

    #[tokio::test]
    async fn test_multiple_threads_per_user() {
        let manager = SessionManager::new();

        let (_, t1) = manager
            .resolve_thread("user-1", "gateway", Some("thread-a"))
            .await;
        let (_, t2) = manager
            .resolve_thread("user-1", "gateway", Some("thread-b"))
            .await;
        let (session, t3) = manager
            .resolve_thread("user-1", "gateway", Some("thread-c"))
            .await;

        // All three should be distinct
        assert_ne!(t1, t2);
        assert_ne!(t2, t3);
        assert_ne!(t1, t3);

        // All three should exist in the same session
        let sess = session.lock().await;
        assert!(sess.threads.contains_key(&t1));
        assert!(sess.threads.contains_key(&t2));
        assert!(sess.threads.contains_key(&t3));
    }

    #[tokio::test]
    async fn test_prune_cleans_thread_map_and_undo_managers() {
        let manager = SessionManager::new();

        let (stale_session, stale_tid) = manager.resolve_thread("user-stale", "cli", None).await;

        // Backdate the session
        {
            let mut sess = stale_session.lock().await;
            sess.last_active_at = chrono::Utc::now() - chrono::TimeDelta::seconds(86400 * 30);
        }

        // Verify thread_map and undo_managers have entries
        {
            let tm = manager.thread_map.read().await;
            assert!(!tm.is_empty());
        }
        {
            let um = manager.undo_managers.read().await;
            assert!(um.contains_key(&stale_tid));
        }

        let pruned = manager
            .prune_stale_sessions(std::time::Duration::from_secs(86400 * 7))
            .await;
        assert_eq!(pruned, 1);

        // Thread map and undo managers should be cleaned up
        {
            let tm = manager.thread_map.read().await;
            assert!(tm.is_empty());
        }
        {
            let um = manager.undo_managers.read().await;
            assert!(!um.contains_key(&stale_tid));
        }
    }

    #[tokio::test]
    async fn test_resolve_thread_active_thread_set() {
        let manager = SessionManager::new();

        let (session, thread_id) = manager
            .resolve_thread("user-1", "gateway", Some("ext-1"))
            .await;

        // The resolved thread should be set as the active thread
        let sess = session.lock().await;
        assert_eq!(sess.active_thread, Some(thread_id));
    }

    #[tokio::test]
    async fn test_register_then_resolve_different_channel_creates_new() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let tid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("user-cross")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(tid, sess.id);
            sess.threads.insert(tid, thread);
        }

        // Register on "gateway" channel
        manager
            .register_thread("user-cross", "gateway", tid, Arc::clone(&session))
            .await;

        // Resolve on a different channel with the same UUID string should NOT
        // find the registered thread (channel is part of the key)
        let (_, resolved) = manager
            .resolve_thread("user-cross", "telegram", Some(&tid.to_string()))
            .await;
        assert_ne!(resolved, tid);
    }
}

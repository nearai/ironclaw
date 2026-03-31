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
use crate::hooks::HookRegistry;

/// Warn when session count exceeds this threshold.
const SESSION_COUNT_WARNING_THRESHOLD: usize = 1000;

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
    hooks: Option<Arc<HookRegistry>>,
}

impl SessionManager {
    /// Create a new session manager.
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            thread_map: RwLock::new(HashMap::new()),
            undo_managers: RwLock::new(HashMap::new()),
            hooks: None,
        }
    }

    /// Attach a hook registry for session lifecycle events.
    pub fn with_hooks(mut self, hooks: Arc<HookRegistry>) -> Self {
        self.hooks = Some(hooks);
        self
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

        let new_session = Session::new(user_id);
        let session_id = new_session.id.to_string();
        let session = Arc::new(Mutex::new(new_session));
        sessions.insert(user_id.to_string(), Arc::clone(&session));

        if sessions.len() >= SESSION_COUNT_WARNING_THRESHOLD && sessions.len() % 100 == 0 {
            tracing::warn!(
                "High session count: {} active sessions. \
                 Pruning runs every 10 minutes; consider reducing session_idle_timeout.",
                sessions.len()
            );
        }

        // Fire OnSessionStart hook (fire-and-forget)
        if let Some(ref hooks) = self.hooks {
            let hooks = hooks.clone();
            let uid = user_id.to_string();
            let sid = session_id;
            tokio::spawn(async move {
                use crate::hooks::HookEvent;
                let event = HookEvent::SessionStart {
                    user_id: uid,
                    session_id: sid,
                };
                if let Err(e) = hooks.run(&event).await {
                    tracing::warn!("OnSessionStart hook error: {}", e);
                }
            });
        }

        session
    }

    /// Resolve an external thread ID to an internal thread.
    ///
    /// Returns the session and thread ID. Creates both if they don't exist.
    /// Delegates to [`resolve_thread_with_parsed_uuid`](Self::resolve_thread_with_parsed_uuid)
    /// with `parsed_uuid: None`.
    pub async fn resolve_thread(
        &self,
        user_id: &str,
        channel: &str,
        external_thread_id: Option<&str>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        self.resolve_thread_with_parsed_uuid(user_id, channel, external_thread_id, None)
            .await
    }

    /// Like [`resolve_thread`](Self::resolve_thread), but accepts a pre-parsed
    /// UUID to skip redundant parsing when the caller has already validated
    /// the external thread ID as a UUID (e.g. the approval routing path).
    ///
    /// Uses a single read-lock acquisition for both the key lookup and the UUID
    /// adoption check to reduce contention under concurrent approval load.
    pub async fn resolve_thread_with_parsed_uuid(
        &self,
        user_id: &str,
        channel: &str,
        external_thread_id: Option<&str>,
        parsed_uuid: Option<Uuid>,
    ) -> (Arc<Mutex<Session>>, Uuid) {
        let session = self.get_or_create_session(user_id).await;

        let key = ThreadKey {
            user_id: user_id.to_string(),
            channel: channel.to_string(),
            external_thread_id: external_thread_id.map(String::from),
        };

        // Use pre-parsed UUID if available, otherwise parse from string.
        let ext_uuid = parsed_uuid
            .or_else(|| external_thread_id.and_then(|ext_tid| Uuid::parse_str(ext_tid).ok()));

        // Validate that parsed_uuid (if provided) is consistent with external_thread_id.
        #[cfg(debug_assertions)]
        if let (Some(parsed), Some(ext_tid)) = (&parsed_uuid, external_thread_id) {
            debug_assert_eq!(
                Uuid::parse_str(ext_tid).ok().as_ref(),
                Some(parsed),
                "parsed_uuid must be the parsed form of external_thread_id"
            );
        }

        // Single read lock for both the key lookup and UUID adoption check
        let adoptable_uuid = {
            let thread_map = self.thread_map.read().await;

            // Fast path: exact key match
            if let Some(&thread_id) = thread_map.get(&key) {
                let sess = session.lock().await;
                if sess.threads.contains_key(&thread_id) {
                    return (Arc::clone(&session), thread_id);
                }
            }

            // UUID adoption check (still under the same read lock).
            // If external_thread_id is a valid UUID not mapped elsewhere,
            // it may be a thread created by chat_new_thread_handler or
            // hydrated from DB that we can adopt.
            // Only attempt adoption when external_thread_id is Some, preserving
            // the invariant that None external_thread_id never triggers adoption.
            if external_thread_id.is_some() {
                ext_uuid.filter(|&uuid| !thread_map.values().any(|&v| v == uuid))
            } else {
                None
            }
        }; // Single read lock dropped here

        // If we found an adoptable UUID, verify it exists in session and acquire write lock
        if let Some(ext_uuid) = adoptable_uuid {
            let sess = session.lock().await;
            if sess.threads.contains_key(&ext_uuid) {
                drop(sess);

                let mut thread_map = self.thread_map.write().await;
                // Re-check after acquiring write lock to prevent race condition
                // where another task mapped this UUID between our read and write.
                if !thread_map.values().any(|&v| v == ext_uuid) {
                    thread_map.insert(key, ext_uuid);
                    drop(thread_map);
                    // Ensure undo manager exists
                    let mut undo_managers = self.undo_managers.write().await;
                    undo_managers
                        .entry(ext_uuid)
                        .or_insert_with(|| Arc::new(Mutex::new(UndoManager::new())));
                    return (session, ext_uuid);
                }
                // If mapped elsewhere while unlocked, fall through to create new thread
            }
        }

        // Create new thread (always create a new one for a new key).
        // If the external_thread_id is a valid UUID, adopt it as the internal
        // thread ID so that callers (e.g. the Responses API) can look up
        // conversations by the same UUID they encoded in the response ID.
        let thread_id = {
            let mut sess = session.lock().await;
            let thread = if let Some(uuid) = ext_uuid {
                sess.create_thread_with_id(uuid)
            } else {
                sess.create_thread()
            };
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

        // Find stale sessions (user_id + session_id)
        let stale_sessions: Vec<(String, String)> = {
            let sessions = self.sessions.read().await;
            sessions
                .iter()
                .filter_map(|(user_id, session)| {
                    // Try to lock; skip if contended (someone is actively using it)
                    let sess = session.try_lock().ok()?;
                    if sess.last_active_at < cutoff {
                        Some((user_id.clone(), sess.id.to_string()))
                    } else {
                        None
                    }
                })
                .collect()
        };

        let stale_users: Vec<String> = stale_sessions
            .iter()
            .map(|(user_id, _)| user_id.clone())
            .collect();

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

        // Fire OnSessionEnd hooks for stale sessions (fire-and-forget)
        if let Some(ref hooks) = self.hooks {
            for (user_id, session_id) in &stale_sessions {
                let hooks = hooks.clone();
                let uid = user_id.clone();
                let sid = session_id.clone();
                tokio::spawn(async move {
                    use crate::hooks::HookEvent;
                    let event = HookEvent::SessionEnd {
                        user_id: uid,
                        session_id: sid,
                    };
                    if let Err(e) = hooks.run(&event).await {
                        tracing::warn!("OnSessionEnd hook error: {}", e);
                    }
                });
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

    #[tokio::test]
    async fn test_register_then_resolve_same_uuid_on_second_channel_reuses_thread() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let tid = Uuid::new_v4();

        let session = Arc::new(Mutex::new(Session::new("user-cross")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(tid, sess.id);
            sess.threads.insert(tid, thread);
        }

        manager
            .register_thread("user-cross", "http", tid, Arc::clone(&session))
            .await;
        manager
            .register_thread("user-cross", "gateway", tid, Arc::clone(&session))
            .await;

        let (_, resolved) = manager
            .resolve_thread("user-cross", "gateway", Some(&tid.to_string()))
            .await;
        assert_eq!(resolved, tid);
    }

    // === QA Plan P3 - 4.2: Concurrent session stress tests ===

    #[tokio::test]
    async fn concurrent_get_or_create_same_user_returns_same_session() {
        let manager = Arc::new(SessionManager::new());

        let handles: Vec<_> = (0..30)
            .map(|_| {
                let mgr = Arc::clone(&manager);
                tokio::spawn(async move { mgr.get_or_create_session("shared-user").await })
            })
            .collect();

        let mut sessions = Vec::new();
        for handle in handles {
            sessions.push(handle.await.expect("task should not panic"));
        }

        // All 30 must return the *same* Arc (double-checked locking guarantee).
        for s in &sessions {
            assert!(Arc::ptr_eq(&sessions[0], s));
        }
    }

    #[tokio::test]
    async fn concurrent_resolve_thread_distinct_users_no_cross_talk() {
        let manager = Arc::new(SessionManager::new());

        let handles: Vec<_> = (0..20)
            .map(|i| {
                let mgr = Arc::clone(&manager);
                tokio::spawn(async move {
                    let user = format!("user-{i}");
                    let (session, tid) = mgr.resolve_thread(&user, "gateway", None).await;
                    (user, session, tid)
                })
            })
            .collect();

        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await.expect("task should not panic"));
        }

        // All thread IDs must be unique.
        let tids: std::collections::HashSet<_> = results.iter().map(|(_, _, t)| *t).collect();
        assert_eq!(tids.len(), 20);

        // Each session should contain exactly 1 thread (its own).
        for (_, session, tid) in &results {
            let sess = session.lock().await;
            assert!(sess.threads.contains_key(tid));
            assert_eq!(sess.threads.len(), 1);
        }
    }

    #[tokio::test]
    async fn concurrent_resolve_thread_same_user_different_channels() {
        let manager = Arc::new(SessionManager::new());
        let channels = ["gateway", "telegram", "slack", "cli", "repl"];

        let handles: Vec<_> = channels
            .iter()
            .map(|ch| {
                let mgr = Arc::clone(&manager);
                let channel = ch.to_string();
                tokio::spawn(async move {
                    let (session, tid) = mgr.resolve_thread("multi-ch", &channel, None).await;
                    (channel, session, tid)
                })
            })
            .collect();

        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.await.expect("task should not panic"));
        }

        // All 5 threads must be unique (different channels = different keys).
        let tids: std::collections::HashSet<_> = results.iter().map(|(_, _, t)| *t).collect();
        assert_eq!(tids.len(), 5);

        // All threads should live in the same session.
        let sess = results[0].1.lock().await;
        assert_eq!(sess.threads.len(), 5);
    }

    #[tokio::test]
    async fn concurrent_get_undo_manager_same_thread_returns_same_arc() {
        let manager = Arc::new(SessionManager::new());
        let (_, tid) = manager.resolve_thread("undo-user", "gateway", None).await;

        let handles: Vec<_> = (0..20)
            .map(|_| {
                let mgr = Arc::clone(&manager);
                tokio::spawn(async move { mgr.get_undo_manager(tid).await })
            })
            .collect();

        let mut managers = Vec::new();
        for handle in handles {
            managers.push(handle.await.expect("task should not panic"));
        }

        // All 20 must point to the same UndoManager.
        for m in &managers {
            assert!(Arc::ptr_eq(&managers[0], m));
        }
    }

    #[tokio::test]
    async fn test_resolve_thread_consolidates_read_path() {
        // Verify that resolve_thread still correctly handles:
        // 1. Fast path: key exists in thread_map
        // 2. UUID adoption: external_thread_id is a UUID in session but not in map
        // 3. New thread: neither path matches
        use crate::agent::session::Thread;

        let manager = SessionManager::new();

        // Case 1: Normal resolution creates thread and maps it
        let (session1, tid1) = manager
            .resolve_thread("user1", "chan1", Some("ext-1"))
            .await;
        // Resolving again with same key should return same thread (fast path)
        let (_, tid1_again) = manager
            .resolve_thread("user1", "chan1", Some("ext-1"))
            .await;
        assert_eq!(tid1, tid1_again);

        // Case 2: UUID adoption - insert a thread directly into session
        let adopted_id = Uuid::new_v4();
        {
            let mut sess = session1.lock().await;
            let thread = Thread::with_id(adopted_id, sess.id);
            sess.threads.insert(adopted_id, thread);
        }
        // Resolve with the UUID as external_thread_id -- should adopt it
        let (_, resolved) = manager
            .resolve_thread("user1", "chan1", Some(&adopted_id.to_string()))
            .await;
        assert_eq!(resolved, adopted_id);

        // Case 3: Different channel gets different thread
        let (_, tid2) = manager.resolve_thread("user1", "chan2", None).await;
        assert_ne!(tid1, tid2);
    }

    #[tokio::test]
    async fn test_resolve_thread_finds_existing_session_thread_by_uuid() {
        use crate::agent::session::{Session, Thread};

        let manager = SessionManager::new();
        let tid = Uuid::new_v4();

        // Simulate chat_new_thread_handler: create thread directly in session
        // without registering it in thread_map
        let session = Arc::new(Mutex::new(Session::new("user-direct")));
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(tid, sess.id);
            sess.threads.insert(tid, thread);
        }
        {
            let mut sessions = manager.sessions.write().await;
            sessions.insert("user-direct".to_string(), Arc::clone(&session));
        }

        // resolve_thread should find the existing thread by UUID
        // instead of creating a duplicate
        let (_, resolved) = manager
            .resolve_thread("user-direct", "gateway", Some(&tid.to_string()))
            .await;
        assert_eq!(
            resolved, tid,
            "should reuse existing thread, not create a new one"
        );

        // Verify no duplicate threads were created
        let sess = session.lock().await;
        assert_eq!(
            sess.threads.len(),
            1,
            "should have exactly 1 thread, not a duplicate"
        );
    }

    #[tokio::test]
    async fn test_resolve_thread_with_pre_parsed_uuid_adopts_thread() {
        use crate::agent::session::Thread;

        let manager = SessionManager::new();
        let (session, _) = manager.resolve_thread("user1", "chan1", None).await;

        // Manually insert a thread with a known UUID
        let known_id = Uuid::new_v4();
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(known_id, sess.id);
            sess.threads.insert(known_id, thread);
        }

        // Resolve with pre-parsed UUID -- should adopt it without re-parsing
        let (_, resolved) = manager
            .resolve_thread_with_parsed_uuid(
                "user1",
                "chan1",
                Some(&known_id.to_string()),
                Some(known_id),
            )
            .await;
        assert_eq!(resolved, known_id);
    }

    #[tokio::test]
    async fn test_resolve_thread_with_parsed_uuid_none_delegates_to_parse() {
        use crate::agent::session::Thread;

        let manager = SessionManager::new();
        let (session, _) = manager.resolve_thread("user2", "chan2", None).await;

        // Insert a thread with a known UUID
        let known_id = Uuid::new_v4();
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(known_id, sess.id);
            sess.threads.insert(known_id, thread);
        }

        // Resolve with parsed_uuid=None but a valid UUID string -- should
        // fall back to parsing the string and still adopt the thread
        let (_, resolved) = manager
            .resolve_thread_with_parsed_uuid("user2", "chan2", Some(&known_id.to_string()), None)
            .await;
        assert_eq!(resolved, known_id);
    }

    #[tokio::test]
    async fn test_resolve_thread_with_none_external_thread_id_does_not_adopt() {
        use crate::agent::session::Thread;

        let manager = SessionManager::new();
        let (session, default_tid) = manager.resolve_thread("user3", "chan3", None).await;

        // Manually insert a thread with a known UUID (simulating a thread
        // created by chat_new_thread_handler)
        let known_id = Uuid::new_v4();
        {
            let mut sess = session.lock().await;
            let thread = Thread::with_id(known_id, sess.id);
            sess.threads.insert(known_id, thread);
        }

        // Resolve with external_thread_id=None but parsed_uuid=Some.
        // This should NOT adopt the UUID — the old code prevented adoption
        // when external_thread_id was None, and we preserve that invariant.
        let (_, resolved) = manager
            .resolve_thread_with_parsed_uuid("user3", "chan3", None, Some(known_id))
            .await;

        // Should return the existing default thread, not the injected UUID
        assert_eq!(
            resolved, default_tid,
            "should return existing default thread when external_thread_id is None"
        );
        assert_ne!(
            resolved, known_id,
            "should NOT adopt UUID when external_thread_id is None"
        );
    }
}

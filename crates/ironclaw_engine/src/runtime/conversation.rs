//! Conversation manager — routes UI messages to threads.
//!
//! The ConversationManager is the bridge between channel I/O (user messages,
//! status updates) and the thread execution model. It maintains conversation
//! surfaces and decides whether to spawn new threads or inject messages into
//! existing ones.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::debug;

use crate::runtime::manager::ThreadManager;
use crate::runtime::messaging::ThreadOutcome;
use crate::types::conversation::{
    ConversationEntry, ConversationId, ConversationSurface,
};
use crate::types::error::EngineError;
use crate::types::message::ThreadMessage;
use crate::types::project::ProjectId;
use crate::types::thread::{ThreadConfig, ThreadId, ThreadType};

/// Manages conversation surfaces and routes messages to threads.
///
/// Each channel message arrives here. The manager decides whether to:
/// 1. Spawn a new foreground thread for the message
/// 2. Inject the message into an existing active thread
/// 3. Create a new conversation if none exists for this channel+user
pub struct ConversationManager {
    thread_manager: Arc<ThreadManager>,
    conversations: RwLock<HashMap<ConversationId, ConversationSurface>>,
    /// Maps (channel, user_id) → conversation ID for lookup.
    channel_user_index: RwLock<HashMap<(String, String), ConversationId>>,
}

impl ConversationManager {
    pub fn new(thread_manager: Arc<ThreadManager>) -> Self {
        Self {
            thread_manager,
            conversations: RwLock::new(HashMap::new()),
            channel_user_index: RwLock::new(HashMap::new()),
        }
    }

    /// Get or create a conversation for a channel+user pair.
    pub async fn get_or_create_conversation(
        &self,
        channel: &str,
        user_id: &str,
    ) -> ConversationId {
        // Check index first
        let key = (channel.to_string(), user_id.to_string());
        {
            let index = self.channel_user_index.read().await;
            if let Some(conv_id) = index.get(&key) {
                return *conv_id;
            }
        }

        // Create new conversation
        let conv = ConversationSurface::new(channel, user_id);
        let conv_id = conv.id;

        let mut convs = self.conversations.write().await;
        let mut index = self.channel_user_index.write().await;
        convs.insert(conv_id, conv);
        index.insert(key, conv_id);

        debug!(conversation_id = %conv_id, channel, user_id, "created conversation");
        conv_id
    }

    /// Handle an incoming user message.
    ///
    /// If the conversation has an active foreground thread, the message is
    /// injected into it. Otherwise, a new foreground thread is spawned.
    ///
    /// Returns the thread ID that is handling the message.
    pub async fn handle_user_message(
        &self,
        conversation_id: ConversationId,
        content: &str,
        project_id: ProjectId,
        user_id: &str,
        thread_config: ThreadConfig,
    ) -> Result<ThreadId, EngineError> {
        let mut convs = self.conversations.write().await;
        let conv = convs
            .get_mut(&conversation_id)
            .ok_or(EngineError::Store {
                reason: format!("conversation {conversation_id} not found"),
            })?;

        // Record the user entry
        conv.add_entry(ConversationEntry::user(content));

        // Check for an active foreground thread
        let active_foreground = self.find_active_foreground(conv).await;

        match active_foreground {
            Some(thread_id) => {
                // Inject into existing thread
                debug!(
                    conversation_id = %conversation_id,
                    thread_id = %thread_id,
                    "injecting message into active thread"
                );
                self.thread_manager
                    .inject_message(thread_id, ThreadMessage::user(content))
                    .await?;
                Ok(thread_id)
            }
            None => {
                // Build conversation history from prior entries for context continuity
                let history = build_history_from_entries(&conv.entries);

                // Spawn new foreground thread with conversation history
                let thread_id = self
                    .thread_manager
                    .spawn_thread_with_history(
                        content, // use message as goal
                        ThreadType::Foreground,
                        project_id,
                        thread_config,
                        None,
                        user_id,
                        history,
                    )
                    .await?;

                conv.track_thread(thread_id);
                conv.add_entry(ConversationEntry::system_for_thread(
                    thread_id,
                    "Thread started",
                ));

                debug!(
                    conversation_id = %conversation_id,
                    thread_id = %thread_id,
                    "spawned new foreground thread"
                );
                Ok(thread_id)
            }
        }
    }

    /// Record a thread's outcome in its conversation.
    pub async fn record_thread_outcome(
        &self,
        conversation_id: ConversationId,
        thread_id: ThreadId,
        outcome: &ThreadOutcome,
    ) {
        let mut convs = self.conversations.write().await;
        if let Some(conv) = convs.get_mut(&conversation_id) {
            match outcome {
                ThreadOutcome::Completed { response } => {
                    if let Some(text) = response {
                        conv.add_entry(ConversationEntry::agent(thread_id, text));
                    }
                    conv.untrack_thread(thread_id);
                }
                ThreadOutcome::Stopped => {
                    conv.add_entry(ConversationEntry::system_for_thread(
                        thread_id,
                        "Thread stopped",
                    ));
                    conv.untrack_thread(thread_id);
                }
                ThreadOutcome::MaxIterations => {
                    conv.add_entry(ConversationEntry::system_for_thread(
                        thread_id,
                        "Thread reached max iterations",
                    ));
                    conv.untrack_thread(thread_id);
                }
                ThreadOutcome::Failed { error } => {
                    conv.add_entry(ConversationEntry::system_for_thread(
                        thread_id,
                        format!("Thread failed: {error}"),
                    ));
                    conv.untrack_thread(thread_id);
                }
                ThreadOutcome::NeedApproval {
                    action_name,
                    call_id: _,
                    parameters: _,
                } => {
                    conv.add_entry(ConversationEntry::system_for_thread(
                        thread_id,
                        format!("Approval needed for action: {action_name}"),
                    ));
                    // Thread stays active — waiting for approval
                }
            }
        }
    }

    /// Get a snapshot of a conversation.
    pub async fn get_conversation(
        &self,
        conversation_id: ConversationId,
    ) -> Option<ConversationSurface> {
        let convs = self.conversations.read().await;
        convs.get(&conversation_id).cloned()
    }

    /// List all conversations for a user.
    pub async fn list_conversations(&self, user_id: &str) -> Vec<ConversationSurface> {
        let convs = self.conversations.read().await;
        convs
            .values()
            .filter(|c| c.user_id == user_id)
            .cloned()
            .collect()
    }

    /// Find an active foreground thread in a conversation.
    async fn find_active_foreground(&self, conv: &ConversationSurface) -> Option<ThreadId> {
        for &tid in &conv.active_threads {
            if self.thread_manager.is_running(tid).await {
                return Some(tid);
            }
        }
        None
    }
}

/// Build ThreadMessage history from conversation entries.
///
/// Converts user and agent entries into ThreadMessages so a new thread
/// inherits context from prior turns in the same conversation.
fn build_history_from_entries(
    entries: &[ConversationEntry],
) -> Vec<crate::types::message::ThreadMessage> {
    use crate::types::conversation::EntrySender;

    // Skip the last entry (it's the current user message, added by the caller
    // before this function runs). Also skip system entries (thread lifecycle
    // notifications aren't useful as LLM context).
    let history_entries = if entries.len() > 1 {
        &entries[..entries.len() - 1]
    } else {
        return Vec::new();
    };

    history_entries
        .iter()
        .filter_map(|entry| match &entry.sender {
            EntrySender::User => {
                Some(crate::types::message::ThreadMessage::user(&entry.content))
            }
            EntrySender::Agent { .. } => {
                Some(crate::types::message::ThreadMessage::assistant(&entry.content))
            }
            EntrySender::System => None, // skip system notifications
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::lease::LeaseManager;
    use crate::capability::policy::PolicyEngine;
    use crate::capability::registry::CapabilityRegistry;
    use crate::traits::effect::EffectExecutor;
    use crate::traits::llm::{LlmBackend, LlmCallConfig, LlmOutput};
    use crate::types::conversation::EntrySender;
    use crate::types::capability::{ActionDef, CapabilityLease};
    use crate::types::event::ThreadEvent;
    use crate::types::memory::{DocId, MemoryDoc};
    use crate::types::project::Project;
    use crate::types::step::{ActionResult, LlmResponse, Step, TokenUsage};
    use crate::types::thread::ThreadState;
    use crate::traits::store::Store;
    use std::sync::Mutex;
    use std::time::Duration;

    // ── Mocks (same as manager tests) ───────────────────────

    struct MockLlm(Mutex<Vec<LlmOutput>>);

    #[async_trait::async_trait]
    impl LlmBackend for MockLlm {
        async fn complete(
            &self, _: &[ThreadMessage], _: &[ActionDef], _: &LlmCallConfig,
        ) -> Result<LlmOutput, EngineError> {
            let mut r = self.0.lock().unwrap();
            if r.is_empty() {
                Ok(LlmOutput { response: LlmResponse::Text("done".into()), usage: TokenUsage::default() })
            } else {
                Ok(r.remove(0))
            }
        }
        fn model_name(&self) -> &str { "mock" }
    }

    struct MockEffects;

    #[async_trait::async_trait]
    impl EffectExecutor for MockEffects {
        async fn execute_action(&self, _: &str, _: serde_json::Value, _: &CapabilityLease, _: &crate::traits::effect::ThreadExecutionContext) -> Result<ActionResult, EngineError> {
            Ok(ActionResult { call_id: String::new(), action_name: String::new(), output: serde_json::json!({}), is_error: false, duration: Duration::from_millis(1) })
        }
        async fn available_actions(&self, _: &[CapabilityLease]) -> Result<Vec<ActionDef>, EngineError> { Ok(vec![]) }
    }

    struct MockStore;

    #[async_trait::async_trait]
    impl Store for MockStore {
        async fn save_thread(&self, _: &crate::types::thread::Thread) -> Result<(), EngineError> { Ok(()) }
        async fn load_thread(&self, _: ThreadId) -> Result<Option<crate::types::thread::Thread>, EngineError> { Ok(None) }
        async fn list_threads(&self, _: ProjectId) -> Result<Vec<crate::types::thread::Thread>, EngineError> { Ok(vec![]) }
        async fn update_thread_state(&self, _: ThreadId, _: ThreadState) -> Result<(), EngineError> { Ok(()) }
        async fn save_step(&self, _: &Step) -> Result<(), EngineError> { Ok(()) }
        async fn load_steps(&self, _: ThreadId) -> Result<Vec<Step>, EngineError> { Ok(vec![]) }
        async fn append_events(&self, _: &[ThreadEvent]) -> Result<(), EngineError> { Ok(()) }
        async fn load_events(&self, _: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> { Ok(vec![]) }
        async fn save_project(&self, _: &Project) -> Result<(), EngineError> { Ok(()) }
        async fn load_project(&self, _: ProjectId) -> Result<Option<Project>, EngineError> { Ok(None) }
        async fn save_memory_doc(&self, _: &MemoryDoc) -> Result<(), EngineError> { Ok(()) }
        async fn load_memory_doc(&self, _: DocId) -> Result<Option<MemoryDoc>, EngineError> { Ok(None) }
        async fn list_memory_docs(&self, _: ProjectId) -> Result<Vec<MemoryDoc>, EngineError> { Ok(vec![]) }
        async fn save_lease(&self, _: &CapabilityLease) -> Result<(), EngineError> { Ok(()) }
        async fn load_active_leases(&self, _: ThreadId) -> Result<Vec<CapabilityLease>, EngineError> { Ok(vec![]) }
        async fn revoke_lease(&self, _: crate::types::capability::LeaseId, _: &str) -> Result<(), EngineError> { Ok(()) }
    }

    fn make_conv_manager() -> (Arc<ThreadManager>, ConversationManager) {
        let tm = Arc::new(ThreadManager::new(
            Arc::new(MockLlm(Mutex::new(vec![
                LlmOutput { response: LlmResponse::Text("Hello!".into()), usage: TokenUsage::default() },
            ]))),
            Arc::new(MockEffects),
            Arc::new(MockStore),
            Arc::new(CapabilityRegistry::new()),
            Arc::new(LeaseManager::new()),
            Arc::new(PolicyEngine::new()),
        ));
        let cm = ConversationManager::new(Arc::clone(&tm));
        (tm, cm)
    }

    // ── Tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn get_or_create_conversation() {
        let (_, cm) = make_conv_manager();
        let c1 = cm.get_or_create_conversation("telegram", "user1").await;
        let c2 = cm.get_or_create_conversation("telegram", "user1").await;
        assert_eq!(c1, c2); // same channel+user returns same conversation

        let c3 = cm.get_or_create_conversation("slack", "user1").await;
        assert_ne!(c1, c3); // different channel → different conversation
    }

    #[tokio::test]
    async fn handle_message_spawns_thread() {
        let (tm, cm) = make_conv_manager();
        let conv_id = cm.get_or_create_conversation("web", "user1").await;
        let project = ProjectId::new();

        let tid = cm
            .handle_user_message(conv_id, "Hello", project, "user1", ThreadConfig::default())
            .await
            .unwrap();

        // Thread was spawned
        let conv = cm.get_conversation(conv_id).await.unwrap();
        assert!(conv.active_threads.contains(&tid));
        assert_eq!(conv.entries.len(), 2); // user message + "Thread started"

        // Wait for thread to complete
        let outcome = tm.join_thread(tid).await.unwrap();
        assert!(matches!(outcome, ThreadOutcome::Completed { .. }));
    }

    #[tokio::test]
    async fn record_outcome_adds_entry() {
        let (_, cm) = make_conv_manager();
        let conv_id = cm.get_or_create_conversation("cli", "user1").await;
        let tid = ThreadId::new();

        // Manually track a thread
        {
            let mut convs = cm.conversations.write().await;
            let conv = convs.get_mut(&conv_id).unwrap();
            conv.track_thread(tid);
        }

        // Record completion
        cm.record_thread_outcome(
            conv_id,
            tid,
            &ThreadOutcome::Completed {
                response: Some("Done!".into()),
            },
        )
        .await;

        let conv = cm.get_conversation(conv_id).await.unwrap();
        assert!(conv.active_threads.is_empty());
        assert_eq!(conv.entries.len(), 1);
        assert_eq!(conv.entries[0].content, "Done!");

        // Check sender is agent
        assert!(matches!(
            conv.entries[0].sender,
            EntrySender::Agent { thread_id } if thread_id == tid
        ));
    }

    #[tokio::test]
    async fn list_conversations_filters_by_user() {
        let (_, cm) = make_conv_manager();
        cm.get_or_create_conversation("web", "alice").await;
        cm.get_or_create_conversation("telegram", "alice").await;
        cm.get_or_create_conversation("web", "bob").await;

        let alice_convs = cm.list_conversations("alice").await;
        assert_eq!(alice_convs.len(), 2);

        let bob_convs = cm.list_conversations("bob").await;
        assert_eq!(bob_convs.len(), 1);
    }
}

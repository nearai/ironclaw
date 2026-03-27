//! Context building for LLM calls.
//!
//! Assembles the message sequence and action definitions from thread state,
//! active leases, and project memory docs retrieved via the [`RetrievalEngine`].

use std::sync::Arc;

use crate::memory::RetrievalEngine;
use crate::traits::effect::EffectExecutor;
use crate::types::capability::{ActionDef, CapabilityLease};
use crate::types::error::EngineError;
use crate::types::memory::MemoryDoc;
use crate::types::message::ThreadMessage;
use crate::types::project::ProjectId;

/// Maximum number of memory docs to inject into context.
const MAX_CONTEXT_DOCS: usize = 5;

/// Build the context for an LLM call: messages and available actions.
///
/// Retrieves relevant memory docs from the project and injects them as a
/// system message after the main system prompt. This gives the LLM access
/// to lessons learned, playbooks, and known issues from prior threads.
pub async fn build_step_context(
    messages: &[ThreadMessage],
    leases: &[CapabilityLease],
    effects: &Arc<dyn EffectExecutor>,
    retrieval: Option<&RetrievalEngine>,
    project_id: ProjectId,
    goal: &str,
) -> Result<(Vec<ThreadMessage>, Vec<ActionDef>), EngineError> {
    let actions = effects.available_actions(leases).await?;

    let mut ctx_messages = messages.to_vec();

    // Inject retrieved memory docs into the existing system prompt.
    // Many providers require all system messages at the beginning (or a single
    // system message), so we append to the first system message rather than
    // inserting a separate one.
    if let Some(engine) = retrieval {
        let docs = engine
            .retrieve_context(project_id, goal, MAX_CONTEXT_DOCS)
            .await?;
        if !docs.is_empty() {
            let context_section = format_docs_as_context(&docs);
            if !ctx_messages.is_empty()
                && ctx_messages[0].role == crate::types::message::MessageRole::System
            {
                // Append to existing system prompt
                ctx_messages[0].content.push_str("\n\n");
                ctx_messages[0].content.push_str(&context_section);
            } else {
                // No system message — prepend as one
                ctx_messages.insert(0, ThreadMessage::system(context_section));
            }
        }
    }

    Ok((ctx_messages, actions))
}

/// Format memory docs into a system message for context injection.
fn format_docs_as_context(docs: &[MemoryDoc]) -> String {
    let mut parts = vec!["## Prior Knowledge (from completed threads)\n".to_string()];

    for doc in docs {
        let type_label = match doc.doc_type {
            crate::types::memory::DocType::Lesson => "LESSON",
            crate::types::memory::DocType::Spec => "MISSING CAPABILITY",
            crate::types::memory::DocType::Playbook => "PLAYBOOK",
            crate::types::memory::DocType::Issue => "KNOWN ISSUE",
            crate::types::memory::DocType::Summary => "CONTEXT",
            crate::types::memory::DocType::Note => "NOTE",
            crate::types::memory::DocType::Skill => "SKILL",
        };
        // Truncate long docs to avoid context bloat
        let content: String = doc.content.chars().take(500).collect();
        let truncated = if doc.content.chars().count() > 500 {
            "..."
        } else {
            ""
        };
        parts.push(format!(
            "### [{type_label}] {}\n{content}{truncated}\n",
            doc.title
        ));
    }

    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::capability::{CapabilityLease, LeaseId};
    use crate::types::event::ThreadEvent;
    use crate::types::memory::{DocId, DocType};
    use crate::types::project::{Project, ProjectId};
    use crate::types::step::{ActionResult, Step};
    use crate::types::thread::{Thread, ThreadId, ThreadState};

    struct MockEffects;

    #[async_trait::async_trait]
    impl EffectExecutor for MockEffects {
        async fn execute_action(
            &self,
            _: &str,
            _: serde_json::Value,
            _: &CapabilityLease,
            _: &crate::traits::effect::ThreadExecutionContext,
        ) -> Result<ActionResult, EngineError> {
            Ok(ActionResult {
                call_id: String::new(),
                action_name: String::new(),
                output: serde_json::json!({}),
                is_error: false,
                duration: std::time::Duration::from_millis(1),
            })
        }

        async fn available_actions(
            &self,
            _: &[CapabilityLease],
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(vec![])
        }
    }

    struct DocStore(Vec<MemoryDoc>);

    #[async_trait::async_trait]
    impl crate::traits::store::Store for DocStore {
        async fn save_thread(&self, _: &Thread) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_thread(&self, _: ThreadId) -> Result<Option<Thread>, EngineError> {
            Ok(None)
        }
        async fn list_threads(&self, _: ProjectId) -> Result<Vec<Thread>, EngineError> {
            Ok(vec![])
        }
        async fn update_thread_state(
            &self,
            _: ThreadId,
            _: ThreadState,
        ) -> Result<(), EngineError> {
            Ok(())
        }
        async fn save_step(&self, _: &Step) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_steps(&self, _: ThreadId) -> Result<Vec<Step>, EngineError> {
            Ok(vec![])
        }
        async fn append_events(&self, _: &[ThreadEvent]) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_events(&self, _: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> {
            Ok(vec![])
        }
        async fn save_project(&self, _: &Project) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_project(&self, _: ProjectId) -> Result<Option<Project>, EngineError> {
            Ok(None)
        }
        async fn save_memory_doc(&self, _: &MemoryDoc) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_memory_doc(&self, _: DocId) -> Result<Option<MemoryDoc>, EngineError> {
            Ok(None)
        }
        async fn list_memory_docs(&self, pid: ProjectId) -> Result<Vec<MemoryDoc>, EngineError> {
            Ok(self
                .0
                .iter()
                .filter(|d| d.project_id == pid)
                .cloned()
                .collect())
        }
        async fn save_lease(&self, _: &CapabilityLease) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_active_leases(
            &self,
            _: ThreadId,
        ) -> Result<Vec<CapabilityLease>, EngineError> {
            Ok(vec![])
        }
        async fn revoke_lease(&self, _: LeaseId, _: &str) -> Result<(), EngineError> {
            Ok(())
        }
        async fn save_mission(
            &self,
            _: &crate::types::mission::Mission,
        ) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_mission(
            &self,
            _: crate::types::mission::MissionId,
        ) -> Result<Option<crate::types::mission::Mission>, EngineError> {
            Ok(None)
        }
        async fn list_missions(
            &self,
            _: ProjectId,
        ) -> Result<Vec<crate::types::mission::Mission>, EngineError> {
            Ok(vec![])
        }
        async fn update_mission_status(
            &self,
            _: crate::types::mission::MissionId,
            _: crate::types::mission::MissionStatus,
        ) -> Result<(), EngineError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn context_injects_docs_after_system_prompt() {
        let project = ProjectId::new();
        let store: Arc<dyn crate::traits::store::Store> = Arc::new(DocStore(vec![MemoryDoc::new(
            project,
            DocType::Lesson,
            "web tool alias",
            "Use web-search not web_search",
        )]));
        let retrieval = RetrievalEngine::new(store);
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects);

        let messages = vec![
            ThreadMessage::system("You are an assistant."),
            ThreadMessage::user("search the web"),
        ];

        let (ctx_msgs, _) = build_step_context(
            &messages,
            &[],
            &effects,
            Some(&retrieval),
            project,
            "search the web",
        )
        .await
        .unwrap();

        // Should have 2 messages: system prompt (with docs appended), user message
        assert_eq!(ctx_msgs.len(), 2);
        assert_eq!(ctx_msgs[0].role, crate::types::message::MessageRole::System);
        assert!(ctx_msgs[0].content.contains("You are an assistant."));
        assert!(ctx_msgs[0].content.contains("Prior Knowledge"));
        assert!(ctx_msgs[0].content.contains("LESSON"));
        assert!(ctx_msgs[0].content.contains("web-search"));
        assert_eq!(ctx_msgs[1].role, crate::types::message::MessageRole::User);
    }

    #[tokio::test]
    async fn context_without_retrieval_passes_through() {
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects);
        let messages = vec![
            ThreadMessage::system("prompt"),
            ThreadMessage::user("hello"),
        ];

        let (ctx_msgs, _) =
            build_step_context(&messages, &[], &effects, None, ProjectId::new(), "hello")
                .await
                .unwrap();

        // No injection — same number of messages
        assert_eq!(ctx_msgs.len(), 2);
    }

    #[tokio::test]
    async fn context_no_docs_means_no_injection() {
        let project = ProjectId::new();
        let store: Arc<dyn crate::traits::store::Store> = Arc::new(DocStore(vec![]));
        let retrieval = RetrievalEngine::new(store);
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects);

        let messages = vec![ThreadMessage::user("hello")];

        let (ctx_msgs, _) =
            build_step_context(&messages, &[], &effects, Some(&retrieval), project, "hello")
                .await
                .unwrap();

        assert_eq!(ctx_msgs.len(), 1);
    }
}

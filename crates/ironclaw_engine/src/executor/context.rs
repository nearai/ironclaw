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
/// to lessons learned, skills, and known issues from prior threads.
pub async fn build_step_context(
    messages: &[ThreadMessage],
    leases: &[CapabilityLease],
    effects: &Arc<dyn EffectExecutor>,
    retrieval: Option<&RetrievalEngine>,
    project_id: ProjectId,
    user_id: &str,
    goal: &str,
) -> Result<(Vec<ThreadMessage>, Vec<ActionDef>), EngineError> {
    // Fetch actions and memory docs in parallel — they are independent.
    let actions_fut = effects.available_actions(leases);
    let docs_fut = async {
        if let Some(engine) = retrieval {
            engine
                .retrieve_context(project_id, user_id, goal, MAX_CONTEXT_DOCS)
                .await
        } else {
            Ok(Vec::new())
        }
    };

    let (actions_result, docs_result) = tokio::join!(actions_fut, docs_fut);
    let actions = actions_result?;
    let docs = docs_result?;

    let mut ctx_messages = messages.to_vec();

    // Inject retrieved memory docs into the existing system prompt.
    // Many providers require all system messages at the beginning (or a single
    // system message), so we append to the first system message rather than
    // inserting a separate one.
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

    Ok((ctx_messages, actions))
}

/// Format memory docs into a system message for context injection.
fn format_docs_as_context(docs: &[MemoryDoc]) -> String {
    let mut parts = vec!["## Prior Knowledge (from completed threads)\n".to_string()];

    for doc in docs {
        let type_label = match doc.doc_type {
            crate::types::memory::DocType::Lesson => "LESSON",
            crate::types::memory::DocType::Spec => "MISSING CAPABILITY",
            crate::types::memory::DocType::Issue => "KNOWN ISSUE",
            crate::types::memory::DocType::Summary => "CONTEXT",
            crate::types::memory::DocType::Note => "NOTE",
            crate::types::memory::DocType::Skill => "SKILL",
            crate::types::memory::DocType::Plan => "PLAN",
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
    use crate::types::capability::CapabilityLease;
    use crate::types::memory::DocType;
    use crate::types::project::ProjectId;
    use crate::types::step::ActionResult;

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

    #[tokio::test]
    async fn context_injects_docs_after_system_prompt() {
        let project = ProjectId::new();
        let store: Arc<dyn crate::traits::store::Store> =
            Arc::new(crate::tests::InMemoryStore::with_docs(vec![
                MemoryDoc::new(
                    project,
                    "test-user",
                    DocType::Lesson,
                    "web tool alias",
                    "Use web-search not web_search",
                ),
            ]));
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
            "test-user",
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

        let (ctx_msgs, _) = build_step_context(
            &messages,
            &[],
            &effects,
            None,
            ProjectId::new(),
            "test-user",
            "hello",
        )
        .await
        .unwrap();

        // No injection — same number of messages
        assert_eq!(ctx_msgs.len(), 2);
    }

    #[tokio::test]
    async fn context_no_docs_means_no_injection() {
        let project = ProjectId::new();
        let store: Arc<dyn crate::traits::store::Store> =
            Arc::new(crate::tests::InMemoryStore::new());
        let retrieval = RetrievalEngine::new(store);
        let effects: Arc<dyn EffectExecutor> = Arc::new(MockEffects);

        let messages = vec![ThreadMessage::user("hello")];

        let (ctx_msgs, _) = build_step_context(
            &messages,
            &[],
            &effects,
            Some(&retrieval),
            project,
            "test-user",
            "hello",
        )
        .await
        .unwrap();

        assert_eq!(ctx_msgs.len(), 1);
    }
}

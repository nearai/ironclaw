//! Reflection pipeline — produces structured knowledge from completed threads.
//!
//! After a thread completes, the reflection pipeline uses the LLM to:
//! 1. Summarize what the thread accomplished
//! 2. Extract lessons from failures and workarounds
//! 3. Detect unresolved issues
//! 4. Identify missing capabilities
//!
//! Each produces a MemoryDoc stored in the thread's project scope.

use std::sync::Arc;

use tracing::debug;

use crate::traits::llm::{LlmBackend, LlmCallConfig};
use crate::types::error::EngineError;
use crate::types::event::EventKind;
use crate::types::memory::{DocType, MemoryDoc};
use crate::types::message::ThreadMessage;
use crate::types::step::{LlmResponse, TokenUsage};
use crate::types::thread::Thread;

/// Result of running the reflection pipeline on a completed thread.
pub struct ReflectionResult {
    /// Memory docs produced by reflection.
    pub docs: Vec<MemoryDoc>,
    /// Total tokens used by reflection LLM calls.
    pub tokens_used: TokenUsage,
}

/// Run the reflection pipeline on a completed thread.
///
/// Produces structured knowledge (MemoryDocs) from the thread's messages
/// and events. Uses the LLM for summarization and analysis.
pub async fn reflect(
    thread: &Thread,
    llm: &Arc<dyn LlmBackend>,
) -> Result<ReflectionResult, EngineError> {
    let mut docs = Vec::new();
    let mut total_tokens = TokenUsage::default();

    // Build a transcript of the thread's work for the LLM to analyze
    let transcript = build_transcript(thread);

    // 1. Summary doc
    let (summary_doc, tokens) =
        produce_doc(thread, llm, DocType::Summary, &transcript, SUMMARY_PROMPT).await?;
    docs.push(summary_doc);
    total_tokens.input_tokens += tokens.input_tokens;
    total_tokens.output_tokens += tokens.output_tokens;

    // 2. Lessons (only if there were errors)
    let had_errors = thread.events.iter().any(|e| {
        matches!(
            e.kind,
            EventKind::ActionFailed { .. } | EventKind::StepFailed { .. }
        )
    });
    if had_errors {
        let (lesson_doc, tokens) =
            produce_doc(thread, llm, DocType::Lesson, &transcript, LESSON_PROMPT).await?;
        docs.push(lesson_doc);
        total_tokens.input_tokens += tokens.input_tokens;
        total_tokens.output_tokens += tokens.output_tokens;
    }

    // 3. Issues (if thread failed or had unresolved problems)
    let thread_failed = thread.state == crate::types::thread::ThreadState::Failed;
    if thread_failed || had_errors {
        let (issue_doc, tokens) =
            produce_doc(thread, llm, DocType::Issue, &transcript, ISSUE_PROMPT).await?;
        // Only add if the LLM produced non-trivial content
        if issue_doc.content.len() > 20 {
            docs.push(issue_doc);
        }
        total_tokens.input_tokens += tokens.input_tokens;
        total_tokens.output_tokens += tokens.output_tokens;
    }

    // 4. Missing capabilities (if tool-not-found errors detected)
    let has_missing_tools = thread.events.iter().any(|e| {
        if let EventKind::ActionFailed { error, .. } = &e.kind {
            error.contains("not found") || error.contains("not available")
        } else {
            false
        }
    });
    if has_missing_tools {
        let (spec_doc, tokens) =
            produce_doc(thread, llm, DocType::Spec, &transcript, SPEC_PROMPT).await?;
        if spec_doc.content.len() > 20 {
            docs.push(spec_doc);
        }
        total_tokens.input_tokens += tokens.input_tokens;
        total_tokens.output_tokens += tokens.output_tokens;
    }

    // 5. Playbook (successful threads with multiple tool-using steps)
    let action_count = thread
        .events
        .iter()
        .filter(|e| matches!(e.kind, EventKind::ActionExecuted { .. }))
        .count();
    let thread_succeeded =
        thread.state == crate::types::thread::ThreadState::Completed && !thread_failed;
    if thread_succeeded && action_count >= 2 {
        let (playbook_doc, tokens) =
            produce_doc(thread, llm, DocType::Playbook, &transcript, PLAYBOOK_PROMPT).await?;
        if playbook_doc.content.len() > 20 {
            docs.push(playbook_doc);
        }
        total_tokens.input_tokens += tokens.input_tokens;
        total_tokens.output_tokens += tokens.output_tokens;
    }

    debug!(
        thread_id = %thread.id,
        docs_produced = docs.len(),
        total_tokens = total_tokens.total(),
        "reflection complete"
    );

    Ok(ReflectionResult {
        docs,
        tokens_used: total_tokens,
    })
}

// ── Prompts ─────────────────────────────────────────────────

const SUMMARY_PROMPT: &str = "\
Summarize what this thread accomplished in 2-4 sentences. Include:
- The goal and whether it was achieved
- Key results or outputs
- Tools/actions that were used
Be factual and concise.";

const LESSON_PROMPT: &str = "\
Extract lessons learned from this thread's execution. Focus on:
- Errors encountered and how they were resolved (or not)
- Workarounds that were discovered
- Surprising findings about tool behavior
- Patterns that could be reused in similar tasks
Write each lesson as a single clear sentence. If there are no meaningful lessons, write 'No lessons.'.";

const ISSUE_PROMPT: &str = "\
Identify any unresolved issues from this thread. Focus on:
- Errors that were not resolved
- Tasks that could not be completed
- Missing tools or capabilities that were needed
- Data quality issues encountered
If there are no unresolved issues, write 'No issues.'.";

const SPEC_PROMPT: &str = "\
This thread encountered missing tools or capabilities. Analyze the errors and identify:
- Which tool names were attempted but not found
- What the correct tool name might be (if a similar tool exists under a different name)
- What capabilities would need to be added to handle this task
For each missing capability, write one line: MISSING: <attempted_name> -> <suggestion or description>.
If the tool exists under a different name, write: ALIAS: <attempted_name> -> <correct_name>.";

const PLAYBOOK_PROMPT: &str = "\
This thread successfully completed a multi-step task. Extract a reusable playbook:
- List the steps taken in order (tool calls, queries, transformations)
- Note which tools were used and in what sequence
- Describe the pattern so it can be reused for similar tasks
Write the playbook as a numbered list of steps. Be specific about tool names and parameters used.";

// ── Helpers ─────────────────────────────────────────────────

/// Build a concise transcript of the thread's work.
fn build_transcript(thread: &Thread) -> String {
    let mut parts = Vec::new();

    parts.push(format!("Goal: {}", thread.goal));
    parts.push(format!("Steps: {}", thread.step_count));
    parts.push(format!("Tokens used: {}", thread.total_tokens_used));
    parts.push(format!("State: {:?}", thread.state));

    // Include messages (truncated for very long threads)
    let max_messages = 30;
    let messages = if thread.messages.len() > max_messages {
        &thread.messages[thread.messages.len() - max_messages..]
    } else {
        &thread.messages
    };

    parts.push("\n--- Messages ---".into());
    for msg in messages {
        let role = format!("{:?}", msg.role);
        let content_preview: String = msg.content.chars().take(500).collect();
        let truncated = if msg.content.len() > 500 { "..." } else { "" };
        parts.push(format!("[{role}] {content_preview}{truncated}"));
    }

    // Include notable events
    let error_events: Vec<String> = thread
        .events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::ActionFailed { action_name, error, .. } => {
                Some(format!("Action '{action_name}' failed: {error}"))
            }
            EventKind::StepFailed { error, .. } => Some(format!("Step failed: {error}")),
            _ => None,
        })
        .collect();

    if !error_events.is_empty() {
        parts.push("\n--- Errors ---".into());
        for err in error_events {
            parts.push(err);
        }
    }

    parts.join("\n")
}

/// Produce a single MemoryDoc by asking the LLM to analyze the transcript.
async fn produce_doc(
    thread: &Thread,
    llm: &Arc<dyn LlmBackend>,
    doc_type: DocType,
    transcript: &str,
    prompt: &str,
) -> Result<(MemoryDoc, TokenUsage), EngineError> {
    let messages = vec![
        ThreadMessage::system(format!(
            "You are analyzing a completed agent thread. Here is the transcript:\n\n{transcript}"
        )),
        ThreadMessage::user(prompt.to_string()),
    ];

    let config = LlmCallConfig {
        force_text: true,
        ..LlmCallConfig::default()
    };

    let output = llm.complete(&messages, &[], &config).await?;

    let content = match output.response {
        LlmResponse::Text(t) => t,
        LlmResponse::ActionCalls { content, .. } | LlmResponse::Code { content, .. } => {
            content.unwrap_or_default()
        }
    };

    let title = match doc_type {
        DocType::Summary => format!("Summary: {}", thread.goal),
        DocType::Lesson => format!("Lessons: {}", thread.goal),
        DocType::Issue => format!("Issues: {}", thread.goal),
        DocType::Playbook => format!("Playbook: {}", thread.goal),
        DocType::Spec => format!("Spec: {}", thread.goal),
        DocType::Note => format!("Note: {}", thread.goal),
    };

    let doc = MemoryDoc::new(thread.project_id, doc_type, title, content)
        .with_source_thread(thread.id);

    Ok((doc, output.usage))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::llm::{LlmCallConfig, LlmOutput};
    use crate::types::capability::ActionDef;
    use crate::types::event::ThreadEvent;
    use crate::types::project::ProjectId;
    use crate::types::step::TokenUsage;
    use crate::types::thread::{ThreadConfig, ThreadType};
    use std::sync::Mutex;

    struct MockLlm {
        responses: Mutex<Vec<String>>,
    }

    impl MockLlm {
        fn with_responses(responses: Vec<&str>) -> Arc<dyn crate::traits::llm::LlmBackend> {
            Arc::new(Self {
                responses: Mutex::new(responses.into_iter().map(String::from).collect()),
            })
        }
    }

    #[async_trait::async_trait]
    impl crate::traits::llm::LlmBackend for MockLlm {
        async fn complete(
            &self,
            _: &[ThreadMessage],
            _: &[ActionDef],
            _: &LlmCallConfig,
        ) -> Result<LlmOutput, EngineError> {
            let mut r = self.responses.lock().unwrap();
            let text = if r.is_empty() {
                "mock response".to_string()
            } else {
                r.remove(0)
            };
            Ok(LlmOutput {
                response: LlmResponse::Text(text),
                usage: TokenUsage {
                    input_tokens: 100,
                    output_tokens: 50,
                    ..TokenUsage::default()
                },
            })
        }

        fn model_name(&self) -> &str {
            "mock"
        }
    }

    fn make_completed_thread() -> Thread {
        let mut thread = Thread::new(
            "test task",
            ThreadType::Foreground,
            ProjectId::new(),
            ThreadConfig::default(),
        );
        thread.state = crate::types::thread::ThreadState::Completed;
        thread
    }

    #[tokio::test]
    async fn reflect_produces_summary_for_clean_thread() {
        let thread = make_completed_thread();
        let llm = MockLlm::with_responses(vec!["Thread accomplished the test task successfully."]);

        let result = reflect(&thread, &llm).await.unwrap();
        assert_eq!(result.docs.len(), 1);
        assert_eq!(result.docs[0].doc_type, DocType::Summary);
    }

    #[tokio::test]
    async fn reflect_produces_lesson_on_errors() {
        let mut thread = make_completed_thread();
        thread.events.push(ThreadEvent::new(
            thread.id,
            EventKind::ActionFailed {
                step_id: crate::types::step::StepId::new(),
                action_name: "web_search".into(),
                call_id: String::new(),
                error: "Tool web_search not found".into(),
            },
        ));

        let llm = MockLlm::with_responses(vec![
            "Summary of thread with errors.",
            "Lesson: use web-search instead of web_search.",
            "Issue: web_search tool is missing.",
            "ALIAS: web_search -> web-search",
        ]);

        let result = reflect(&thread, &llm).await.unwrap();
        let types: Vec<DocType> = result.docs.iter().map(|d| d.doc_type).collect();
        assert!(types.contains(&DocType::Summary));
        assert!(types.contains(&DocType::Lesson));
        assert!(types.contains(&DocType::Issue));
        assert!(types.contains(&DocType::Spec));
    }

    #[tokio::test]
    async fn reflect_produces_spec_on_tool_not_found() {
        let mut thread = make_completed_thread();
        thread.events.push(ThreadEvent::new(
            thread.id,
            EventKind::ActionFailed {
                step_id: crate::types::step::StepId::new(),
                action_name: "missing_tool".into(),
                call_id: String::new(),
                error: "Tool missing_tool not found".into(),
            },
        ));

        let llm = MockLlm::with_responses(vec![
            "Summary.",
            "Lesson learned.",
            "Issues found.",
            "MISSING: missing_tool -> needs implementation",
        ]);

        let result = reflect(&thread, &llm).await.unwrap();
        let spec_docs: Vec<&MemoryDoc> = result
            .docs
            .iter()
            .filter(|d| d.doc_type == DocType::Spec)
            .collect();
        assert_eq!(spec_docs.len(), 1);
        assert!(spec_docs[0].content.contains("MISSING"));
    }

    #[tokio::test]
    async fn reflect_produces_playbook_on_successful_multi_step() {
        let mut thread = make_completed_thread();
        // Add 2+ action executed events to trigger playbook
        thread.events.push(ThreadEvent::new(
            thread.id,
            EventKind::ActionExecuted {
                step_id: crate::types::step::StepId::new(),
                action_name: "web-search".into(),
                call_id: String::new(),
                duration_ms: 100,
            },
        ));
        thread.events.push(ThreadEvent::new(
            thread.id,
            EventKind::ActionExecuted {
                step_id: crate::types::step::StepId::new(),
                action_name: "llm_query".into(),
                call_id: String::new(),
                duration_ms: 200,
            },
        ));

        let llm = MockLlm::with_responses(vec![
            "Summary of successful thread.",
            "1. Search web for topic\n2. Analyze results with llm_query\n3. Return summary",
        ]);

        let result = reflect(&thread, &llm).await.unwrap();
        let playbook_docs: Vec<&MemoryDoc> = result
            .docs
            .iter()
            .filter(|d| d.doc_type == DocType::Playbook)
            .collect();
        assert_eq!(playbook_docs.len(), 1);
        assert!(playbook_docs[0].title.starts_with("Playbook:"));
    }

    #[tokio::test]
    async fn reflect_skips_playbook_for_single_action() {
        let mut thread = make_completed_thread();
        // Only 1 action — not enough for a playbook
        thread.events.push(ThreadEvent::new(
            thread.id,
            EventKind::ActionExecuted {
                step_id: crate::types::step::StepId::new(),
                action_name: "echo".into(),
                call_id: String::new(),
                duration_ms: 5,
            },
        ));

        let llm = MockLlm::with_responses(vec!["Simple summary."]);

        let result = reflect(&thread, &llm).await.unwrap();
        let playbook_docs: Vec<&MemoryDoc> = result
            .docs
            .iter()
            .filter(|d| d.doc_type == DocType::Playbook)
            .collect();
        assert!(playbook_docs.is_empty());
    }
}

//! Reflection pipeline — produces structured knowledge from completed threads.
//!
//! After a thread completes, the reflection pipeline spawns a CodeAct thread
//! that uses reflection-specific tools (transcript inspection, memory queries,
//! tool registry checks) to produce structured MemoryDocs.
//!
//! The reflection thread runs with [`ThreadType::Reflection`] and its own
//! [`ExecutionLoop`], making it a fully recursive CodeAct agent.

use std::sync::Arc;

use tracing::{debug, warn};

use crate::capability::lease::LeaseManager;
use crate::capability::policy::PolicyEngine;
use crate::capability::registry::CapabilityRegistry;
use crate::executor::ExecutionLoop;
use crate::reflection::executor::{ReflectionExecutor, build_reflection_prompt};
use crate::runtime::messaging::{self, ThreadOutcome};
use crate::traits::llm::LlmBackend;
use crate::traits::store::Store;
use crate::types::error::EngineError;
use crate::types::event::EventKind;
use crate::types::memory::{DocType, MemoryDoc};
use crate::types::message::ThreadMessage;
use crate::types::step::TokenUsage;
use crate::types::thread::{Thread, ThreadConfig, ThreadType};

/// Result of running the reflection pipeline on a completed thread.
pub struct ReflectionResult {
    /// Memory docs produced by reflection.
    pub docs: Vec<MemoryDoc>,
    /// Total tokens used by reflection LLM calls.
    pub tokens_used: TokenUsage,
}

/// Run the reflection pipeline on a completed thread.
///
/// Spawns a CodeAct thread with reflection-specific tools that can:
/// - Read the completed thread's execution transcript
/// - Query existing knowledge in the project
/// - Verify tool names against the capability registry
///
/// The reflection thread produces structured findings via `FINAL()` which
/// are parsed into MemoryDocs.
pub async fn reflect(
    thread: &Thread,
    llm: &Arc<dyn LlmBackend>,
    store: &Arc<dyn Store>,
    capabilities: &Arc<CapabilityRegistry>,
) -> Result<ReflectionResult, EngineError> {
    let transcript = build_transcript(thread);

    // Build the reflection-specific effect executor
    let executor: Arc<dyn crate::traits::effect::EffectExecutor> =
        Arc::new(ReflectionExecutor::new(
            Arc::clone(store),
            Arc::clone(capabilities),
            transcript,
            thread.project_id,
        ));

    // Create a reflection thread
    let mut refl_thread = Thread::new(
        format!("Reflect on: {}", thread.goal),
        ThreadType::Reflection,
        thread.project_id,
        ThreadConfig {
            max_iterations: 10,
            enable_reflection: false, // no recursive reflection
            ..ThreadConfig::default()
        },
    );

    // Build and inject the reflection system prompt
    let actions = executor.available_actions(&[]).await?;
    let system_prompt = build_reflection_prompt(&actions, &thread.goal);
    refl_thread
        .messages
        .insert(0, ThreadMessage::system(system_prompt));
    refl_thread.add_message(ThreadMessage::user(format!(
        "Analyze the completed thread '{}' and produce structured findings.",
        thread.goal
    )));

    // Set up infrastructure for the reflection loop
    let lease_manager = Arc::new(LeaseManager::new());
    let policy = Arc::new(PolicyEngine::new());
    let (_signal_tx, signal_rx) = messaging::signal_channel(32);

    // Grant a blanket lease (empty granted_actions = all actions allowed)
    let lease = lease_manager
        .grant(refl_thread.id, "reflection_tools", vec![], None, None)
        .await;
    refl_thread.capability_leases.push(lease.id);
    store.save_thread(&refl_thread).await?;
    store.save_lease(&lease).await?;

    // Run the execution loop
    let mut exec_loop = ExecutionLoop::new(
        refl_thread,
        Arc::clone(llm),
        executor,
        lease_manager,
        policy,
        signal_rx,
        "system".to_string(),
    )
    .with_store(Arc::clone(store));

    let outcome = exec_loop.run().await?;

    // Parse the outcome into MemoryDocs
    let response = match outcome {
        ThreadOutcome::Completed { response: Some(r) } => r,
        ThreadOutcome::Completed { response: None } => String::new(),
        ThreadOutcome::Failed { error } => {
            warn!(
                thread_id = %thread.id,
                "reflection thread failed: {error}"
            );
            String::new()
        }
        _ => String::new(),
    };

    let docs = parse_reflection_output(&response, thread);
    let tokens_used = TokenUsage {
        input_tokens: exec_loop.thread.total_tokens_used,
        output_tokens: 0, // total already tracked
        ..TokenUsage::default()
    };

    debug!(
        thread_id = %thread.id,
        docs_produced = docs.len(),
        total_tokens = tokens_used.total(),
        "reflection complete (CodeAct)"
    );

    Ok(ReflectionResult { docs, tokens_used })
}

/// Run a simplified reflection pipeline using direct LLM calls.
///
/// This is a fallback for when CodeAct execution is not available or when
/// the reflection thread overhead is not desired (e.g., in tests).
pub async fn reflect_simple(
    thread: &Thread,
    llm: &Arc<dyn LlmBackend>,
) -> Result<ReflectionResult, EngineError> {
    let mut docs = Vec::new();
    let mut total_tokens = TokenUsage::default();
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
        if issue_doc.content.chars().count() > 20 {
            docs.push(issue_doc);
        }
        total_tokens.input_tokens += tokens.input_tokens;
        total_tokens.output_tokens += tokens.output_tokens;
    }

    // 4. Missing capabilities
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
        if spec_doc.content.chars().count() > 20 {
            docs.push(spec_doc);
        }
        total_tokens.input_tokens += tokens.input_tokens;
        total_tokens.output_tokens += tokens.output_tokens;
    }

    // 5. Playbook
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
        if playbook_doc.content.chars().count() > 20 {
            docs.push(playbook_doc);
        }
        total_tokens.input_tokens += tokens.input_tokens;
        total_tokens.output_tokens += tokens.output_tokens;
    }

    debug!(
        thread_id = %thread.id,
        docs_produced = docs.len(),
        total_tokens = total_tokens.total(),
        "reflection complete (simple)"
    );

    Ok(ReflectionResult {
        docs,
        tokens_used: total_tokens,
    })
}

// ── Output parsing ────────────────────────────────────────────

/// Parse the FINAL() output from a reflection CodeAct thread into MemoryDocs.
fn parse_reflection_output(response: &str, source_thread: &Thread) -> Vec<MemoryDoc> {
    // Try parsing as JSON first (the expected format)
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(response)
        && let Some(docs_arr) = value.get("docs").and_then(|d| d.as_array())
    {
        return docs_arr
            .iter()
            .filter_map(|doc_val| parse_doc_entry(doc_val, source_thread))
            .collect();
    }

    // If the response is not valid JSON, try to find JSON in the response
    if let Some(start) = response.find('{')
        && let Some(end) = response.rfind('}')
    {
        let json_str = &response[start..=end];
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str)
            && let Some(docs_arr) = value.get("docs").and_then(|d| d.as_array())
        {
            return docs_arr
                .iter()
                .filter_map(|doc_val| parse_doc_entry(doc_val, source_thread))
                .collect();
        }
    }

    // Fallback: treat the entire response as a summary
    if response.chars().count() > 20 {
        vec![
            MemoryDoc::new(
                source_thread.project_id,
                DocType::Summary,
                format!("Summary: {}", source_thread.goal),
                response,
            )
            .with_source_thread(source_thread.id),
        ]
    } else {
        vec![]
    }
}

/// Parse a single doc entry from the JSON output.
fn parse_doc_entry(value: &serde_json::Value, source_thread: &Thread) -> Option<MemoryDoc> {
    let doc_type_str = value.get("type")?.as_str()?;
    let title = value.get("title")?.as_str()?;
    let content = value.get("content")?.as_str()?;

    if content.chars().count() <= 20 {
        return None;
    }

    let doc_type = match doc_type_str.to_lowercase().as_str() {
        "summary" => DocType::Summary,
        "lesson" => DocType::Lesson,
        "issue" => DocType::Issue,
        "spec" => DocType::Spec,
        "playbook" => DocType::Playbook,
        "note" => DocType::Note,
        _ => return None,
    };

    Some(
        MemoryDoc::new(source_thread.project_id, doc_type, title, content)
            .with_source_thread(source_thread.id),
    )
}

// ── Prompts (for reflect_simple fallback) ─────────────────────

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

// ── Helpers ───────────────────────────────────────────────────

/// Build a concise transcript of the thread's work.
pub(crate) fn build_transcript(thread: &Thread) -> String {
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
        let truncated = if msg.content.chars().count() > 500 {
            "..."
        } else {
            ""
        };
        parts.push(format!("[{role}] {content_preview}{truncated}"));
    }

    // Include notable events
    let error_events: Vec<String> = thread
        .events
        .iter()
        .filter_map(|e| match &e.kind {
            EventKind::ActionFailed {
                action_name, error, ..
            } => Some(format!("Action '{action_name}' failed: {error}")),
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

    let config = crate::traits::llm::LlmCallConfig {
        force_text: true,
        ..crate::traits::llm::LlmCallConfig::default()
    };

    let output = llm.complete(&messages, &[], &config).await?;

    let content = match output.response {
        crate::types::step::LlmResponse::Text(t) => t,
        crate::types::step::LlmResponse::ActionCalls { content, .. }
        | crate::types::step::LlmResponse::Code { content, .. } => content.unwrap_or_default(),
    };

    let title = match doc_type {
        DocType::Summary => format!("Summary: {}", thread.goal),
        DocType::Lesson => format!("Lessons: {}", thread.goal),
        DocType::Issue => format!("Issues: {}", thread.goal),
        DocType::Playbook => format!("Playbook: {}", thread.goal),
        DocType::Spec => format!("Spec: {}", thread.goal),
        DocType::Note => format!("Note: {}", thread.goal),
    };

    let doc =
        MemoryDoc::new(thread.project_id, doc_type, title, content).with_source_thread(thread.id);

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
    use crate::types::thread::ThreadConfig;
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
                response: crate::types::step::LlmResponse::Text(text),
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

    // ── reflect_simple tests (direct LLM calls) ────────────────

    #[tokio::test]
    async fn reflect_simple_produces_summary() {
        let thread = make_completed_thread();
        let llm = MockLlm::with_responses(vec!["Thread accomplished the test task successfully."]);

        let result = reflect_simple(&thread, &llm).await.unwrap();
        assert_eq!(result.docs.len(), 1);
        assert_eq!(result.docs[0].doc_type, DocType::Summary);
    }

    #[tokio::test]
    async fn reflect_simple_produces_lesson_on_errors() {
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

        let result = reflect_simple(&thread, &llm).await.unwrap();
        let types: Vec<DocType> = result.docs.iter().map(|d| d.doc_type).collect();
        assert!(types.contains(&DocType::Summary));
        assert!(types.contains(&DocType::Lesson));
        assert!(types.contains(&DocType::Issue));
        assert!(types.contains(&DocType::Spec));
    }

    #[tokio::test]
    async fn reflect_simple_produces_spec_on_tool_not_found() {
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

        let result = reflect_simple(&thread, &llm).await.unwrap();
        let spec_docs: Vec<&MemoryDoc> = result
            .docs
            .iter()
            .filter(|d| d.doc_type == DocType::Spec)
            .collect();
        assert_eq!(spec_docs.len(), 1);
        assert!(spec_docs[0].content.contains("MISSING"));
    }

    #[tokio::test]
    async fn reflect_simple_produces_playbook_on_multi_step() {
        let mut thread = make_completed_thread();
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
            "1. Search web\n2. Analyze results\n3. Return summary",
        ]);

        let result = reflect_simple(&thread, &llm).await.unwrap();
        assert!(result.docs.iter().any(|d| d.doc_type == DocType::Playbook));
    }

    #[tokio::test]
    async fn reflect_simple_skips_playbook_for_single_action() {
        let mut thread = make_completed_thread();
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

        let result = reflect_simple(&thread, &llm).await.unwrap();
        assert!(!result.docs.iter().any(|d| d.doc_type == DocType::Playbook));
    }

    // ── parse_reflection_output tests ──────────────────────────

    #[test]
    fn parse_valid_json_output() {
        let thread = make_completed_thread();
        let json = r#"{"docs": [
            {"type": "summary", "title": "Summary: test", "content": "The thread completed successfully with good results."},
            {"type": "lesson", "title": "Lesson: test", "content": "Always check tool names before calling them."}
        ]}"#;

        let docs = parse_reflection_output(json, &thread);
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].doc_type, DocType::Summary);
        assert_eq!(docs[1].doc_type, DocType::Lesson);
    }

    #[test]
    fn parse_json_embedded_in_text() {
        let thread = make_completed_thread();
        let text = r#"Here are my findings: {"docs": [{"type": "summary", "title": "test", "content": "The thread did something interesting and useful."}]} end"#;

        let docs = parse_reflection_output(text, &thread);
        assert_eq!(docs.len(), 1);
    }

    #[test]
    fn parse_fallback_to_summary() {
        let thread = make_completed_thread();
        let text = "This is a plain text response with enough content to be a valid summary doc.";

        let docs = parse_reflection_output(text, &thread);
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].doc_type, DocType::Summary);
    }

    #[test]
    fn parse_skips_short_content() {
        let thread = make_completed_thread();
        let json = r#"{"docs": [{"type": "summary", "title": "test", "content": "too short"}]}"#;

        let docs = parse_reflection_output(json, &thread);
        assert!(docs.is_empty());
    }

    #[test]
    fn parse_skips_unknown_doc_type() {
        let thread = make_completed_thread();
        let json = r#"{"docs": [{"type": "unknown_type", "title": "test", "content": "This has enough content but unknown type so it gets skipped."}]}"#;

        let docs = parse_reflection_output(json, &thread);
        assert!(docs.is_empty());
    }
}

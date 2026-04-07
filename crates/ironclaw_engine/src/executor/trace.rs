//! Execution trace recording and analysis.
//!
//! Records full execution traces to JSON files for debugging. Optionally
//! runs a post-execution analysis to detect common issues.
//!
//! Enable with `ENGINE_V2_TRACE=1` env var. Traces are written to
//! `engine_trace_{timestamp}.json` in the current directory.

use std::path::PathBuf;

use chrono::Utc;
use serde::Serialize;
use tracing::debug;

use crate::types::event::ThreadEvent;
use crate::types::thread::{Thread, ThreadId, ThreadState};

/// Check if trace recording is enabled.
pub fn is_trace_enabled() -> bool {
    std::env::var("ENGINE_V2_TRACE")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
}

/// A complete execution trace for a single thread.
#[derive(Debug, Serialize)]
pub struct ExecutionTrace {
    pub thread_id: ThreadId,
    pub goal: String,
    pub final_state: ThreadState,
    pub step_count: usize,
    pub total_tokens: u64,
    pub messages: Vec<MessageRecord>,
    pub events: Vec<ThreadEvent>,
    pub issues: Vec<TraceIssue>,
    pub timestamp: chrono::DateTime<Utc>,
}

/// A single doc record, for the trace.
#[derive(Debug, Serialize)]
pub struct DocRecord {
    pub doc_type: String,
    pub title: String,
    pub content: String,
}

/// A message in the trace with role labeling.
#[derive(Debug, Serialize)]
pub struct MessageRecord {
    pub role: String,
    pub content_length: usize,
    pub content_preview: String,
    pub full_content: String,
    pub action_name: Option<String>,
    pub action_call_id: Option<String>,
}

/// An issue detected by the retrospective analyzer.
#[derive(Debug, Serialize)]
pub struct TraceIssue {
    pub severity: IssueSeverity,
    pub category: String,
    pub description: String,
    pub step: Option<usize>,
}

#[derive(Debug, PartialEq, Serialize)]
pub enum IssueSeverity {
    Error,
    Warning,
    Info,
}

/// Build a trace from a completed thread.
pub fn build_trace(thread: &Thread) -> ExecutionTrace {
    let messages: Vec<MessageRecord> = thread
        .messages
        .iter()
        .map(|m| {
            let preview: String = m.content.chars().take(300).collect();
            MessageRecord {
                role: format!("{:?}", m.role),
                content_length: m.content.chars().count(),
                content_preview: if m.content.chars().count() > 300 {
                    format!("{preview}...")
                } else {
                    preview
                },
                full_content: m.content.clone(),
                action_name: m.action_name.clone(),
                action_call_id: m.action_call_id.clone(),
            }
        })
        .collect();

    let issues = analyze_trace(thread);

    ExecutionTrace {
        thread_id: thread.id,
        goal: thread.goal.clone(),
        final_state: thread.state,
        step_count: thread.step_count,
        total_tokens: thread.total_tokens_used,
        messages,
        events: thread.events.clone(),
        issues,
        timestamp: Utc::now(),
    }
}

/// Write a trace to a JSON file.
pub fn write_trace(trace: &ExecutionTrace) -> Option<PathBuf> {
    let filename = format!("engine_trace_{}.json", Utc::now().format("%Y%m%dT%H%M%S"));
    let path = PathBuf::from(&filename);

    match serde_json::to_string_pretty(trace) {
        Ok(json) => match std::fs::write(&path, json) {
            Ok(()) => {
                debug!(path = %path.display(), "Execution trace written");
                Some(path)
            }
            Err(e) => {
                debug!("Failed to write trace: {e}");
                None
            }
        },
        Err(e) => {
            debug!("Failed to serialize trace: {e}");
            None
        }
    }
}

/// Print a summary of the trace to the log.
pub fn log_trace_summary(trace: &ExecutionTrace) {
    debug!(
        thread_id = %trace.thread_id,
        goal = %trace.goal,
        state = ?trace.final_state,
        steps = trace.step_count,
        tokens = trace.total_tokens,
        messages = trace.messages.len(),
        events = trace.events.len(),
        issues = trace.issues.len(),
        "=== Engine V2 Trace Summary ==="
    );

    for issue in &trace.issues {
        match issue.severity {
            IssueSeverity::Error => debug!(
                category = %issue.category,
                step = ?issue.step,
                "ISSUE: {}",
                issue.description
            ),
            IssueSeverity::Warning => debug!(
                category = %issue.category,
                step = ?issue.step,
                "WARNING: {}",
                issue.description
            ),
            IssueSeverity::Info => debug!(
                category = %issue.category,
                step = ?issue.step,
                "NOTE: {}",
                issue.description
            ),
        }
    }
}

// ── Retrospective analysis ──────────────────────────────────

/// Analyze a completed thread for common issues.
fn analyze_trace(thread: &Thread) -> Vec<TraceIssue> {
    let mut issues = Vec::new();

    // 1. Check if the thread failed
    if thread.state == ThreadState::Failed {
        issues.push(TraceIssue {
            severity: IssueSeverity::Error,
            category: "thread_failure".into(),
            description: "Thread ended in Failed state".into(),
            step: None,
        });
    }

    // 2. Check for empty response (no FINAL, no useful output)
    let has_assistant_response = thread
        .messages
        .iter()
        .any(|m| m.role == crate::types::message::MessageRole::Assistant && !m.content.is_empty());
    if !has_assistant_response {
        issues.push(TraceIssue {
            severity: IssueSeverity::Warning,
            category: "no_response".into(),
            description: "No assistant message in thread — model may not have generated output"
                .into(),
            step: None,
        });
    }

    // 3. Check for tool errors
    let tool_errors: Vec<&ThreadEvent> = thread
        .events
        .iter()
        .filter(|e| matches!(e.kind, crate::types::event::EventKind::ActionFailed { .. }))
        .collect();
    if !tool_errors.is_empty() {
        for event in &tool_errors {
            if let crate::types::event::EventKind::ActionFailed {
                action_name, error, ..
            } = &event.kind
            {
                issues.push(TraceIssue {
                    severity: IssueSeverity::Warning,
                    category: "tool_error".into(),
                    description: format!("Tool '{action_name}' failed: {error}"),
                    step: None,
                });
            }
        }
    }

    // 4. Check for code execution errors in output messages.
    // Code output appears as User-role messages (Monty stdout/stderr) with
    // prefixes like "[stdout]" or "[stderr]". Skip the System prompt (index 0)
    // and Assistant messages to avoid false positives from example text.
    let error_patterns = [
        "NameError",
        "SyntaxError",
        "TypeError",
        "NotImplementedError",
    ];
    for (i, msg) in thread.messages.iter().enumerate() {
        let is_code_output = msg.role == crate::types::message::MessageRole::User
            && (msg.content.starts_with("[stdout]")
                || msg.content.starts_with("[stderr]")
                || msg.content.starts_with("[code ")
                || msg.content.starts_with("Traceback"));
        if is_code_output && error_patterns.iter().any(|p| msg.content.contains(p)) {
            let preview: String = msg.content.chars().take(200).collect();
            issues.push(TraceIssue {
                severity: IssueSeverity::Warning,
                category: "code_error".into(),
                description: format!("Code execution error in message {i}: {preview}"),
                step: None,
            });
        }
    }

    // 5. Check for empty call_id on ActionResult messages (causes LLM API rejection).
    for (i, msg) in thread.messages.iter().enumerate() {
        if msg.role == crate::types::message::MessageRole::ActionResult {
            let call_id_empty = msg.action_call_id.as_ref().is_none_or(|id| id.is_empty());
            if call_id_empty {
                let name = msg.action_name.as_deref().unwrap_or("unknown");
                issues.push(TraceIssue {
                    severity: IssueSeverity::Error,
                    category: "empty_call_id".into(),
                    description: format!(
                        "ActionResult message {i} (tool '{name}') has empty call_id — will cause LLM API rejection"
                    ),
                    step: None,
                });
            }
        }
    }

    // 6. Check for model ignoring tool results (hallucination risk).
    // In Tier 0 (structured), results appear as ActionResult messages.
    // In Tier 1 (CodeAct), results appear as User messages with "[tool result]" prefixes.
    let has_tool_results = thread
        .messages
        .iter()
        .any(|m| m.role == crate::types::message::MessageRole::ActionResult);
    let has_tool_output_in_context = thread.messages.iter().any(|m| {
        m.role == crate::types::message::MessageRole::User
            && (m.content.contains(" result]") || m.content.contains(" error]"))
    });
    if has_tool_results && !has_tool_output_in_context {
        issues.push(TraceIssue {
            severity: IssueSeverity::Warning,
            category: "missing_tool_output".into(),
            description:
                "Tool results exist but no tool output in messages — model may not see tool results"
                    .into(),
            step: None,
        });
    }

    // 7. Check for excessive iterations
    if thread.step_count > 10 {
        issues.push(TraceIssue {
            severity: IssueSeverity::Warning,
            category: "excessive_steps".into(),
            description: format!(
                "Thread took {} steps — may be stuck in a loop",
                thread.step_count
            ),
            step: None,
        });
    }

    // 8. Check for text response without FINAL (model answered from memory)
    let text_without_code = thread.events.iter().all(|e| {
        !matches!(
            e.kind,
            crate::types::event::EventKind::ActionExecuted { .. }
        )
    });
    if text_without_code && thread.step_count == 1 && has_assistant_response {
        issues.push(TraceIssue {
            severity: IssueSeverity::Info,
            category: "no_tools_used".into(),
            description: "Model answered in one step without using any tools — may be answering from training data".into(),
            step: Some(1),
        });
    }

    // 9. Check for LLM not producing code blocks
    let code_steps = thread
        .events
        .iter()
        .filter(|e| matches!(e.kind, crate::types::event::EventKind::StepStarted { .. }))
        .count();
    let text_responses_without_code = thread
        .messages
        .iter()
        .filter(|m| {
            m.role == crate::types::message::MessageRole::Assistant
                && !m.content.contains("```")
                && !m.content.contains("FINAL(")
        })
        .count();
    if text_responses_without_code > 0 && code_steps > 0 {
        issues.push(TraceIssue {
            severity: IssueSeverity::Info,
            category: "mixed_mode".into(),
            description: format!(
                "{text_responses_without_code} text response(s) without code blocks — model may not be following CodeAct prompt"
            ),
            step: None,
        });
    }

    // 10. Extract failure reason from StateChanged → Failed events
    for event in &thread.events {
        if let crate::types::event::EventKind::StateChanged {
            to: ThreadState::Failed,
            reason: Some(reason),
            ..
        } = &event.kind
        {
            if reason.contains("LLM") || reason.contains("Provider") {
                issues.push(TraceIssue {
                    severity: IssueSeverity::Error,
                    category: "llm_error".into(),
                    description: format!("LLM provider error: {}", truncate(reason, 300)),
                    step: None,
                });
            } else if reason.contains("orchestrator") {
                issues.push(TraceIssue {
                    severity: IssueSeverity::Error,
                    category: "orchestrator_error".into(),
                    description: format!("Orchestrator error: {}", truncate(reason, 300)),
                    step: None,
                });
            }
        }
    }

    issues
}

fn truncate(s: &str, max_chars: usize) -> String {
    let chars: String = s.chars().take(max_chars).collect();
    if s.chars().count() > max_chars {
        format!("{chars}...")
    } else {
        chars
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::event::EventKind;
    use crate::types::message::ThreadMessage;
    use crate::types::project::ProjectId;
    use crate::types::step::StepId;
    use crate::types::thread::{ThreadConfig, ThreadType};

    fn make_thread() -> Thread {
        Thread::new(
            "test goal",
            ThreadType::Foreground,
            ProjectId::new(),
            "test-user",
            ThreadConfig::default(),
        )
    }

    // ── empty_call_id detection (OpenAI / Codex rejection) ───

    /// OpenAI and Codex reject ActionResult messages with empty call_id.
    /// The trace analyzer must flag these as errors.
    #[test]
    fn detects_empty_call_id_on_action_result() {
        let mut thread = make_thread();
        thread.add_message(ThreadMessage::system("sys"));
        thread.add_message(ThreadMessage::assistant("calling tool"));
        // Simulate the bug: empty call_id
        thread.add_message(ThreadMessage::action_result("", "web_search", "result"));

        let issues = analyze_trace(&thread);
        let empty_id_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.category == "empty_call_id")
            .collect();

        assert_eq!(empty_id_issues.len(), 1);
        assert_eq!(empty_id_issues[0].severity, IssueSeverity::Error);
        assert!(empty_id_issues[0].description.contains("web_search"));
    }

    /// ActionResult with None call_id should also be flagged.
    #[test]
    fn detects_none_call_id_on_action_result() {
        let mut thread = make_thread();
        thread.add_message(ThreadMessage::system("sys"));
        thread.add_message(ThreadMessage::assistant("calling tool"));
        // Manually construct a message with None call_id
        thread.add_message(ThreadMessage {
            role: crate::types::message::MessageRole::ActionResult,
            content: "result".into(),
            provenance: crate::types::provenance::Provenance::ToolOutput {
                action_name: "shell".into(),
            },
            action_call_id: None,
            action_name: Some("shell".into()),
            action_calls: None,
            timestamp: chrono::Utc::now(),
        });

        let issues = analyze_trace(&thread);
        assert!(issues.iter().any(|i| i.category == "empty_call_id"));
    }

    /// No false positive: valid call_id should not be flagged.
    #[test]
    fn no_false_positive_for_valid_call_id() {
        let mut thread = make_thread();
        thread.add_message(ThreadMessage::system("sys"));
        thread.add_message(ThreadMessage::assistant("calling tool"));
        thread.add_message(ThreadMessage::action_result(
            "call_abc123",
            "web_search",
            "result",
        ));

        let issues = analyze_trace(&thread);
        assert!(
            !issues.iter().any(|i| i.category == "empty_call_id"),
            "valid call_id should not be flagged"
        );
    }

    // ── tool_error detection ─────────────────────────────────

    /// ActionFailed events should produce tool_error warnings.
    #[test]
    fn detects_tool_failures_in_events() {
        let mut thread = make_thread();
        thread.add_message(ThreadMessage::system("sys"));
        thread.add_message(ThreadMessage::assistant("ok"));
        thread.events.push(ThreadEvent::new(
            thread.id,
            EventKind::ActionFailed {
                step_id: StepId::new(),
                action_name: "web_search".into(),
                call_id: "call_123".into(),
                error: "No lease for action 'web_search'".into(),
                params_summary: None,
            },
        ));

        let issues = analyze_trace(&thread);
        let tool_errors: Vec<_> = issues
            .iter()
            .filter(|i| i.category == "tool_error")
            .collect();
        assert_eq!(tool_errors.len(), 1);
        assert!(tool_errors[0].description.contains("web_search"));
    }

    // ── thread_failure detection ─────────────────────────────

    #[test]
    fn detects_failed_thread_state() {
        let mut thread = make_thread();
        thread.add_message(ThreadMessage::system("sys"));
        thread.add_message(ThreadMessage::assistant("trying"));
        thread.state = ThreadState::Failed;

        let issues = analyze_trace(&thread);
        assert!(issues.iter().any(|i| i.category == "thread_failure"));
    }

    // ── LLM error detection from StateChanged events ─────────

    /// Reproduces the exact pattern from the trace: OpenAI rejects empty call_id.
    #[test]
    fn detects_llm_error_from_state_changed() {
        let mut thread = make_thread();
        thread.add_message(ThreadMessage::system("sys"));
        thread.add_message(ThreadMessage::assistant("ok"));
        thread.state = ThreadState::Failed;
        thread.events.push(ThreadEvent::new(
            thread.id,
            EventKind::StateChanged {
                from: ThreadState::Running,
                to: ThreadState::Failed,
                reason: Some(
                    "LLM error: Provider openai_codex request failed: HTTP 400 Bad Request: \
                     Invalid 'input[5].call_id': empty string"
                        .into(),
                ),
            },
        ));

        let issues = analyze_trace(&thread);
        assert!(
            issues.iter().any(|i| i.category == "llm_error"),
            "should detect LLM provider error in StateChanged reason"
        );
    }

    // ── Multiple empty call_ids ──────────────────────────────

    /// Anthropic sends consecutive tool results merged into one User message.
    /// If multiple ActionResults have empty call_ids, each must be flagged.
    #[test]
    fn flags_each_empty_call_id_separately() {
        let mut thread = make_thread();
        thread.add_message(ThreadMessage::system("sys"));
        thread.add_message(ThreadMessage::assistant("parallel calls"));
        thread.add_message(ThreadMessage::action_result("", "tool_a", "result_a"));
        thread.add_message(ThreadMessage::action_result("", "tool_b", "result_b"));
        thread.add_message(ThreadMessage::action_result(
            "call_ok", "tool_c", "result_c",
        ));

        let issues = analyze_trace(&thread);
        let empty_issues: Vec<_> = issues
            .iter()
            .filter(|i| i.category == "empty_call_id")
            .collect();
        assert_eq!(
            empty_issues.len(),
            2,
            "should flag exactly the 2 empty call_ids"
        );
    }

    #[test]
    fn trace_serializes_approval_request_payload() {
        let mut thread = make_thread();
        thread.add_message(ThreadMessage::system("sys"));
        thread.add_message(ThreadMessage::assistant("installing notion"));
        thread.events.push(ThreadEvent::new(
            thread.id,
            EventKind::ApprovalRequested {
                action_name: "tool_install".into(),
                call_id: "call_install_1".into(),
                parameters: Some(serde_json::json!({"name": "notion", "kind": "mcp_server"})),
                description: Some("Install an extension".into()),
                allow_always: Some(true),
                gate_name: Some("approval".into()),
                params_summary: Some("notion".into()),
            },
        ));

        let trace = build_trace(&thread);
        match &trace.events[0].kind {
            EventKind::ApprovalRequested {
                action_name,
                call_id,
                parameters,
                description,
                allow_always,
                gate_name,
                params_summary,
            } => {
                assert_eq!(action_name, "tool_install");
                assert_eq!(call_id, "call_install_1");
                assert_eq!(
                    parameters.as_ref().and_then(|p| p.get("name")),
                    Some(&serde_json::json!("notion"))
                );
                assert_eq!(description.as_deref(), Some("Install an extension"));
                assert_eq!(*allow_always, Some(true));
                assert_eq!(gate_name.as_deref(), Some("approval"));
                assert_eq!(params_summary.as_deref(), Some("notion"));
            }
            other => panic!("unexpected event kind: {other:?}"),
        }

        let json = serde_json::to_string(&trace).expect("trace serializes");
        assert!(json.contains("\"ApprovalRequested\""));
        assert!(json.contains("\"action_name\":\"tool_install\""));
        assert!(json.contains("\"call_id\":\"call_install_1\""));
        assert!(json.contains("\"parameters\":{\"name\":\"notion\",\"kind\":\"mcp_server\"}"));
        assert!(json.contains("\"description\":\"Install an extension\""));
        assert!(json.contains("\"allow_always\":true"));
        assert!(json.contains("\"gate_name\":\"approval\""));
        assert!(json.contains("\"params_summary\":\"notion\""));
    }
}

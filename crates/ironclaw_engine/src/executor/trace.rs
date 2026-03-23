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
use tracing::{info, warn};

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

#[derive(Debug, Serialize)]
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
                content_length: m.content.len(),
                content_preview: if m.content.len() > 300 {
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
    let filename = format!(
        "engine_trace_{}.json",
        Utc::now().format("%Y%m%dT%H%M%S")
    );
    let path = PathBuf::from(&filename);

    match serde_json::to_string_pretty(trace) {
        Ok(json) => match std::fs::write(&path, json) {
            Ok(()) => {
                info!(path = %path.display(), "Execution trace written");
                Some(path)
            }
            Err(e) => {
                warn!("Failed to write trace: {e}");
                None
            }
        },
        Err(e) => {
            warn!("Failed to serialize trace: {e}");
            None
        }
    }
}

/// Print a summary of the trace to the log.
pub fn log_trace_summary(trace: &ExecutionTrace) {
    info!(
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
            IssueSeverity::Error => warn!(
                category = %issue.category,
                step = ?issue.step,
                "ISSUE: {}",
                issue.description
            ),
            IssueSeverity::Warning => warn!(
                category = %issue.category,
                step = ?issue.step,
                "WARNING: {}",
                issue.description
            ),
            IssueSeverity::Info => info!(
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
            description: "No assistant message in thread — model may not have generated output".into(),
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
                action_name,
                error,
                ..
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

    // 4. Check for code execution errors (NameError, etc. in messages)
    for (i, msg) in thread.messages.iter().enumerate() {
        if msg.role == crate::types::message::MessageRole::System
            && (msg.content.contains("NameError")
                || msg.content.contains("SyntaxError")
                || msg.content.contains("TypeError")
                || msg.content.contains("Error:"))
        {
            let preview: String = msg.content.chars().take(200).collect();
            issues.push(TraceIssue {
                severity: IssueSeverity::Warning,
                category: "code_error".into(),
                description: format!("Code error in message {i}: {preview}"),
                step: None,
            });
        }
    }

    // 5. Check for model ignoring tool results (hallucination risk)
    let has_tool_results = thread
        .messages
        .iter()
        .any(|m| m.role == crate::types::message::MessageRole::ActionResult);
    let has_tool_result_system_msg = thread
        .messages
        .iter()
        .any(|m| {
            m.role == crate::types::message::MessageRole::System
                && m.content.contains("[") && m.content.contains("result]")
        });
    if has_tool_results && !has_tool_result_system_msg {
        issues.push(TraceIssue {
            severity: IssueSeverity::Warning,
            category: "missing_tool_output".into(),
            description: "Tool results exist but no tool output in system messages — model may not see tool results".into(),
            step: None,
        });
    }

    // 6. Check for excessive iterations
    if thread.step_count > 10 {
        issues.push(TraceIssue {
            severity: IssueSeverity::Warning,
            category: "excessive_steps".into(),
            description: format!("Thread took {} steps — may be stuck in a loop", thread.step_count),
            step: None,
        });
    }

    // 7. Check for text response without FINAL (model answered from memory)
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

    // 8. Check for LLM not producing code blocks
    let code_steps = thread
        .events
        .iter()
        .filter(|e| {
            matches!(
                e.kind,
                crate::types::event::EventKind::StepStarted { .. }
            )
        })
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

    issues
}

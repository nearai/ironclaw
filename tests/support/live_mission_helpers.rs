//! Shared helpers for live tests that drive engine v2 missions/routines.
//!
//! Extracted from `tests/e2e_live_routine.rs` so additional scenarios
//! (`tests/e2e_live_mission_gmail.rs`, etc.) can reuse the same approval
//! responder and notification heuristics without copy-paste drift.

#![allow(dead_code)] // shared API; not every test uses every helper

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ironclaw::channels::{IncomingMessage, StatusUpdate};
use uuid::Uuid;

use crate::support::test_channel::TestChannel;
use crate::support::test_rig::TestRig;

/// Returns true if `tool` is the bare action name `expected` or carries
/// the same name with a parenthesised argument suffix
/// (e.g. `routine_create(name)` from `format_action_display_name`).
pub fn tool_is(tool: &str, expected: &str) -> bool {
    tool == expected
        || tool
            .strip_prefix(expected)
            .is_some_and(|rest| rest.starts_with('('))
}

/// Anchor for "the routine/mission actually fired and delivered output via
/// the notification channel" — independent of the formatting quality of
/// that output. Engine v2 mission fires (which is what `routine_fire`
/// becomes via the bridge alias) wrap their notifications with
/// `**[<mission-name>]**` so the channel can distinguish the fire output
/// from the agent's foreground reply.
pub fn looks_like_routine_notification(text: &str) -> bool {
    if let Some(open) = text.find("**[")
        && let Some(close_rel) = text[open + 3..].find("]**")
    {
        // Reject empty names (`**[]**`) — that's not a real marker.
        return close_rel > 0;
    }
    false
}

/// Wait until at least one captured response satisfies `predicate`,
/// polling every 500ms until `deadline`. Returns the first match.
pub async fn wait_for_response_matching<F>(
    rig: &TestRig,
    predicate: F,
    deadline: Instant,
) -> Option<String>
where
    F: Fn(&str) -> bool,
{
    loop {
        let responses = rig.wait_for_responses(0, Duration::from_millis(0)).await;
        if let Some(r) = responses.iter().find(|r| predicate(&r.content)) {
            return Some(r.content.clone());
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Background approval auto-responder.
///
/// Polls the `TestChannel`'s captured status events every 500ms and
/// resolves each `StatusUpdate::ApprovalNeeded` it has not already seen
/// by injecting a `Submission::ExecApproval { approved: true,
/// always: false }` back into the channel. Per-call approval is
/// deliberate — every fire round-trips through the gate so a regression
/// that breaks the second gate (e.g. session-state leakage between
/// fires) shows up.
///
/// Holds an `Arc<TestChannel>` rather than `&TestRig` so it can outlive
/// any individual borrow on the rig.
pub struct ApprovalAutoResponder {
    approved: Arc<tokio::sync::Mutex<Vec<(String, Uuid)>>>,
    handle: tokio::task::JoinHandle<()>,
}

impl ApprovalAutoResponder {
    pub fn spawn(channel: Arc<TestChannel>) -> Self {
        let approved = Arc::new(tokio::sync::Mutex::new(Vec::<(String, Uuid)>::new()));
        let approved_for_task = approved.clone();
        let handle = tokio::spawn(async move {
            let mut seen: HashSet<Uuid> = HashSet::new();
            loop {
                for event in channel.captured_status_events() {
                    if let StatusUpdate::ApprovalNeeded {
                        request_id,
                        tool_name,
                        ..
                    } = event
                    {
                        let Ok(rid) = Uuid::parse_str(&request_id) else {
                            continue;
                        };
                        if seen.insert(rid) {
                            eprintln!(
                                "[ApprovalAutoResponder] Auto-approving '{tool_name}' \
                                 (request_id={rid})"
                            );
                            let submission =
                                ironclaw::agent::submission::Submission::ExecApproval {
                                    request_id: rid,
                                    approved: true,
                                    always: false,
                                };
                            let msg =
                                IncomingMessage::new(channel.channel_name(), channel.user_id(), "")
                                    .with_structured_submission(submission);
                            channel.send_incoming(msg).await;
                            approved_for_task.lock().await.push((tool_name, rid));
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
        Self { approved, handle }
    }

    pub async fn approved_tools(&self) -> Vec<(String, Uuid)> {
        self.approved.lock().await.clone()
    }

    pub fn shutdown(self) {
        self.handle.abort();
    }
}

impl Drop for ApprovalAutoResponder {
    fn drop(&mut self) {
        // Defensive abort: if the test panics or returns before
        // calling `shutdown()`, the background task would otherwise
        // keep polling the channel for the lifetime of the test
        // process and bleed into subsequent tests on the same tokio
        // runtime. Calling `abort()` here is idempotent — a task
        // already aborted by an explicit `shutdown()` call simply
        // ignores it.
        self.handle.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_routine_notification_accepts_marker() {
        assert!(looks_like_routine_notification(
            "**[bitcoin_price_checker]** Some output\nbody body body"
        ));
    }

    #[test]
    fn looks_like_routine_notification_rejects_foreground_reply() {
        // Foreground replies have no marker.
        assert!(!looks_like_routine_notification(
            "## Bitcoin Price Checker Routine Created ✅\nSchedule: */5 * * * *"
        ));
    }

    #[test]
    fn looks_like_routine_notification_rejects_empty_marker() {
        assert!(!looks_like_routine_notification("**[]** empty name"));
    }

    #[test]
    fn tool_is_matches_bare_and_parenthesised() {
        assert!(tool_is("mission_fire", "mission_fire"));
        assert!(tool_is("mission_fire(abc)", "mission_fire"));
        assert!(!tool_is("mission_fired", "mission_fire"));
        assert!(!tool_is("other_mission_fire", "mission_fire"));
    }
}

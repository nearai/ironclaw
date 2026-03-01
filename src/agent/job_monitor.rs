//! Background job monitor that forwards Claude Code output to the main agent loop.
//!
//! When the main agent kicks off a sandbox job (especially Claude Code), this
//! monitor subscribes to the broadcast event channel and injects relevant
//! assistant messages back into the channel manager's stream. This lets the
//! main agent see what the sub-agent is producing and surface it to the user.
//!
//! ```text
//!   Container ──NDJSON──► Orchestrator ──broadcast──► JobMonitor
//!                                                        │
//!                                                  inject_tx (mpsc)
//!                                                        │
//!                                                        ▼
//!                                                   Agent Loop
//! ```

use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::channels::IncomingMessage;
use crate::channels::web::types::SseEvent;

/// Spawn a background task that watches for events from a specific job and
/// injects assistant messages into the agent loop.
///
/// The monitor forwards:
/// - `SseEvent::JobMessage` (assistant role): injected as incoming messages so
///   the main agent can read and relay to the user.
/// - `SseEvent::JobResult`: injected as a completion notice, then the task exits.
///
/// Tool use/result and status events are intentionally skipped (too noisy for
/// the main agent's context window).
pub fn spawn_job_monitor(
    job_id: Uuid,
    mut event_rx: broadcast::Receiver<(Uuid, String, SseEvent)>,
    inject_tx: mpsc::Sender<IncomingMessage>,
) -> JoinHandle<()> {
    let short_id = job_id.to_string()[..8].to_string();

    tokio::spawn(async move {
        tracing::info!(job_id = %short_id, "Job monitor started successfully");

        loop {
            match event_rx.recv().await {
                Ok((ev_job_id, _user_id, event)) => {
                    if ev_job_id != job_id {
                        continue;
                    }

                    match event {
                        SseEvent::JobMessage { role, content, .. } if role == "assistant" => {
                            let msg = IncomingMessage::new(
                                "job_monitor",
                                "system",
                                format!("[Job {}] Claude Code: {}", short_id, content),
                            );
                            if inject_tx.send(msg).await.is_err() {
                                tracing::debug!(
                                    job_id = %short_id,
                                    "Inject channel closed, stopping monitor"
                                );
                                break;
                            }
                        }
                        SseEvent::JobResult { status, .. } => {
                            let msg = IncomingMessage::new(
                                "job_monitor",
                                "system",
                                format!(
                                    "[Job {}] Container finished (status: {})",
                                    short_id, status
                                ),
                            );
                            let _ = inject_tx.send(msg).await;
                            tracing::debug!(
                                job_id = %short_id,
                                status = %status,
                                "Job monitor exiting (job finished)"
                            );
                            break;
                        }
                        _ => {
                            // Skip tool_use, tool_result, status events
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(
                        job_id = %short_id,
                        skipped = n,
                        "Job monitor lagged, some events were dropped"
                    );
                }
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::debug!(
                        job_id = %short_id,
                        "Broadcast channel closed, stopping monitor"
                    );
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_monitor_forwards_assistant_messages() {
        let (event_tx, _) = broadcast::channel::<(Uuid, String, SseEvent)>(16);
        let (inject_tx, mut inject_rx) = mpsc::channel::<IncomingMessage>(16);

        let job_id = Uuid::new_v4();
        let _handle = spawn_job_monitor(job_id, event_tx.subscribe(), inject_tx);

        // Send an assistant message
        event_tx
            .send((
                job_id,
                "test-user".to_string(),
                SseEvent::JobMessage {
                    job_id: job_id.to_string(),
                    role: "assistant".to_string(),
                    content: "I found a bug".to_string(),
                },
            ))
            .unwrap();

        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), inject_rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(msg.channel, "job_monitor");
        assert_eq!(msg.user_id, "system");
        assert!(msg.content.contains("I found a bug"));
    }

    #[tokio::test]
    async fn test_monitor_ignores_other_jobs() {
        let (event_tx, _) = broadcast::channel::<(Uuid, String, SseEvent)>(16);
        let (inject_tx, mut inject_rx) = mpsc::channel::<IncomingMessage>(16);

        let job_id = Uuid::new_v4();
        let other_job_id = Uuid::new_v4();
        let _handle = spawn_job_monitor(job_id, event_tx.subscribe(), inject_tx);

        // Send a message for a different job
        event_tx
            .send((
                other_job_id,
                "test-user".to_string(),
                SseEvent::JobMessage {
                    job_id: other_job_id.to_string(),
                    role: "assistant".to_string(),
                    content: "wrong job".to_string(),
                },
            ))
            .unwrap();

        // Should not receive anything
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(100), inject_rx.recv()).await;
        assert!(
            result.is_err(),
            "should have timed out, no message expected"
        );
    }

    #[tokio::test]
    async fn test_monitor_exits_on_job_result() {
        let (event_tx, _) = broadcast::channel::<(Uuid, String, SseEvent)>(16);
        let (inject_tx, mut inject_rx) = mpsc::channel::<IncomingMessage>(16);

        let job_id = Uuid::new_v4();
        let handle = spawn_job_monitor(job_id, event_tx.subscribe(), inject_tx);

        // Send a completion event
        event_tx
            .send((
                job_id,
                "test-user".to_string(),
                SseEvent::JobResult {
                    job_id: job_id.to_string(),
                    status: "completed".to_string(),
                    session_id: None,
                },
            ))
            .unwrap();

        // Should receive the completion message
        let msg = tokio::time::timeout(std::time::Duration::from_secs(1), inject_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(msg.content.contains("finished"));

        // The monitor task should exit
        tokio::time::timeout(std::time::Duration::from_secs(1), handle)
            .await
            .expect("monitor should have exited")
            .expect("monitor task should not panic");
    }

    #[tokio::test]
    async fn test_monitor_skips_tool_events() {
        let (event_tx, _) = broadcast::channel::<(Uuid, String, SseEvent)>(16);
        let (inject_tx, mut inject_rx) = mpsc::channel::<IncomingMessage>(16);

        let job_id = Uuid::new_v4();
        let _handle = spawn_job_monitor(job_id, event_tx.subscribe(), inject_tx);

        // Send tool use event (should be skipped)
        event_tx
            .send((
                job_id,
                "test-user".to_string(),
                SseEvent::JobToolUse {
                    job_id: job_id.to_string(),
                    tool_name: "shell".to_string(),
                    input: serde_json::json!({"command": "ls"}),
                },
            ))
            .unwrap();

        // Send user message (should be skipped)
        event_tx
            .send((
                job_id,
                "test-user".to_string(),
                SseEvent::JobMessage {
                    job_id: job_id.to_string(),
                    role: "user".to_string(),
                    content: "user prompt".to_string(),
                },
            ))
            .unwrap();

        // Should not receive anything for tool events or user messages
        let result =
            tokio::time::timeout(std::time::Duration::from_millis(100), inject_rx.recv()).await;
        assert!(
            result.is_err(),
            "should have timed out, no message expected"
        );
    }
}

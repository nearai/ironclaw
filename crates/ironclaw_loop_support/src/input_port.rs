//! Adapter from a host-owned input queue to the loop input port contract.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoopInputBatch, LoopInputCursor, LoopInputPort,
    LoopRunContext, LoopRunInfoPort,
};

use crate::{HostInputQueue, HostInputQueueError};

pub struct HostQueueLoopInputPort {
    queue: Arc<dyn HostInputQueue>,
    run_context: LoopRunContext,
}

impl HostQueueLoopInputPort {
    pub fn new(queue: Arc<dyn HostInputQueue>, run_context: LoopRunContext) -> Self {
        Self { queue, run_context }
    }
}

impl LoopRunInfoPort for HostQueueLoopInputPort {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl LoopInputPort for HostQueueLoopInputPort {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        validate_cursor_for_run(&after, &self.run_context)?;
        let host_batch = self
            .queue
            .next_after(self.run_context.run_id, after.token().clone(), limit)
            .await
            .map_err(host_queue_error_into_host_error)?;

        Ok(LoopInputBatch {
            inputs: host_batch.inputs,
            next_cursor: LoopInputCursor::from_host_token(
                &self.run_context,
                host_batch.next_cursor,
            ),
        })
    }

    async fn ack_inputs(&self, cursor: LoopInputCursor) -> Result<(), AgentLoopHostError> {
        validate_cursor_for_run(&cursor, &self.run_context)?;
        self.queue
            .ack_through(self.run_context.run_id, cursor.token().clone())
            .await
            .map_err(host_queue_error_into_host_error)
    }
}

fn validate_cursor_for_run(
    cursor: &LoopInputCursor,
    run_context: &LoopRunContext,
) -> Result<(), AgentLoopHostError> {
    if cursor.is_for_run(run_context) {
        Ok(())
    } else {
        Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::ScopeMismatch,
            "input cursor is not scoped to this loop run",
        ))
    }
}

fn host_queue_error_into_host_error(error: HostInputQueueError) -> AgentLoopHostError {
    match error {
        HostInputQueueError::Unavailable { reason } => {
            AgentLoopHostError::new(AgentLoopHostErrorKind::Unavailable, reason)
        }
        HostInputQueueError::InvalidCursor { reason } => {
            AgentLoopHostError::new(AgentLoopHostErrorKind::InvalidInvocation, reason)
        }
        HostInputQueueError::Internal => AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            "input queue internal error",
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId};
    use ironclaw_turns::{
        LoopGateRef, LoopMessageRef, RunProfileResolutionRequest, RunProfileResolver, TurnId,
        TurnRunId, TurnScope,
        run_profile::{
            AgentLoopHostErrorKind, CapabilitySurfaceVersion, InMemoryRunProfileResolver,
            LoopCancelReasonKind, LoopInput, LoopInputCursor, LoopInputCursorToken, LoopInputPort,
            LoopInterruptKind, LoopRunContext,
        },
    };

    use super::*;
    use crate::{HostInputBatch, HostInputQueue};

    #[tokio::test]
    async fn poll_returns_inputs_in_order() {
        let run_context = test_run_context("run-input-order").await;
        let queue = Arc::new(FakeInputQueue::new(vec![
            LoopInput::UserMessage {
                message_ref: message_ref("msg:user"),
            },
            LoopInput::Steering {
                message_ref: message_ref("msg:steering"),
            },
            LoopInput::FollowUp {
                message_ref: message_ref("msg:followup"),
            },
        ]));
        let port = HostQueueLoopInputPort::new(queue, run_context.clone());

        let batch = port
            .poll_inputs(LoopInputCursor::origin_for_run(&run_context), 8)
            .await
            .expect("poll should succeed");

        assert_eq!(
            batch.inputs,
            vec![
                LoopInput::UserMessage {
                    message_ref: message_ref("msg:user")
                },
                LoopInput::Steering {
                    message_ref: message_ref("msg:steering")
                },
                LoopInput::FollowUp {
                    message_ref: message_ref("msg:followup")
                },
            ]
        );
        assert_eq!(batch.next_cursor.token().as_str(), "input-cursor:3");
    }

    #[tokio::test]
    async fn poll_after_ack_returns_empty() {
        let run_context = test_run_context("run-after-ack").await;
        let queue = Arc::new(FakeInputQueue::new(vec![LoopInput::UserMessage {
            message_ref: message_ref("msg:user"),
        }]));
        let port = HostQueueLoopInputPort::new(queue, run_context.clone());
        let origin = LoopInputCursor::origin_for_run(&run_context);

        let first = port.poll_inputs(origin, 8).await.expect("first poll");
        port.ack_inputs(first.next_cursor.clone())
            .await
            .expect("ack should succeed");
        let second = port
            .poll_inputs(first.next_cursor.clone(), 8)
            .await
            .expect("second poll");

        assert!(second.inputs.is_empty());
        assert_eq!(second.next_cursor, first.next_cursor);
    }

    #[tokio::test]
    async fn polled_unacked_input_is_redelivered() {
        let run_context = test_run_context("run-redeliver").await;
        let queue = Arc::new(FakeInputQueue::new(vec![LoopInput::Steering {
            message_ref: message_ref("msg:steering"),
        }]));
        let port = HostQueueLoopInputPort::new(queue, run_context.clone());
        let origin = LoopInputCursor::origin_for_run(&run_context);

        let first = port
            .poll_inputs(origin.clone(), 8)
            .await
            .expect("first poll");
        let second = port.poll_inputs(origin, 8).await.expect("second poll");

        assert_eq!(second.inputs, first.inputs);
        assert_eq!(second.next_cursor, first.next_cursor);
    }

    #[tokio::test]
    async fn ack_idempotent() {
        let run_context = test_run_context("run-ack-idempotent").await;
        let queue = Arc::new(FakeInputQueue::new(vec![LoopInput::FollowUp {
            message_ref: message_ref("msg:followup"),
        }]));
        let port = HostQueueLoopInputPort::new(queue, run_context.clone());
        let batch = port
            .poll_inputs(LoopInputCursor::origin_for_run(&run_context), 8)
            .await
            .expect("poll");

        port.ack_inputs(batch.next_cursor.clone())
            .await
            .expect("first ack");
        port.ack_inputs(batch.next_cursor)
            .await
            .expect("second ack should be a no-op");
    }

    #[tokio::test]
    async fn cursor_for_different_run_is_rejected() {
        let run_context = test_run_context("run-local").await;
        let other_context = test_run_context("run-foreign").await;
        let queue = Arc::new(FakeInputQueue::new(vec![LoopInput::UserMessage {
            message_ref: message_ref("msg:user"),
        }]));
        let port = HostQueueLoopInputPort::new(queue.clone(), run_context);

        let error = port
            .poll_inputs(LoopInputCursor::origin_for_run(&other_context), 8)
            .await
            .expect_err("foreign cursor should be rejected");

        assert_eq!(error.kind, AgentLoopHostErrorKind::ScopeMismatch);
        assert_eq!(queue.call_count(), 0);
    }

    #[tokio::test]
    async fn control_inputs_pass_through_unfiltered() {
        let run_context = test_run_context("run-control-inputs").await;
        let inputs = vec![
            LoopInput::Cancel {
                reason_kind: LoopCancelReasonKind::UserRequested,
            },
            LoopInput::CapabilitySurfaceChanged {
                version: CapabilitySurfaceVersion::new("surface-v2").unwrap(),
            },
            LoopInput::Interrupt {
                kind: LoopInterruptKind::UserInterrupt,
            },
            LoopInput::GateResolved {
                gate_ref: LoopGateRef::new("gate:approval").unwrap(),
            },
        ];
        let queue = Arc::new(FakeInputQueue::new(inputs.clone()));
        let port = HostQueueLoopInputPort::new(queue, run_context.clone());

        let batch = port
            .poll_inputs(LoopInputCursor::origin_for_run(&run_context), 8)
            .await
            .expect("poll");

        assert_eq!(batch.inputs, inputs);
    }

    #[tokio::test]
    async fn host_queue_unavailable_maps_to_unavailable_host_error() {
        let run_context = test_run_context("run-unavailable").await;
        let queue = Arc::new(FailingInputQueue);
        let port = HostQueueLoopInputPort::new(queue, run_context.clone());

        let error = port
            .poll_inputs(LoopInputCursor::origin_for_run(&run_context), 8)
            .await
            .expect_err("queue failure should map to host error");

        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    }

    #[derive(Debug)]
    struct FakeInputQueue {
        entries: Vec<(usize, LoopInput)>,
        acked: Mutex<HashMap<TurnRunId, usize>>,
        calls: AtomicUsize,
    }

    impl FakeInputQueue {
        fn new(inputs: Vec<LoopInput>) -> Self {
            Self {
                entries: inputs
                    .into_iter()
                    .enumerate()
                    .map(|(index, input)| (index + 1, input))
                    .collect(),
                acked: Mutex::new(HashMap::new()),
                calls: AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl HostInputQueue for FakeInputQueue {
        async fn next_after(
            &self,
            run_id: TurnRunId,
            after: LoopInputCursorToken,
            limit: usize,
        ) -> Result<HostInputBatch, HostInputQueueError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let after_sequence = cursor_sequence(&after)?;
            let acked_sequence = self
                .acked
                .lock()
                .expect("acked map")
                .get(&run_id)
                .copied()
                .unwrap_or(0);
            let floor = after_sequence.max(acked_sequence);
            let inputs = self
                .entries
                .iter()
                .filter(|(sequence, _)| *sequence > floor)
                .take(limit)
                .cloned()
                .collect::<Vec<_>>();
            let next_cursor = inputs
                .last()
                .map(|(sequence, _)| cursor_token(*sequence))
                .unwrap_or_else(|| {
                    if floor == after_sequence {
                        after
                    } else {
                        cursor_token(floor)
                    }
                });

            Ok(HostInputBatch {
                inputs: inputs.into_iter().map(|(_, input)| input).collect(),
                next_cursor,
            })
        }

        async fn ack_through(
            &self,
            run_id: TurnRunId,
            cursor: LoopInputCursorToken,
        ) -> Result<(), HostInputQueueError> {
            let sequence = cursor_sequence(&cursor)?;
            let mut acked = self.acked.lock().expect("acked map");
            let stored = acked.entry(run_id).or_insert(0);
            *stored = (*stored).max(sequence);
            Ok(())
        }
    }

    struct FailingInputQueue;

    #[async_trait]
    impl HostInputQueue for FailingInputQueue {
        async fn next_after(
            &self,
            _run_id: TurnRunId,
            _after: LoopInputCursorToken,
            _limit: usize,
        ) -> Result<HostInputBatch, HostInputQueueError> {
            Err(HostInputQueueError::Unavailable {
                reason: "offline".to_string(),
            })
        }

        async fn ack_through(
            &self,
            _run_id: TurnRunId,
            _cursor: LoopInputCursorToken,
        ) -> Result<(), HostInputQueueError> {
            Ok(())
        }
    }

    async fn test_run_context(label: &str) -> LoopRunContext {
        let tenant_id = TenantId::new(format!("tenant-{label}")).unwrap();
        let agent_id = AgentId::new(format!("agent-{label}")).unwrap();
        let project_id = ProjectId::new(format!("project-{label}")).unwrap();
        let thread_id = ThreadId::new(format!("thread-{label}")).unwrap();
        let turn_scope = TurnScope::new(tenant_id, Some(agent_id), Some(project_id), thread_id);
        let resolved = InMemoryRunProfileResolver::default()
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .unwrap();
        LoopRunContext::new(turn_scope, TurnId::new(), TurnRunId::new(), resolved)
    }

    fn cursor_sequence(cursor: &LoopInputCursorToken) -> Result<usize, HostInputQueueError> {
        match cursor.as_str() {
            "input-cursor:origin" => Ok(0),
            value => value
                .strip_prefix("input-cursor:")
                .and_then(|suffix| suffix.parse::<usize>().ok())
                .ok_or_else(|| HostInputQueueError::InvalidCursor {
                    reason: "cursor token is not a sequence cursor".to_string(),
                }),
        }
    }

    fn cursor_token(sequence: usize) -> LoopInputCursorToken {
        LoopInputCursorToken::new(format!("input-cursor:{sequence}")).unwrap()
    }

    fn message_ref(value: &str) -> LoopMessageRef {
        LoopMessageRef::new(value).unwrap()
    }
}

//! Adapter from a host-owned input queue to the loop input port contract.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoopInputAck, LoopInputAckToken, LoopInputBatch,
    LoopInputCursor, LoopInputPort, LoopRunContext, LoopRunInfoPort,
};

use crate::{HostInputQueue, HostInputQueueError};

const MAX_HOST_INPUT_POLL_LIMIT: usize = 128;

pub struct HostQueueLoopInputPort {
    queue: Arc<dyn HostInputQueue>,
    run_context: LoopRunContext,
    issued_ack_tokens: Mutex<HashSet<LoopInputAckToken>>,
}

impl HostQueueLoopInputPort {
    pub fn new(queue: Arc<dyn HostInputQueue>, run_context: LoopRunContext) -> Self {
        Self {
            queue,
            run_context,
            issued_ack_tokens: Mutex::new(HashSet::new()),
        }
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
        let bounded_limit = bounded_limit(limit, MAX_HOST_INPUT_POLL_LIMIT);
        let host_batch = self
            .queue
            .next_after(
                self.run_context.run_id,
                after.token().clone(),
                bounded_limit,
            )
            .await
            .map_err(host_queue_error_into_host_error)?;

        let mut inputs = Vec::with_capacity(host_batch.inputs.len());
        let mut input_acks = Vec::with_capacity(host_batch.inputs.len());
        {
            let mut issued_ack_tokens = self.issued_ack_tokens.lock().map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "input ack provenance cache unavailable",
                )
            })?;
            for envelope in host_batch.inputs {
                let cursor = LoopInputCursor::from_host_token(&self.run_context, envelope.cursor);
                issued_ack_tokens.insert(envelope.ack_token.clone());
                inputs.push(envelope.input);
                input_acks.push(LoopInputAck {
                    cursor,
                    token: envelope.ack_token,
                });
            }
        }

        Ok(LoopInputBatch {
            inputs,
            input_acks,
            next_cursor: LoopInputCursor::from_host_token(
                &self.run_context,
                host_batch.next_cursor,
            ),
        })
    }

    async fn ack_inputs(&self, tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        self.validate_issued_ack_tokens(&tokens)?;
        self.queue
            .ack_consumed(self.run_context.run_id, tokens)
            .await
            .map_err(host_queue_error_into_host_error)
    }
}

impl HostQueueLoopInputPort {
    fn validate_issued_ack_tokens(
        &self,
        tokens: &[LoopInputAckToken],
    ) -> Result<(), AgentLoopHostError> {
        let issued = self.issued_ack_tokens.lock().map_err(|_| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "input ack provenance cache unavailable",
            )
        })?;
        if tokens.iter().all(|token| issued.contains(token)) {
            Ok(())
        } else {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "input ack token was not issued by this host",
            ))
        }
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

fn bounded_limit(requested: usize, configured: usize) -> usize {
    let configured = configured.max(1);
    if requested == 0 {
        configured
    } else {
        requested.min(configured)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
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
            LoopCancelReasonKind, LoopInput, LoopInputAckToken, LoopInputCursor,
            LoopInputCursorToken, LoopInputPort, LoopInterruptKind, LoopRunContext,
        },
    };

    use super::*;
    use crate::{HostInputBatch, HostInputEnvelope, HostInputQueue};

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
        assert_eq!(batch.input_acks.len(), 3);
        assert_eq!(
            batch.input_acks[2].cursor.token().as_str(),
            "input-cursor:3"
        );
        assert_eq!(batch.input_acks[2].token.as_str(), "input-ack:3");
    }

    #[tokio::test]
    async fn poll_after_exact_ack_returns_empty_for_consumed() {
        let run_context = test_run_context("run-after-ack").await;
        let queue = Arc::new(FakeInputQueue::new(vec![LoopInput::UserMessage {
            message_ref: message_ref("msg:user"),
        }]));
        let port = HostQueueLoopInputPort::new(queue, run_context.clone());
        let origin = LoopInputCursor::origin_for_run(&run_context);

        let first = port.poll_inputs(origin, 8).await.expect("first poll");
        let ack_tokens = first
            .input_acks
            .iter()
            .map(|ack| ack.token.clone())
            .collect();
        port.ack_inputs(ack_tokens)
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

        let ack_tokens = batch
            .input_acks
            .iter()
            .map(|ack| ack.token.clone())
            .collect::<Vec<_>>();
        port.ack_inputs(ack_tokens.clone())
            .await
            .expect("first ack");
        port.ack_inputs(ack_tokens)
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
    async fn durable_cursor_is_accepted_after_port_rebuild() {
        let run_context = test_run_context("run-durable-cursor").await;
        let queue = Arc::new(FakeInputQueue::new(vec![
            LoopInput::UserMessage {
                message_ref: message_ref("msg:first"),
            },
            LoopInput::UserMessage {
                message_ref: message_ref("msg:second"),
            },
        ]));
        let first_port = HostQueueLoopInputPort::new(queue.clone(), run_context.clone());
        let first = first_port
            .poll_inputs(LoopInputCursor::origin_for_run(&run_context), 1)
            .await
            .expect("first poll should succeed");

        let rebuilt_port = HostQueueLoopInputPort::new(queue, run_context.clone());
        let second = rebuilt_port
            .poll_inputs(first.next_cursor, 8)
            .await
            .expect("rebuilt port should accept durable cursor");

        assert_eq!(
            second.inputs,
            vec![LoopInput::UserMessage {
                message_ref: message_ref("msg:second")
            }]
        );
    }

    #[tokio::test]
    async fn invalid_cursor_token_is_rejected_by_host_queue() {
        let run_context = test_run_context("run-invalid-poll").await;
        let queue = Arc::new(FakeInputQueue::new(vec![LoopInput::UserMessage {
            message_ref: message_ref("msg:user"),
        }]));
        let port = HostQueueLoopInputPort::new(queue.clone(), run_context.clone());
        let invalid = LoopInputCursor::from_host_token(
            &run_context,
            LoopInputCursorToken::new("input-cursor:not-a-sequence").unwrap(),
        );

        let error = port
            .poll_inputs(invalid, 8)
            .await
            .expect_err("invalid cursor should be rejected by queue");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert_eq!(queue.call_count(), 1);
    }

    #[tokio::test]
    async fn forged_future_cursor_is_rejected_by_host_queue() {
        let run_context = test_run_context("run-future-poll").await;
        let queue = Arc::new(FakeInputQueue::new(vec![LoopInput::UserMessage {
            message_ref: message_ref("msg:user"),
        }]));
        let port = HostQueueLoopInputPort::new(queue.clone(), run_context.clone());
        let future = LoopInputCursor::from_host_token(&run_context, cursor_token(99));

        let error = port
            .poll_inputs(future, 8)
            .await
            .expect_err("future cursor should be rejected by queue");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert_eq!(queue.call_count(), 1);
    }

    #[tokio::test]
    async fn forged_future_ack_token_is_rejected_for_ack_inputs() {
        let run_context = test_run_context("run-forged-ack").await;
        let queue = Arc::new(FakeInputQueue::new(vec![LoopInput::UserMessage {
            message_ref: message_ref("msg:user"),
        }]));
        let port = HostQueueLoopInputPort::new(queue.clone(), run_context);

        let error = port
            .ack_inputs(vec![ack_token(99)])
            .await
            .expect_err("forged ack token should be rejected before queue access");

        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        assert_eq!(queue.ack_call_count(), 0);
    }

    #[tokio::test]
    async fn mixed_batch_exact_ack_only_acks_consumed_tokens() {
        let run_context = test_run_context("run-mixed-exact-ack").await;
        let queue = Arc::new(FakeInputQueue::new(vec![
            LoopInput::Cancel {
                reason_kind: LoopCancelReasonKind::UserRequested,
            },
            LoopInput::UserMessage {
                message_ref: message_ref("msg:user"),
            },
        ]));
        let port = HostQueueLoopInputPort::new(queue.clone(), run_context.clone());
        let origin = LoopInputCursor::origin_for_run(&run_context);

        let batch = port.poll_inputs(origin.clone(), 8).await.expect("poll");
        port.ack_inputs(vec![batch.input_acks[1].token.clone()])
            .await
            .expect("exact ack should succeed");

        assert_eq!(queue.acked_sequences(&run_context.run_id), vec![2]);
        let redelivered = port.poll_inputs(origin, 8).await.expect("redeliver");
        assert_eq!(
            redelivered.inputs,
            vec![LoopInput::Cancel {
                reason_kind: LoopCancelReasonKind::UserRequested
            }]
        );
    }

    #[tokio::test]
    async fn poll_limit_is_bounded_before_reaching_host_queue() {
        let run_context = test_run_context("run-limit-bound").await;
        let queue = Arc::new(FakeInputQueue::new(vec![LoopInput::UserMessage {
            message_ref: message_ref("msg:user"),
        }]));
        let port = HostQueueLoopInputPort::new(queue.clone(), run_context.clone());

        port.poll_inputs(
            LoopInputCursor::origin_for_run(&run_context),
            MAX_HOST_INPUT_POLL_LIMIT + 1000,
        )
        .await
        .expect("poll should succeed");

        assert_eq!(queue.last_limit(), Some(MAX_HOST_INPUT_POLL_LIMIT));
    }

    #[tokio::test]
    async fn zero_poll_limit_uses_default_bound_before_reaching_host_queue() {
        let run_context = test_run_context("run-zero-limit-bound").await;
        let queue = Arc::new(FakeInputQueue::new(vec![LoopInput::UserMessage {
            message_ref: message_ref("msg:user"),
        }]));
        let port = HostQueueLoopInputPort::new(queue.clone(), run_context.clone());

        port.poll_inputs(LoopInputCursor::origin_for_run(&run_context), 0)
            .await
            .expect("poll should succeed");

        assert_eq!(queue.last_limit(), Some(MAX_HOST_INPUT_POLL_LIMIT));
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
        acked: Mutex<HashMap<TurnRunId, HashSet<usize>>>,
        calls: AtomicUsize,
        ack_calls: AtomicUsize,
        last_limit: Mutex<Option<usize>>,
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
                ack_calls: AtomicUsize::new(0),
                last_limit: Mutex::new(None),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }

        fn ack_call_count(&self) -> usize {
            self.ack_calls.load(Ordering::SeqCst)
        }

        fn last_limit(&self) -> Option<usize> {
            *self.last_limit.lock().expect("last limit")
        }

        fn acked_sequences(&self, run_id: &TurnRunId) -> Vec<usize> {
            let mut sequences = self
                .acked
                .lock()
                .expect("acked map")
                .get(run_id)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<_>>();
            sequences.sort_unstable();
            sequences
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
            *self.last_limit.lock().expect("last limit") = Some(limit);
            let after_sequence = cursor_sequence(&after)?;
            let max_sequence = self
                .entries
                .iter()
                .map(|(sequence, _)| *sequence)
                .max()
                .unwrap_or(0);
            if after_sequence > max_sequence {
                return Err(HostInputQueueError::InvalidCursor {
                    reason: "cursor token was not issued by this input queue".to_string(),
                });
            }
            let acked = self
                .acked
                .lock()
                .expect("acked map")
                .get(&run_id)
                .cloned()
                .unwrap_or_default();
            let inputs = self
                .entries
                .iter()
                .filter(|(sequence, _)| *sequence > after_sequence && !acked.contains(sequence))
                .take(limit)
                .cloned()
                .collect::<Vec<_>>();
            let next_cursor = inputs
                .last()
                .map(|(sequence, _)| cursor_token(*sequence))
                .unwrap_or(after);

            Ok(HostInputBatch {
                inputs: inputs
                    .into_iter()
                    .map(|(sequence, input)| HostInputEnvelope {
                        input,
                        cursor: cursor_token(sequence),
                        ack_token: ack_token(sequence),
                    })
                    .collect(),
                next_cursor,
            })
        }

        async fn ack_consumed(
            &self,
            run_id: TurnRunId,
            tokens: Vec<LoopInputAckToken>,
        ) -> Result<(), HostInputQueueError> {
            self.ack_calls.fetch_add(1, Ordering::SeqCst);
            let mut acked = self.acked.lock().expect("acked map");
            let stored = acked.entry(run_id).or_default();
            for token in tokens {
                stored.insert(ack_sequence(&token)?);
            }
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

        async fn ack_consumed(
            &self,
            _run_id: TurnRunId,
            _tokens: Vec<LoopInputAckToken>,
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

    fn ack_sequence(token: &LoopInputAckToken) -> Result<usize, HostInputQueueError> {
        token
            .as_str()
            .strip_prefix("input-ack:")
            .and_then(|suffix| suffix.parse::<usize>().ok())
            .ok_or_else(|| HostInputQueueError::InvalidCursor {
                reason: "ack token is not a sequence token".to_string(),
            })
    }

    fn ack_token(sequence: usize) -> LoopInputAckToken {
        LoopInputAckToken::new(format!("input-ack:{sequence}")).unwrap()
    }

    fn message_ref(value: &str) -> LoopMessageRef {
        LoopMessageRef::new(value).unwrap()
    }
}

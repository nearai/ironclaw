use crate::{
    OpenAiCompatHttpError, OpenAiCompatInternalRefs, OpenAiCompatProductActionRef,
    OpenAiCompatTurnRunRef,
};
use ironclaw_product_contracts::inbound::ProductInboundAck;
use ironclaw_product_contracts::product_wire::RebornSubmitTurnResponse;

pub(crate) fn internal_refs_from_ack(
    ack: &ProductInboundAck,
) -> Result<OpenAiCompatInternalRefs, OpenAiCompatHttpError> {
    let mut ack = ack;
    loop {
        match ack {
            ProductInboundAck::Accepted {
                accepted_message_ref,
                submitted_run_id,
            } => {
                return Ok(
                    OpenAiCompatInternalRefs::new(OpenAiCompatProductActionRef::new(format!(
                        "accepted:{}",
                        accepted_message_ref.as_str()
                    ))?)
                    .with_turn_run_ref(OpenAiCompatTurnRunRef::new(submitted_run_id.to_string())?),
                );
            }
            ProductInboundAck::Duplicate { prior } => ack = prior,
            ProductInboundAck::DeferredBusy { .. } => {
                // A deferred submission is a durable acceptance queued for the
                // ACTIVE run — there is no new run this surface could bind the
                // public ref to, and binding the active run would let
                // lookup/cancel/stream target another submission's run. The
                // workflows' `accepted_ack_from_ack` screens normally reject
                // this before refs are bound; if one reaches here, surface the
                // same client-visible retryable busy shape, never an internal
                // 500 (the message was accepted, nothing failed internally).
                return Err(OpenAiCompatHttpError::from_kind(
                    429,
                    true,
                    crate::OpenAiCompatErrorKind::RateLimited,
                    None,
                ));
            }
            ProductInboundAck::RejectedBusy { .. }
            | ProductInboundAck::Rejected(_)
            | ProductInboundAck::CommandResult { .. }
            | ProductInboundAck::NoOp => return Err(OpenAiCompatHttpError::internal()),
        }
    }
}

pub(crate) fn product_ack_from_reborn_submit(
    response: RebornSubmitTurnResponse,
) -> ProductInboundAck {
    match response {
        RebornSubmitTurnResponse::Submitted {
            accepted_message_ref,
            run_id,
            ..
        }
        | RebornSubmitTurnResponse::AlreadySubmitted {
            accepted_message_ref,
            run_id,
            ..
        } => ProductInboundAck::Accepted {
            accepted_message_ref,
            submitted_run_id: run_id,
        },
        RebornSubmitTurnResponse::RejectedBusy {
            accepted_message_ref,
            active_run_id,
            ..
        } => ProductInboundAck::RejectedBusy {
            accepted_message_ref,
            active_run_id,
        },
        RebornSubmitTurnResponse::DeferredBusy {
            accepted_message_ref,
            active_run_id,
            ..
        } => ProductInboundAck::DeferredBusy {
            accepted_message_ref,
            active_run_id,
        },
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_product_contracts::inbound::ProductInboundAck;
    use ironclaw_turns::{AcceptedMessageRef, TurnRunId};

    use super::internal_refs_from_ack;

    #[test]
    fn deferred_busy_yields_retryable_busy_not_internal() {
        let ack = ProductInboundAck::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("msg:deferred-busy").expect("ref"),
            active_run_id: TurnRunId::new(),
        };
        let err = internal_refs_from_ack(&ack).unwrap_err();
        assert_eq!(
            err.status_code(),
            429,
            "DeferredBusy is a durable acceptance queued for the active run — busy, not 500"
        );
        assert!(err.retryable(), "DeferredBusy must be retryable");
    }

    #[test]
    fn duplicate_of_deferred_busy_yields_retryable_busy_not_internal() {
        let ack = ProductInboundAck::Duplicate {
            prior: Box::new(ProductInboundAck::DeferredBusy {
                accepted_message_ref: AcceptedMessageRef::new("msg:deferred-busy-dup")
                    .expect("ref"),
                active_run_id: TurnRunId::new(),
            }),
        };
        let err = internal_refs_from_ack(&ack).unwrap_err();
        assert_eq!(err.status_code(), 429);
        assert!(err.retryable(), "DeferredBusy must be retryable");
    }

    #[test]
    fn rejected_busy_yields_internal_error_no_refs() {
        let ack = ProductInboundAck::RejectedBusy {
            accepted_message_ref: AcceptedMessageRef::new("msg:rejected-busy").expect("ref"),
            active_run_id: None,
        };
        let err = internal_refs_from_ack(&ack).unwrap_err();
        assert_eq!(err.status_code(), 500);
        assert!(
            !err.retryable(),
            "RejectedBusy is terminal — internal_refs_from_ack must not bind refs"
        );
    }
}

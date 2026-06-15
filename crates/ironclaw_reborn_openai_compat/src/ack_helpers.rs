use crate::{
    OpenAiCompatHttpError, OpenAiCompatInternalRefs, OpenAiCompatProductActionRef,
    OpenAiCompatTurnRunRef,
};
use ironclaw_product_adapters::ProductInboundAck;

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
            ProductInboundAck::DeferredBusy { .. }
            | ProductInboundAck::RejectedBusy { .. }
            | ProductInboundAck::Rejected(_)
            | ProductInboundAck::CommandResult { .. }
            | ProductInboundAck::NoOp => return Err(OpenAiCompatHttpError::internal()),
        }
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_product_adapters::ProductInboundAck;
    use ironclaw_turns::AcceptedMessageRef;

    use super::internal_refs_from_ack;

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

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
            | ProductInboundAck::Rejected(_)
            | ProductInboundAck::CommandResult { .. }
            | ProductInboundAck::NoOp => return Err(OpenAiCompatHttpError::internal()),
        }
    }
}

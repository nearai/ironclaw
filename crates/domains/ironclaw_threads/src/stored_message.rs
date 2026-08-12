use serde::Serialize;

use crate::{ProviderToolCallReferenceEnvelope, SessionThreadError, ThreadMessageRecord};

/// Backend-neutral persisted transcript shape.
///
/// `ThreadMessageRecord` omits provider replay metadata from product-facing
/// serialization. Both durable and in-memory bounded reads use this stored
/// representation as their byte-budget denominator.
#[derive(Serialize)]
struct StoredThreadMessageRecord<'a> {
    #[serde(flatten)]
    record: &'a ThreadMessageRecord,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_result_provider_call: &'a Option<ProviderToolCallReferenceEnvelope>,
}

pub(crate) fn serialize_stored_thread_message(
    record: &ThreadMessageRecord,
) -> Result<Vec<u8>, SessionThreadError> {
    serde_json::to_vec_pretty(&StoredThreadMessageRecord {
        record,
        tool_result_provider_call: &record.tool_result_provider_call,
    })
    .map_err(|error| SessionThreadError::Serialization(error.to_string()))
}

use serde_json::Value;

use super::ProcessJournalStoreError;

pub(super) fn legacy_turn_record_contains_data(
    path: &str,
    body: &[u8],
) -> Result<bool, ProcessJournalStoreError> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|error| ProcessJournalStoreError::Deserialization(error.to_string()))?;
    if path.ends_with("/meta/state.json") {
        return Ok(value
            .get("journal_seq")
            .and_then(Value::as_u64)
            .is_some_and(|sequence| sequence > 0));
    }
    const COLLECTIONS: &[&str] = &[
        "turns",
        "runs",
        "active_locks",
        "checkpoints",
        "loop_checkpoints",
        "idempotency_records",
        "events",
        "admission_reservations",
        "spawn_tree_reservations",
    ];
    Ok(COLLECTIONS.iter().any(|key| {
        value.get(*key).is_some_and(|collection| match collection {
            Value::Array(values) => !values.is_empty(),
            Value::Object(values) => !values.is_empty(),
            _ => false,
        })
    }))
}

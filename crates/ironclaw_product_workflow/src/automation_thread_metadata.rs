use std::fmt;

/// Metadata source tag for threads created to record automation trigger runs.
pub const AUTOMATION_TRIGGER_THREAD_SOURCE_TAG: &str = "automation_trigger";

pub fn automation_trigger_thread_metadata_json(trigger_id: impl fmt::Display) -> String {
    serde_json::json!({
        "source": AUTOMATION_TRIGGER_THREAD_SOURCE_TAG,
        "trigger_id": trigger_id.to_string(),
    })
    .to_string()
}

pub fn thread_metadata_is_automation_trigger(
    metadata_json: &str,
) -> Result<bool, serde_json::Error> {
    if !metadata_json.contains(AUTOMATION_TRIGGER_THREAD_SOURCE_TAG) {
        return Ok(false);
    }
    let metadata = serde_json::from_str::<serde_json::Value>(metadata_json)?;
    Ok(metadata.get("source").and_then(serde_json::Value::as_str)
        == Some(AUTOMATION_TRIGGER_THREAD_SOURCE_TAG))
}

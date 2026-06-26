use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode, RuntimeDispatchErrorKind};
use serde_json::{Value, json};

use crate::FirstPartyCapabilityError;

use super::{first_party_capability_manifest, resource_profile};

pub const JSON_CAPABILITY_ID: &str = "builtin.json";

/// Build an input-error carrying a descriptive, model-visible summary.
///
/// Without a `safe_summary`, a failed `builtin.json` call surfaces an empty
/// observation to the model, which then re-reads or asks the user instead of
/// correcting its input. The summaries are plain prose (no path/payload
/// delimiters) so they survive `LoopSafeSummary` validation. See
/// `.claude/rules/tool-evidence.md` ("Empty-Fast Outputs Are Errors").
fn invalid(summary: &str) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::with_safe_summary(RuntimeDispatchErrorKind::InputEncode, summary)
}

fn operation_failed(summary: &str) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::with_safe_summary(RuntimeDispatchErrorKind::OperationFailed, summary)
}

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        JSON_CAPABILITY_ID,
        "Parse, query, stringify, and validate JSON",
        vec![EffectKind::DispatchCapability],
        PermissionMode::Allow,
        resource_profile(),
    )
}

pub(super) fn dispatch(input: &Value) -> Result<Value, FirstPartyCapabilityError> {
    if input.get("source_tool_call_id").is_some() {
        return Err(invalid("json does not accept a source_tool_call_id field"));
    }
    let operation = input
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            invalid("json operation must be one of parse, query, stringify, or validate")
        })?;
    match operation {
        "parse" => {
            let data = input
                .get("data")
                .ok_or_else(|| invalid("json parse requires a data field"))?;
            let text = data
                .as_str()
                .ok_or_else(|| invalid("json parse expects data to be a JSON-encoded string"))?;
            serde_json::from_str::<Value>(text)
                .map_err(|_| invalid("json parse failed: data is not valid JSON"))
        }
        "stringify" => {
            let data = input
                .get("data")
                .ok_or_else(|| invalid("json stringify requires a data field"))?;
            let value = if let Some(text) = data.as_str() {
                serde_json::from_str::<Value>(text)
                    .map_err(|_| invalid("json stringify failed: data string is not valid JSON"))?
            } else {
                data.clone()
            };
            serde_json::to_string_pretty(&value)
                .map(Value::String)
                .map_err(|_| operation_failed("json stringify could not serialize the value"))
        }
        "query" => {
            let data = input
                .get("data")
                .ok_or_else(|| invalid("json query requires a data field"))?;
            let path = input
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("json query requires a string path field"))?;
            let value = if let Some(text) = data.as_str() {
                serde_json::from_str::<Value>(text)
                    .map_err(|_| invalid("json query failed: data string is not valid JSON"))?
            } else {
                data.clone()
            };
            query_json(&value, path).cloned()
        }
        "validate" => {
            let valid = input
                .get("data")
                .and_then(Value::as_str)
                .map(|text| serde_json::from_str::<Value>(text).is_ok())
                .unwrap_or(false);
            Ok(json!({ "valid": valid }))
        }
        _ => Err(invalid(
            "json operation must be one of parse, query, stringify, or validate",
        )),
    }
}

fn query_json<'a>(value: &'a Value, path: &str) -> Result<&'a Value, FirstPartyCapabilityError> {
    let mut current = value;
    for segment in path.split('.') {
        if segment.is_empty() {
            continue;
        }
        if let Some((field, rest)) = segment.split_once('[') {
            if !field.is_empty() {
                current = current
                    .get(field)
                    .ok_or_else(|| invalid("json query path not found in the provided data"))?;
            }
            let index_text = rest
                .strip_suffix(']')
                .ok_or_else(|| invalid("json query path has an unterminated array index"))?;
            let index = index_text
                .parse::<usize>()
                .map_err(|_| invalid("json query path contains an invalid array index"))?;
            current = current
                .get(index)
                .ok_or_else(|| invalid("json query array index is out of range"))?;
        } else {
            current = current
                .get(segment)
                .ok_or_else(|| invalid("json query path not found in the provided data"))?;
        }
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(error: &FirstPartyCapabilityError) -> &str {
        error
            .safe_summary()
            .expect("failed json dispatch must carry a model-visible summary")
    }

    #[test]
    fn parse_of_non_json_data_returns_descriptive_error() {
        // A path-like string (the reg-002 shape) is not valid JSON: the model
        // must see why rather than an empty observation.
        let error =
            dispatch(&json!({ "operation": "parse", "data": "/workspace/report.csv" })).unwrap_err();
        assert_eq!(summary(&error), "json parse failed: data is not valid JSON");
    }

    #[test]
    fn query_missing_path_returns_descriptive_error() {
        let error = dispatch(&json!({
            "operation": "query",
            "data": "{\"a\":1}",
            "path": "b"
        }))
        .unwrap_err();
        assert_eq!(summary(&error), "json query path not found in the provided data");
    }
}

use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode, RuntimeDispatchErrorKind};
use serde_json::{Value, json};

use crate::FirstPartyCapabilityError;

use super::{first_party_capability_manifest, operation_error, resource_profile};

pub const JSON_CAPABILITY_ID: &str = "builtin.json";

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
        return Err(json_input_error(
            "json input must not include source tool call refs",
        ));
    }
    let operation = input
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| json_input_error("json operation must be a string"))?;
    match operation {
        "parse" => {
            let data = input
                .get("data")
                .ok_or_else(|| json_input_error("json parse requires data"))?;
            let text = data
                .as_str()
                .ok_or_else(|| json_input_error("json parse expected data to be a JSON string"))?;
            serde_json::from_str::<Value>(text)
                .map_err(|_| json_input_error("json parse received invalid JSON"))
        }
        "stringify" => {
            let data = input
                .get("data")
                .ok_or_else(|| json_input_error("json stringify requires data"))?;
            let value = if let Some(text) = data.as_str() {
                serde_json::from_str::<Value>(text)
                    .map_err(|_| json_input_error("json stringify received invalid JSON"))?
            } else {
                data.clone()
            };
            serde_json::to_string_pretty(&value)
                .map(Value::String)
                .map_err(|_| operation_error())
        }
        "query" => {
            let data = input
                .get("data")
                .ok_or_else(|| json_input_error("json query requires data"))?;
            let path = input
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| json_input_error("json query requires a path string"))?;
            let value = if let Some(text) = data.as_str() {
                serde_json::from_str::<Value>(text)
                    .map_err(|_| json_input_error("json query received invalid JSON"))?
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
        _ => Err(json_input_error(
            "json operation must be parse stringify query or validate",
        )),
    }
}

fn json_input_error(summary: &'static str) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::with_safe_summary(RuntimeDispatchErrorKind::InputEncode, summary)
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
                    .ok_or_else(|| json_input_error("json query path did not match data"))?;
            }
            let index_text = rest
                .strip_suffix(']')
                .ok_or_else(|| json_input_error("json query path has invalid array syntax"))?;
            let index = index_text
                .parse::<usize>()
                .map_err(|_| json_input_error("json query path has invalid array index"))?;
            current = current
                .get(index)
                .ok_or_else(|| json_input_error("json query path did not match data"))?;
        } else {
            current = current
                .get(segment)
                .ok_or_else(|| json_input_error("json query path did not match data"))?;
        }
    }
    Ok(current)
}

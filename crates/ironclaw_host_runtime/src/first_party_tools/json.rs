use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{EffectKind, PermissionMode};
use serde_json::{Value, json};

use crate::FirstPartyCapabilityError;

use super::{first_party_capability_manifest, guest_error, input_error, resource_profile};

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
        return Err(input_error());
    }
    let operation = input
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    match operation {
        "parse" => {
            let data = input.get("data").ok_or_else(input_error)?;
            let text = data.as_str().ok_or_else(input_error)?;
            serde_json::from_str::<Value>(text).map_err(|_| input_error())
        }
        "stringify" => {
            let data = input.get("data").ok_or_else(input_error)?;
            let value = if let Some(text) = data.as_str() {
                serde_json::from_str::<Value>(text).map_err(|_| input_error())?
            } else {
                data.clone()
            };
            serde_json::to_string_pretty(&value)
                .map(Value::String)
                .map_err(|_| guest_error())
        }
        "query" => {
            let data = input.get("data").ok_or_else(input_error)?;
            let path = input
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(input_error)?;
            let value = if let Some(text) = data.as_str() {
                serde_json::from_str::<Value>(text).map_err(|_| input_error())?
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
        _ => Err(input_error()),
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
                current = current.get(field).ok_or_else(input_error)?;
            }
            let index_text = rest.strip_suffix(']').ok_or_else(input_error)?;
            let index = index_text.parse::<usize>().map_err(|_| input_error())?;
            current = current.get(index).ok_or_else(input_error)?;
        } else {
            current = current.get(segment).ok_or_else(input_error)?;
        }
    }
    Ok(current)
}

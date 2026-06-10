//! Input extraction + validation for NEAR capability requests.

use super::*;

pub(crate) fn required_account_id(input: &Value, key: &str) -> Result<String, NearDispatchError> {
    let value = required_bounded(input, key, MAX_ACCOUNT_ID_CHARS)?;
    if !is_valid_near_account_id(&value) {
        return Err(input_error());
    }
    Ok(value)
}

/// Validate a NEAR account ID per the protocol rules: 2–64 chars, lowercase
/// alphanumeric plus `-`, `_`, `.` as separators that may not lead, trail, or
/// repeat. Length-only checks admit IDs that fail at the RPC layer with opaque
/// errors, so reject malformed IDs at the boundary.
pub(crate) fn is_valid_near_account_id(account_id: &str) -> bool {
    if !(2..=MAX_ACCOUNT_ID_CHARS).contains(&account_id.len()) {
        return false;
    }
    // Treat the start as a preceding separator so a leading `-`, `_`, or `.` is
    // rejected.
    let mut prev_is_separator = true;
    for byte in account_id.bytes() {
        let is_separator = matches!(byte, b'-' | b'_' | b'.');
        if is_separator {
            if prev_is_separator {
                return false;
            }
        } else if !byte.is_ascii_lowercase() && !byte.is_ascii_digit() {
            return false;
        }
        prev_is_separator = is_separator;
    }
    // A trailing separator leaves `prev_is_separator` set.
    !prev_is_separator
}

pub(crate) fn required_method_name(input: &Value, key: &str) -> Result<String, NearDispatchError> {
    required_bounded(input, key, MAX_METHOD_NAME_CHARS)
}

pub(crate) fn required_bounded(
    input: &Value,
    key: &str,
    max_chars: usize,
) -> Result<String, NearDispatchError> {
    let value = input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    if value.chars().count() > max_chars {
        return Err(input_error());
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(input_error());
    }
    Ok(trimmed.to_string())
}

pub(crate) fn optional_string(
    input: &Value,
    key: &str,
) -> Result<Option<String>, NearDispatchError> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    value
        .as_str()
        .map(|value| Some(value.to_string()))
        .ok_or_else(input_error)
}

pub(crate) fn optional_u64(input: &Value, key: &str) -> Result<Option<u64>, NearDispatchError> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    value.as_u64().map(Some).ok_or_else(input_error)
}

pub(crate) fn optional_object(
    input: &Value,
    key: &str,
) -> Result<Option<Value>, NearDispatchError> {
    let Some(value) = input.get(key) else {
        return Ok(None);
    };
    if value.is_object() {
        Ok(Some(value.clone()))
    } else {
        Err(input_error())
    }
}

pub(crate) fn required_string_array(
    input: &Value,
    key: &str,
    max_items: usize,
    max_chars: usize,
) -> Result<Vec<String>, NearDispatchError> {
    let values = input
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(input_error)?;
    if values.is_empty() || values.len() > max_items {
        return Err(input_error());
    }
    values
        .iter()
        .map(|item| {
            let value = item.as_str().ok_or_else(input_error)?;
            if value.chars().count() > max_chars {
                return Err(input_error());
            }
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Err(input_error());
            }
            Ok(trimmed.to_string())
        })
        .collect()
}

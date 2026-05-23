use serde_json::Value;

use ironclaw_host_runtime::FirstPartyCapabilityError;

use super::input_error;

pub(super) fn required_str<'a>(
    input: &'a Value,
    field: &str,
) -> Result<&'a str, FirstPartyCapabilityError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(input_error)
}

pub(super) fn optional_usize(
    input: &Value,
    field: &str,
) -> Result<Option<usize>, FirstPartyCapabilityError> {
    input
        .get(field)
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(input_error)
        })
        .transpose()
}

pub(super) fn optional_usize_allow_zero(
    input: &Value,
    field: &str,
) -> Result<Option<usize>, FirstPartyCapabilityError> {
    input
        .get(field)
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(input_error)
        })
        .transpose()
}

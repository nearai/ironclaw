use ironclaw_extension_registry::{CapabilityManifest, ExtensionError};
use ironclaw_filesystem::{FilesystemError, ScopedFilesystem};
use ironclaw_host_api::{
    capability::{EffectKind, PermissionMode},
    dispatch::{DispatchInputIssue, DispatchInputIssueCode, RuntimeDispatchErrorKind},
    path::ScopedPath,
};
use serde_json::{Value, json};

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

use super::{first_party_capability_manifest, input_error, operation_error, resource_profile};

pub const JSON_CAPABILITY_ID: &str = "builtin.json";
const MAX_JSON_FILE_BYTES: usize = 8 * 1_024 * 1_024;
const MAX_QUERY_PATH_BYTES: usize = 4_096;
const MAX_QUERY_PATH_COMPONENTS: usize = 256;
const MAX_SLICE_ITEMS: usize = 4_096;
const MAX_AGGREGATE_ITEMS: usize = 100_000;

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        JSON_CAPABILITY_ID,
        "Parse, query, stringify, validate, and run bounded collection analysis on JSON",
        vec![EffectKind::DispatchCapability, EffectKind::ReadFilesystem],
        PermissionMode::Allow,
        resource_profile(),
    )
}

pub(super) async fn dispatch(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let input = &request.input;
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
            serde_json::from_str::<Value>(text).map_err(|error| invalid_json("data", error))
        }
        "stringify" => {
            let data = input.get("data").ok_or_else(input_error)?;
            let value = if let Some(text) = data.as_str() {
                serde_json::from_str::<Value>(text).map_err(|error| invalid_json("data", error))?
            } else {
                data.clone()
            };
            serde_json::to_string_pretty(&value)
                .map(Value::String)
                .map_err(|_| operation_error())
        }
        "query" => {
            let value = query_input(request).await?;
            select(&request.input, &value).cloned()
        }
        "length" => {
            let value = query_input(request).await?;
            let selected = select(&request.input, &value)?;
            let length = match selected {
                Value::Array(values) => values.len(),
                Value::Object(values) => values.len(),
                _ => {
                    return Err(structured_input_error(
                        "JSON collection input failed validation",
                        "path",
                        "a path resolving to an array or object",
                        json_type(selected),
                    ));
                }
            };
            Ok(json!(length))
        }
        "last" => {
            let value = query_input(request).await?;
            let selected = select(&request.input, &value)?;
            let values = selected.as_array().ok_or_else(|| {
                structured_input_error(
                    "JSON collection input failed validation",
                    "path",
                    "a path resolving to an array",
                    json_type(selected),
                )
            })?;
            values.last().cloned().ok_or_else(|| {
                structured_input_error(
                    "JSON collection input failed validation",
                    "path",
                    "a non-empty array",
                    "an empty array",
                )
            })
        }
        "slice" => {
            let value = query_input(request).await?;
            let selected = select(&request.input, &value)?;
            let values = selected.as_array().ok_or_else(|| {
                structured_input_error(
                    "JSON collection input failed validation",
                    "path",
                    "a path resolving to an array",
                    json_type(selected),
                )
            })?;
            let start =
                bounded_input_usize(input, "start", "JSON collection input failed validation")?;
            let end = bounded_input_usize(input, "end", "JSON collection input failed validation")?;
            if end < start {
                return Err(structured_input_error(
                    "JSON collection input failed validation",
                    "end",
                    "an end index greater than or equal to start",
                    end.to_string(),
                ));
            }
            if end > values.len() {
                return Err(structured_input_error(
                    "JSON collection input failed validation",
                    "end",
                    "an end index within the selected array",
                    end.to_string(),
                ));
            }
            if end - start > MAX_SLICE_ITEMS {
                return Err(FirstPartyCapabilityError::with_safe_summary(
                    RuntimeDispatchErrorKind::Resource,
                    format!("JSON slice exceeds the {MAX_SLICE_ITEMS}-item limit"),
                ));
            }
            Ok(Value::Array(values[start..end].to_vec()))
        }
        "aggregate" => {
            let value = query_input(request).await?;
            aggregate(request, &value).await
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

fn select<'a>(input: &Value, value: &'a Value) -> Result<&'a Value, FirstPartyCapabilityError> {
    let path = input
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(input_error)?;
    query_json(value, path)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AggregateFunction {
    Sum,
    Average,
    Min,
    Max,
}

impl AggregateFunction {
    fn from_wire(function: &str) -> Option<Self> {
        match function {
            "sum" => Some(Self::Sum),
            "average" => Some(Self::Average),
            "min" => Some(Self::Min),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    fn as_wire(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Average => "average",
            Self::Min => "min",
            Self::Max => "max",
        }
    }
}

async fn aggregate(
    request: &FirstPartyCapabilityRequest,
    value: &Value,
) -> Result<Value, FirstPartyCapabilityError> {
    let selected = select(&request.input, value)?;
    let values = selected.as_array().ok_or_else(|| {
        structured_input_error(
            "JSON aggregate input failed validation",
            "path",
            "a path resolving to an array",
            json_type(selected),
        )
    })?;
    if values.is_empty() {
        return Err(structured_input_error(
            "JSON aggregate input failed validation",
            "path",
            "a non-empty array",
            "an empty array",
        ));
    }
    if values.len() > MAX_AGGREGATE_ITEMS {
        return Err(FirstPartyCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::Resource,
            format!("JSON aggregate exceeds the {MAX_AGGREGATE_ITEMS}-item limit"),
        ));
    }

    let function = match request.input.get("function").and_then(Value::as_str) {
        Some(wire) => AggregateFunction::from_wire(wire).ok_or_else(|| {
            structured_input_error(
                "JSON aggregate input failed validation",
                "function",
                "sum, average, min, or max",
                bounded_received(wire),
            )
        })?,
        None => return Err(input_error()),
    };
    let value_index = request
        .input
        .get("value_index")
        .map(|_| {
            bounded_input_usize(
                &request.input,
                "value_index",
                "JSON aggregate input failed validation",
            )
        })
        .transpose()?;

    // Classify the numeric domain before computing: integer-only input
    // aggregates exactly (no f64 rounding), any decimal input computes in
    // floating point. This pass also validates numeric shape and row indices.
    let mut all_integer = true;
    for (position, item) in values.iter().enumerate() {
        let candidate = aggregate_candidate(item, value_index, position)?;
        match candidate {
            Value::Number(number) if number.is_i64() || number.is_u64() => {}
            Value::Number(_) => all_integer = false,
            _ => {
                return Err(structured_input_error(
                    "JSON aggregate input failed validation",
                    "path",
                    "numeric selected values",
                    format!("item {} was {}", position + 1, json_type(candidate)),
                ));
            }
        }
    }

    let result = if all_integer {
        integer_aggregate(values, value_index, function)?
    } else {
        float_aggregate(values, value_index, function)?
    };
    Ok(json!({
        "count": values.len(),
        "function": function.as_wire(),
        "value": result,
    }))
}

fn aggregate_candidate(
    item: &Value,
    value_index: Option<usize>,
    position: usize,
) -> Result<&Value, FirstPartyCapabilityError> {
    if let Some(index) = value_index {
        item.as_array()
            .and_then(|row| row.get(index))
            .ok_or_else(|| {
                structured_input_error(
                    "JSON aggregate input failed validation",
                    "value_index",
                    "an in-bounds index for every selected array item",
                    format!("item {} did not resolve", position + 1),
                )
            })
    } else {
        Ok(item)
    }
}

fn integer_value(value: &Value) -> Option<i128> {
    value
        .as_i64()
        .map(i128::from)
        .or_else(|| value.as_u64().map(i128::from))
}

fn integer_aggregate(
    values: &[Value],
    value_index: Option<usize>,
    function: AggregateFunction,
) -> Result<serde_json::Number, FirstPartyCapabilityError> {
    let mut sum: i128 = 0;
    let mut minimum = i128::MAX;
    let mut maximum = i128::MIN;
    for (position, item) in values.iter().enumerate() {
        let candidate = aggregate_candidate(item, value_index, position)?;
        let number = integer_value(candidate).ok_or_else(|| {
            structured_input_error(
                "JSON aggregate input failed validation",
                "path",
                "numeric selected values",
                format!("item {} was {}", position + 1, json_type(candidate)),
            )
        })?;
        sum += number;
        minimum = minimum.min(number);
        maximum = maximum.max(number);
    }
    let result = match function {
        AggregateFunction::Sum => integer_result(sum)?,
        // A single rounding of the exact i128 sum; the sum itself cannot
        // overflow because 100000 items bounded by u64/i64 fit in i128.
        AggregateFunction::Average => {
            serde_json::Number::from_f64(sum as f64 / values.len() as f64)
                .ok_or_else(operation_error)?
        }
        AggregateFunction::Min => integer_result(minimum)?,
        AggregateFunction::Max => integer_result(maximum)?,
    };
    Ok(result)
}

fn float_aggregate(
    values: &[Value],
    value_index: Option<usize>,
    function: AggregateFunction,
) -> Result<serde_json::Number, FirstPartyCapabilityError> {
    let mut sum = 0.0;
    // Incremental mean (single-pass update) so "average" never materializes
    // an overflowing total: [1e308, 1e308] averages to 1e308.
    let mut mean = 0.0;
    let mut minimum = f64::INFINITY;
    let mut maximum = f64::NEG_INFINITY;
    for (position, item) in values.iter().enumerate() {
        let candidate = aggregate_candidate(item, value_index, position)?;
        let number = candidate.as_f64().ok_or_else(|| {
            structured_input_error(
                "JSON aggregate input failed validation",
                "path",
                "numeric selected values",
                format!("item {} was {}", position + 1, json_type(candidate)),
            )
        })?;
        sum += number;
        mean += (number - mean) / (position + 1) as f64;
        minimum = minimum.min(number);
        maximum = maximum.max(number);
    }
    let result = match function {
        AggregateFunction::Sum => sum,
        AggregateFunction::Average => mean,
        AggregateFunction::Min => minimum,
        AggregateFunction::Max => maximum,
    };
    if !result.is_finite() {
        return Err(FirstPartyCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::Resource,
            "JSON aggregate exceeded the finite numeric range".to_string(),
        ));
    }
    serde_json::Number::from_f64(result).ok_or_else(operation_error)
}

fn integer_result(value: i128) -> Result<serde_json::Number, FirstPartyCapabilityError> {
    serde_json::Number::from_i128(value).ok_or_else(|| {
        FirstPartyCapabilityError::with_safe_summary(
            RuntimeDispatchErrorKind::Resource,
            "JSON aggregate result exceeds the exact integer range".to_string(),
        )
    })
}

fn bounded_input_usize(
    input: &Value,
    field: &'static str,
    summary: &'static str,
) -> Result<usize, FirstPartyCapabilityError> {
    let value = input.get(field).and_then(Value::as_u64).ok_or_else(|| {
        structured_input_error(summary, field, "a non-negative integer", "invalid value")
    })?;
    usize::try_from(value).map_err(|_| {
        structured_input_error(
            summary,
            field,
            "an integer that fits in usize",
            value.to_string(),
        )
    })
}

fn bounded_received(value: &str) -> String {
    const MAX_RECEIVED_CHARS: usize = 64;
    let mut chars = value.chars();
    let mut bounded = chars.by_ref().take(MAX_RECEIVED_CHARS).collect::<String>();
    if chars.next().is_some() {
        bounded.push('…');
    }
    bounded
}

fn structured_input_error(
    summary: &'static str,
    field: &'static str,
    expected: impl Into<String>,
    received: impl Into<String>,
) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::invalid_input_issues(
        summary,
        vec![
            DispatchInputIssue::new(field, DispatchInputIssueCode::InvalidValue)
                .expected(expected.into())
                .received(received.into()),
        ],
    )
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

async fn query_input(
    request: &FirstPartyCapabilityRequest,
) -> Result<Value, FirstPartyCapabilityError> {
    let data = request.input.get("data");
    let file_path = request.input.get("file_path");
    match (data, file_path) {
        (Some(data), None) => parse_json_value(data),
        (None, Some(file_path)) => {
            let file_path = file_path
                .as_str()
                .ok_or_else(|| invalid_field("file_path"))?;
            if file_path.len() > MAX_QUERY_PATH_BYTES {
                return Err(invalid_file_path());
            }
            let path = ScopedPath::new(file_path).map_err(|_| invalid_file_path())?;
            if path
                .as_str()
                .strip_prefix("/workspace/")
                .is_none_or(str::is_empty)
            {
                return Err(invalid_file_path());
            }
            let mounts = request.mounts.clone().ok_or_else(|| {
                FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied)
            })?;
            let filesystem = ScopedFilesystem::with_fixed_view(
                std::sync::Arc::clone(&request.services.filesystem),
                mounts,
            );
            let bytes = filesystem
                .read_bytes_bounded(&request.scope, &path, MAX_JSON_FILE_BYTES)
                .await
                .map_err(map_filesystem_error)?
                .ok_or_else(json_file_too_large)?;
            serde_json::from_slice(&bytes).map_err(|error| invalid_json("file_path", error))
        }
        _ => Err(FirstPartyCapabilityError::invalid_input_issues(
            "JSON query input failed validation",
            vec![
                DispatchInputIssue::new("data", DispatchInputIssueCode::InvalidValue)
                    .expected("exactly one of data or file_path"),
                DispatchInputIssue::new("file_path", DispatchInputIssueCode::InvalidValue)
                    .expected("exactly one of data or file_path"),
            ],
        )),
    }
}

fn parse_json_value(data: &Value) -> Result<Value, FirstPartyCapabilityError> {
    if let Some(text) = data.as_str() {
        serde_json::from_str::<Value>(text).map_err(|error| invalid_json("data", error))
    } else {
        Ok(data.clone())
    }
}

fn invalid_json(field: &'static str, error: serde_json::Error) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::invalid_input_issues(
        "JSON input is not valid JSON",
        vec![
            DispatchInputIssue::new(field, DispatchInputIssueCode::InvalidValue)
                .expected("valid JSON")
                .received(format!(
                    "invalid JSON at line {}, column {}",
                    error.line(),
                    error.column()
                )),
        ],
    )
}

fn invalid_field(field: &'static str) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::invalid_input_issues(
        "JSON query input failed validation",
        vec![
            DispatchInputIssue::new(field, DispatchInputIssueCode::TypeMismatch).expected("string"),
        ],
    )
}

fn invalid_file_path() -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::invalid_input_issues(
        "JSON query file path failed validation",
        vec![
            DispatchInputIssue::new("file_path", DispatchInputIssueCode::InvalidValue)
                .expected("a file below /workspace"),
        ],
    )
}

fn json_file_too_large() -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::with_safe_summary(
        RuntimeDispatchErrorKind::Resource,
        format!("JSON query file exceeds the {MAX_JSON_FILE_BYTES}-byte limit"),
    )
}

fn map_filesystem_error(error: FilesystemError) -> FirstPartyCapabilityError {
    tracing::debug!(error = %error, "JSON query file read failed");
    match error {
        FilesystemError::PermissionDenied { .. }
        | FilesystemError::Contract(_)
        | FilesystemError::MountNotFound { .. }
        | FilesystemError::PathOutsideMount { .. }
        | FilesystemError::SymlinkEscape { .. } => {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::FilesystemDenied)
        }
        FilesystemError::NotFound { .. } => FirstPartyCapabilityError::invalid_input_issues(
            "JSON query file was not found",
            vec![
                DispatchInputIssue::new("file_path", DispatchInputIssueCode::InvalidValue)
                    .expected("an existing JSON file below /workspace"),
            ],
        ),
        _ => FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryPathComponent<'a> {
    Field(&'a str),
    Index(usize),
}

fn query_json<'a>(value: &'a Value, path: &str) -> Result<&'a Value, FirstPartyCapabilityError> {
    let components = parse_query_path(path)?;
    let mut current = value;
    for (position, component) in components.into_iter().enumerate() {
        current = match component {
            QueryPathComponent::Field(field) => current.get(field),
            QueryPathComponent::Index(index) => current.get(index),
        }
        .ok_or_else(|| unresolved_path(position))?;
    }
    Ok(current)
}

fn parse_query_path(path: &str) -> Result<Vec<QueryPathComponent<'_>>, FirstPartyCapabilityError> {
    if path.len() > MAX_QUERY_PATH_BYTES {
        return Err(invalid_query_path(
            MAX_QUERY_PATH_BYTES,
            "a path no longer than 4096 bytes",
        ));
    }
    let bytes = path.as_bytes();
    let mut cursor = 0;
    let mut components = Vec::new();
    let mut first = true;

    if bytes.first() == Some(&b'$') {
        match bytes.get(1) {
            None => return Ok(components),
            Some(b'.') => {
                cursor = 2;
                if cursor == bytes.len() || matches!(bytes[cursor], b'.' | b'[' | b']') {
                    return Err(invalid_query_path(
                        cursor,
                        "an object field after the '$.' root marker",
                    ));
                }
            }
            Some(b'[') => {
                cursor = 1;
                parse_indices(path, &mut cursor, &mut components)?;
                first = false;
            }
            Some(_) => {}
        }
    }

    while cursor < bytes.len() {
        if !first {
            if bytes[cursor] != b'.' {
                return Err(invalid_query_path(cursor, "'.', '[' or end of path"));
            }
            cursor += 1;
            if cursor == bytes.len() || matches!(bytes[cursor], b'.' | b'[' | b']') {
                return Err(invalid_query_path(cursor, "an object field after '.'"));
            }
        }

        if first && bytes[cursor] == b'[' {
            parse_indices(path, &mut cursor, &mut components)?;
        } else {
            let start = cursor;
            while cursor < bytes.len() && !matches!(bytes[cursor], b'.' | b'[' | b']') {
                cursor += 1;
            }
            if cursor == start || (cursor < bytes.len() && bytes[cursor] == b']') {
                return Err(invalid_query_path(cursor, "an object field or array index"));
            }
            push_component(
                &mut components,
                QueryPathComponent::Field(&path[start..cursor]),
                cursor,
            )?;
            parse_indices(path, &mut cursor, &mut components)?;
        }
        first = false;
    }
    Ok(components)
}

fn parse_indices<'a>(
    path: &'a str,
    cursor: &mut usize,
    components: &mut Vec<QueryPathComponent<'a>>,
) -> Result<(), FirstPartyCapabilityError> {
    let bytes = path.as_bytes();
    while *cursor < bytes.len() && bytes[*cursor] == b'[' {
        let bracket = *cursor;
        *cursor += 1;
        let start = *cursor;
        while *cursor < bytes.len() && bytes[*cursor].is_ascii_digit() {
            *cursor += 1;
        }
        if *cursor == start || *cursor >= bytes.len() || bytes[*cursor] != b']' {
            return Err(invalid_query_path(bracket, "an array index like [0]"));
        }
        let index = path[start..*cursor]
            .parse::<usize>()
            .map_err(|_| invalid_query_path(bracket, "an array index that fits in usize"))?;
        *cursor += 1;
        push_component(components, QueryPathComponent::Index(index), bracket)?;
    }
    Ok(())
}

fn push_component<'a>(
    components: &mut Vec<QueryPathComponent<'a>>,
    component: QueryPathComponent<'a>,
    byte_offset: usize,
) -> Result<(), FirstPartyCapabilityError> {
    if components.len() >= MAX_QUERY_PATH_COMPONENTS {
        return Err(invalid_query_path(
            byte_offset,
            "a path with at most 256 fields and indices",
        ));
    }
    components.push(component);
    Ok(())
}

fn invalid_query_path(byte_offset: usize, expected: &'static str) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::invalid_input_issues(
        "JSON query path failed validation",
        vec![
            DispatchInputIssue::new("path", DispatchInputIssueCode::InvalidValue)
                .expected(expected)
                .received(format!("invalid syntax at byte {byte_offset}")),
        ],
    )
}

fn unresolved_path(component: usize) -> FirstPartyCapabilityError {
    FirstPartyCapabilityError::invalid_input_issues(
        "JSON query path did not resolve",
        vec![
            DispatchInputIssue::new("path", DispatchInputIssueCode::InvalidValue)
                .expected("an existing object field or in-bounds array index")
                .received(format!("component {} did not resolve", component + 1)),
        ],
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_filesystem::{CasExpectation, Entry, InMemoryBackend, RootFilesystem};
    use ironclaw_host_api::{
        dispatch::{DispatchFailureDetail, DispatchInputIssueCode, RuntimeDispatchErrorKind},
        ids::CapabilityId,
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
        resource::ResourceScope,
    };
    use serde_json::json;

    use super::{
        JSON_CAPABILITY_ID, MAX_QUERY_PATH_BYTES, MAX_QUERY_PATH_COMPONENTS, dispatch,
        parse_query_path, query_json,
    };
    use crate::FirstPartyCapabilityRequest;

    const WORKSPACE_TARGET: &str = "/projects/json-tool-tests";

    fn workspace_mount(permissions: MountPermissions) -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("workspace alias"),
            VirtualPath::new(WORKSPACE_TARGET).expect("workspace target"),
            permissions,
        )])
        .expect("workspace mount")
    }

    fn request(input: serde_json::Value) -> FirstPartyCapabilityRequest {
        FirstPartyCapabilityRequest::request_for_test(
            CapabilityId::new(JSON_CAPABILITY_ID).expect("JSON capability id"),
            ResourceScope::system(),
            input,
            None,
        )
    }

    #[test]
    fn query_path_supports_repeated_indices_root_arrays_and_jsonpath_roots() {
        let value = json!({"nodes": [null, null, {"data": vec![vec![0]; 16]}]});
        assert_eq!(
            query_json(&value, "nodes[2].data[15][0]").expect("nested path resolves"),
            &json!(0)
        );
        assert_eq!(query_json(&value, "$"), Ok(&value));
        assert_eq!(
            query_json(&value, "$.nodes[2].data[15][0]")
                .expect("JSONPath-style object root resolves"),
            &json!(0)
        );
        assert_eq!(
            query_json(&json!([["zero"], ["one"]]), "[1][0]").expect("root-array path resolves"),
            &json!("one")
        );
        assert_eq!(
            query_json(&json!([["zero"], ["one"]]), "$[1][0]")
                .expect("JSONPath-style array root resolves"),
            &json!("one")
        );
    }

    #[test]
    fn invalid_query_path_reports_bounded_position_and_expectation() {
        let error =
            parse_query_path("nodes[2].data[15][oops]").expect_err("non-numeric index must fail");
        let crate::FirstPartyCapabilityError::Dispatch {
            safe_summary,
            detail: Some(detail),
            ..
        } = error
        else {
            panic!("expected structured invalid-input error");
        };
        assert_eq!(
            safe_summary.as_deref(),
            Some("JSON query path failed validation")
        );
        let DispatchFailureDetail::InvalidInput { issues } = *detail else {
            panic!("expected invalid-input details");
        };
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, "path");
        assert_eq!(
            issues[0].expected.as_deref(),
            Some("an array index like [0]")
        );
        assert!(
            issues[0]
                .received
                .as_deref()
                .is_some_and(|received| received.starts_with("invalid syntax at byte "))
        );
        assert!(
            issues[0]
                .received
                .as_ref()
                .is_some_and(|value| value.len() < 64)
        );
    }

    #[test]
    fn query_path_enforces_length_component_and_resolution_bounds() {
        let longest_path = "a".repeat(MAX_QUERY_PATH_BYTES);
        assert_eq!(
            parse_query_path(&longest_path)
                .expect("maximum-length path remains valid")
                .len(),
            1
        );

        let oversized_path = format!("{longest_path}a");
        let oversized_error =
            parse_query_path(&oversized_path).expect_err("oversized path must fail");
        assert_eq!(
            oversized_error.safe_summary(),
            Some("JSON query path failed validation")
        );

        let maximum_components = vec!["a"; MAX_QUERY_PATH_COMPONENTS].join(".");
        assert_eq!(
            parse_query_path(&maximum_components)
                .expect("maximum component count remains valid")
                .len(),
            MAX_QUERY_PATH_COMPONENTS
        );
        let too_many_components = format!("{maximum_components}.a");
        let component_error = parse_query_path(&too_many_components)
            .expect_err("path with too many components must fail");
        assert_eq!(
            component_error.safe_summary(),
            Some("JSON query path failed validation")
        );

        let unresolved_error = query_json(&json!({"present": true}), "missing")
            .expect_err("missing component must fail");
        assert_eq!(
            unresolved_error.safe_summary(),
            Some("JSON query path did not resolve")
        );
    }

    #[tokio::test]
    async fn collection_operations_cover_large_json_analysis_without_jsonpath_expressions() {
        assert_eq!(
            dispatch(&request(json!({
                "operation": "length",
                "data": {"prices": [[1, 10.0], [2, 20.0], [3, 30.0]]},
                "path": "prices"
            })))
            .await
            .expect("array length resolves"),
            json!(3)
        );
        assert_eq!(
            dispatch(&request(json!({
                "operation": "last",
                "data": {"prices": [[1, 10.0], [2, 20.0], [3, 30.0]]},
                "path": "prices"
            })))
            .await
            .expect("last array item resolves"),
            json!([3, 30.0])
        );
        assert_eq!(
            dispatch(&request(json!({
                "operation": "slice",
                "data": {"prices": [[1, 10.0], [2, 20.0], [3, 30.0]]},
                "path": "prices",
                "start": 1,
                "end": 3
            })))
            .await
            .expect("bounded array slice resolves"),
            json!([[2, 20.0], [3, 30.0]])
        );

        for (function, expected) in [
            ("sum", json!({"count": 3, "function": "sum", "value": 60.0})),
            (
                "average",
                json!({"count": 3, "function": "average", "value": 20.0}),
            ),
            ("min", json!({"count": 3, "function": "min", "value": 10.0})),
            ("max", json!({"count": 3, "function": "max", "value": 30.0})),
        ] {
            assert_eq!(
                dispatch(&request(json!({
                    "operation": "aggregate",
                    "data": {"prices": [[1, 10.0], [2, 20.0], [3, 30.0]]},
                    "path": "prices",
                    "function": function,
                    "value_index": 1
                })))
                .await
                .expect("numeric aggregate resolves"),
                expected
            );
        }
    }

    #[tokio::test]
    async fn collection_operations_reject_unbounded_or_incompatible_inputs() {
        let oversized_slice = dispatch(&request(json!({
            "operation": "slice",
            "data": {"items": vec![0; 4_097]},
            "path": "items",
            "start": 0,
            "end": 4_097
        })))
        .await
        .expect_err("slice output must remain bounded");
        assert_eq!(
            oversized_slice.kind(),
            Some(RuntimeDispatchErrorKind::Resource)
        );

        let non_numeric = dispatch(&request(json!({
            "operation": "aggregate",
            "data": {"items": [1, "two", 3]},
            "path": "items",
            "function": "sum"
        })))
        .await
        .expect_err("aggregate values must be numeric");
        assert_eq!(
            non_numeric.kind(),
            Some(RuntimeDispatchErrorKind::InputEncode)
        );
        assert_eq!(
            non_numeric.safe_summary(),
            Some("JSON aggregate input failed validation")
        );

        let oversized_aggregate = dispatch(&request(json!({
            "operation": "aggregate",
            "data": {"items": vec![1; 100_001]},
            "path": "items",
            "function": "sum"
        })))
        .await
        .expect_err("aggregate computation must remain bounded");
        assert_eq!(
            oversized_aggregate.kind(),
            Some(RuntimeDispatchErrorKind::Resource)
        );

        let empty_last = dispatch(&request(json!({
            "operation": "last",
            "data": {"items": []},
            "path": "items"
        })))
        .await
        .expect_err("last requires a non-empty array");
        assert_eq!(
            empty_last.kind(),
            Some(RuntimeDispatchErrorKind::InputEncode)
        );
    }

    #[tokio::test]
    async fn collection_operations_reject_out_of_range_slice_bounds() {
        for (input, message) in [
            (
                json!({
                    "operation": "slice",
                    "data": {"items": [1, 2, 3]},
                    "path": "items",
                    "start": 2,
                    "end": 1
                }),
                "an end index below start must fail",
            ),
            (
                json!({
                    "operation": "slice",
                    "data": {"items": [1, 2, 3]},
                    "path": "items",
                    "start": 0,
                    "end": 4
                }),
                "an end index beyond the selected array must fail",
            ),
            (
                json!({
                    "operation": "slice",
                    "data": {"items": [1, 2, 3]},
                    "path": "items",
                    "start": -1,
                    "end": 1
                }),
                "a negative start must fail the defensive u64 conversion",
            ),
            (
                json!({
                    "operation": "slice",
                    "data": {"items": [1, 2, 3]},
                    "path": "items",
                    "start": 1,
                    "end": 1.5
                }),
                "a fractional end must fail the defensive u64 conversion",
            ),
        ] {
            let error = dispatch(&request(input)).await.expect_err(message);
            assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::InputEncode));
            assert_eq!(
                error.safe_summary(),
                Some("JSON collection input failed validation")
            );
        }
    }

    #[tokio::test]
    async fn collection_operations_reject_non_collection_paths() {
        assert_eq!(
            dispatch(&request(json!({
                "operation": "length",
                "data": {"obj": {"a": 1, "b": 2}},
                "path": "obj"
            })))
            .await
            .expect("object length resolves"),
            json!(2)
        );

        for operation in ["length", "last", "slice", "aggregate"] {
            let input = match operation {
                "slice" => json!({
                    "operation": operation,
                    "data": {"n": 5},
                    "path": "n",
                    "start": 0,
                    "end": 1
                }),
                "aggregate" => json!({
                    "operation": operation,
                    "data": {"n": 5},
                    "path": "n",
                    "function": "sum"
                }),
                _ => json!({"operation": operation, "data": {"n": 5}, "path": "n"}),
            };
            let error = dispatch(&request(input))
                .await
                .expect_err("scalar paths must fail collection selection");
            assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::InputEncode));
        }

        let object_last = dispatch(&request(json!({
            "operation": "last",
            "data": {"obj": {"a": 1}},
            "path": "obj"
        })))
        .await
        .expect_err("last requires an array, not an object");
        assert_eq!(
            object_last.kind(),
            Some(RuntimeDispatchErrorKind::InputEncode)
        );
    }

    #[tokio::test]
    async fn aggregate_rejects_empty_unresolvable_and_unknown_inputs() {
        let empty = dispatch(&request(json!({
            "operation": "aggregate",
            "data": {"items": []},
            "path": "items",
            "function": "sum"
        })))
        .await
        .expect_err("aggregate requires a non-empty array");
        assert_eq!(empty.kind(), Some(RuntimeDispatchErrorKind::InputEncode));
        assert_eq!(
            empty.safe_summary(),
            Some("JSON aggregate input failed validation")
        );

        let missing_row_cell = dispatch(&request(json!({
            "operation": "aggregate",
            "data": {"rows": [[1, 10.0], [2]]},
            "path": "rows",
            "function": "sum",
            "value_index": 1
        })))
        .await
        .expect_err("every row must resolve the value_index cell");
        assert_eq!(
            missing_row_cell.kind(),
            Some(RuntimeDispatchErrorKind::InputEncode)
        );

        let unknown_function = dispatch(&request(json!({
            "operation": "aggregate",
            "data": {"items": [1, 2]},
            "path": "items",
            "function": "median"
        })))
        .await
        .expect_err("unsupported aggregate functions must fail");
        assert_eq!(
            unknown_function.kind(),
            Some(RuntimeDispatchErrorKind::InputEncode)
        );

        let overlong_function = dispatch(&request(json!({
            "operation": "aggregate",
            "data": {"items": [1, 2]},
            "path": "items",
            "function": "x".repeat(200)
        })))
        .await
        .expect_err("overlong function names must fail");
        let crate::FirstPartyCapabilityError::Dispatch {
            detail: Some(detail),
            ..
        } = overlong_function
        else {
            panic!("expected structured invalid-input error");
        };
        let DispatchFailureDetail::InvalidInput { issues } = *detail else {
            panic!("expected invalid-input details");
        };
        assert_eq!(issues[0].path, "function");
        assert!(
            issues[0]
                .received
                .as_ref()
                .is_some_and(|received| received.len() < 70),
            "the model-supplied function name must be bounded when echoed back"
        );
    }

    #[tokio::test]
    async fn aggregate_preserves_exact_integer_arithmetic() {
        // Sums, minimums, and maximums of integers above 2^53 must not be
        // silently rounded through f64: [2^53, 1] sums to 2^53 + 1 exactly.
        for (input, function, expected) in [
            (
                json!([9_007_199_254_740_992i64, 1]),
                "sum",
                json!(9_007_199_254_740_993i64),
            ),
            (
                json!([9_007_199_254_740_993i64, -9_007_199_254_740_993i64]),
                "min",
                json!(-9_007_199_254_740_993i64),
            ),
            (
                json!([9_007_199_254_740_993i64, -9_007_199_254_740_993i64]),
                "max",
                json!(9_007_199_254_740_993i64),
            ),
            (
                json!([18_446_744_073_709_551_615u64, 0]),
                "sum",
                json!(18_446_744_073_709_551_615u64),
            ),
        ] {
            assert_eq!(
                dispatch(&request(json!({
                    "operation": "aggregate",
                    "data": {"items": input},
                    "path": "items",
                    "function": function
                })))
                .await
                .expect("exact integer aggregate resolves"),
                json!({"count": 2, "function": function, "value": expected})
            );
        }

        // Average of [2^53 + 1, 1] stays exact: the i128 sum divides once.
        assert_eq!(
            dispatch(&request(json!({
                "operation": "aggregate",
                "data": {"items": [9_007_199_254_740_993i64, 1]},
                "path": "items",
                "function": "average"
            })))
            .await
            .expect("exact integer average resolves"),
            json!({"count": 2, "function": "average", "value": 4_503_599_627_370_497f64})
        );

        // A sum beyond the u64 range cannot be represented exactly in JSON;
        // fail with a Resource error instead of rounding silently.
        let out_of_range = dispatch(&request(json!({
            "operation": "aggregate",
            "data": {"items": [18_446_744_073_709_551_615u64, 18_446_744_073_709_551_615u64]},
            "path": "items",
            "function": "sum"
        })))
        .await
        .expect_err("out-of-range exact sums must fail instead of rounding");
        assert_eq!(
            out_of_range.kind(),
            Some(RuntimeDispatchErrorKind::Resource)
        );
        assert_eq!(
            out_of_range.safe_summary(),
            Some("JSON aggregate result exceeds the exact integer range")
        );

        // Input mixing integers and decimals computes in floating point.
        assert_eq!(
            dispatch(&request(json!({
                "operation": "aggregate",
                "data": {"items": [1, 2.5]},
                "path": "items",
                "function": "sum"
            })))
            .await
            .expect("mixed numeric aggregate resolves"),
            json!({"count": 2, "function": "sum", "value": 3.5})
        );
    }

    #[tokio::test]
    async fn aggregate_average_avoids_sum_overflow() {
        // The average of near-f64-MAX values is computable even though their
        // sum is not; incremental mean must not materialize the overflowing total.
        for function in ["average", "min", "max"] {
            assert_eq!(
                dispatch(&request(json!({
                    "operation": "aggregate",
                    "data": {"items": [1e308, 1e308]},
                    "path": "items",
                    "function": function
                })))
                .await
                .expect("computable near-limit aggregate resolves"),
                json!({"count": 2, "function": function, "value": 1e308})
            );
        }

        let overflowing_sum = dispatch(&request(json!({
            "operation": "aggregate",
            "data": {"items": [1e308, 1e308]},
            "path": "items",
            "function": "sum"
        })))
        .await
        .expect_err("a genuinely overflowing sum must fail");
        assert_eq!(
            overflowing_sum.kind(),
            Some(RuntimeDispatchErrorKind::Resource)
        );
        assert_eq!(
            overflowing_sum.safe_summary(),
            Some("JSON aggregate exceeded the finite numeric range")
        );
    }

    #[tokio::test]
    async fn file_query_requires_invocation_mount_authority() {
        let error = dispatch(&request(json!({
            "operation": "query",
            "file_path": "/workspace/source.json",
            "path": "value"
        })))
        .await
        .expect_err("missing mount authority must fail");
        assert_eq!(
            error.kind(),
            Some(RuntimeDispatchErrorKind::FilesystemDenied)
        );
    }

    #[tokio::test]
    async fn file_query_rejects_ambiguous_and_missing_input() {
        for input in [
            json!({
                "operation": "query",
                "data": {},
                "file_path": "/workspace/source.json",
                "path": "value"
            }),
            json!({"operation": "query", "path": "value"}),
        ] {
            let error = dispatch(&request(input))
                .await
                .expect_err("exactly one of data or file_path is required");
            assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::InputEncode));

            let crate::FirstPartyCapabilityError::Dispatch {
                detail: Some(detail),
                ..
            } = error
            else {
                panic!("expected structured invalid-input error");
            };
            let DispatchFailureDetail::InvalidInput { issues } = *detail else {
                panic!("expected invalid-input details");
            };
            assert_eq!(issues.len(), 2);
            assert_eq!(issues[0].path, "data");
            assert_eq!(issues[1].path, "file_path");
        }
    }

    #[tokio::test]
    async fn file_query_rejects_paths_outside_workspace_before_filesystem_access() {
        for file_path in ["/etc/passwd", "/workspace", "/workspace/"] {
            let error = dispatch(&request(json!({
                "operation": "query",
                "file_path": file_path,
                "path": "value"
            })))
            .await
            .expect_err("only files below /workspace are readable");
            assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::InputEncode));

            let crate::FirstPartyCapabilityError::Dispatch {
                detail: Some(detail),
                ..
            } = error
            else {
                panic!("expected structured invalid-input error");
            };
            let DispatchFailureDetail::InvalidInput { issues } = *detail else {
                panic!("expected invalid-input details");
            };
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].path, "file_path");
            assert_eq!(issues[0].code, DispatchInputIssueCode::InvalidValue);
        }
    }

    #[tokio::test]
    async fn file_query_reports_missing_and_malformed_json() {
        let root = Arc::new(InMemoryBackend::new());
        root.put(
            &VirtualPath::new(format!("{WORKSPACE_TARGET}/malformed.json"))
                .expect("malformed file target"),
            Entry::bytes(br#"{"unterminated": "#.to_vec()),
            CasExpectation::Absent,
        )
        .await
        .expect("seed malformed file");

        for (file_name, expected_summary) in [
            ("missing.json", "JSON query file was not found"),
            ("malformed.json", "JSON input is not valid JSON"),
        ] {
            let mut request = request(json!({
                "operation": "query",
                "file_path": format!("/workspace/{file_name}"),
                "path": "$"
            }));
            request.mounts = Some(workspace_mount(MountPermissions::read_only()));
            request.services.filesystem = root.clone();

            let error = dispatch(&request)
                .await
                .expect_err("missing and malformed files must fail safely");
            assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::InputEncode));
            assert_eq!(error.safe_summary(), Some(expected_summary));

            let crate::FirstPartyCapabilityError::Dispatch {
                detail: Some(detail),
                ..
            } = error
            else {
                panic!("expected structured invalid-input error");
            };
            let DispatchFailureDetail::InvalidInput { issues } = *detail else {
                panic!("expected invalid-input details");
            };
            assert_eq!(issues.len(), 1);
            assert_eq!(issues[0].path, "file_path");
        }
    }

    #[tokio::test]
    async fn file_query_accepts_eight_mib_and_rejects_one_byte_over_before_materializing() {
        const EXPECTED_JSON_FILE_BYTES: usize = 8 * 1_024 * 1_024;
        let root = Arc::new(InMemoryBackend::new());
        let mut exact_limit_json = br#"{"value":"ok"}"#.to_vec();
        exact_limit_json.resize(EXPECTED_JSON_FILE_BYTES, b' ');
        root.put(
            &VirtualPath::new(format!("{WORKSPACE_TARGET}/exact-limit.json"))
                .expect("exact-limit file target"),
            Entry::bytes(exact_limit_json),
            CasExpectation::Absent,
        )
        .await
        .expect("seed exact-limit file");
        root.put(
            &VirtualPath::new(format!("{WORKSPACE_TARGET}/large.json")).expect("file target"),
            Entry::bytes(vec![b' '; EXPECTED_JSON_FILE_BYTES + 1]),
            CasExpectation::Absent,
        )
        .await
        .expect("seed oversized file");

        let mut exact_limit_request = request(json!({
            "operation": "query",
            "file_path": "/workspace/exact-limit.json",
            "path": "value"
        }));
        exact_limit_request.mounts = Some(workspace_mount(MountPermissions::read_only()));
        exact_limit_request.services.filesystem = root.clone();
        assert_eq!(
            dispatch(&exact_limit_request)
                .await
                .expect("an eight MiB JSON file remains queryable"),
            json!("ok")
        );

        let mut request = request(json!({
            "operation": "query",
            "file_path": "/workspace/large.json",
            "path": "value"
        }));
        request.mounts = Some(workspace_mount(MountPermissions::read_only()));
        request.services.filesystem = root;

        let error = dispatch(&request)
            .await
            .expect_err("oversized file must fail closed");
        assert_eq!(error.kind(), Some(RuntimeDispatchErrorKind::Resource));
        assert_eq!(
            error.safe_summary(),
            Some("JSON query file exceeds the 8388608-byte limit")
        );
    }
}

use ironclaw_extensions::{CapabilityManifest, ExtensionError};
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
const MAX_JSON_FILE_BYTES: usize = 1_048_576;
const MAX_QUERY_PATH_BYTES: usize = 4_096;
const MAX_QUERY_PATH_COMPONENTS: usize = 256;

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    first_party_capability_manifest(
        JSON_CAPABILITY_ID,
        "Parse, query, stringify, and validate JSON",
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
                .map_err(|_| operation_error())
        }
        "query" => {
            let path = input
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(input_error)?;
            let value = query_input(request).await?;
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
        dispatch::{DispatchFailureDetail, RuntimeDispatchErrorKind},
        ids::CapabilityId,
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
        resource::ResourceScope,
    };
    use serde_json::json;

    use super::{JSON_CAPABILITY_ID, MAX_JSON_FILE_BYTES, dispatch, parse_query_path, query_json};
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
    fn query_path_supports_repeated_indices_and_root_arrays() {
        let value = json!({"nodes": [null, null, {"data": vec![vec![0]; 16]}]});
        assert_eq!(
            query_json(&value, "nodes[2].data[15][0]").expect("nested path resolves"),
            &json!(0)
        );
        assert_eq!(
            query_json(&json!([["zero"], ["one"]]), "[1][0]").expect("root-array path resolves"),
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
    async fn file_query_rejects_oversized_file_before_materializing_it() {
        let root = Arc::new(InMemoryBackend::new());
        root.put(
            &VirtualPath::new(format!("{WORKSPACE_TARGET}/large.json")).expect("file target"),
            Entry::bytes(vec![b' '; MAX_JSON_FILE_BYTES + 1]),
            CasExpectation::Absent,
        )
        .await
        .expect("seed oversized file");
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
            Some("JSON query file exceeds the 1048576-byte limit")
        );
    }
}

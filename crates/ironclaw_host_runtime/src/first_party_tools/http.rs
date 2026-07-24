use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use ironclaw_extensions::{CapabilityManifest, ExtensionError};
use ironclaw_host_api::{
    EffectKind, MountAlias, MountGrant, MountPermissions, MountView, NetworkMethod, NetworkPolicy,
    PermissionMode, RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED, ResourceCeiling,
    ResourceEstimate, ResourceProfile, ResourceUsage, RuntimeDispatchErrorKind,
    RuntimeHttpEgressError, RuntimeHttpEgressReasonCode, RuntimeHttpEgressRequest,
    RuntimeHttpSaveTarget, RuntimeKind, SandboxQuota, ScopedPath, VirtualPath,
    valid_http_field_name,
};
use serde_json::Value;

use crate::{
    FirstPartyCapabilityError, FirstPartyCapabilityRequest,
    http_body::{
        RESPONSE_BODY_STORE_FAILED_REASON, RESPONSE_BODY_STORE_UNAUTHORIZED_REASON,
        RESPONSE_BODY_STORE_UNAVAILABLE_REASON,
    },
};

use super::{
    first_party_capability_manifest,
    http_output::{HttpDispatchOutput, shape_response},
    input_error,
};

pub const HTTP_CAPABILITY_ID: &str = "builtin.http";
pub const HTTP_SAVE_CAPABILITY_ID: &str = "builtin.http.save";

const DEFAULT_HTTP_TIMEOUT_MS: u32 = 10_000;
const MAX_HTTP_TIMEOUT_MS: u32 = 30_000;
pub(super) const MAX_HTTP_OUTPUT_BYTES: u64 = 15 * 1024 * 1024;
const DEFAULT_INLINE_RESPONSE_BODY_LIMIT: u64 = 48 * 1024;
const MAX_INLINE_RESPONSE_BODY_LIMIT: u64 = 256 * 1024;
const DEFAULT_SAVE_RESPONSE_BODY_LIMIT: u64 = 10 * 1024 * 1024;
const MAX_SAVE_RESPONSE_BODY_LIMIT: u64 = 10 * 1024 * 1024;
const DEFAULT_NETWORK_EGRESS_BYTES: u64 = 16 * 1024;
const MAX_NETWORK_EGRESS_BYTES: u64 = 256 * 1024;
const MAX_HTTP_HEADERS: usize = 64;
const MAX_HTTP_HEADER_NAME_BYTES: usize = 512;
const MAX_HTTP_HEADER_VALUE_BYTES: usize = 8 * 1024;
const GITHUB_EXTENSION_PREFERENCE: &str = "Prefer GitHub extension capabilities for GitHub repository, issue, pull request, release, or workflow data when they are available.";
const SAVE_RESPONSE_BODY_LIMIT_EXCEEDED_SUMMARY: &str =
    "response body exceeded builtin.http.save response_body_limit; nothing was saved";
const SAVE_BODY_STORE_UNAVAILABLE_SUMMARY: &str =
    "response body store is unavailable for builtin.http.save";
const SAVE_BODY_STORE_UNAUTHORIZED_SUMMARY: &str =
    "response body store denied builtin.http.save write access";
const SAVE_BODY_STORE_FAILED_SUMMARY: &str =
    "response body store failed while saving builtin.http.save response";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpSaveMode {
    Disabled,
    Required,
}

impl HttpSaveMode {
    fn for_capability(capability_id: &str) -> Self {
        if capability_id == HTTP_SAVE_CAPABILITY_ID {
            Self::Required
        } else {
            Self::Disabled
        }
    }
}

pub(super) fn manifest() -> Result<CapabilityManifest, ExtensionError> {
    http_manifest(
        HTTP_CAPABILITY_ID,
        "Perform an outbound HTTP request through host egress. Redirect responses are returned; the host transport does not follow them.",
        vec![EffectKind::DispatchCapability, EffectKind::Network],
    )
}

pub(super) fn save_manifest() -> Result<CapabilityManifest, ExtensionError> {
    http_manifest(
        HTTP_SAVE_CAPABILITY_ID,
        "Perform an outbound HTTP request through host egress and save the sanitized response body through scoped filesystem authority.",
        vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::WriteFilesystem,
        ],
    )
}

fn http_manifest(
    capability_id: &str,
    description: &str,
    effects: Vec<EffectKind>,
) -> Result<CapabilityManifest, ExtensionError> {
    let description = format!("{description} {GITHUB_EXTENSION_PREFERENCE}");
    first_party_capability_manifest(
        capability_id,
        &description,
        effects,
        PermissionMode::Ask,
        Some(http_resource_profile()),
    )
}

fn http_resource_profile() -> ResourceProfile {
    ResourceProfile {
        default_estimate: ResourceEstimate::default()
            .set_wall_clock_ms(DEFAULT_HTTP_TIMEOUT_MS.into())
            .set_output_bytes(DEFAULT_INLINE_RESPONSE_BODY_LIMIT)
            .set_network_egress_bytes(DEFAULT_NETWORK_EGRESS_BYTES),
        hard_ceiling: Some(ResourceCeiling {
            max_usd: None,
            max_input_tokens: None,
            max_output_tokens: None,
            max_wall_clock_ms: Some(MAX_HTTP_TIMEOUT_MS.into()),
            max_output_bytes: Some(MAX_HTTP_OUTPUT_BYTES),
            sandbox: Some(SandboxQuota {
                network_egress_bytes: Some(MAX_NETWORK_EGRESS_BYTES),
                ..SandboxQuota::default()
            }),
        }),
    }
}

pub(super) async fn dispatch(
    request: &FirstPartyCapabilityRequest,
) -> Result<HttpDispatchOutput, FirstPartyCapabilityError> {
    let egress = request
        .services
        .runtime_http_egress
        .as_ref()
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::NetworkDenied))?
        .clone();
    // Keep this handler as a translator only: URL parsing, DNS/private-IP
    // enforcement, allowlists, transport, and credential injection remain in
    // HostHttpEgressService / ironclaw_network.
    let unsafe_raw_diagnostics_allowed = request.services.unsafe_raw_diagnostics_allowed;
    let mut headers = headers(&request.input).map_err(|error| {
        log_raw_http_input_error_for_local_diagnostics(
            unsafe_raw_diagnostics_allowed,
            &request.input,
            "headers",
            error,
        )
    })?;
    if json_body_needs_default_content_type(&request.input) && !has_header(&headers, "content-type")
    {
        headers.push(("content-type".to_string(), "application/json".to_string()));
    }
    let method = method(&request.input).map_err(|error| {
        log_raw_http_input_error_for_local_diagnostics(
            unsafe_raw_diagnostics_allowed,
            &request.input,
            "method",
            error,
        )
    })?;
    let url = required_string(&request.input, "url")
        .map_err(|error| {
            log_raw_http_input_error_for_local_diagnostics(
                unsafe_raw_diagnostics_allowed,
                &request.input,
                "url",
                error,
            )
        })?
        .to_string();
    let body = body(&request.input).map_err(|error| {
        log_raw_http_input_error_for_local_diagnostics(
            unsafe_raw_diagnostics_allowed,
            &request.input,
            "body",
            error,
        )
    })?;
    let save_mode = HttpSaveMode::for_capability(request.capability_id.as_str());
    let response_body_limit = response_body_limit(&request.input, save_mode).map_err(|error| {
        log_raw_http_input_error_for_local_diagnostics(
            unsafe_raw_diagnostics_allowed,
            &request.input,
            "response_body_limit",
            error,
        )
    })?;
    let timeout_ms = timeout_ms(&request.input).map_err(|error| {
        log_raw_http_input_error_for_local_diagnostics(
            unsafe_raw_diagnostics_allowed,
            &request.input,
            "timeout_ms",
            error,
        )
    })?;
    let save_body_to =
        save_body_to(&request.input, request.mounts.as_ref(), save_mode).map_err(|error| {
            log_raw_http_input_error_for_local_diagnostics(
                unsafe_raw_diagnostics_allowed,
                &request.input,
                "save_to",
                error,
            )
        })?;
    let http_request = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
        method,
        url,
        headers,
        body,
        network_policy: staged_policy_placeholder(),
        credential_injections: Vec::new(),
        // Always send a bounded limit, even when caller omits the field, so the
        // host transport stays fail-closed instead of inheriting an unbounded cap.
        response_body_limit: Some(response_body_limit),
        save_body_to,
        timeout_ms: Some(timeout_ms),
    };
    let tool_call_egress = request.services.tool_call_http_egress.clone();
    let egress_future = async move {
        match save_mode {
            HttpSaveMode::Disabled => {
                let tool_call_egress =
                    tool_call_egress.ok_or_else(|| RuntimeHttpEgressError::Network {
                        reason: "tool-call HTTP egress was not configured".to_string(),
                        request_bytes: 0,
                        response_bytes: 0,
                    })?;
                tool_call_egress
                    .execute_for_model_visible_output(http_request)
                    .await
            }
            HttpSaveMode::Required => egress.execute(http_request).await,
        }
    };
    let response = super::run_egress_catching_panic(
        egress_future,
        "first-party HTTP egress future panicked",
        || FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend),
    )
    .await?
    .map_err(|error| http_error(error, save_mode))?;
    Ok(shape_response(response, response_body_limit))
}

fn method(input: &Value) -> Result<NetworkMethod, FirstPartyCapabilityError> {
    let method = match input.get("method") {
        Some(value) => value.as_str().ok_or_else(input_error)?,
        None => "get",
    };
    match method.to_ascii_lowercase().as_str() {
        "get" => Ok(NetworkMethod::Get),
        "post" => Ok(NetworkMethod::Post),
        "put" => Ok(NetworkMethod::Put),
        "patch" => Ok(NetworkMethod::Patch),
        "delete" => Ok(NetworkMethod::Delete),
        "head" => Ok(NetworkMethod::Head),
        _ => Err(input_error()),
    }
}

fn required_string<'a>(
    input: &'a Value,
    field: &str,
) -> Result<&'a str, FirstPartyCapabilityError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(input_error)
}

fn headers(input: &Value) -> Result<Vec<(String, String)>, FirstPartyCapabilityError> {
    let Some(headers) = input.get("headers") else {
        return Ok(Vec::new());
    };
    let parsed: Vec<(String, String)> = match headers {
        Value::Object(object) => object
            .iter()
            .map(|(name, value)| {
                let value = value.as_str().ok_or_else(input_error)?;
                header_pair(name, value)
            })
            .collect::<Result<_, _>>()?,
        Value::Array(items) => items
            .iter()
            .map(|item| {
                let name = required_string(item, "name")?;
                let value = required_string(item, "value")?;
                header_pair(name, value)
            })
            .collect::<Result<_, _>>()?,
        _ => return Err(input_error()),
    };
    if parsed.len() > MAX_HTTP_HEADERS {
        return Err(input_error());
    }
    Ok(parsed)
}

fn header_pair(name: &str, value: &str) -> Result<(String, String), FirstPartyCapabilityError> {
    if !valid_http_field_name(name)
        || name.len() > MAX_HTTP_HEADER_NAME_BYTES
        || value.len() > MAX_HTTP_HEADER_VALUE_BYTES
        || value
            .chars()
            .any(|character| matches!(character, '\r' | '\n' | '\0'))
    {
        return Err(input_error());
    }
    Ok((name.to_string(), value.to_string()))
}

fn body(input: &Value) -> Result<Vec<u8>, FirstPartyCapabilityError> {
    // Treat a null / empty-string field as absent. Models routinely emit *both*
    // `body` and `body_base64` as `""` defaults for a bodyless request (the
    // schema lists both), which must not trip the mutual-exclusion check or be
    // decoded as a real (empty) body. Only a non-empty value counts as "set".
    let is_set = |key: &str| {
        input
            .get(key)
            .is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
    };
    if is_set("body") && is_set("body_base64") {
        return Err(input_error());
    }
    let body = if is_set("body_base64") {
        let encoded = input
            .get("body_base64")
            .and_then(Value::as_str)
            .ok_or_else(input_error)?;
        BASE64_STANDARD.decode(encoded).map_err(|_| input_error())?
    } else if is_set("body") {
        match input.get("body") {
            Some(Value::String(value)) => value.as_bytes().to_vec(),
            Some(value) => serde_json::to_vec(value).map_err(|_| input_error())?,
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };
    if body.len() as u64 > MAX_NETWORK_EGRESS_BYTES {
        return Err(input_error());
    }
    Ok(body)
}

fn json_body_needs_default_content_type(input: &Value) -> bool {
    matches!(
        input.get("body"),
        Some(Value::Array(_))
            | Some(Value::Bool(_))
            | Some(Value::Number(_))
            | Some(Value::Object(_))
    )
}

fn has_header(headers: &[(String, String)], expected: &str) -> bool {
    headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case(expected))
}

fn staged_policy_placeholder() -> NetworkPolicy {
    // First-party HTTP policy is staged in HostHttpEgressService from the grant
    // obligation for this scope/capability. This fallback request field is
    // ignored on that path and only exists for request-policy test services.
    NetworkPolicy::default()
}

fn response_body_limit(
    input: &Value,
    save_mode: HttpSaveMode,
) -> Result<u64, FirstPartyCapabilityError> {
    let default = match save_mode {
        HttpSaveMode::Disabled => DEFAULT_INLINE_RESPONSE_BODY_LIMIT,
        HttpSaveMode::Required => DEFAULT_SAVE_RESPONSE_BODY_LIMIT,
    };
    let max = match save_mode {
        HttpSaveMode::Disabled => MAX_INLINE_RESPONSE_BODY_LIMIT,
        HttpSaveMode::Required => MAX_SAVE_RESPONSE_BODY_LIMIT,
    };
    let limit = ranged_u64(input, "response_body_limit", default, 1, max)?;
    Ok(limit)
}

fn timeout_ms(input: &Value) -> Result<u32, FirstPartyCapabilityError> {
    let value = ranged_u64(
        input,
        "timeout_ms",
        DEFAULT_HTTP_TIMEOUT_MS.into(),
        1,
        u64::MAX,
    )?;
    let value = value.min(u64::from(MAX_HTTP_TIMEOUT_MS));
    u32::try_from(value).map_err(|_| input_error())
}

fn save_body_to(
    input: &Value,
    mounts: Option<&MountView>,
    mode: HttpSaveMode,
) -> Result<Option<RuntimeHttpSaveTarget>, FirstPartyCapabilityError> {
    let Some(value) = input.get("save_to") else {
        if mode == HttpSaveMode::Required {
            return Err(input_error());
        }
        return Ok(None);
    };
    if mode == HttpSaveMode::Disabled {
        return Err(input_error());
    }
    let path = value.as_str().ok_or_else(input_error)?;
    if path.trim().is_empty() {
        return Err(input_error());
    }
    let mounts = mounts.ok_or_else(input_error)?;
    let path = mounts
        .scoped_path(path.to_string())
        .map_err(|_| input_error())?;
    let (virtual_path, grant) = mounts
        .resolve_with_grant(&path)
        .map_err(|_| input_error())?;
    if !grant.permissions.write {
        return Err(input_error());
    }
    Ok(Some(RuntimeHttpSaveTarget {
        mount_grant: Some(write_only_save_grant(&path, virtual_path)?),
        path,
    }))
}

fn write_only_save_grant(
    path: &ScopedPath,
    virtual_path: VirtualPath,
) -> Result<MountGrant, FirstPartyCapabilityError> {
    Ok(MountGrant::new(
        MountAlias::new(path.as_str()).map_err(|_| input_error())?,
        virtual_path,
        MountPermissions {
            read: false,
            write: true,
            delete: false,
            list: false,
            execute: false,
        },
    ))
}

fn ranged_u64(
    input: &Value,
    field: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, FirstPartyCapabilityError> {
    let Some(value) = input.get(field) else {
        return Ok(default);
    };
    let value = value.as_u64().ok_or_else(input_error)?;
    if value < min || value > max {
        return Err(input_error());
    }
    Ok(value)
}

fn http_error(error: RuntimeHttpEgressError, save_mode: HttpSaveMode) -> FirstPartyCapabilityError {
    let save_error = save_error_kind_and_summary(&error, save_mode);
    let kind = match save_error {
        Some((kind, _summary)) => kind,
        // Host credential injection failures are backend/client integration faults;
        // production maps RuntimeDispatchErrorKind::Client to RuntimeFailureKind::Backend.
        None => match error.reason_code() {
            RuntimeHttpEgressReasonCode::CredentialUnavailable => RuntimeDispatchErrorKind::Client,
            RuntimeHttpEgressReasonCode::RequestDenied => RuntimeDispatchErrorKind::InputEncode,
            RuntimeHttpEgressReasonCode::PolicyDenied => RuntimeDispatchErrorKind::PolicyDenied,
            RuntimeHttpEgressReasonCode::NetworkError => RuntimeDispatchErrorKind::NetworkDenied,
            RuntimeHttpEgressReasonCode::ResponseError => RuntimeDispatchErrorKind::OperationFailed,
            RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
                RuntimeDispatchErrorKind::OutputTooLarge
            }
        },
    };
    let safe_summary = save_error.map(|(_kind, summary)| summary);
    tracing::debug!(
        runtime_http_reason = error.stable_runtime_reason(),
        dispatch_error_kind = kind.as_str(),
        "first-party HTTP egress failed"
    );
    let mut usage = ResourceUsage::default();
    if !matches!(error, RuntimeHttpEgressError::Credential { .. }) {
        usage.network_egress_bytes = error.request_bytes();
    }
    match safe_summary {
        Some(summary) => FirstPartyCapabilityError::with_safe_summary(kind, summary),
        None => FirstPartyCapabilityError::new(kind),
    }
    .with_usage(usage)
}

fn save_error_kind_and_summary(
    error: &RuntimeHttpEgressError,
    save_mode: HttpSaveMode,
) -> Option<(RuntimeDispatchErrorKind, &'static str)> {
    if save_mode != HttpSaveMode::Required {
        return None;
    }
    match error {
        RuntimeHttpEgressError::Request { reason, .. }
            if reason == RESPONSE_BODY_STORE_UNAVAILABLE_REASON =>
        {
            Some((
                RuntimeDispatchErrorKind::Backend,
                SAVE_BODY_STORE_UNAVAILABLE_SUMMARY,
            ))
        }
        RuntimeHttpEgressError::Request { reason, .. }
            if reason == RESPONSE_BODY_STORE_UNAUTHORIZED_REASON =>
        {
            Some((
                RuntimeDispatchErrorKind::FilesystemDenied,
                SAVE_BODY_STORE_UNAUTHORIZED_SUMMARY,
            ))
        }
        RuntimeHttpEgressError::Response { reason, .. }
            if reason == RESPONSE_BODY_STORE_FAILED_REASON =>
        {
            Some((
                RuntimeDispatchErrorKind::OperationFailed,
                SAVE_BODY_STORE_FAILED_SUMMARY,
            ))
        }
        RuntimeHttpEgressError::Network { reason, .. }
        | RuntimeHttpEgressError::Response { reason, .. }
            if reason == RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED =>
        {
            Some((
                RuntimeDispatchErrorKind::OperationFailed,
                SAVE_RESPONSE_BODY_LIMIT_EXCEEDED_SUMMARY,
            ))
        }
        _ => None,
    }
}

fn log_raw_http_input_error_for_local_diagnostics(
    unsafe_raw_diagnostics_allowed: bool,
    input: &Value,
    validation_stage: &'static str,
    error: FirstPartyCapabilityError,
) -> FirstPartyCapabilityError {
    tracing::debug!(
        validation_stage,
        "first-party HTTP tool input validation failed"
    );
    if crate::unsafe_raw_http_diagnostics_enabled(unsafe_raw_diagnostics_allowed) {
        tracing::warn!(
            validation_stage,
            raw_http_tool_input = %input,
            unsafe_raw_diagnostics = true,
            "unsafe raw HTTP tool input diagnostic enabled"
        );
    }
    error
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        CapabilityId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView,
        RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED, ResourceEstimate, ResourceScope,
        RuntimeDispatchErrorKind, RuntimeHttpEgress, RuntimeHttpEgressError,
        RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeHttpSavedBody, TenantId,
        UserId, VirtualPath,
    };
    use serde_json::json;

    use crate::{
        BuiltinFirstPartyTools, FirstPartyCapabilityHandler, FirstPartyCapabilityRequest,
        HostProcessPort, InvocationServices, ToolCallHttpEgress,
    };

    use super::{HTTP_SAVE_CAPABILITY_ID, SAVE_RESPONSE_BODY_LIMIT_EXCEEDED_SUMMARY};

    #[tokio::test]
    async fn save_dispatch_returns_compact_metadata_for_large_saved_body() {
        let tools = BuiltinFirstPartyTools::default();
        let request = save_request(
            json!({
                "url": "https://example.com/large.html",
                "save_to": "/workspace/large.html",
            }),
            Arc::new(LargeSavedBodyEgress),
        );

        let result = tools
            .dispatch(request)
            .await
            .expect("save should return compact metadata");

        assert_eq!(result.output["status"], 200);
        assert_eq!(
            result.output["saved_body"],
            json!({
                "path": "/workspace/large.html",
                "bytes_written": super::super::FIRST_PARTY_MAX_OUTPUT_BYTES + 1,
            })
        );
        assert!(result.output.get("body_text").is_none());
        assert!(result.output.get("body_base64").is_none());
        assert!(result.output.get("body_base64_omitted").is_none());
        assert!(
            result.usage.output_bytes < super::super::FIRST_PARTY_MAX_OUTPUT_BYTES,
            "saved body bytes must not be accounted as model-visible output"
        );
    }

    #[tokio::test]
    async fn save_dispatch_maps_response_limit_to_precise_save_error() {
        let tools = BuiltinFirstPartyTools::default();
        let request = save_request(
            json!({
                "url": "https://example.com/too-large.html",
                "save_to": "/workspace/too-large.html",
            }),
            Arc::new(ResponseLimitEgress),
        );

        let error = tools
            .dispatch(request)
            .await
            .expect_err("save limit breach should fail precisely");

        assert_eq!(
            error.kind(),
            Some(RuntimeDispatchErrorKind::OperationFailed)
        );
        assert_eq!(
            error.safe_summary(),
            Some(SAVE_RESPONSE_BODY_LIMIT_EXCEEDED_SUMMARY)
        );
    }

    fn save_request(
        input: serde_json::Value,
        runtime_http_egress: Arc<dyn RuntimeHttpEgress>,
    ) -> FirstPartyCapabilityRequest {
        FirstPartyCapabilityRequest {
            origin: None,
            capability_id: CapabilityId::new(HTTP_SAVE_CAPABILITY_ID).unwrap(),
            scope: sample_scope(),
            run_id: None,
            authenticated_actor_user_id: None,
            estimate: ResourceEstimate::default(),
            mounts: Some(workspace_mount()),
            services: InvocationServices {
                filesystem: Arc::new(InMemoryBackend::new()),
                runtime_http_egress: Some(runtime_http_egress),
                tool_call_http_egress: Some(Arc::new(PanickingToolCallHttpEgress)),
                runtime_secret_material_stager: None,
                process: Arc::new(HostProcessPort::new()),
                post_edit_check: None,
                secret_store: None,
                audit_sink: None,
                unsafe_raw_diagnostics_allowed: false,
            },
            input,
        }
    }

    fn workspace_mount() -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").unwrap(),
            VirtualPath::new("/projects/project-a").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap()
    }

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-http-save").unwrap(),
            user_id: UserId::new("user-http-save").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    #[derive(Debug)]
    struct LargeSavedBodyEgress;

    #[async_trait]
    impl RuntimeHttpEgress for LargeSavedBodyEgress {
        async fn execute(
            &self,
            mut request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            let target = request
                .save_body_to
                .take()
                .expect("builtin.http.save must pass a save target to strict egress");
            let bytes_written = super::super::FIRST_PARTY_MAX_OUTPUT_BYTES + 1;
            Ok(RuntimeHttpEgressResponse {
                status: 200,
                headers: vec![("content-type".to_string(), "text/html".to_string())],
                body: vec![b'x'; bytes_written as usize],
                saved_body: Some(RuntimeHttpSavedBody {
                    path: target.path,
                    bytes_written,
                }),
                request_bytes: 0,
                response_bytes: bytes_written,
                redaction_applied: false,
            })
        }
    }

    #[derive(Debug)]
    struct ResponseLimitEgress;

    #[async_trait]
    impl RuntimeHttpEgress for ResponseLimitEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            assert!(
                request.save_body_to.is_some(),
                "builtin.http.save must pass a save target to strict egress"
            );
            Err(RuntimeHttpEgressError::Network {
                reason: RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED.to_string(),
                request_bytes: 0,
                response_bytes: request.response_body_limit.unwrap_or_default() + 1,
            })
        }
    }

    #[derive(Debug)]
    struct PanickingToolCallHttpEgress;

    #[async_trait]
    impl ToolCallHttpEgress for PanickingToolCallHttpEgress {
        async fn execute_for_model_visible_output(
            &self,
            _request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            panic!("builtin.http.save must not use model-visible HTTP egress")
        }
    }
}

#[cfg(test)]
mod body_tests {
    use super::body;
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
    use serde_json::json;

    #[test]
    fn empty_string_body_and_body_base64_are_treated_as_absent() {
        // Models commonly emit both schema fields as "" defaults for a bodyless
        // GET. Both empty must be accepted as "no body", not rejected by the
        // mutual-exclusion check.
        let input = json!({ "method": "get", "body": "", "body_base64": "" });
        assert_eq!(
            body(&input).expect("empty fields accepted"),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn absent_body_fields_yield_empty_body() {
        let input = json!({ "method": "get" });
        assert_eq!(body(&input).expect("no body fields"), Vec::<u8>::new());
    }

    #[test]
    fn non_empty_body_is_used() {
        let input = json!({ "method": "post", "body": "hello", "body_base64": "" });
        assert_eq!(body(&input).expect("string body"), b"hello".to_vec());
    }

    #[test]
    fn non_empty_body_base64_is_decoded() {
        let encoded = BASE64_STANDARD.encode("hello");
        let input = json!({ "method": "post", "body": "", "body_base64": encoded });
        assert_eq!(body(&input).expect("base64 body"), b"hello".to_vec());
    }

    #[test]
    fn both_non_empty_is_rejected() {
        let input = json!({ "method": "post", "body": "a", "body_base64": "Yg==" });
        assert!(body(&input).is_err(), "ambiguous body must be rejected");
    }

    #[test]
    fn json_object_body_is_serialized() {
        let input = json!({ "method": "post", "body": { "k": "v" } });
        assert_eq!(body(&input).expect("json body"), br#"{"k":"v"}"#.to_vec());
    }
}

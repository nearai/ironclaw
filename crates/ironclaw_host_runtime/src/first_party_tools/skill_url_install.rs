use std::path::PathBuf;

use ironclaw_host_api::{
    NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern, ResourceUsage,
    RuntimeDispatchErrorKind, RuntimeHttpEgressError, RuntimeHttpEgressReasonCode,
    RuntimeHttpEgressRequest, RuntimeKind,
};

use crate::{FirstPartyCapabilityError, FirstPartyCapabilityRequest};

mod bundle;
mod github;
mod zip_bundle;

const SKILL_URL_RESPONSE_BODY_LIMIT_BYTES: u64 = 10 * 1024 * 1024;
const SKILL_URL_FETCH_TIMEOUT_MS: u32 = 10_000;
const MAX_ZIP_ENTRY_BYTES: u64 = ironclaw_skills::MAX_INSTALL_BUNDLE_FILE_BYTES as u64;
const MAX_TOTAL_UNZIPPED_BYTES: u64 = ironclaw_skills::MAX_INSTALL_BUNDLE_TOTAL_BYTES as u64;
const MAX_GITHUB_PATH_SEGMENTS: usize = 8;
const MAX_GITHUB_CONTENT_API_REQUESTS: usize = 64;
const MAX_GITHUB_CONTENT_API_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ZIP_FILE_ENTRIES: usize = ironclaw_skills::MAX_INSTALL_BUNDLE_FILES * 4;
const ALLOWED_SKILL_URL_HOSTS: [&str; 4] = [
    "api.github.com",
    "codeload.github.com",
    "github.com",
    "raw.githubusercontent.com",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SkillUrlPayload {
    pub(super) content: String,
    pub(super) files: Vec<SkillUrlPayloadFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SkillUrlPayloadFile {
    pub(super) path: PathBuf,
    pub(super) contents: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FetchedBytes {
    pub(super) status: u16,
    pub(super) body: Vec<u8>,
}

pub(super) async fn fetch_skill_url_payload(
    request: &FirstPartyCapabilityRequest,
    url: &str,
    usage: &mut ResourceUsage,
) -> Result<SkillUrlPayload, FirstPartyCapabilityError> {
    let parsed = validate_skill_url(url)?;
    if let Some(payload) = github::fetch_payload_if_supported(request, &parsed, usage).await? {
        return Ok(payload);
    }

    let bytes = fetch_url_bytes(request, &parsed, usage).await?;
    if bytes.starts_with(b"PK\x03\x04") {
        let bundle = zip_bundle::extract_skill_bundle_blocking(bytes, None).await?;
        return Ok(SkillUrlPayload {
            content: bundle.skill_md,
            files: bundle.files,
        });
    }

    Ok(SkillUrlPayload {
        content: String::from_utf8(bytes).map_err(|_| {
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                .with_usage(usage.clone())
        })?,
        files: Vec::new(),
    })
}

async fn fetch_url_bytes(
    request: &FirstPartyCapabilityRequest,
    url: &url::Url,
    usage: &mut ResourceUsage,
) -> Result<Vec<u8>, FirstPartyCapabilityError> {
    fetch_url_bytes_with_headers(request, url, usage, Vec::new()).await
}

async fn fetch_url_bytes_with_headers(
    request: &FirstPartyCapabilityRequest,
    url: &url::Url,
    usage: &mut ResourceUsage,
    headers: Vec<(String, String)>,
) -> Result<Vec<u8>, FirstPartyCapabilityError> {
    let response = fetch_url_response(request, url, usage, headers).await?;
    if !(200..300).contains(&response.status) {
        return Err(
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                .with_usage(usage.clone()),
        );
    }
    Ok(response.body)
}

pub(super) async fn fetch_url_response(
    request: &FirstPartyCapabilityRequest,
    url: &url::Url,
    usage: &mut ResourceUsage,
    headers: Vec<(String, String)>,
) -> Result<FetchedBytes, FirstPartyCapabilityError> {
    let egress = request
        .services
        .runtime_http_egress
        .as_ref()
        .ok_or_else(|| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::NetworkDenied))?
        .clone();
    let http_request = RuntimeHttpEgressRequest {
        runtime: RuntimeKind::FirstParty,
        scope: request.scope.clone(),
        capability_id: request.capability_id.clone(),
        method: NetworkMethod::Get,
        url: url.to_string(),
        headers,
        body: Vec::new(),
        network_policy: skill_url_network_policy(),
        credential_injections: Vec::new(),
        response_body_limit: Some(SKILL_URL_RESPONSE_BODY_LIMIT_BYTES),
        timeout_ms: Some(SKILL_URL_FETCH_TIMEOUT_MS),
    };
    let response = tokio::task::spawn_blocking(move || egress.execute(http_request))
        .await
        .map_err(|error| {
            if error.is_panic() {
                tracing::error!("skill URL fetch egress worker panicked");
            }
            FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::Backend)
        })?
        .map_err(|error| skill_url_fetch_error(error, usage))?;
    usage.network_egress_bytes = usage
        .network_egress_bytes
        .saturating_add(response.request_bytes);
    Ok(FetchedBytes {
        status: response.status,
        body: response.body,
    })
}

fn validate_skill_url(url: &str) -> Result<url::Url, FirstPartyCapabilityError> {
    let parsed = url::Url::parse(url)
        .map_err(|_| FirstPartyCapabilityError::new(RuntimeDispatchErrorKind::InputEncode))?;
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    let Some(host) = parsed.host_str() else {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    };
    if !ALLOWED_SKILL_URL_HOSTS.contains(&host) {
        return Err(FirstPartyCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    Ok(parsed)
}

fn skill_url_network_policy() -> NetworkPolicy {
    NetworkPolicy {
        allowed_targets: ALLOWED_SKILL_URL_HOSTS
            .iter()
            .map(|host| NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: (*host).to_string(),
                port: None,
            })
            .collect(),
        deny_private_ip_ranges: true,
        max_egress_bytes: Some(SKILL_URL_RESPONSE_BODY_LIMIT_BYTES),
    }
}

fn skill_url_fetch_error(
    error: RuntimeHttpEgressError,
    usage: &mut ResourceUsage,
) -> FirstPartyCapabilityError {
    usage.network_egress_bytes = usage
        .network_egress_bytes
        .saturating_add(error.request_bytes());
    let kind = match error.reason_code() {
        RuntimeHttpEgressReasonCode::CredentialUnavailable => RuntimeDispatchErrorKind::Client,
        RuntimeHttpEgressReasonCode::RequestDenied => RuntimeDispatchErrorKind::InputEncode,
        RuntimeHttpEgressReasonCode::PolicyDenied => RuntimeDispatchErrorKind::PolicyDenied,
        RuntimeHttpEgressReasonCode::NetworkError => RuntimeDispatchErrorKind::NetworkDenied,
        RuntimeHttpEgressReasonCode::ResponseError => RuntimeDispatchErrorKind::OperationFailed,
        RuntimeHttpEgressReasonCode::ResponseBodyLimitExceeded => {
            RuntimeDispatchErrorKind::OutputTooLarge
        }
    };
    FirstPartyCapabilityError::new(kind).with_usage(usage.clone())
}

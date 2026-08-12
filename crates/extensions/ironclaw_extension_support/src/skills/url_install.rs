//! HTTPS/GitHub skill-source fetching for `builtin.skill_install`.
//!
//! Moved out of `ironclaw_host_runtime::first_party_tools::skill_url_install`
//! with WS3 (target-architecture CHECKLIST WS3, PROPOSAL §6.8.4). It reaches
//! the network only through the host-declared [`RuntimeHttpEgress`] port
//! supplied per invocation, so it carries no kernel dependency: the host
//! runtime adapts an already-authorized capability request into
//! [`SkillUrlFetchContext`], exactly as it does for the gsuite and web-access
//! executors.

use std::{future::Future, panic::AssertUnwindSafe, path::PathBuf, sync::Arc};

use futures_util::FutureExt as _;
use ironclaw_host_api::{
    action::{NetworkMethod, NetworkPolicy, NetworkScheme, NetworkTargetPattern},
    dispatch::RuntimeDispatchErrorKind,
    http::{
        RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressReasonCode,
        RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
    },
    ids::CapabilityId,
    resource::{ResourceScope, ResourceUsage},
    runtime::RuntimeKind,
};

use crate::skills::SkillManagementCapabilityError;

mod bundle;
mod github;
mod zip_bundle;

/// The host-runtime-free slice of an already-authorized capability request the
/// skill-URL fetch path needs: caller scope, the capability being served, and
/// the mediated egress port. Everything else on the host's dispatch input
/// (mounts, filesystem, secret staging, process ports) is deliberately absent —
/// this path never touches it.
#[derive(Clone)]
pub struct SkillUrlFetchContext {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
}

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
const ALLOWED_CODE_ARTIFACT_HOSTS: [&str; 4] = [
    "github.com",
    "objects.githubusercontent.com",
    "github-releases.githubusercontent.com",
    "raw.githubusercontent.com",
];

/// Return whether `host` is a public code-artifact host recognized by the
/// host-owned URL installation boundary.
///
/// Registry clients may use this classification to narrow their own signed
/// artifact policies without duplicating concrete code-host knowledge in a
/// generic extension package manager.
pub fn is_allowed_code_artifact_host(host: &str) -> bool {
    ALLOWED_CODE_ARTIFACT_HOSTS
        .iter()
        .any(|allowed| host.eq_ignore_ascii_case(allowed))
        || host
            .to_ascii_lowercase()
            .ends_with(".githubusercontent.com")
}

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
    request: &SkillUrlFetchContext,
    url: &str,
    usage: &mut ResourceUsage,
) -> Result<SkillUrlPayload, SkillManagementCapabilityError> {
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
            SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                .with_usage(usage.clone())
        })?,
        files: Vec::new(),
    })
}

async fn fetch_url_bytes(
    request: &SkillUrlFetchContext,
    url: &url::Url,
    usage: &mut ResourceUsage,
) -> Result<Vec<u8>, SkillManagementCapabilityError> {
    fetch_url_bytes_with_headers(request, url, usage, Vec::new()).await
}

async fn fetch_url_bytes_with_headers(
    request: &SkillUrlFetchContext,
    url: &url::Url,
    usage: &mut ResourceUsage,
    headers: Vec<(String, String)>,
) -> Result<Vec<u8>, SkillManagementCapabilityError> {
    let response = fetch_url_response(request, url, usage, headers).await?;
    if !(200..300).contains(&response.status) {
        return Err(
            SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::OperationFailed)
                .with_usage(usage.clone()),
        );
    }
    Ok(response.body)
}

pub(super) async fn fetch_url_response(
    request: &SkillUrlFetchContext,
    url: &url::Url,
    usage: &mut ResourceUsage,
    headers: Vec<(String, String)>,
) -> Result<FetchedBytes, SkillManagementCapabilityError> {
    let egress = request
        .runtime_http_egress
        .as_ref()
        .ok_or_else(|| {
            SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::NetworkDenied)
        })?
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
        save_body_to: None,
        timeout_ms: Some(SKILL_URL_FETCH_TIMEOUT_MS),
    };
    let response = run_egress_catching_panic(
        egress.execute(http_request),
        "skill URL HTTP egress future panicked",
        || {
            SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::Backend)
                .with_usage(usage.clone())
        },
    )
    .await?
    .map_err(|error| skill_url_fetch_error(error, usage))?;
    usage.network_egress_bytes = usage
        .network_egress_bytes
        .saturating_add(response.request_bytes);
    Ok(FetchedBytes {
        status: response.status,
        body: response.body,
    })
}

fn validate_skill_url(url: &str) -> Result<url::Url, SkillManagementCapabilityError> {
    let parsed = url::Url::parse(url)
        .map_err(|_| SkillManagementCapabilityError::new(RuntimeDispatchErrorKind::InputEncode))?;
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(SkillManagementCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    }
    let Some(host) = parsed.host_str() else {
        return Err(SkillManagementCapabilityError::new(
            RuntimeDispatchErrorKind::InputEncode,
        ));
    };
    if !ALLOWED_SKILL_URL_HOSTS.contains(&host) {
        return Err(SkillManagementCapabilityError::new(
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

/// Run a mediated-egress future, converting a panic into a `Backend` dispatch
/// failure instead of unwinding through the capability boundary. Mirrors the
/// host runtime's own `run_egress_catching_panic`, which the builtin HTTP tool
/// still uses; the two are independent because the crates no longer share a
/// dependency edge.
async fn run_egress_catching_panic<F, P>(
    future: F,
    panic_message: &'static str,
    on_panic: P,
) -> Result<Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>, SkillManagementCapabilityError>
where
    F: Future<Output = Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError>>,
    P: FnOnce() -> SkillManagementCapabilityError,
{
    AssertUnwindSafe(future).catch_unwind().await.map_err(|_| {
        tracing::error!("{panic_message}");
        on_panic()
    })
}

fn skill_url_fetch_error(
    error: RuntimeHttpEgressError,
    usage: &mut ResourceUsage,
) -> SkillManagementCapabilityError {
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
    SkillManagementCapabilityError::new(kind).with_usage(usage.clone())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_host_api::{
        http::{RuntimeHttpEgress, RuntimeHttpEgressResponse},
        ids::{CapabilityId, InvocationId, TenantId, UserId},
        resource::ResourceScope,
    };

    use super::*;

    #[test]
    fn code_artifact_hosts_cover_release_and_raw_downloads_only() {
        assert!(is_allowed_code_artifact_host("github.com"));
        assert!(is_allowed_code_artifact_host(
            "release-assets.githubusercontent.com"
        ));
        assert!(!is_allowed_code_artifact_host("api.github.com"));
        assert!(!is_allowed_code_artifact_host("github.example"));
    }

    #[tokio::test]
    async fn fetch_url_response_maps_panicking_runtime_egress_to_backend_failure() {
        let request = SkillUrlFetchContext {
            capability_id: CapabilityId::new("builtin.skill_install").unwrap(),
            scope: sample_scope(),
            runtime_http_egress: Some(Arc::new(PanickingRuntimeHttpEgress)),
        };
        let url = validate_skill_url(
            "https://raw.githubusercontent.com/Pika-Labs/Pika-Skills/main/fetched-helper/SKILL.md",
        )
        .unwrap();
        let mut usage = ResourceUsage::default();

        let error = fetch_url_response(&request, &url, &mut usage, Vec::new())
            .await
            .unwrap_err();

        assert_eq!(error.kind(), RuntimeDispatchErrorKind::Backend);
        assert_eq!(error.usage(), Some(&ResourceUsage::default()));
    }

    #[derive(Debug)]
    struct PanickingRuntimeHttpEgress;

    #[async_trait]
    impl RuntimeHttpEgress for PanickingRuntimeHttpEgress {
        async fn execute(
            &self,
            _request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
            panic!("skill URL runtime HTTP egress panic")
        }
    }

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant1").unwrap(),
            user_id: UserId::new("user1").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }
}

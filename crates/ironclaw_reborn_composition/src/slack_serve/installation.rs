//! Slack installation resolution and post-auth installation-scoped ingress policy.

use std::collections::HashMap;
use std::future::Future;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use ironclaw_host_api::TenantId;
use ironclaw_product_adapters::{AdapterInstallationId, ProtocolAuthEvidence};
use ironclaw_slack_v2_adapter::{SlackPayloadParseError, parse_slack_url_verification_challenge};
use ironclaw_wasm_product_adapters::RunnerError;
use serde::Deserialize;
use thiserror::Error;

use super::{
    SlackEventsWebhookDispatcher, SlackWebhookErrorCategory, error_response,
    ingress_error_response, runner_error_response,
};

const SLACK_INSTALLATION_MAX_REQUESTS: NonZeroU32 = NonZeroU32::new(120).unwrap(); // safety: 120 requests is a non-zero literal.
const SLACK_INSTALLATION_RATE_WINDOW: Duration = Duration::from_secs(60);
const MAX_SLACK_METADATA_PAYLOAD_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackEnvelopeMetadata {
    pub team_id: Option<String>,
    pub enterprise_id: Option<String>,
    pub api_app_id: Option<String>,
    pub install_user_id: Option<String>,
    pub event_user_id: Option<String>,
    pub event_channel_id: Option<String>,
}

impl SlackEnvelopeMetadata {
    fn empty() -> Self {
        Self {
            team_id: None,
            enterprise_id: None,
            api_app_id: None,
            install_user_id: None,
            event_user_id: None,
            event_channel_id: None,
        }
    }
}

fn parse_slack_envelope_metadata(
    raw_payload: &[u8],
    auth_evidence: &ProtocolAuthEvidence,
) -> Result<SlackEnvelopeMetadata, SlackPayloadParseError> {
    if !auth_evidence.is_verified() {
        return Err(SlackPayloadParseError::UnauthenticatedPayload);
    }
    if raw_payload.len() > MAX_SLACK_METADATA_PAYLOAD_BYTES {
        return Err(SlackPayloadParseError::InvalidJson {
            reason: "payload exceeds size limit".into(),
        });
    }
    let wrapper: SlackEnvelopeMetadataWrapper =
        serde_json::from_slice(raw_payload).map_err(|err| SlackPayloadParseError::InvalidJson {
            reason: err.to_string(),
        })?;
    Ok(wrapper.into_metadata())
}

#[derive(Debug, Clone, Deserialize)]
struct SlackEnvelopeMetadataWrapper {
    team_id: Option<String>,
    enterprise_id: Option<String>,
    context_team_id: Option<String>,
    context_enterprise_id: Option<String>,
    api_app_id: Option<String>,
    event: Option<SlackEnvelopeEventMetadata>,
    #[serde(default)]
    authorizations: Vec<SlackAuthorizationMetadata>,
}

impl SlackEnvelopeMetadataWrapper {
    fn into_metadata(self) -> SlackEnvelopeMetadata {
        let authorization = self.authorizations.into_iter().next();
        SlackEnvelopeMetadata {
            team_id: authorization
                .as_ref()
                .and_then(|auth| auth.team_id.clone())
                .or(self.context_team_id)
                .or(self.team_id),
            enterprise_id: authorization
                .as_ref()
                .and_then(|auth| auth.enterprise_id.clone())
                .or(self.context_enterprise_id)
                .or(self.enterprise_id),
            api_app_id: self.api_app_id,
            install_user_id: authorization.as_ref().and_then(|auth| auth.user_id.clone()),
            event_user_id: self.event.as_ref().and_then(|event| event.user.clone()),
            event_channel_id: self.event.and_then(|event| event.channel),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct SlackEnvelopeEventMetadata {
    user: Option<String>,
    channel: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct SlackAuthorizationMetadata {
    team_id: Option<String>,
    enterprise_id: Option<String>,
    user_id: Option<String>,
}

#[derive(Clone)]
pub struct ResolvedSlackInstallation {
    tenant_id: TenantId,
    adapter_installation_id: AdapterInstallationId,
    metadata: SlackEnvelopeMetadata,
    evidence: ProtocolAuthEvidence,
    dispatcher: Arc<dyn SlackEventsWebhookDispatcher>,
}

impl ResolvedSlackInstallation {
    pub fn new(
        tenant_id: TenantId,
        adapter_installation_id: AdapterInstallationId,
        metadata: SlackEnvelopeMetadata,
        evidence: ProtocolAuthEvidence,
        dispatcher: Arc<dyn SlackEventsWebhookDispatcher>,
    ) -> Self {
        Self {
            tenant_id,
            adapter_installation_id,
            metadata,
            evidence,
            dispatcher,
        }
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn adapter_installation_id(&self) -> &AdapterInstallationId {
        &self.adapter_installation_id
    }

    pub fn metadata(&self) -> &SlackEnvelopeMetadata {
        &self.metadata
    }

    pub fn evidence(&self) -> &ProtocolAuthEvidence {
        &self.evidence
    }

    pub fn dispatcher(&self) -> Arc<dyn SlackEventsWebhookDispatcher> {
        Arc::clone(&self.dispatcher)
    }
}

impl std::fmt::Debug for ResolvedSlackInstallation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedSlackInstallation")
            .field("tenant_id", &self.tenant_id)
            .field("adapter_installation_id", &self.adapter_installation_id)
            .field("metadata", &self.metadata)
            .field("dispatcher", &"Arc<dyn SlackEventsWebhookDispatcher>")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SlackIngressError {
    #[error(transparent)]
    Runner(#[from] RunnerError),
    #[error(transparent)]
    Envelope(#[from] SlackPayloadParseError),
    #[error("no verified Slack installation matched the signed envelope")]
    InstallationNotFound,
    #[error("multiple verified Slack installations matched the signed envelope")]
    AmbiguousInstallation,
    #[error(
        "Slack installation rate limit exceeded for tenant {tenant_id} installation {adapter_installation_id}"
    )]
    InstallationRateLimited {
        tenant_id: TenantId,
        adapter_installation_id: AdapterInstallationId,
    },
}

pub trait SlackInstallationResolver: Send + Sync {
    fn resolve_installation<'a>(
        &'a self,
        headers: &'a HeaderMap,
        body: &'a [u8],
    ) -> Pin<
        Box<dyn Future<Output = Result<ResolvedSlackInstallation, SlackIngressError>> + Send + 'a>,
    >;

    fn drain_installations<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

#[derive(Clone)]
pub struct SlackInstallationRecord {
    tenant_id: TenantId,
    adapter_installation_id: AdapterInstallationId,
    selector: SlackInstallationSelector,
    dispatcher: Arc<dyn SlackEventsWebhookDispatcher>,
}

impl SlackInstallationRecord {
    pub fn new(
        tenant_id: TenantId,
        adapter_installation_id: AdapterInstallationId,
        selector: SlackInstallationSelector,
        dispatcher: Arc<dyn SlackEventsWebhookDispatcher>,
    ) -> Self {
        Self {
            tenant_id,
            adapter_installation_id,
            selector,
            dispatcher,
        }
    }
}

impl std::fmt::Debug for SlackInstallationRecord {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackInstallationRecord")
            .field("tenant_id", &self.tenant_id)
            .field("adapter_installation_id", &self.adapter_installation_id)
            .field("selector", &self.selector)
            .field("dispatcher", &"Arc<dyn SlackEventsWebhookDispatcher>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackInstallationSelector {
    team_id: Option<String>,
    enterprise_id: Option<String>,
    api_app_id: Option<String>,
    install_user_id: Option<String>,
}

impl SlackInstallationSelector {
    pub fn new(
        team_id: Option<String>,
        enterprise_id: Option<String>,
        api_app_id: Option<String>,
    ) -> Self {
        Self {
            team_id,
            enterprise_id,
            api_app_id,
            install_user_id: None,
        }
    }

    pub fn team(team_id: impl Into<String>) -> Self {
        Self::new(Some(team_id.into()), None, None)
    }

    pub fn with_install_user_id(mut self, install_user_id: impl Into<String>) -> Self {
        self.install_user_id = Some(install_user_id.into());
        self
    }

    fn matches(&self, metadata: &SlackEnvelopeMetadata) -> bool {
        selector_part_matches(self.team_id.as_deref(), metadata.team_id.as_deref())
            && selector_part_matches(
                self.enterprise_id.as_deref(),
                metadata.enterprise_id.as_deref(),
            )
            && selector_part_matches(self.api_app_id.as_deref(), metadata.api_app_id.as_deref())
            && selector_part_matches(
                self.install_user_id.as_deref(),
                metadata.install_user_id.as_deref(),
            )
            && (self.team_id.is_some()
                || self.enterprise_id.is_some()
                || self.api_app_id.is_some()
                || self.install_user_id.is_some())
    }
}

fn selector_part_matches(configured: Option<&str>, observed: Option<&str>) -> bool {
    match configured {
        Some(configured) => observed == Some(configured),
        None => true,
    }
}

#[derive(Debug, Clone)]
pub struct StaticSlackInstallationResolver {
    installations: Vec<SlackInstallationRecord>,
}

impl StaticSlackInstallationResolver {
    pub fn new(installations: impl IntoIterator<Item = SlackInstallationRecord>) -> Self {
        Self {
            installations: installations.into_iter().collect(),
        }
    }

    fn resolve_sync(
        &self,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Result<ResolvedSlackInstallation, SlackIngressError> {
        let mut auth_failure: Option<RunnerError> = None;
        let mut verified = Vec::new();
        for installation in &self.installations {
            match installation.dispatcher.verify_webhook_auth(headers, body) {
                Ok(evidence) => verified.push((installation, evidence)),
                Err(error) => {
                    auth_failure.get_or_insert(error);
                }
            };
        }

        if verified.is_empty() {
            return Err(auth_failure
                .unwrap_or(RunnerError::AuthenticationFailed {
                    failure: ironclaw_product_adapters::ProtocolAuthFailure::Missing,
                })
                .into());
        }

        if parse_slack_url_verification_challenge(body, &verified[0].1)?.is_some() {
            let (installation, evidence) = verified.remove(0);
            return Ok(ResolvedSlackInstallation::new(
                installation.tenant_id.clone(),
                installation.adapter_installation_id.clone(),
                SlackEnvelopeMetadata::empty(),
                evidence,
                Arc::clone(&installation.dispatcher),
            ));
        }

        let metadata = parse_slack_envelope_metadata(body, &verified[0].1)?;
        let mut matches = verified
            .into_iter()
            .filter(|(installation, _)| installation.selector.matches(&metadata));
        let Some((installation, evidence)) = matches.next() else {
            return Err(SlackIngressError::InstallationNotFound);
        };
        if matches.next().is_some() {
            return Err(SlackIngressError::AmbiguousInstallation);
        }

        Ok(ResolvedSlackInstallation::new(
            installation.tenant_id.clone(),
            installation.adapter_installation_id.clone(),
            metadata,
            evidence,
            Arc::clone(&installation.dispatcher),
        ))
    }
}

impl SlackInstallationResolver for StaticSlackInstallationResolver {
    fn resolve_installation<'a>(
        &'a self,
        headers: &'a HeaderMap,
        body: &'a [u8],
    ) -> Pin<
        Box<dyn Future<Output = Result<ResolvedSlackInstallation, SlackIngressError>> + Send + 'a>,
    > {
        Box::pin(async move { self.resolve_sync(headers, body) })
    }

    fn drain_installations<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            for installation in &self.installations {
                installation.dispatcher.drain_immediate_ack_tasks().await;
            }
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlackInstallationRateLimitConfig {
    pub max_requests: NonZeroU32,
    pub window: Duration,
}

impl SlackInstallationRateLimitConfig {
    pub fn new(max_requests: NonZeroU32, window: Duration) -> Self {
        Self {
            max_requests,
            window,
        }
    }
}

impl Default for SlackInstallationRateLimitConfig {
    fn default() -> Self {
        Self {
            max_requests: SLACK_INSTALLATION_MAX_REQUESTS,
            window: SLACK_INSTALLATION_RATE_WINDOW,
        }
    }
}

#[derive(Clone)]
struct SlackInstallationRateLimiter {
    config: SlackInstallationRateLimitConfig,
    buckets: Arc<Mutex<HashMap<SlackInstallationRateLimitKey, SlackRateLimitBucket>>>,
}

impl SlackInstallationRateLimiter {
    fn new(config: SlackInstallationRateLimitConfig) -> Self {
        Self {
            config,
            buckets: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn check(&self, installation: &ResolvedSlackInstallation) -> Result<(), SlackIngressError> {
        let now = Instant::now();
        let key = SlackInstallationRateLimitKey {
            tenant_id: installation.tenant_id.clone(),
            adapter_installation_id: installation.adapter_installation_id.clone(),
        };
        let mut buckets = match self.buckets.lock() {
            Ok(buckets) => buckets,
            Err(poisoned) => poisoned.into_inner(),
        };
        let bucket = buckets.entry(key).or_insert(SlackRateLimitBucket {
            window_started_at: now,
            requests: 0,
        });
        if now.duration_since(bucket.window_started_at) >= self.config.window {
            bucket.window_started_at = now;
            bucket.requests = 0;
        }
        if bucket.requests >= self.config.max_requests.get() {
            return Err(SlackIngressError::InstallationRateLimited {
                tenant_id: installation.tenant_id.clone(),
                adapter_installation_id: installation.adapter_installation_id.clone(),
            });
        }
        bucket.requests += 1;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SlackInstallationRateLimitKey {
    tenant_id: TenantId,
    adapter_installation_id: AdapterInstallationId,
}

#[derive(Debug, Clone)]
struct SlackRateLimitBucket {
    window_started_at: Instant,
    requests: u32,
}

#[derive(Clone)]
pub struct SlackIngressService {
    resolver: Arc<dyn SlackInstallationResolver>,
    installation_rate_limiter: SlackInstallationRateLimiter,
}

impl SlackIngressService {
    pub fn new(resolver: Arc<dyn SlackInstallationResolver>) -> Self {
        Self::with_rate_limit_config(resolver, SlackInstallationRateLimitConfig::default())
    }

    pub fn with_rate_limit_config(
        resolver: Arc<dyn SlackInstallationResolver>,
        rate_limit: SlackInstallationRateLimitConfig,
    ) -> Self {
        Self {
            resolver,
            installation_rate_limiter: SlackInstallationRateLimiter::new(rate_limit),
        }
    }

    pub(super) async fn handle_events(&self, headers: HeaderMap, body: Bytes) -> Response {
        let installation = match self
            .resolver
            .resolve_installation(&headers, body.as_ref())
            .await
        {
            Ok(installation) => installation,
            Err(error) => return ingress_error_response(error),
        };
        if let Err(error) = self.installation_rate_limiter.check(&installation) {
            return ingress_error_response(error);
        }

        match parse_slack_url_verification_challenge(body.as_ref(), installation.evidence()) {
            Ok(Some(challenge)) => return (StatusCode::OK, challenge.challenge).into_response(),
            Ok(None) => {}
            Err(error) => {
                tracing::debug!(
                    target = "ironclaw::reborn::slack_events",
                    tenant_id = %installation.tenant_id(),
                    adapter_installation_id = %installation.adapter_installation_id(),
                    error = %error,
                    "Slack URL verification parse failed"
                );
                return error_response(
                    StatusCode::BAD_REQUEST,
                    SlackWebhookErrorCategory::MalformedPayload,
                );
            }
        }

        match installation
            .dispatcher()
            .process_verified_webhook_immediate_ack(body.as_ref(), installation.evidence())
            .await
        {
            Ok(_) => (StatusCode::OK, "ok").into_response(),
            Err(error) => runner_error_response(error),
        }
    }

    pub async fn drain_installations(&self) {
        self.resolver.drain_installations().await;
    }
}

impl std::fmt::Debug for SlackIngressService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackIngressService")
            .field("resolver", &"Arc<dyn SlackInstallationResolver>")
            .field(
                "installation_rate_limiter",
                &self.installation_rate_limiter.config,
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

    use axum::http::HeaderMap;
    use ironclaw_product_adapters::auth::mark_request_signature_verified;
    use ironclaw_wasm_product_adapters::WebhookProcessOutcome;

    use super::*;

    struct AlwaysVerifiedDispatcher {
        subject: &'static str,
    }

    impl SlackEventsWebhookDispatcher for AlwaysVerifiedDispatcher {
        fn verify_webhook_auth(
            &self,
            _headers: &HeaderMap,
            _body: &[u8],
        ) -> Result<ProtocolAuthEvidence, RunnerError> {
            Ok(mark_request_signature_verified(
                "X-Slack-Signature",
                Some("X-Slack-Request-Timestamp".to_string()),
                self.subject,
            ))
        }

        fn process_verified_webhook_immediate_ack<'a>(
            &'a self,
            _body: &'a [u8],
            _evidence: &'a ProtocolAuthEvidence,
        ) -> Pin<Box<dyn Future<Output = Result<WebhookProcessOutcome, RunnerError>> + Send + 'a>>
        {
            Box::pin(async { Ok(WebhookProcessOutcome::AcceptedForAsyncDispatch) })
        }

        fn drain_immediate_ack_tasks<'a>(
            &'a self,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async {})
        }
    }

    fn verified() -> ProtocolAuthEvidence {
        mark_request_signature_verified(
            "X-Slack-Signature",
            Some("X-Slack-Request-Timestamp".to_string()),
            "slack-install",
        )
    }

    fn tenant_id(value: &str) -> TenantId {
        TenantId::new(value).expect("valid tenant")
    }

    fn installation_id(value: &str) -> AdapterInstallationId {
        AdapterInstallationId::new(value).expect("valid installation")
    }

    fn dispatcher(subject: &'static str) -> Arc<dyn SlackEventsWebhookDispatcher> {
        Arc::new(AlwaysVerifiedDispatcher { subject })
    }

    #[test]
    fn envelope_metadata_prefers_authorization_context_for_slack_connect() {
        let metadata = parse_slack_envelope_metadata(
            br#"{
                "type": "event_callback",
                "team_id": "T-external",
                "context_team_id": "T-context",
                "api_app_id": "A-slack",
                "authorizations": [{
                    "team_id": "T-install",
                    "enterprise_id": "E-install",
                    "user_id": "U-install"
                }],
                "event": {"type": "message", "user": "U-external", "channel": "C-shared"}
            }"#,
            &verified(),
        )
        .expect("metadata parses");

        assert_eq!(metadata.team_id.as_deref(), Some("T-install"));
        assert_eq!(metadata.enterprise_id.as_deref(), Some("E-install"));
        assert_eq!(metadata.install_user_id.as_deref(), Some("U-install"));
        assert_eq!(metadata.event_user_id.as_deref(), Some("U-external"));
    }

    #[tokio::test]
    async fn static_resolver_allows_url_verification_before_selector_matching() {
        let resolver = StaticSlackInstallationResolver::new(vec![SlackInstallationRecord::new(
            tenant_id("tenant-a"),
            installation_id("install-a"),
            SlackInstallationSelector::team("T-A"),
            dispatcher("install-a"),
        )]);

        let installation = resolver
            .resolve_installation(
                &HeaderMap::new(),
                br#"{"type":"url_verification","challenge":"challenge-token"}"#,
            )
            .await
            .expect("url verification resolves before selector matching");

        assert_eq!(installation.tenant_id().as_str(), "tenant-a");
        assert_eq!(installation.adapter_installation_id().as_str(), "install-a");
        assert_eq!(installation.metadata().team_id, None);
    }

    #[tokio::test]
    async fn static_resolver_disambiguates_same_workspace_by_authorization_user() {
        let resolver = StaticSlackInstallationResolver::new(vec![
            SlackInstallationRecord::new(
                tenant_id("tenant-a"),
                installation_id("install-a"),
                SlackInstallationSelector::team("T-shared").with_install_user_id("U-install-a"),
                dispatcher("install-a"),
            ),
            SlackInstallationRecord::new(
                tenant_id("tenant-b"),
                installation_id("install-b"),
                SlackInstallationSelector::team("T-shared").with_install_user_id("U-install-b"),
                dispatcher("install-b"),
            ),
        ]);

        let installation = resolver
            .resolve_installation(
                &HeaderMap::new(),
                br#"{
                    "type":"event_callback",
                    "team_id":"T-shared",
                    "api_app_id":"A-slack",
                    "authorizations":[{"team_id":"T-shared","user_id":"U-install-b"}],
                    "event":{"type":"message","user":"U-event","channel":"D123"}
                }"#,
            )
            .await
            .expect("authorization user disambiguates install");

        assert_eq!(installation.tenant_id().as_str(), "tenant-b");
        assert_eq!(installation.adapter_installation_id().as_str(), "install-b");
        assert_eq!(
            installation.metadata().install_user_id.as_deref(),
            Some("U-install-b")
        );
    }
}

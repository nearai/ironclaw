//! Bundled hook implementations and declarative hook registration.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::hooks::{
    Hook, HookContext, HookError, HookEvent, HookFailureMode, HookOutcome, HookPoint, HookRegistry,
};

const DEFAULT_RULE_PRIORITY: u32 = 100;
const DEFAULT_WEBHOOK_PRIORITY: u32 = 300;
const DEFAULT_WEBHOOK_TIMEOUT_MS: u64 = 2000;
const DEFAULT_WEBHOOK_MAX_IN_FLIGHT: usize = 32;
const MAX_HOOK_TIMEOUT_MS: u64 = 30_000;

const ALL_HOOK_POINTS: [HookPoint; 6] = [
    HookPoint::BeforeInbound,
    HookPoint::BeforeToolCall,
    HookPoint::BeforeOutbound,
    HookPoint::OnSessionStart,
    HookPoint::OnSessionEnd,
    HookPoint::TransformResponse,
];

/// Errors while parsing or compiling declarative hook bundles.
#[derive(Debug, thiserror::Error)]
pub enum HookBundleError {
    #[error("Invalid hook bundle format: {0}")]
    InvalidFormat(String),

    #[error("Hook '{hook}' must declare at least one hook point")]
    MissingHookPoints { hook: String },

    #[error("Hook '{hook}' has invalid regex '{pattern}': {reason}")]
    InvalidRegex {
        hook: String,
        pattern: String,
        reason: String,
    },

    #[error("Hook '{hook}' timeout must be between 1 and {max_ms} ms")]
    InvalidTimeout { hook: String, max_ms: u64 },

    #[error("Outbound webhook hook '{hook}' has invalid url: {url}")]
    InvalidWebhookUrl { hook: String, url: String },

    #[error("Outbound webhook hook '{hook}' must use https, got '{scheme}'")]
    InvalidWebhookScheme { hook: String, scheme: String },

    #[error("Outbound webhook hook '{hook}' cannot target host '{host}'")]
    ForbiddenWebhookHost { hook: String, host: String },

    #[error("Outbound webhook hook '{hook}' has invalid header '{header}': {reason}")]
    InvalidWebhookHeader {
        hook: String,
        header: String,
        reason: String,
    },

    #[error("Outbound webhook hook '{hook}' cannot set restricted header '{header}'")]
    ForbiddenWebhookHeader { hook: String, header: String },

    #[error("Outbound webhook hook '{hook}' max_in_flight must be at least 1")]
    InvalidWebhookMaxInFlight { hook: String },
}

/// A declarative hook bundle loaded from workspace files or extension capabilities.
///
/// Supports two bundled hook types:
/// - Rule hooks (`rules`) for reject/regex transform/prepend/append logic
/// - Outbound webhook hooks (`outbound_webhooks`) for fire-and-forget event delivery
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookBundleConfig {
    /// Declarative content/tool/session rules.
    #[serde(default)]
    pub rules: Vec<HookRuleConfig>,
    /// Fire-and-forget webhook notifications on selected hook points.
    #[serde(default)]
    pub outbound_webhooks: Vec<OutboundWebhookConfig>,
}

impl HookBundleConfig {
    /// Parse a hook bundle from JSON value.
    ///
    /// Accepts either:
    /// - object form: `{ "rules": [...], "outbound_webhooks": [...] }`
    /// - array form:  `[ {rule}, {rule} ]` (shorthand for rules only)
    pub fn from_value(value: &serde_json::Value) -> Result<Self, HookBundleError> {
        if value.is_array() {
            let rules: Vec<HookRuleConfig> = serde_json::from_value(value.clone())
                .map_err(|e| HookBundleError::InvalidFormat(e.to_string()))?;
            return Ok(Self {
                rules,
                outbound_webhooks: Vec::new(),
            });
        }

        serde_json::from_value(value.clone())
            .map_err(|e| HookBundleError::InvalidFormat(e.to_string()))
    }
}

/// Summary of hook registrations performed from a bundle.
#[derive(Debug, Default, Clone, Copy)]
pub struct HookRegistrationSummary {
    /// Number of non-webhook hook registrations (audit/rule hooks).
    pub hooks: usize,
    /// Number of outbound webhook hook registrations.
    pub outbound_webhooks: usize,
    /// Number of invalid/failed registrations skipped.
    pub errors: usize,
}

impl HookRegistrationSummary {
    /// Total number of hooks successfully registered.
    pub fn total_registered(&self) -> usize {
        self.hooks + self.outbound_webhooks
    }

    pub fn merge(&mut self, other: HookRegistrationSummary) {
        self.hooks += other.hooks;
        self.outbound_webhooks += other.outbound_webhooks;
        self.errors += other.errors;
    }
}

/// Register bundled built-in hooks that ship with IronClaw.
pub async fn register_bundled_hooks(registry: &Arc<HookRegistry>) -> HookRegistrationSummary {
    registry
        .register_with_priority(Arc::new(AuditLogHook), 25)
        .await;

    HookRegistrationSummary {
        hooks: 1,
        outbound_webhooks: 0,
        errors: 0,
    }
}

/// Register all hooks from a declarative bundle.
pub async fn register_bundle(
    registry: &Arc<HookRegistry>,
    source: &str,
    bundle: HookBundleConfig,
) -> HookRegistrationSummary {
    let mut summary = HookRegistrationSummary::default();

    for rule in bundle.rules {
        match RuleHook::from_config(source, rule) {
            Ok((hook, priority)) => {
                registry
                    .register_with_priority(Arc::new(hook), priority)
                    .await;
                summary.hooks += 1;
            }
            Err(err) => {
                summary.errors += 1;
                tracing::warn!(source = source, error = %err, "Skipping invalid declarative hook rule");
            }
        }
    }

    for webhook in bundle.outbound_webhooks {
        match OutboundWebhookHook::from_config(source, webhook) {
            Ok((hook, priority)) => {
                registry
                    .register_with_priority(Arc::new(hook), priority)
                    .await;
                summary.outbound_webhooks += 1;
            }
            Err(err) => {
                summary.errors += 1;
                tracing::warn!(source = source, error = %err, "Skipping invalid outbound webhook hook");
            }
        }
    }

    summary
}

/// Declarative regex/string rule hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookRuleConfig {
    /// Stable hook name (scoped with source during registration).
    pub name: String,
    /// Lifecycle points where this rule applies.
    pub points: Vec<HookPoint>,
    /// Optional priority override (lower runs first).
    #[serde(default)]
    pub priority: Option<u32>,
    /// Failure handling mode (default fail_open).
    #[serde(default)]
    pub failure_mode: Option<HookFailureMode>,
    /// Optional timeout override for this hook in milliseconds.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Optional regex guard. If provided and no match, rule is a no-op.
    #[serde(default)]
    pub when_regex: Option<String>,
    /// Optional immediate reject reason if guard matches.
    #[serde(default)]
    pub reject_reason: Option<String>,
    /// Regex replacements applied in order.
    #[serde(default)]
    pub replacements: Vec<RegexReplacementConfig>,
    /// Text prepended to the event's primary content.
    #[serde(default)]
    pub prepend: Option<String>,
    /// Text appended to the event's primary content.
    #[serde(default)]
    pub append: Option<String>,
}

/// A single regex replacement step in a rule hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegexReplacementConfig {
    pub pattern: String,
    pub replacement: String,
}

/// Declarative fire-and-forget outbound webhook hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundWebhookConfig {
    /// Stable webhook hook name (scoped with source during registration).
    pub name: String,
    /// Lifecycle points that trigger this webhook.
    pub points: Vec<HookPoint>,
    /// Target URL.
    pub url: String,
    /// Optional static headers.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Optional timeout override in milliseconds.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Optional priority override (lower runs first).
    #[serde(default)]
    pub priority: Option<u32>,
    /// Optional max number of concurrent in-flight deliveries.
    #[serde(default)]
    pub max_in_flight: Option<usize>,
}

/// Built-in audit trail hook that logs lifecycle events.
struct AuditLogHook;

#[async_trait]
impl Hook for AuditLogHook {
    fn name(&self) -> &str {
        "builtin.audit_log"
    }

    fn hook_points(&self) -> &[HookPoint] {
        &ALL_HOOK_POINTS
    }

    async fn execute(
        &self,
        event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        tracing::debug!(
            target: "hooks::audit",
            hook = self.name(),
            point = event.hook_point().as_str(),
            user_id = %event_user_id(event),
            "Lifecycle hook event"
        );

        Ok(HookOutcome::ok())
    }
}

#[derive(Debug, Clone)]
struct CompiledReplacement {
    regex: Regex,
    replacement: String,
}

/// Runtime hook compiled from [`HookRuleConfig`].
#[derive(Debug)]
struct RuleHook {
    name: String,
    points: Vec<HookPoint>,
    failure_mode: HookFailureMode,
    timeout: Duration,
    when_regex: Option<Regex>,
    reject_reason: Option<String>,
    replacements: Vec<CompiledReplacement>,
    prepend: Option<String>,
    append: Option<String>,
}

impl RuleHook {
    fn from_config(source: &str, config: HookRuleConfig) -> Result<(Self, u32), HookBundleError> {
        let scoped_name = format!("{}::{}", source, config.name);

        if config.points.is_empty() {
            return Err(HookBundleError::MissingHookPoints { hook: scoped_name });
        }

        let timeout = timeout_from_ms(config.timeout_ms, &scoped_name)?;

        let when_regex = match config.when_regex {
            Some(pattern) => {
                Some(
                    Regex::new(&pattern).map_err(|e| HookBundleError::InvalidRegex {
                        hook: scoped_name.clone(),
                        pattern,
                        reason: e.to_string(),
                    })?,
                )
            }
            None => None,
        };

        let mut replacements = Vec::with_capacity(config.replacements.len());
        for replacement in config.replacements {
            let compiled =
                Regex::new(&replacement.pattern).map_err(|e| HookBundleError::InvalidRegex {
                    hook: scoped_name.clone(),
                    pattern: replacement.pattern.clone(),
                    reason: e.to_string(),
                })?;

            replacements.push(CompiledReplacement {
                regex: compiled,
                replacement: replacement.replacement,
            });
        }

        if when_regex.is_some()
            && config.reject_reason.is_none()
            && replacements.is_empty()
            && config.prepend.as_deref().is_none()
            && config.append.as_deref().is_none()
        {
            tracing::warn!(
                hook = %scoped_name,
                "Rule hook has a guard but no actions; it will always no-op"
            );
        }

        let hook = Self {
            name: scoped_name,
            points: config.points,
            failure_mode: config.failure_mode.unwrap_or(HookFailureMode::FailOpen),
            timeout,
            when_regex,
            reject_reason: config.reject_reason,
            replacements,
            prepend: config.prepend,
            append: config.append,
        };

        Ok((hook, config.priority.unwrap_or(DEFAULT_RULE_PRIORITY)))
    }
}

#[async_trait]
impl Hook for RuleHook {
    fn name(&self) -> &str {
        &self.name
    }

    fn hook_points(&self) -> &[HookPoint] {
        &self.points
    }

    fn failure_mode(&self) -> HookFailureMode {
        self.failure_mode
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    async fn execute(
        &self,
        event: &HookEvent,
        _ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        let content = extract_primary_content(event);

        if let Some(ref guard) = self.when_regex
            && !guard.is_match(&content)
        {
            return Ok(HookOutcome::ok());
        }

        if let Some(ref reason) = self.reject_reason {
            return Ok(HookOutcome::reject(reason.clone()));
        }

        let mut modified = content.clone();

        for replacement in &self.replacements {
            modified = replacement
                .regex
                .replace_all(&modified, replacement.replacement.as_str())
                .into_owned();
        }

        if let Some(ref prefix) = self.prepend {
            modified = format!("{}{}", prefix, modified);
        }

        if let Some(ref suffix) = self.append {
            modified.push_str(suffix);
        }

        if modified != content {
            Ok(HookOutcome::modify(modified))
        } else {
            Ok(HookOutcome::ok())
        }
    }
}

/// Runtime outbound webhook hook.
#[derive(Debug)]
struct OutboundWebhookHook {
    name: String,
    points: Vec<HookPoint>,
    client: reqwest::Client,
    url: String,
    headers: HeaderMap,
    timeout: Duration,
    semaphore: Arc<Semaphore>,
}

impl OutboundWebhookHook {
    fn from_config(
        source: &str,
        config: OutboundWebhookConfig,
    ) -> Result<(Self, u32), HookBundleError> {
        let scoped_name = format!("{}::{}", source, config.name);

        if config.points.is_empty() {
            return Err(HookBundleError::MissingHookPoints { hook: scoped_name });
        }

        let url = validate_webhook_url(&scoped_name, &config.url)?;
        let headers = validate_webhook_headers(&scoped_name, &config.headers)?;

        let timeout = timeout_from_ms(
            config.timeout_ms.or(Some(DEFAULT_WEBHOOK_TIMEOUT_MS)),
            &scoped_name,
        )?;

        let max_in_flight = config
            .max_in_flight
            .unwrap_or(DEFAULT_WEBHOOK_MAX_IN_FLIGHT);
        if max_in_flight == 0 {
            return Err(HookBundleError::InvalidWebhookMaxInFlight { hook: scoped_name });
        }

        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| HookBundleError::InvalidFormat(e.to_string()))?;

        let hook = Self {
            name: scoped_name,
            points: config.points,
            client,
            url: url.to_string(),
            headers,
            timeout,
            semaphore: Arc::new(Semaphore::new(max_in_flight)),
        };

        Ok((hook, config.priority.unwrap_or(DEFAULT_WEBHOOK_PRIORITY)))
    }
}

#[derive(Debug, Serialize)]
struct OutboundWebhookPayload {
    hook: String,
    point: String,
    timestamp: String,
    event: OutboundWebhookEventSummary,
    metadata_present: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum OutboundWebhookEventSummary {
    Inbound {
        channel: String,
        has_thread_id: bool,
        content_length: usize,
    },
    ToolCall {
        tool_name: String,
        context: String,
        parameter_count: usize,
    },
    Outbound {
        channel: String,
        has_thread_id: bool,
        content_length: usize,
    },
    SessionStart,
    SessionEnd,
    ResponseTransform {
        response_length: usize,
    },
}

#[async_trait]
impl Hook for OutboundWebhookHook {
    fn name(&self) -> &str {
        &self.name
    }

    fn hook_points(&self) -> &[HookPoint] {
        &self.points
    }

    fn timeout(&self) -> Duration {
        self.timeout
    }

    async fn execute(
        &self,
        event: &HookEvent,
        ctx: &HookContext,
    ) -> Result<HookOutcome, HookError> {
        let payload = OutboundWebhookPayload {
            hook: self.name.clone(),
            point: event.hook_point().as_str().to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            event: summarize_webhook_event(event),
            metadata_present: !ctx.metadata.is_null(),
        };

        let permit = match self.semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                tracing::warn!(
                    hook = %self.name,
                    "Dropping outbound webhook delivery due to concurrency limit"
                );
                return Ok(HookOutcome::ok());
            }
        };

        let base_client = self.client.clone();
        let url = self.url.clone();
        let headers = self.headers.clone();
        let hook_name = self.name.clone();
        let timeout = self.timeout;

        tokio::spawn(async move {
            let _permit = permit;

            let client = match dispatch_client_for_target(&base_client, &url, timeout).await {
                Ok(client) => client,
                Err(err) => {
                    tracing::warn!(
                        hook = %hook_name,
                        error = %err,
                        "Outbound webhook target blocked by runtime network policy"
                    );
                    return;
                }
            };

            let request = client.post(url).headers(headers).json(&payload);

            if let Err(err) = request.send().await {
                tracing::warn!(
                    hook = %hook_name,
                    error = %err,
                    "Outbound webhook delivery failed"
                );
            }
        });

        Ok(HookOutcome::ok())
    }
}

fn summarize_webhook_event(event: &HookEvent) -> OutboundWebhookEventSummary {
    match event {
        HookEvent::Inbound {
            channel,
            content,
            thread_id,
            ..
        } => OutboundWebhookEventSummary::Inbound {
            channel: channel.clone(),
            has_thread_id: thread_id.is_some(),
            content_length: content.len(),
        },
        HookEvent::ToolCall {
            tool_name,
            context,
            parameters,
            ..
        } => OutboundWebhookEventSummary::ToolCall {
            tool_name: tool_name.clone(),
            context: context.clone(),
            parameter_count: match parameters {
                serde_json::Value::Object(map) => map.len(),
                serde_json::Value::Null => 0,
                _ => 1,
            },
        },
        HookEvent::Outbound {
            channel,
            content,
            thread_id,
            ..
        } => OutboundWebhookEventSummary::Outbound {
            channel: channel.clone(),
            has_thread_id: thread_id.is_some(),
            content_length: content.len(),
        },
        HookEvent::SessionStart { .. } => OutboundWebhookEventSummary::SessionStart,
        HookEvent::SessionEnd { .. } => OutboundWebhookEventSummary::SessionEnd,
        HookEvent::ResponseTransform { response, .. } => {
            OutboundWebhookEventSummary::ResponseTransform {
                response_length: response.len(),
            }
        }
    }
}

fn validate_webhook_url(hook_name: &str, url: &str) -> Result<reqwest::Url, HookBundleError> {
    let parsed = reqwest::Url::parse(url).map_err(|_| HookBundleError::InvalidWebhookUrl {
        hook: hook_name.to_string(),
        url: url.to_string(),
    })?;

    if parsed.scheme() != "https" {
        return Err(HookBundleError::InvalidWebhookScheme {
            hook: hook_name.to_string(),
            scheme: parsed.scheme().to_string(),
        });
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(HookBundleError::InvalidWebhookUrl {
            hook: hook_name.to_string(),
            url: url.to_string(),
        });
    }

    if let Some(host) = parsed.host_str() {
        if is_forbidden_webhook_host(host) {
            return Err(HookBundleError::ForbiddenWebhookHost {
                hook: hook_name.to_string(),
                host: host.to_string(),
            });
        }

        if let Ok(ip) = host.parse::<IpAddr>()
            && is_forbidden_ip(ip)
        {
            return Err(HookBundleError::ForbiddenWebhookHost {
                hook: hook_name.to_string(),
                host: host.to_string(),
            });
        }
    }

    Ok(parsed)
}

async fn dispatch_client_for_target(
    base_client: &reqwest::Client,
    url: &str,
    timeout: Duration,
) -> Result<reqwest::Client, String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "Webhook URL has no host".to_string())?;

    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_forbidden_ip(ip) {
            return Err(format!("Webhook target resolves to blocked IP {ip}"));
        }
        return Ok(base_client.clone());
    }

    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "Webhook URL has no valid port".to_string())?;

    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("DNS resolution failed: {e}"))?
        .collect();

    if addrs.is_empty() {
        return Err("DNS resolution returned no addresses".to_string());
    }

    for addr in &addrs {
        if is_forbidden_ip(addr.ip()) {
            return Err(format!("Webhook target resolves to blocked IP {}", addr.ip()));
        }
    }

    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &addrs)
        .build()
        .map_err(|e| format!("Failed to build resolved webhook client: {e}"))
}

fn validate_webhook_headers(
    hook_name: &str,
    headers: &HashMap<String, String>,
) -> Result<HeaderMap, HookBundleError> {
    let mut validated = HeaderMap::new();

    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|e| {
            HookBundleError::InvalidWebhookHeader {
                hook: hook_name.to_string(),
                header: name.clone(),
                reason: e.to_string(),
            }
        })?;

        if is_forbidden_header(header_name.as_str()) {
            return Err(HookBundleError::ForbiddenWebhookHeader {
                hook: hook_name.to_string(),
                header: name.clone(),
            });
        }

        let header_value =
            HeaderValue::from_str(value).map_err(|e| HookBundleError::InvalidWebhookHeader {
                hook: hook_name.to_string(),
                header: name.clone(),
                reason: e.to_string(),
            })?;

        validated.insert(header_name, header_value);
    }

    Ok(validated)
}

fn is_forbidden_webhook_host(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    lower == "localhost"
        || lower.ends_with(".localhost")
        || lower == "host.docker.internal"
        || lower == "metadata.google.internal"
        || lower == "metadata.aws.internal"
}

fn is_forbidden_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
            {
                return true;
            }

            let octets = v4.octets();

            // Carrier-grade NAT range (100.64.0.0/10).
            if octets[0] == 100 && (64..=127).contains(&octets[1]) {
                return true;
            }

            // Benchmark testing range (198.18.0.0/15).
            if octets[0] == 198 && matches!(octets[1], 18 | 19) {
                return true;
            }

            false
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_multicast()
            {
                return true;
            }

            // Documentation range (2001:db8::/32).
            let segments = v6.segments();
            segments[0] == 0x2001 && segments[1] == 0x0db8
        }
    }
}

fn is_forbidden_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "host"
        || lower == "authorization"
        || lower == "cookie"
        || lower == "proxy-authorization"
        || lower == "forwarded"
        || lower == "x-real-ip"
        || lower == "transfer-encoding"
        || lower == "connection"
        || lower.starts_with("x-forwarded-")
}

fn timeout_from_ms(timeout_ms: Option<u64>, hook_name: &str) -> Result<Duration, HookBundleError> {
    if let Some(ms) = timeout_ms {
        if ms == 0 || ms > MAX_HOOK_TIMEOUT_MS {
            return Err(HookBundleError::InvalidTimeout {
                hook: hook_name.to_string(),
                max_ms: MAX_HOOK_TIMEOUT_MS,
            });
        }
        Ok(Duration::from_millis(ms))
    } else {
        Ok(Duration::from_secs(5))
    }
}

fn event_user_id(event: &HookEvent) -> &str {
    match event {
        HookEvent::Inbound { user_id, .. }
        | HookEvent::ToolCall { user_id, .. }
        | HookEvent::Outbound { user_id, .. }
        | HookEvent::SessionStart { user_id, .. }
        | HookEvent::SessionEnd { user_id, .. }
        | HookEvent::ResponseTransform { user_id, .. } => user_id,
    }
}

fn extract_primary_content(event: &HookEvent) -> String {
    match event {
        HookEvent::Inbound { content, .. } | HookEvent::Outbound { content, .. } => content.clone(),
        HookEvent::ToolCall { parameters, .. } => {
            serde_json::to_string(parameters).unwrap_or_default()
        }
        HookEvent::SessionStart { session_id, .. } | HookEvent::SessionEnd { session_id, .. } => {
            session_id.clone()
        }
        HookEvent::ResponseTransform { response, .. } => response.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inbound_event(content: &str) -> HookEvent {
        HookEvent::Inbound {
            user_id: "user-1".to_string(),
            channel: "test".to_string(),
            content: content.to_string(),
            thread_id: None,
        }
    }

    #[test]
    fn test_parse_bundle_array_shorthand() {
        let value = serde_json::json!([
            {
                "name": "append-bang",
                "points": ["beforeInbound"],
                "append": "!"
            }
        ]);

        let parsed = HookBundleConfig::from_value(&value).unwrap();
        assert_eq!(parsed.rules.len(), 1);
        assert!(parsed.outbound_webhooks.is_empty());
    }

    #[tokio::test]
    async fn test_rule_hook_modifies_content() {
        let registry = Arc::new(HookRegistry::new());

        let bundle = HookBundleConfig {
            rules: vec![HookRuleConfig {
                name: "redact-secret".to_string(),
                points: vec![HookPoint::BeforeInbound],
                priority: None,
                failure_mode: None,
                timeout_ms: None,
                when_regex: None,
                reject_reason: None,
                replacements: vec![RegexReplacementConfig {
                    pattern: "secret".to_string(),
                    replacement: "[redacted]".to_string(),
                }],
                prepend: None,
                append: None,
            }],
            outbound_webhooks: vec![],
        };

        let summary = register_bundle(&registry, "workspace:hooks/hooks.json", bundle).await;
        assert_eq!(summary.hooks, 1);
        assert_eq!(summary.errors, 0);

        let result = registry
            .run(&inbound_event("contains secret here"))
            .await
            .unwrap();
        match result {
            HookOutcome::Continue {
                modified: Some(value),
            } => {
                assert_eq!(value, "contains [redacted] here");
            }
            other => panic!("expected modified output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_rule_hook_rejects() {
        let registry = Arc::new(HookRegistry::new());

        let bundle = HookBundleConfig {
            rules: vec![HookRuleConfig {
                name: "block-forbidden".to_string(),
                points: vec![HookPoint::BeforeInbound],
                priority: None,
                failure_mode: None,
                timeout_ms: None,
                when_regex: Some("forbidden".to_string()),
                reject_reason: Some("forbidden content".to_string()),
                replacements: vec![],
                prepend: None,
                append: None,
            }],
            outbound_webhooks: vec![],
        };

        let summary = register_bundle(&registry, "plugin:tool:test", bundle).await;
        assert_eq!(summary.hooks, 1);

        let result = registry.run(&inbound_event("this is forbidden")).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            HookError::Rejected { reason } if reason == "forbidden content"
        ));
    }

    #[tokio::test]
    async fn test_outbound_webhook_hook_registers() {
        let registry = Arc::new(HookRegistry::new());

        let bundle = HookBundleConfig {
            rules: vec![],
            outbound_webhooks: vec![OutboundWebhookConfig {
                name: "notify".to_string(),
                points: vec![HookPoint::BeforeInbound],
                url: "https://example.com/hook".to_string(),
                headers: HashMap::new(),
                timeout_ms: Some(1000),
                priority: None,
                max_in_flight: None,
            }],
        };

        let summary = register_bundle(&registry, "workspace:hooks/webhook.hook.json", bundle).await;
        assert_eq!(summary.outbound_webhooks, 1);

        // Should return immediately regardless of webhook delivery result.
        let result = registry.run(&inbound_event("hello")).await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_timeout_from_ms_rejects_zero() {
        let err = timeout_from_ms(Some(0), "hook").unwrap_err();
        assert!(matches!(err, HookBundleError::InvalidTimeout { .. }));
    }

    #[test]
    fn test_timeout_from_ms_rejects_above_limit() {
        let err = timeout_from_ms(Some(30_001), "hook").unwrap_err();
        assert!(matches!(err, HookBundleError::InvalidTimeout { .. }));
    }

    #[test]
    fn test_rule_hook_requires_points() {
        let config = HookRuleConfig {
            name: "invalid".to_string(),
            points: vec![],
            priority: None,
            failure_mode: None,
            timeout_ms: None,
            when_regex: None,
            reject_reason: None,
            replacements: vec![],
            prepend: None,
            append: None,
        };

        let err = RuleHook::from_config("workspace:hooks/hooks.json", config).unwrap_err();
        assert!(matches!(err, HookBundleError::MissingHookPoints { .. }));
    }

    #[test]
    fn test_invalid_webhook_scheme_rejected() {
        let config = OutboundWebhookConfig {
            name: "notify".to_string(),
            points: vec![HookPoint::BeforeInbound],
            url: "http://example.com/hook".to_string(),
            headers: HashMap::new(),
            timeout_ms: None,
            priority: None,
            max_in_flight: None,
        };

        let err =
            OutboundWebhookHook::from_config("workspace:hooks/hooks.json", config).unwrap_err();
        assert!(matches!(err, HookBundleError::InvalidWebhookScheme { .. }));
    }

    #[test]
    fn test_private_webhook_host_rejected() {
        let config = OutboundWebhookConfig {
            name: "notify".to_string(),
            points: vec![HookPoint::BeforeInbound],
            url: "https://127.0.0.1/hook".to_string(),
            headers: HashMap::new(),
            timeout_ms: None,
            priority: None,
            max_in_flight: None,
        };

        let err =
            OutboundWebhookHook::from_config("workspace:hooks/hooks.json", config).unwrap_err();
        assert!(matches!(err, HookBundleError::ForbiddenWebhookHost { .. }));
    }

    #[test]
    fn test_restricted_webhook_header_rejected() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer token".to_string());

        let config = OutboundWebhookConfig {
            name: "notify".to_string(),
            points: vec![HookPoint::BeforeInbound],
            url: "https://example.com/hook".to_string(),
            headers,
            timeout_ms: None,
            priority: None,
            max_in_flight: None,
        };

        let err =
            OutboundWebhookHook::from_config("workspace:hooks/hooks.json", config).unwrap_err();
        assert!(matches!(
            err,
            HookBundleError::ForbiddenWebhookHeader { .. }
        ));
    }

    #[test]
    fn test_zero_max_in_flight_rejected() {
        let config = OutboundWebhookConfig {
            name: "notify".to_string(),
            points: vec![HookPoint::BeforeInbound],
            url: "https://example.com/hook".to_string(),
            headers: HashMap::new(),
            timeout_ms: None,
            priority: None,
            max_in_flight: Some(0),
        };

        let err =
            OutboundWebhookHook::from_config("workspace:hooks/hooks.json", config).unwrap_err();
        assert!(matches!(
            err,
            HookBundleError::InvalidWebhookMaxInFlight { .. }
        ));
    }

    #[tokio::test]
    async fn test_runtime_target_validation_blocks_private_ip() {
        let base_client = reqwest::Client::builder().build().unwrap();
        let err = dispatch_client_for_target(
            &base_client,
            "https://127.0.0.1/hook",
            Duration::from_secs(1),
        )
            .await
            .unwrap_err();
        assert!(err.contains("blocked IP"));
    }

    #[tokio::test]
    async fn test_runtime_target_validation_allows_public_ip() {
        let base_client = reqwest::Client::builder().build().unwrap();
        let result =
            dispatch_client_for_target(&base_client, "https://1.1.1.1/hook", Duration::from_secs(1))
                .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_rule_guard_no_match_is_passthrough() {
        let registry = Arc::new(HookRegistry::new());

        let bundle = HookBundleConfig {
            rules: vec![HookRuleConfig {
                name: "guarded-rewrite".to_string(),
                points: vec![HookPoint::BeforeInbound],
                priority: None,
                failure_mode: None,
                timeout_ms: None,
                when_regex: Some("forbidden".to_string()),
                reject_reason: None,
                replacements: vec![RegexReplacementConfig {
                    pattern: "hello".to_string(),
                    replacement: "hi".to_string(),
                }],
                prepend: None,
                append: None,
            }],
            outbound_webhooks: vec![],
        };

        register_bundle(&registry, "workspace:hooks/hooks.json", bundle).await;
        let result = registry.run(&inbound_event("hello world")).await.unwrap();
        assert!(matches!(result, HookOutcome::Continue { modified: None }));
    }

    #[tokio::test]
    async fn test_rule_hook_combined_actions() {
        let registry = Arc::new(HookRegistry::new());

        let bundle = HookBundleConfig {
            rules: vec![HookRuleConfig {
                name: "combined".to_string(),
                points: vec![HookPoint::BeforeInbound],
                priority: None,
                failure_mode: None,
                timeout_ms: None,
                when_regex: None,
                reject_reason: None,
                replacements: vec![RegexReplacementConfig {
                    pattern: "secret".to_string(),
                    replacement: "safe".to_string(),
                }],
                prepend: Some("[".to_string()),
                append: Some("]".to_string()),
            }],
            outbound_webhooks: vec![],
        };

        register_bundle(&registry, "workspace:hooks/hooks.json", bundle).await;
        let result = registry.run(&inbound_event("secret")).await.unwrap();
        match result {
            HookOutcome::Continue {
                modified: Some(value),
            } => assert_eq!(value, "[safe]"),
            other => panic!("expected modified output, got {other:?}"),
        }
    }
}

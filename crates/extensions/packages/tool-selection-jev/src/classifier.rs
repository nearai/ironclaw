//! The classifier: slice, ask concurrently, merge, take the top N.

use std::{
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use futures::future::try_join_all;
use ironclaw_host_api::{
    action::{NetworkMethod, NetworkPolicy},
    resource::ResourceScope,
};
use ironclaw_loop_contracts::{
    ChosenTool, LoopSafeSummary, ToolSelection, ToolSelectionCandidate, ToolSelectionClassifier,
    ToolSelectionError, ToolSelectionRequest,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, PolicyNetworkHttpEgress,
    ReqwestNetworkTransport,
};
use tracing::debug;
use zeroize::Zeroizing;

use crate::{
    endpoint::JevEndpoint,
    request::{TokenLimits, plan_slices, slice_body},
    response::parse_answer,
};

/// The default model: TypeSafe's `jev-latest` alias, its flagship model. It
/// moves between Jev releases; an operator who needs reproducible selections
/// names a pinned version instead.
pub const DEFAULT_JEV_MODEL: &str = "jev-latest";

/// Name this classifier reports in logs.
pub const JEV_CLASSIFIER_NAME: &str = "jev";

/// `tracing` target shared with the loop host's selection logs.
const LOG_TARGET: &str = "ironclaw::reborn::tool_selection";

/// Largest response body read. One answer per tool is a few dozen bytes.
const RESPONSE_BODY_LIMIT: u64 = 1024 * 1024;

/// The Jev provider's API key. Zeroed on drop and never printed.
pub struct JevApiKey(Zeroizing<String>);

impl JevApiKey {
    /// Wrap a key read host-side from the environment variable the operator
    /// named. Refuses an empty key.
    pub fn new(value: impl Into<String>) -> Result<Self, JevConfigError> {
        let value = Zeroizing::new(value.into());
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(JevConfigError::EmptyApiKey);
        }
        Ok(Self(Zeroizing::new(trimmed.to_string())))
    }
}

impl fmt::Debug for JevApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JevApiKey(<redacted>)")
    }
}

/// Why a [`JevToolClassifier`] could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum JevConfigError {
    #[error("the Jev model name must not be empty")]
    EmptyModel,
    #[error("the Jev API key must not be empty")]
    EmptyApiKey,
    #[error("the Jev endpoint is refused: {reason}")]
    InvalidEndpoint { reason: &'static str },
    #[error("the Jev timeout must be greater than zero")]
    ZeroTimeout,
}

/// Jev, served by the configured decisions endpoint, behind the
/// `ToolSelectionClassifier` port.
pub struct JevToolClassifier {
    model: String,
    api_key: JevApiKey,
    timeout: Duration,
    endpoint: String,
    policy: NetworkPolicy,
    egress: Arc<dyn NetworkHttpEgress>,
}

impl fmt::Debug for JevToolClassifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JevToolClassifier")
            .field("model", &self.model)
            .field("endpoint", &self.endpoint)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl JevToolClassifier {
    /// A classifier for `model` that posts to `endpoint` through the host's
    /// policy egress, pinned to that endpoint's host, giving up on a whole
    /// classification (every slice) after `timeout`.
    pub fn new(
        endpoint: JevEndpoint,
        model: impl Into<String>,
        api_key: JevApiKey,
        timeout: Duration,
    ) -> Result<Self, JevConfigError> {
        let model = model.into().trim().to_string();
        if model.is_empty() {
            return Err(JevConfigError::EmptyModel);
        }
        if timeout.is_zero() {
            return Err(JevConfigError::ZeroTimeout);
        }
        Ok(Self {
            model,
            api_key,
            timeout,
            policy: endpoint.policy(),
            endpoint: endpoint.url().to_string(),
            egress: Arc::new(PolicyNetworkHttpEgress::new(ReqwestNetworkTransport::new(
                timeout,
            ))),
        })
    }

    /// The model every request names.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// How many requests (slices) classifying `request` sends.
    pub fn slice_count(&self, request: &ToolSelectionRequest) -> usize {
        planned_slices(request).len()
    }

    /// Ask one slice and return its probabilities in slice order.
    async fn ask_slice(
        &self,
        request: &ToolSelectionRequest,
        slice: &[usize],
    ) -> Result<Vec<f32>, ToolSelectionError> {
        let body = slice_body(&self.model, &request.context, &request.candidates, slice);
        let asked: Vec<&str> = slice
            .iter()
            .filter_map(|index| request.candidates.get(*index))
            .map(ToolSelectionCandidate::name)
            .collect();
        let response = self
            .egress
            .execute(NetworkHttpRequest {
                scope: ResourceScope::system(),
                method: NetworkMethod::Post,
                url: self.endpoint.clone(),
                headers: vec![
                    (
                        "authorization".to_string(),
                        format!("Bearer {}", self.api_key.0.as_str()),
                    ),
                    ("content-type".to_string(), "application/json".to_string()),
                ],
                body: body.to_string().into_bytes(),
                policy: self.policy.clone(),
                response_body_limit: Some(RESPONSE_BODY_LIMIT),
                // Saturates: a timeout past `u32::MAX` ms is already unbounded.
                timeout_ms: Some(u32::try_from(self.timeout.as_millis()).unwrap_or(u32::MAX)),
            })
            .await
            .map_err(egress_failure)?;
        match response.status {
            200..=299 => parse_answer(&response.body, &asked).map_err(|error| {
                debug!(
                    target: LOG_TARGET,
                    classifier = JEV_CLASSIFIER_NAME,
                    error = ?error,
                    "Jev classification response was unusable"
                );
                invalid_output(error.summary())
            }),
            401 | 403 => Err(ToolSelectionError::Unauthorized),
            402 => Err(ToolSelectionError::PaymentRequired),
            // Overload: `429` (rate limit), `503` (model at capacity), `529`
            // (overloaded). Not retried: the host falls back.
            429 | 503 | 529 => Err(ToolSelectionError::RateLimited),
            // Any other server or gateway failure: the service is down, not
            // refusing this request.
            status @ 500..=599 => Err(unavailable(format!(
                "the classification service failed (status {status})"
            ))),
            status => Err(ToolSelectionError::Rejected { status }),
        }
    }
}

/// Send `classifier`'s requests to `endpoint` under `policy` instead of the
/// configured HTTPS endpoint and its pin. Dev-only: for a loopback stub
/// server, which plain HTTP to a loopback address needs.
#[cfg(feature = "test-support")]
pub fn with_stub_endpoint(
    mut classifier: JevToolClassifier,
    endpoint: impl Into<String>,
    policy: NetworkPolicy,
) -> JevToolClassifier {
    classifier.endpoint = endpoint.into();
    classifier.policy = policy;
    classifier
}

fn unavailable(summary: String) -> ToolSelectionError {
    ToolSelectionError::Unavailable {
        reason: LoopSafeSummary::capability_failure_summary(summary),
    }
}

fn invalid_output(summary: &str) -> ToolSelectionError {
    ToolSelectionError::InvalidOutput {
        reason: LoopSafeSummary::capability_failure_summary(summary),
    }
}

/// Map a failed exchange with the service. A body past
/// [`RESPONSE_BODY_LIMIT`] cannot be a valid answer, so it is
/// `invalid_output`; everything else (connection, transport, policy) means
/// the service was not reached.
fn egress_failure(error: NetworkHttpError) -> ToolSelectionError {
    let error_kind = error.stable_reason();
    debug!(
        target: LOG_TARGET,
        classifier = JEV_CLASSIFIER_NAME,
        error_kind,
        "Jev classification exchange failed"
    );
    if matches!(error, NetworkHttpError::ResponseBodyLimit { .. }) {
        return invalid_output("the classification response exceeded the size limit");
    }
    unavailable(format!(
        "the classification service could not be reached ({error_kind})"
    ))
}

/// The slices a classification of `request` sends: none when nothing may be
/// chosen.
fn planned_slices(request: &ToolSelectionRequest) -> Vec<Vec<usize>> {
    if request.max_tools == 0 {
        return Vec::new();
    }
    plan_slices(&request.context, &request.candidates, TokenLimits::DEFAULT)
}

/// The candidates by probability, highest first (ties in catalog order):
/// at most `max_tools` of them, stopping at the first one that would take
/// the chosen tools' schema tokens past `token_budget`.
fn choose(request: &ToolSelectionRequest, probabilities: &[f32]) -> Vec<ChosenTool> {
    let mut ranked: Vec<(&ToolSelectionCandidate, f32)> = request
        .candidates
        .iter()
        .zip(probabilities.iter().copied())
        .collect();
    // Stable: equal probabilities keep catalog order.
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
    let mut chosen = Vec::new();
    let mut tokens = 0_u32;
    for (candidate, probability) in ranked.into_iter().take(request.max_tools) {
        tokens = tokens.saturating_add(candidate.est_schema_tokens);
        if tokens > request.token_budget {
            break;
        }
        chosen.push(ChosenTool::new(candidate.name(), probability));
    }
    chosen
}

#[async_trait]
impl ToolSelectionClassifier for JevToolClassifier {
    fn classifier_name(&self) -> &str {
        JEV_CLASSIFIER_NAME
    }

    async fn classify(
        &self,
        request: &ToolSelectionRequest,
    ) -> Result<ToolSelection, ToolSelectionError> {
        let started = Instant::now();
        let slices = planned_slices(request);
        // Every slice concurrently, under one deadline; the first failure
        // cancels the rest, since a partial vector must never be ranked.
        let asked = try_join_all(slices.iter().map(|slice| self.ask_slice(request, slice)));
        let timed_out = ToolSelectionError::Timeout {
            elapsed: self.timeout,
        };
        let outcome = match tokio::time::timeout(self.timeout, asked).await {
            // The transport's own timeout can fire just before this one.
            Err(_) => Err(timed_out),
            Ok(Err(ToolSelectionError::Unavailable { .. }))
                if started.elapsed() >= self.timeout =>
            {
                Err(timed_out)
            }
            Ok(outcome) => outcome,
        };
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let answers = outcome.inspect_err(|error| {
            debug!(
                target: LOG_TARGET,
                classifier = JEV_CLASSIFIER_NAME,
                model = %self.model,
                error_kind = error.kind_label(),
                slices = slices.len(),
                latency_ms,
                "Jev tool classification failed"
            );
        })?;

        // Merge the slices into one probability per candidate before choosing.
        let mut probabilities = vec![0.0_f32; request.candidates.len()];
        for (index, probability) in slices.iter().flatten().zip(answers.into_iter().flatten()) {
            if let Some(slot) = probabilities.get_mut(*index) {
                *slot = probability;
            }
        }
        let chosen = choose(request, &probabilities);
        let logged: Vec<(&str, f32)> = chosen
            .iter()
            .map(|tool| (tool.name.as_str(), tool.score))
            .collect();
        debug!(
            target: LOG_TARGET,
            classifier = JEV_CLASSIFIER_NAME,
            model = %self.model,
            chosen = ?logged,
            candidates = request.candidates.len(),
            slices = slices.len(),
            latency_ms,
            "Jev scored the candidate tools"
        );
        Ok(ToolSelection {
            chosen,
            // Provider-neutral: the same model scores on the same scale
            // whichever provider serves it.
            scorer: format!("{JEV_CLASSIFIER_NAME}:{}", self.model),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_settings_are_validated_the_pin_is_the_endpoints_and_the_key_never_prints() {
        let build = |model: &str, timeout| {
            let key = JevApiKey::new(" jev-secret-value ").expect("key");
            let endpoint = JevEndpoint::parse("https://jev.example.test:8443/v1").expect("url");
            JevToolClassifier::new(endpoint, model, key, timeout)
        };
        let error = |result: Result<JevToolClassifier, JevConfigError>| result.err();
        let empty_key = JevApiKey::new("   ").err();
        assert_eq!(empty_key, Some(JevConfigError::EmptyApiKey));
        let one_second = Duration::from_secs(1);
        let empty_model = error(build(" ", one_second));
        assert_eq!(empty_model, Some(JevConfigError::EmptyModel));
        let no_timeout = error(build("jev-latest", Duration::ZERO));
        assert_eq!(no_timeout, Some(JevConfigError::ZeroTimeout));

        let classifier = build(DEFAULT_JEV_MODEL, one_second).expect("classifier");
        assert_eq!(classifier.model(), "jev-latest");
        assert_eq!(classifier.api_key.0.as_str(), "jev-secret-value");
        assert_eq!(classifier.endpoint, "https://jev.example.test:8443/v1");
        let pin = &classifier.policy.allowed_targets;
        assert_eq!(pin.len(), 1);
        assert_eq!(pin[0].host_pattern, "jev.example.test");
        assert_eq!(pin[0].port, Some(8443));
        let printed = format!("{classifier:?} {:?}", classifier.api_key);
        assert!(!printed.contains("secret"), "{printed}");
    }
}

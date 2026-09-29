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
    ChosenTool, LoopSafeSummary, ToolSelection, ToolSelectionClassifier, ToolSelectionError,
    ToolSelectionRequest,
};
use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpRequest, PolicyNetworkHttpEgress, ReqwestNetworkTransport,
};
use tracing::debug;
use zeroize::Zeroizing;

use crate::{
    endpoint::JevEndpoint,
    request::{plan_slices, slice_body},
    response::{SliceAnswer, parse_answer},
};

/// The default model: TypeSafe's `jev-latest` alias, its flagship model. It
/// moves between Jev releases, so the classifier logs the model the server reports answering
/// with (`served_model`); an operator who needs reproducible selections pins
/// a version in `[tool_selection.jev] model`.
pub const DEFAULT_JEV_MODEL: &str = "jev-latest";

/// Most estimated tokens one request (conversation, tool entries and
/// questions) may carry. The limits behind it are TypeSafe's published
/// figures for `jev-1.13.0`: 64k tokens per request and 32k for the state
/// plus its longest question. This stays well inside both, because the
/// estimate is only an estimate. Another provider may publish other limits;
/// [`JevToolClassifier::with_max_slice_tokens`] changes the budget.
pub const DEFAULT_MAX_SLICE_TOKENS: usize = 24_000;

/// Name this classifier reports in logs.
pub const JEV_CLASSIFIER_NAME: &str = "jev";

/// `tracing` target shared with the loop host's selection logs.
const LOG_TARGET: &str = "ironclaw::reborn::tool_prefetch";

/// Largest response body read. One answer per tool is a few dozen bytes.
const RESPONSE_BODY_LIMIT: u64 = 1024 * 1024;

/// Waits between retries of an overload status (`429`, `503`, `529`),
/// doubling up to the maximum.
/// A `retry-after` header is honoured instead when the response has one.
const FIRST_BACKOFF: Duration = Duration::from_millis(50);
const MAX_BACKOFF: Duration = Duration::from_millis(400);

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
    #[error("the Jev slice token budget must be greater than zero")]
    ZeroSliceTokens,
}

/// Jev, served by the configured decisions endpoint, behind the
/// `ToolSelectionClassifier` port.
pub struct JevToolClassifier {
    model: String,
    api_key: JevApiKey,
    timeout: Duration,
    max_slice_tokens: usize,
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
            .field("max_slice_tokens", &self.max_slice_tokens)
            .finish_non_exhaustive()
    }
}

impl JevToolClassifier {
    /// A classifier for `model` that posts to `endpoint` through the host's
    /// policy egress, pinned to that endpoint's host, giving up on a whole
    /// classification (every slice, retries included) after `timeout`.
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
        let transport = ReqwestNetworkTransport::new(timeout);
        Ok(Self {
            model,
            api_key,
            timeout,
            max_slice_tokens: DEFAULT_MAX_SLICE_TOKENS,
            policy: endpoint.policy(),
            endpoint: endpoint.url().to_string(),
            egress: Arc::new(PolicyNetworkHttpEgress::new(transport)),
        })
    }

    /// Cap each request at `tokens` estimated tokens; a catalog that does
    /// not fit is split into more slices.
    pub fn with_max_slice_tokens(mut self, tokens: usize) -> Result<Self, JevConfigError> {
        if tokens == 0 {
            return Err(JevConfigError::ZeroSliceTokens);
        }
        self.max_slice_tokens = tokens;
        Ok(self)
    }

    /// The model every request names.
    pub fn model(&self) -> &str {
        &self.model
    }

    async fn ask_slice(
        &self,
        request: &ToolSelectionRequest,
        slice: &[usize],
        deadline: Instant,
    ) -> Result<SliceAnswer, ToolSelectionError> {
        let body = serde_json::to_vec(&slice_body(
            &self.model,
            &request.context,
            self.max_slice_tokens,
            &request.candidates,
            slice,
        ))
        .map_err(|_| ToolSelectionError::Unavailable {
            reason: LoopSafeSummary::capability_failure_summary(
                "the classification request could not be encoded",
            ),
        })?;
        let asked: Vec<&str> = slice
            .iter()
            .filter_map(|index| request.candidates.get(*index))
            .map(|candidate| candidate.name())
            .collect();
        let mut backoff = FIRST_BACKOFF;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ToolSelectionError::Timeout {
                    elapsed: self.timeout,
                });
            }
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
                    body: body.clone(),
                    policy: self.policy.clone(),
                    response_body_limit: Some(RESPONSE_BODY_LIMIT),
                    timeout_ms: Some(
                        u32::try_from(remaining.as_millis())
                            .unwrap_or(u32::MAX)
                            .max(1),
                    ),
                })
                .await
                .map_err(|error| ToolSelectionError::Unavailable {
                    reason: LoopSafeSummary::capability_failure_summary(format!(
                        "the classification service could not be reached ({})",
                        error.stable_reason()
                    )),
                })?;
            match response.status {
                200..=299 => {
                    return parse_answer(&response.body, &asked).map_err(|error| {
                        ToolSelectionError::InvalidOutput {
                            reason: LoopSafeSummary::capability_failure_summary(error.summary()),
                        }
                    });
                }
                401 | 403 => return Err(ToolSelectionError::Unauthorized),
                402 => return Err(ToolSelectionError::PaymentRequired),
                // Overload: `429` (rate limit), `503` (model at capacity),
                // `529` (overloaded). Retried until the deadline.
                429 | 503 | 529 => {
                    // A server-named wait past the deadline ends the
                    // attempt at once.
                    let wait = retry_after(&response.headers).unwrap_or(backoff);
                    if Instant::now() + wait >= deadline {
                        return Err(ToolSelectionError::RateLimited);
                    }
                    tokio::time::sleep(wait).await;
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
                status => return Err(ToolSelectionError::Rejected { status }),
            }
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

/// A `retry-after` given in whole seconds, when the response carries one.
fn retry_after(headers: &[(String, String)]) -> Option<Duration> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, value)| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

/// The top `selectable_tools()` non-pinned candidates by probability (ties
/// by catalog order), trimmed from the lowest probability up until they fit
/// `selectable_tokens()`. Returns the chosen candidate indices, best first,
/// and the probability of the best candidate left out.
fn top_n(request: &ToolSelectionRequest, probabilities: &[f32]) -> (Vec<usize>, Option<f32>) {
    let mut order: Vec<usize> = (0..request.candidates.len().min(probabilities.len()))
        .filter(|index| !request.is_pinned(request.candidates[*index].name()))
        .collect();
    // Stable: equal probabilities keep catalog order.
    order.sort_by(|left, right| probabilities[*right].total_cmp(&probabilities[*left]));
    let mut chosen: Vec<usize> = order
        .iter()
        .copied()
        .take(request.selectable_tools())
        .collect();
    let tokens = |chosen: &[usize]| {
        chosen.iter().fold(0_u32, |sum, index| {
            sum.saturating_add(request.candidates[*index].est_schema_tokens)
        })
    };
    while tokens(&chosen) > request.selectable_tokens() {
        chosen.pop();
    }
    let first_left_out = order.get(chosen.len()).map(|index| probabilities[*index]);
    (chosen, first_left_out)
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
        let deadline = started + self.timeout;
        let slices = plan_slices(&request.context, &request.candidates, self.max_slice_tokens);
        // Every slice concurrently; the first failure cancels the rest, since
        // a partial vector must never be ranked.
        let asked = try_join_all(
            slices
                .iter()
                .map(|slice| self.ask_slice(request, slice, deadline)),
        );
        let outcome = match tokio::time::timeout(self.timeout, asked).await {
            Ok(outcome) => outcome,
            Err(_) => Err(ToolSelectionError::Timeout {
                elapsed: self.timeout,
            }),
        };
        let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let answers = match outcome {
            Ok(answers) => answers,
            Err(error) => {
                debug!(
                    target: LOG_TARGET,
                    classifier = JEV_CLASSIFIER_NAME,
                    model = %self.model,
                    error_kind = error.kind_label(),
                    slices = slices.len(),
                    latency_ms,
                    "Jev tool classification failed"
                );
                return Err(error);
            }
        };

        let mut probabilities = vec![0.0_f32; request.candidates.len()];
        let mut input_tokens = 0_u64;
        // The models the server reported, in first-seen order; with an alias
        // such as `jev-latest` this is the only record of the release used.
        let mut served_models: Vec<String> = Vec::new();
        for (slice, answer) in slices.iter().zip(answers) {
            input_tokens = input_tokens.saturating_add(answer.input_tokens);
            if let Some(model) = answer.served_model
                && !served_models.contains(&model)
            {
                served_models.push(model);
            }
            for (index, probability) in slice.iter().zip(answer.probabilities) {
                if let Some(slot) = probabilities.get_mut(*index) {
                    *slot = probability;
                }
            }
        }
        let (chosen, first_left_out) = top_n(request, &probabilities);
        let chosen: Vec<ChosenTool> = chosen
            .into_iter()
            .map(|index| ChosenTool::new(request.candidates[index].name(), probabilities[index]))
            .collect();
        let logged: Vec<(&str, f32)> = chosen
            .iter()
            .map(|tool| (tool.name.as_str(), tool.score))
            .collect();
        debug!(
            target: LOG_TARGET,
            classifier = JEV_CLASSIFIER_NAME,
            model = %self.model,
            served_model = %served_models.join(","),
            chosen = ?logged,
            first_left_out = ?first_left_out,
            slices = slices.len(),
            latency_ms,
            input_tokens,
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
    use ironclaw_host_api::ids::CapabilityId;
    use ironclaw_loop_contracts::{
        ConversationContext, ProviderToolDefinition, ToolSelectionCandidate,
    };
    use serde_json::json;

    use super::*;

    fn request(
        tokens: &[(&str, u32)],
        pinned: &[&str],
        max: usize,
        budget: u32,
    ) -> ToolSelectionRequest {
        ToolSelectionRequest {
            context: ConversationContext::new(vec!["hello".to_string()]),
            called_tools: Vec::new(),
            candidates: tokens
                .iter()
                .map(|(name, est_schema_tokens)| ToolSelectionCandidate {
                    definition: ProviderToolDefinition::from_parts(
                        CapabilityId::new(format!("demo.{name}")).expect("id"),
                        *name,
                        "d",
                        json!({}),
                    )
                    .expect("definition"),
                    est_schema_tokens: *est_schema_tokens,
                })
                .collect(),
            pinned: pinned.iter().map(|name| name.to_string()).collect(),
            max_tools: max,
            token_budget: budget,
            reserved_tools: 0,
            reserved_tokens: 0,
        }
    }

    #[test]
    fn top_n_takes_the_highest_probabilities_with_catalog_order_ties() {
        let request = request(
            &[("a", 10), ("b", 10), ("c", 10), ("d", 10), ("e", 10)],
            &["e"],
            3,
            1_000,
        );
        // `e` is pinned, so its top probability does not use a slot; `b`
        // and `c` tie and keep catalog order; `d` is the first left out.
        let (chosen, first_left_out) = top_n(&request, &[0.2, 0.6, 0.6, 0.1, 0.99]);
        assert_eq!(chosen, vec![1, 2, 0]);
        assert_eq!(first_left_out, Some(0.1));
    }

    #[test]
    fn top_n_trims_the_token_budget_from_the_lowest_probability_up() {
        let request = request(&[("a", 50), ("b", 40), ("c", 30)], &[], 3, 90);
        let (chosen, first_left_out) = top_n(&request, &[0.9, 0.8, 0.7]);
        assert_eq!(chosen, vec![0, 1]);
        assert_eq!(first_left_out, Some(0.7));
    }

    #[test]
    fn keys_and_settings_are_validated_and_the_key_never_prints() {
        assert_eq!(
            JevApiKey::new("   ").map(|_| ()),
            Err(JevConfigError::EmptyApiKey)
        );
        let key = JevApiKey::new("jev-secret-value").expect("key");
        assert!(!format!("{key:?}").contains("secret"));
        assert_eq!(
            JevToolClassifier::new(
                JevEndpoint::default(),
                " ",
                JevApiKey::new("k").expect("key"),
                Duration::from_millis(5)
            )
            .map(|_| ()),
            Err(JevConfigError::EmptyModel)
        );
        assert_eq!(
            JevToolClassifier::new(JevEndpoint::default(), "jev-latest", key, Duration::ZERO)
                .map(|_| ()),
            Err(JevConfigError::ZeroTimeout)
        );
        let classifier = JevToolClassifier::new(
            JevEndpoint::default(),
            DEFAULT_JEV_MODEL,
            JevApiKey::new("jev-secret-value").expect("key"),
            Duration::from_millis(500),
        )
        .expect("classifier");
        assert!(!format!("{classifier:?}").contains("secret"));
        assert_eq!(
            classifier.with_max_slice_tokens(0).map(|_| ()),
            Err(JevConfigError::ZeroSliceTokens)
        );
    }

    /// Answers every request with one `noul` per question and records the URL
    /// and headers; nothing leaves the process.
    /// One request the transport saw: URL, headers and JSON body.
    type Seen = (String, Vec<(String, String)>, serde_json::Value);

    #[derive(Clone, Default)]
    struct RecordingTransport {
        seen: Arc<std::sync::Mutex<Vec<Seen>>>,
    }

    #[async_trait]
    impl ironclaw_network::NetworkHttpTransport for RecordingTransport {
        async fn execute(
            &self,
            request: ironclaw_network::NetworkTransportRequest,
        ) -> Result<ironclaw_network::NetworkHttpResponse, ironclaw_network::NetworkHttpError>
        {
            let body: serde_json::Value = serde_json::from_slice(&request.body).expect("json");
            let answers: serde_json::Map<String, serde_json::Value> = body["questions"]
                .as_object()
                .expect("questions")
                .keys()
                .map(|id| (id.clone(), json!({"type": "noul", "noul": 0.5})))
                .collect();
            self.seen.lock().expect("seen").push((
                request.url.clone(),
                request.headers.clone(),
                body,
            ));
            Ok(ironclaw_network::NetworkHttpResponse {
                status: 200,
                headers: Vec::new(),
                body: json!({"model": "jev-latest", "answers": answers})
                    .to_string()
                    .into_bytes(),
                usage: Default::default(),
            })
        }
    }

    /// Resolves every host to one public address without DNS.
    #[derive(Clone)]
    struct FixedResolver;

    impl ironclaw_network::NetworkResolver for FixedResolver {
        fn resolve_ips(
            &self,
            _host: &str,
            _port: u16,
        ) -> Result<Vec<std::net::IpAddr>, ironclaw_network::NetworkHttpError> {
            Ok(vec![std::net::IpAddr::from([8, 8, 8, 8])])
        }
    }

    /// A classifier configured with `endpoint` (the default when `None`)
    /// whose egress records instead of sending.
    fn recorded_classifier(endpoint: Option<&str>) -> (JevToolClassifier, RecordingTransport) {
        let transport = RecordingTransport::default();
        let endpoint = endpoint
            .map(|raw| JevEndpoint::parse(raw).expect("endpoint"))
            .unwrap_or_default();
        let mut classifier = JevToolClassifier::new(
            endpoint,
            DEFAULT_JEV_MODEL,
            JevApiKey::new("typesafe-key").expect("key"),
            Duration::from_secs(5),
        )
        .expect("classifier");
        classifier.egress = Arc::new(PolicyNetworkHttpEgress::new_with_resolver(
            transport.clone(),
            FixedResolver,
        ));
        (classifier, transport)
    }

    async fn classify_one(
        classifier: &JevToolClassifier,
    ) -> Result<ToolSelection, ToolSelectionError> {
        classifier
            .classify(&request(&[("a", 10), ("b", 10)], &[], 5, 1_000))
            .await
    }

    #[tokio::test]
    async fn the_default_classifier_posts_to_typesafe_with_jev_latest_and_its_key() {
        assert_eq!(
            crate::DEFAULT_JEV_ENDPOINT,
            "https://api.typesafe.ai/v1/systemone"
        );
        assert_eq!(DEFAULT_JEV_MODEL, "jev-latest");
        let (classifier, transport) = recorded_classifier(None);
        let selection = classify_one(&classifier)
            .await
            .expect("the pinned policy allows the default host");
        assert_eq!(selection.scorer, "jev:jev-latest");
        let seen = transport.seen.lock().expect("seen").clone();
        assert_eq!(seen.len(), 1);
        let (url, headers, body) = &seen[0];
        assert_eq!(url, "https://api.typesafe.ai/v1/systemone");
        assert!(
            headers
                .iter()
                .any(|(name, value)| name == "authorization" && value == "Bearer typesafe-key")
        );
        assert_eq!(body["model"], "jev-latest");
        assert_eq!(body["questions"]["a"]["type"], "noul");
        assert_eq!(body["questions"]["b"]["type"], "noul");
    }

    #[tokio::test]
    async fn a_configured_endpoint_is_posted_to_and_its_pin_refuses_every_other_target() {
        let configured = "https://jev.example.test:8443/api/v1/decisions";
        let (classifier, transport) = recorded_classifier(Some(configured));
        classify_one(&classifier)
            .await
            .expect("the pin allows the configured host");
        let seen = transport.seen.lock().expect("seen").clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, configured);

        // The same pin, pointed elsewhere: another host (the default one
        // included), another port, or plain HTTP to the right host.
        for elsewhere in [
            "https://api.typesafe.ai/v1/systemone",
            "https://other.example.test:8443/api/v1/decisions",
            "https://jev.example.test/api/v1/decisions",
            "http://jev.example.test:8443/api/v1/decisions",
        ] {
            let (mut classifier, transport) = recorded_classifier(Some(configured));
            classifier.endpoint = elsewhere.to_string();
            let error = classify_one(&classifier).await.expect_err(elsewhere);
            assert!(
                matches!(error, ToolSelectionError::Unavailable { .. }),
                "{elsewhere}: {error:?}"
            );
            assert!(
                transport.seen.lock().expect("seen").is_empty(),
                "{elsewhere}"
            );
        }
    }

    #[test]
    fn retry_after_reads_whole_seconds_only() {
        let headers = |value: &str| vec![("Retry-After".to_string(), value.to_string())];
        assert_eq!(retry_after(&headers("2")), Some(Duration::from_secs(2)));
        assert_eq!(retry_after(&headers("soon")), None);
        assert_eq!(retry_after(&[]), None);
    }
}

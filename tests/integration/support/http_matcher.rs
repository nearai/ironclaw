//! `ScriptedHttpResponse` — the canonical URL/method/capability-keyed scripting
//! layer over `RecordingRuntimeHttpEgress`, letting a multi-step tool-HTTP flow
//! script a different body per request. First-match-wins scripted list, falling
//! back to the FIFO queue then the default body. A match can also script a
//! non-2xx status (`.with_status`, still Completed) or a runtime egress error
//! (`egress_error`, mapping to Failed/Denied — see [`ScriptedHttpOutcome`]).
//!
//! `Recording*` structs are deliberately concrete, not generic, ahead of need.

// Shared integration-test support: not every binary that mounts the
// `ironclaw_support` tree consumes this module, so its symbols read as dead there
// under `-D warnings`. Module-level allow matches the sibling support modules.
#![allow(dead_code)]

use ironclaw_host_api::{RuntimeHttpEgressError, RuntimeHttpEgressRequest};

/// What a matched [`ScriptedHttpResponse`] yields: a successful response
/// (status + body), or a scripted egress error letting tests exercise runtime
/// error paths (`policy_denied`, `response_body_limit_exceeded`) that the real
/// `HostHttpEgressService` produces — the egress is the seam this tier fakes.
#[derive(Debug, Clone)]
pub enum ScriptedHttpOutcome {
    /// A successful egress response with the given HTTP status and body.
    Body { status: u16, bytes: Vec<u8> },
    /// A scripted egress error (`Err(RuntimeHttpEgressError)` from `execute`).
    Error(RuntimeHttpEgressError),
}

/// One scripted HTTP response keyed by request attributes; first-match-wins in
/// scripted order. URL-substring-only matches any method/capability; narrow
/// with [`with_method`]/[`with_capability`].
///
/// [`with_method`]: ScriptedHttpResponse::with_method
/// [`with_capability`]: ScriptedHttpResponse::with_capability
#[derive(Debug, Clone)]
pub struct ScriptedHttpResponse {
    /// Required: the request URL must contain this substring.
    url_contains: String,
    /// Optional: lowercase HTTP method (`NetworkMethod` Display form, e.g.
    /// `"get"`/`"post"`). Compared case-insensitively.
    method: Option<String>,
    /// Optional: the request's capability id must equal this exactly.
    capability_id: Option<String>,
    /// Scripted outcome returned when this response matches.
    outcome: ScriptedHttpOutcome,
}

impl ScriptedHttpResponse {
    /// Script a `200` response with `body` for any request whose URL contains
    /// `url_substr`. Use [`with_status`](Self::with_status) for a non-2xx status
    /// or [`egress_error`](Self::egress_error) for a runtime egress error.
    pub fn for_url(url_substr: impl Into<String>, body: impl Into<Vec<u8>>) -> Self {
        Self {
            url_contains: url_substr.into(),
            method: None,
            capability_id: None,
            outcome: ScriptedHttpOutcome::Body {
                status: 200,
                bytes: body.into(),
            },
        }
    }

    /// Script a runtime egress error (`Err(error)` from `execute`) for requests
    /// whose URL contains `url_substr` — drives `builtin.http`'s error mapping
    /// (e.g. `policy_denied` → `Denied`, `response_body_limit_exceeded` → `Failed{OutputTooLarge}`).
    pub fn egress_error(url_substr: impl Into<String>, error: RuntimeHttpEgressError) -> Self {
        Self {
            url_contains: url_substr.into(),
            method: None,
            capability_id: None,
            outcome: ScriptedHttpOutcome::Error(error),
        }
    }

    /// `RuntimeHttpEgressError::Network` with `reason` (e.g. `"policy_denied"` →
    /// `Denied`). Named wrapper over [`egress_error`](Self::egress_error).
    pub fn network_error(url_substr: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::egress_error(
            url_substr,
            RuntimeHttpEgressError::Network {
                reason: reason.into(),
                request_bytes: 0,
                response_bytes: 0,
            },
        )
    }

    /// `RuntimeHttpEgressError::Response` with `reason` (e.g.
    /// `RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED` → `Failed{OutputTooLarge}`).
    /// Named wrapper over [`egress_error`](Self::egress_error).
    pub fn response_error(url_substr: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::egress_error(
            url_substr,
            RuntimeHttpEgressError::Response {
                reason: reason.into(),
                request_bytes: 0,
                response_bytes: 0,
            },
        )
    }

    /// Override the HTTP status of a body response (default `200`). Panics on
    /// an [`egress_error`](ScriptedHttpResponse::egress_error) response — mutually
    /// exclusive outcomes; silently no-op'ing would exercise the wrong path.
    pub fn with_status(mut self, status: u16) -> Self {
        match &mut self.outcome {
            ScriptedHttpOutcome::Body { status: s, .. } => *s = status,
            ScriptedHttpOutcome::Error(_) => {
                panic!(
                    "with_status() has no effect on an egress_error() response; these are mutually exclusive outcomes"
                )
            }
        }
        self
    }

    /// Narrow the key to a specific HTTP method (lowercase, e.g. `"post"`).
    pub fn with_method(mut self, method: impl Into<String>) -> Self {
        self.method = Some(method.into());
        self
    }

    /// Narrow the key to a specific capability id (e.g. `"builtin.http"`).
    pub fn with_capability(mut self, capability_id: impl Into<String>) -> Self {
        self.capability_id = Some(capability_id.into());
        self
    }

    /// True when every present key component matches `request`.
    pub(crate) fn matches(&self, request: &RuntimeHttpEgressRequest) -> bool {
        if !request.url.contains(&self.url_contains) {
            return false;
        }
        if let Some(method) = &self.method
            && !request.method.to_string().eq_ignore_ascii_case(method)
        {
            return false;
        }
        if let Some(capability_id) = &self.capability_id
            && request.capability_id.as_str() != capability_id
        {
            return false;
        }
        true
    }

    /// The scripted outcome to return on a match.
    pub(crate) fn outcome(&self) -> ScriptedHttpOutcome {
        self.outcome.clone()
    }
}

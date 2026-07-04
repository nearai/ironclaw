//! `ScriptedHttpResponse` — the URL/method/capability-keyed scripting layer over
//! `RecordingRuntimeHttpEgress` (design §3.6 "P1 ergonomics", §3.7 Tier-2).
//!
//! Slice 2 shipped the recording egress with a single FIFO body queue. A
//! multi-step tool-HTTP flow (two `builtin.http` calls to different URLs in one
//! turn) needs a DIFFERENT scripted body per request. This module is the
//! canonical keyed-matcher API: a test scripts a list of
//! [`ScriptedHttpResponse`]s; on each `RuntimeHttpEgress::execute` the recording
//! egress returns the body of the FIRST scripted response whose key matches the
//! request, falling back to the FIFO queue, then the default body.
//!
//! A matched response is not always a `200` body: `.with_status(u16)` scripts a
//! non-2xx status (still a successful egress call — `builtin.http` surfaces it
//! as a Completed tool result carrying that status), and
//! `ScriptedHttpResponse::egress_error(url, RuntimeHttpEgressError)` scripts a
//! runtime egress failure (`Err` from `execute`, mapping to a
//! `Failed`/`Denied` capability outcome). See [`ScriptedHttpOutcome`].
//!
//! Concrete by design (spec §3.7): the `Recording*` structs are deliberately not
//! a premature generic `Recording<P>` — this is written in the extractable
//! scripted-responses + captured-calls shape so a future rule-of-three lift is
//! mechanical, but no generic is introduced ahead of need.

// Shared integration-test support: not every binary that mounts the
// `reborn_support` tree consumes this module, so its symbols read as dead there
// under `-D warnings`. Module-level allow matches the sibling support modules.
#![allow(dead_code)]

use ironclaw_host_api::{RuntimeHttpEgressError, RuntimeHttpEgressRequest};

/// What a matched [`ScriptedHttpResponse`] yields at the runtime HTTP egress
/// boundary: either a successful response (status + body) or a scripted egress
/// error. The error arm lets tests exercise the runtime error paths
/// (`policy_denied`, `response_body_limit_exceeded`, …) that the real
/// `HostHttpEgressService` produces but the recording egress otherwise cannot —
/// the egress *is* the vendor/network seam this tier fakes, so scripting an
/// error here is the same seam as scripting a body.
#[derive(Debug, Clone)]
pub enum ScriptedHttpOutcome {
    /// A successful egress response with the given HTTP status and body.
    Body { status: u16, bytes: Vec<u8> },
    /// A scripted egress error (`Err(RuntimeHttpEgressError)` from `execute`).
    Error(RuntimeHttpEgressError),
}

/// One scripted HTTP response keyed by request attributes. Matching is
/// first-match-wins in scripted order. A response with only a URL substring
/// matches any method/capability hitting that URL; adding [`with_method`] or
/// [`with_capability`] narrows the key.
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
    /// `url_substr`. Use [`with_status`] to script a non-2xx status (which the
    /// `builtin.http` tool surfaces as a *successful* tool result carrying the
    /// status), or [`egress_error`] for a runtime egress error.
    ///
    /// [`with_status`]: ScriptedHttpResponse::with_status
    /// [`egress_error`]: ScriptedHttpResponse::egress_error
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

    /// Script a runtime egress error for any request whose URL contains
    /// `url_substr`. The `execute` boundary returns `Err(error)`, driving the
    /// `builtin.http` tool's error mapping (e.g. `policy_denied` → `Denied`,
    /// `response_body_limit_exceeded` → `Failed{OutputTooLarge}`).
    pub fn egress_error(url_substr: impl Into<String>, error: RuntimeHttpEgressError) -> Self {
        Self {
            url_contains: url_substr.into(),
            method: None,
            capability_id: None,
            outcome: ScriptedHttpOutcome::Error(error),
        }
    }

    /// Script a network-layer egress failure (`RuntimeHttpEgressError::Network`)
    /// with the given `reason` — e.g. `"policy_denied"`, which the tool's error
    /// mapping surfaces as a `Denied` capability outcome. Thin named wrapper over
    /// [`egress_error`](Self::egress_error) so test bodies select the scenario by
    /// name instead of hand-building the nested error struct.
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

    /// Script a response-layer egress failure (`RuntimeHttpEgressError::Response`)
    /// with the given `reason` — e.g. `RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED`,
    /// which the tool's error mapping surfaces as `Failed{OutputTooLarge}`. Thin
    /// named wrapper over [`egress_error`](Self::egress_error) so test bodies
    /// select the scenario by name instead of hand-building the nested error struct.
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
    /// an [`egress_error`](ScriptedHttpResponse::egress_error) response — the
    /// two are mutually exclusive scripted outcomes, and silently no-op'ing
    /// would leave the test exercising the egress-error path instead of the
    /// status the author intended.
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

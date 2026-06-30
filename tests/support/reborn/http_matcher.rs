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
//! Concrete by design (spec §3.7): the `Recording*` structs are deliberately not
//! a premature generic `Recording<P>` — this is written in the extractable
//! scripted-responses + captured-calls shape so a future rule-of-three lift is
//! mechanical, but no generic is introduced ahead of need.

// Shared integration-test support: not every binary that mounts the
// `reborn_support` tree consumes this module, so its symbols read as dead there
// under `-D warnings`. Module-level allow matches the sibling support modules.
#![allow(dead_code)]

use ironclaw_host_api::RuntimeHttpEgressRequest;

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
    /// Scripted response body returned when this response matches.
    body: Vec<u8>,
}

impl ScriptedHttpResponse {
    /// Script `body` for any request whose URL contains `url_substr`.
    pub fn for_url(url_substr: impl Into<String>, body: impl Into<Vec<u8>>) -> Self {
        Self {
            url_contains: url_substr.into(),
            method: None,
            capability_id: None,
            body: body.into(),
        }
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

    /// The scripted body to return on a match.
    pub(crate) fn body_bytes(&self) -> Vec<u8> {
        self.body.clone()
    }
}

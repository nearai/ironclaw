//! Shared descriptor → route matching helpers used by both the
//! per-route rate-limit and body-limit middlewares.
//!
//! Both middlewares walk the same `IngressRouteDescriptor` set and need
//! to answer the same question ("does this request match a known
//! route?") with the same semantics. Centralising the matcher avoids a
//! silent divergence where one enforcer treats a request as in-scope
//! and the other doesn't.

use axum::http::Method;
use ironclaw_host_api::NetworkMethod;

/// Map the host-api network method enum onto axum's `http::Method`.
pub(crate) fn network_method_to_axum(method: NetworkMethod) -> Method {
    match method {
        NetworkMethod::Get => Method::GET,
        NetworkMethod::Post => Method::POST,
        NetworkMethod::Put => Method::PUT,
        NetworkMethod::Patch => Method::PATCH,
        NetworkMethod::Delete => Method::DELETE,
        NetworkMethod::Head => Method::HEAD,
    }
}

/// Split a descriptor pattern (e.g. `/api/webchat/v2/threads/{id}/events`)
/// into segments. `None` marks a `{name}` wildcard so the matcher
/// accepts any non-empty value in that position.
pub(crate) fn parse_pattern(pattern: &str) -> Vec<Option<String>> {
    pattern
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            if segment.starts_with('{') && segment.ends_with('}') {
                None
            } else {
                Some(segment.to_string())
            }
        })
        .collect()
}

/// Match a parsed pattern against a concrete request path. Empty
/// wildcard segments (`/api/.../threads//events`) are rejected to
/// preserve the v2 descriptors' contract that path parameters are
/// always non-empty.
pub(crate) fn segments_match(pattern: &[Option<String>], path: &str) -> bool {
    let mut iter = path.split('/').filter(|segment| !segment.is_empty());
    for expected in pattern {
        match iter.next() {
            None => return false,
            Some(actual) => match expected {
                Some(literal) if literal == actual => {}
                Some(_) => return false,
                None => {
                    if actual.is_empty() {
                        return false;
                    }
                }
            },
        }
    }
    iter.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_parser_handles_literal_and_wildcard_segments() {
        assert_eq!(
            parse_pattern("/api/webchat/v2/threads/{thread_id}/events"),
            vec![
                Some("api".into()),
                Some("webchat".into()),
                Some("v2".into()),
                Some("threads".into()),
                None,
                Some("events".into()),
            ],
        );
    }

    #[test]
    fn segments_match_matches_literal_and_wildcard() {
        let pattern = parse_pattern("/api/webchat/v2/threads/{id}/events");
        assert!(segments_match(
            &pattern,
            "/api/webchat/v2/threads/abc/events"
        ));
        assert!(!segments_match(
            &pattern,
            "/api/webchat/v2/threads/abc/timeline"
        ));
        assert!(!segments_match(
            &pattern,
            "/api/webchat/v2/threads/abc/events/extra"
        ));
        // Empty wildcard segment.
        assert!(!segments_match(&pattern, "/api/webchat/v2/threads//events"));
    }
}

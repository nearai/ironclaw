//! Validation for the deployment-configured public web origin used to build
//! extension connect/setup links (`/extensions`, `/chat?connect=…`) shown to
//! chat users.
//!
//! The origin's source is `IRONCLAW_REBORN_WEBUI_BASE_URL`
//! (`ironclaw_composition::extension_host_assembly::connect_link_base_url_from_env`),
//! read independently by three call sites — the deployment-channel notice
//! (`ironclaw_extension_host::channel_host::configured_origin`), the
//! device-link-unavailable chat prompt
//! (`ironclaw_assistant::run_delivery::prompts::extensions_page_link`), and
//! the personal-account setup nudge
//! (`ironclaw_extension_manager::install_guidance::personal_setup_link`) —
//! that used to trim-and-check-non-empty only, never confirming the value was
//! an absolute origin at all. A deployment misconfigured as
//! `IRONCLAW_REBORN_WEBUI_BASE_URL=app.example.com` (no scheme) rendered the
//! relative `app.example.com/extensions` into a customer conversation, and a
//! value carrying a query or fragment could redirect the link away from the
//! Extensions page. This module is the one place that decides an origin is
//! safe to render.

/// Validate `base_url` as an absolute `http`/`https` origin with no query
/// string or fragment, and return it with any trailing slash trimmed.
///
/// Returns `None` for anything that is not a safely renderable origin: no
/// value, blank/whitespace, a scheme-less (relative) value, a non-http(s)
/// scheme, or a value carrying a `?query` or `#fragment` (either would change
/// where the rendered link actually goes). `None` here means "ship the
/// link-free copy" at every call site — never a startup failure. That
/// deliberately differs from the OAuth callback consumer of the same
/// environment variable, which fails startup on a blank value; see
/// `connect_link_base_url_from_env`'s own doc comment for why the two
/// consumers of one variable are allowed to disagree on how unusable costs.
///
/// `https://x.test/` and `https://x.test` are equivalent — the trailing
/// slash is trimmed, not treated as part of the origin's identity.
pub fn validated_connect_link_origin(base_url: Option<&str>) -> Option<&str> {
    let trimmed = base_url?.trim();
    if trimmed.is_empty() {
        return None; // silent-ok: blank/whitespace origin means "unset"; every caller ships link-free
    }

    let scheme_end = trimmed.find("://")?; // silent-ok: no scheme means a relative value, never safe to render as a link
    // Schemes are case-insensitive (RFC 3986 §3.1), and `.claude/rules/types.md`
    // requires normalizing case-insensitive external values at the boundary —
    // an operator writing `HTTPS://` means the same origin.
    let scheme = &trimmed[..scheme_end];
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return None; // silent-ok: only http(s) origins are safe to render as a clickable link
    }

    let authority = &trimmed[scheme_end + 3..];
    if authority.contains(['?', '#']) {
        return None; // silent-ok: a query string or fragment can redirect the link away from its intended page
    }

    let authority = authority.trim_end_matches('/');
    if authority.is_empty() {
        return None; // silent-ok: a scheme with no host is not a usable origin
    }

    Some(&trimmed[..scheme_end + 3 + authority.len()])
}

#[cfg(test)]
mod tests {
    use super::validated_connect_link_origin;

    #[test]
    fn accepts_absolute_https_origin() {
        assert_eq!(
            validated_connect_link_origin(Some("https://app.example.com")),
            Some("https://app.example.com")
        );
    }

    #[test]
    fn accepts_an_uppercase_scheme() {
        // Fail-safe rejection would have shipped link-free for a perfectly
        // valid origin.
        assert_eq!(
            validated_connect_link_origin(Some("HTTPS://app.example.com")),
            Some("HTTPS://app.example.com")
        );
    }

    #[test]
    fn accepts_absolute_http_origin() {
        assert_eq!(
            validated_connect_link_origin(Some("http://app.example.com")),
            Some("http://app.example.com")
        );
    }

    #[test]
    fn trims_trailing_slash() {
        assert_eq!(
            validated_connect_link_origin(Some("https://app.example.com/")),
            validated_connect_link_origin(Some("https://app.example.com"))
        );
        assert_eq!(
            validated_connect_link_origin(Some("https://app.example.com/")),
            Some("https://app.example.com")
        );
    }

    #[test]
    fn rejects_scheme_less_value() {
        // The exact CodeRabbit-reported case: no scheme renders the RELATIVE
        // `app.example.com/extensions` into a customer conversation.
        assert_eq!(validated_connect_link_origin(Some("app.example.com")), None);
    }

    #[test]
    fn rejects_non_http_scheme() {
        assert_eq!(
            validated_connect_link_origin(Some("ftp://app.example.com")),
            None
        );
        assert_eq!(
            validated_connect_link_origin(Some("javascript://app.example.com")),
            None
        );
    }

    #[test]
    fn rejects_query_string() {
        assert_eq!(
            validated_connect_link_origin(Some("https://x.test/?a=1")),
            None
        );
    }

    #[test]
    fn rejects_fragment() {
        assert_eq!(
            validated_connect_link_origin(Some("https://x.test#f")),
            None
        );
    }

    #[test]
    fn rejects_query_and_fragment_together() {
        assert_eq!(
            validated_connect_link_origin(Some("https://x.test/?a=1#f")),
            None
        );
    }

    #[test]
    fn rejects_blank_whitespace_and_none() {
        assert_eq!(validated_connect_link_origin(None), None);
        assert_eq!(validated_connect_link_origin(Some("")), None);
        assert_eq!(validated_connect_link_origin(Some("   ")), None);
        assert_eq!(validated_connect_link_origin(Some("/")), None);
    }

    #[test]
    fn rejects_scheme_with_no_host() {
        assert_eq!(validated_connect_link_origin(Some("https://")), None);
        assert_eq!(validated_connect_link_origin(Some("https:///")), None);
    }
}

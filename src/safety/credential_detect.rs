//! Broad detection of manually-provided credentials in HTTP request parameters.
//!
//! Used by the built-in HTTP tool to decide whether approval is needed when
//! the LLM provides auth data directly in headers or URL query parameters.

/// Check whether HTTP request parameters contain manually-provided credentials.
///
/// Inspects headers (name/value), URL query parameters, and URL userinfo
/// for patterns that indicate authentication data.
pub fn params_contain_manual_credentials(params: &serde_json::Value) -> bool {
    headers_contain_credentials(params)
        || url_contains_credential_params(params)
        || url_contains_userinfo(params)
}

/// Header names that are exact matches for credential-carrying headers (case-insensitive).
const AUTH_HEADER_EXACT: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "x-api-key",
    "api-key",
    "x-auth-token",
    "x-token",
    "x-access-token",
    "x-session-token",
    "x-csrf-token",
    "x-secret",
    "x-api-secret",
];

/// Substrings in header names that suggest credentials (case-insensitive).
/// Note: "key" is excluded to avoid false positives like "X-Idempotency-Key".
const AUTH_HEADER_SUBSTRINGS: &[&str] = &["auth", "token", "secret", "credential", "password"];

/// Value prefixes that indicate auth schemes (case-insensitive).
const AUTH_VALUE_PREFIXES: &[&str] = &[
    "bearer ",
    "basic ",
    "token ",
    "digest ",
    "hoba ",
    "mutual ",
    "aws4-hmac-sha256 ",
];

/// URL query parameter names that are exact matches for credentials (case-insensitive).
const AUTH_QUERY_EXACT: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "access_token",
    "token",
    "key",
    "secret",
    "password",
    "auth",
    "auth_token",
    "session_token",
    "client_secret",
    "client_id",
    "app_key",
    "app_secret",
    "sig",
    "signature",
];

/// Substrings in query parameter names that suggest credentials (case-insensitive).
const AUTH_QUERY_SUBSTRINGS: &[&str] = &["token", "secret", "auth", "password", "credential"];

fn header_name_is_credential(name: &str) -> bool {
    let lower = name.to_lowercase();

    if AUTH_HEADER_EXACT.contains(&lower.as_str()) {
        return true;
    }

    AUTH_HEADER_SUBSTRINGS.iter().any(|sub| lower.contains(sub))
}

fn header_value_is_credential(value: &str) -> bool {
    let lower = value.to_lowercase();
    AUTH_VALUE_PREFIXES.iter().any(|pfx| lower.starts_with(pfx))
}

fn headers_contain_credentials(params: &serde_json::Value) -> bool {
    match params.get("headers") {
        Some(serde_json::Value::Object(map)) => map.iter().any(|(k, v)| {
            header_name_is_credential(k) || v.as_str().is_some_and(header_value_is_credential)
        }),
        Some(serde_json::Value::Array(items)) => items.iter().any(|item| {
            let name_match = item
                .get("name")
                .and_then(|n| n.as_str())
                .is_some_and(header_name_is_credential);
            let value_match = item
                .get("value")
                .and_then(|v| v.as_str())
                .is_some_and(header_value_is_credential);
            name_match || value_match
        }),
        _ => false,
    }
}

fn query_param_is_credential(name: &str) -> bool {
    let lower = name.to_lowercase();

    if AUTH_QUERY_EXACT.contains(&lower.as_str()) {
        return true;
    }

    AUTH_QUERY_SUBSTRINGS.iter().any(|sub| lower.contains(sub))
}

fn url_contains_credential_params(params: &serde_json::Value) -> bool {
    let url_str = match params.get("url").and_then(|u| u.as_str()) {
        Some(u) => u,
        None => return false,
    };

    let parsed = match url::Url::parse(url_str) {
        Ok(u) => u,
        Err(_) => return false,
    };

    parsed
        .query_pairs()
        .any(|(name, _)| query_param_is_credential(&name))
}

/// Detect credentials embedded in URL userinfo (e.g., `https://user:pass@host/`).
fn url_contains_userinfo(params: &serde_json::Value) -> bool {
    let url_str = match params.get("url").and_then(|u| u.as_str()) {
        Some(u) => u,
        None => return false,
    };

    let parsed = match url::Url::parse(url_str) {
        Ok(u) => u,
        Err(_) => return false,
    };

    // Non-empty username or password in the URL indicates embedded credentials
    !parsed.username().is_empty() || parsed.password().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Header name exact match ────────────────────────────────────────

    #[test]
    fn test_authorization_header_detected() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com",
            "headers": {"Authorization": "Bearer token123"}
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_all_exact_header_names() {
        for name in AUTH_HEADER_EXACT {
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://example.com",
                "headers": {name.to_string(): "some_value"}
            });
            assert!(
                params_contain_manual_credentials(&params),
                "Header '{}' should be detected",
                name
            );
        }
    }

    #[test]
    fn test_header_name_case_insensitive() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"AUTHORIZATION": "value"}
        });
        assert!(params_contain_manual_credentials(&params));
    }

    // ── Header name substring match ────────────────────────────────────

    #[test]
    fn test_header_substring_auth() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"X-Custom-Auth-Header": "value"}
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_header_substring_token() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"X-My-Token": "value"}
        });
        assert!(params_contain_manual_credentials(&params));
    }

    // ── Header value prefix match ──────────────────────────────────────

    #[test]
    fn test_bearer_value_detected() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"X-Custom": "Bearer sk-abc123"}
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_basic_value_detected() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"X-Custom": "Basic dXNlcjpwYXNz"}
        });
        assert!(params_contain_manual_credentials(&params));
    }

    // ── Array-format headers ───────────────────────────────────────────

    #[test]
    fn test_array_format_header_name() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": [{"name": "Authorization", "value": "Bearer token"}]
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_array_format_header_value_prefix() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": [{"name": "X-Custom", "value": "Token abc123"}]
        });
        assert!(params_contain_manual_credentials(&params));
    }

    // ── URL query parameter detection ──────────────────────────────────

    #[test]
    fn test_url_api_key_param() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data?api_key=abc123"
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_url_access_token_param() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data?access_token=xyz"
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_url_query_substring_match() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data?my_auth_code=xyz"
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_url_query_case_insensitive() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/data?API_KEY=abc"
        });
        assert!(params_contain_manual_credentials(&params));
    }

    // ── False positive checks ──────────────────────────────────────────

    #[test]
    fn test_idempotency_key_not_detected() {
        let params = serde_json::json!({
            "method": "POST",
            "url": "https://api.example.com",
            "headers": {"X-Idempotency-Key": "uuid-1234"}
        });
        assert!(!params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_content_type_not_detected() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {"Content-Type": "application/json", "Accept": "text/html"}
        });
        assert!(!params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_no_headers_no_query() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com/path"
        });
        assert!(!params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_safe_query_params() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://api.example.com/search?q=hello&page=1&limit=10"
        });
        assert!(!params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_empty_headers() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://example.com",
            "headers": {}
        });
        assert!(!params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_invalid_url_returns_false() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "not a url"
        });
        assert!(!params_contain_manual_credentials(&params));
    }

    // ── URL userinfo detection ─────────────────────────────────────────

    #[test]
    fn test_url_userinfo_with_password_detected() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://user:pass@api.example.com/data"
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_url_userinfo_username_only_detected() {
        let params = serde_json::json!({
            "method": "GET",
            "url": "https://apikey@api.example.com/data"
        });
        assert!(params_contain_manual_credentials(&params));
    }

    #[test]
    fn test_url_without_userinfo_not_detected_by_userinfo_check() {
        // This specifically tests that url_contains_userinfo returns false
        // for a normal URL (the broader function may still detect query params).
        assert!(!url_contains_userinfo(&serde_json::json!({
            "url": "https://api.example.com/data"
        })));
    }

    /// Adversarial tests for credential detection with Unicode, control chars,
    /// and case folding edge cases.
    /// See <https://github.com/nearai/ironclaw/issues/1025>.
    mod adversarial {
        use super::*;

        // ── B. Unicode edge cases ────────────────────────────────────

        #[test]
        fn header_name_with_zwsp_not_detected() {
            // ZWSP in header name: "Author\u{200B}ization" is NOT "Authorization"
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://example.com",
                "headers": {"Author\u{200B}ization": "Bearer token123"}
            });
            // The header NAME won't match exact "authorization" due to ZWSP.
            // But the VALUE still starts with "Bearer " — so value check catches it.
            assert!(
                params_contain_manual_credentials(&params),
                "Bearer prefix in value should still be detected even with ZWSP in header name"
            );
        }

        #[test]
        fn bearer_prefix_with_zwsp_bypass() {
            // ZWSP inside "Bearer": "Bear\u{200B}er token123"
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://example.com",
                "headers": {"X-Custom": "Bear\u{200B}er token123"}
            });
            // ZWSP breaks the "bearer " prefix match. Header name "X-Custom"
            // doesn't match exact/substring either. Documents bypass vector.
            let result = params_contain_manual_credentials(&params);
            // This should NOT be detected — documenting the limitation
            assert!(
                !result,
                "ZWSP in 'Bearer' prefix breaks detection — known limitation"
            );
        }

        #[test]
        fn rtl_override_in_url_query_param() {
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://api.example.com/data?\u{202E}api_key=secret"
            });
            // RTL override before "api_key" in query. url::Url::parse
            // percent-encodes the RTL char, making the query pair name
            // "%E2%80%AEapi_key" which does NOT match "api_key" exactly.
            // The substring check for "auth"/"token" also misses.
            // Document: RTL override can bypass query param detection.
            let result = params_contain_manual_credentials(&params);
            assert!(
                !result,
                "RTL override before query param name breaks detection — known limitation"
            );
        }

        #[test]
        fn zwnj_in_header_name() {
            // ZWNJ (\u{200C}) inserted into "Authorization"
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://example.com",
                "headers": {"Author\u{200C}ization": "some_value"}
            });
            // ZWNJ breaks the exact match for "authorization".
            // Substring check for "auth" still matches "author\u{200C}ization"
            // because to_lowercase preserves ZWNJ and "auth" appears before it.
            assert!(
                params_contain_manual_credentials(&params),
                "ZWNJ in header name — substring 'auth' check should still catch it"
            );
        }

        #[test]
        fn emoji_in_url_path_does_not_panic() {
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://api.example.com/🔑?api_key=secret"
            });
            // url::Url::parse handles emoji in paths. Credential param should still detect.
            assert!(params_contain_manual_credentials(&params));
        }

        #[test]
        fn unicode_case_folding_turkish_i() {
            // Turkish İ (U+0130) lowercases to "i̇" (i + combining dot above)
            // in Unicode, but to_lowercase() in Rust follows Unicode rules.
            // "Authorization" with Turkish İ: "Authorİzation"
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://example.com",
                "headers": {"Author\u{0130}zation": "value"}
            });
            // to_lowercase() of İ is "i̇" (2 chars), so "authorİzation" becomes
            // "authori̇zation" — does NOT match "authorization".
            // The substring check for "auth" WILL match though.
            assert!(
                params_contain_manual_credentials(&params),
                "Turkish İ — substring 'auth' check should still catch it"
            );
        }

        #[test]
        fn multibyte_userinfo_in_url() {
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://用户:密码@api.example.com/data"
            });
            // Non-ASCII username/password in URL userinfo
            assert!(
                params_contain_manual_credentials(&params),
                "multibyte userinfo should be detected"
            );
        }

        // ── C. Control character variants ────────────────────────────

        #[test]
        fn control_chars_in_header_name_still_detects() {
            for byte in [0x01u8, 0x02, 0x0B, 0x1F] {
                let name = format!("Authorization{}", char::from(byte));
                let params = serde_json::json!({
                    "method": "GET",
                    "url": "https://example.com",
                    "headers": {name: "Bearer token"}
                });
                // Header name contains "auth" substring, and value starts with
                // "Bearer " — both checks should still work with trailing control char.
                assert!(
                    params_contain_manual_credentials(&params),
                    "control char 0x{:02X} appended to header name should not prevent detection",
                    byte
                );
            }
        }

        #[test]
        fn control_chars_in_header_value_breaks_prefix() {
            for byte in [0x01u8, 0x02, 0x0B, 0x1F] {
                let value = format!("Bearer{}token123456789012345", char::from(byte));
                let params = serde_json::json!({
                    "method": "GET",
                    "url": "https://example.com",
                    "headers": {"Authorization": value}
                });
                // Header name "Authorization" is an exact match — always detected
                // regardless of value content. No panic is secondary assertion.
                assert!(
                    params_contain_manual_credentials(&params),
                    "Authorization header name should be detected regardless of value content"
                );
            }
        }

        #[test]
        fn bom_prefix_in_url() {
            let params = serde_json::json!({
                "method": "GET",
                "url": "\u{FEFF}https://api.example.com/data?api_key=secret"
            });
            // BOM before "https://" makes url::Url::parse fail, so
            // query param detection returns false. Document this.
            let result = params_contain_manual_credentials(&params);
            assert!(
                !result,
                "BOM prefix makes URL unparseable — query param detection fails (known limitation)"
            );
        }

        #[test]
        fn null_byte_in_query_value() {
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://api.example.com/data?api_key=sec\x00ret"
            });
            // The param NAME "api_key" still matches regardless of value content.
            assert!(
                params_contain_manual_credentials(&params),
                "null byte in query value should not prevent param name detection"
            );
        }

        #[test]
        fn idn_unicode_hostname_with_credential_params() {
            // Internationalized domain name (IDN) with credential query param
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://例え.jp/api?api_key=secret123"
            });
            // url::Url::parse handles IDN. Credential param should still detect.
            assert!(
                params_contain_manual_credentials(&params),
                "IDN hostname should not prevent credential param detection"
            );
        }

        #[test]
        fn non_ascii_header_names_substring_detection() {
            // Header names with various non-ASCII characters — test both
            // detection behavior AND no-panic guarantee.
            let detected_cases = [
                ("🔑Auth", true),       // contains "auth" substring
                ("Autorización", true), // contains "auth" via to_lowercase
                ("Héader-Tökën", true), // contains "token" via "tökën"? No — "ö" ≠ "o"
            ];

            // These should NOT be detected — no auth substring
            let not_detected_cases = [
                "认证",        // Chinese — no ASCII substring match
                "Авторизация", // Russian — no ASCII substring match
            ];

            for name in not_detected_cases {
                let params = serde_json::json!({
                    "method": "GET",
                    "url": "https://example.com",
                    "headers": {name: "some_value"}
                });
                assert!(
                    !params_contain_manual_credentials(&params),
                    "non-ASCII header '{}' should not be detected (no ASCII auth substring)",
                    name
                );
            }

            // "🔑Auth" contains "auth" substring
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://example.com",
                "headers": {"🔑Auth": "some_value"}
            });
            assert!(
                params_contain_manual_credentials(&params),
                "emoji+Auth header should be detected via 'auth' substring"
            );

            // "Autorización" lowercases to "autorización" — does NOT contain
            // "auth" (it has "aut" + "o", not "auth"). Document this.
            let params = serde_json::json!({
                "method": "GET",
                "url": "https://example.com",
                "headers": {"Autorización": "some_value"}
            });
            assert!(
                !params_contain_manual_credentials(&params),
                "Spanish 'Autorización' does not contain 'auth' substring — not detected"
            );

            let _ = detected_cases; // suppress unused warning
        }
    }
}

//! Fit tool names to the tightest length + charset limit any supported LLM
//! provider enforces on `tool.name` / `function.name`, so a long registered
//! name does not break the wire.
//!
//! AWS Bedrock's `ToolSpecification.name` is documented as
//! `[a-zA-Z0-9_-]{1,64}` and rejects over-length names with HTTP 400. OpenAI's
//! Responses API enforces the same `^[a-zA-Z0-9_-]{1,64}$` regex. Anthropic,
//! Gemini, and rig-based providers apply similar caps at the wire — the
//! symptom is always "every call after this MCP server connected fails".
//!
//! Tool names come from many sources — MCP tools
//! (`{server}_{tool}` built in `src/tools/mcp/client.rs`), WASM tools
//! (user-authored manifest names), skills, routines, future extensions.
//! Catching the length mismatch at one boundary (the tool registry) beats
//! threading per-provider reverse maps through every `complete_with_tools`
//! implementation, because the fit is **deterministic**: the LLM-emitted
//! fitted name can be resolved back to the registered raw name via a single
//! alias branch in `ToolRegistry::resolve_key`, mirroring the existing
//! hyphen/underscore alias pattern.

use sha2::{Digest, Sha256};

/// The tightest `tool.name` length limit any supported LLM provider enforces.
/// Bedrock and OpenAI Responses both cap at 64; treating this as the
/// universal upper bound is defense-in-depth — future strict providers get
/// the fit for free.
pub const PROVIDER_TOOL_NAME_LIMIT: usize = 64;

/// 12 hex chars of SHA-256 = 48 bits of hash. Collision probability across
/// the n tools exposed by a single server is negligible at that width.
const HASH_HEX_LEN: usize = 12;
/// `_` + 12 hex chars. Separator makes the hashed suffix visually distinct
/// from the truncated prefix.
const HASH_SUFFIX_LEN: usize = 1 + HASH_HEX_LEN;

/// Deterministically fit `name` into `limit` characters using an alphanumeric
/// + underscore + dash charset. If `name` already fits, returned unchanged.
///
/// Over-length input is truncated at a UTF-8 char boundary to `limit - 13`
/// chars, joined to `_` + 12 hex chars of `SHA-256(name)`. Any non-
/// `[A-Za-z0-9_-]` byte in the truncated prefix is normalized to `_` to keep
/// the result inside the strict provider charset — mirrors the sanitization
/// `mcp_tool_id` and `sanitize_tool_name` already apply upstream.
///
/// Determinism is load-bearing: the fitted name becomes an alias key that
/// `ToolRegistry::resolve_key` uses to route LLM-emitted tool calls back to
/// the registered tool. A non-deterministic fit would break dispatch on
/// process restart.
pub fn fit_tool_name(name: &str, limit: usize) -> String {
    // Byte length is always >= char count, so `len() <= limit` is a
    // sufficient O(1) fast path. This runs on every tool emitted to every
    // LLM turn, so skipping the O(n) codepoint walk matters.
    if name.len() <= limit {
        return name.to_string();
    }

    let prefix_budget = limit.saturating_sub(HASH_SUFFIX_LEN);

    // Walk chars (not bytes) so we never slice mid-codepoint for multi-byte
    // input (e.g. emoji in a user-authored manifest name).
    let truncated: String = name
        .chars()
        .take(prefix_budget)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();

    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    let digest = hasher.finalize();
    let hex_suffix = hex::encode(&digest[..HASH_HEX_LEN / 2]);

    if prefix_budget == 0 {
        // Pathologically small limit — return just the hash, trimmed to fit.
        return hex_suffix.chars().take(limit).collect();
    }

    format!("{truncated}_{hex_suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_path_short_name_unchanged() {
        assert_eq!(fit_tool_name("echo", PROVIDER_TOOL_NAME_LIMIT), "echo");
        assert_eq!(
            fit_tool_name("memory_search", PROVIDER_TOOL_NAME_LIMIT),
            "memory_search"
        );
    }

    #[test]
    fn boundary_exactly_64_unchanged() {
        let name = "a".repeat(64);
        assert_eq!(fit_tool_name(&name, PROVIDER_TOOL_NAME_LIMIT), name);
    }

    #[test]
    fn over_budget_fits_to_limit() {
        let name = "a".repeat(65);
        let fitted = fit_tool_name(&name, PROVIDER_TOOL_NAME_LIMIT);
        assert_eq!(fitted.chars().count(), PROVIDER_TOOL_NAME_LIMIT);
    }

    #[test]
    fn fitted_matches_provider_charset() {
        let name = format!("some_mcp_server_name_{}", "x".repeat(100));
        let fitted = fit_tool_name(&name, PROVIDER_TOOL_NAME_LIMIT);
        assert!(fitted.chars().count() <= PROVIDER_TOOL_NAME_LIMIT);
        assert!(
            fitted
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "fitted name must match [a-zA-Z0-9_-]: got {:?}",
            fitted
        );
    }

    #[test]
    fn fit_is_deterministic() {
        let name = "very_long_server_name_".repeat(10);
        let a = fit_tool_name(&name, PROVIDER_TOOL_NAME_LIMIT);
        let b = fit_tool_name(&name, PROVIDER_TOOL_NAME_LIMIT);
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_long_inputs_produce_distinct_fits() {
        let a = format!("prefix_{}_suffix_a", "x".repeat(80));
        let b = format!("prefix_{}_suffix_b", "x".repeat(80));
        assert_ne!(
            fit_tool_name(&a, PROVIDER_TOOL_NAME_LIMIT),
            fit_tool_name(&b, PROVIDER_TOOL_NAME_LIMIT),
            "hash suffix must disambiguate inputs that share a prefix"
        );
    }

    #[test]
    fn utf8_input_no_panic() {
        // 70 "smiling face" emoji — each 4 UTF-8 bytes, so walking chars is
        // the only safe way to truncate. A naive byte slice would panic.
        let name = "\u{1F600}".repeat(70);
        let fitted = fit_tool_name(&name, PROVIDER_TOOL_NAME_LIMIT);
        assert!(fitted.chars().count() <= PROVIDER_TOOL_NAME_LIMIT);
        // Non-ASCII codepoints in the prefix get normalized to `_`.
        assert!(
            fitted
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
    }

    #[test]
    fn sanitizes_invalid_chars_in_prefix() {
        // Raw MCP-style name with dots/colons — mcp_tool_id normalizes these
        // upstream, but defense-in-depth keeps us inside the charset even if
        // a future tool source skips that step.
        let name = format!("bad.char:name_{}", "x".repeat(80));
        let fitted = fit_tool_name(&name, PROVIDER_TOOL_NAME_LIMIT);
        assert!(
            fitted
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
    }
}

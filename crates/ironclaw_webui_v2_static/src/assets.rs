//! Embedded asset bytes.
//!
//! Populated at compile time by `build.rs` from
//! `crates/ironclaw_webui_v2_static/static/`. Each file becomes one
//! `Asset` row keyed by its URL path (relative to the `/v2` mount
//! prefix). `index.html` is handled separately — see
//! [`INDEX_HTML_TEMPLATE`].

pub(crate) struct Asset {
    pub bytes: &'static [u8],
    pub content_type: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/assets_generated.rs"));

pub(crate) fn lookup(path: &str) -> Option<&'static Asset> {
    // Path table is sorted at build time; binary search keeps the
    // per-request work O(log n) without pulling in a hash map.
    ASSETS
        .binary_search_by(|(p, _)| (*p).cmp(path))
        .ok()
        .map(|idx| &ASSETS[idx].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset_text(path: &str) -> &'static str {
        std::str::from_utf8(lookup(path).expect("asset exists").bytes).expect("asset is utf-8")
    }

    #[test]
    fn lookup_returns_none_for_unknown_path() {
        // Direct coverage of the `None` arm. The router-level tests
        // exercise the `Some` path via known assets and the SPA-shell
        // fallback for unknown paths, but neither directly asserts
        // that the asset table itself returns `None` — a future
        // refactor that swaps `binary_search_by` for something that
        // returns the closest match instead would regress this
        // contract silently without this guard.
        assert!(lookup("nonexistent.js").is_none());
        assert!(lookup("../etc/passwd").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn chat_auth_gate_assets_submit_manual_token_then_resolve_gate() {
        let auth_card = asset_text("js/pages/chat/components/auth-token-card.js");
        assert!(auth_card.contains("await onSubmit(value);"));
        assert!(auth_card.contains("setToken(\"\");"));
        assert!(auth_card.contains("t(\"authGate.submitFailed\")"));
        assert!(auth_card.contains("authGate.resolveFailedAfterTokenSaved"));
        assert!(!auth_card.contains("err?.message"));

        let api = asset_text("js/lib/api.js");
        assert!(api.contains("/api/reborn/product-auth/manual-token/submit"));
        assert!(api.contains("signal,"));
        assert!(api.contains("account_label: accountLabel"));
        assert!(api.contains("gate_ref: gateRef"));

        let use_chat = asset_text("js/pages/chat/hooks/useChat.js");
        assert!(use_chat.contains("AUTH_TOKEN_FLOW_TIMEOUT_MS"));
        assert!(use_chat.contains("authTokenSubmitRef"));
        assert!(use_chat.contains("submitManualToken({"));
        assert!(use_chat.contains("authTokenSubmitRef.current.credentialRef"));
        assert!(use_chat.contains("authTokenSubmitRef.current.inFlight"));
        assert!(use_chat.contains("throw new Error(\"auth gate is no longer pending\")"));
        assert!(
            use_chat
                .contains("throw new Error(\"auth gate is missing required credential metadata\")")
        );
        assert!(use_chat.contains("resolveGateRequest({"));
        assert!(use_chat.contains("resolution: \"credential_provided\""));
        assert!(use_chat.contains("credentialRef"));
        assert!(use_chat.contains("safeAuthGateCode"));
    }
}

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
        assert!(use_chat.contains("submitResponseResumedTurnGate"));
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
        assert!(use_chat.contains("continuation?.type === \"turn_gate_resume\""));
        assert!(use_chat.contains("credentialRef"));
        assert!(use_chat.contains("safeAuthGateCode"));
    }

    #[test]
    fn chat_cancelled_gate_resolution_exits_processing_state() {
        let use_chat = asset_text("js/pages/chat/hooks/useChat.js");
        assert!(
            use_chat
                .contains("resolution === \"approved\" || resolution === \"credential_provided\"")
        );
        assert!(use_chat.contains("setIsProcessing(shouldContinueProcessing);"));
        assert!(use_chat.contains("setActiveRun(null);"));

        let events = asset_text("js/pages/chat/lib/useChatEvents.js");
        assert!(events.contains("TERMINAL_RUN_STATUSES.has(status)"));
        assert!(events.contains("setPendingGate(null);"));
        assert!(events.contains("setActiveRun?.(null);"));
        assert!(events.contains("latestRunIdRef.current = null;"));
    }

    #[test]
    fn chat_projection_text_preserves_pending_gate() {
        let events = asset_text("js/pages/chat/lib/useChatEvents.js");
        let text_branch = events
            .split("if (item.text)")
            .nth(1)
            .expect("text projection branch exists")
            .split("if (item.thinking)")
            .next()
            .expect("thinking branch follows text branch");
        assert!(
            text_branch.contains("terminal run_status is the only"),
            "text branch should document that run_status owns gate clearing"
        );
        assert!(
            !text_branch.contains("setPendingGate(null);"),
            "projection text must not hide a still-blocked auth gate"
        );
    }

    #[test]
    fn extensions_onboarding_messages_render_in_cards() {
        let extension_card = asset_text("js/pages/extensions/components/extension-card.js");

        assert!(
            extension_card.contains("state === \"setup_required\" || state === \"auth_required\""),
            "setup/auth states must prefer credential setup instructions"
        );
        assert!(
            extension_card.contains(
                "ext.onboarding?.credential_instructions || ext.onboarding?.credential_next_step"
            ),
            "setup/auth onboarding should render credential instructions before next-step copy"
        );
        assert!(
            extension_card.contains(
                "ext.onboarding?.credential_next_step || ext.onboarding?.credential_instructions"
            ),
            "configured/no-credential onboarding should render next-step copy before setup copy"
        );
        assert!(
            extension_card.contains("${onboardingHint}"),
            "extension cards must render the projected onboarding hint"
        );
    }
}

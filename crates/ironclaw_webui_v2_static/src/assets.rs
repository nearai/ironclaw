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
    fn chat_pending_reconciliation_has_caller_level_js_regression() {
        let use_chat = asset_text("js/pages/chat/hooks/useChat.js");
        assert!(use_chat.contains("recordAcceptedMessageRef("));
        assert!(use_chat.contains("pendingMessagesRef.current"));
        assert!(use_chat.contains("response?.accepted_message_ref"));

        let pending_messages = asset_text("js/pages/chat/lib/pending-messages.js");
        assert!(pending_messages.contains("timelineMessageIdFromAcceptedRef"));
        assert!(
            pending_messages
                .contains("return ref.startsWith(\"msg:\") ? ref.slice(\"msg:\".length) : null;")
        );

        let regression = asset_text("js/pages/chat/lib/useChat-send.test.mjs");
        assert!(regression.contains("useChat.send: accepted ref reconciles"));
        assert!(regression.contains("accepted_message_ref: \"msg:message-1\""));
        assert!(regression.contains("await loadHistory();"));
        assert!(regression.contains("[\"msg-message-1\"]"));

        let pending_regression = asset_text("js/pages/chat/lib/pending-messages.test.mjs");
        assert!(pending_regression.contains(
            "recordAcceptedMessageRef: null and non-msg refs leave pending record unchanged"
        ));
        assert!(pending_regression.contains("\"thread:1\""));
        assert!(pending_regression.contains("\"message-1\""));
    }

    #[test]
    fn chat_connect_action_assets_render_slack_pairing_and_extensions_channel_picker() {
        let chat = asset_text("js/pages/chat/chat.js");
        assert!(chat.contains("ChannelConnectCard"));
        assert!(chat.contains("channelConnectAction"));
        assert!(chat.contains("dismissChannelConnectAction"));

        let card = asset_text("js/pages/chat/components/channel-connect-card.js");
        assert!(card.contains("SlackPairingSection"));
        assert!(card.contains("isSlackStrategy(connectAction, \"inbound_proof_code\")"));
        assert!(card.contains("action=${connectAction.action}"));

        let picker = asset_text("js/components/slack-channel-picker.js");
        assert!(picker.contains("listSlackAllowedChannels"));
        assert!(picker.contains("saveSlackAllowedChannels(channels)"));

        let channels_tab = asset_text("js/pages/extensions/components/channels-tab.js");
        assert!(channels_tab.contains("slackBuiltinStatus"));
        assert!(channels_tab.contains("admin_managed_channels"));
        assert!(channels_tab.contains("inbound_proof_code"));
        assert!(channels_tab.contains("SlackChannelPicker"));
        assert!(channels_tab.contains("SlackPairingSection"));
        assert!(channels_tab.contains("findSlackConnectActions"));
        assert!(channels_tab.contains("slackConnectActions"));
        assert!(channels_tab.contains("action=${action.action}"));

        let regression = asset_text("js/pages/chat/lib/useChat-send.test.mjs");
        assert!(regression.contains("channel connect requests return an action"));
        assert!(regression.contains("without submitting a prompt"));
        assert!(regression.contains("unmatched channel connect requests submit the prompt"));
    }

    #[test]
    fn automations_panel_assets_are_embedded() {
        let app = asset_text("js/app/app.js");
        assert!(app.contains("AutomationsPage"));
        assert!(app.contains("path=\"automations\""));

        let routes = asset_text("js/app/routes.js");
        assert!(routes.contains("nav.automations"));
        assert!(routes.contains("path: \"/automations\""));

        let api = asset_text("js/lib/api.js");
        assert!(api.contains("listAutomations"));
        assert!(api.contains("/automations"));
        assert!(api.contains("getOutboundPreferences"));
        assert!(api.contains("setOutboundPreferences"));
        assert!(api.contains("/outbound/preferences"));
        assert!(api.contains("/outbound/targets"));

        let page = asset_text("js/pages/automations/automations-page.js");
        assert!(page.contains("AutomationsSummaryStrip"));
        assert!(page.contains("AutomationDeliveryDefaultsPanel"));
        assert!(page.contains("useOutboundDeliveryDefaults"));
        assert!(page.contains("AutomationsList"));

        let defaults_panel =
            asset_text("js/pages/automations/components/automation-delivery-defaults-panel.js");
        assert!(defaults_panel.contains("finalReplyTargets"));
        assert!(defaults_panel.contains("saveFinalReplyTarget"));
        // Badge label must branch on optStatus — unavailable targets must not
        // display the "ready" label.
        assert!(
            defaults_panel.contains("automations.delivery.pill.unavailable"),
            "unavailable badge label key must be used in the target option rows"
        );
        assert!(
            !defaults_panel.contains(r#"label=${t("automations.delivery.pill.ready")}"#),
            "target option badge label must not be unconditionally hardcoded to .pill.ready"
        );

        let defaults_hook = asset_text("js/pages/automations/hooks/useOutboundDeliveryDefaults.js");
        assert!(defaults_hook.contains("listOutboundDeliveryTargets"));
        assert!(defaults_hook.contains("setOutboundPreferences"));

        let presenter = asset_text("js/pages/automations/lib/automations-presenters.js");
        assert!(presenter.contains("source?.type === \"schedule\""));
        assert!(presenter.contains("Custom schedule"));
        assert!(!presenter.contains("Webhook"));
    }

    #[test]
    fn auth_session_assets_use_server_capabilities_for_admin_status() {
        let api = asset_text("js/lib/api.js");
        assert!(api.contains("fetchSession"));
        assert!(api.contains("/session"));

        let auth = asset_text("js/app/auth.js");
        assert!(auth.contains("fetchSession()"));
        assert!(auth.contains("operator_webui_config"));
        assert!(auth.contains("err?.status === 401 || err?.status === 403"));
        assert!(auth.contains("Your session expired. Please sign in again."));
        assert!(auth.contains("setIsSessionChecking(Boolean(nextToken))"));
        assert!(auth.contains("setIsSessionChecking(true);"));
        assert!(auth.contains("isAdmin: Boolean(session?.capabilities?.operator_webui_config)"));
        assert!(!auth.contains("isAdmin: false"));

        let sidebar_nav = asset_text("js/components/sidebar-nav.js");
        assert!(sidebar_nav.contains("isAdmin = false"));
        assert!(sidebar_nav.contains("[\"users\", \"inference\"].includes(subRoute.id)"));

        let settings_page = asset_text("js/pages/settings/settings-page.js");
        assert!(settings_page.contains("isAdmin = false"));
        assert!(settings_page.contains("const defaultTabIsVisible = tabContentHas(defaultTab)"));
        assert!(settings_page.contains("const redirectTab = defaultTabIsVisible"));
        assert!(settings_page.contains("isOperatorTab(tab)"));

        let settings_tabs = asset_text("js/pages/settings/components/settings-tabs.js");
        assert!(settings_tabs.contains("isAdmin = false"));
        assert!(!settings_tabs.contains("isAdmin = true"));
        assert!(settings_tabs.contains("tab.id !== \"inference\""));

        let layout = asset_text("js/layout/gateway-layout.js");
        assert!(layout.contains("enabled: isAdmin"));
        assert!(layout.contains("const needsOnboarding ="));
        assert!(layout.contains("isAdmin &&"));
        assert!(layout.contains("shouldRouteToOnboarding({"));

        let app = asset_text("js/app/app.js");
        assert!(app.contains("isChecking=${auth.isChecking}"));

        let providers = asset_text("js/pages/settings/hooks/useLlmProviders.js");
        assert!(providers.contains("const hasActiveProvider = Boolean("));
        assert!(!providers.contains("!enabled || Boolean"));

        let onboarding = asset_text("js/pages/onboarding/onboarding-page.js");
        assert!(onboarding.contains("isChecking = false"));
        assert!(onboarding.contains("if (isChecking) return null;"));
        assert!(onboarding.contains("if (!isAdmin)"));
        assert!(onboarding.contains("OperatorOnboardingPage"));
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
            text_branch.contains("run_status remains the source of"),
            "text branch should document that run_status owns gate clearing"
        );
        assert!(
            !text_branch.contains("setPendingGate(null);"),
            "projection text must not hide a still-blocked auth gate"
        );
    }

    #[test]
    fn chat_message_grouping_hoists_only_final_replies() {
        let groups = asset_text("js/pages/chat/lib/message-groups.js");
        assert!(groups.contains("function isFinalAssistantReply"));
        assert!(groups.contains("msg.isFinalReply === true"));
        assert!(groups.contains("msg.status === \"finalized\""));
        assert!(groups.contains("function followingActivity"));
        assert!(groups.contains("type: \"activity-run\""));
        assert!(groups.contains("appendActivityRun(items, activity);"));
        assert!(!groups.contains("lastAssistantReplyIndex"));

        let history = asset_text("js/pages/chat/lib/history-messages.js");
        assert!(history.contains("isFinalReply: isFinalAssistantRecord(record)"));
        assert!(history.contains("record.status === \"finalized\""));

        let events = asset_text("js/pages/chat/lib/useChatEvents.js");
        assert!(events.contains("isFinalReply: true"));
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

    #[test]
    fn extension_oauth_setup_refreshes_while_popup_is_open() {
        let use_extensions = asset_text("js/pages/extensions/hooks/useExtensions.js");

        assert!(
            use_extensions.contains("OAUTH_SETUP_REFRESH_MS = 2000"),
            "OAuth setup should poll often enough for setup-complete state to appear promptly"
        );
        assert!(
            use_extensions.contains("const watchOauthProgress = React.useCallback"),
            "OAuth setup should watch in-flight authorization, not only popup close"
        );
        assert!(
            use_extensions.contains(
                "refreshSetupState();\n        if (\n          setupIsConfigured() ||\n          (popup && popup.closed)"
            ),
            "OAuth setup must refresh setup state before waiting for popup close"
        );
    }
}

//! Embedded asset bytes.
//!
//! Populated at compile time by `build.rs` from
//! `crates/ironclaw_webui_v2/static/`. Each file becomes one
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

    /// Read a source file straight from `static/` on disk. Used for fixtures
    /// that are deliberately *not* embedded/served — e.g. `*.test.mjs` Node
    /// unit tests — so a caller-level JS regression can still assert their
    /// content without shipping them to clients.
    fn source_text(path: &str) -> String {
        let full = format!("{}/static/{path}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("read {full}: {e}"))
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
    fn project_file_download_chips_are_wired() {
        // The download UI: message bubble renders chips fed by extracted
        // workspace paths. Each chip is the shared `AttachmentChip` fed a
        // descriptor whose `fetch_url` targets the bearer-authenticated v2
        // `/files/content` endpoint, so clicking opens the same
        // `AttachmentPreviewModal` (image/pdf/text preview + Download) as a
        // message attachment.
        let chips = asset_text("js/pages/chat/components/project-file-chips.js");
        assert!(chips.contains("extractWorkspaceFilePaths"));
        assert!(chips.contains("statProjectFile"));
        assert!(chips.contains("projectFileContentUrl"));
        assert!(chips.contains("AttachmentChip"));
        assert!(chips.contains("AttachmentPreviewModal"));
        // The chip passes the e2e selector hooks through to the shared chip,
        // including the inline one-click download icon.
        assert!(chips.contains("project-file-chip"));
        assert!(chips.contains("project-file-download"));
        assert!(chips.contains("dataPath"));

        let bubble = asset_text("js/pages/chat/components/message-bubble.js");
        assert!(bubble.contains("ProjectFileChips"));
        assert!(bubble.contains("threadId"));

        // Both surfaces share the chip + preview implementation, so message
        // attachments and project files cannot drift. The shared chip renders
        // the stable e2e selector attributes.
        let chip = asset_text("js/pages/chat/components/attachment-chip.js");
        assert!(chip.contains("export function AttachmentChip"));
        assert!(chip.contains("export function AttachmentThumbnail"));
        assert!(chip.contains("data-testid=${testId}"));
        assert!(chip.contains("data-file-path=${dataPath}"));
        // The inline download icon fetches the bearer-authenticated bytes and
        // saves them directly (separate from the preview modal's own Download).
        assert!(chip.contains("data-testid=${downloadTestId}"));
        assert!(chip.contains("fetchAttachmentBlob"));
        assert!(chip.contains("saveBlob"));
        assert!(bubble.contains("AttachmentChip"));

        // The preview modal fetches the blob (bearer-authenticated, via the
        // shared `fetchAttachmentBlob`) and offers a Download action with a
        // stable test hook.
        let preview = asset_text("js/pages/chat/components/attachment-preview.js");
        assert!(preview.contains("fetchAttachmentBlob"));
        assert!(preview.contains("data-testid=\"attachment-download\""));

        // The api client exposes a same-origin content URL helper and keeps the
        // bearer-authenticated blob fetch; it does no DOM (object URLs live in
        // the preview modal / `download.js`).
        let api = asset_text("js/lib/api.js");
        assert!(api.contains("projectFilesBase"));
        assert!(api.contains("/content"));
        assert!(api.contains("projectFileContentUrl"));
        assert!(api.contains("Authorization"));
        assert!(!api.contains("createObjectURL"));

        // The pure extraction module is served; its test sibling is not.
        assert!(lookup("js/pages/chat/lib/project-file-paths.js").is_some());
        assert!(lookup("js/pages/chat/lib/project-file-paths.test.js").is_none());
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
    fn chat_input_persists_staged_attachments_across_navigation() {
        // The composer keeps a text draft across navigation; staged attachments
        // must follow a parallel per-key store so they are not silently dropped
        // when the composer unmounts and remounts (e.g. leaving the new-chat
        // screen and returning). The store is in-memory because the files carry
        // base64 bytes that would blow localStorage's quota.
        let store = asset_text("js/pages/chat/lib/draft-store.js");
        assert!(store.contains("export function getStagedAttachments"));
        assert!(store.contains("export function setStagedAttachments"));
        assert!(store.contains("export function clearStagedAttachments"));
        // Sign-out drops the in-memory staged files too, so they can't resurface
        // for the next user on the same browser.
        assert!(store.contains("stagedAttachments.clear()"));

        let input = asset_text("js/pages/chat/components/chat-input.js");
        // Initialized from the store (not a bare `[]`), persisted on change, and
        // cleared on a successful send.
        assert!(input.contains("getStagedAttachments(draftKey)"));
        assert!(input.contains("setStagedAttachments(draftKey"));
        assert!(input.contains("clearStagedAttachments(draftKey)"));
    }

    #[test]
    fn chat_gate_resolution_uses_resume_outcome() {
        let use_chat = asset_text("js/pages/chat/hooks/useChat.js");
        // Gate resolution normally resumes the run, but stale/terminal gate
        // responses must not synthesize a processing state.
        assert!(use_chat.contains("const outcome = resolveGateOutcome(response);"));
        assert!(use_chat.contains("if (outcome === \"resumed\")"));
        assert!(use_chat.contains("setIsProcessing(true);"));
        assert!(use_chat.contains("setIsProcessing(false);"));
        assert!(use_chat.contains("setActiveRun(null);"));
        assert!(!use_chat.contains("setIsProcessing(shouldContinueProcessing);"));

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

        let regression = source_text("js/pages/chat/lib/useChat-send.test.mjs");
        assert!(regression.contains("useChat.send: accepted ref reconciles"));
        assert!(regression.contains("accepted_message_ref: \"msg:message-1\""));
        assert!(regression.contains("await loadHistory();"));
        assert!(regression.contains("[\"msg-message-1\"]"));

        let pending_regression = source_text("js/pages/chat/lib/pending-messages.test.mjs");
        assert!(pending_regression.contains(
            "recordAcceptedMessageRef: null and non-msg refs leave pending record unchanged"
        ));
        assert!(pending_regression.contains("\"thread:1\""));
        assert!(pending_regression.contains("\"message-1\""));
    }

    #[test]
    fn markdown_code_blocks_keep_horizontal_scroll_local_to_block() {
        let renderer = asset_text("js/pages/chat/components/markdown-renderer.js");
        assert!(renderer.contains("wrap.className = \"markdown-code-frame\";"));
        assert!(renderer.contains("pre.style.overflowX = \"auto\";"));
        assert!(renderer.contains("pre.style.overflowY = \"hidden\";"));
        assert!(!renderer.contains("pre.style.overflow = \"hidden\";"));
        assert!(!renderer.contains("codeEl.style.whiteSpace"));

        let styles = asset_text("styles/app.css");
        assert!(styles.contains(".markdown-body {\n  max-width: 100%;\n  min-width: 0;\n}"));
        assert!(styles.contains(".markdown-code-frame {\n  position: relative;"));
        assert!(styles.contains("width: 100%;\n  max-width: 100%;\n  min-width: 0;"));
        assert!(styles.contains("overflow: hidden;"));
        assert!(styles.contains("border-radius: 8px; box-sizing: border-box; width: 100%;"));
        assert!(styles.contains("overflow-x: auto; white-space: pre; margin-bottom: 0.75em;"));
        assert!(styles.contains("display: inline; background: transparent; padding: 0;"));
        assert!(styles.contains("font-size: 0.9em; line-height: 1.65; white-space: inherit;"));
        assert!(!styles.contains("width: max-content"));

        let message_list = asset_text("js/pages/chat/components/message-list.js");
        assert!(message_list.contains("relative flex min-h-0 min-w-0 flex-1"));
        assert!(message_list.contains("flex min-w-0 flex-1 overflow-y-auto"));
        assert!(!message_list.contains("overflow-x-hidden"));
        assert!(message_list.contains("mx-auto flex w-full min-w-0 max-w-5xl flex-col"));

        let message_bubble = asset_text("js/pages/chat/components/message-bubble.js");
        assert!(message_bubble.contains("group flex w-full min-w-0 flex-col"));
        assert!(message_bubble.contains(
            "const bubbleWidthClass = isUser ? \"max-w-[85%]\" : isNotice ? \"mx-auto max-w-[85%]\" : \"w-full max-w-[85%]\";"
        ));
        assert!(
            message_bubble.contains(
                "const contentWidthClass = isUser ? \"\" : \"w-full min-w-0 max-w-full\";"
            )
        );
        assert!(message_bubble.contains("contentWidthClass,"));
    }

    #[test]
    fn chat_omits_connect_action_while_extensions_render_slack_setup_ui() {
        let chat = asset_text("js/pages/chat/chat.js");
        assert!(!chat.contains("ChannelConnectCard"));
        assert!(!chat.contains("channelConnectAction"));
        assert!(!chat.contains("dismissChannelConnectAction"));

        let picker = asset_text("js/components/slack-channel-picker.js");
        assert!(picker.contains("listSlackAllowedChannels"));
        assert!(picker.contains("saveSlackAllowedChannels(channels)"));

        let setup_panel = asset_text("js/components/slack-setup-panel.js");
        assert!(setup_panel.contains("SlackChannelPicker"));
        assert!(setup_panel.contains("<${SlackChannelPicker} action=${action} />"));

        let channels_tab = asset_text("js/pages/extensions/components/channels-tab.js");
        assert!(channels_tab.contains("showBuiltinSlackConnectActions"));
        assert!(channels_tab.contains("admin_managed_channels"));
        assert!(channels_tab.contains("inbound_proof_code"));
        assert!(channels_tab.contains("SlackAdminManagedSection"));
        assert!(channels_tab.contains("SlackPairingSection"));
        assert!(channels_tab.contains("findSlackConnectActions"));
        assert!(channels_tab.contains("slackConnectActions"));
        assert!(channels_tab.contains("action=${action.action}"));

        let regression = source_text("js/pages/chat/lib/useChat-send.test.mjs");
        assert!(regression.contains(
            "slash connect text submits to the model without fetching connectable channels"
        ));
        assert!(regression.contains("ordinary Slack chat prompts submit to the model"));
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
        assert!(api.contains("pauseAutomation"));
        assert!(api.contains("resumeAutomation"));
        assert!(api.contains("deleteAutomation"));
        assert!(api.contains("/automations"));
        assert!(api.contains("/pause"));
        assert!(api.contains("/resume"));
        assert!(api.contains(r#"method: "DELETE""#));
        assert!(api.contains("getOutboundPreferences"));
        assert!(api.contains("setOutboundPreferences"));
        assert!(api.contains("/outbound/preferences"));
        assert!(api.contains("/outbound/targets"));

        let page = asset_text("js/pages/automations/automations-page.js");
        assert!(page.contains("AutomationsSummaryStrip"));
        assert!(page.contains("AutomationDeliveryDefaultsPanel"));
        assert!(page.contains("useOutboundDeliveryDefaults"));
        assert!(page.contains("AutomationsList"));

        let automations_hook = asset_text("js/pages/automations/hooks/useAutomations.js");
        assert!(automations_hook.contains("AUTOMATIONS_BASE_REFETCH_MS"));
        assert!(automations_hook.contains("nextAutomationsRefetchDelay"));
        assert!(automations_hook.contains("query.refetch()"));

        let list = asset_text("js/pages/automations/components/automations-list.js");
        assert!(list.contains("primary_status_label"));
        assert!(list.contains("primary_status_tone"));

        let detail_panel = asset_text("js/pages/automations/components/automation-detail-panel.js");
        assert!(detail_panel.contains("onPauseAutomation"));
        assert!(detail_panel.contains("onResumeAutomation"));
        assert!(detail_panel.contains("onDeleteAutomation"));
        assert!(detail_panel.contains("automation.state === \"active\""));
        assert!(detail_panel.contains("automation.state === \"scheduled\""));
        assert!(detail_panel.contains("primary_status_label"));
        assert!(detail_panel.contains("primary_status_tone"));
        assert!(detail_panel.contains("window.confirm"));

        let app_bundle = asset_text("dist/app.js");
        let app_bundle_contains_encoded_automation_route = |suffix: &str| {
            app_bundle
                .split("/automations/${encodeURIComponent(")
                .any(|tail| tail.contains(&format!(")}}/{suffix}")))
        };
        assert!(
            app_bundle_contains_encoded_automation_route("pause"),
            "served WebUI bundle must include the automation pause endpoint; run frontend build after editing static/js/**"
        );
        assert!(
            app_bundle_contains_encoded_automation_route("resume"),
            "served WebUI bundle must include the automation resume endpoint; run frontend build after editing static/js/**"
        );
        let app_bundle_contains_encoded_automation_delete = app_bundle
            .split("/automations/${encodeURIComponent(")
            .any(|tail| {
                let near = tail.chars().take(220).collect::<String>();
                near.contains("method:\"DELETE\"")
                    && !near.contains("/pause")
                    && !near.contains("/resume")
            });
        assert!(
            app_bundle_contains_encoded_automation_delete,
            "served WebUI bundle must include the automation delete endpoint; run frontend build after editing static/js/**"
        );

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
    fn sidebar_new_chat_label_owns_typography() {
        let sidebar_nav = asset_text("js/components/sidebar-nav.js");

        assert!(sidebar_nav.contains("<span className=\"text-[13px] font-medium\""));
        assert!(sidebar_nav.contains("t(\"chat.newThread\")"));
    }

    #[test]
    fn desktop_sidebar_toggle_assets_are_wired() {
        let header = asset_text("js/components/page-header.js");
        assert!(header.contains("type=\"button\""));
        assert!(header.contains("aria-label=\"Toggle sidebar\""));
        assert!(header.contains("aria-controls=\"gateway-sidebar\""));
        assert!(header.contains("aria-expanded=${sidebarOpen ? \"true\" : \"false\"}"));
        assert!(!header.contains("md:hidden"));

        let sidebar = asset_text("js/components/sidebar.js");
        assert!(sidebar.contains("id=${id}"));

        let sidebar_state = asset_text("js/lib/sidebar-state.js");
        assert!(sidebar_state.contains("ironclaw:v2-sidebar-open"));
        assert!(sidebar_state.contains("export function readDesktopSidebarOpen"));
        assert!(sidebar_state.contains("export function writeDesktopSidebarOpen"));
        assert!(sidebar_state.contains("matchMedia?.(\"(min-width: 768px)\")"));
        assert!(sidebar_state.contains("export function currentSidebarOpen"));

        let hook = asset_text("js/hooks/useSidebar.js");
        assert!(hook.contains("from \"../lib/sidebar-state.js\""));
        assert!(hook.contains("desktopOpen: readDesktopSidebarOpen()"));
        assert!(hook.contains("React.useState(() =>"));
        assert!(hook.contains("isDesktopSidebarViewport()"));
        assert!(hook.contains("setIsDesktopViewport(query.matches)"));
        assert!(hook.contains("toggleSidebarState(current, isDesktopViewport)"));
        assert!(hook.contains("currentOpen: currentSidebarOpen(state, isDesktopViewport)"));

        let layout = asset_text("js/layout/gateway-layout.js");
        assert!(layout.contains("${sidebar.mobileOpen &&"));
        assert!(layout.contains("sidebar.mobileOpen ? \"flex\" : \"hidden\""));
        assert!(layout.contains("sidebar.desktopOpen ? \"md:flex\" : \"md:hidden\""));
        assert!(layout.contains("sidebarOpen=${sidebar.currentOpen}"));

        let bundle = asset_text("dist/app.js");
        assert!(bundle.contains("ironclaw:v2-sidebar-open"));
        assert!(bundle.contains("desktopOpen"));
        assert!(bundle.contains("mobileOpen"));
    }

    #[test]
    fn sidebar_trace_credits_card_assets_are_embedded() {
        // The compact card is mounted in the sidebar above the conversation
        // list and reuses the existing trace-credits hook + endpoint.
        let card = asset_text("js/components/sidebar-trace-credits.js");
        assert!(card.contains("export function SidebarTraceCredits"));
        // Reuses the shared hook (and thus the `/api/webchat/v2/traces/credit`
        // endpoint), not a parallel fetch.
        assert!(card.contains("useTraceCredits"));
        // Renders nothing unless enrolled — keeps the sidebar clean.
        assert!(card.contains("if (!credits || !credits.enrolled) return null;"));
        // Click-through opens the full Settings -> Trace Commons tab.
        assert!(card.contains("to=\"/settings/traces\""));
        assert!(card.contains("traceCommons.cardAccepted"));
        // Held-for-review count surfaces only when there are holds.
        assert!(card.contains("manual_review_hold_count"));
        assert!(card.contains("heldCount > 0"));
        assert!(card.contains("traceCommons.cardHeld"));

        let sidebar = asset_text("js/components/sidebar.js");
        assert!(sidebar.contains("SidebarTraceCredits"));
        // Mounted between the nav and the threads list.
        assert!(sidebar.contains("<${SidebarTraceCredits} />"));

        // The Settings tab lists held traces (reason + submission id) sourced
        // from the credits response `holds[]`.
        let tab = asset_text("js/pages/settings/components/trace-commons-tab.js");
        assert!(tab.contains("const holds = credits.holds || [];"));
        assert!(tab.contains("traceCommons.heldTitle"));
        assert!(tab.contains("traceCommons.heldDescription"));
        assert!(tab.contains("hold.submission_id"));
        assert!(tab.contains("hold.reason"));
        // Per-hold Authorize button wired to the authorize mutation.
        assert!(tab.contains("authorize.mutate(hold.submission_id)"));
        assert!(tab.contains("traceCommons.authorize"));

        // Authorize calls the POST endpoint and invalidates the credits query.
        let settings_api = asset_text("js/pages/settings/lib/settings-api.js");
        assert!(settings_api.contains("export function authorizeTraceHold"));
        assert!(settings_api.contains("/authorize"));
        assert!(settings_api.contains("method: \"POST\""));
        let trace_hook = asset_text("js/pages/settings/hooks/useTraceCredits.js");
        assert!(trace_hook.contains("authorizeTraceHold"));
        assert!(trace_hook.contains("invalidateQueries({ queryKey: [\"trace-credits\"] })"));

        // The hook refetches so the card and Settings tab reflect new
        // accepted submissions without a manual reload. Polling is infrequent
        // (300s) and paused while the tab is hidden; a focus refetch keeps the
        // surface live and staleTime dedupes redundant focus refetches.
        let hook = asset_text("js/pages/settings/hooks/useTraceCredits.js");
        assert!(hook.contains("refetchInterval: 300_000"));
        assert!(hook.contains("refetchIntervalInBackground: false"));
        assert!(hook.contains("refetchOnWindowFocus: true"));
        assert!(hook.contains("staleTime: 60_000"));

        // The new i18n keys are present in the eagerly-bundled English pack
        // (other locales fall back to it if missing, but all 11 carry them).
        let en = asset_text("js/i18n/en.js");
        assert!(en.contains("\"traceCommons.cardAccepted\""));
        assert!(en.contains("\"traceCommons.cardHeld\""));
        assert!(en.contains("\"traceCommons.heldTitle\""));
        assert!(en.contains("\"traceCommons.heldDescription\""));
        assert!(en.contains("\"traceCommons.authorize\""));
        assert!(en.contains("\"traceCommons.authorizing\""));
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
        assert!(
            auth.contains(
                "globalAutoApproveEnabled: Boolean(session?.features?.global_auto_approve)"
            )
        );
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
    fn extension_config_modal_hides_activate_for_active_extensions() {
        let extension_actions = asset_text("js/pages/extensions/lib/extension-actions.js");
        assert!(extension_actions.contains("export function extensionIsActive"));
        assert!(extension_actions.contains("state === \"active\" || state === \"ready\""));
        assert!(
            extension_actions.contains("setupReadyForActivation({ extension"),
            "setup readiness must receive lifecycle state, not only setup fields"
        );
        assert!(
            extension_actions.contains("extensionIsActive(extension)"),
            "active extensions must not be considered ready for another activation"
        );

        let extension_card = asset_text("js/pages/extensions/components/extension-card.js");
        assert!(extension_card.contains("activationStatus: ext.activation_status"));
        assert!(extension_card.contains("onboardingState: ext.onboarding_state"));

        let configure_modal = asset_text("js/pages/extensions/components/configure-modal.js");
        assert!(configure_modal.contains("const isActive = extensionIsActive(extension);"));
        assert!(
            configure_modal.contains("const canActivate = setupReadyForActivation({ extension"),
            "the modal Activate button must be gated by lifecycle-aware setup readiness"
        );
        assert!(configure_modal.contains("extensions.activeConfigured"));

        let regression = source_text("js/pages/extensions/lib/extension-actions.test.mjs");
        assert!(
            regression.contains("extensionIsActive accepts card payload lifecycle fields"),
            "caller-visible card payload lifecycle fields need JS regression coverage"
        );
        assert!(regression.contains("extension: { active: true }"));
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

    #[test]
    fn extension_registry_keeps_installed_entries_visible_first() {
        let use_extensions = asset_text("js/pages/extensions/hooks/useExtensions.js");
        let registry_tab = asset_text("js/pages/extensions/components/registry-tab.js");
        let extensions_page = asset_text("js/pages/extensions/extensions-page.js");
        let routes = asset_text("js/app/routes.js");
        let schema = asset_text("js/pages/extensions/lib/extensions-schema.js");

        assert!(
            use_extensions.contains("catalogEntries"),
            "extensions hook should expose a merged installed-plus-registry catalog"
        );
        assert!(
            use_extensions.contains("catalogId(\"registry\", entry, index)")
                && use_extensions.contains("catalogId(\"installed\", extension, index)"),
            "merged extension catalog should use stable fallback keys for id-less entries"
        );
        assert!(
            use_extensions
                .contains("if (a.installed !== b.installed) return a.installed ? -1 : 1;")
                && use_extensions.contains("displayName(a.entry || a.extension)"),
            "merged extension catalog should sort installed entries before available entries"
        );
        assert!(
            registry_tab.contains("installedEntries") && registry_tab.contains("availableEntries"),
            "registry tab should render installed and available sections from one catalog"
        );
        assert!(
            registry_tab.contains("return entry.entry || entry.extension || {};"),
            "registry search should prefer richer registry metadata when installed entries are available"
        );
        assert!(
            registry_tab.contains("<${ExtensionCard}") && registry_tab.contains("<${RegistryCard}"),
            "installed registry entries should keep management actions while available entries keep install actions"
        );
        assert!(
            extensions_page.contains("tab = \"registry\"")
                && extensions_page.contains("to=\"/extensions/registry\""),
            "extensions page should default and redirect to the unified registry surface"
        );
        assert!(
            !routes.contains("id: \"installed\"") && !schema.contains("id: \"installed\""),
            "installed should no longer be a separate extensions navigation tab"
        );
    }
}

//! Embedded asset bytes.
//!
//! Populated at compile time by `build.rs` from the crate-owned WebUI bundle
//! and committed public assets. Each file becomes one
//! `Asset` row keyed by its URL path (relative to the gateway root).
//! `index.html` is handled separately — see
//! [`INDEX_HTML_TEMPLATE`].

pub(crate) struct Asset {
    pub bytes: &'static [u8],
    pub content_type: &'static str,
    pub etag: &'static str,
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

    fn bundled_javascript() -> String {
        ASSETS
            .iter()
            // Route splitting moves feature code out of the app entry and into
            // named lazy chunks, so source-contract assertions must search the
            // complete embedded JavaScript graph.
            .filter(|(path, _)| path.ends_with(".js"))
            .map(|(_, asset)| std::str::from_utf8(asset.bytes).expect("asset is utf-8"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Read a frontend source file straight from `frontend/src/` on disk.
    /// Used for fixtures that are deliberately *not* embedded/served — e.g. Vitest
    /// unit tests — so a caller-level JS regression can still assert their
    /// content without shipping them to clients.
    fn source_text(path: &str) -> String {
        let full = format!("{}/frontend/src/{path}", env!("CARGO_MANIFEST_DIR"));
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
    fn durable_message_attachment_chips_are_wired_without_prose_inference() {
        // Assistant attachments are rendered only from the structured
        // `message.attachments` projection. A workspace-looking string in
        // prose must never mint semantic attachment state in the browser.
        let bubble = source_text("pages/chat/components/message-bubble.tsx");
        assert!(bubble.contains("attachments && attachments.length > 0"));
        assert!(bubble.contains("attachments.map"));
        assert!(bubble.contains("AttachmentChip"));
        assert!(!bubble.contains("ProjectFileChips"));
        assert!(!bubble.contains("extractWorkspaceFilePaths"));

        // The shared chip renders stable selector attributes and fetches only
        // an authorized URL supplied by the attachment projection.
        let chip = source_text("pages/chat/components/attachment-chip.tsx");
        assert!(chip.contains("export function AttachmentChip"));
        assert!(chip.contains("export function AttachmentThumbnail"));
        assert!(chip.contains("data-testid={testId}"));
        assert!(chip.contains("data-file-path={dataPath}"));
        // The inline download icon fetches the bearer-authenticated bytes and
        // saves them directly (separate from the preview modal's own Download).
        assert!(chip.contains("data-testid={downloadTestId}"));
        assert!(chip.contains("fetchAttachmentBlob"));
        assert!(chip.contains("saveBlob"));
        // The preview modal fetches the blob (bearer-authenticated, via the
        // shared `fetchAttachmentBlob`) and offers a Download action with a
        // stable test hook.
        let preview = source_text("pages/chat/components/attachment-preview.tsx");
        assert!(preview.contains("fetchAttachmentBlob"));
        assert!(preview.contains("data-testid=\"attachment-download\""));

        // History projection maps durable refs to the message-scoped,
        // bearer-authenticated attachment route.
        let history = source_text("pages/chat/lib/history-messages.ts");
        assert!(history.contains("record.attachments"));
        assert!(history.contains("attachmentUrl"));

        // The API client keeps the bearer-authenticated blob fetch; it does no
        // DOM (object URLs live in the preview modal / `download.js`).
        let api = source_text("lib/api.ts");
        assert!(api.contains("attachmentUrl"));
        assert!(api.contains("Authorization"));
        assert!(!api.contains("createObjectURL"));

        assert!(lookup("pages/chat/lib/history-messages.ts").is_none());
        assert!(lookup("pages/chat/lib/history-messages.test.ts").is_none());
    }

    #[test]
    fn chat_auth_gate_assets_submit_manual_token_then_resolve_gate() {
        let auth_card = source_text("pages/chat/components/auth-token-card.tsx");
        assert!(auth_card.contains("await onSubmit(value);"));
        assert!(auth_card.contains("setToken(\"\");"));
        assert!(auth_card.contains("t(\"authGate.submitFailed\")"));
        assert!(auth_card.contains("authGate.resolveFailedAfterTokenSaved"));
        assert!(!auth_card.contains("err?.message"));

        let api = source_text("lib/api.ts");
        assert!(api.contains("/api/reborn/product-auth/manual-token/submit"));
        assert!(api.contains("signal,"));
        assert!(api.contains("account_label: accountLabel"));
        assert!(api.contains("gate_ref: gateRef"));

        let use_chat = source_text("pages/chat/hooks/useChat.ts");
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
        let store = source_text("pages/chat/lib/draft-store.ts");
        assert!(store.contains("export function getStagedAttachments"));
        assert!(store.contains("export function setStagedAttachments"));
        assert!(store.contains("export function clearStagedAttachments"));
        // Sign-out drops the in-memory staged files too, so they can't resurface
        // for the next user on the same browser.
        assert!(store.contains("stagedAttachments.clear()"));

        let input = source_text("pages/chat/components/chat-input.tsx");
        // Initialized from the store (not a bare `[]`), persisted on change, and
        // cleared on a successful send.
        assert!(input.contains("getStagedAttachments(draftKey)"));
        assert!(input.contains("setStagedAttachments(draftKey"));
        assert!(input.contains("clearStagedAttachments(draftKey)"));
    }

    #[test]
    fn chat_gate_resolution_uses_resume_outcome() {
        let use_chat = source_text("pages/chat/hooks/useChat.ts");
        // Gate resolution normally resumes the run, but stale/terminal gate
        // responses must not synthesize a processing state.
        assert!(use_chat.contains("const outcome = resolveGateOutcome(response);"));
        assert!(use_chat.contains("if (outcome === \"resumed\")"));
        assert!(use_chat.contains("setIsProcessing(true);"));
        assert!(use_chat.contains("setIsProcessing(false);"));
        assert!(use_chat.contains("setActiveRun(null);"));
        assert!(!use_chat.contains("setIsProcessing(shouldContinueProcessing);"));

        let events = source_text("pages/chat/lib/useChatEvents.ts");
        assert!(events.contains("TERMINAL_RUN_STATUSES.has(status)"));
        assert!(events.contains("setPendingGate(null);"));
        assert!(events.contains("setActiveRun?.(null);"));
        assert!(events.contains("latestRunIdRef.current = null;"));
    }

    #[test]
    fn chat_pending_reconciliation_has_caller_level_js_regression() {
        let use_chat = source_text("pages/chat/hooks/useChat.ts");
        assert!(use_chat.contains("recordAcceptedMessageRef("));
        assert!(use_chat.contains("pendingMessagesRef.current"));
        assert!(use_chat.contains("response?.accepted_message_ref"));

        let pending_messages = source_text("pages/chat/lib/pending-messages.ts");
        assert!(pending_messages.contains("timelineMessageIdFromAcceptedRef"));
        assert!(
            pending_messages
                .contains("return ref.startsWith(\"msg:\") ? ref.slice(\"msg:\".length) : null;")
        );

        let regression = source_text("pages/chat/lib/useChat-send.test.ts");
        assert!(regression.contains("useChat.send: accepted ref reconciles"));
        assert!(regression.contains("accepted_message_ref: \"msg:message-1\""));
        assert!(regression.contains("await loadHistory();"));
        assert!(regression.contains("[\"msg-message-1\"]"));

        let pending_regression = source_text("pages/chat/lib/pending-messages.test.ts");
        assert!(pending_regression.contains(
            "recordAcceptedMessageRef: null and non-msg refs leave pending record unchanged"
        ));
        assert!(pending_regression.contains("\"thread:1\""));
        assert!(pending_regression.contains("\"message-1\""));
    }

    #[test]
    fn chat_send_keeps_thread_cache_import_wired() {
        // A main-merge conflict resolution once dropped this import while
        // keeping the call: every WebChat v2 send then failed in the browser
        // with `ReferenceError: touchThreadInCache is not defined`. The JS
        // unit harness injects stubs for unknown identifiers, so only a
        // source-level pin (and the Playwright smoke) catches a re-drop.
        let use_chat = source_text("pages/chat/hooks/useChat.ts");
        assert!(use_chat.contains("import { touchThreadInCache } from \"../lib/thread-cache\""));
        assert!(use_chat.contains("touchThreadInCache({"));
    }

    #[test]
    fn markdown_code_blocks_keep_horizontal_scroll_local_to_block() {
        let renderer = source_text("pages/chat/components/markdown-renderer.tsx");
        assert!(renderer.contains("wrap.className = \"markdown-code-frame\";"));
        assert!(renderer.contains("pre.style.overflowX = \"auto\";"));
        assert!(renderer.contains("pre.style.overflowY = \"hidden\";"));
        assert!(!renderer.contains("pre.style.overflow = \"hidden\";"));
        assert!(!renderer.contains("codeEl.style.whiteSpace"));

        let styles = source_text("styles/app.css");
        let markdown_styles = styles
            .split_once("/* ── Markdown body ")
            .and_then(|(_, markdown_and_rest)| {
                markdown_and_rest
                    .split_once("/* ── Mobile responsive overrides ")
                    .map(|(markdown, _)| markdown)
            })
            .expect("app.css keeps bounded Markdown styles");
        assert!(styles.contains(".markdown-body {\n  max-width: 100%;\n  min-width: 0;"));
        assert!(styles.contains("overflow-wrap: anywhere;"));
        assert!(styles.contains(".markdown-code-frame {\n  position: relative;"));
        assert!(styles.contains("width: 100%;\n  max-width: 100%;\n  min-width: 0;"));
        assert!(styles.contains("overflow: hidden;"));
        assert!(styles.contains("border-radius: 8px; box-sizing: border-box; width: 100%;"));
        assert!(styles.contains("overflow-x: auto; white-space: pre; margin-bottom: 0.75em;"));
        assert!(styles.contains("overflow-wrap: normal;\n  word-break: normal;"));
        assert!(styles.contains("display: inline; background: transparent; padding: 0;"));
        assert!(styles.contains("font-size: 0.9em; line-height: 1.65; white-space: inherit;"));
        assert!(!markdown_styles.contains("word-break: break-word"));
        assert!(!markdown_styles.contains("white-space: pre-wrap"));
        assert!(!markdown_styles.contains("word-break: break-all"));
        assert!(!markdown_styles.contains("width: max-content"));
        assert!(styles.contains("--v2-chat-readable-max-width:"));
        assert!(styles.contains(".v2-chat-readable-width {\n  max-width: 100%;\n}"));
        assert!(styles.contains("@media (min-width: 640px) {"));
        assert!(styles.contains("max-width: var(--v2-chat-readable-max-width);"));
        assert!(styles.contains("@media (max-width: 639.98px) {"));
        assert!(!styles.contains("@media (max-width: 768px)"));

        let message_list = source_text("pages/chat/components/message-list.tsx");
        assert!(message_list.contains("relative flex min-h-0 min-w-0 flex-1"));
        assert!(message_list.contains("flex min-w-0 flex-1 overflow-y-auto overflow-x-hidden"));
        assert!(message_list.contains("mx-auto flex w-full min-w-0 max-w-5xl flex-col"));

        let message_bubble = source_text("pages/chat/components/message-bubble.tsx");
        assert!(message_bubble.contains("group flex w-full min-w-0 flex-col"));
        assert!(message_bubble.contains("const bubbleWidthClass = isUser"));
        assert!(message_bubble.contains("const isNotice = role === CHAT_MESSAGE_ROLES.SYSTEM;"));
        assert!(message_bubble.contains("const isError = role === CHAT_MESSAGE_ROLES.ERROR;"));
        assert!(message_bubble.contains("\"v2-chat-readable-width\""));
        assert!(message_bubble.contains("\"mx-auto v2-chat-readable-width\""));
        assert!(message_bubble.contains("\"mr-auto v2-chat-readable-width\""));
        assert!(message_bubble.contains("\"w-full v2-chat-readable-width\""));
        assert!(!message_bubble.contains("sm:max-w-["));
        assert!(message_bubble.contains("isUser || isError ? \"min-w-0 max-w-full\""));
        assert!(message_bubble.contains("contentWidthClass,"));
    }

    #[test]
    fn chat_omits_connect_action_while_extensions_render_generic_connect_ui() {
        let chat = source_text("pages/chat/chat.tsx");
        assert!(!chat.contains("ChannelConnectCard"));
        assert!(!chat.contains("channelConnectAction"));
        assert!(!chat.contains("dismissChannelConnectAction"));

        let use_chat = source_text("pages/chat/hooks/useChat.ts");
        assert!(!use_chat.contains("resolveConnectAction"));
        assert!(!use_chat.contains("channel_connect_action"));

        // The Slack-specific frontend surface is deleted (extension-runtime
        // P6): no admin setup panel, no allowed-channel picker, no bespoke
        // setup/channel APIs.
        for deleted in [
            "components/slack-setup-panel.tsx",
            "components/slack-channel-picker.tsx",
            "lib/slack-setup-api.ts",
            "lib/slack-channels-api.ts",
        ] {
            let full = format!("{}/frontend/src/{deleted}", env!("CARGO_MANIFEST_DIR"));
            assert!(
                !std::path::Path::new(&full).exists(),
                "{deleted} is a deleted Slack-specific surface and must not return"
            );
        }

        // Connection presentation derives from the manifest strategy. The
        // channels tab remains a lifecycle list; Configure owns setup UI.
        let schema = source_text("pages/extensions/lib/extensions-schema.ts");
        assert!(
            schema.contains("isWebGeneratedCodeConnection"),
            "the shared surface taxonomy owns the connect-strategy vocabulary"
        );

        let channels_tab = source_text("pages/extensions/components/channels-tab.tsx");
        assert!(channels_tab.contains("ExtensionCard"));
        assert!(!channels_tab.contains("PairingSection"));
        assert!(!channels_tab.contains("redeemPairingCode"));
        assert!(!channels_tab.to_ascii_lowercase().contains("slack"));

        let configure_modal = source_text("pages/extensions/components/configure-modal.tsx");
        // Lane F split the expression into `const connection =
        // channelConnection(extension)` + `isWebGeneratedCodeConnection(connection)`;
        // the contract is unchanged: strategy derives from the manifest surface.
        assert!(
            configure_modal.contains("channelConnection(extension)"),
            "WebGeneratedCode routing derives directly from the manifest surface"
        );
        assert!(
            configure_modal.contains("isWebGeneratedCodeConnection(connection)"),
            "WebGeneratedCode routing derives directly from the manifest surface"
        );
        assert!(!configure_modal.contains("getExtensionPairingStatus"));
        assert!(!configure_modal.contains("redeemPairingCode"));
        assert!(
            configure_modal.contains("queryClient.invalidateQueries({ queryKey })"),
            "OAuth completion must refresh the server-owned lifecycle projection"
        );
        assert!(!configure_modal.contains("activateExtension"));
        assert!(!configure_modal.to_ascii_lowercase().contains("slack"));

        let regression = source_text("pages/chat/lib/useChat-send.test.ts");
        assert!(regression.contains("connect-like prompts submit to the model"));
        assert!(!regression.contains("channel connect requests return an action"));
    }

    #[test]
    fn channel_connection_gate_rides_the_auth_rail() {
        // Slack's bespoke pairing artifacts are gone (slack-pairing-api.js,
        // SlackPairingSection); channel pairing now rides the standard
        // auth-gate rail. The generic channel-connection machinery is retained
        // and generalized: channel-connection-events keeps its cross-tab connect
        // broadcast AND the waiter/continuation half that resumes a chat blocked
        // on a channel connected in another tab. This dual wiring is locked by
        // the useChat-send / configure-modal suites that PR #5604 authored.
        let events = source_text("lib/channel-connection-events.ts");
        assert!(events.contains("notifyChannelConnected"));
        assert!(events.contains("subscribeChannelConnected"));
        assert!(events.contains("BroadcastChannel"));
        assert!(events.contains("channelConnectionContinuationMessage"));
        assert!(events.contains("rememberChannelConnectionWaiter"));
        assert!(events.contains("resumeWaitingChannelConnections"));
        assert!(events.contains("ironclaw:channel-connection:waiting:v1"));

        // gates.ts recognizes the challenge kinds (manual_token / oauth_url /
        // pairing)
        // that ALL auth gates use, and normalizes the optional channel
        // `connection` context onto the pending gate.
        let gates = source_text("pages/chat/lib/gates.ts");
        assert!(gates.contains("manual_token"));
        assert!(gates.contains("oauth_url"));
        assert!(gates.contains("pairing"));
        assert!(gates.contains("connectionFromContext"));

        // The channel-connection/onboarding state machine observes generic
        // connection completion and resumes the originating chat. Codes minted
        // by the host are completed provider-side; the browser has no pasted
        // proof redemption route.
        let use_chat = source_text("pages/chat/hooks/useChat.ts");
        assert!(use_chat.contains("useChannelOnboarding(threadId, {"));
        assert!(!use_chat.contains("submitOnboardingPairing"));
        assert!(!use_chat.contains("submitChannelConnectionPairing"));
        assert!(!use_chat.contains("Slack is connected. Continue the previous request."));

        let use_channel_onboarding = source_text("pages/chat/hooks/useChannelOnboarding.ts");
        assert!(use_channel_onboarding.contains("subscribeChannelConnected"));
        assert!(!use_channel_onboarding.contains("submitOnboardingPairing"));
        assert!(!use_channel_onboarding.contains("submitChannelConnectionPairing"));
        assert!(!use_channel_onboarding.contains("redeemPairingCode"));
        assert!(use_channel_onboarding.contains("connectionEventMatchesOnboarding"));
        // OAuth completion/failure matching goes through the shared flow-id
        // matchers, not a hand-rolled comparison that could drift on the
        // payload contract (type guard, snake_case flow_id fallback).
        assert!(use_channel_onboarding.contains("completionMatchesFlow"));
        assert!(use_channel_onboarding.contains("failureMatchesFlow"));
        assert!(use_channel_onboarding.contains("rememberChannelConnectionWaiter"));
        assert!(use_channel_onboarding.contains("forgetChannelConnectionWaiter"));
        assert!(use_channel_onboarding.contains("channelConnectionRequirementFromCard"));
        assert!(use_channel_onboarding.contains("pendingOnboarding"));

        // chat.ts renders the pairing card off a pairing gate carrying a
        // connection, on the same auth-gate switch as the token / oauth cards.
        // Lane F (eebd239be) centralized the pairing-gate predicate in
        // gates.ts::channelConnectionFromGate so the composer affordance and
        // the card selector cannot disagree; the chat surface consumes the
        // derived `channelConnectionGate` instead of re-reading raw gate
        // fields.
        let chat = source_text("pages/chat/chat.tsx");
        assert!(chat.contains("OnboardingPairingCard"));
        assert!(chat.contains("channelConnectionFromGate(pendingGate)"));
        assert!(chat.contains("channelConnectionGate"));
        assert!(!chat.contains("submitChannelConnectionPairing"));
        assert!(
            gates.contains("channelConnectionFromGate"),
            "the shared predicate owns pairing-gate recognition"
        );

        for deleted in [
            "pages/extensions/lib/pairing-api.ts",
            "pages/extensions/components/pairing-section.tsx",
        ] {
            let full = format!("{}/frontend/src/{deleted}", env!("CARGO_MANIFEST_DIR"));
            assert!(
                !std::path::Path::new(&full).exists(),
                "{deleted} was unsupported scaffolding and must not return"
            );
        }
    }

    #[test]
    fn automations_panel_assets_are_embedded() {
        let app = source_text("app/app.tsx");
        assert!(app.contains("AutomationsPage"));
        assert!(app.contains("path=\"automations\""));

        let routes = source_text("app/routes.ts");
        assert!(routes.contains("nav.automations"));
        assert!(routes.contains("path: \"/automations\""));

        let api = source_text("lib/api.ts");
        assert!(api.contains("listAutomations"));
        assert!(api.contains("pauseAutomation"));
        assert!(api.contains("resumeAutomation"));
        assert!(api.contains("runAutomation"));
        assert!(api.contains("deleteAutomation"));
        assert!(api.contains("/automations"));
        assert!(api.contains("/run"));
        assert!(api.contains("/pause"));
        assert!(api.contains("/resume"));
        assert!(api.contains(r#"method: "DELETE""#));
        assert!(api.contains("listOutboundDeliveryTargets"));
        assert!(api.contains("/outbound/targets"));
        assert!(api.contains("getNotificationChannels"));
        assert!(api.contains("setNotificationChannels"));
        assert!(api.contains("/outbound/notification-channels"));

        let page = source_text("pages/automations/automations-page.tsx");
        assert!(page.contains("AutomationsSummaryStrip"));
        assert!(page.contains("NotificationChannelsPanel"));
        assert!(page.contains("useNotificationChannels"));
        assert!(page.contains("AutomationsList"));

        let automations_hook = source_text("pages/automations/hooks/useAutomations.ts");
        assert!(automations_hook.contains("AUTOMATIONS_BASE_REFETCH_MS"));
        assert!(automations_hook.contains("nextAutomationsRefetchDelay"));
        assert!(automations_hook.contains("query.refetch()"));

        let list = source_text("pages/automations/components/automations-list.tsx");
        assert!(list.contains("primary_status_label"));
        assert!(list.contains("primary_status_tone"));

        let detail_panel = source_text("pages/automations/components/automation-detail-panel.tsx");
        assert!(detail_panel.contains("onPauseAutomation"));
        assert!(detail_panel.contains("onResumeAutomation"));
        assert!(detail_panel.contains("onDeleteAutomation"));
        assert!(detail_panel.contains("automation.state === \"active\""));
        assert!(detail_panel.contains("automation.state === \"scheduled\""));
        assert!(detail_panel.contains("primary_status_label"));
        assert!(detail_panel.contains("primary_status_tone"));
        assert!(detail_panel.contains("import { ConfirmDialog }"));
        assert!(detail_panel.contains("<ConfirmDialog"));
        assert!(!detail_panel.contains("window.confirm"));

        let app_bundle = bundled_javascript();
        let app_bundle_contains_encoded_automation_route = |suffix: &str| {
            app_bundle
                .split("/automations/${encodeURIComponent(")
                .any(|tail| tail.contains(&format!(")}}/{suffix}")))
        };
        assert!(
            app_bundle_contains_encoded_automation_route("pause"),
            "served WebUI bundle must include the automation pause endpoint; run the frontend build after editing frontend/src/**"
        );
        assert!(
            app_bundle_contains_encoded_automation_route("resume"),
            "served WebUI bundle must include the automation resume endpoint; run the frontend build after editing frontend/src/**"
        );
        assert!(
            app_bundle_contains_encoded_automation_route("run"),
            "served WebUI bundle must include the automation run endpoint; run the frontend build after editing frontend/src/**"
        );
        let app_bundle_contains_encoded_automation_delete = app_bundle
            .split("/automations/${encodeURIComponent(")
            .any(|tail| {
                let near = tail.chars().take(220).collect::<String>();
                near.contains("DELETE") && !near.contains("/pause") && !near.contains("/resume")
            });
        assert!(
            app_bundle_contains_encoded_automation_delete,
            "served WebUI bundle must include the automation delete endpoint; run the frontend build after editing frontend/src/**"
        );

        let channels_panel =
            source_text("pages/automations/components/notification-channels-panel.tsx");
        assert!(channels_panel.contains(r#"type="checkbox""#));
        assert!(channels_panel.contains("saveNotificationChannels"));
        assert!(channels_panel.contains("automations.notificationChannels.noSelectionHelper"));
        // Badge label must branch on row status — an unavailable (stored but
        // no-longer-resolving) channel must not display the "ready" label.
        assert!(
            channels_panel.contains("automations.notificationChannels.pill.unavailable"),
            "unavailable badge label key must be used in the channel rows"
        );
        assert!(
            !channels_panel.contains(r#"label={t("automations.notificationChannels.pill.ready")}"#),
            "channel row badge label must not be unconditionally hardcoded to .pill.ready"
        );

        let channels_hook = source_text("pages/automations/hooks/useNotificationChannels.ts");
        assert!(channels_hook.contains("listOutboundDeliveryTargets"));
        assert!(channels_hook.contains("getNotificationChannels"));
        assert!(channels_hook.contains("setNotificationChannels"));
        // A stored id absent from the live catalog must still produce a row
        // (Task 8's `RebornNotificationChannel`: unavailable, not dropped).
        assert!(channels_hook.contains("orphanRows"));

        let presenter = source_text("pages/automations/lib/automations-presenters.ts");
        assert!(presenter.contains("source?.type === \"schedule\""));
        assert!(presenter.contains("Custom schedule"));
        assert!(!presenter.contains("Webhook"));

        for deleted in [
            "pages/automations/components/automation-delivery-defaults-panel.tsx",
            "pages/automations/hooks/useOutboundDeliveryDefaults.ts",
        ] {
            let full = format!("{}/frontend/src/{deleted}", env!("CARGO_MANIFEST_DIR"));
            assert!(
                !std::path::Path::new(&full).exists(),
                "{deleted} was renamed to the notification-channels panel/hook and must not return"
            );
        }
    }

    #[test]
    fn sidebar_new_chat_label_owns_typography() {
        let sidebar_nav = source_text("components/sidebar-nav.tsx");

        assert!(sidebar_nav.contains("<span className=\"text-[13px] font-medium\""));
        assert!(sidebar_nav.contains("t(\"chat.newThread\")"));
    }

    #[test]
    fn sidebar_thread_delete_assets_are_wired() {
        let sidebar = source_text("components/sidebar.tsx");
        assert!(sidebar.contains("onDeleteThread"));
        assert!(sidebar.contains("onDelete={onDeleteThread}"));

        let sidebar_threads = source_text("components/sidebar-threads.tsx");
        assert!(sidebar_threads.contains("data-testid=\"thread-delete\""));
        assert!(sidebar_threads.contains("data-thread-id={thread.id}"));
        assert!(sidebar_threads.contains("t(\"common.deleteChat\")"));
        assert!(sidebar_threads.contains("t(\"thread.deleteConfirm\")"));
        assert!(sidebar_threads.contains("deleteThreadErrorMessage"));
        assert!(
            sidebar_threads
                .contains(r#"toast(deleteThreadErrorMessage(error, t), { tone: "error" });"#)
        );
        assert!(!sidebar_threads.contains("window.alert"));

        let api = source_text("lib/api.ts");
        assert!(api.contains("export function deleteThread"));
        assert!(api.contains("/threads/${encodeURIComponent(threadId)}"));
        assert!(api.contains("method: \"DELETE\""));

        let bundle = bundled_javascript();
        assert!(bundle.contains("thread-delete"));
        assert!(bundle.contains("common.deleteChat"));
        assert!(bundle.contains("thread.deleteConfirm"));
    }

    #[test]
    fn desktop_sidebar_toggle_assets_are_wired() {
        let header = source_text("components/page-header.tsx");
        assert!(header.contains("type=\"button\""));
        assert!(header.contains(r#"const toggleSidebarLabel = t("sidebar.toggle")"#));
        assert!(header.contains("aria-label={toggleSidebarLabel}"));
        assert!(header.contains("aria-controls=\"gateway-sidebar\""));
        assert!(header.contains("aria-expanded={sidebarOpen ? \"true\" : \"false\"}"));
        assert!(header.contains("title={toggleSidebarLabel}"));
        assert!(!header.contains("md:hidden"));

        let sidebar = source_text("components/sidebar.tsx");
        assert!(sidebar.contains("id={id}"));

        let sidebar_state = source_text("lib/sidebar-state.ts");
        assert!(sidebar_state.contains("ironclaw:v2-sidebar-open"));
        assert!(sidebar_state.contains("export function readDesktopSidebarOpen"));
        assert!(sidebar_state.contains("export function writeDesktopSidebarOpen"));
        assert!(sidebar_state.contains("matchMedia?.(\"(min-width: 768px)\")"));
        assert!(sidebar_state.contains("export function currentSidebarOpen"));

        let hook = source_text("hooks/useSidebar.ts");
        assert!(hook.contains("from \"../lib/sidebar-state\""));
        assert!(hook.contains("desktopOpen: readDesktopSidebarOpen()"));
        assert!(hook.contains("React.useState(() =>"));
        assert!(hook.contains("isDesktopSidebarViewport()"));
        assert!(hook.contains("setIsDesktopViewport(query.matches)"));
        assert!(hook.contains("toggleSidebarState(current, isDesktopViewport)"));
        assert!(hook.contains("currentOpen: currentSidebarOpen(state, isDesktopViewport)"));

        let layout = source_text("layout/gateway-layout.tsx");
        assert!(layout.contains("{sidebar.mobileOpen &&"));
        assert!(layout.contains("sidebar.mobileOpen ? \"flex\" : \"hidden\""));
        assert!(layout.contains("sidebar.desktopOpen ? \"md:flex\" : \"md:hidden\""));
        assert!(layout.contains("sidebarOpen={sidebar.currentOpen}"));

        let bundle = bundled_javascript();
        assert!(bundle.contains("ironclaw:v2-sidebar-open"));
        assert!(bundle.contains("desktopOpen"));
        assert!(bundle.contains("mobileOpen"));
    }

    #[test]
    fn sidebar_trace_credits_card_assets_are_embedded() {
        // The compact card is mounted in the sidebar above the conversation
        // list and reuses the existing trace-credits hook + endpoint.
        let card = source_text("components/sidebar-trace-credits.tsx");
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

        let sidebar = source_text("components/sidebar.tsx");
        assert!(sidebar.contains("SidebarTraceCredits"));
        // Mounted between the nav and the threads list.
        assert!(sidebar.contains("<SidebarTraceCredits />"));

        // The Settings tab lists held traces (reason + submission id) sourced
        // from the credits response `holds[]`.
        let tab = source_text("pages/settings/components/trace-commons-tab.tsx");
        assert!(tab.contains("const holds = credits.holds || [];"));
        assert!(tab.contains("traceCommons.heldTitle"));
        assert!(tab.contains("traceCommons.heldDescription"));
        assert!(tab.contains("hold.submission_id"));
        assert!(tab.contains("hold.reason"));
        // Per-hold Authorize button wired to the authorize mutation.
        assert!(tab.contains("authorize.mutate(hold.submission_id)"));
        assert!(tab.contains("traceCommons.authorize"));

        // Authorize calls the POST endpoint and invalidates the credits query.
        let settings_api = source_text("pages/settings/lib/settings-api.ts");
        assert!(settings_api.contains("export function authorizeTraceHold"));
        assert!(settings_api.contains("/authorize"));
        assert!(settings_api.contains("method: \"POST\""));
        let trace_hook = source_text("pages/settings/hooks/useTraceCredits.ts");
        assert!(trace_hook.contains("authorizeTraceHold"));
        assert!(trace_hook.contains("invalidateQueries({ queryKey: [\"trace-credits\"] })"));

        // The hook refetches so the card and Settings tab reflect new
        // accepted submissions without a manual reload. Polling is infrequent
        // (300s) and paused while the tab is hidden; a focus refetch keeps the
        // surface live and staleTime dedupes redundant focus refetches.
        let hook = source_text("pages/settings/hooks/useTraceCredits.ts");
        assert!(hook.contains("refetchInterval: 300_000"));
        assert!(hook.contains("refetchIntervalInBackground: false"));
        assert!(hook.contains("refetchOnWindowFocus: true"));
        assert!(hook.contains("staleTime: 60_000"));

        // The new i18n keys are present in the eagerly-bundled English pack
        // (other locales fall back to it if missing, but all 11 carry them).
        let en = source_text("i18n/en.ts");
        assert!(en.contains("\"traceCommons.cardAccepted\""));
        assert!(en.contains("\"traceCommons.cardHeld\""));
        assert!(en.contains("\"traceCommons.heldTitle\""));
        assert!(en.contains("\"traceCommons.heldDescription\""));
        assert!(en.contains("\"traceCommons.authorize\""));
        assert!(en.contains("\"traceCommons.authorizing\""));
    }

    #[test]
    fn settings_and_extensions_navigation_has_one_source_of_truth() {
        // Settings and Extensions sub-navigation is declared once, in
        // `app/routes.ts`; the sidebar and the page header read it from there.
        // The per-page tab components kept a second copy of the same lists with
        // no production consumer, and the Settings copy had already drifted —
        // it still advertised agent/channels/networking/users entries that are
        // not routed.
        for deleted in [
            "pages/settings/components/settings-tabs.tsx",
            "pages/settings/components/settings-tabs.test.ts",
            "pages/extensions/components/extensions-tabs.tsx",
        ] {
            let full = format!("{}/frontend/src/{deleted}", env!("CARGO_MANIFEST_DIR"));
            assert!(
                !std::path::Path::new(&full).exists(),
                "{deleted} duplicated app/routes.ts navigation and must not return"
            );
        }

        let settings_schema = source_text("pages/settings/lib/settings-schema.ts");
        let extensions_schema = source_text("pages/extensions/lib/extensions-schema.ts");
        assert!(
            !settings_schema.contains("SETTINGS_TABS"),
            "Settings navigation belongs to app/routes.ts, not the field schema"
        );
        assert!(
            !extensions_schema.contains("EXTENSIONS_TABS"),
            "Extensions navigation belongs to app/routes.ts, not the surface schema"
        );

        // Unrelated field schemas and extension presentation metadata stay put.
        assert!(settings_schema.contains("export const AGENT_FIELDS"));
        assert!(settings_schema.contains("export const NETWORKING_FIELDS"));
        assert!(settings_schema.contains("export const RESTART_REQUIRED_KEYS"));
        assert!(extensions_schema.contains("export const RUNTIME_LABELS"));
        assert!(extensions_schema.contains("export function hasChannelSurface"));

        // The remaining source of truth still feeds both live consumers.
        let routes = source_text("app/routes.ts");
        assert!(routes.contains("export const SETTINGS_SUB_ROUTES"));
        assert!(routes.contains("export const EXTENSIONS_SUB_ROUTES"));
        assert!(routes.contains("settings: SETTINGS_SUB_ROUTES"));
        assert!(routes.contains("extensions: EXTENSIONS_SUB_ROUTES"));
        assert!(source_text("components/sidebar-nav.tsx").contains("EXPANDABLE_SUB_ROUTES"));
        assert!(source_text("components/page-header.tsx").contains("EXPANDABLE_SUB_ROUTES"));
    }

    #[test]
    fn auth_session_assets_use_server_capabilities_for_admin_status() {
        let api = source_text("lib/api.ts");
        assert!(api.contains("fetchSession"));
        assert!(api.contains("/session"));

        let auth = source_text("app/auth.ts");
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

        let sidebar_nav = source_text("components/sidebar-nav.tsx");
        assert!(sidebar_nav.contains("isAdmin = false"));
        assert!(sidebar_nav.contains("export function visibleSidebarSubRoutes"));
        assert!(sidebar_nav.contains("!(routeId === \"settings\" && subRoute.id === \"users\")"));
        assert!(sidebar_nav.contains("visibleSidebarSubRoutes(route.id, isAdmin)"));
        assert!(!sidebar_nav.contains("[\"users\", \"inference\"].includes(subRoute.id)"));

        let routes = source_text("app/routes.ts");
        assert!(
            routes
                .contains(r#"{ id: "inference", labelKey: "settings.inference", icon: "spark" }"#)
        );

        let settings_page = source_text("pages/settings/settings-page.tsx");
        assert!(settings_page.contains("isAdmin = false"));
        assert!(
            settings_page.contains("const defaultTab = isAdmin ? \"inference\" : \"language\"")
        );
        assert!(settings_page.contains("const defaultTabIsVisible = tabContentHas(defaultTab)"));
        assert!(settings_page.contains("const redirectTab = defaultTabIsVisible"));
        assert!(!settings_page.contains("isOperatorTab(tab)"));

        let layout = source_text("layout/gateway-layout.tsx");
        assert!(layout.contains("enabled: isAdmin"));
        assert!(layout.contains("const needsOnboarding ="));
        assert!(layout.contains("isAdmin &&"));
        assert!(layout.contains("shouldRouteToOnboarding({"));

        let app = source_text("app/app.tsx");
        assert!(app.contains("isChecking={auth.isChecking}"));

        let providers = source_text("pages/settings/hooks/useLlmProviders.ts");
        assert!(providers.contains("const hasActiveProvider = Boolean("));
        assert!(!providers.contains("!enabled || Boolean"));

        let onboarding = source_text("pages/onboarding/onboarding-page.tsx");
        assert!(onboarding.contains("isChecking = false"));
        assert!(onboarding.contains("if (isChecking) return null;"));
        assert!(onboarding.contains("if (!isAdmin)"));
        assert!(onboarding.contains("OperatorOnboardingPage"));
    }

    #[test]
    fn chat_projection_text_preserves_pending_gate() {
        let events = source_text("pages/chat/lib/useChatEvents.ts");
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
        let groups = source_text("pages/chat/lib/message-groups.ts");
        assert!(groups.contains("function isFinalAssistantReply"));
        assert!(groups.contains("msg.isFinalReply === true"));
        assert!(groups.contains("msg.status === \"finalized\""));
        assert!(groups.contains("function followingActivity"));
        assert!(groups.contains("type: \"activity-run\""));
        assert!(groups.contains("appendActivityRun(items, activity);"));
        assert!(!groups.contains("lastAssistantReplyIndex"));

        let history = source_text("pages/chat/lib/history-messages.ts");
        assert!(history.contains("isFinalReply: isFinalAssistantRecord(record)"));
        assert!(history.contains("record.status === \"finalized\""));

        let events = source_text("pages/chat/lib/useChatEvents.ts");
        assert!(events.contains("isFinalReply: true"));
        // The in-flight marker is evidence-driven now: projection text is
        // final only when the durable projection says so, so the pin asserts
        // the derivation instead of a hardcoded `false`.
        assert!(
            events.contains("const finalizedText = item.text.finalized === true;"),
            "projection-text finality must be driven by the durable finalized bit"
        );
        assert!(
            events.contains("isFinalReply: finalizedText"),
            "live projection text must remain in-flight until final reply/timeline finalizes it"
        );
        assert!(events.contains("isStreaming: !finalizedText"));
        assert!(events.contains("const textRunId = item.text.run_id || null;"));
    }

    #[test]
    fn extensions_onboarding_messages_render_in_cards() {
        let extension_card = source_text("pages/extensions/components/extension-card.tsx");

        assert!(
            extension_card.contains("const setupState = state === \"setup_needed\";"),
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
            extension_card.contains("{onboardingHint}"),
            "extension cards must render the projected onboarding hint"
        );
    }

    #[test]
    fn extension_configuration_uses_only_the_public_lifecycle_states() {
        let extension_actions = source_text("pages/extensions/lib/extension-actions.ts");
        assert!(
            extension_actions.contains("const installationState = ext?.installation_state;"),
            "the caller-visible lifecycle must use only installation_state"
        );
        assert!(extension_actions.contains("export function extensionIsActive"));
        assert!(
            extension_actions.contains("return state === \"active\";"),
            "active readiness must derive from the public lifecycle state"
        );
        assert!(!extension_actions.contains("setupReadyForActivation"));
        assert!(!extension_actions.contains("state === \"ready\""));

        let extension_card = source_text("pages/extensions/components/extension-card.tsx");
        assert!(extension_card.contains("const state = extensionLifecycleState(ext);"));
        for retired in ["ext.active", "ext.needs_setup", "ext.onboarding_state"] {
            assert!(
                !extension_card.contains(retired),
                "extension cards must not consume retired lifecycle field {retired}"
            );
        }

        let channel_onboarding = source_text("pages/chat/hooks/useChannelOnboarding.ts");
        assert!(
            channel_onboarding.contains("extension?.installation_state === \"active\""),
            "chat connection-card suppression must use the caller-scoped lifecycle projection"
        );
        for retired in [
            "extension.authenticated",
            "extension.needs_setup",
            "extension.onboarding_state",
        ] {
            assert!(
                !channel_onboarding.contains(retired),
                "chat connection handling must not consume retired lifecycle field {retired}"
            );
        }

        let settings_channels = source_text("pages/settings/components/channels-tab.tsx");
        assert!(settings_channels.contains("channel?.installation_state || \"setup_needed\""));
        assert!(!settings_channels.contains("channel?.onboarding_state"));

        let configure_modal = source_text("pages/extensions/components/configure-modal.tsx");
        assert!(configure_modal.contains("const isActive = extensionIsActive(extension);"));
        assert!(!configure_modal.contains("const canActivate ="));
        assert!(!configure_modal.contains("setupReadyForActivation"));
        assert!(!configure_modal.contains("extensions.activate"));
        assert!(configure_modal.contains("extensions.activeConfigured"));

        let regression = source_text("pages/extensions/lib/extension-actions.test.ts");
        assert!(
            regression.contains("extensionIsActive requires authoritative active state"),
            "the authoritative caller-visible lifecycle predicate needs JS regression coverage"
        );
        assert!(
            regression
                .contains("extensionIsActive({ installation_state: \"setup_needed\" }), false")
        );
    }

    #[test]
    fn extension_oauth_setup_refreshes_while_popup_is_open() {
        let use_extensions = source_text("pages/extensions/hooks/useExtensions.ts");

        let oauth_events = source_text("lib/product-auth-oauth-events.ts");

        assert!(
            oauth_events.contains("OAUTH_FLOW_POLL_MS = 2000"),
            "OAuth setup should poll often enough for setup-complete state to appear promptly"
        );
        assert!(
            use_extensions.contains("OAUTH_FLOW_POLL_MS"),
            "OAuth setup should poll on the shared product-auth OAuth flow cadence"
        );
        assert!(
            use_extensions.contains("const watchOauthProgress = React.useCallback"),
            "OAuth setup should watch in-flight authorization, not only popup close"
        );
        // The in-flight watcher polls on an interval; within that poll it must
        // refresh setup state (so a setup-complete callback lands promptly)
        // BEFORE it considers giving up because the popup was closed. Assert the
        // ordering structurally rather than by exact whitespace so a reformat
        // doesn't false-flag while a real reorder still would.
        let poll_body_start = use_extensions
            .find("browserWindow.setInterval(")
            .expect("OAuth setup should poll on an interval while the popup is open");
        let poll_body = &use_extensions[poll_body_start..];
        let refresh_idx = poll_body
            .find("refreshSetupState();")
            .expect("OAuth setup poll must refresh setup state");
        let popup_close_idx = poll_body
            .find("popup && popup.closed")
            .expect("OAuth setup poll must still handle the popup closing");
        assert!(
            refresh_idx < popup_close_idx,
            "OAuth setup must refresh setup state before waiting for popup close"
        );
    }

    #[test]
    fn extension_registry_keeps_installed_entries_visible_first() {
        let use_extensions = source_text("pages/extensions/hooks/useExtensions.ts");
        let registry_tab = source_text("pages/extensions/components/registry-tab.tsx");
        let extensions_page = source_text("pages/extensions/extensions-page.tsx");
        let routes = source_text("app/routes.ts");
        let schema = source_text("pages/extensions/lib/extensions-schema.ts");

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
            registry_tab.contains("<ExtensionCard") && registry_tab.contains("<RegistryCard"),
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

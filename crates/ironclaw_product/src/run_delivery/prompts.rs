//! Generic user-facing copy and prompt text for channel run delivery.
//!
//! Every string here is channel-neutral English; the channel adapter owns
//! only formatting (markdown → vendor markup) and splitting. Prompt bodies
//! are authored in markdown (`**bold**`, backticks) so adapters can render
//! them into their native markup.

use crate::{ApprovalPromptContextView, AuthPromptChallengeKind, AuthPromptView, GatePromptView};
use ironclaw_outbound::RunNotificationEventKind;
use ironclaw_turns::{GateRef, TurnRunId};

use crate::is_approval_gate_ref;

pub(crate) const WORKING_MESSAGE: &str = "Ironclaw is thinking...";
pub(crate) const AUTH_CANCELED_MESSAGE: &str = "Authentication canceled.";
/// Posted when a run has no channel-serviceable auth challenge. This stays
/// deliberately generic because missing/unknown challenge metadata cannot
/// safely imply a credential type or setup recipe.
pub(crate) const AUTH_UNAVAILABLE_MESSAGE: &str = "This authentication step can't be completed in chat. Open the Ironclaw web app to review it, then ask me again here.";
/// Posted for a typed credential-entry challenge. It explicitly redirects
/// secret entry to the private WebUI surface without echoing prompt material.
pub(crate) const MANUAL_TOKEN_AUTH_UNAVAILABLE_MESSAGE: &str = "Setting this up needs a credential (an API key or token). Sharing one here is a security risk — anything entered in chat is stored in the conversation — so credential-based connections can only be set up in the Ironclaw web app. Connect it there, then ask me again here.";
/// Posted when a pairing challenge reaches a non-private target: the pairing
/// code itself is a bearer secret and must not be echoed into a shared thread.
pub(crate) const PAIRING_PRIVATE_SETUP_MESSAGE: &str = "Open the Ironclaw web app to connect or pair this extension in a private setup surface, then ask me again here.";
/// Posted when an OAuth challenge reaches a non-private target: the setup link
/// is single-use and must not be echoed into a shared thread.
pub(crate) const OAUTH_PRIVATE_SETUP_MESSAGE: &str = "Open the Ironclaw web app to complete this private authorization step, then ask me again here.";
pub(crate) const DELIVERY_TIMEOUT_MESSAGE: &str =
    "This is taking longer than expected — check the WebUI for the result.";
pub(crate) const DELIVERY_ERROR_MESSAGE: &str =
    "Something went wrong delivering the result here. Check the WebUI.";
/// Posted when the blocking run is `BlockedApproval` and no gate_ref is
/// available.
pub(crate) const BUSY_APPROVAL_MESSAGE: &str = "Ironclaw is waiting on a pending approval before taking new messages — reply `approve` or `deny` (or `approve gate:<ref>`) to resume.";
/// Posted for any other non-terminal blocking state, or when the state
/// lookup fails.
pub(crate) const BUSY_GENERIC_MESSAGE: &str = "Ironclaw is still working on a previous message and can't take this one yet — please resend it once the current task finishes.";

/// Stable per-(run, kind) projection id for run-notification deliveries.
pub(crate) fn run_notification_projection_id(
    run_id: TurnRunId,
    event_kind: RunNotificationEventKind,
) -> String {
    let suffix = match event_kind {
        RunNotificationEventKind::FinalReplyReady => "final",
        RunNotificationEventKind::ProgressUpdate => "progress",
        RunNotificationEventKind::ApprovalNeeded => "approval",
        RunNotificationEventKind::AuthRequired => "auth",
        RunNotificationEventKind::RunBlocked => "blocked",
        RunNotificationEventKind::DeliveryStatus => "delivery-status",
    };
    format!("run-notification:{suffix}:{run_id}")
}

/// Build the approval-gate prompt view. The body carries only the semantic
/// *What/Why* of the gate; the channel-agnostic *how to reply* is appended
/// once by [`gate_prompt_text`].
pub(crate) fn approval_gate_prompt_view(
    run_id: TurnRunId,
    gate_ref: &GateRef,
    context: Option<&ApprovalPromptContextView>,
) -> GatePromptView {
    let gate_ref_str = gate_ref.as_str();
    let body = match context {
        Some(ctx) => {
            let mut body = format!("**What:** {}", ctx.tool_name);
            if let Some(reason) = ctx.reason.as_deref() {
                body.push_str(&format!("\n**Why:** {reason}"));
            }
            body
        }
        None => "A step in this workflow needs your approval to continue.".to_string(),
    };

    GatePromptView {
        turn_run_id: run_id,
        gate_ref: gate_ref_str.to_string(),
        invocation_id: None,
        headline: "Approval needed".to_string(),
        body,
        allow_always: is_approval_gate_ref(gate_ref_str),
        approval_context: context.cloned(),
    }
}

/// Render a gate prompt into its channel-neutral message text.
pub(crate) fn gate_prompt_text(view: &GatePromptView, direct_message: bool) -> String {
    format!(
        "{}\n\n{}\n\n{}",
        view.headline,
        view.body,
        gate_prompt_reply_instruction(direct_message, &view.gate_ref)
    )
}

fn gate_prompt_reply_instruction(direct_message: bool, gate_ref: &str) -> String {
    if direct_message {
        format!(
            "Reply `approve` or `deny` in this chat to respond to this request. If several \
             approvals are pending here, use `approve {gate_ref}` or `deny {gate_ref}`."
        )
    } else {
        format!(
            "Reply by mentioning me with `approve` or `deny` in this thread. If several \
             approvals are pending here, use `approve {gate_ref}` or `deny {gate_ref}`."
        )
    }
}

/// Render an auth prompt into its channel-neutral message text. The
/// `authorization_url`, when present, is appended as a trailing setup link;
/// callers strip it BEFORE this point for non-private targets.
pub(crate) fn auth_prompt_text(view: &AuthPromptView, direct_message: bool) -> String {
    // Delegate to the canonical channel renderer. It is a strict superset of
    // the old local formatting -- headline, body, cancel affordance and the
    // OAuth `Setup link` -- and additionally emits the pairing code, deep link
    // and expiry. Rendering locally silently dropped the code, so a pairing
    // challenge told the user to "send the displayed code" without ever
    // displaying one.
    ironclaw_host_api::render_channel_auth_prompt(view, direct_message)
}

/// The body to render for a serviceable challenge. A pairing challenge
/// carries its own host-issued instructions; a manual-token challenge is
/// never serviceable in chat and gets the redirect copy.
pub(crate) fn actionable_auth_prompt_body(view: &AuthPromptView) -> String {
    match view.challenge_kind {
        Some(AuthPromptChallengeKind::ManualToken) => {
            MANUAL_TOKEN_AUTH_UNAVAILABLE_MESSAGE.to_string()
        }
        Some(AuthPromptChallengeKind::Pairing) => view
            .pairing
            .as_ref()
            .map(|pairing| pairing.instructions.clone())
            .unwrap_or_else(|| PAIRING_PRIVATE_SETUP_MESSAGE.to_string()),
        Some(AuthPromptChallengeKind::Other) => AUTH_UNAVAILABLE_MESSAGE.to_string(),
        Some(AuthPromptChallengeKind::OAuthUrl) | None => view.body.clone(),
    }
}

/// Can this challenge actually be completed from a chat surface?
///
/// Only two kinds can: an OAuth challenge carrying a usable authorization URL
/// (the user authenticates on the provider's site and the callback stores the
/// credential server-side), and a pairing challenge carrying a complete
/// host-issued code. A manual-token challenge never can -- pasting a secret
/// into chat stores it in the conversation. Anything unknown fails closed.
pub(crate) fn auth_prompt_is_serviceable(view: &AuthPromptView) -> bool {
    match view.challenge_kind {
        Some(AuthPromptChallengeKind::OAuthUrl) => view
            .authorization_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty()),
        Some(AuthPromptChallengeKind::Pairing) => view.pairing.as_ref().is_some_and(|pairing| {
            !pairing.channel.trim().is_empty()
                && !pairing.display_name.trim().is_empty()
                && !pairing.instructions.trim().is_empty()
                && !pairing.code.trim().is_empty()
        }),
        Some(AuthPromptChallengeKind::ManualToken | AuthPromptChallengeKind::Other) => false,
        // Compatibility for prompt rows created before challenge_kind became
        // part of the additive wire contract.
        None => view
            .authorization_url
            .as_deref()
            .is_some_and(|url| !url.trim().is_empty()),
    }
}

/// The notice for a challenge that cannot be serviced here. Only a typed
/// credential-entry challenge earns the "this needs an API key" copy --
/// telling a pairing user to go find an API key is simply wrong.
pub(crate) fn unserviceable_auth_prompt_message(view: Option<&AuthPromptView>) -> &'static str {
    match view.and_then(|view| view.challenge_kind) {
        Some(AuthPromptChallengeKind::ManualToken) => MANUAL_TOKEN_AUTH_UNAVAILABLE_MESSAGE,
        _ => AUTH_UNAVAILABLE_MESSAGE,
    }
}

/// Footer for triggered **gate** prompts (approval / OAuth auth). The user
/// can act on this specific request in the channel, but cannot otherwise
/// drive the run. `label` is a short trigger identifier (truncated prompt);
/// omitted when empty.
pub(crate) fn triggered_gate_footer(label: &str) -> String {
    let label = label.trim();
    let lead = if label.is_empty() {
        "From a triggered event.".to_string()
    } else {
        format!("From a triggered event: “{label}”.")
    };
    format!(
        "\n\n_{lead} You can respond to this request here — to otherwise interact \
         with this run, open the Ironclaw web app._"
    )
}

/// Footer for triggered **updates / final replies**. These are output only —
/// there is nothing to act on in the channel, so it points to the web app.
pub(crate) fn triggered_update_footer(label: &str) -> String {
    let label = label.trim();
    let lead = if label.is_empty() {
        "From a triggered event.".to_string()
    } else {
        format!("From a triggered event: “{label}”.")
    };
    format!(
        "\n\n_{lead} You can't interact with triggered events here — open the \
         Ironclaw web app to interact with this run._"
    )
}

/// Truncate a trigger prompt to a short single-line label for the footer.
pub(crate) fn triggered_label_from_prompt(prompt: &str) -> String {
    const MAX_LABEL_CHARS: usize = 60;
    let first_line = prompt.lines().next().unwrap_or("").trim();
    if first_line.chars().count() <= MAX_LABEL_CHARS {
        first_line.to_string()
    } else {
        let truncated: String = first_line.chars().take(MAX_LABEL_CHARS).collect();
        format!("{truncated}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::PairingPromptView;
    use ironclaw_turns::TurnRunId;

    fn view(challenge_kind: Option<AuthPromptChallengeKind>) -> AuthPromptView {
        AuthPromptView {
            turn_run_id: TurnRunId::new(),
            auth_request_ref: "gate:auth:1".to_string(),
            invocation_id: None,
            headline: "Authenticate to continue".to_string(),
            body: "body".to_string(),
            challenge_kind,
            provider: None,
            account_label: None,
            authorization_url: None,
            expires_at: None,
            connection: None,
            pairing: None,
        }
    }

    fn pairing(code: &str) -> PairingPromptView {
        PairingPromptView {
            channel: "fixture".to_string(),
            display_name: "Fixture Messenger".to_string(),
            instructions: "Send this code to the bot.".to_string(),
            code: code.to_string(),
            deep_link: None,
            expires_at: chrono::Utc::now(),
        }
    }

    /// A pairing challenge is completable from a chat surface, so it must not
    /// be denied like a credential-entry challenge. Regression for #6616,
    /// whose merge resolution dropped every consumer of `challenge_kind` and
    /// left a bare `authorization_url.is_some()` check -- which sends every
    /// pairing user the "this needs an API key" copy and cancels their run.
    #[test]
    fn pairing_challenge_with_a_code_is_serviceable_in_chat() {
        let mut pairing_view = view(Some(AuthPromptChallengeKind::Pairing));
        pairing_view.pairing = Some(pairing("ABCD2345"));

        assert!(auth_prompt_is_serviceable(&pairing_view));
        assert_eq!(
            actionable_auth_prompt_body(&pairing_view),
            "Send this code to the bot.",
            "a serviceable pairing challenge renders its host-issued instructions"
        );
    }

    #[test]
    fn incomplete_pairing_challenge_fails_closed_without_the_credential_copy() {
        let mut pairing_view = view(Some(AuthPromptChallengeKind::Pairing));
        pairing_view.pairing = Some(pairing("   "));

        assert!(
            !auth_prompt_is_serviceable(&pairing_view),
            "a pairing challenge with no usable code cannot be serviced"
        );
        assert_eq!(
            unserviceable_auth_prompt_message(Some(&pairing_view)),
            AUTH_UNAVAILABLE_MESSAGE,
            "a pairing user must not be told to go find an API key"
        );
    }

    /// A serviceable pairing challenge is useless unless the CODE reaches the
    /// user. `auth_prompt_text` only ever rendered headline/body/reply-hint and
    /// the OAuth `Setup link`, so a Slack/Telegram DM showed "send /start
    /// followed by the displayed code" and never displayed one. #6520 rendered
    /// through the canonical `render_channel_auth_prompt`, which emits the
    /// code, deep link and expiry; the restoration kept the older renderer.
    #[test]
    fn channel_auth_prompt_renders_the_pairing_code_not_just_instructions() {
        let mut view = view(Some(AuthPromptChallengeKind::Pairing));
        view.pairing = Some(pairing("ABCD2345"));
        view.body = actionable_auth_prompt_body(&view);

        let text = auth_prompt_text(&view, true);

        assert!(
            text.contains("ABCD2345"),
            "the pairing code must reach the channel message; got:\n{text}"
        );
        assert!(
            text.contains("Send this code to the bot."),
            "instructions must still render; got:\n{text}"
        );
        assert!(
            text.contains("auth deny"),
            "the cancel affordance must survive; got:\n{text}"
        );
    }

    /// Switching renderers must not drop the OAuth setup link.
    #[test]
    fn channel_auth_prompt_still_renders_the_oauth_setup_link() {
        let mut view = view(Some(AuthPromptChallengeKind::OAuthUrl));
        view.authorization_url = Some("https://provider.example/authorize".to_string());

        let text = auth_prompt_text(&view, true);

        assert!(
            text.contains("https://provider.example/authorize"),
            "OAuth setup link must survive; got:\n{text}"
        );
    }

    #[test]
    fn manual_token_challenge_is_never_serviceable_and_keeps_the_credential_copy() {
        let token_view = view(Some(AuthPromptChallengeKind::ManualToken));

        assert!(
            !auth_prompt_is_serviceable(&token_view),
            "pasting a secret into chat stores it in the conversation"
        );
        assert_eq!(
            unserviceable_auth_prompt_message(Some(&token_view)),
            MANUAL_TOKEN_AUTH_UNAVAILABLE_MESSAGE
        );
    }

    #[test]
    fn oauth_challenge_needs_a_usable_url_and_unknown_kinds_fail_closed() {
        let mut oauth = view(Some(AuthPromptChallengeKind::OAuthUrl));
        assert!(!auth_prompt_is_serviceable(&oauth), "no url, no service");
        oauth.authorization_url = Some("   ".to_string());
        assert!(!auth_prompt_is_serviceable(&oauth), "blank url, no service");
        oauth.authorization_url = Some("https://provider.example/authorize".to_string());
        assert!(auth_prompt_is_serviceable(&oauth));

        assert!(!auth_prompt_is_serviceable(&view(Some(
            AuthPromptChallengeKind::Other
        ))));
        assert_eq!(
            unserviceable_auth_prompt_message(None),
            AUTH_UNAVAILABLE_MESSAGE,
            "an absent view cannot imply a credential type"
        );
    }
}

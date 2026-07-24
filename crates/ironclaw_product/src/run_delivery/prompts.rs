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
pub(crate) const PAIRING_PRIVATE_SETUP_MESSAGE: &str = "Open the Ironclaw web app to connect or pair this extension in a private setup surface, then ask me again here.";
pub(crate) const OAUTH_PRIVATE_SETUP_MESSAGE: &str = "Open the Ironclaw web app to complete this private authorization step, then ask me again here.";
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

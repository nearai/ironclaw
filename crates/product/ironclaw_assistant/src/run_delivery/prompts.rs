//! Generic user-facing copy and prompt text for channel run delivery.
//!
//! Every string here is channel-neutral English; the channel adapter owns
//! only formatting (markdown → vendor markup) and splitting. Prompt bodies
//! are authored in markdown (`**bold**`, backticks) so adapters can render
//! them into their native markup.

use ironclaw_extension_contracts::auth_prompt::{AuthPromptChallengeKind, AuthPromptView};
use ironclaw_host_api::turn::{TurnGateRef, TurnRunId};
use ironclaw_outbound::RunNotificationEventKind;
use ironclaw_product_contracts::outbound::{ApprovalPromptContextView, GatePromptView};

use crate::is_approval_gate_ref;

/// A small rotation of warm "on it" notices posted while a run works, so a
/// shared channel with several runs in flight does not fill with identical
/// lines. Picked deterministically from a per-run seed (see [`working_message`])
/// so a single run keeps one voice while concurrent runs vary.
pub(crate) const WORKING_MESSAGES: &[&str] = &[
    "On it!",
    "Let me look into that…",
    "Working on it…",
    "Give me a sec…",
    "Digging in…",
    "Let me take care of that…",
    "Looking into it…",
    "Got it — on this now…",
];

/// Pick a working notice deterministically from `seed` (the run id), so the
/// same run always shows the same line and two concurrent runs usually differ.
pub(crate) fn working_message(seed: u64) -> &'static str {
    WORKING_MESSAGES[(seed % WORKING_MESSAGES.len() as u64) as usize]
}

/// Escalated "still working" notices that replace the initial indicator when a
/// run runs long, so the user knows it hasn't stalled. Picked by `(seed, nudge)`
/// so successive nudges on one run vary their wording (see [`long_running_message`]).
pub(crate) const LONG_RUNNING_MESSAGES: &[&str] = &[
    "Still on it — this one's taking a little longer than usual…",
    "Hang tight, I haven't forgotten you — still working through this…",
    "This is taking a bit longer than expected, but I'm still on it…",
    "Bear with me — still crunching through this one…",
];

/// Pick an escalated "still working" notice for the `nudge`-th update of a run,
/// varying the wording each time while staying deterministic for replays.
pub(crate) fn long_running_message(seed: u64, nudge: u64) -> &'static str {
    LONG_RUNNING_MESSAGES[(seed.wrapping_add(nudge) % LONG_RUNNING_MESSAGES.len() as u64) as usize]
}
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
/// Posted to a background run's notification channels when it fails. The
/// failure detail itself stays in the run history — a notification channel is
/// a shared surface and carries no diagnostics.
pub(crate) const BACKGROUND_RUN_FAILED_MESSAGE: &str =
    "A routine run failed — open the Ironclaw web app for details.";
/// Redacted stand-in for an OAuth auth prompt on a notification channel that
/// is NOT a personal DM: the authorization URL is a bearer-grade secret and
/// must never land in a shared conversation.
pub(crate) const BACKGROUND_RUN_REAUTH_MESSAGE: &str =
    "A routine needs re-authorization — open the Ironclaw web app to continue.";
pub(crate) const DELIVERY_TIMEOUT_MESSAGE: &str =
    "This is taking longer than expected — check the WebUI for the result.";
pub(crate) const DELIVERY_ERROR_MESSAGE: &str =
    "Something went wrong delivering the result here. Check the WebUI.";
/// Posted when an interactive channel run reaches a terminal *failed* state
/// (as opposed to a self-authored error reply): the working indicator is
/// replaced with this so the user gets closure instead of a stuck "thinking"
/// line. Channel-neutral and diagnostic-free — details stay in the web app.
pub(crate) const RUN_FAILED_MESSAGE: &str =
    "That run didn't finish — open the Ironclaw web app for details.";
/// Posted as the terminal reply for a triggered run that ended in
/// `Cancelled` (host cancel: auth-auto-deny, policy, supersession, or an
/// operator action). A scheduled run has no user in the channel to act on a
/// cancel, so this is informational only — it closes the loop on a fire that
/// would otherwise vanish (#6896).
pub(crate) const TRIGGERED_RUN_CANCELED_MESSAGE: &str = "This scheduled run was canceled before it could finish. Open the Ironclaw web app for details.";
/// Posted when the blocking run is `BlockedApproval` and no gate_ref is
/// available.
pub(crate) const BUSY_APPROVAL_MESSAGE: &str = "Ironclaw is waiting on a pending approval before taking new messages — reply `approve` or `deny` (or `approve gate:<ref>`) to resume.";
/// Posted for any other non-terminal blocking state, or when the state
/// lookup fails.
pub(crate) const BUSY_GENERIC_MESSAGE: &str = "Ironclaw is still working on a previous message and can't take this one yet — please resend it once the current task finishes.";

/// Stable per-(run, kind) projection id for run-notification deliveries.
///
/// `discriminator` separates notices that share an event kind within one run.
/// The delivery id hashes this ref, so two notices that collapse to the same
/// string share a durable delivery identity: the second is reported
/// `AlreadyDelivered` and never actually sent. `None` preserves the historical
/// id shape for kinds that can only occur once per run.
pub(crate) fn run_notification_projection_id(
    run_id: TurnRunId,
    event_kind: RunNotificationEventKind,
    discriminator: Option<&str>,
) -> String {
    let suffix = match event_kind {
        RunNotificationEventKind::FinalReplyReady => "final",
        RunNotificationEventKind::ProgressUpdate => "progress",
        RunNotificationEventKind::ApprovalNeeded => "approval",
        RunNotificationEventKind::AuthRequired => "auth",
        RunNotificationEventKind::RunBlocked => "blocked",
        RunNotificationEventKind::DeliveryStatus => "delivery-status",
        RunNotificationEventKind::ModelDelivery => "model-delivery",
    };
    match discriminator {
        Some(discriminator) => {
            let discriminator = bounded_discriminator(discriminator);
            format!("run-notification:{suffix}:{discriminator}:{run_id}")
        }
        None => format!("run-notification:{suffix}:{run_id}"),
    }
}

/// Longest discriminator embedded verbatim in a projection id. The composed
/// id must fit `ProjectionUpdateRef`'s 256-byte cap for ANY legal input — a
/// `TurnGateRef` may itself be up to 256 bytes — or the notice becomes
/// undeliverable at the ref constructor. Production gate refs are far shorter
/// (`gate:approval-<uuid>`), so this bound never changes their id shape.
const DISCRIMINATOR_VERBATIM_MAX: usize = 96;

/// Over-long discriminators keep a readable prefix plus a stable FNV-1a 64
/// content hash: still one id per distinct gate, never truncation-collided.
/// FNV-1a is implemented inline because these ids are DURABLE delivery
/// identities — a std hasher whose output can change across releases would
/// silently re-key them.
fn bounded_discriminator(discriminator: &str) -> std::borrow::Cow<'_, str> {
    if discriminator.len() <= DISCRIMINATOR_VERBATIM_MAX {
        return std::borrow::Cow::Borrowed(discriminator);
    }
    // Room for the ':' + 16 hex digits inside the verbatim budget.
    let mut end = DISCRIMINATOR_VERBATIM_MAX - 17;
    while !discriminator.is_char_boundary(end) {
        end -= 1;
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in discriminator.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    std::borrow::Cow::Owned(format!("{}:{hash:016x}", &discriminator[..end]))
}

/// Build the approval-gate prompt view. The body carries only the semantic
/// *What/Why* of the gate; the channel-agnostic *how to reply* is appended
/// once by [`gate_prompt_text`].
pub(crate) fn approval_gate_prompt_view(
    run_id: TurnRunId,
    gate_ref: &TurnGateRef,
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
    ironclaw_extension_contracts::auth_prompt::render_channel_auth_prompt(view, direct_message)
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
    use ironclaw_extension_contracts::auth_prompt::PairingPromptView;
    use ironclaw_host_api::turn::TurnRunId;

    #[test]
    fn working_message_is_always_a_member_and_varies_by_seed() {
        // Every seed maps to one of the rotation lines.
        for seed in 0u64..32 {
            assert!(WORKING_MESSAGES.contains(&working_message(seed)));
        }
        // Deterministic for a given seed (one run keeps one voice).
        assert_eq!(working_message(3), working_message(3));
        // The rotation actually varies across runs (not one hardcoded line).
        let distinct: std::collections::HashSet<&str> = (0u64..WORKING_MESSAGES.len() as u64)
            .map(working_message)
            .collect();
        assert!(
            distinct.len() > 1,
            "the working notice should vary across runs, got {distinct:?}"
        );
    }

    /// The delivery id is derived from this ref, so two notices that share a
    /// string share a durable delivery identity — the second comes back
    /// `AlreadyDelivered` and is silently never sent while still recorded as
    /// delivered. One run legitimately emits several `RunBlocked` notices, so
    /// they must not collapse.
    #[test]
    fn run_blocked_notices_in_one_run_do_not_share_a_projection_id() {
        let run_id = TurnRunId::new();
        let discriminated: Vec<String> = ["reauth", "auth-unavailable", "failed"]
            .into_iter()
            .map(|discriminator| {
                run_notification_projection_id(
                    run_id,
                    RunNotificationEventKind::RunBlocked,
                    Some(discriminator),
                )
            })
            .collect();

        let unique: std::collections::HashSet<&String> = discriminated.iter().collect();
        assert_eq!(
            unique.len(),
            discriminated.len(),
            "each RunBlocked notice needs its own delivery identity: {discriminated:?}"
        );
        for id in &discriminated {
            assert!(
                id.contains(&run_id.to_string()),
                "{id} must stay run-scoped"
            );
        }
    }

    /// Kinds that occur at most once per run keep the historical id shape, so
    /// this change does not re-identify existing deliveries.
    #[test]
    fn undiscriminated_projection_ids_keep_their_historical_shape() {
        let run_id = TurnRunId::new();
        assert_eq!(
            run_notification_projection_id(run_id, RunNotificationEventKind::ApprovalNeeded, None),
            format!("run-notification:approval:{run_id}")
        );
        assert_eq!(
            run_notification_projection_id(run_id, RunNotificationEventKind::AuthRequired, None),
            format!("run-notification:auth:{run_id}")
        );
        // FinalReplyReady is the kind production always sends refless: its
        // suffix keys the in-flight final-reply delivery identities, so a
        // rename would double-post final replies across a deploy.
        assert_eq!(
            run_notification_projection_id(run_id, RunNotificationEventKind::FinalReplyReady, None),
            format!("run-notification:final:{run_id}")
        );
    }

    #[test]
    fn discriminated_projection_ids_embed_short_gate_refs_verbatim() {
        let run_id = TurnRunId::new();
        assert_eq!(
            run_notification_projection_id(
                run_id,
                RunNotificationEventKind::ApprovalNeeded,
                Some("gate:approval-1234"),
            ),
            format!("run-notification:approval:gate:approval-1234:{run_id}")
        );
    }

    /// A legal `TurnGateRef` can be up to 256 bytes while `ProjectionUpdateRef`
    /// caps at 256: over-long discriminators must compress rather than make
    /// the gate notice undeliverable, without colliding distinct gates.
    #[test]
    fn over_long_discriminators_stay_deliverable_and_distinct() {
        let run_id = TurnRunId::new();
        let shared_prefix = "g".repeat(240);
        let ref_a = format!("{shared_prefix}-a");
        let ref_b = format!("{shared_prefix}-b");
        let id_a = run_notification_projection_id(
            run_id,
            RunNotificationEventKind::ApprovalNeeded,
            Some(&ref_a),
        );
        let id_b = run_notification_projection_id(
            run_id,
            RunNotificationEventKind::ApprovalNeeded,
            Some(&ref_b),
        );
        assert!(
            id_a.len() <= 256,
            "composed id must fit the ref cap: {id_a}"
        );
        assert_ne!(id_a, id_b, "prefix-sharing gates must not collide");
        // Deterministic: the same gate re-announced computes the same id.
        assert_eq!(
            id_a,
            run_notification_projection_id(
                run_id,
                RunNotificationEventKind::ApprovalNeeded,
                Some(&ref_a),
            )
        );
    }

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

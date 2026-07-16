//! Pending gate/auth state and resolution `Effect`.
//!
//! `turn_run_id`/`gate_ref` are stored as plain `String` rather than the
//! wire types (`TurnRunId`, and `AuthPromptView::auth_request_ref` doubling
//! as this state's `gate_ref`): the wire types come from `ironclaw_turns`,
//! which this crate's dependency boundary forbids naming (see `app/mod.rs`'s
//! module doc). The `String` is captured once, via `Display`, at the moment
//! the prompt is received — `ApiCall::ResolveGate` wants `String` anyway.
//! Same reasoning for `AuthPromptView::challenge_kind`: kept as its wire
//! string (via [`super::wire_label`]) rather than the un-nameable
//! `AuthPromptChallengeKind` enum — the same "subtractive mirror, raw wire
//! string" pattern `client/threads.rs`'s `ThreadMessageSummary` already uses
//! for `kind`/`status`.

use crossterm::event::{KeyCode, KeyEvent};
use ironclaw_product_workflow::{AuthPromptView, GatePromptView, WebUiGateResolution};

use super::{ApiCall, AppState, Effect, wire_label};

#[derive(Debug, Clone)]
pub enum PendingGate {
    Approval {
        turn_run_id: String,
        gate_ref: String,
        headline: String,
        body: String,
        allow_always: bool,
    },
    Auth {
        turn_run_id: String,
        gate_ref: String,
        headline: String,
        body: String,
        /// Wire value of `AuthPromptChallengeKind` (e.g. `"oauth_url"`,
        /// `"manual_token"`) — see the module doc.
        challenge_kind: Option<String>,
        authorization_url: Option<String>,
        /// Provider id (e.g. `"google"`) the manual-token flow submits
        /// alongside the raw token — from `AuthPromptView::provider` or the
        /// projection item's `AuthPromptContextView::provider`.
        provider: Option<String>,
        /// From `AuthPromptView::account_label`; [`submit_token`] falls
        /// back to `"{provider} credential"` when absent, mirroring
        /// webui's `useChat.ts::submitAuthToken`.
        account_label: Option<String>,
        /// `Some(buffer)` while the manual-token input sub-mode is active
        /// (entered via the `t` key — see [`dispatch_gate_key`]), holding
        /// the characters typed so far; `None` while the gate shows its
        /// normal a/A/d/o/esc prompt. Only meaningful on `Auth` — approval
        /// gates have no manual-token flow.
        token_input: Option<String>,
    },
}

impl PendingGate {
    pub fn gate_ref(&self) -> &str {
        match self {
            Self::Approval { gate_ref, .. } | Self::Auth { gate_ref, .. } => gate_ref,
        }
    }

    pub fn turn_run_id(&self) -> &str {
        match self {
            Self::Approval { turn_run_id, .. } | Self::Auth { turn_run_id, .. } => turn_run_id,
        }
    }

    fn allow_always(&self) -> bool {
        matches!(
            self,
            Self::Approval {
                allow_always: true,
                ..
            }
        )
    }

    /// The single fixture ctor for tests across `app::*` (collapses the
    /// plan's separate `approval_stub`/`approval_stub_with` into one name
    /// that always takes the headline).
    #[cfg(test)]
    pub(crate) fn approval_stub(headline: &str) -> Self {
        Self::Approval {
            turn_run_id: "run-stub".to_string(),
            gate_ref: "gate-stub".to_string(),
            headline: headline.to_string(),
            body: String::new(),
            allow_always: true,
        }
    }
}

pub(crate) fn apply_gate_prompt(state: &mut AppState, prompt: GatePromptView) -> Vec<Effect> {
    set_pending_gate_if_new(
        state,
        PendingGate::Approval {
            turn_run_id: prompt.turn_run_id.to_string(),
            gate_ref: prompt.gate_ref,
            headline: prompt.headline,
            body: prompt.body,
            allow_always: prompt.allow_always,
        },
    );
    Vec::new()
}

pub(crate) fn apply_auth_prompt(state: &mut AppState, prompt: AuthPromptView) -> Vec<Effect> {
    set_pending_gate_if_new(
        state,
        PendingGate::Auth {
            turn_run_id: prompt.turn_run_id.to_string(),
            gate_ref: prompt.auth_request_ref,
            headline: prompt.headline,
            body: prompt.body,
            challenge_kind: prompt.challenge_kind.map(|kind| wire_label(&kind)),
            authorization_url: prompt.authorization_url,
            provider: prompt.provider,
            account_label: prompt.account_label,
            token_input: None,
        },
    );
    Vec::new()
}

/// Already wire-type-erased fields of a `ProductProjectionItem::Gate`,
/// bundled into one struct so [`apply_projection_gate`] stays under
/// clippy's `too_many_arguments` — see that function's doc for why the
/// fields are primitives rather than the wire types themselves.
pub(crate) struct ProjectionGateFields {
    pub(crate) turn_run_id: String,
    pub(crate) gate_ref: String,
    pub(crate) headline: String,
    pub(crate) body: String,
    pub(crate) allow_always: bool,
    pub(crate) is_auth: bool,
    pub(crate) challenge_kind: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) account_label: Option<String>,
    pub(crate) authorization_url: Option<String>,
}

/// Builds a pending gate from a `ProductProjectionItem::Gate`'s fields
/// (mirrors the frontend's `gateFromProjectionGate` in
/// `crates/ironclaw_webui_v2/frontend/src/pages/chat/lib/gates.ts`), then
/// applies the same first-wins dedupe as [`apply_gate_prompt`]/
/// [`apply_auth_prompt`].
///
/// Takes primitive fields (via [`ProjectionGateFields`]) rather than
/// `&ProductProjectionItem` / `ProductGateKind` / `AuthPromptContextView`
/// directly: none of those three types are re-exported by
/// `ironclaw_product_workflow`, so — per this module's/`app/mod.rs`'s
/// boundary doc — this crate's production code never names them.
/// `transcript::apply_projection_item` destructures the wire item (where the
/// compiler still has the concrete types in scope) and passes through
/// already-erased values: `is_auth` is `gate_kind` reduced through
/// `wire_label`, and `challenge_kind`/`authorization_url` are pulled out of
/// `auth_context` the same way.
pub(crate) fn apply_projection_gate(
    state: &mut AppState,
    fields: ProjectionGateFields,
) -> Vec<Effect> {
    let ProjectionGateFields {
        turn_run_id,
        gate_ref,
        headline,
        body,
        allow_always,
        is_auth,
        challenge_kind,
        provider,
        account_label,
        authorization_url,
    } = fields;
    let candidate = if is_auth {
        PendingGate::Auth {
            turn_run_id,
            gate_ref,
            headline,
            body,
            challenge_kind,
            authorization_url,
            provider,
            account_label,
            token_input: None,
        }
    } else {
        PendingGate::Approval {
            turn_run_id,
            gate_ref,
            headline,
            body,
            allow_always,
        }
    };
    set_pending_gate_if_new(state, candidate);
    Vec::new()
}

/// The single gate-dedupe seam: the same gate can arrive twice — once as a
/// raw `gate`/`auth_required` frame, once as a `ProjectionUpdate` `Gate`
/// item (or the projection item can repeat across a reconnect's replayed
/// snapshot) — keyed on `(turn_run_id, gate_ref)`. First arrival wins;
/// later arrivals for the same key are a no-op, mirroring the frontend's
/// `setPendingGate((current) => current || pendingGate)`. A different
/// pending gate (different key) still overwrites, same as before this
/// dedupe existed.
fn set_pending_gate_if_new(state: &mut AppState, candidate: PendingGate) {
    let is_duplicate = state.pending_gate.as_ref().is_some_and(|existing| {
        existing.turn_run_id() == candidate.turn_run_id()
            && existing.gate_ref() == candidate.gate_ref()
    });
    if !is_duplicate {
        state.pending_gate = Some(candidate);
    }
}

/// Key handling while a gate is pending (`Focus::GateZone`), reached from
/// `dispatch_key_inner` in `app/mod.rs`.
pub(crate) fn dispatch_gate_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(pending) = state.pending_gate.clone() else {
        return Vec::new();
    };

    // Token-input sub-mode claims every key itself (typed chars, Enter,
    // Esc) before the normal a/A/d/o handlers below ever see it.
    if let PendingGate::Auth {
        token_input: Some(_),
        ..
    } = &pending
    {
        return dispatch_token_input_key(state, key);
    }

    match key.code {
        KeyCode::Char('a') => resolve(
            state,
            &pending,
            WebUiGateResolution::Approved { always: false },
        ),
        KeyCode::Char('A') if pending.allow_always() => resolve(
            state,
            &pending,
            WebUiGateResolution::Approved { always: true },
        ),
        KeyCode::Char('d') => resolve(state, &pending, WebUiGateResolution::Declined),
        KeyCode::Esc => {
            // Esc is a real server-side cancel, mirroring webui's Cancel
            // button — it must not merely clear the local view, or the run
            // stays blocked server-side forever. `WebUiGateResolution` has
            // no distinct `Cancelled` variant (only `Approved`/`Declined`/
            // `CredentialProvided`), and the server's `parse_gate_resolution`
            // (`webui_inbound.rs`) already treats the wire strings
            // `"denied"` and `"cancelled"` as synonyms for `Declined`, so
            // `Declined` is the correct resolution to send here — same as
            // the `d` key, just triggered by a different key.
            resolve(state, &pending, WebUiGateResolution::Declined)
        }
        KeyCode::Char('o') => {
            if let PendingGate::Auth {
                authorization_url: Some(url),
                ..
            } = &pending
            {
                open_authorization_url(url);
            }
            Vec::new()
        }
        // Enters the manual-token input sub-mode. Only meaningful on an
        // `Auth` gate — the pattern below simply doesn't match `Approval`,
        // so `t` stays a no-op there, same as any other unbound key.
        KeyCode::Char('t') => {
            if let Some(PendingGate::Auth { token_input, .. }) = &mut state.pending_gate {
                *token_input = Some(String::new());
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// Key handling once the manual-token input sub-mode is active (reached
/// from [`dispatch_gate_key`] above). Esc cancels back to the normal gate
/// prompt without resolving anything server-side — unlike the gate's own
/// Esc, which is a real decline; this Esc only ever touches local state.
fn dispatch_token_input_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    match key.code {
        KeyCode::Esc => {
            if let Some(PendingGate::Auth { token_input, .. }) = &mut state.pending_gate {
                *token_input = None;
            }
            Vec::new()
        }
        KeyCode::Backspace => {
            if let Some(PendingGate::Auth {
                token_input: Some(buf),
                ..
            }) = &mut state.pending_gate
            {
                buf.pop();
            }
            Vec::new()
        }
        KeyCode::Char(c) => {
            if let Some(PendingGate::Auth {
                token_input: Some(buf),
                ..
            }) = &mut state.pending_gate
            {
                buf.push(c);
            }
            Vec::new()
        }
        KeyCode::Enter => submit_token(state),
        _ => Vec::new(),
    }
}

/// Submits the token typed so far (Enter, while in the sub-mode). A blank
/// (or whitespace-only) buffer is a no-op — mirrors `dispatch_composer_key`'s
/// `Enter` guard on `state.composer_text` — so an accidental empty Enter
/// doesn't fire a doomed request or exit the sub-mode. On a non-empty
/// submit, exits the sub-mode back to the normal gate prompt (the gate
/// itself stays pending until the server confirms — same posture as
/// [`resolve`]) and emits [`ApiCall::SubmitManualToken`]; `lib.rs`'s
/// `execute_api_call` chains the returned `credential_ref` into
/// `ApiCall::ResolveGate`'s `CredentialProvided` resolution (step 2 of the
/// two-step flow — see `client/gates.rs::submit_manual_token`'s doc).
fn submit_token(state: &mut AppState) -> Vec<Effect> {
    let Some(pending) = state.pending_gate.clone() else {
        return Vec::new();
    };
    let PendingGate::Auth {
        token_input: Some(token),
        provider,
        account_label,
        ..
    } = &pending
    else {
        return Vec::new();
    };
    let token = token.trim().to_string();
    if token.is_empty() {
        return Vec::new();
    }
    let provider = provider.clone().unwrap_or_default();
    let account_label = account_label
        .clone()
        .unwrap_or_else(|| format!("{provider} credential"));
    let thread_id = state.thread_id.clone().unwrap_or_default();
    let run_id = pending.turn_run_id().to_string();
    let gate_ref = pending.gate_ref().to_string();

    if let Some(PendingGate::Auth { token_input, .. }) = &mut state.pending_gate {
        *token_input = None;
    }

    vec![Effect::Api(ApiCall::SubmitManualToken {
        thread_id,
        run_id,
        gate_ref,
        provider,
        account_label,
        token,
    })]
}

/// Best-effort local browser launch for an auth prompt's `authorization_url`
/// — the `o` key. Local-only: emits no `Effect`, since opening a URL is not
/// a server-side mutation. Never surfaces an error to the user or panics on
/// failure (e.g. no GUI/opener binary in a headless session): the URL stays
/// printed in the gate body regardless (`ui/gate_zone.rs::describe`), so a
/// failed shell-out just leaves the user to copy/paste it manually. Does not
/// wait for the opener to exit — `spawn`, not `status`/`output` — so a slow
/// or hanging opener can't freeze the TUI's event loop.
fn open_authorization_url(url: &str) {
    let (program, args) = open_command(url);
    let _ = std::process::Command::new(program).args(args).spawn();
}

/// Pure command/args construction for [`open_authorization_url`], kept
/// separate so tests can assert the platform opener choice without actually
/// spawning a process (which would pop open a real browser).
fn open_command(url: &str) -> (&'static str, Vec<String>) {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    (program, vec![url.to_string()])
}

/// The gate stays `pending_gate: Some(..)` after this — it only clears when
/// the server confirms the resolution via a later event (see
/// `transcript::apply_server_event`'s `Cancelled` handling), never
/// optimistically here.
fn resolve(
    state: &AppState,
    pending: &PendingGate,
    resolution: WebUiGateResolution,
) -> Vec<Effect> {
    vec![Effect::Api(ApiCall::ResolveGate {
        thread_id: state.thread_id.clone().unwrap_or_default(),
        run_id: pending.turn_run_id().to_string(),
        gate_ref: pending.gate_ref().to_string(),
        resolution,
    })]
}

#[cfg(test)]
mod tests {
    use ironclaw_product_workflow::webchat_schema::WebChatV2Event;

    use super::super::test_support::{auth_prompt, boxed_frame, gate_prompt, key};
    use super::super::{AppState, Focus, reduce};
    use super::*;
    use crate::app::AppEvent;

    #[test]
    fn gate_event_sets_pending_gate_and_focuses_gate_zone() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::Gate {
                prompt: gate_prompt("gr-1", false),
            })),
        );
        assert_eq!(state.focus(), Focus::GateZone);
        assert_eq!(state.pending_gate.as_ref().unwrap().gate_ref(), "gr-1");
    }

    #[test]
    fn approve_key_emits_resolve_effect_and_clears_pending_gate_optimistically_off() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::Gate {
                prompt: gate_prompt("gr-1", false),
            })),
        );
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Char('a'))));
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::ResolveGate { gate_ref, resolution: WebUiGateResolution::Approved { .. }, .. })
                if gate_ref == "gr-1"
        ));
        assert!(
            state.pending_gate.is_some(),
            "gate stays pending until server confirms via a new event"
        );
    }

    #[test]
    fn allow_always_requires_allow_always_flag() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::Gate {
                prompt: gate_prompt("gr-1", false),
            })),
        );
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Char('A'))));
        assert!(
            effects.is_empty(),
            "Shift+A must not always-approve when the prompt disallows it"
        );
    }

    #[test]
    fn auth_required_event_sets_pending_gate_with_auth_kind() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::AuthRequired {
                prompt: auth_prompt("ar-1"),
            })),
        );
        assert!(matches!(state.pending_gate, Some(PendingGate::Auth { .. })));
    }

    #[test]
    fn esc_on_auth_gate_emits_declined_resolution_not_just_a_local_dismiss() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::AuthRequired {
                prompt: auth_prompt("ar-1"),
            })),
        );
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Esc)));
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::ResolveGate {
                gate_ref,
                resolution: WebUiGateResolution::Declined,
                ..
            }) if gate_ref == "ar-1"
        ));
        assert!(
            state.pending_gate.is_some(),
            "gate stays pending until the server confirms the cancel via a new event"
        );
    }

    #[test]
    fn esc_on_approval_gate_emits_declined_resolution() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::Gate {
                prompt: gate_prompt("gr-1", false),
            })),
        );
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Esc)));
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::ResolveGate {
                gate_ref,
                resolution: WebUiGateResolution::Declined,
                ..
            }) if gate_ref == "gr-1"
        ));
    }

    #[test]
    fn open_command_picks_the_platform_opener_and_passes_the_url_verbatim() {
        // Pure-helper test only — `open_authorization_url` (the `o` key's
        // real handler) actually spawns the opener, which would pop open a
        // real browser if exercised through `reduce`/`dispatch_gate_key`
        // here. This asserts the command construction without spawning.
        let (program, args) = open_command("https://example.com/oauth");
        let expected_program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        assert_eq!(program, expected_program);
        assert_eq!(args, vec!["https://example.com/oauth".to_string()]);
    }

    #[test]
    fn decline_key_emits_declined_resolution() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::Gate {
                prompt: gate_prompt("gr-1", false),
            })),
        );
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Char('d'))));
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::ResolveGate {
                resolution: WebUiGateResolution::Declined,
                ..
            })
        ));
    }

    #[test]
    fn t_key_on_auth_gate_enters_token_input_sub_mode() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::AuthRequired {
                prompt: auth_prompt("ar-1"),
            })),
        );
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Char('t'))));
        assert!(
            effects.is_empty(),
            "entering the sub-mode is local-only, no API call yet"
        );
        assert!(matches!(
            &state.pending_gate,
            Some(PendingGate::Auth { token_input: Some(buf), .. }) if buf.is_empty()
        ));
    }

    #[test]
    fn t_key_on_approval_gate_is_a_no_op() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::Gate {
                prompt: gate_prompt("gr-1", false),
            })),
        );
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Char('t'))));
        assert!(
            effects.is_empty(),
            "approval gates have no manual-token flow"
        );
        assert!(matches!(
            state.pending_gate,
            Some(PendingGate::Approval { .. })
        ));
    }

    #[test]
    fn typed_chars_in_sub_mode_accumulate_and_never_reach_the_a_d_handlers() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::AuthRequired {
                prompt: auth_prompt("ar-1"),
            })),
        );
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char('t'))));
        // `a`/`d` would normally approve/decline — while in the sub-mode
        // they must just be captured as text instead.
        for c in ['a', 'b', 'd'] {
            reduce(&mut state, AppEvent::Key(key(KeyCode::Char(c))));
        }
        reduce(&mut state, AppEvent::Key(key(KeyCode::Backspace)));
        assert!(matches!(
            &state.pending_gate,
            Some(PendingGate::Auth { token_input: Some(buf), .. }) if buf == "ab"
        ));
    }

    #[test]
    fn esc_in_sub_mode_exits_locally_without_resolving_the_gate() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::AuthRequired {
                prompt: auth_prompt("ar-1"),
            })),
        );
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char('t'))));
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char('x'))));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Esc)));
        assert!(
            effects.is_empty(),
            "sub-mode Esc must not emit ResolveGate, unlike the gate's own Esc"
        );
        assert!(matches!(
            &state.pending_gate,
            Some(PendingGate::Auth {
                token_input: None,
                ..
            })
        ));
    }

    #[test]
    fn enter_in_sub_mode_emits_submit_manual_token_and_exits_the_sub_mode() {
        let mut state = AppState::default().set_thread_id("t-1");
        let prompt = AuthPromptView {
            provider: Some("google".to_string()),
            account_label: Some("work@example.com".to_string()),
            ..auth_prompt("ar-1")
        };
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::AuthRequired { prompt })),
        );
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char('t'))));
        for c in "sekret".chars() {
            reduce(&mut state, AppEvent::Key(key(KeyCode::Char(c))));
        }
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));

        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::SubmitManualToken {
                thread_id,
                gate_ref,
                provider,
                account_label,
                token,
                ..
            }) if thread_id == "t-1"
                && gate_ref == "ar-1"
                && provider == "google"
                && account_label == "work@example.com"
                && token == "sekret"
        ));
        assert!(
            matches!(
                &state.pending_gate,
                Some(PendingGate::Auth {
                    token_input: None,
                    ..
                })
            ),
            "submitting exits the sub-mode back to the normal gate prompt"
        );
    }

    #[test]
    fn enter_in_sub_mode_falls_back_to_a_derived_account_label_when_absent() {
        let mut state = AppState::default().set_thread_id("t-1");
        let prompt = AuthPromptView {
            provider: Some("google".to_string()),
            account_label: None,
            ..auth_prompt("ar-1")
        };
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::AuthRequired { prompt })),
        );
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char('t'))));
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char('x'))));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));

        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::SubmitManualToken { account_label, .. })
                if account_label == "google credential"
        ));
    }

    #[test]
    fn enter_with_a_blank_token_input_is_a_no_op() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::AuthRequired {
                prompt: auth_prompt("ar-1"),
            })),
        );
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char('t'))));
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char(' '))));
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));
        assert!(effects.is_empty());
        assert!(
            matches!(
                &state.pending_gate,
                Some(PendingGate::Auth {
                    token_input: Some(_),
                    ..
                })
            ),
            "still in the sub-mode — nothing was submitted"
        );
    }
}

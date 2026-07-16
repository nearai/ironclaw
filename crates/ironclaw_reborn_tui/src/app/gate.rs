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
            // Esc is local-only: it dismisses the local gate-zone view but
            // does not resolve the gate server-side, so the run stays
            // blocked until a resolution actually arrives.
            state.pending_gate = None;
            Vec::new()
        }
        _ => Vec::new(),
    }
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
    fn esc_on_auth_gate_emits_no_api_call_and_just_dismisses_local_view() {
        let mut state = AppState::default();
        reduce(
            &mut state,
            AppEvent::Server(boxed_frame(WebChatV2Event::AuthRequired {
                prompt: auth_prompt("ar-1"),
            })),
        );
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Esc)));
        assert!(effects.is_empty());
        assert!(
            state.pending_gate.is_none(),
            "Esc dismisses the local gate-zone view only"
        );
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
}

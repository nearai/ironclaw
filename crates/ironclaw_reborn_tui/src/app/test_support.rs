//! Shared test-only fixtures for `app::*` reducer tests. One place for
//! constructing wire-schema values (`WebChatV2EventFrame`, `GatePromptView`,
//! …) and `KeyEvent`s so each submodule's test list stays focused on
//! behavior, not boilerplate. `ironclaw_turns`/`ironclaw_host_api`/`chrono`
//! are dev-dependencies ONLY (see `app/mod.rs`'s module doc): production
//! code in this crate never names those types, but constructing a valid
//! `GatePromptView`/`CapabilityActivityView`/… test fixture requires them.
#![allow(dead_code)]

use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ironclaw_host_api::{CapabilityId, InvocationId, ThreadId};
use ironclaw_product_workflow::webchat_schema::{WebChatV2Event, WebChatV2EventFrame};
use ironclaw_product_workflow::{
    AuthPromptView, CapabilityActivityStatusView, CapabilityActivityView, FinalReplyView,
    GatePromptView, LlmProviderView, ProjectionCursor, RebornGetRunStateResponse,
};
use ironclaw_turns::{AcceptedMessageRef, EventCursor, SanitizedFailure, TurnRunId, TurnStatus};

use super::automations_modal::AutomationsModalState;
use super::provider_modal::ProviderModalState;
use super::threads_modal::ThreadsModalState;
use super::{Modal, PendingGate};
use crate::client::{AutomationSummary, ThreadSummary};

pub(crate) fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub(crate) fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

pub(crate) fn frame(event: WebChatV2Event) -> WebChatV2EventFrame {
    WebChatV2EventFrame {
        cursor: ProjectionCursor::new("cursor:tui:test:1").expect("cursor"),
        event,
    }
}

/// `AppEvent::Server` boxes its frame (clippy `large_enum_variant`); this is
/// the boxed counterpart of [`frame`] for tests that drive events through
/// `reduce(state, AppEvent::Server(..))` rather than calling
/// `transcript::apply_server_event` directly.
pub(crate) fn boxed_frame(event: WebChatV2Event) -> Box<WebChatV2EventFrame> {
    Box::new(frame(event))
}

pub(crate) fn gate_prompt(gate_ref: &str, allow_always: bool) -> GatePromptView {
    GatePromptView {
        turn_run_id: TurnRunId::new(),
        gate_ref: gate_ref.to_string(),
        invocation_id: None,
        headline: "Approve action".to_string(),
        body: "Review the requested action.".to_string(),
        allow_always,
        approval_context: None,
    }
}

pub(crate) fn auth_prompt(auth_request_ref: &str) -> AuthPromptView {
    AuthPromptView {
        turn_run_id: TurnRunId::new(),
        auth_request_ref: auth_request_ref.to_string(),
        invocation_id: None,
        headline: "Connect account".to_string(),
        body: "Connect before continuing.".to_string(),
        challenge_kind: None,
        provider: None,
        account_label: None,
        authorization_url: None,
        expires_at: None,
        connection: None,
    }
}

/// `invocation_id` is caller-supplied (rather than freshly minted here) so a
/// test can call this twice with the SAME id to exercise upsert-by-id
/// behavior (`InvocationId` is `Copy`).
pub(crate) fn activity_view(
    invocation_id: InvocationId,
    status: CapabilityActivityStatusView,
    output_summary: Option<&str>,
) -> CapabilityActivityView {
    CapabilityActivityView {
        invocation_id,
        turn_run_id: Some(TurnRunId::new()),
        thread_id: Some(ThreadId::new("thread-tui-test").expect("thread")),
        capability_id: CapabilityId::new("builtin.test_tool").expect("capability"),
        status,
        provider: None,
        runtime: None,
        process_id: None,
        output_bytes: None,
        error_kind: None,
        error_detail: None,
        subtitle: output_summary.map(str::to_string),
        input_summary: None,
        updated_at: Utc::now(),
        activity_order: None,
    }
}

pub(crate) fn final_reply_view(text: &str) -> FinalReplyView {
    FinalReplyView {
        turn_run_id: TurnRunId::new(),
        text: text.to_string(),
        generated_at: Utc::now(),
    }
}

pub(crate) fn run_state_with_failure(
    category: &str,
    detail: Option<&str>,
) -> RebornGetRunStateResponse {
    let failure = detail.into_iter().fold(
        SanitizedFailure::new(category).expect("sanitized failure"),
        |f, d| f.with_detail(d),
    );
    RebornGetRunStateResponse {
        turn_id: "turn-tui-test".to_string(),
        run_id: TurnRunId::new(),
        status: TurnStatus::Failed,
        event_cursor: EventCursor(1),
        accepted_message_ref: AcceptedMessageRef::new("msg:tui-test").expect("message ref"),
        resolved_run_profile_id: "default".to_string(),
        resolved_run_profile_version: 1,
        received_at: Utc::now(),
        checkpoint_id: None,
        gate_ref: None,
        failure: Some(failure),
    }
}

pub(crate) fn thread_summary(thread_id: &str) -> ThreadSummary {
    ThreadSummary {
        thread_id: thread_id.to_string(),
        title: None,
        created_at: None,
        updated_at: None,
    }
}

pub(crate) fn threads_modal_with<const N: usize>(ids: [&str; N], selected: usize) -> Modal {
    Modal::Threads(ThreadsModalState {
        threads: ids.into_iter().map(thread_summary).collect(),
        selected,
        pending_delete_confirm: false,
        loading: false,
    })
}

pub(crate) fn automation_summary(id: &str, name: &str, state: &str) -> AutomationSummary {
    AutomationSummary {
        automation_id: id.to_string(),
        name: name.to_string(),
        state: state.to_string(),
        next_run_at: None,
        last_run_at: None,
        last_status: None,
        is_active: state == "active",
    }
}

pub(crate) fn automations_modal_with(rows: &[(&str, &str, &str)], selected: usize) -> Modal {
    Modal::Automations(AutomationsModalState {
        automations: rows
            .iter()
            .map(|(id, name, state)| automation_summary(id, name, state))
            .collect(),
        selected,
        loading: false,
        renaming: None,
    })
}

pub(crate) fn provider_view(id: &str, adapter: &str) -> LlmProviderView {
    LlmProviderView {
        id: id.to_string(),
        description: id.to_string(),
        adapter: adapter.to_string(),
        default_model: "default-model".to_string(),
        base_url: None,
        builtin: true,
        active: false,
        active_model: None,
        api_key_required: false,
        accepts_api_key: true,
        api_key_set: false,
        can_list_models: true,
    }
}

pub(crate) fn providers_modal_with(rows: &[(&str, &str)], selected: usize) -> Modal {
    Modal::Provider(ProviderModalState::Providers {
        providers: rows
            .iter()
            .map(|(id, adapter)| provider_view(id, adapter))
            .collect(),
        selected,
        loading: false,
    })
}

pub(crate) fn models_modal_with(
    provider_id: &str,
    adapter: &str,
    models: &[&str],
    selected: usize,
) -> Modal {
    Modal::Provider(ProviderModalState::Models {
        provider_id: provider_id.to_string(),
        adapter: adapter.to_string(),
        base_url: None,
        models: models.iter().map(|m| m.to_string()).collect(),
        selected,
        loading: false,
    })
}

pub(crate) fn approval_gate(gate_ref: &str, allow_always: bool) -> PendingGate {
    PendingGate::Approval {
        turn_run_id: TurnRunId::new().to_string(),
        gate_ref: gate_ref.to_string(),
        headline: "Approve action".to_string(),
        body: String::new(),
        allow_always,
    }
}

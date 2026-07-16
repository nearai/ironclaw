//! Reborn TUI application state — a pure reducer.
//!
//! [`reduce`] turns one [`AppEvent`] into a state mutation plus a list of
//! [`Effect`]s the runtime (`lib.rs`, a later task) executes. Nothing in this
//! module or its submodules performs I/O: HTTP calls, terminal I/O, and the
//! `crossterm`/SSE event sources all live outside `app/`.
//!
//! Dependency-boundary note (verified against
//! `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`):
//! among internal `ironclaw_*` crates, `ironclaw_reborn_tui` may depend ONLY
//! on `ironclaw_product_workflow` — the allowlist enforcing that also names
//! `ironclaw_turns` explicitly as a crate that must never sneak in. That
//! means wire types this crate cannot re-export by name (`TurnRunId`,
//! `TurnStatus`, `AuthPromptChallengeKind`, `SanitizedFailure`, …) are never
//! stored as typed fields here: run ids are captured as `String` (via each
//! type's own `Display`) at the point a `GatePromptView`/`AuthPromptView` is
//! received, and enum wire values are rendered through [`wire_label`] (their
//! own `Serialize` impl, never `Debug`) instead of being named and matched.
//! `ironclaw_turns`/`ironclaw_host_api`/`chrono` are still fine as
//! *dev-dependencies* to build test-only fixture values of the wire types
//! this crate's production code only ever threads through opaquely — the
//! boundary test reads only `[dependencies]`.

pub mod automations_modal;
pub mod connection;
pub mod gate;
pub mod provider_modal;
pub mod threads_modal;
pub mod transcript;

#[cfg(test)]
mod test_support;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ironclaw_product_workflow::webchat_schema::WebChatV2EventFrame;
use ironclaw_product_workflow::{CapabilityActivityView, CapabilityDisplayPreviewView};

pub use automations_modal::AutomationsModalState;
pub use gate::PendingGate;
pub use provider_modal::ProviderModalState;
pub use threads_modal::ThreadsModalState;

/// Renders any wire enum through its own `Serialize` impl (its stable wire
/// string, e.g. `"blocked_approval"`), never through `Debug`. Used for
/// `TurnStatus`/`AuthPromptChallengeKind` values this crate receives as
/// opaque fields (see the module doc) but cannot name as types.
pub(crate) fn wire_label<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Events the terminal event loop (`lib.rs`) feeds into [`reduce`].
///
/// `Server` boxes the frame (clippy `large_enum_variant`: a
/// `WebChatV2EventFrame` is ~370 bytes, dwarfing every other variant) —
/// pure indirection, not a contract change; construct with
/// `AppEvent::Server(Box::new(frame))`.
#[derive(Debug, Clone)]
pub enum AppEvent {
    Server(Box<WebChatV2EventFrame>),
    Key(KeyEvent),
    Tick,
    Conn(ConnState),
}

/// SSE connection health, driven by the runtime's reconnect loop
/// (`client/events.rs` owns the actual backoff).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnState {
    #[default]
    Connected,
    Reconnecting {
        attempt: u8,
    },
    Lost,
}

/// Which modal (if any) is on top of the main screen. `OverlayKind` was
/// deleted in review round 2 (thermo MAJOR 3 + maintainability #7): no
/// consumer existed once `render()` branches directly on `state.modal` /
/// `state.pending_gate`.
#[derive(Debug, Clone)]
pub enum Modal {
    Threads(ThreadsModalState),
    Automations(AutomationsModalState),
    Provider(ProviderModalState),
}

/// Where keyboard input currently routes. Precedence (highest to lowest):
/// a lost connection banner beats an open modal, which beats a pending
/// gate, which beats the composer. See [`AppState::focus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    ConnBanner,
    Modal,
    GateZone,
    Composer,
}

/// One row of the rendered transcript. Canonical shape (B2.0, binding) —
/// every consumer uses this exact enum.
#[derive(Debug, Clone)]
pub enum TranscriptItem {
    User { text: String },
    Assistant { text: String },
    Activity(CapabilityActivityView),
    Preview(CapabilityDisplayPreviewView),
    System { text: String },
    Error { text: String },
}

impl TranscriptItem {
    pub fn final_text(text: impl Into<String>) -> Self {
        Self::Assistant { text: text.into() }
    }

    pub fn as_activity(&self) -> Option<&CapabilityActivityView> {
        match self {
            Self::Activity(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_final_text(&self) -> Option<&str> {
        match self {
            Self::Assistant { text } => Some(text),
            _ => None,
        }
    }

    pub fn as_error_text(&self) -> Option<&str> {
        match self {
            Self::Error { text } => Some(text),
            _ => None,
        }
    }
}

/// Canonical shape (B2.0, binding) of every server-mutating call the
/// reducer can request. `WebUiGateResolution` is `ironclaw_product_workflow`'s
/// client-facing resolution enum — reused rather than a local mirror (review
/// round 2).
#[derive(Debug, Clone, PartialEq)]
pub enum ApiCall {
    ListThreads,
    CreateThread,
    DeleteThread {
        thread_id: String,
    },
    LoadTimeline {
        thread_id: String,
        limit: u32,
        cursor: Option<String>,
    },
    SendMessage {
        thread_id: String,
        text: String,
    },
    ResolveGate {
        thread_id: String,
        run_id: String,
        gate_ref: String,
        resolution: ironclaw_product_workflow::WebUiGateResolution,
    },
    ListAutomations,
    PauseAutomation {
        id: String,
    },
    ResumeAutomation {
        id: String,
    },
    RenameAutomation {
        id: String,
        name: String,
    },
    LlmProviders,
    LlmListModels {
        provider_id: String,
        adapter: String,
        base_url: Option<String>,
    },
    LlmSetActive {
        provider_id: String,
        model: String,
    },
    LlmTestConnection {
        provider_id: String,
        adapter: String,
        base_url: Option<String>,
    },
}

/// Canonical shape (B2.0, binding): everything the reducer hands back to the
/// runtime to execute.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Api(ApiCall),
    Quit,
}

/// The whole TUI application state. Sparse-override construction follows
/// `.claude/rules/default-builders.md`: `AppState::default().set_x(..)`.
#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub conn: ConnState,
    pub modal: Option<Modal>,
    pub pending_gate: Option<PendingGate>,
    pub transcript: Vec<TranscriptItem>,
    pub thread_id: Option<String>,
    pub composer_text: String,
    pub quitting: bool,
    pub running: bool,
    pub last_local_error: Option<String>,
}

impl AppState {
    pub fn set_conn(mut self, conn: ConnState) -> Self {
        self.conn = conn;
        self
    }

    pub fn set_modal(mut self, modal: Option<Modal>) -> Self {
        self.modal = modal;
        self
    }

    pub fn set_pending_gate(mut self, pending_gate: Option<PendingGate>) -> Self {
        self.pending_gate = pending_gate;
        self
    }

    pub fn set_thread_id(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }

    pub fn set_composer_text(mut self, composer_text: impl Into<String>) -> Self {
        self.composer_text = composer_text.into();
        self
    }

    pub fn set_running(mut self, running: bool) -> Self {
        self.running = running;
        self
    }

    pub fn set_last_local_error(mut self, last_local_error: Option<impl Into<String>>) -> Self {
        self.last_local_error = last_local_error.map(Into::into);
        self
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Focus precedence: a lost connection beats an open modal, which beats
    /// a pending gate, which beats the composer. `Reconnecting` (unlike
    /// `Lost`) does not pre-empt anything — only a fully lost connection
    /// takes over the screen.
    pub fn focus(&self) -> Focus {
        if matches!(self.conn, ConnState::Lost) {
            return Focus::ConnBanner;
        }
        if self.modal.is_some() {
            return Focus::Modal;
        }
        if self.pending_gate.is_some() {
            return Focus::GateZone;
        }
        Focus::Composer
    }
}

/// The one reducer entry point: turns one event into zero or more effects,
/// mutating `state` in place.
pub fn reduce(state: &mut AppState, event: AppEvent) -> Vec<Effect> {
    match event {
        AppEvent::Server(frame) => transcript::apply_server_event(state, *frame),
        AppEvent::Key(key) => dispatch_key(state, key),
        AppEvent::Tick => Vec::new(),
        AppEvent::Conn(conn) => connection::apply_conn_change(state, conn),
    }
}

const DISCONNECTED_LOCAL_ERROR: &str = "disconnected — reconnecting…";

/// The single disconnected-gate seam (B2.7): every key-driven branch below
/// is free to compute whatever `Effect`s it would normally produce; this
/// wrapper is the ONE place that inspects the result and, if the connection
/// is `Lost` and the branch wanted to touch the server, swaps the effects
/// for a local error instead. No modal/gate/composer handler duplicates this
/// check itself.
fn dispatch_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let effects = dispatch_key_inner(state, key);
    if matches!(state.conn, ConnState::Lost)
        && effects
            .iter()
            .any(|effect| matches!(effect, Effect::Api(_)))
    {
        state.last_local_error = Some(DISCONNECTED_LOCAL_ERROR.to_string());
        return Vec::new();
    }
    effects
}

/// Routes a key press to whichever surface is logically active, ignoring
/// connection state (that's `dispatch_key`'s job, above). Note this is
/// deliberately NOT keyed off `state.focus()`: `Focus::ConnBanner` is a
/// rendering-only concept (there is nothing on the banner itself to
/// interact with) — the modal/gate/composer underneath still owns key
/// routing even while the banner is showing, so the disconnected seam has
/// something to intercept.
fn dispatch_key_inner(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    if is_quit_key(key) {
        return vec![Effect::Quit];
    }
    if state.modal.is_some() {
        dispatch_modal_key(state, key)
    } else if state.pending_gate.is_some() {
        gate::dispatch_gate_key(state, key)
    } else {
        dispatch_composer_key(state, key)
    }
}

fn is_quit_key(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
}

fn dispatch_modal_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(modal) = state.modal.clone() else {
        return Vec::new();
    };
    match modal {
        Modal::Threads(m) => threads_modal::dispatch_key(state, key, m),
        Modal::Automations(m) => automations_modal::dispatch_key(state, key, m),
        Modal::Provider(m) => provider_modal::dispatch_key(state, key, m),
    }
}

/// Global open-a-modal shortcuts plus composer text editing. Only reachable
/// when no modal is open and no gate is pending (see `dispatch_key_inner`).
fn dispatch_composer_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('x') | KeyCode::Char('X') => threads_modal::open(state),
            KeyCode::Char('a') | KeyCode::Char('A') => automations_modal::open(state),
            KeyCode::Char('l') | KeyCode::Char('L') => provider_modal::open(state),
            _ => Vec::new(),
        };
    }
    match key.code {
        KeyCode::Enter => {
            let text = state.composer_text.trim().to_string();
            if text.is_empty() {
                return Vec::new();
            }
            let thread_id = state.thread_id.clone().unwrap_or_default();
            state.composer_text.clear();
            vec![Effect::Api(ApiCall::SendMessage { thread_id, text })]
        }
        KeyCode::Backspace => {
            state.composer_text.pop();
            Vec::new()
        }
        KeyCode::Char(c) => {
            state.composer_text.push(c);
            Vec::new()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;

    struct Case {
        name: &'static str,
        state: AppState,
        want: Focus,
    }

    #[test]
    fn precedence_table() {
        let cases = vec![
            Case {
                name: "lost connection beats an open modal",
                state: AppState::default()
                    .set_conn(ConnState::Lost)
                    .set_modal(Some(Modal::Threads(ThreadsModalState::default()))),
                want: Focus::ConnBanner,
            },
            Case {
                name: "modal beats a pending gate",
                state: AppState::default()
                    .set_modal(Some(Modal::Automations(AutomationsModalState::default())))
                    .set_pending_gate(Some(PendingGate::approval_stub("Allow write_file?"))),
                want: Focus::Modal,
            },
            Case {
                name: "pending gate beats composer",
                state: AppState::default()
                    .set_pending_gate(Some(PendingGate::approval_stub("Allow write_file?"))),
                want: Focus::GateZone,
            },
            Case {
                name: "reconnecting (not Lost) does not pre-empt the modal",
                state: AppState::default()
                    .set_conn(ConnState::Reconnecting { attempt: 1 })
                    .set_modal(Some(Modal::Provider(ProviderModalState::default()))),
                want: Focus::Modal,
            },
            Case {
                name: "nothing pending falls through to composer",
                state: AppState::default(),
                want: Focus::Composer,
            },
        ];
        for case in cases {
            assert_eq!(case.state.focus(), case.want, "case: {}", case.name);
        }
    }
}

#[cfg(test)]
mod dispatch_key_tests {
    use crossterm::event::KeyCode;

    use super::test_support::{ctrl, key};
    use super::*;

    #[test]
    fn composer_typing_appends_chars_and_backspace_removes() {
        let mut state = AppState::default();
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char('h'))));
        reduce(&mut state, AppEvent::Key(key(KeyCode::Char('i'))));
        assert_eq!(state.composer_text, "hi");
        reduce(&mut state, AppEvent::Key(key(KeyCode::Backspace)));
        assert_eq!(state.composer_text, "h");
    }

    #[test]
    fn composer_enter_sends_message_and_clears_composer() {
        let mut state = AppState::default()
            .set_thread_id("t-1")
            .set_composer_text("hello");
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));
        assert!(state.composer_text.is_empty());
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::SendMessage { thread_id, text })
                if thread_id == "t-1" && text == "hello"
        ));
    }

    #[test]
    fn composer_enter_with_empty_text_emits_nothing() {
        let mut state = AppState::default().set_thread_id("t-1");
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));
        assert!(effects.is_empty());
    }

    #[test]
    fn ctrl_c_emits_quit_from_the_composer() {
        let mut state = AppState::default();
        let effects = reduce(&mut state, AppEvent::Key(ctrl('c')));
        assert_eq!(effects, vec![Effect::Quit]);
    }
}

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

use std::collections::HashSet;

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
///
/// `LiveText`/`Thinking` (added for projection streaming — see
/// `app/transcript.rs`'s `apply_projection`) are distinct from `Assistant`:
/// they carry the projection item's own `id` so a repeated
/// `ProductProjectionItem::Text`/`Thinking` for the same id UPSERTS in
/// place (live text replaces itself as it streams) instead of appending a
/// new row per delta. `Assistant` stays the shape for a *durable* message —
/// the legacy `FinalReply` event and timeline rehydration
/// (`transcript_item_from_message` in `lib.rs`) both still produce it.
#[derive(Debug, Clone)]
pub enum TranscriptItem {
    User { text: String },
    Assistant { text: String },
    LiveText { id: String, body: String },
    Thinking { id: String, body: String },
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

    pub fn as_live_text(&self) -> Option<&str> {
        match self {
            Self::LiveText { body, .. } => Some(body),
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
    /// Step 1 of the manual-token auth flow (`app::gate::submit_token`,
    /// entered via the pending auth gate's `t` key). `lib.rs`'s
    /// `execute_api_call` chains the response's `credential_ref` straight
    /// into a follow-up `ResolveGate { resolution: CredentialProvided, .. }`
    /// — step 2 — rather than surfacing it as a separate effect, mirroring
    /// how `PauseAutomation`/`RenameAutomation` already chain a follow-up
    /// call within their own arm.
    SubmitManualToken {
        thread_id: String,
        run_id: String,
        gate_ref: String,
        provider: String,
        account_label: String,
        token: String,
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
    CancelRun {
        thread_id: String,
        run_id: String,
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
    /// Index (into `transcript`) of the first item the transcript pane
    /// should render. `None` is the default "follow" mode: the pane always
    /// shows the tail (`ui/transcript.rs` computes the window from the
    /// current length), so newly appended items stay visible. `Some(idx)`
    /// pins the view to an absolute, content-stable index set by
    /// `PageUp`/`PageDown`/`Home` (see `scroll_*` below) — because it's
    /// absolute rather than relative-to-the-tail, appending new transcript
    /// items never shifts what's on screen while scrolled. `End` (or
    /// `PageDown` walking off the tail) resets to `None` to resume follow.
    pub transcript_scroll: Option<usize>,
    /// `run_id` of the turn currently in flight, captured from the last
    /// non-terminal `RunStatus` projection item (`app/transcript.rs`'s
    /// `apply_run_status`) and cleared once that run settles. Lets a
    /// composer-focus `Esc` target the right run for `ApiCall::CancelRun`
    /// without this crate storing a typed `TurnRunId` (see the module
    /// doc's boundary note).
    pub active_run_id: Option<String>,
    /// `turn_run_id`s of every run whose outcome the currently-loaded
    /// timeline snapshot already represents — either from the last
    /// `LoadTimeline` page (`lib.rs`'s `apply_timeline_page` rebuilds this
    /// set alongside `transcript` on every load, from that page's messages'
    /// `turn_run_id`s) or from a live `FinalReply` this session already
    /// appended (`app/transcript.rs`'s `FinalReply` handling inserts as it
    /// goes). `app/transcript.rs`'s `apply_server_event`/`apply_projection_item`
    /// check membership here before applying ANY run-scoped item (not just
    /// `FinalReply`): a cursor-less SSE resubscribe (fired on every thread
    /// switch and on startup, see `lib.rs`'s `run_event_loop` — the wire's
    /// `after_cursor`/`Last-Event-ID` param exists but the timeline read
    /// exposes no resumable `ProjectionCursor` to derive it from, only a
    /// message-`sequence`-keyed backward-pagination cursor in a different,
    /// incompatible opaque-token space — see `client/threads.rs`'s module
    /// doc) replays the whole thread's event history from origin, which
    /// would otherwise re-append/resurrect every already-settled run's
    /// `Text`/`Thinking`/`WorkSummary`/`SkillActivation`/`Gate`/`AuthRequired`
    /// on top of the timeline snapshot, not just duplicate `FinalReply`.
    ///
    /// Coverage boundary: this only protects runs the loaded PAGE actually
    /// captured (`LoadTimeline`'s default page is the thread's newest N
    /// messages — see `reborn_services.rs::paginate_timeline_messages`).
    /// A run older than the loaded page's oldest message is not in this set
    /// and its replayed items are NOT filtered — the wire carries no
    /// per-item ordering key (sequence/timestamp) on `ProductProjectionItem`
    /// that would let the reducer tell "replay of out-of-page history" apart
    /// from "genuinely live" without one, so this is a real, documented
    /// residual gap (shared by the browser frontend's equivalent first-visit
    /// full redrain, `useChatEvents.ts`), not something this fix can close
    /// without a server-side resumable-cursor change.
    ///
    /// `HashSet` (not a `Vec`) because membership, not order, is all a dedup
    /// check needs — display order is exactly `transcript`'s own order.
    pub settled_run_ids: HashSet<String>,
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

    pub fn set_transcript_scroll(mut self, transcript_scroll: usize) -> Self {
        self.transcript_scroll = Some(transcript_scroll);
        self
    }

    pub fn set_active_run_id(mut self, active_run_id: impl Into<String>) -> Self {
        self.active_run_id = Some(active_run_id.into());
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
        && matches!(
            key.code,
            KeyCode::Char('c') | KeyCode::Char('C') | KeyCode::Char('d') | KeyCode::Char('D')
        )
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

/// Fixed logical page size for `PageUp`/`PageDown` transcript scrolling.
/// The reducer has no access to the actual render area (it stays pure, no
/// terminal I/O — see the module doc), so paging moves a constant number of
/// transcript items rather than a viewport-derived count; `ui/transcript.rs`
/// separately clamps whatever window this produces to what actually fits.
const TRANSCRIPT_PAGE_SIZE: usize = 10;

/// `PageUp`: enters (or advances) scrolled mode, moving the pinned top index
/// back by one page. Starting from follow (`None`), the first page anchors
/// one page above the current tail.
fn scroll_transcript_page_up(state: &mut AppState) {
    // While following, the implicit bottom-anchor is the length itself (one
    // past the last item) — subtracting a page from that lands one page
    // above the tail, matching what the follow window was just showing.
    let current = state.transcript_scroll.unwrap_or(state.transcript.len());
    state.transcript_scroll = Some(current.saturating_sub(TRANSCRIPT_PAGE_SIZE));
}

/// `PageDown`: while scrolled, advances the pinned top index down by one
/// page; once that would reach (or pass) the tail, resumes follow (`None`)
/// instead of pinning at an index that's already the tail. A no-op while
/// already following (there's nothing further down than the tail).
fn scroll_transcript_page_down(state: &mut AppState) {
    let Some(current) = state.transcript_scroll else {
        return;
    };
    let len = state.transcript.len();
    let next = current.saturating_add(TRANSCRIPT_PAGE_SIZE);
    state.transcript_scroll = if next >= len { None } else { Some(next) };
}

/// Global open-a-modal shortcuts plus composer text editing. Only reachable
/// when no modal is open and no gate is pending (see `dispatch_key_inner`).
fn dispatch_composer_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('x') | KeyCode::Char('X') => threads_modal::open(state),
            KeyCode::Char('a') | KeyCode::Char('A') => automations_modal::open(state),
            // Design doc wants `Ctrl+P`; `Ctrl+L` is kept working too (an
            // already-shipped binding) — both open the same modal.
            KeyCode::Char('l') | KeyCode::Char('L') | KeyCode::Char('p') | KeyCode::Char('P') => {
                provider_modal::open(state)
            }
            _ => Vec::new(),
        };
    }
    match key.code {
        KeyCode::Enter => {
            let text = state.composer_text.trim().to_string();
            if text.is_empty() {
                return Vec::new();
            }
            if text == "/exit" {
                return vec![Effect::Quit];
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
        // Cancels the in-flight run, mirroring webui's Cancel affordance.
        // Only reachable in plain chat: a modal or a pending gate would
        // have already claimed `Esc` via `dispatch_key_inner`'s focus
        // precedence, and this arm itself only fires an effect while a run
        // is actually active — otherwise there's nothing to cancel.
        KeyCode::Esc => match (
            state.running,
            state.thread_id.clone(),
            state.active_run_id.clone(),
        ) {
            (true, Some(thread_id), Some(run_id)) => {
                vec![Effect::Api(ApiCall::CancelRun { thread_id, run_id })]
            }
            _ => Vec::new(),
        },
        KeyCode::PageUp => {
            scroll_transcript_page_up(state);
            Vec::new()
        }
        KeyCode::PageDown => {
            scroll_transcript_page_down(state);
            Vec::new()
        }
        KeyCode::Home => {
            state.transcript_scroll = Some(0);
            Vec::new()
        }
        KeyCode::End => {
            state.transcript_scroll = None;
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

#[cfg(test)]
mod transcript_scroll_tests {
    use crossterm::event::KeyCode;

    use super::test_support::key;
    use super::*;

    fn state_with_items(count: usize) -> AppState {
        let mut state = AppState::default();
        for i in 0..count {
            state.transcript.push(TranscriptItem::System {
                text: format!("item {i}"),
            });
        }
        state
    }

    #[test]
    fn default_state_follows_with_no_scroll_pin() {
        let state = state_with_items(3);
        assert_eq!(
            state.transcript_scroll, None,
            "fresh state must default to follow mode"
        );
    }

    #[test]
    fn page_up_from_follow_pins_one_page_above_the_tail() {
        let mut state = state_with_items(25);
        reduce(&mut state, AppEvent::Key(key(KeyCode::PageUp)));
        assert_eq!(
            state.transcript_scroll,
            Some(15),
            "25 items - one page (10) = pinned top index 15"
        );
    }

    #[test]
    fn page_up_clamps_at_the_top_instead_of_going_negative() {
        let mut state = state_with_items(5);
        reduce(&mut state, AppEvent::Key(key(KeyCode::PageUp)));
        reduce(&mut state, AppEvent::Key(key(KeyCode::PageUp)));
        assert_eq!(
            state.transcript_scroll,
            Some(0),
            "saturating_sub must clamp at 0, never wrap"
        );
    }

    #[test]
    fn new_content_does_not_move_a_pinned_scroll_position() {
        let mut state = state_with_items(25);
        reduce(&mut state, AppEvent::Key(key(KeyCode::PageUp)));
        assert_eq!(state.transcript_scroll, Some(15));

        // Appending items past PageUp must not force the view back to the
        // tail — the pin is an absolute index, unaffected by pushes.
        state.transcript.push(TranscriptItem::System {
            text: "new item".to_string(),
        });
        assert_eq!(
            state.transcript_scroll,
            Some(15),
            "a pinned scroll position must survive new transcript content"
        );
    }

    #[test]
    fn end_resumes_follow() {
        let mut state = state_with_items(25).set_transcript_scroll(15);
        reduce(&mut state, AppEvent::Key(key(KeyCode::End)));
        assert_eq!(state.transcript_scroll, None, "End must resume follow");
    }

    #[test]
    fn home_pins_to_the_very_top() {
        let mut state = state_with_items(25);
        reduce(&mut state, AppEvent::Key(key(KeyCode::Home)));
        assert_eq!(state.transcript_scroll, Some(0));
    }

    #[test]
    fn page_down_walking_off_the_tail_resumes_follow() {
        let mut state = state_with_items(15).set_transcript_scroll(5);
        reduce(&mut state, AppEvent::Key(key(KeyCode::PageDown)));
        assert_eq!(
            state.transcript_scroll, None,
            "5 + page(10) = 15 >= len(15) must resume follow, not overshoot"
        );
    }

    #[test]
    fn page_down_while_following_is_a_no_op() {
        let mut state = state_with_items(25);
        reduce(&mut state, AppEvent::Key(key(KeyCode::PageDown)));
        assert_eq!(
            state.transcript_scroll, None,
            "nothing to page down to past the tail"
        );
    }
}

#[cfg(test)]
mod cancel_run_tests {
    use crossterm::event::KeyCode;

    use super::test_support::key;
    use super::*;

    #[test]
    fn esc_cancels_the_active_run_in_plain_chat() {
        let mut state = AppState::default()
            .set_thread_id("t-1")
            .set_active_run_id("run-1")
            .set_running(true);
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Esc)));
        assert_eq!(
            effects,
            vec![Effect::Api(ApiCall::CancelRun {
                thread_id: "t-1".to_string(),
                run_id: "run-1".to_string(),
            })]
        );
    }

    #[test]
    fn esc_is_a_no_op_when_no_run_is_active() {
        let mut state = AppState::default().set_thread_id("t-1");
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Esc)));
        assert!(
            effects.is_empty(),
            "nothing running, nothing to cancel: {effects:?}"
        );
    }

    #[test]
    fn esc_is_a_no_op_when_active_run_id_is_missing_even_if_running() {
        // Defensive: `running` alone isn't enough without a run id to send.
        let mut state = AppState::default().set_thread_id("t-1").set_running(true);
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Esc)));
        assert!(effects.is_empty());
    }
}

#[cfg(test)]
mod navigation_polish_tests {
    use crossterm::event::KeyCode;

    use super::test_support::{ctrl, key};
    use super::*;

    #[test]
    fn ctrl_p_opens_the_provider_modal_same_as_ctrl_l() {
        let mut state = AppState::default();
        let effects = reduce(&mut state, AppEvent::Key(ctrl('p')));
        assert!(matches!(state.modal, Some(Modal::Provider(_))));
        assert!(matches!(effects[0], Effect::Api(ApiCall::LlmProviders)));
    }

    #[test]
    fn slash_exit_on_enter_quits() {
        let mut state = AppState::default().set_composer_text("/exit");
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));
        assert_eq!(effects, vec![Effect::Quit]);
    }

    #[test]
    fn a_message_that_merely_contains_slash_exit_is_sent_normally() {
        let mut state = AppState::default()
            .set_thread_id("t-1")
            .set_composer_text("please /exit the loop");
        let effects = reduce(&mut state, AppEvent::Key(key(KeyCode::Enter)));
        assert!(matches!(
            &effects[0],
            Effect::Api(ApiCall::SendMessage { .. })
        ));
    }

    #[test]
    fn ctrl_d_quits_from_the_composer() {
        let mut state = AppState::default();
        let effects = reduce(&mut state, AppEvent::Key(ctrl('d')));
        assert_eq!(effects, vec![Effect::Quit]);
    }
}

/// Round-trip reversibility: every modal must eventually return focus to
/// the composer via `Esc`, never dead-end. Threads/Automations close in one
/// `Esc`; the provider modal's `Models` step steps back to `Providers`
/// first (existing, intentional — see `provider_modal::dispatch_esc`) and
/// needs a second `Esc` to reach the composer. One consolidated test over
/// all three modal kinds rather than three near-duplicates (each modal's
/// own `Esc` behavior is already unit-tested in its own module; this test's
/// job is specifically the cross-modal `Focus` contract).
#[cfg(test)]
mod modal_round_trip_tests {
    use crossterm::event::KeyCode;

    use super::test_support::{
        automations_modal_with, key, models_modal_with, providers_modal_with, threads_modal_with,
    };
    use super::*;

    fn esc(state: &mut AppState) {
        reduce(state, AppEvent::Key(key(KeyCode::Esc)));
    }

    #[test]
    fn threads_modal_esc_returns_focus_to_the_composer() {
        let mut state = AppState::default().set_modal(Some(threads_modal_with(["t-1"], 0)));
        esc(&mut state);
        assert_eq!(state.focus(), Focus::Composer);
    }

    #[test]
    fn automations_modal_esc_returns_focus_to_the_composer() {
        let mut state = AppState::default()
            .set_modal(Some(automations_modal_with(&[("a-1", "n", "active")], 0)));
        esc(&mut state);
        assert_eq!(state.focus(), Focus::Composer);
    }

    #[test]
    fn provider_modal_providers_level_esc_returns_focus_to_the_composer() {
        let mut state =
            AppState::default().set_modal(Some(providers_modal_with(&[("p-1", "openai")], 0)));
        esc(&mut state);
        assert_eq!(state.focus(), Focus::Composer);
    }

    #[test]
    fn provider_modal_models_level_esc_steps_back_then_closes_on_a_second_esc() {
        let mut state =
            AppState::default().set_modal(Some(models_modal_with("p-1", "openai", &["gpt-x"], 0)));
        esc(&mut state);
        assert_eq!(
            state.focus(),
            Focus::Modal,
            "Models steps back to Providers first, not straight to the composer"
        );
        assert!(matches!(
            &state.modal,
            Some(Modal::Provider(ProviderModalState::Providers { .. }))
        ));
        esc(&mut state);
        assert_eq!(state.focus(), Focus::Composer);
    }
}

//! `ironclaw_tui` — Modular Ratatui-based TUI for IronClaw.
//!
//! This crate provides the rendering engine, widget system, and event loop
//! for IronClaw's terminal user interface. It is intentionally decoupled
//! from the main `ironclaw` crate: the Channel trait bridge lives in
//! `src/channels/tui.rs` in the main crate.
//!
//! # Architecture
//!
//! ```text
//! ┌─ TuiApp (app.rs) ────────────────────────────────────────────┐
//! │  Event loop: poll crossterm → merge with TuiEvent rx         │
//! │  Render: Layout → Widget::render() → Terminal::draw()        │
//! │                                                              │
//! │  ┌─ Header ─────────────────────────────────────────────┐    │
//! │  │  version · model · duration · tokens                 │    │
//! │  ├─ Conversation ──────────┬─ Sidebar ──────────────────┤    │
//! │  │  Messages + markdown    │  Tools: live activity      │    │
//! │  │                         │  Threads: active/recent    │    │
//! │  ├─ Input ─────────────────┴────────────────────────────┤    │
//! │  │  › user input (tui-textarea)                         │    │
//! │  ├─ Status Bar ─────────────────────────────────────────┤    │
//! │  │  model │ tokens │ cost │ keybinds                    │    │
//! │  └──────────────────────────────────────────────────────┘    │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Communication
//!
//! The main crate sends [`TuiEvent`]s via the handle's `event_tx`, and
//! receives user messages via `msg_rx`. The TUI never calls into the
//! main crate directly.

pub mod app;
pub mod event;
pub mod input;
pub mod layout;
pub mod render;
pub mod spinner;
pub mod theme;
pub mod widgets;

pub use app::{TuiAppConfig, TuiAppHandle, start_tui};
pub use event::{
    EngineThreadEntry, HistoryApprovalRequest, HistoryMessage, ThreadEntry, TuiAttachment,
    TuiEvent, TuiLogEntry,
    TuiUserMessage,
};
pub use layout::TuiLayout;
pub use theme::Theme;
pub use widgets::{AppState, SkillCategory, ToolCategory};

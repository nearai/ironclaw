//! Application state for the TUI.

use std::collections::VecDeque;

use crate::channels::cli::composer::ChatComposer;
use crate::channels::cli::overlay::{ApprovalOverlay, ApprovalRequest};

/// Events that can occur in the TUI.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Keyboard/mouse input event.
    Input(crossterm::event::Event),
    /// Response from the agent.
    Response(String),
    /// Tool execution started.
    ToolStarted { name: String },
    /// Tool execution completed.
    ToolCompleted { name: String, success: bool },
    /// Request approval for a tool.
    ApprovalRequested(ApprovalRequest),
    /// Streaming chunk received.
    StreamChunk(String),
    /// Force a redraw.
    Redraw,
    /// Quit the application.
    Quit,
}

/// Current input mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Normal input mode.
    Normal,
    /// Editing input.
    Editing,
    /// Approval overlay is active.
    Approval,
}

/// Message in the chat history.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// Who sent this message.
    pub role: MessageRole,
    /// The message content.
    pub content: String,
    /// Optional status indicator.
    pub status: Option<MessageStatus>,
}

/// Who sent a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Agent,
    System,
}

/// Status of a message (for in-progress indicators).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStatus {
    Pending,
    InProgress,
    Complete,
    Error,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            status: None,
        }
    }

    pub fn agent(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Agent,
            content: content.into(),
            status: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
            status: None,
        }
    }

    pub fn with_status(mut self, status: MessageStatus) -> Self {
        self.status = Some(status);
        self
    }
}

/// Application state.
pub struct AppState {
    /// Current input mode.
    pub mode: InputMode,
    /// Chat message history.
    pub messages: Vec<ChatMessage>,
    /// Input composer.
    pub composer: ChatComposer,
    /// Approval overlay (if active).
    pub approval: Option<ApprovalOverlay>,
    /// Scroll offset for messages.
    pub scroll_offset: u16,
    /// Whether the app should quit.
    pub should_quit: bool,
    /// Pending approvals queue.
    pub pending_approvals: VecDeque<ApprovalRequest>,
    /// Current streaming response buffer.
    pub streaming_buffer: Option<String>,
    /// Status line message.
    pub status_message: Option<String>,
}

impl AppState {
    /// Create a new app state.
    pub fn new() -> Self {
        Self {
            mode: InputMode::Editing,
            messages: vec![ChatMessage::system(
                "Welcome to NEAR Agent. Type a message or /help for commands.",
            )],
            composer: ChatComposer::new(),
            approval: None,
            scroll_offset: 0,
            should_quit: false,
            pending_approvals: VecDeque::new(),
            streaming_buffer: None,
            status_message: None,
        }
    }

    /// Add a user message to history.
    pub fn add_user_message(&mut self, content: impl Into<String>) {
        self.messages.push(ChatMessage::user(content));
        self.scroll_to_bottom();
    }

    /// Add an agent response to history.
    pub fn add_agent_message(&mut self, content: impl Into<String>) {
        // If we were streaming, finalize it
        if self.streaming_buffer.is_some() {
            self.streaming_buffer = None;
        }
        self.messages.push(ChatMessage::agent(content));
        self.scroll_to_bottom();
    }

    /// Start streaming a response.
    pub fn start_streaming(&mut self) {
        self.streaming_buffer = Some(String::new());
        self.messages
            .push(ChatMessage::agent("").with_status(MessageStatus::InProgress));
    }

    /// Append to the streaming buffer.
    pub fn append_stream(&mut self, chunk: &str) {
        if let Some(ref mut buffer) = self.streaming_buffer {
            buffer.push_str(chunk);
            // Update the last message
            if let Some(last) = self.messages.last_mut() {
                if last.role == MessageRole::Agent {
                    last.content = buffer.clone();
                }
            }
        }
    }

    /// Finalize streaming.
    pub fn finish_streaming(&mut self) {
        if let Some(last) = self.messages.last_mut() {
            if last.role == MessageRole::Agent {
                last.status = Some(MessageStatus::Complete);
            }
        }
        self.streaming_buffer = None;
    }

    /// Show an approval request.
    pub fn show_approval(&mut self, request: ApprovalRequest) {
        self.approval = Some(ApprovalOverlay::new(request));
        self.mode = InputMode::Approval;
    }

    /// Queue an approval request.
    pub fn queue_approval(&mut self, request: ApprovalRequest) {
        if self.approval.is_none() {
            self.show_approval(request);
        } else {
            self.pending_approvals.push_back(request);
        }
    }

    /// Handle approval response.
    pub fn handle_approval_response(&mut self, approved: bool) -> Option<ApprovalRequest> {
        let request = self.approval.take().map(|o| o.request);

        // Show next pending approval if any
        if let Some(next) = self.pending_approvals.pop_front() {
            self.show_approval(next);
        } else {
            self.mode = InputMode::Editing;
        }

        if approved { request } else { None }
    }

    /// Clear all pending approvals.
    pub fn clear_approvals(&mut self) {
        self.approval = None;
        self.pending_approvals.clear();
        self.mode = InputMode::Editing;
    }

    /// Set the status message.
    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
    }

    /// Clear the status message.
    pub fn clear_status(&mut self) {
        self.status_message = None;
    }

    /// Scroll to the bottom of messages.
    pub fn scroll_to_bottom(&mut self) {
        // Will be calculated based on render area in render.rs
        self.scroll_offset = 0;
    }

    /// Scroll up.
    pub fn scroll_up(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
    }

    /// Scroll down.
    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

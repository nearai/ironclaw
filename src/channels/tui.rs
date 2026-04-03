//! TUI channel — bridges the `Channel` trait to `ironclaw_tui`.
//!
//! The TUI crate owns the terminal and event loop. This module translates
//! between the agent's `Channel` trait and `ironclaw_tui`'s event/message
//! channels.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use ironclaw_tui::{SkillCategory, ToolCategory, TuiAppConfig, TuiEvent, TuiLayout, start_tui};

use crate::channels::web::log_layer::LogBroadcaster;
use crate::channels::{
    AttachmentKind, Channel, IncomingAttachment, IncomingMessage, MessageStream, OutgoingResponse,
    StatusUpdate,
};
use crate::error::ChannelError;

/// Group tool names by their prefix (text before the first `_`).
///
/// Tools like `memory_search`, `memory_write` become `memory: search, write`.
/// Tools without an underscore are placed in a "general" category.
pub fn group_tools_by_prefix(mut names: Vec<String>) -> Vec<ToolCategory> {
    use std::collections::BTreeMap;
    names.sort();

    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for name in &names {
        if let Some(pos) = name.find('_') {
            let prefix = &name[..pos];
            let suffix = &name[pos + 1..];
            groups
                .entry(prefix.to_string())
                .or_default()
                .push(suffix.to_string());
        } else {
            groups
                .entry("general".to_string())
                .or_default()
                .push(name.clone());
        }
    }

    groups
        .into_iter()
        .map(|(name, tools)| ToolCategory { name, tools })
        .collect()
}

/// Group skills by their first tag.
///
/// Skills without tags are placed in a "general" category.
pub fn group_skills_by_tag(
    skills: &[(String, Vec<String>)], // (name, tags)
) -> Vec<SkillCategory> {
    use std::collections::BTreeMap;

    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, tags) in skills {
        let category = tags
            .first()
            .cloned()
            .unwrap_or_else(|| "general".to_string());
        groups.entry(category).or_default().push(name.clone());
    }

    groups
        .into_iter()
        .map(|(name, skills)| SkillCategory { name, skills })
        .collect()
}

/// Resolve the effective TUI layout from the workspace file plus env-backed
/// channel config. File-based widget settings are loaded first, then the
/// explicit config overrides theme and sidebar visibility.
pub fn resolve_tui_layout(
    config: &crate::config::TuiChannelConfig,
    workspace_root: &Path,
) -> TuiLayout {
    let layout_path = workspace_root.join("tui").join("layout.json");
    let mut layout = TuiLayout::load_from_file(&layout_path);
    layout.theme = config.theme.clone();
    layout.sidebar.visible = config.sidebar_visible;
    layout
}

fn infer_context_window(model_id: &str) -> u64 {
    let normalized = model_id
        .trim()
        .to_ascii_lowercase()
        .rsplit('/')
        .next()
        .unwrap_or(model_id)
        .split(':')
        .next()
        .unwrap_or(model_id)
        .to_string();

    if normalized.starts_with("gpt-5") {
        if normalized.contains("-chat") {
            return 128_000;
        }
        return 400_000;
    }

    if normalized.starts_with("gpt-4.1") {
        return 1_047_576;
    }

    if normalized.contains("claude-opus-4-6") || normalized.contains("claude-sonnet-4-6") {
        return 1_000_000;
    }

    if normalized.contains("claude") {
        return 200_000;
    }

    if normalized.starts_with("gemini-") {
        return 1_000_000;
    }

    128_000
}

/// TUI channel backed by `ironclaw_tui`.
pub struct TuiChannel {
    user_id: String,
    event_tx: Arc<Mutex<Option<mpsc::Sender<TuiEvent>>>>,
    started: AtomicBool,
    version: String,
    model: String,
    layout: TuiLayout,
    log_broadcaster: Option<Arc<LogBroadcaster>>,
    tools: Vec<ToolCategory>,
    skills: Vec<SkillCategory>,
    workspace_path: String,
    memory_count: usize,
    identity_files: Vec<String>,
    available_models: Vec<String>,
}

impl TuiChannel {
    /// Create a new TUI channel.
    pub fn new(
        user_id: impl Into<String>,
        version: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            user_id: user_id.into(),
            event_tx: Arc::new(Mutex::new(None)),
            started: AtomicBool::new(false),
            version: version.into(),
            model: model.into(),
            layout: TuiLayout::default(),
            log_broadcaster: None,
            tools: Vec::new(),
            skills: Vec::new(),
            workspace_path: String::new(),
            memory_count: 0,
            identity_files: Vec::new(),
            available_models: Vec::new(),
        }
    }

    /// Set the layout configuration.
    pub fn with_layout(mut self, layout: TuiLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Set the log broadcaster for forwarding log entries to the TUI.
    pub fn with_log_broadcaster(mut self, broadcaster: Arc<LogBroadcaster>) -> Self {
        self.log_broadcaster = Some(broadcaster);
        self
    }

    /// Set tool categories for the welcome screen.
    pub fn with_tools(mut self, tools: Vec<ToolCategory>) -> Self {
        self.tools = tools;
        self
    }

    /// Set skill categories for the welcome screen.
    pub fn with_skills(mut self, skills: Vec<SkillCategory>) -> Self {
        self.skills = skills;
        self
    }

    /// Set workspace path for the welcome screen.
    pub fn with_workspace_path(mut self, path: impl Into<String>) -> Self {
        self.workspace_path = path.into();
        self
    }

    /// Set the memory entry count for the welcome screen.
    pub fn with_memory_count(mut self, count: usize) -> Self {
        self.memory_count = count;
        self
    }

    /// Set the identity files for the welcome screen.
    pub fn with_identity_files(mut self, files: Vec<String>) -> Self {
        self.identity_files = files;
        self
    }

    /// Set the available models for the `/model` picker.
    pub fn with_available_models(mut self, models: Vec<String>) -> Self {
        self.available_models = models;
        self
    }
}

#[async_trait]
impl Channel for TuiChannel {
    fn name(&self) -> &str {
        "tui"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        if self.started.swap(true, Ordering::Relaxed) {
            return Err(ChannelError::StartupFailed {
                name: "tui".to_string(),
                reason: "TUI channel already started".to_string(),
            });
        }

        let config = TuiAppConfig {
            version: self.version.clone(),
            model: self.model.clone(),
            layout: self.layout.clone(),
            context_window: infer_context_window(&self.model),
            tools: self.tools.clone(),
            skills: self.skills.clone(),
            workspace_path: self.workspace_path.clone(),
            memory_count: self.memory_count,
            identity_files: self.identity_files.clone(),
            available_models: self.available_models.clone(),
        };

        let ironclaw_tui::TuiAppHandle {
            event_tx,
            mut msg_rx,
            join_handle: _join,
        } = start_tui(config);

        // Store event_tx for sending status updates and responses
        *self.event_tx.lock().await = Some(event_tx.clone());

        // Forward log entries from the LogBroadcaster to the TUI's Logs tab
        if let Some(ref broadcaster) = self.log_broadcaster {
            // Replay recent history first
            let log_tx = event_tx.clone();
            for entry in broadcaster.recent_entries() {
                let _ = log_tx
                    .send(TuiEvent::Log {
                        level: entry.level,
                        target: entry.target,
                        message: entry.message,
                        timestamp: entry.timestamp,
                    })
                    .await;
            }

            // Subscribe to live log stream
            let mut log_rx = broadcaster.subscribe();
            tokio::spawn(async move {
                while let Ok(entry) = log_rx.recv().await {
                    let event = TuiEvent::Log {
                        level: entry.level,
                        target: entry.target,
                        message: entry.message,
                        timestamp: entry.timestamp,
                    };
                    if log_tx.send(event).await.is_err() {
                        break;
                    }
                }
            });
        }

        // Bridge: forward user messages from TUI to the agent's MessageStream
        let (incoming_tx, incoming_rx) = mpsc::channel::<IncomingMessage>(32);
        let user_id = self.user_id.clone();
        let sys_tz = crate::timezone::detect_system_timezone().name().to_string();

        tokio::spawn(async move {
            while let Some(user_msg) = msg_rx.recv().await {
                let attachments: Vec<IncomingAttachment> = user_msg
                    .attachments
                    .into_iter()
                    .enumerate()
                    .map(|(i, a)| IncomingAttachment {
                        id: format!("tui-paste-{i}"),
                        kind: AttachmentKind::Image,
                        mime_type: a.mime_type,
                        filename: Some(format!("{}.png", a.label)),
                        size_bytes: Some(a.data.len() as u64),
                        source_url: None,
                        storage_key: None,
                        extracted_text: None,
                        data: a.data,
                        duration_secs: None,
                    })
                    .collect();

                let msg = IncomingMessage::new("tui", &user_id, &user_msg.text)
                    .with_timezone(&sys_tz)
                    .with_attachments(attachments);
                if incoming_tx.send(msg).await.is_err() {
                    break;
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(incoming_rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        if let Some(ref tx) = *self.event_tx.lock().await {
            let _ = tx
                .send(TuiEvent::Response {
                    content: response.content,
                    thread_id: response.thread_id,
                })
                .await;
        }
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let tx_guard = self.event_tx.lock().await;
        let Some(ref tx) = *tx_guard else {
            return Ok(());
        };

        let event = match status {
            StatusUpdate::Thinking(msg) => TuiEvent::Thinking(msg),
            StatusUpdate::ToolStarted {
                name,
                detail,
                call_id,
            } => TuiEvent::ToolStarted {
                name,
                detail,
                call_id,
            },
            StatusUpdate::ToolCompleted {
                name,
                success,
                error,
                call_id,
                ..
            } => TuiEvent::ToolCompleted {
                name,
                success,
                error,
                call_id,
            },
            StatusUpdate::ToolResult {
                name,
                preview,
                call_id,
            } => TuiEvent::ToolResult {
                name,
                preview,
                call_id,
            },
            StatusUpdate::StreamChunk(chunk) => TuiEvent::StreamChunk(chunk),
            StatusUpdate::Status(msg) => TuiEvent::Status(msg),
            StatusUpdate::JobStarted { job_id, title, .. } => {
                TuiEvent::JobStarted { job_id, title }
            }
            StatusUpdate::JobStatus { job_id, status } => TuiEvent::JobStatus { job_id, status },
            StatusUpdate::JobResult { job_id, status } => TuiEvent::JobResult { job_id, status },
            StatusUpdate::RoutineUpdate {
                id,
                name,
                trigger_type,
                enabled,
                last_run,
                next_fire,
            } => TuiEvent::RoutineUpdate {
                id,
                name,
                trigger_type,
                enabled,
                last_run,
                next_fire,
            },
            StatusUpdate::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters,
                allow_always,
            } => TuiEvent::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters,
                allow_always,
            },
            StatusUpdate::AuthRequired {
                extension_name,
                instructions,
                ..
            } => TuiEvent::AuthRequired {
                extension_name,
                instructions,
            },
            StatusUpdate::AuthCompleted {
                extension_name,
                success,
                message,
            } => TuiEvent::AuthCompleted {
                extension_name,
                success,
                message,
            },
            StatusUpdate::ReasoningUpdate {
                narrative,
                decisions: _,
            } => TuiEvent::ReasoningUpdate { narrative },
            StatusUpdate::TurnCost {
                input_tokens,
                output_tokens,
                cost_usd,
            } => TuiEvent::TurnCost {
                input_tokens,
                output_tokens,
                cost_usd,
            },
            StatusUpdate::ContextPressure {
                used_tokens,
                max_tokens,
                percentage,
                warning,
            } => TuiEvent::ContextPressure {
                used_tokens,
                max_tokens,
                percentage,
                warning,
            },
            StatusUpdate::SandboxStatus {
                docker_available,
                running_containers,
                status,
            } => TuiEvent::SandboxStatus {
                docker_available,
                running_containers,
                status,
            },
            StatusUpdate::SecretsStatus {
                count,
                vault_unlocked,
            } => TuiEvent::SecretsStatus {
                count,
                vault_unlocked,
            },
            StatusUpdate::CostGuard {
                session_budget_usd,
                spent_usd,
                remaining_usd,
                limit_reached,
            } => TuiEvent::CostGuard {
                session_budget_usd,
                spent_usd,
                remaining_usd,
                limit_reached,
            },
            StatusUpdate::Suggestions { suggestions } => TuiEvent::Suggestions { suggestions },
            StatusUpdate::ThreadList { threads } => TuiEvent::ThreadList {
                threads: threads
                    .into_iter()
                    .map(|t| ironclaw_tui::ThreadEntry {
                        id: t.id,
                        title: t.title,
                        message_count: t.message_count,
                        last_activity: t.last_activity,
                        channel: t.channel,
                    })
                    .collect(),
            },
            StatusUpdate::ConversationHistory {
                thread_id,
                messages,
                pending_approval,
            } => TuiEvent::ConversationHistory {
                thread_id,
                messages: messages
                    .into_iter()
                    .map(|m| ironclaw_tui::HistoryMessage {
                        role: m.role,
                        content: m.content,
                        timestamp: m.timestamp,
                    })
                    .collect(),
                pending_approval: pending_approval.map(|approval| {
                    ironclaw_tui::HistoryApprovalRequest {
                        request_id: approval.request_id,
                        tool_name: approval.tool_name,
                        description: approval.description,
                        parameters: approval.parameters,
                        allow_always: approval.allow_always,
                    }
                }),
            },
            StatusUpdate::SkillActivated { .. } | StatusUpdate::ImageGenerated { .. } => {
                return Ok(());
            }
        };

        let _ = tx.send(event).await;
        Ok(())
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        if let Some(ref tx) = *self.event_tx.lock().await {
            let _ = tx
                .send(TuiEvent::Response {
                    content: response.content,
                    thread_id: response.thread_id,
                })
                .await;
        }
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        // The TUI thread will exit when event channels are dropped
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn resolve_tui_layout_merges_file_and_config() {
        let temp = tempfile::tempdir().expect("tempdir");
        let layout_dir = temp.path().join("tui");
        std::fs::create_dir_all(&layout_dir).expect("layout dir");
        std::fs::write(
            layout_dir.join("layout.json"),
            r#"{"theme":"light","sidebar":{"visible":false,"width_percent":33}}"#,
        )
        .expect("layout file");

        let config = crate::config::TuiChannelConfig {
            theme: "dark".to_string(),
            sidebar_visible: true,
        };

        let layout = super::resolve_tui_layout(&config, temp.path());

        assert_eq!(layout.theme, "dark");
        assert!(layout.sidebar.visible);
        assert_eq!(layout.sidebar.width_percent, 33);
    }
}

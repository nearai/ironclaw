//! Migration helpers for importing external assistant state into IronClaw.
//!
//! The CLI entrypoint lives in `src/cli/migrate.rs`. This module owns the
//! source-specific readers plus the common write path into:
//! - Engine V2 workspace-backed state (`HybridStore`)
//! - Legacy DB conversation history (for current web/thread list views)
//! - Workspace documents
//! - Settings + encrypted secrets

pub mod hermes;
pub mod openclaw;

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use ironclaw_engine::types::conversation::EntryId;
use ironclaw_engine::{
    ConversationEntry, ConversationId, ConversationSurface, DocId, DocType, EntrySender, MemoryDoc,
    MessageRole, Project, ProjectId, Provenance, Store, Thread, ThreadConfig, ThreadId,
    ThreadMessage, ThreadState, ThreadType,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::bridge::HybridStore;
use crate::config::Config;
use crate::db::{Database, SettingsStore};
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::settings::{CustomLlmProviderSettings, LlmBuiltinOverride, Settings};
use crate::workspace::{Workspace, WorkspaceSettingsAdapter};

#[derive(Debug, Clone, Default)]
pub struct MigrationStats {
    pub workspace_documents: usize,
    pub memory_docs: usize,
    pub engine_threads: usize,
    pub engine_conversations: usize,
    pub legacy_conversations: usize,
    pub messages: usize,
    pub settings: usize,
    pub secrets: usize,
    pub projects: usize,
    pub skipped: usize,
    pub notes: Vec<String>,
}

impl MigrationStats {
    pub fn total_imported(&self) -> usize {
        self.workspace_documents
            + self.memory_docs
            + self.engine_threads
            + self.engine_conversations
            + self.legacy_conversations
            + self.messages
            + self.settings
            + self.secrets
            + self.projects
    }

    pub fn push_note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error("Migration source not found at {path}: {reason}")]
    NotFound { path: PathBuf, reason: String },

    #[error("Config parse error: {0}")]
    ConfigParse(String),

    #[error("SQLite error: {0}")]
    Sqlite(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Workspace error: {0}")]
    Workspace(String),

    #[error("Secrets error: {0}")]
    Secret(String),

    #[error("Engine error: {0}")]
    Engine(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<crate::import::ImportError> for MigrationError {
    fn from(value: crate::import::ImportError) -> Self {
        match value {
            crate::import::ImportError::NotFound { path, reason } => {
                Self::NotFound { path, reason }
            }
            crate::import::ImportError::ConfigParse(reason) => Self::ConfigParse(reason),
            crate::import::ImportError::Sqlite(reason) => Self::Sqlite(reason),
            crate::import::ImportError::Database(reason) => Self::Database(reason),
            crate::import::ImportError::Workspace(reason) => Self::Workspace(reason),
            crate::import::ImportError::Secret(reason) => Self::Secret(reason),
            crate::import::ImportError::Io(err) => Self::Io(err),
            crate::import::ImportError::InvalidUtf8(reason) => Self::ConfigParse(reason),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImportedSecret {
    pub name: String,
    pub value: SecretString,
    pub provider: Option<String>,
}

impl ImportedSecret {
    pub fn new(name: impl Into<String>, value: SecretString) -> Self {
        Self {
            name: name.into(),
            value,
            provider: None,
        }
    }

    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct ImportedDocument {
    pub source: &'static str,
    pub namespace: String,
    pub external_id: String,
    pub workspace_path: String,
    pub title: String,
    pub content: String,
    pub tags: Vec<String>,
    pub doc_type: DocType,
    pub created_at: Option<DateTime<Utc>>,
    pub metadata: Value,
}

#[derive(Debug, Clone)]
pub enum ImportedMessageRole {
    User,
    Assistant,
    System,
    Tool { name: Option<String> },
}

#[derive(Debug, Clone)]
pub struct ImportedMessage {
    pub role: ImportedMessageRole,
    pub content: String,
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ImportedConversation {
    pub source: &'static str,
    pub namespace: String,
    pub external_id: String,
    pub source_channel: String,
    pub title: String,
    pub created_at: Option<DateTime<Utc>>,
    pub messages: Vec<ImportedMessage>,
    pub metadata: Value,
}

pub struct MigrationServices {
    pub user_id: String,
    pub db: Arc<dyn Database>,
    pub workspace: Arc<Workspace>,
    pub settings_store: Arc<dyn SettingsStore + Send + Sync>,
    pub secrets_store: Arc<dyn SecretsStore + Send + Sync>,
    pub engine_store: Arc<HybridStore>,
    pub project_id: ProjectId,
}

impl MigrationServices {
    pub async fn from_config(config: &Config, user_id: String) -> Result<Self, MigrationError> {
        let db = crate::db::connect_from_config(&config.database)
            .await
            .map_err(|e| MigrationError::Database(e.to_string()))?;

        let master_key = config.secrets.master_key().ok_or_else(|| {
            MigrationError::Secret(
                "SECRETS_MASTER_KEY not set. Run 'ironclaw onboard' first or set it in .env"
                    .to_string(),
            )
        })?;
        let crypto = Arc::new(
            crate::secrets::SecretsCrypto::new(master_key.clone())
                .map_err(|e| MigrationError::Secret(e.to_string()))?,
        );
        let secrets_store = crate::db::create_secrets_store(&config.database, crypto)
            .await
            .map_err(|e| MigrationError::Secret(e.to_string()))?;

        let workspace = Arc::new(Workspace::new_with_db(user_id.clone(), Arc::clone(&db)));
        let adapter = Arc::new(WorkspaceSettingsAdapter::new(
            Arc::clone(&workspace),
            Arc::clone(&db),
        ));
        adapter
            .ensure_system_config()
            .await
            .map_err(|e| MigrationError::Database(e.to_string()))?;
        let settings_store: Arc<dyn SettingsStore + Send + Sync> = adapter;

        let engine_store = Arc::new(HybridStore::new(Some(Arc::clone(&workspace))));
        engine_store.load_state_from_workspace().await;
        let store_dyn: Arc<dyn Store> = engine_store.clone();
        let (project_id, _created_project) = resolve_default_project(&store_dyn, &user_id).await?;

        Ok(Self {
            user_id,
            db,
            workspace,
            settings_store,
            secrets_store,
            engine_store,
            project_id,
        })
    }

    pub async fn apply_settings_patch(
        &self,
        patch: HashMap<String, Value>,
        stats: &mut MigrationStats,
    ) -> Result<(), MigrationError> {
        if patch.is_empty() {
            return Ok(());
        }

        let existing_map = self
            .settings_store
            .get_all_settings(&self.user_id)
            .await
            .map_err(|e| MigrationError::Database(e.to_string()))?;
        let existing_settings = Settings::from_db_map(&existing_map);

        let mut merged_patch = patch;

        if let Some(value) = merged_patch.remove("llm_custom_providers") {
            let incoming: Vec<CustomLlmProviderSettings> = serde_json::from_value(value)
                .map_err(|e| MigrationError::Serialization(e.to_string()))?;
            let mut providers = existing_settings.llm_custom_providers;
            for provider in incoming {
                if let Some(existing) = providers.iter_mut().find(|item| item.id == provider.id) {
                    *existing = provider;
                } else {
                    providers.push(provider);
                }
            }
            providers.sort_by(|a, b| a.id.cmp(&b.id));
            merged_patch.insert(
                "llm_custom_providers".to_string(),
                serde_json::to_value(providers)
                    .map_err(|e| MigrationError::Serialization(e.to_string()))?,
            );
        }

        if let Some(value) = merged_patch.remove("llm_builtin_overrides") {
            let incoming: HashMap<String, LlmBuiltinOverride> = serde_json::from_value(value)
                .map_err(|e| MigrationError::Serialization(e.to_string()))?;
            let mut overrides = existing_settings.llm_builtin_overrides;
            for (provider_id, override_value) in incoming {
                overrides.insert(provider_id, override_value);
            }
            merged_patch.insert(
                "llm_builtin_overrides".to_string(),
                serde_json::to_value(overrides)
                    .map_err(|e| MigrationError::Serialization(e.to_string()))?,
            );
        }

        self.settings_store
            .set_all_settings(&self.user_id, &merged_patch)
            .await
            .map_err(|e| MigrationError::Database(e.to_string()))?;
        stats.settings += merged_patch.len();
        Ok(())
    }

    pub async fn store_secret(
        &self,
        secret: ImportedSecret,
        stats: &mut MigrationStats,
    ) -> Result<(), MigrationError> {
        let mut params = CreateSecretParams::new(secret.name, secret.value.expose_secret());
        if let Some(provider) = secret.provider {
            params = params.with_provider(provider);
        }
        self.secrets_store
            .create(&self.user_id, params)
            .await
            .map_err(|e| MigrationError::Secret(e.to_string()))?;
        stats.secrets += 1;
        Ok(())
    }

    pub async fn upsert_document(
        &self,
        doc: ImportedDocument,
        stats: &mut MigrationStats,
    ) -> Result<(), MigrationError> {
        let marker = migration_marker(doc.source, "document", &doc.namespace, &doc.external_id);
        let metadata = merge_metadata(
            &json!({
                "migration": marker,
                "workspace_path": doc.workspace_path,
                "tags": doc.tags,
            }),
            &doc.metadata,
        );

        let doc_id = DocId(deterministic_uuid(
            "memory-doc",
            doc.source,
            &doc.namespace,
            &doc.external_id,
        ));
        let existing_doc = Store::load_memory_doc(self.engine_store.as_ref(), doc_id)
            .await
            .map_err(|e| MigrationError::Engine(e.to_string()))?;
        let existing_workspace = self.workspace.read(&doc.workspace_path).await.ok();

        let now = Utc::now();
        let mut memory_doc = MemoryDoc::new(
            self.project_id,
            self.user_id.clone(),
            doc.doc_type,
            doc.title,
            doc.content.clone(),
        );
        memory_doc.id = doc_id;
        memory_doc.tags = doc.tags;
        memory_doc.metadata = metadata.clone();
        memory_doc.created_at = doc.created_at.unwrap_or(now);
        memory_doc.updated_at = now;
        if let Some(existing) = existing_doc.as_ref() {
            memory_doc.created_at = existing.created_at;
        }

        let workspace_unchanged = existing_workspace
            .as_ref()
            .is_some_and(|value| value.content == doc.content && value.metadata == metadata);
        let doc_unchanged = existing_doc.as_ref().is_some_and(|ed| {
            ed.content == memory_doc.content
                && ed.metadata == memory_doc.metadata
                && ed.title == memory_doc.title
                && ed.tags == memory_doc.tags
        });

        if workspace_unchanged && doc_unchanged {
            stats.skipped += 1;
            return Ok(());
        }

        let workspace_doc = self
            .workspace
            .write(&doc.workspace_path, &doc.content)
            .await
            .map_err(|e| MigrationError::Workspace(e.to_string()))?;
        self.workspace
            .update_metadata(workspace_doc.id, &metadata)
            .await
            .map_err(|e| MigrationError::Workspace(e.to_string()))?;
        Store::save_memory_doc(self.engine_store.as_ref(), &memory_doc)
            .await
            .map_err(|e| MigrationError::Engine(e.to_string()))?;

        stats.workspace_documents += 1;
        stats.memory_docs += 1;
        Ok(())
    }

    pub async fn upsert_conversation(
        &self,
        conversation: ImportedConversation,
        stats: &mut MigrationStats,
    ) -> Result<(), MigrationError> {
        let namespace = if conversation.namespace.is_empty() {
            "default".to_string()
        } else {
            conversation.namespace.clone()
        };
        let base_ts = conversation
            .created_at
            .or_else(|| {
                conversation
                    .messages
                    .iter()
                    .find_map(|message| message.timestamp)
            })
            .unwrap_or_else(Utc::now);
        let last_ts = conversation
            .messages
            .iter()
            .filter_map(|message| message.timestamp)
            .max()
            .unwrap_or(base_ts);

        let thread_id = ThreadId(deterministic_uuid(
            "thread",
            conversation.source,
            &namespace,
            &conversation.external_id,
        ));
        let engine_conv_id = ConversationId(deterministic_uuid(
            "conversation",
            conversation.source,
            &namespace,
            &conversation.external_id,
        ));
        let legacy_conv_id = deterministic_uuid(
            "legacy-conversation",
            conversation.source,
            &namespace,
            &conversation.external_id,
        );
        let synthetic_channel = format!(
            "migrate/{}/{}/{}",
            slugify(conversation.source),
            slugify(&namespace),
            deterministic_uuid(
                "channel",
                conversation.source,
                &namespace,
                &conversation.external_id,
            )
        );

        let marker = migration_marker(
            conversation.source,
            "conversation",
            &namespace,
            &conversation.external_id,
        );
        let metadata = merge_metadata(
            &json!({
                "migration": marker,
                "source_channel": conversation.source_channel,
                "source_title": conversation.title,
                "thread_type": "migration",
            }),
            &conversation.metadata,
        );

        let mut thread = Thread::new(
            conversation.title.clone(),
            ThreadType::Foreground,
            self.project_id,
            self.user_id.clone(),
            ThreadConfig::default(),
        );
        thread.id = thread_id;
        thread.state = ThreadState::Done;
        thread.created_at = base_ts;
        thread.updated_at = last_ts;
        thread.completed_at = Some(last_ts);
        thread.metadata = metadata.clone();
        thread.messages = conversation
            .messages
            .iter()
            .map(imported_message_to_thread_message)
            .collect();

        let mut surface = ConversationSurface::new(synthetic_channel.clone(), self.user_id.clone());
        surface.id = engine_conv_id;
        surface.metadata = metadata.clone();
        surface.created_at = base_ts;
        surface.updated_at = last_ts;
        surface.entries = conversation
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| {
                imported_message_to_entry(
                    thread_id,
                    message,
                    conversation.source,
                    &namespace,
                    &conversation.external_id,
                    index,
                )
            })
            .collect();
        surface.active_threads.clear();

        let existing_thread = Store::load_thread(self.engine_store.as_ref(), thread_id)
            .await
            .map_err(|e| MigrationError::Engine(e.to_string()))?;
        let existing_surface = Store::load_conversation(self.engine_store.as_ref(), engine_conv_id)
            .await
            .map_err(|e| MigrationError::Engine(e.to_string()))?;

        let thread_same = existing_thread
            .as_ref()
            .and_then(|value| serde_json::to_value(value).ok())
            == serde_json::to_value(&thread).ok();
        let surface_same = existing_surface
            .as_ref()
            .and_then(|value| serde_json::to_value(value).ok())
            == serde_json::to_value(&surface).ok();

        if !thread_same {
            Store::save_thread(self.engine_store.as_ref(), &thread)
                .await
                .map_err(|e| MigrationError::Engine(e.to_string()))?;
            stats.engine_threads += 1;
        }
        if !surface_same {
            Store::save_conversation(self.engine_store.as_ref(), &surface)
                .await
                .map_err(|e| MigrationError::Engine(e.to_string()))?;
            stats.engine_conversations += 1;
        }

        let thread_id_text = thread_id.0.to_string();
        self.db
            .ensure_conversation(
                legacy_conv_id,
                &synthetic_channel,
                &self.user_id,
                Some(&thread_id_text),
                Some(&conversation.source_channel),
            )
            .await
            .map_err(|e| MigrationError::Database(e.to_string()))?;

        if let Some(metadata_obj) = metadata.as_object() {
            for (key, value) in metadata_obj {
                self.db
                    .update_conversation_metadata_field(legacy_conv_id, key, value)
                    .await
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
            }
        }

        let expected_len = conversation.messages.len() as i64 + 1;
        let (existing_messages, has_more) = self
            .db
            .list_conversation_messages_paginated(legacy_conv_id, None, expected_len)
            .await
            .map_err(|e| MigrationError::Database(e.to_string()))?;

        if existing_messages.is_empty() {
            for message in &conversation.messages {
                let (role, content) = imported_message_to_legacy_pair(message);
                self.db
                    .add_conversation_message(legacy_conv_id, &role, &content)
                    .await
                    .map_err(|e| MigrationError::Database(e.to_string()))?;
            }
            stats.legacy_conversations += 1;
            stats.messages += conversation.messages.len();
        } else if existing_messages.len() == conversation.messages.len() && !has_more {
            if thread_same && surface_same {
                stats.skipped += 1;
            }
        } else {
            stats.push_note(format!(
                "Legacy conversation {legacy_conv_id} already existed with {} messages; expected {}. Engine V2 state was refreshed, but legacy DB history was left unchanged.",
                existing_messages.len(),
                conversation.messages.len()
            ));
        }

        Ok(())
    }
}

async fn resolve_default_project(
    store: &Arc<dyn Store>,
    user_id: &str,
) -> Result<(ProjectId, bool), MigrationError> {
    let projects = store
        .list_projects(user_id)
        .await
        .map_err(|e| MigrationError::Engine(e.to_string()))?;

    if let Some(project) = projects.iter().find(|project| project.name == "default") {
        return Ok((project.id, false));
    }

    let project = Project::new(user_id, "default", "Default project for migrated data");
    let project_id = project.id;
    store
        .save_project(&project)
        .await
        .map_err(|e| MigrationError::Engine(e.to_string()))?;
    Ok((project_id, true))
}

pub(crate) fn deterministic_uuid(
    kind: &str,
    source: &str,
    namespace: &str,
    external_id: &str,
) -> Uuid {
    let seed = format!("ironclaw-migrate::{kind}::{source}::{namespace}::{external_id}");
    Uuid::new_v5(&Uuid::NAMESPACE_URL, seed.as_bytes())
}

pub(crate) fn normalize_relative_path(path: &str) -> String {
    let mut parts = Vec::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(value) => parts.push(value.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !parts.is_empty() {
                    parts.pop();
                }
            }
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    parts.join("/")
}

pub(crate) fn slugify(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.chars() {
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if matches!(ch, '-' | '_' | '/' | ' ' | '.') && !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "default".to_string()
    } else {
        out
    }
}

pub(crate) fn migration_marker(
    source: &str,
    kind: &str,
    namespace: &str,
    external_id: &str,
) -> Value {
    json!({
        "source": source,
        "kind": kind,
        "namespace": namespace,
        "external_id": external_id,
    })
}

pub(crate) fn merge_metadata(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Object(base_obj), Value::Object(overlay_obj)) => {
            let mut merged: Map<String, Value> = base_obj.clone();
            for (key, value) in overlay_obj {
                let next = merged
                    .get(key)
                    .map(|existing| merge_metadata(existing, value))
                    .unwrap_or_else(|| value.clone());
                merged.insert(key.clone(), next);
            }
            Value::Object(merged)
        }
        (_, overlay_value) if !overlay_value.is_null() => overlay_value.clone(),
        (base_value, _) => base_value.clone(),
    }
}

pub(crate) fn imported_message_to_thread_message(message: &ImportedMessage) -> ThreadMessage {
    let timestamp = message.timestamp.unwrap_or_else(Utc::now);
    match &message.role {
        ImportedMessageRole::User => ThreadMessage {
            role: MessageRole::User,
            content: message.content.clone(),
            provenance: Provenance::User,
            action_call_id: None,
            action_name: None,
            action_calls: None,
            timestamp,
        },
        ImportedMessageRole::Assistant => ThreadMessage {
            role: MessageRole::Assistant,
            content: message.content.clone(),
            provenance: Provenance::LlmGenerated,
            action_call_id: None,
            action_name: None,
            action_calls: None,
            timestamp,
        },
        ImportedMessageRole::System => ThreadMessage {
            role: MessageRole::System,
            content: message.content.clone(),
            provenance: Provenance::System,
            action_call_id: None,
            action_name: None,
            action_calls: None,
            timestamp,
        },
        ImportedMessageRole::Tool { name } => ThreadMessage {
            role: MessageRole::ActionResult,
            content: message.content.clone(),
            provenance: Provenance::ToolOutput {
                action_name: name.clone().unwrap_or_else(|| "imported_tool".to_string()),
            },
            action_call_id: None,
            action_name: name.clone(),
            action_calls: None,
            timestamp,
        },
    }
}

pub(crate) fn imported_message_to_entry(
    thread_id: ThreadId,
    message: &ImportedMessage,
    source: &str,
    namespace: &str,
    external_id: &str,
    index: usize,
) -> ConversationEntry {
    let timestamp = message.timestamp.unwrap_or_else(Utc::now);
    let entry_id = EntryId(deterministic_uuid(
        "entry",
        source,
        namespace,
        &format!("{external_id}:{index}"),
    ));
    match &message.role {
        ImportedMessageRole::User => ConversationEntry {
            id: entry_id,
            sender: EntrySender::User,
            content: message.content.clone(),
            origin_thread_id: None,
            timestamp,
            metadata: Value::Null,
        },
        ImportedMessageRole::Assistant => ConversationEntry {
            id: entry_id,
            sender: EntrySender::Agent { thread_id },
            content: message.content.clone(),
            origin_thread_id: Some(thread_id),
            timestamp,
            metadata: Value::Null,
        },
        ImportedMessageRole::System => ConversationEntry {
            id: entry_id,
            sender: EntrySender::System,
            content: message.content.clone(),
            origin_thread_id: Some(thread_id),
            timestamp,
            metadata: Value::Null,
        },
        ImportedMessageRole::Tool { name } => ConversationEntry {
            id: entry_id,
            sender: EntrySender::System,
            content: match name {
                Some(name) if !name.is_empty() => format!("[tool:{name}] {}", message.content),
                _ => format!("[tool] {}", message.content),
            },
            origin_thread_id: Some(thread_id),
            timestamp,
            metadata: Value::Null,
        },
    }
}

pub(crate) fn imported_message_to_legacy_pair(message: &ImportedMessage) -> (String, String) {
    match &message.role {
        ImportedMessageRole::User => ("user".to_string(), message.content.clone()),
        ImportedMessageRole::Assistant => ("assistant".to_string(), message.content.clone()),
        ImportedMessageRole::System => ("system".to_string(), message.content.clone()),
        ImportedMessageRole::Tool { name } => (
            "assistant".to_string(),
            match name {
                Some(name) if !name.is_empty() => format!("[tool:{name}] {}", message.content),
                _ => format!("[tool] {}", message.content),
            },
        ),
    }
}

pub(crate) fn collect_markdown_files(root: &Path) -> Result<Vec<(String, String)>, MigrationError> {
    let mut results = Vec::new();
    if !root.exists() {
        return Ok(results);
    }
    collect_markdown_files_recursive(root, root, &mut results)?;
    results.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(results)
}

fn collect_markdown_files_recursive(
    root: &Path,
    current: &Path,
    results: &mut Vec<(String, String)>,
) -> Result<(), MigrationError> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files_recursive(root, &path, results)?;
            continue;
        }

        let is_markdown = path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("md"));
        if !is_markdown {
            continue;
        }

        let rel = path
            .strip_prefix(root)
            .map_err(|e| MigrationError::Workspace(e.to_string()))?;
        let rel = normalize_relative_path(&rel.to_string_lossy());
        let content = std::fs::read_to_string(&path)?;
        results.push((rel, content));
    }
    Ok(())
}

pub(crate) fn parse_timestamp_str(raw: &str) -> Option<DateTime<Utc>> {
    if raw.trim().is_empty() {
        return None;
    }

    if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
        return Some(parsed.with_timezone(&Utc));
    }
    if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f") {
        return Some(DateTime::<Utc>::from_naive_utc_and_offset(parsed, Utc));
    }
    if let Ok(parsed) = chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S") {
        return Some(DateTime::<Utc>::from_naive_utc_and_offset(parsed, Utc));
    }
    if let Ok(epoch) = raw.parse::<i64>() {
        return DateTime::<Utc>::from_timestamp(epoch, 0);
    }
    None
}

pub(crate) fn compatible_scalar_settings_patch(
    candidates: &HashMap<String, Value>,
) -> (HashMap<String, Value>, Vec<String>) {
    let mut accepted = HashMap::new();
    let mut ignored = Vec::new();

    for (key, value) in candidates {
        let value_str = match value {
            Value::String(value) => value.clone(),
            Value::Bool(value) => value.to_string(),
            Value::Number(value) => value.to_string(),
            Value::Array(_) | Value::Object(_) => {
                ignored.push(key.clone());
                continue;
            }
            Value::Null => continue,
        };

        let mut probe = Settings::default();
        if probe.set(key, &value_str).is_ok() {
            accepted.insert(key.clone(), value.clone());
        } else {
            ignored.push(key.clone());
        }
    }

    (accepted, ignored)
}

//! Core OpenClaw importer.
//!
//! Orchestrates the 5-phase import pipeline: settings, identity files,
//! memory documents, conversations, and credentials.

use std::io::BufRead;
use std::sync::Arc;

use uuid::Uuid;

use crate::db::{ConversationStore, SettingsStore};
use crate::import::config_parser::OpenClawConfig;
use crate::import::discovery::OpenClawInstallation;
use crate::import::progress::ImportProgress;
use crate::import::{ImportError, ImportReport};
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::workspace::Workspace;

/// Well-known identity files that map directly between OpenClaw and IronClaw.
const DIRECT_IDENTITY_FILES: &[&str] = &[
    "AGENTS.md",
    "SOUL.md",
    "IDENTITY.md",
    "USER.md",
    "HEARTBEAT.md",
    "MEMORY.md",
];

/// Orchestrates importing data from an OpenClaw installation into IronClaw.
pub struct OpenClawImporter {
    installation: OpenClawInstallation,
    config: OpenClawConfig,
    dry_run: bool,
}

impl OpenClawImporter {
    /// Create a new importer for a discovered installation.
    pub fn new(installation: OpenClawInstallation, config: OpenClawConfig, dry_run: bool) -> Self {
        Self {
            installation,
            config,
            dry_run,
        }
    }

    /// Run the full import pipeline.
    pub async fn run(
        &self,
        db: &Arc<dyn crate::db::Database>,
        workspace: Option<&Workspace>,
        secrets_store: Option<&Arc<dyn SecretsStore + Send + Sync>>,
        progress: &mut dyn ImportProgress,
    ) -> Result<ImportReport, ImportError> {
        let mut report = ImportReport {
            dry_run: self.dry_run,
            ..Default::default()
        };

        self.phase_settings(db.as_ref(), progress, &mut report)
            .await;
        self.phase_identity_files(workspace, progress, &mut report)
            .await;
        self.phase_memory_documents(workspace, progress, &mut report)
            .await;
        self.phase_conversations(db.as_ref(), progress, &mut report)
            .await;
        self.phase_credentials(secrets_store, progress, &mut report)
            .await;

        Ok(report)
    }

    /// Phase 1: Import settings.
    async fn phase_settings(
        &self,
        db: &dyn SettingsStore,
        progress: &mut dyn ImportProgress,
        report: &mut ImportReport,
    ) {
        let settings = &self.config.mapped_settings;
        progress.start_phase("settings", settings.len());

        for (key, value) in settings {
            match db.get_setting("default", key).await {
                Ok(Some(_)) => {
                    progress.item_skipped(key, "already exists");
                    report.skipped_already_exists += 1;
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    let msg = format!("Failed to check setting '{}': {}", key, e);
                    progress.item_error(key, &msg);
                    report.errors.push(msg);
                    continue;
                }
            }

            if self.dry_run {
                progress.item_imported(key);
                report.settings_count += 1;
                continue;
            }

            match db.set_setting("default", key, value).await {
                Ok(()) => {
                    progress.item_imported(key);
                    report.settings_count += 1;
                }
                Err(e) => {
                    let msg = format!("Failed to write setting '{}': {}", key, e);
                    progress.item_error(key, &msg);
                    report.errors.push(msg);
                }
            }
        }

        progress.end_phase();
    }

    /// Phase 2: Import identity files.
    async fn phase_identity_files(
        &self,
        workspace: Option<&Workspace>,
        progress: &mut dyn ImportProgress,
        report: &mut ImportReport,
    ) {
        progress.start_phase("identity files", self.installation.identity_files.len());

        let Some(ws) = workspace else {
            progress.end_phase();
            return;
        };

        for (filename, path) in &self.installation.identity_files {
            // Only import files that have direct IronClaw equivalents
            let ironclaw_path = if DIRECT_IDENTITY_FILES.contains(&filename.as_str()) {
                filename.to_string()
            } else {
                // TOOLS.md, BOOTSTRAP.md etc -> import as custom paths
                format!("imported/{}", filename)
            };

            // Check if already exists with content
            match ws.exists(&ironclaw_path).await {
                Ok(true) => {
                    // Check if the existing file has non-empty content
                    if let Ok(doc) = ws.read(&ironclaw_path).await
                        && !doc.content.is_empty()
                    {
                        progress.item_skipped(filename, "already exists with content");
                        report.skipped_already_exists += 1;
                        continue;
                    }
                }
                Ok(false) => {}
                Err(e) => {
                    let msg = format!("Failed to check '{}': {}", ironclaw_path, e);
                    progress.item_error(filename, &msg);
                    report.errors.push(msg);
                    continue;
                }
            }

            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    let msg = format!("Failed to read '{}': {}", path.display(), e);
                    progress.item_error(filename, &msg);
                    report.errors.push(msg);
                    continue;
                }
            };

            if content.trim().is_empty() {
                progress.item_skipped(filename, "empty file");
                continue;
            }

            if self.dry_run {
                progress.item_imported(filename);
                report.identity_files += 1;
                continue;
            }

            match ws.write(&ironclaw_path, &content).await {
                Ok(_) => {
                    progress.item_imported(filename);
                    report.identity_files += 1;
                }
                Err(e) => {
                    let msg = format!("Failed to write '{}': {}", ironclaw_path, e);
                    progress.item_error(filename, &msg);
                    report.errors.push(msg);
                }
            }
        }

        progress.end_phase();
    }

    /// Phase 3: Import memory documents from filesystem.
    async fn phase_memory_documents(
        &self,
        workspace: Option<&Workspace>,
        progress: &mut dyn ImportProgress,
        report: &mut ImportReport,
    ) {
        progress.start_phase("memory documents", self.installation.memory_files.len());

        let Some(ws) = workspace else {
            progress.end_phase();
            return;
        };

        for path in &self.installation.memory_files {
            let filename = match path.file_name() {
                Some(f) => f.to_string_lossy().to_string(),
                None => continue,
            };

            let ironclaw_path = format!("memory/{}", filename);

            match ws.exists(&ironclaw_path).await {
                Ok(true) => {
                    progress.item_skipped(&filename, "already exists");
                    report.skipped_already_exists += 1;
                    continue;
                }
                Ok(false) => {}
                Err(e) => {
                    let msg = format!("Failed to check '{}': {}", ironclaw_path, e);
                    progress.item_error(&filename, &msg);
                    report.errors.push(msg);
                    continue;
                }
            }

            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    let msg = format!("Failed to read '{}': {}", path.display(), e);
                    progress.item_error(&filename, &msg);
                    report.errors.push(msg);
                    continue;
                }
            };

            if content.trim().is_empty() {
                progress.item_skipped(&filename, "empty file");
                continue;
            }

            if self.dry_run {
                progress.item_imported(&filename);
                report.memory_documents += 1;
                continue;
            }

            match ws.write(&ironclaw_path, &content).await {
                Ok(_) => {
                    progress.item_imported(&filename);
                    report.memory_documents += 1;
                }
                Err(e) => {
                    let msg = format!("Failed to write '{}': {}", ironclaw_path, e);
                    progress.item_error(&filename, &msg);
                    report.errors.push(msg);
                }
            }
        }

        progress.end_phase();
    }

    /// Phase 4: Import conversations from JSONL session files.
    async fn phase_conversations(
        &self,
        db: &(dyn ConversationStore + Sync),
        progress: &mut dyn ImportProgress,
        report: &mut ImportReport,
    ) {
        let total_files: usize = self
            .installation
            .session_dirs
            .iter()
            .map(|s| s.jsonl_files.len())
            .sum();
        progress.start_phase("conversations", total_files);

        for session_dir in &self.installation.session_dirs {
            for jsonl_path in &session_dir.jsonl_files {
                let session_key = jsonl_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();

                // Skip sessions.json (metadata, not a transcript)
                if session_key == "sessions" {
                    continue;
                }

                let label = format!("{}:{}", session_dir.agent_id, session_key);

                // Use a deterministic conversation ID from the session key for idempotency.
                let conv_id = deterministic_uuid(&label);

                // Check idempotency: if this conversation already has messages, skip.
                match db.list_conversation_messages(conv_id).await {
                    Ok(msgs) if !msgs.is_empty() => {
                        progress.item_skipped(&label, "already imported");
                        report.skipped_already_exists += 1;
                        continue;
                    }
                    _ => {}
                }

                // Parse JSONL file
                let messages = match Self::parse_jsonl(jsonl_path) {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        let msg = format!("Failed to parse '{}': {}", jsonl_path.display(), e);
                        progress.item_error(&label, &msg);
                        report.errors.push(msg);
                        continue;
                    }
                };

                if messages.is_empty() {
                    progress.item_skipped(&label, "no messages");
                    continue;
                }

                if self.dry_run {
                    progress.item_imported(&format!("{} ({} messages)", label, messages.len()));
                    report.conversations += 1;
                    report.messages += messages.len();
                    continue;
                }

                // Create conversation with deterministic ID using ensure_conversation
                let thread_id = format!("openclaw:{}:{}", session_dir.agent_id, session_key);
                if let Err(e) = db
                    .ensure_conversation(conv_id, "openclaw_import", "default", Some(&thread_id))
                    .await
                {
                    let msg = format!("Failed to create conversation for '{}': {}", label, e);
                    progress.item_error(&label, &msg);
                    report.errors.push(msg);
                    continue;
                }

                // Import messages
                let mut msg_count = 0;
                for (role, content) in &messages {
                    match db.add_conversation_message(conv_id, role, content).await {
                        Ok(_) => msg_count += 1,
                        Err(e) => {
                            let msg = format!("Failed to add message to '{}': {}", label, e);
                            report.errors.push(msg);
                        }
                    }
                }

                progress.item_imported(&format!("{} ({} messages)", label, msg_count));
                report.conversations += 1;
                report.messages += msg_count;
            }
        }

        progress.end_phase();
    }

    /// Phase 5: Import credentials.
    async fn phase_credentials(
        &self,
        secrets_store: Option<&Arc<dyn SecretsStore + Send + Sync>>,
        progress: &mut dyn ImportProgress,
        report: &mut ImportReport,
    ) {
        let creds = &self.config.credentials;
        progress.start_phase("credentials", creds.len());

        let Some(store) = secrets_store else {
            if !creds.is_empty() {
                progress.item_skipped("all", "no secrets store available");
            }
            progress.end_phase();
            return;
        };

        for cred in creds {
            match store.exists("default", &cred.name).await {
                Ok(true) => {
                    progress.item_skipped(&cred.name, "already exists");
                    report.skipped_already_exists += 1;
                    continue;
                }
                Ok(false) => {}
                Err(e) => {
                    let msg = format!("Failed to check credential '{}': {}", cred.name, e);
                    progress.item_error(&cred.name, &msg);
                    report.errors.push(msg);
                    continue;
                }
            }

            if self.dry_run {
                progress.item_imported(&cred.name);
                report.credentials += 1;
                continue;
            }

            let mut params = CreateSecretParams::new(&cred.name, &cred.value);
            if let Some(ref provider) = cred.provider {
                params = params.with_provider(provider);
            }

            match store.create("default", params).await {
                Ok(_) => {
                    progress.item_imported(&cred.name);
                    report.credentials += 1;
                }
                Err(e) => {
                    let msg = format!("Failed to store credential '{}': {}", cred.name, e);
                    progress.item_error(&cred.name, &msg);
                    report.errors.push(msg);
                }
            }
        }

        progress.end_phase();
    }

    /// Parse a JSONL session file into `(role, content)` pairs.
    fn parse_jsonl(path: &std::path::Path) -> Result<Vec<(String, String)>, ImportError> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let mut messages = Vec::new();

        for (line_num, line) in reader.lines().enumerate() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let val: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue, // Skip unparseable lines
            };

            // Filter for message records
            if val.get("type").and_then(|v| v.as_str()) != Some("message") {
                continue;
            }

            let Some(msg) = val.get("message") else {
                continue;
            };

            let role = msg
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("user")
                .to_string();

            let content = extract_content(msg);
            if content.is_empty() {
                continue;
            }

            messages.push((role, content));

            // Safety limit: don't import absurdly large sessions
            if messages.len() > 10_000 {
                tracing::warn!(
                    "Session file {} truncated at 10,000 messages (line {})",
                    path.display(),
                    line_num + 1
                );
                break;
            }
        }

        Ok(messages)
    }
}

/// Extract text content from a message, handling both string and array formats.
///
/// OpenClaw messages use either:
/// - `"content": "Hello"` (plain string)
/// - `"content": [{"type": "text", "text": "Hello"}]` (structured array)
fn extract_content(msg: &serde_json::Value) -> String {
    let Some(content) = msg.get("content") else {
        return String::new();
    };

    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => {
            let mut parts = Vec::new();
            for item in arr {
                if item.get("type").and_then(|v| v.as_str()) == Some("text")
                    && let Some(text) = item.get("text").and_then(|v| v.as_str())
                {
                    parts.push(text.to_string());
                }
            }
            parts.join("\n")
        }
        _ => String::new(),
    }
}

/// Generate a deterministic UUID from a string key using blake3.
fn deterministic_uuid(key: &str) -> Uuid {
    let hash = blake3::hash(key.as_bytes());
    let bytes = hash.as_bytes();
    // Take the first 16 bytes and construct a UUID v4-like value.
    let mut uuid_bytes = [0u8; 16];
    uuid_bytes.copy_from_slice(&bytes[..16]);
    // Set version (4) and variant (RFC 4122) bits for valid UUID format
    uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x40; // version 4
    uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80; // variant RFC 4122
    Uuid::from_bytes(uuid_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_content_string() {
        let msg = serde_json::json!({"content": "Hello world"});
        assert_eq!(extract_content(&msg), "Hello world");
    }

    #[test]
    fn test_extract_content_array() {
        let msg = serde_json::json!({
            "content": [
                {"type": "text", "text": "Hello"},
                {"type": "text", "text": "World"}
            ]
        });
        assert_eq!(extract_content(&msg), "Hello\nWorld");
    }

    #[test]
    fn test_extract_content_empty() {
        let msg = serde_json::json!({"role": "user"});
        assert_eq!(extract_content(&msg), "");
    }

    #[test]
    fn test_extract_content_mixed_array() {
        let msg = serde_json::json!({
            "content": [
                {"type": "text", "text": "Hello"},
                {"type": "image", "url": "https://example.com/img.png"},
                {"type": "text", "text": "World"}
            ]
        });
        assert_eq!(extract_content(&msg), "Hello\nWorld");
    }

    #[test]
    fn test_deterministic_uuid_is_stable() {
        let a = deterministic_uuid("test_key");
        let b = deterministic_uuid("test_key");
        assert_eq!(a, b);
    }

    #[test]
    fn test_deterministic_uuid_differs_for_different_keys() {
        let a = deterministic_uuid("key_a");
        let b = deterministic_uuid("key_b");
        assert_ne!(a, b);
    }

    #[test]
    fn test_parse_jsonl_various_formats() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("test.jsonl");

        std::fs::write(
            &path,
            r#"{"type": "message", "message": {"role": "user", "content": "Hello"}}
{"type": "message", "message": {"role": "assistant", "content": [{"type": "text", "text": "Hi there"}]}}
{"type": "tool_call", "data": {}}
{"type": "message", "message": {"role": "user", "content": ""}}
invalid json line
{"type": "message", "message": {"role": "user", "content": "Goodbye"}}
"#,
        )
        .unwrap();

        let messages = OpenClawImporter::parse_jsonl(&path).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0], ("user".to_string(), "Hello".to_string()));
        assert_eq!(
            messages[1],
            ("assistant".to_string(), "Hi there".to_string())
        );
        assert_eq!(messages[2], ("user".to_string(), "Goodbye".to_string()));
    }

    #[test]
    fn test_parse_jsonl_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("empty.jsonl");
        std::fs::write(&path, "").unwrap();

        let messages = OpenClawImporter::parse_jsonl(&path).unwrap();
        assert!(messages.is_empty());
    }
}

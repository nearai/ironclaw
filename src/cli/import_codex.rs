//! OpenAI Codex CLI local history importer.
//!
//! Parses Codex CLI session JSONL logs from `~/.codex/sessions` (or
//! `~/.config/codex/sessions`) into imported conversations.

use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::cli::import::{
    ImportError, ImportedConversation, ImportedMessage, Importer, parse_timestamp, truncate_chars,
};

/// Importer for local Codex CLI JSONL history.
pub struct CodexCliImporter;

// Guardrails for malformed or hostile local history layouts.
const MAX_SESSION_FILES: usize = 100_000;
const MAX_SESSION_FILE_BYTES: u64 = 256 * 1024 * 1024; // 256 MiB
const MAX_JSONL_LINE_BYTES: usize = 4 * 1024 * 1024; // 4 MiB per line
const MAX_MESSAGES_PER_CONVERSATION: usize = 100_000;
const MAX_MESSAGE_TEXT_CHARS: usize = 400_000;
const MAX_CONTENT_ITEMS: usize = 20_000;

#[derive(Debug)]
struct PendingMessage {
    role: String,
    content: String,
    created_at: Option<DateTime<Utc>>,
}

impl Importer for CodexCliImporter {
    fn source_name(&self) -> &str {
        "Codex CLI"
    }

    fn parse(&self, path: &Path) -> Result<Vec<ImportedConversation>, ImportError> {
        let resolved = resolve_codex_history_path(path);
        if !resolved.exists() {
            return Ok(Vec::new());
        }

        if resolved.is_file() {
            if is_jsonl_file(&resolved)
                && let Some(conversation) = parse_session_file(&resolved, resolved.parent())?
            {
                return Ok(vec![conversation]);
            }
            return Ok(Vec::new());
        }

        if !resolved.is_dir() {
            return Ok(Vec::new());
        }

        let mut files = collect_session_files(&resolved)?;
        files.sort();

        let mut conversations = Vec::new();
        for file_path in files {
            if let Some(conversation) = parse_session_file(&file_path, Some(&resolved))? {
                conversations.push(conversation);
            }
        }

        Ok(conversations)
    }
}

fn collect_session_files(root: &Path) -> Result<Vec<PathBuf>, ImportError> {
    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if dir == root => return Err(ImportError::Io(error)),
            Err(error) => {
                tracing::warn!(
                    "Skipping unreadable Codex directory {}: {}",
                    dir.display(),
                    error
                );
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    tracing::warn!("Skipping unreadable Codex path entry: {}", error);
                    continue;
                }
            };

            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    tracing::warn!(
                        "Skipping Codex path with unreadable type {}: {}",
                        path.display(),
                        error
                    );
                    continue;
                }
            };

            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() || !is_jsonl_file(&path) {
                continue;
            }

            files.push(path);
            if files.len() > MAX_SESSION_FILES {
                return Err(ImportError::Parse {
                    reason: format!(
                        "Too many Codex session JSONL files (>{})",
                        MAX_SESSION_FILES
                    ),
                });
            }
        }
    }

    Ok(files)
}

fn parse_session_file(
    file_path: &Path,
    root: Option<&Path>,
) -> Result<Option<ImportedConversation>, ImportError> {
    let file_size = match fs::metadata(file_path) {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            tracing::warn!(
                "Skipping Codex session with unreadable metadata {}: {}",
                file_path.display(),
                error
            );
            return Ok(None);
        }
    };
    if file_size > MAX_SESSION_FILE_BYTES {
        tracing::warn!(
            "Skipping oversized Codex session file {} ({} bytes; max {})",
            file_path.display(),
            file_size,
            MAX_SESSION_FILE_BYTES
        );
        return Ok(None);
    }

    let file = File::open(file_path)?;
    let mut reader = BufReader::new(file);

    let relative_path = root
        .and_then(|base| {
            file_path
                .strip_prefix(base)
                .ok()
                .map(|value| value.to_string_lossy().replace('\\', "/"))
        })
        .unwrap_or_else(|| file_path.to_string_lossy().to_string());

    let mut malformed_lines = 0_usize;
    let mut empty_messages = 0_usize;
    let mut line_num = 0_usize;
    let mut pending_messages = Vec::new();
    let mut line_bytes = Vec::new();
    let mut first_session_meta: Option<CodexSessionMeta> = None;
    let mut content_types_seen = HashSet::new();

    loop {
        line_bytes.clear();
        let read = reader.read_until(b'\n', &mut line_bytes)?;
        if read == 0 {
            break;
        }
        line_num += 1;

        if read > MAX_JSONL_LINE_BYTES {
            malformed_lines += 1;
            tracing::warn!(
                "Skipping oversized line {} in {} ({} bytes; max {})",
                line_num,
                file_path.display(),
                read,
                MAX_JSONL_LINE_BYTES
            );
            continue;
        }

        if line_bytes.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }

        if pending_messages.len() >= MAX_MESSAGES_PER_CONVERSATION {
            tracing::warn!(
                "Reached message cap ({}) for {}; skipping remaining lines",
                MAX_MESSAGES_PER_CONVERSATION,
                file_path.display()
            );
            break;
        }

        let line = match std::str::from_utf8(&line_bytes) {
            Ok(value) => value,
            Err(error) => {
                malformed_lines += 1;
                tracing::warn!(
                    "Skipping non-UTF8 JSONL line {} in {}: {}",
                    line_num,
                    file_path.display(),
                    error
                );
                continue;
            }
        };

        let raw: serde_json::Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(error) => {
                malformed_lines += 1;
                tracing::warn!(
                    "Skipping malformed JSON line {} in {}: {}",
                    line_num,
                    file_path.display(),
                    error
                );
                continue;
            }
        };

        let entry_type = value_string(&raw, "type");
        let timestamp_raw = value_string(&raw, "timestamp");

        if entry_type == "session_meta" {
            let payload = raw.get("payload").unwrap_or(&serde_json::Value::Null);
            if first_session_meta.is_none() {
                first_session_meta = Some(CodexSessionMeta {
                    id: value_string(payload, "id"),
                    timestamp: value_string(payload, "timestamp"),
                    cwd: value_string(payload, "cwd"),
                    originator: value_string(payload, "originator"),
                    cli_version: value_string(payload, "cli_version"),
                    source: value_string(payload, "source"),
                    model_provider: value_string(payload, "model_provider"),
                });
            }
            continue;
        }

        if entry_type != "response_item" {
            continue;
        }

        let payload = raw.get("payload").unwrap_or(&serde_json::Value::Null);
        if value_string(payload, "type") != "message" {
            continue;
        }

        let role = value_string(payload, "role").to_ascii_lowercase();
        if !is_supported_role(role.as_str()) {
            continue;
        }

        let content = payload.get("content").unwrap_or(&serde_json::Value::Null);
        for content_type in content_types(content) {
            content_types_seen.insert(content_type);
        }

        let text = clamp_message_text(content_to_text(content), file_path, line_num);
        if text.trim().is_empty() {
            empty_messages += 1;
            continue;
        }

        pending_messages.push(PendingMessage {
            role,
            content: text,
            created_at: parse_timestamp(timestamp_raw.as_str()),
        });
    }

    if pending_messages.is_empty() {
        return Ok(None);
    }

    let title = pending_messages
        .iter()
        .find(|message| message.role == "user" && !message.content.trim().is_empty())
        .map(|message| truncate_chars(message.content.trim(), 100))
        .filter(|value| !value.is_empty());

    let session_meta = first_session_meta.unwrap_or_default();
    let session_timestamp = parse_timestamp(session_meta.timestamp.as_str());
    let created_at = pending_messages
        .iter()
        .filter_map(|message| message.created_at.as_ref().cloned())
        .min()
        .or(session_timestamp.clone())
        .unwrap_or_else(unix_epoch);
    let last_activity = pending_messages
        .iter()
        .filter_map(|message| message.created_at.as_ref().cloned())
        .max()
        .or(session_timestamp)
        .unwrap_or_else(|| created_at.clone());

    let messages = pending_messages
        .into_iter()
        .map(|message| ImportedMessage {
            role: message.role,
            content: message.content,
            created_at: message.created_at.unwrap_or_else(|| created_at.clone()),
        })
        .collect();

    let source_id = if !session_meta.id.trim().is_empty() {
        session_meta.id.clone()
    } else {
        derive_fallback_source_id(file_path, root)
    };

    let mut content_types = content_types_seen.into_iter().collect::<Vec<_>>();
    content_types.sort();

    Ok(Some(ImportedConversation {
        source_id,
        title,
        messages,
        created_at,
        last_activity,
        source_metadata: serde_json::json!({
            "relative_path": relative_path,
            "session_meta": {
                "id": session_meta.id,
                "timestamp": session_meta.timestamp,
                "cwd": session_meta.cwd,
                "originator": session_meta.originator,
                "cli_version": session_meta.cli_version,
                "source": session_meta.source,
                "model_provider": session_meta.model_provider,
            },
            "line_count": line_num,
            "malformed_lines": malformed_lines,
            "skipped_empty_messages": empty_messages,
            "content_types": content_types,
        }),
    }))
}

fn resolve_codex_history_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        let sessions = path.join("sessions");
        if sessions.is_dir() {
            return sessions;
        }
    }

    path.to_path_buf()
}

fn derive_fallback_source_id(file_path: &Path, root: Option<&Path>) -> String {
    if let Some(base) = root
        && let Ok(relative) = file_path.strip_prefix(base)
    {
        let relative = relative.to_string_lossy().replace('\\', "/");
        if !relative.trim().is_empty() {
            return relative;
        }
    }

    file_path
        .file_stem()
        .map(|value| value.to_string_lossy().to_string())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "codex-session".to_string())
}

fn is_jsonl_file(path: &Path) -> bool {
    path.extension().and_then(|value| value.to_str()) == Some("jsonl")
}

fn is_supported_role(role: &str) -> bool {
    matches!(role, "user" | "assistant" | "system" | "tool")
}

fn content_to_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                String::new()
            } else {
                trimmed.to_string()
            }
        }
        serde_json::Value::Array(items) => {
            if items.len() > MAX_CONTENT_ITEMS {
                tracing::warn!(
                    "Codex message content has {} item(s); reading first {}",
                    items.len(),
                    MAX_CONTENT_ITEMS
                );
            }

            let mut pieces = Vec::new();
            for item in items.iter().take(MAX_CONTENT_ITEMS) {
                if let Some(text) = text_from_content_item(item) {
                    pieces.push(text);
                }
            }

            pieces.join("\n\n")
        }
        serde_json::Value::Object(_) => text_from_content_item(value).unwrap_or_default(),
        _ => String::new(),
    }
}

fn text_from_content_item(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        serde_json::Value::Object(map) => {
            for key in ["text", "input_text", "output_text"] {
                if let Some(text) = map.get(key).and_then(|value| value.as_str()) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        return Some(trimmed.to_string());
                    }
                }
            }

            if let Some(inner) = map.get("content") {
                let inner_text = content_to_text(inner);
                if !inner_text.trim().is_empty() {
                    return Some(inner_text);
                }
            }

            let kind = map
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if kind.contains("image") {
                return Some("[image]".to_string());
            }

            None
        }
        serde_json::Value::Array(items) => {
            let mut out = Vec::new();
            for item in items.iter().take(MAX_CONTENT_ITEMS) {
                if let Some(text) = text_from_content_item(item) {
                    out.push(text);
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out.join("\n\n"))
            }
        }
        _ => None,
    }
}

fn content_types(content: &serde_json::Value) -> Vec<String> {
    let Some(items) = content.as_array() else {
        return Vec::new();
    };

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items.iter().take(MAX_CONTENT_ITEMS) {
        let Some(kind) = item.get("type").and_then(|value| value.as_str()) else {
            continue;
        };

        let kind_string = kind.to_string();
        if seen.insert(kind_string.clone()) {
            out.push(kind_string);
        }
    }

    out
}

fn value_string(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn clamp_message_text(text: String, file_path: &Path, line_num: usize) -> String {
    if text.chars().count() <= MAX_MESSAGE_TEXT_CHARS {
        text
    } else {
        tracing::warn!(
            "Truncating oversized Codex message text at {}:{} to {} chars",
            file_path.display(),
            line_num,
            MAX_MESSAGE_TEXT_CHARS
        );
        truncate_chars(&text, MAX_MESSAGE_TEXT_CHARS)
    }
}

fn unix_epoch() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_else(Utc::now)
}

#[derive(Debug, Default, Clone)]
struct CodexSessionMeta {
    id: String,
    timestamp: String,
    cwd: String,
    originator: String,
    cli_version: String,
    source: String,
    model_provider: String,
}

pub fn default_codex_cli_path() -> PathBuf {
    if let Ok(custom_home) = std::env::var("CODEX_HOME") {
        let candidate = PathBuf::from(custom_home).join("sessions");
        if candidate.is_dir() {
            return candidate;
        }
    }

    if let Some(home) = dirs::home_dir() {
        let primary = home.join(".codex").join("sessions");
        if primary.is_dir() {
            return primary;
        }

        let fallback = home.join(".config").join("codex").join("sessions");
        if fallback.is_dir() {
            return fallback;
        }

        return primary;
    }

    PathBuf::from(".codex/sessions")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use crate::cli::import::Importer;

    use super::CodexCliImporter;

    #[test]
    fn parses_codex_sessions_and_extracts_messages() {
        let temp = tempdir().expect("tempdir");
        let codex_root = temp.path().join(".codex");
        let sessions_dir = codex_root
            .join("sessions")
            .join("2026")
            .join("02")
            .join("23");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");

        let session_path = sessions_dir.join("rollout-1.jsonl");
        write_lines(
            &session_path,
            &[
                r#"{"timestamp":"2026-02-23T00:00:00.000Z","type":"session_meta","payload":{"id":"session-123","cwd":"/repo","cli_version":"0.99.0","source":"cli"}}"#,
                r#"{"timestamp":"2026-02-23T00:00:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}"#,
                r#"{"timestamp":"2026-02-23T00:00:02.000Z","type":"response_item","payload":{"type":"function_call","name":"shell_command"}}"#,
                r#"{"timestamp":"2026-02-23T00:00:03.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"world"}]}}"#,
            ],
        );

        let importer = CodexCliImporter;
        let conversations = importer.parse(&codex_root).expect("parse codex sessions");

        assert_eq!(conversations.len(), 1);
        let conversation = &conversations[0];
        assert_eq!(conversation.source_id, "session-123");
        assert_eq!(conversation.messages.len(), 2);
        assert_eq!(conversation.messages[0].role, "user");
        assert_eq!(conversation.messages[0].content, "hello");
        assert_eq!(conversation.messages[1].role, "assistant");
        assert_eq!(conversation.messages[1].content, "world");
    }

    #[test]
    fn returns_empty_when_history_path_does_not_exist() {
        let temp = tempdir().expect("tempdir");
        let missing = temp.path().join("missing").join("sessions");

        let importer = CodexCliImporter;
        let conversations = importer.parse(&missing).expect("parse should not fail");

        assert!(conversations.is_empty());
    }

    #[test]
    fn returns_empty_for_empty_sessions_directory() {
        let temp = tempdir().expect("tempdir");
        let sessions = temp.path().join("sessions");
        fs::create_dir_all(&sessions).expect("create sessions");

        let importer = CodexCliImporter;
        let conversations = importer.parse(&sessions).expect("parse empty directory");

        assert!(conversations.is_empty());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn roundtrips_codex_sessions_into_db_without_losing_messages_or_workspace_docs() {
        let temp = tempdir().expect("tempdir");
        let codex_root = temp.path().join(".codex");
        let sessions_dir = codex_root
            .join("sessions")
            .join("2026")
            .join("02")
            .join("23");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");

        let first_session = sessions_dir.join("rollout-1.jsonl");
        write_lines(
            &first_session,
            &[
                r#"{"timestamp":"2026-02-23T00:00:00.000Z","type":"session_meta","payload":{"id":"session-123","cwd":"/repo","cli_version":"0.99.0","source":"cli"}}"#,
                r#"{"timestamp":"2026-02-23T00:00:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}"#,
                r#"{"timestamp":"2026-02-23T00:00:03.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"world"}]}}"#,
            ],
        );

        let second_session = sessions_dir.join("rollout-2.jsonl");
        write_lines(
            &second_session,
            &[
                r#"{"timestamp":"2026-02-24T00:00:00.000Z","type":"session_meta","payload":{"id":"session-456","cwd":"/repo","cli_version":"0.99.0","source":"cli"}}"#,
                r#"{"timestamp":"2026-02-24T00:00:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"repeat title"}]}}"#,
                r#"{"timestamp":"2026-02-24T00:00:02.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"reply"}]}}"#,
            ],
        );

        let importer = CodexCliImporter;
        let expected = importer.parse(&codex_root).expect("parse codex sessions");
        let expected_count = expected.len();

        let (db, _tmp) = crate::testing::test_db().await;
        let user_id = "codex-roundtrip";
        let args = crate::cli::import::HistoryImportArgs {
            path: Some(codex_root.clone()),
            user_id: user_id.to_string(),
            dedup: true,
            to_workspace: true,
            dry_run: false,
        };

        crate::cli::import::run_import_command_with_db(
            crate::cli::import::ImportSource::CodexCli,
            &args,
            db.clone(),
        )
        .await
        .expect("import");

        let workspace = crate::workspace::Workspace::new_with_db(user_id, db.clone());
        let workspace_paths = workspace.list_all().await.expect("list workspace");
        assert_eq!(workspace_paths.len(), expected_count);

        for conversation in &expected {
            let conversation_id = db
                .find_conversation_by_import_source(
                    user_id,
                    crate::cli::import::ImportSource::CodexCli.source_key(),
                    &conversation.source_id,
                )
                .await
                .expect("find conversation")
                .expect("conversation exists");

            let stored = db
                .list_conversation_messages(conversation_id)
                .await
                .expect("list messages");
            assert_eq!(stored.len(), conversation.messages.len());

            for (stored, expected) in stored.iter().zip(&conversation.messages) {
                assert_eq!(stored.role, expected.role);
                assert_eq!(stored.content, expected.content);
            }
        }

        crate::cli::import::run_import_command_with_db(
            crate::cli::import::ImportSource::CodexCli,
            &args,
            db.clone(),
        )
        .await
        .expect("dedup reimport");

        let summaries = db
            .list_conversations_all_channels(user_id, 100)
            .await
            .expect("list conversations");
        assert_eq!(summaries.len(), expected_count);
    }

    fn write_lines(path: &Path, lines: &[&str]) {
        let payload = lines.join("\n");
        fs::write(path, payload).expect("write jsonl");
    }
}

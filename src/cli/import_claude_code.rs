//! Claude Code local history importer.
//!
//! Parses `~/.claude/projects/**.jsonl` files into imported conversations.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::cli::import::{
    ImportError, ImportParseSummary, ImportedConversation, ImportedMessage, Importer,
    parse_timestamp, truncate_chars,
};

/// Importer for local Claude Code JSONL history.
pub struct ClaudeCodeImporter;

// Guardrails for malformed or hostile local history layouts.
const MAX_JSONL_FILES: usize = 50_000;
const MAX_JSONL_FILE_BYTES: u64 = 256 * 1024 * 1024; // 256 MiB
const MAX_JSONL_LINE_BYTES: usize = 4 * 1024 * 1024; // 4 MiB per JSON line
const MAX_MESSAGES_PER_CONVERSATION: usize = 100_000;
const MAX_MESSAGE_TEXT_CHARS: usize = 400_000;
const MAX_CONTENT_BLOCKS: usize = 20_000;
const MAX_PROJECTS_INDEX_BYTES: u64 = 20 * 1024 * 1024; // 20 MiB
const MAX_PROJECT_NAME_CHARS: usize = 200;

impl Importer for ClaudeCodeImporter {
    fn source_key(&self) -> &str {
        "claude_code"
    }

    fn source_name(&self) -> &str {
        "Claude Code"
    }

    fn parse_stream<F>(
        &self,
        path: &Path,
        mut on_conversation: F,
    ) -> Result<ImportParseSummary, ImportError>
    where
        F: FnMut(ImportedConversation) -> Result<(), ImportError>,
    {
        if !path.exists() {
            return Err(ImportError::Parse(format!(
                "Path does not exist: {}",
                path.display()
            )));
        }
        if !path.is_dir() {
            return Err(ImportError::Parse(format!(
                "Expected directory for Claude Code history: {}",
                path.display()
            )));
        }

        let project_names = load_project_index(path);
        let mut summary = ImportParseSummary::default();
        let mut jsonl_files = 0_usize;

        for entry in WalkDir::new(path)
            .into_iter()
            .filter_map(|entry| match entry {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!("Skipping unreadable path entry: {}", e);
                    None
                }
            })
        {
            let file_path = entry.path();
            if !entry.file_type().is_file()
                || file_path.extension().is_none_or(|ext| ext != "jsonl")
            {
                continue;
            }

            jsonl_files += 1;
            if jsonl_files > MAX_JSONL_FILES {
                return Err(ImportError::Parse(format!(
                    "Too many JSONL files in Claude Code history (>{})",
                    MAX_JSONL_FILES
                )));
            }

            let file_size = match entry.metadata() {
                Ok(metadata) => metadata.len(),
                Err(e) => {
                    tracing::warn!(
                        "Skipping JSONL file with unreadable metadata {}: {}",
                        file_path.display(),
                        e
                    );
                    continue;
                }
            };
            if file_size > MAX_JSONL_FILE_BYTES {
                tracing::warn!(
                    "Skipping oversized Claude Code JSONL file {} ({} bytes; max {})",
                    file_path.display(),
                    file_size,
                    MAX_JSONL_FILE_BYTES
                );
                continue;
            }

            if let Some(conversation) = parse_jsonl_file(path, file_path, &project_names)? {
                summary.parsed_conversations += 1;
                summary.parsed_messages += conversation.messages.len();
                on_conversation(conversation)?;
            }
        }

        Ok(summary)
    }
}

fn parse_jsonl_file(
    root: &Path,
    file_path: &Path,
    project_names: &HashMap<String, String>,
) -> Result<Option<ImportedConversation>, ImportError> {
    let file = File::open(file_path)?;
    let mut reader = BufReader::new(file);

    let relative_path = file_path
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| file_path.to_string_lossy().to_string());

    let project_hash = extract_project_hash(root, file_path);
    let project_name = project_names.get(project_hash.as_str()).cloned();
    let source_id = build_source_id(project_hash.as_str(), root, file_path);

    let mut malformed_lines = 0_usize;
    let mut duplicate_lines = 0_usize;
    let mut messages = Vec::new();
    let mut seen_message_uuids: HashSet<String> = HashSet::new();
    let mut line_num = 0_usize;
    let mut line_bytes = Vec::new();

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

        if line_bytes.iter().all(|b| b.is_ascii_whitespace()) {
            continue;
        }

        if messages.len() >= MAX_MESSAGES_PER_CONVERSATION {
            tracing::warn!(
                "Reached message cap ({}) for {}; skipping remaining lines",
                MAX_MESSAGES_PER_CONVERSATION,
                file_path.display()
            );
            break;
        }

        let line = match std::str::from_utf8(&line_bytes) {
            Ok(v) => v,
            Err(e) => {
                malformed_lines += 1;
                tracing::warn!(
                    "Skipping non-UTF8 JSONL line {} in {}: {}",
                    line_num,
                    file_path.display(),
                    e
                );
                continue;
            }
        };

        let raw: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                malformed_lines += 1;
                tracing::warn!(
                    "Skipping malformed JSON line {} in {}: {}",
                    line_num,
                    file_path.display(),
                    e
                );
                continue;
            }
        };

        let source_type = value_string(&raw, "type");
        let role = match source_type.as_str() {
            "human" | "user" => "user",
            "assistant" => "assistant",
            other => {
                tracing::warn!(
                    "Skipping unsupported Claude Code message type '{}' in {}",
                    other,
                    file_path.display()
                );
                continue;
            }
        };

        let timestamp_raw = value_string(&raw, "timestamp");
        let message_uuid = message_uuid(&raw);
        if let Some(uuid) = message_uuid.as_deref()
            && !seen_message_uuids.insert(uuid.to_string())
        {
            duplicate_lines += 1;
            continue;
        }

        let timestamp = if timestamp_raw.is_empty() {
            None
        } else {
            parse_timestamp(timestamp_raw.as_str())
        };

        messages.push(ImportedMessage {
            role: role.to_string(),
            content: clamp_message_text(choose_message_text(&raw), file_path, line_num),
            timestamp,
            source_metadata: serde_json::json!({
                "source_type": source_type,
                "source_timestamp": timestamp_raw,
                "uuid": message_uuid,
                "session_id": nested_value_string(&raw, &["sessionId"]),
                "parent_uuid": nested_value_string(&raw, &["parentUuid"]),
                "leaf_uuid": nested_value_string(&raw, &["leafUuid"]),
                "message_id": nested_value_string(&raw, &["message", "id"]),
                "message_role": nested_value_string(&raw, &["message", "role"]),
                "cwd": nested_value_string(&raw, &["cwd"]),
            }),
        });
    }

    if messages.is_empty() {
        return Ok(None);
    }

    let title = messages
        .iter()
        .find(|m| m.role == "user" && !m.content.trim().is_empty())
        .map(|m| truncate_chars(m.content.trim(), 100))
        .filter(|s| !s.is_empty())
        .or(project_name.clone());

    let started_at = messages
        .iter()
        .filter_map(|m| m.timestamp.as_ref().cloned())
        .min();
    let updated_at = messages
        .iter()
        .filter_map(|m| m.timestamp.as_ref().cloned())
        .max();

    Ok(Some(ImportedConversation {
        source_id,
        title,
        started_at,
        updated_at,
        messages,
        source_metadata: serde_json::json!({
            "project_key": project_hash,
            "project_hash": project_hash.clone(),
            "project_name": project_name,
            "relative_path": relative_path,
            "malformed_lines": malformed_lines,
            "duplicate_lines": duplicate_lines,
        }),
    }))
}

fn choose_message_text(raw: &serde_json::Value) -> String {
    if let Some(text) = nested_value_string_opt(raw, &["text"]) {
        return text.to_string();
    }

    if let Some(text) = nested_value(raw, &["message", "content"]).and_then(content_to_text) {
        return text;
    }

    if let Some(text) = nested_value_string_opt(raw, &["message", "text"]) {
        return text.to_string();
    }

    if let Some(text) = nested_value(raw, &["content"]).and_then(content_to_text) {
        return text;
    }

    String::new()
}

fn content_to_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(v) => {
            let text = v.trim();
            if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            }
        }
        serde_json::Value::Array(items) => {
            if items.len() > MAX_CONTENT_BLOCKS {
                tracing::warn!(
                    "Claude Code message content has {} block(s); reading first {}",
                    items.len(),
                    MAX_CONTENT_BLOCKS
                );
            }

            let mut pieces = Vec::new();
            for item in items.iter().take(MAX_CONTENT_BLOCKS) {
                match item {
                    serde_json::Value::String(v) => {
                        let text = v.trim();
                        if !text.is_empty() {
                            pieces.push(text.to_string());
                        }
                    }
                    serde_json::Value::Object(_) => {
                        let kind = value_string(item, "type");
                        if (kind.is_empty() || kind == "text")
                            && let Some(text) = nested_value_string_opt(item, &["text"])
                        {
                            pieces.push(text.to_string());
                        }
                    }
                    _ => {}
                }
            }

            if pieces.is_empty() {
                None
            } else {
                Some(pieces.join("\n\n"))
            }
        }
        serde_json::Value::Object(_) => {
            if let Some(text) = nested_value_string_opt(value, &["text"]) {
                return Some(text.to_string());
            }
            nested_value(value, &["content"]).and_then(content_to_text)
        }
        _ => None,
    }
}

fn message_uuid(raw: &serde_json::Value) -> Option<String> {
    nested_value_string_opt(raw, &["uuid"])
        .or_else(|| nested_value_string_opt(raw, &["message", "id"]))
        .or_else(|| nested_value_string_opt(raw, &["message", "uuid"]))
        .map(|v| v.to_string())
}

fn nested_value<'a>(value: &'a serde_json::Value, path: &[&str]) -> Option<&'a serde_json::Value> {
    let mut cursor = value;
    for key in path {
        cursor = cursor.get(*key)?;
    }
    Some(cursor)
}

fn nested_value_string(value: &serde_json::Value, path: &[&str]) -> String {
    nested_value_string_opt(value, path)
        .unwrap_or_default()
        .to_string()
}

fn nested_value_string_opt<'a>(value: &'a serde_json::Value, path: &[&str]) -> Option<&'a str> {
    nested_value(value, path)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn value_string(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn extract_project_hash(root: &Path, file_path: &Path) -> String {
    if let Ok(relative) = file_path.strip_prefix(root)
        && let Some(component) = relative.components().next()
    {
        let value = component.as_os_str().to_string_lossy().trim().to_string();
        if !value.is_empty() {
            return value;
        }
    }

    "unknown".to_string()
}

fn build_source_id(project_hash: &str, root: &Path, file_path: &Path) -> String {
    if let Ok(relative) = file_path.strip_prefix(root) {
        let rel = relative.to_string_lossy().replace('\\', "/");
        if !rel.trim().is_empty() {
            return rel;
        }
    }

    let file_name = file_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "conversation.jsonl".to_string());
    format!("{}/{}", project_hash, file_name)
}

fn load_project_index(root: &Path) -> HashMap<String, String> {
    let path = root.join("projects.json");
    if !path.exists() {
        return HashMap::new();
    }

    let metadata = match fs::metadata(&path) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Failed to read metadata for {}: {}", path.display(), e);
            return HashMap::new();
        }
    };
    if metadata.len() > MAX_PROJECTS_INDEX_BYTES {
        tracing::warn!(
            "Skipping oversized {} ({} bytes; max {})",
            path.display(),
            metadata.len(),
            MAX_PROJECTS_INDEX_BYTES
        );
        return HashMap::new();
    }

    let file = match File::open(&path) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Failed to open {}: {}", path.display(), e);
            return HashMap::new();
        }
    };

    let value: serde_json::Value = match serde_json::from_reader(file) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Failed to parse {}: {}", path.display(), e);
            return HashMap::new();
        }
    };

    parse_project_name_map(&value)
}

fn parse_project_name_map(value: &serde_json::Value) -> HashMap<String, String> {
    let mut map = HashMap::new();

    if let Some(projects) = value.get("projects").and_then(|v| v.as_array()) {
        for project in projects {
            if let Some((id, name)) = parse_project_entry(project) {
                map.insert(id, name);
            }
        }
        return map;
    }

    if let Some(obj) = value.as_object() {
        for (key, val) in obj {
            if let Some(name) = val.as_str() {
                map.insert(key.clone(), name.to_string());
                continue;
            }

            if let Some((id, name)) = parse_project_entry(val) {
                map.insert(id, name);
            } else if let Some(name) = parse_project_name(val) {
                map.insert(key.clone(), name);
            }
        }
    }

    map
}

fn parse_project_entry(value: &serde_json::Value) -> Option<(String, String)> {
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("project_id").and_then(|v| v.as_str()))
        .or_else(|| value.get("hash").and_then(|v| v.as_str()))
        .or_else(|| value.get("project_hash").and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|v| !v.is_empty())?
        .to_string();

    let name = parse_project_name(value)?;
    Some((id, name))
}

fn parse_project_name(value: &serde_json::Value) -> Option<String> {
    value
        .get("name")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("title").and_then(|v| v.as_str()))
        .or_else(|| value.get("project_name").and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| truncate_chars(v, MAX_PROJECT_NAME_CHARS))
}

fn clamp_message_text(text: String, file_path: &Path, line_num: usize) -> String {
    if text.chars().count() <= MAX_MESSAGE_TEXT_CHARS {
        text
    } else {
        tracing::warn!(
            "Truncating oversized Claude Code message text at {}:{} to {} chars",
            file_path.display(),
            line_num,
            MAX_MESSAGE_TEXT_CHARS
        );
        truncate_chars(&text, MAX_MESSAGE_TEXT_CHARS)
    }
}

pub fn default_claude_code_path() -> PathBuf {
    dirs::home_dir()
        .map(|home| home.join(".claude").join("projects"))
        .unwrap_or_else(|| PathBuf::from(".claude/projects"))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::fs::File;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::{ClaudeCodeImporter, Importer, MAX_JSONL_FILE_BYTES, MAX_MESSAGE_TEXT_CHARS};

    #[test]
    fn parses_multiple_conversations_and_title_truncation() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();

        let project_a = root.join("abc123");
        let project_b = root.join("def456");
        fs::create_dir_all(&project_a).expect("create project A");
        fs::create_dir_all(&project_b).expect("create project B");

        let long_user = "x".repeat(140);
        write_jsonl(
            &project_a.join("conversation1.jsonl"),
            &[
                format!(
                    r#"{{"type":"human","text":"{}","timestamp":"2025-05-14T10:00:00.000Z"}}"#,
                    long_user
                ),
                r#"{"type":"assistant","text":"reply","timestamp":"2025-05-14T10:00:05.000Z"}"#
                    .to_string(),
            ],
        );
        write_jsonl(
            &project_a.join("conversation2.jsonl"),
            &[
                r#"{"type":"human","text":"second","timestamp":"2025-05-15T10:00:00.000Z"}"#.to_string(),
                r#"{"type":"assistant","text":"second reply","timestamp":"2025-05-15T10:00:03.000Z"}"#.to_string(),
            ],
        );
        write_jsonl(
            &project_b.join("conversation3.jsonl"),
            &[
                r#"{"type":"human","text":"third","timestamp":"2025-05-16T10:00:00.000Z"}"#
                    .to_string(),
            ],
        );

        let importer = ClaudeCodeImporter;
        let conversations = importer.parse(root).expect("parse");

        assert_eq!(conversations.len(), 3);
        assert!(
            conversations
                .iter()
                .any(|c| c.source_id == "abc123/conversation1.jsonl")
        );
        assert!(
            conversations
                .iter()
                .any(|c| c.source_id == "abc123/conversation2.jsonl")
        );
        assert!(
            conversations
                .iter()
                .any(|c| c.source_id == "def456/conversation3.jsonl")
        );

        let first = conversations
            .iter()
            .find(|c| c.source_id == "abc123/conversation1.jsonl")
            .expect("first conversation");
        let title = first.title.clone().unwrap_or_default();
        assert_eq!(title.chars().count(), 100);
    }

    #[test]
    fn skips_malformed_lines_and_empty_files() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let project = root.join("abc123");
        fs::create_dir_all(&project).expect("create project");

        write_jsonl(
            &project.join("conversation1.jsonl"),
            &[
                r#"{"type":"human","text":"hello","timestamp":"2025-05-14T10:00:00.000Z"}"#
                    .to_string(),
                "{not-valid-json}".to_string(),
                r#"{"type":"assistant","text":"world","timestamp":"2025-05-14T10:00:03.000Z"}"#
                    .to_string(),
            ],
        );
        write_jsonl(&project.join("empty.jsonl"), &[]);

        let importer = ClaudeCodeImporter;
        let conversations = importer.parse(root).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].messages.len(), 2);
    }

    #[test]
    fn parses_modern_nested_schema_and_skips_duplicate_message_uuids() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let project = root.join("abc123");
        fs::create_dir_all(&project).expect("create project");

        write_jsonl(
            &project.join("stream.jsonl"),
            &[
                r#"{"type":"user","uuid":"u-1","timestamp":"2025-09-01T10:00:00.000Z","message":{"role":"user","content":"hello from nested user payload"}}"#.to_string(),
                r#"{"type":"assistant","uuid":"a-1","timestamp":"2025-09-01T10:00:01.000Z","message":{"role":"assistant","content":[{"type":"text","text":"assistant reply from content blocks"},{"type":"tool_use","name":"read_file"}]}}"#.to_string(),
                r#"{"type":"assistant","uuid":"a-1","timestamp":"2025-09-01T10:00:02.000Z","message":{"role":"assistant","content":[{"type":"text","text":"duplicate assistant reply"}]}}"#.to_string(),
                r#"{"type":"assistant","uuid":"a-2","timestamp":"2025-09-01T10:00:03.000Z","content":[{"type":"text","text":"assistant from top-level content"}]}"#.to_string(),
                r#"{"type":"summary","uuid":"s-1","summary":"ignored"}"#.to_string(),
            ],
        );

        let importer = ClaudeCodeImporter;
        let conversations = importer.parse(root).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].messages.len(), 3);
        assert_eq!(conversations[0].messages[0].role, "user");
        assert_eq!(
            conversations[0].messages[0].content,
            "hello from nested user payload"
        );
        assert_eq!(
            conversations[0].messages[1].content,
            "assistant reply from content blocks"
        );
        assert_eq!(
            conversations[0].messages[2].content,
            "assistant from top-level content"
        );
        assert_eq!(
            conversations[0]
                .source_metadata
                .get("duplicate_lines")
                .and_then(|v| v.as_u64()),
            Some(1)
        );
    }

    #[test]
    fn reads_projects_json_for_project_names() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let project = root.join("abc123");
        fs::create_dir_all(&project).expect("create project");

        fs::write(
            root.join("projects.json"),
            r#"{"projects":[{"id":"abc123","name":"Core App"}]}"#,
        )
        .expect("write projects.json");
        write_jsonl(
            &project.join("conversation.jsonl"),
            &[r#"{"type":"assistant","text":"no user title","timestamp":"2025-05-14T10:00:00.000Z"}"#.to_string()],
        );

        let importer = ClaudeCodeImporter;
        let conversations = importer.parse(root).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].title.as_deref(), Some("Core App"));
    }

    #[test]
    fn parses_fixture_files() {
        let fixture_root = fixture_dir("claude_code");
        let importer = ClaudeCodeImporter;
        let conversations = importer.parse(&fixture_root).expect("parse fixtures");

        assert_eq!(conversations.len(), 4);
        assert!(
            conversations
                .iter()
                .any(|c| c.source_id == "abc123/conversation1.jsonl")
        );
        assert!(
            conversations
                .iter()
                .any(|c| c.source_id == "abc123/conversation2.jsonl")
        );
        assert!(
            conversations
                .iter()
                .any(|c| c.source_id == "def456/conversation1.jsonl")
        );
        assert!(
            conversations
                .iter()
                .any(|c| c.source_id == "abc123/malformed.jsonl")
        );
    }

    #[test]
    fn skips_oversized_jsonl_files() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let project = root.join("abc123");
        fs::create_dir_all(&project).expect("create project");

        write_jsonl(
            &project.join("small.jsonl"),
            &[
                r#"{"type":"human","text":"ok","timestamp":"2025-05-14T10:00:00.000Z"}"#
                    .to_string(),
            ],
        );

        let large_path = project.join("large.jsonl");
        let file = File::create(&large_path).expect("create large jsonl");
        file.set_len(MAX_JSONL_FILE_BYTES + 1)
            .expect("set large jsonl size");

        let importer = ClaudeCodeImporter;
        let conversations = importer.parse(root).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].source_id, "abc123/small.jsonl");
    }

    #[test]
    fn truncates_oversized_message_text() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let project = root.join("abc123");
        fs::create_dir_all(&project).expect("create project");

        let huge_text = "x".repeat(MAX_MESSAGE_TEXT_CHARS + 20);
        write_jsonl(
            &project.join("conversation.jsonl"),
            &[format!(
                r#"{{"type":"human","text":"{}","timestamp":"2025-05-14T10:00:00.000Z"}}"#,
                huge_text
            )],
        );

        let importer = ClaudeCodeImporter;
        let conversations = importer.parse(root).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(
            conversations[0].messages[0].content.chars().count(),
            MAX_MESSAGE_TEXT_CHARS
        );
    }

    fn write_jsonl(path: &Path, lines: &[String]) {
        let mut content = lines.join("\n");
        if !content.is_empty() {
            content.push('\n');
        }
        fs::write(path, content).expect("write jsonl");
    }

    fn fixture_dir(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("import")
            .join(name)
    }
}

//! Claude.ai ZIP export importer.
//!
//! Parses `conversations.json` from Claude.ai export ZIP files.

use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::de::{Deserializer as _, Error as _, SeqAccess, Visitor};

use crate::cli::import::{
    ImportError, ImportedConversation, ImportedMessage, Importer, parse_timestamp, truncate_chars,
};

/// Importer for Claude.ai browser exports.
pub struct ClaudeWebImporter;

// Guardrails to reduce abuse risk from malformed or intentionally hostile exports.
const MAX_ZIP_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB
const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_CONVERSATIONS_JSON_BYTES: u64 = 1_200 * 1024 * 1024; // 1.2 GiB
const MAX_CONVERSATIONS: usize = 250_000;
const MAX_MESSAGES_PER_CONVERSATION: usize = 100_000;
const MAX_MESSAGE_TEXT_CHARS: usize = 400_000;
const MAX_CONTENT_BLOCKS: usize = 20_000;
const MAX_ATTACHMENT_ITEMS: usize = 512;
const MAX_ACCOUNT_METADATA_CHARS: usize = 20_000;

#[derive(Debug)]
struct PendingMessage {
    role: String,
    content: String,
    created_at: Option<DateTime<Utc>>,
}

impl Importer for ClaudeWebImporter {
    fn source_name(&self) -> &str {
        "Claude.ai"
    }

    fn parse(&self, path: &Path) -> Result<Vec<ImportedConversation>, ImportError> {
        let metadata = fs::metadata(path)?;
        if metadata.len() > MAX_ZIP_FILE_BYTES {
            return Err(ImportError::Parse {
                reason: format!(
                    "ZIP too large ({} bytes), max supported is {} bytes",
                    metadata.len(),
                    MAX_ZIP_FILE_BYTES
                ),
            });
        }

        let file = File::open(path)?;
        let mut archive =
            zip::ZipArchive::new(file).map_err(|error| ImportError::Zip(error.to_string()))?;
        if archive.len() > MAX_ARCHIVE_ENTRIES {
            return Err(ImportError::Parse {
                reason: format!(
                    "ZIP has too many entries ({}), max allowed is {}",
                    archive.len(),
                    MAX_ARCHIVE_ENTRIES
                ),
            });
        }

        let mut conversations_index = None;
        for index in 0..archive.len() {
            let entry = archive
                .by_index(index)
                .map_err(|error| ImportError::Zip(error.to_string()))?;
            let name = entry.name().to_string();
            if name == "conversations.json" || name.ends_with("/conversations.json") {
                conversations_index = Some(index);
                break;
            }
        }

        let Some(index) = conversations_index else {
            return Err(ImportError::Parse {
                reason: "ZIP does not contain conversations.json".to_string(),
            });
        };

        let entry = archive
            .by_index(index)
            .map_err(|error| ImportError::Zip(error.to_string()))?;
        if entry.size() > MAX_CONVERSATIONS_JSON_BYTES {
            return Err(ImportError::Parse {
                reason: format!(
                    "conversations.json too large ({} bytes), max allowed is {} bytes",
                    entry.size(),
                    MAX_CONVERSATIONS_JSON_BYTES
                ),
            });
        }

        let limited_reader = entry.take(MAX_CONVERSATIONS_JSON_BYTES);
        parse_streamed_conversations(limited_reader)
    }
}

#[derive(Debug, Deserialize, Default)]
struct ClaudeWebRawConversation {
    #[serde(default, deserialize_with = "de_string_lenient")]
    uuid: String,
    #[serde(default, deserialize_with = "de_string_lenient")]
    name: String,
    #[serde(default, deserialize_with = "de_string_lenient")]
    summary: String,
    #[serde(default, deserialize_with = "de_string_lenient")]
    created_at: String,
    #[serde(default, deserialize_with = "de_string_lenient")]
    updated_at: String,
    #[serde(default)]
    account: serde_json::Value,
    #[serde(default, deserialize_with = "de_chat_messages_lenient")]
    chat_messages: Vec<ClaudeWebRawMessage>,
}

#[derive(Debug, Deserialize, Default)]
struct ClaudeWebRawMessage {
    #[serde(default, rename = "uuid", deserialize_with = "de_string_lenient")]
    _uuid: String,
    #[serde(default, deserialize_with = "de_string_lenient")]
    sender: String,
    #[serde(default, deserialize_with = "de_string_lenient")]
    text: String,
    #[serde(default)]
    content: serde_json::Value,
    #[serde(default, deserialize_with = "de_string_lenient")]
    created_at: String,
    #[serde(default, rename = "updated_at", deserialize_with = "de_string_lenient")]
    _updated_at: String,
    #[serde(default)]
    attachments: serde_json::Value,
    #[serde(default)]
    files: serde_json::Value,
}

fn parse_streamed_conversations<R: std::io::Read>(
    reader: R,
) -> Result<Vec<ImportedConversation>, ImportError> {
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    deserializer
        .deserialize_seq(ConversationsVisitor)
        .map_err(|error: serde_json::Error| ImportError::Parse {
            reason: error.to_string(),
        })
}

struct ConversationsVisitor;

impl<'de> Visitor<'de> for ConversationsVisitor {
    type Value = Vec<ImportedConversation>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON array of Claude conversations")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut conversations = Vec::new();
        let mut index = 0_usize;

        while let Some(raw_value) = seq.next_element::<serde_json::Value>()? {
            if conversations.len() >= MAX_CONVERSATIONS {
                return Err(A::Error::custom(format!(
                    "Too many conversations (>{}) in export",
                    MAX_CONVERSATIONS
                )));
            }

            let raw: ClaudeWebRawConversation = match serde_json::from_value(raw_value) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(
                        "Skipping malformed conversation entry in Claude.ai export: {}",
                        error
                    );
                    continue;
                }
            };

            conversations.push(map_conversation(raw, index));
            index += 1;
        }

        Ok(conversations)
    }
}

fn map_conversation(raw: ClaudeWebRawConversation, index: usize) -> ImportedConversation {
    let mut pending_messages = Vec::new();

    let total_messages = raw.chat_messages.len();
    if total_messages > MAX_MESSAGES_PER_CONVERSATION {
        tracing::warn!(
            "Claude.ai conversation {} has {} messages; importing first {}",
            raw.uuid,
            total_messages,
            MAX_MESSAGES_PER_CONVERSATION
        );
    }

    let mut skipped_unknown_senders = 0_usize;
    let mut attachment_message_count = 0_usize;
    let mut file_message_count = 0_usize;
    let mut content_types_seen = HashSet::new();

    for message in raw
        .chat_messages
        .into_iter()
        .take(MAX_MESSAGES_PER_CONVERSATION)
    {
        let role = match message.sender.as_str() {
            "human" => "user",
            "assistant" => "assistant",
            _ => {
                skipped_unknown_senders += 1;
                continue;
            }
        };

        for content_type in content_types(&message.content) {
            content_types_seen.insert(content_type);
        }

        let normalized_attachments = clamp_array_len(
            normalize_array(message.attachments.clone()),
            MAX_ATTACHMENT_ITEMS,
            "attachments",
        );
        let normalized_files = clamp_array_len(
            normalize_array(message.files.clone()),
            MAX_ATTACHMENT_ITEMS,
            "files",
        );
        if normalized_attachments
            .as_array()
            .is_some_and(|items| !items.is_empty())
        {
            attachment_message_count += 1;
        }
        if normalized_files
            .as_array()
            .is_some_and(|items| !items.is_empty())
        {
            file_message_count += 1;
        }

        let text = clamp_text(choose_message_text(&message), MAX_MESSAGE_TEXT_CHARS);
        let created_at = if message.created_at.is_empty() {
            None
        } else {
            parse_timestamp(message.created_at.as_str())
        };

        pending_messages.push(PendingMessage {
            role: role.to_string(),
            content: text,
            created_at,
        });
    }

    if skipped_unknown_senders > 0 {
        tracing::warn!(
            "Claude.ai conversation {} skipped {} unsupported sender message(s)",
            raw.uuid,
            skipped_unknown_senders
        );
    }

    let mut source_id = raw.uuid.trim().to_string();
    if source_id.is_empty() {
        source_id = format!("missing-uuid-{}", index);
        tracing::warn!(
            "Claude.ai conversation missing uuid at index {}; using {}",
            index,
            source_id
        );
    }

    let mut title = if raw.name.trim().is_empty() {
        None
    } else {
        Some(truncate_chars(raw.name.trim(), 100))
    };
    if title.is_none() {
        title = pending_messages
            .iter()
            .find(|message| message.role == "user" && !message.content.trim().is_empty())
            .map(|message| truncate_chars(message.content.trim(), 100));
    }

    let conversation_created_at = if raw.created_at.is_empty() {
        None
    } else {
        parse_timestamp(raw.created_at.as_str())
    };
    let conversation_updated_at = if raw.updated_at.is_empty() {
        None
    } else {
        parse_timestamp(raw.updated_at.as_str())
    };

    let message_created_at = pending_messages
        .iter()
        .filter_map(|message| message.created_at.as_ref().cloned())
        .min();
    let message_updated_at = pending_messages
        .iter()
        .filter_map(|message| message.created_at.as_ref().cloned())
        .max();

    let created_at = conversation_created_at
        .or(message_created_at)
        .unwrap_or_else(unix_epoch);
    let last_activity = conversation_updated_at
        .or(message_updated_at)
        .unwrap_or_else(|| created_at.clone());

    let messages = pending_messages
        .into_iter()
        .map(|message| ImportedMessage {
            role: message.role,
            content: message.content,
            created_at: message.created_at.unwrap_or_else(|| created_at.clone()),
        })
        .collect();

    let mut content_types = content_types_seen.into_iter().collect::<Vec<_>>();
    content_types.sort();

    ImportedConversation {
        source_id,
        title,
        messages,
        created_at,
        last_activity,
        source_metadata: serde_json::json!({
            "name": raw.name,
            "summary": clamp_text(raw.summary, 20_000),
            "account": clamp_json_metadata(raw.account, MAX_ACCOUNT_METADATA_CHARS, "account"),
            "total_messages_in_source": total_messages,
            "messages_imported": total_messages.min(MAX_MESSAGES_PER_CONVERSATION),
            "skipped_unknown_senders": skipped_unknown_senders,
            "created_at": raw.created_at,
            "updated_at": raw.updated_at,
            "attachment_message_count": attachment_message_count,
            "file_message_count": file_message_count,
            "content_types": content_types,
        }),
    }
}

fn choose_message_text(message: &ClaudeWebRawMessage) -> String {
    if !message.text.trim().is_empty() {
        return message.text.clone();
    }

    if let Some(text) = content_text(&message.content) {
        return text;
    }

    if let Some(text) = extracted_attachment_text(&message.attachments) {
        return text;
    }

    if let Some(text) = extracted_attachment_text(&message.files) {
        return text;
    }

    String::new()
}

fn content_text(content: &serde_json::Value) -> Option<String> {
    match content {
        serde_json::Value::String(text) => {
            if text.trim().is_empty() {
                None
            } else {
                Some(text.clone())
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(|value| value.as_str())
                && !text.trim().is_empty()
            {
                return Some(text.to_string());
            }

            for key in ["content", "parts", "blocks"] {
                if let Some(inner) = map.get(key)
                    && let Some(text) = content_text(inner)
                {
                    return Some(text);
                }
            }

            None
        }
        serde_json::Value::Array(blocks) => {
            let mut pieces = Vec::new();
            if blocks.len() > MAX_CONTENT_BLOCKS {
                tracing::warn!(
                    "Claude.ai message content has {} block(s); reading first {}",
                    blocks.len(),
                    MAX_CONTENT_BLOCKS
                );
            }

            for block in blocks.iter().take(MAX_CONTENT_BLOCKS) {
                if let Some(text) = block.as_str() {
                    if !text.trim().is_empty() {
                        pieces.push(text.to_string());
                    }
                    continue;
                }

                let kind = block
                    .get("type")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                let text = block
                    .get("text")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();

                if (kind.is_empty() || kind == "text") && !text.trim().is_empty() {
                    pieces.push(text.to_string());
                }
            }

            if pieces.is_empty() {
                None
            } else {
                Some(pieces.join("\n\n"))
            }
        }
        _ => None,
    }
}

fn content_types(content: &serde_json::Value) -> Vec<String> {
    let Some(blocks) = content.as_array() else {
        return Vec::new();
    };

    if blocks.len() > MAX_CONTENT_BLOCKS {
        tracing::warn!(
            "Claude.ai message content has {} block(s); scanning first {} for content types",
            blocks.len(),
            MAX_CONTENT_BLOCKS
        );
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for block in blocks.iter().take(MAX_CONTENT_BLOCKS) {
        let Some(kind) = block.get("type").and_then(|value| value.as_str()) else {
            continue;
        };
        if seen.insert(kind.to_string()) {
            out.push(kind.to_string());
        }
    }

    out
}

fn extracted_attachment_text(value: &serde_json::Value) -> Option<String> {
    let items = attachment_items(value);
    if items.is_empty() {
        return None;
    }

    let mut pieces = Vec::new();
    for item in items.iter().take(MAX_ATTACHMENT_ITEMS) {
        let extracted = item
            .get("extracted_content")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if !extracted.trim().is_empty() {
            pieces.push(extracted.to_string());
            continue;
        }

        let text = item
            .get("text")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if !text.trim().is_empty() {
            pieces.push(text.to_string());
        }
    }

    if pieces.is_empty() {
        None
    } else {
        Some(pieces.join("\n\n"))
    }
}

fn attachment_items<'a>(value: &'a serde_json::Value) -> Vec<&'a serde_json::Value> {
    if let Some(items) = value.as_array() {
        return items.iter().collect();
    }

    let Some(object) = value.as_object() else {
        return Vec::new();
    };

    for key in ["attachments", "files", "items", "results", "data"] {
        if let Some(items) = object.get(key).and_then(|value| value.as_array()) {
            return items.iter().collect();
        }
    }

    if object.contains_key("extracted_content") || object.contains_key("text") {
        return vec![value];
    }

    Vec::new()
}

fn normalize_array(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => serde_json::Value::Array(values),
        serde_json::Value::Null => serde_json::Value::Array(Vec::new()),
        serde_json::Value::Object(map) => {
            for key in ["attachments", "files", "items", "results", "data"] {
                if let Some(serde_json::Value::Array(items)) = map.get(key) {
                    return serde_json::Value::Array(items.clone());
                }
            }
            if map.contains_key("extracted_content") || map.contains_key("text") {
                return serde_json::Value::Array(vec![serde_json::Value::Object(map)]);
            }

            tracing::warn!("Expected array metadata field but got object value; coercing to []");
            serde_json::Value::Array(Vec::new())
        }
        _ => {
            tracing::warn!("Expected array metadata field but got non-array value; coercing to []");
            serde_json::Value::Array(Vec::new())
        }
    }
}

fn clamp_text(value: String, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value
    } else {
        tracing::warn!(
            "Truncating oversized message text ({} chars) to {} chars",
            value.chars().count(),
            max_chars
        );
        truncate_chars(&value, max_chars)
    }
}

fn clamp_array_len(
    value: serde_json::Value,
    max_items: usize,
    field_name: &str,
) -> serde_json::Value {
    let mut items = match value {
        serde_json::Value::Array(values) => values,
        _ => Vec::new(),
    };
    if items.len() > max_items {
        tracing::warn!(
            "Truncating {} array from {} to {} item(s)",
            field_name,
            items.len(),
            max_items
        );
        items.truncate(max_items);
    }
    serde_json::Value::Array(items)
}

fn clamp_json_metadata(
    value: serde_json::Value,
    max_chars: usize,
    field_name: &str,
) -> serde_json::Value {
    let serialized = match serde_json::to_string(&value) {
        Ok(value) => value,
        Err(_) => return serde_json::Value::Null,
    };
    if serialized.chars().count() <= max_chars {
        value
    } else {
        tracing::warn!(
            "Truncating oversized {} metadata payload to summary",
            field_name
        );
        serde_json::json!({
            "truncated": true,
            "field": field_name,
            "original_chars": serialized.chars().count(),
        })
    }
}

fn de_string_lenient<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::String(value) => value,
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => String::new(),
    })
}

fn de_chat_messages_lenient<'de, D>(deserializer: D) -> Result<Vec<ClaudeWebRawMessage>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let serde_json::Value::Array(items) = value else {
        return Ok(Vec::new());
    };

    let mut messages = Vec::with_capacity(items.len());
    for item in items {
        match serde_json::from_value::<ClaudeWebRawMessage>(item) {
            Ok(message) => messages.push(message),
            Err(error) => tracing::warn!(
                "Skipping malformed chat_messages entry in Claude.ai export: {}",
                error
            ),
        }
    }

    Ok(messages)
}

fn unix_epoch() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_else(Utc::now)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;

    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    use super::{ClaudeWebImporter, ImportError, Importer};

    #[test]
    fn parses_three_conversations_from_zip() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("claude_export.zip");
        let payload = r#"
        [
          {
            "uuid": "a1",
            "name": "One",
            "created_at": "2025-01-15T10:00:00.000Z",
            "updated_at": "2025-01-15T10:30:00.000Z",
            "chat_messages": [
              {"uuid":"m1","sender":"human","text":"hello","created_at":"2025-01-15T10:00:05.000Z","attachments":[{"id":"att-1"}],"files":[]},
              {"uuid":"m2","sender":"assistant","text":"hi","created_at":"2025-01-15T10:00:10.000Z"}
            ]
          },
          {
            "uuid": "a2",
            "name": "Two",
            "chat_messages": [
              {"sender":"human","text":"second","created_at":"2025-01-15T11:00:05.000Z"},
              {"sender":"assistant","text":"done","created_at":"2025-01-15T11:00:10.000Z"}
            ]
          },
          {
            "uuid": "a3",
            "name": "Three",
            "chat_messages": [
              {"sender":"human","text":"third","created_at":"2025-01-15T12:00:05.000Z"}
            ]
          }
        ]
        "#;

        write_zip_with_file(&zip_path, "conversations.json", payload);

        let importer = ClaudeWebImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 3);
        assert_eq!(conversations[0].source_id, "a1");
        assert_eq!(conversations[1].source_id, "a2");
        assert_eq!(conversations[2].source_id, "a3");
        assert_eq!(
            conversations[0]
                .source_metadata
                .get("attachment_message_count")
                .and_then(|value| value.as_u64()),
            Some(1)
        );
        assert_eq!(
            conversations[0]
                .source_metadata
                .get("file_message_count")
                .and_then(|value| value.as_u64()),
            Some(0)
        );
    }

    #[test]
    fn errors_when_conversations_json_missing() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("claude_export.zip");
        write_zip_with_file(&zip_path, "other.json", "{}");

        let importer = ClaudeWebImporter;
        let result = importer.parse(&zip_path);
        match result {
            Err(ImportError::Parse { reason }) => {
                assert!(reason.contains("conversations.json"));
            }
            other => panic!("expected parse error, got {:?}", other),
        }
    }

    #[test]
    fn handles_empty_conversation_messages() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("claude_export.zip");
        write_zip_with_file(
            &zip_path,
            "conversations.json",
            r#"[{"uuid":"empty-1","name":"Empty","chat_messages":[]}]"#,
        );

        let importer = ClaudeWebImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].source_id, "empty-1");
        assert!(conversations[0].messages.is_empty());
    }

    #[test]
    fn parses_fixture_payload_from_zip() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("claude_export.zip");
        let fixture = fs::read_to_string(fixture_path("conversations.json")).expect("fixture");
        write_zip_with_file(&zip_path, "conversations.json", &fixture);

        let importer = ClaudeWebImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 3);
        assert_eq!(conversations[0].source_id, "web-1");
        assert_eq!(conversations[1].source_id, "web-2");
        assert_eq!(conversations[2].source_id, "web-3");
    }

    #[test]
    fn parses_empty_fixture_payload() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("claude_export.zip");
        let fixture =
            fs::read_to_string(fixture_path("conversations_empty.json")).expect("fixture");
        write_zip_with_file(&zip_path, "conversations.json", &fixture);

        let importer = ClaudeWebImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert!(conversations[0].messages.is_empty());
    }

    #[test]
    fn skips_malformed_conversations_and_messages_leniently() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("claude_export.zip");
        let payload = r#"
        [
          {
            "uuid": "a1",
            "name": "One",
            "chat_messages": [
              {"sender":"human","text":"hello"},
              {"sender":"assistant","text":"hi"}
            ]
          },
          42,
          {
            "uuid": "a2",
            "name": {"bad":"name"},
            "chat_messages": "not-an-array"
          },
          {
            "uuid": "a3",
            "name": "Three",
            "chat_messages": [
              {"sender":"human","text":"valid"},
              {"sender":{"nested":"oops"},"text":"ignored"},
              true
            ]
          }
        ]
        "#;
        write_zip_with_file(&zip_path, "conversations.json", payload);

        let importer = ClaudeWebImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 3);
        assert_eq!(conversations[0].source_id, "a1");
        assert_eq!(conversations[1].source_id, "a2");
        assert_eq!(conversations[2].source_id, "a3");
        assert_eq!(conversations[0].messages.len(), 2);
        assert_eq!(conversations[1].messages.len(), 0);
        assert_eq!(conversations[2].messages.len(), 1);
    }

    #[test]
    fn falls_back_to_attachment_extracted_content_when_text_is_empty() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("claude_export.zip");
        let payload = r#"
        [
          {
            "uuid": "att-1",
            "name": "Attachment fallback",
            "chat_messages": [
              {
                "sender":"human",
                "text":"",
                "content":[{"type":"text","text":""}],
                "attachments":[{"file_name":"paste.txt","extracted_content":"good could you please solve errors"}]
              },
              {
                "sender":"assistant",
                "text":"Looking at your Rust code, ..."
              }
            ]
          }
        ]
        "#;
        write_zip_with_file(&zip_path, "conversations.json", payload);

        let importer = ClaudeWebImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].messages.len(), 2);
        assert_eq!(
            conversations[0].messages[0].content,
            "good could you please solve errors"
        );
    }

    #[test]
    fn falls_back_to_files_extracted_content_when_text_is_empty() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("claude_export.zip");
        let payload = r#"
        [
          {
            "uuid": "files-1",
            "name": "Files fallback",
            "chat_messages": [
              {
                "sender":"human",
                "text":"",
                "content":[{"type":"text","text":""}],
                "attachments":[],
                "files":[{"file_name":"paste.txt","extracted_content":"from files extracted content"}]
              },
              {
                "sender":"assistant",
                "text":"ack"
              }
            ]
          }
        ]
        "#;
        write_zip_with_file(&zip_path, "conversations.json", payload);

        let importer = ClaudeWebImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].messages.len(), 2);
        assert_eq!(
            conversations[0].messages[0].content,
            "from files extracted content"
        );
    }

    #[test]
    fn reads_attachment_extracted_content_from_object_items_shape() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("claude_export.zip");
        let payload = r#"
        [
          {
            "uuid": "att-obj-1",
            "name": "Attachment object shape",
            "chat_messages": [
              {
                "sender":"human",
                "text":"",
                "content":[{"type":"text","text":""}],
                "attachments":{"items":[{"file_name":"paste.txt","extracted_content":"attachment object payload"}]},
                "files":[]
              }
            ]
          }
        ]
        "#;
        write_zip_with_file(&zip_path, "conversations.json", payload);

        let importer = ClaudeWebImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].messages.len(), 1);
        assert_eq!(
            conversations[0].messages[0].content,
            "attachment object payload"
        );
        assert_eq!(
            conversations[0]
                .source_metadata
                .get("attachment_message_count")
                .and_then(|value| value.as_u64()),
            Some(1)
        );
    }

    #[test]
    fn reads_content_text_from_object_shape_when_text_is_empty() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("claude_export.zip");
        let payload = r#"
        [
          {
            "uuid": "content-obj-1",
            "name": "Content object shape",
            "chat_messages": [
              {
                "sender":"assistant",
                "text":"",
                "content":{"type":"text","text":"content object text"}
              }
            ]
          }
        ]
        "#;
        write_zip_with_file(&zip_path, "conversations.json", payload);

        let importer = ClaudeWebImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].messages.len(), 1);
        assert_eq!(conversations[0].messages[0].content, "content object text");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn roundtrips_fixture_into_db_without_losing_messages_or_workspace_docs() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("claude_export.zip");
        let fixture = fs::read_to_string(fixture_path("conversations.json")).expect("fixture");
        write_zip_with_file(&zip_path, "conversations.json", &fixture);

        let importer = ClaudeWebImporter;
        let expected = importer.parse(&zip_path).expect("parse");
        let expected_count = expected.len();

        let (db, _tmp) = crate::testing::test_db().await;
        let user_id = "claude-web-roundtrip";
        let args = crate::cli::import::HistoryImportArgs {
            path: Some(zip_path.clone()),
            user_id: user_id.to_string(),
            dedup: true,
            to_workspace: true,
            dry_run: false,
        };

        crate::cli::import::run_import_command_with_db(
            crate::cli::import::ImportSource::ClaudeWeb,
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
                    crate::cli::import::ImportSource::ClaudeWeb.source_key(),
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
            crate::cli::import::ImportSource::ClaudeWeb,
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

        let workspace_paths_after = workspace.list_all().await.expect("list workspace");
        assert_eq!(workspace_paths_after.len(), expected_count);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    #[ignore = "manual real-export verification"]
    async fn manual_real_export_roundtrip_preserves_all_messages() {
        let export_path = std::env::var("IRONCLAW_REAL_CLAUDE_EXPORT")
            .expect("IRONCLAW_REAL_CLAUDE_EXPORT must be set");
        let path = std::path::PathBuf::from(export_path);

        let importer = ClaudeWebImporter;
        let expected = importer.parse(&path).expect("parse");
        let expected_count = expected.len();

        let (db, _tmp) = crate::testing::test_db().await;
        let user_id = "claude-web-real-export";
        let args = crate::cli::import::HistoryImportArgs {
            path: Some(path.clone()),
            user_id: user_id.to_string(),
            dedup: true,
            to_workspace: true,
            dry_run: false,
        };

        crate::cli::import::run_import_command_with_db(
            crate::cli::import::ImportSource::ClaudeWeb,
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
                    crate::cli::import::ImportSource::ClaudeWeb.source_key(),
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
    }

    fn write_zip_with_file(path: &std::path::Path, name: &str, content: &str) {
        let file = File::create(path).expect("create zip");
        let mut writer = zip::ZipWriter::new(file);
        writer
            .start_file(name, SimpleFileOptions::default())
            .expect("start zip file");
        writer
            .write_all(content.as_bytes())
            .expect("write zip payload");
        writer.finish().expect("finish zip");
    }

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("import")
            .join("claude_web")
            .join(name)
    }
}

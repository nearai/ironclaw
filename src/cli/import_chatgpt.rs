//! ChatGPT export ZIP importer.
//!
//! Parses `conversations.json` from ChatGPT export ZIP files and linearizes
//! the mapping tree by following the last child at each branch.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde::de::{Deserializer as _, Error as _, SeqAccess, Visitor};

use crate::cli::import::{
    ImportError, ImportedConversation, ImportedMessage, Importer, truncate_chars,
};

/// Importer for ChatGPT browser exports.
pub struct ChatGptImporter;

// Guardrails for malformed or intentionally hostile exports.
const MAX_ZIP_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB
const MAX_ARCHIVE_ENTRIES: usize = 10_000;
const MAX_CONVERSATIONS_JSON_BYTES: u64 = 1_500 * 1024 * 1024; // 1.5 GiB
const MAX_CONVERSATIONS: usize = 300_000;
const MAX_MAPPING_NODES_PER_CONVERSATION: usize = 200_000;
const MAX_MESSAGE_PARTS: usize = 20_000;
const MAX_MESSAGE_TEXT_CHARS: usize = 400_000;

#[derive(Debug)]
struct PendingMessage {
    id: String,
    role: String,
    content: String,
    created_at: DateTime<Utc>,
    model_slug: Option<String>,
}

impl Importer for ChatGptImporter {
    fn source_name(&self) -> &str {
        "ChatGPT"
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

        let conversation_entries = find_conversation_json_entries(&mut archive)?;
        if conversation_entries.is_empty() {
            return Err(ImportError::Parse {
                reason: "ZIP does not contain conversations.json or conversations-*.json"
                    .to_string(),
            });
        }

        let mut conversations = Vec::new();
        for entry_ref in conversation_entries {
            let entry = archive
                .by_index(entry_ref.index)
                .map_err(|error| ImportError::Zip(error.to_string()))?;
            if entry.size() > MAX_CONVERSATIONS_JSON_BYTES {
                return Err(ImportError::Parse {
                    reason: format!(
                        "{} too large ({} bytes), max allowed is {} bytes",
                        entry_ref.name,
                        entry.size(),
                        MAX_CONVERSATIONS_JSON_BYTES
                    ),
                });
            }

            let limited_reader = entry.take(MAX_CONVERSATIONS_JSON_BYTES);
            let mut parsed = parse_streamed_conversations(limited_reader)?;
            conversations.append(&mut parsed);
            if conversations.len() > MAX_CONVERSATIONS {
                return Err(ImportError::Parse {
                    reason: format!("Too many conversations (>{}) in export", MAX_CONVERSATIONS),
                });
            }
        }

        Ok(conversations)
    }
}

#[derive(Debug)]
struct ConversationEntry {
    index: usize,
    name: String,
}

fn find_conversation_json_entries(
    archive: &mut zip::ZipArchive<File>,
) -> Result<Vec<ConversationEntry>, ImportError> {
    let mut single_entry: Option<ConversationEntry> = None;
    let mut sharded_entries = Vec::new();

    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| ImportError::Zip(error.to_string()))?;
        let name = entry.name().to_string();
        let file_name = name.rsplit('/').next().unwrap_or(name.as_str());
        if file_name == "conversations.json" {
            if single_entry.is_none() {
                single_entry = Some(ConversationEntry { index, name });
            }
            continue;
        }

        if file_name.starts_with("conversations-") && file_name.ends_with(".json") {
            sharded_entries.push(ConversationEntry { index, name });
        }
    }

    if let Some(entry) = single_entry {
        return Ok(vec![entry]);
    }

    sharded_entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(sharded_entries)
}

#[derive(Debug, Deserialize, Default)]
struct ChatGptRawConversation {
    #[serde(default, deserialize_with = "de_string_lenient")]
    conversation_id: String,
    #[serde(default, deserialize_with = "de_string_lenient")]
    id: String,
    #[serde(default, deserialize_with = "de_string_lenient")]
    title: String,
    #[serde(default, deserialize_with = "de_f64_opt_lenient")]
    create_time: Option<f64>,
    #[serde(default, deserialize_with = "de_f64_opt_lenient")]
    update_time: Option<f64>,
    #[serde(default, deserialize_with = "de_mapping_lenient")]
    mapping: HashMap<String, ChatGptRawNode>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatGptRawNode {
    #[serde(default, deserialize_with = "de_string_lenient")]
    id: String,
    #[serde(default)]
    message: Option<ChatGptRawMessage>,
    #[serde(default, deserialize_with = "de_string_opt_lenient")]
    parent: Option<String>,
    #[serde(default, deserialize_with = "de_string_vec_lenient")]
    children: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatGptRawMessage {
    #[serde(default, deserialize_with = "de_string_lenient")]
    id: String,
    #[serde(default)]
    author: ChatGptRawAuthor,
    #[serde(default)]
    content: ChatGptRawContent,
    #[serde(default, deserialize_with = "de_f64_opt_lenient")]
    create_time: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
struct ChatGptRawAuthor {
    #[serde(default, deserialize_with = "de_string_lenient")]
    role: String,
    #[serde(default)]
    metadata: serde_json::Value,
}

#[derive(Debug, Deserialize, Default)]
struct ChatGptRawContent {
    #[serde(
        default,
        rename = "content_type",
        deserialize_with = "de_string_lenient"
    )]
    _content_type: String,
    #[serde(default, deserialize_with = "de_parts_lenient")]
    parts: Vec<serde_json::Value>,
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
        formatter.write_str("a JSON array of ChatGPT conversations")
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

            let raw: ChatGptRawConversation = match serde_json::from_value(raw_value) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!("Skipping malformed ChatGPT conversation entry: {}", error);
                    continue;
                }
            };

            conversations.push(map_conversation(raw, index));
            index += 1;
        }

        Ok(conversations)
    }
}

fn map_conversation(raw: ChatGptRawConversation, index: usize) -> ImportedConversation {
    let total_nodes = raw.mapping.len();
    if total_nodes > MAX_MAPPING_NODES_PER_CONVERSATION {
        tracing::warn!(
            "ChatGPT conversation {} has {} mapping nodes; linearization is capped at {} visited nodes",
            raw.conversation_id,
            total_nodes,
            MAX_MAPPING_NODES_PER_CONVERSATION
        );
    }

    let linearized = linearize_mapping(&raw.mapping, raw.create_time);
    let messages = linearized
        .messages
        .into_iter()
        .map(|message| ImportedMessage {
            role: message.role,
            content: message.content,
            created_at: message.created_at,
        })
        .collect::<Vec<_>>();

    let mut source_id = if !raw.conversation_id.trim().is_empty() {
        raw.conversation_id.trim().to_string()
    } else {
        raw.id.trim().to_string()
    };
    if source_id.is_empty() {
        source_id = format!("missing-conversation-id-{}", index);
        tracing::warn!(
            "ChatGPT conversation missing conversation_id at index {}; using {}",
            index,
            source_id
        );
    }

    let mut title = if raw.title.trim().is_empty() {
        None
    } else {
        Some(truncate_chars(raw.title.trim(), 100))
    };
    if title.is_none() {
        title = messages
            .iter()
            .find(|message| message.role == "user" && !message.content.trim().is_empty())
            .map(|message| truncate_chars(message.content.trim(), 100));
    }

    let conversation_created_at = raw.create_time.and_then(timestamp_from_unix_seconds);
    let conversation_updated_at = raw.update_time.and_then(timestamp_from_unix_seconds);
    let message_created_at = messages
        .iter()
        .map(|message| message.created_at.clone())
        .min();
    let message_updated_at = messages
        .iter()
        .map(|message| message.created_at.clone())
        .max();
    let created_at = conversation_created_at
        .or(message_created_at)
        .unwrap_or_else(unix_epoch);
    let last_activity = conversation_updated_at
        .or(message_updated_at)
        .unwrap_or_else(|| created_at.clone());

    ImportedConversation {
        source_id,
        title,
        messages,
        created_at,
        last_activity,
        source_metadata: serde_json::json!({
            "mapping_nodes_total": total_nodes,
            "linearized_nodes_visited": linearized.visited_nodes,
            "linearized_root_node_id": linearized.root_node_id,
            "null_message_nodes": linearized.null_message_nodes,
            "empty_text_messages": linearized.empty_text_messages,
            "created_at_unix": raw.create_time,
            "updated_at_unix": raw.update_time,
            "model_slugs": linearized.model_slugs,
            "message_model_slugs": linearized.message_model_slugs,
        }),
    }
}

#[derive(Debug, Default)]
struct LinearizedMapping {
    root_node_id: Option<String>,
    visited_nodes: usize,
    null_message_nodes: usize,
    empty_text_messages: usize,
    model_slugs: Vec<String>,
    message_model_slugs: Vec<serde_json::Value>,
    messages: Vec<PendingMessage>,
}

fn linearize_mapping(
    mapping: &HashMap<String, ChatGptRawNode>,
    conversation_create_time: Option<f64>,
) -> LinearizedMapping {
    let Some(root_id) = find_root_node_id(mapping) else {
        return LinearizedMapping::default();
    };

    let mut out = LinearizedMapping {
        root_node_id: Some(root_id.clone()),
        ..LinearizedMapping::default()
    };

    let mut current_id = root_id;
    let mut seen = HashSet::new();

    loop {
        if out.visited_nodes >= MAX_MAPPING_NODES_PER_CONVERSATION {
            tracing::warn!(
                "ChatGPT linearization hit node visit cap ({}); truncating path",
                MAX_MAPPING_NODES_PER_CONVERSATION
            );
            break;
        }

        if !seen.insert(current_id.clone()) {
            tracing::warn!(
                "Detected cycle while linearizing ChatGPT mapping at node {}",
                current_id
            );
            break;
        }

        let Some(node) = mapping.get(current_id.as_str()) else {
            break;
        };
        out.visited_nodes += 1;

        if let Some(message) = node.message.as_ref() {
            match convert_message(message, conversation_create_time) {
                Some(message) => {
                    if let Some(model_slug) = message.model_slug.as_ref() {
                        if !out.model_slugs.contains(model_slug) {
                            out.model_slugs.push(model_slug.clone());
                        }
                        if !message.id.trim().is_empty() {
                            out.message_model_slugs.push(serde_json::json!({
                                "message_id": message.id.clone(),
                                "model_slug": model_slug,
                            }));
                        }
                    }
                    out.messages.push(message);
                }
                None => out.empty_text_messages += 1,
            }
        } else {
            out.null_message_nodes += 1;
        }

        let next_child = node
            .children
            .iter()
            .rev()
            .find(|id| mapping.contains_key(id.as_str()))
            .cloned();

        let Some(next_id) = next_child else {
            break;
        };

        current_id = next_id;
    }

    out
}

fn find_root_node_id(mapping: &HashMap<String, ChatGptRawNode>) -> Option<String> {
    if mapping.is_empty() {
        return None;
    }

    if let Some((id, _)) = mapping.iter().find(|(_, node)| {
        node.parent
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    }) {
        return Some(id.clone());
    }

    let mut child_ids = HashSet::new();
    for node in mapping.values() {
        for child in &node.children {
            child_ids.insert(child.clone());
        }
    }

    if let Some((id, _)) = mapping.iter().find(|(id, _)| !child_ids.contains(*id)) {
        return Some((*id).clone());
    }

    mapping.keys().next().cloned()
}

fn convert_message(
    message: &ChatGptRawMessage,
    conversation_create_time: Option<f64>,
) -> Option<PendingMessage> {
    let content = clamp_message_text(parts_to_text(message.content.parts.as_slice()));
    if content.trim().is_empty() {
        return None;
    }

    let created_at = message
        .create_time
        .or(conversation_create_time)
        .and_then(timestamp_from_unix_seconds)
        .unwrap_or_else(unix_epoch);

    Some(PendingMessage {
        id: message.id.clone(),
        role: normalize_role(message.author.role.as_str()),
        content,
        created_at,
        model_slug: message
            .author
            .metadata
            .get("model_slug")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
    })
}

fn normalize_role(role: &str) -> String {
    let normalized = role.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return "assistant".to_string();
    }

    if matches!(
        normalized.as_str(),
        "user" | "assistant" | "system" | "tool"
    ) {
        return normalized;
    }

    normalized
}

fn parts_to_text(parts: &[serde_json::Value]) -> String {
    if parts.len() > MAX_MESSAGE_PARTS {
        tracing::warn!(
            "ChatGPT message has {} content part(s); reading first {}",
            parts.len(),
            MAX_MESSAGE_PARTS
        );
    }

    let mut pieces = Vec::new();
    for part in parts.iter().take(MAX_MESSAGE_PARTS) {
        if let Some(piece) = part_to_text(part) {
            pieces.push(piece);
        }
    }

    pieces.join("\n\n")
}

fn part_to_text(part: &serde_json::Value) -> Option<String> {
    match part {
        serde_json::Value::String(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(|value| value.as_str()) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }

            let content_type = map
                .get("content_type")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if content_type == "image_asset_pointer" || map.contains_key("asset_pointer") {
                return Some("[image]".to_string());
            }
            if !content_type.trim().is_empty() {
                return Some(format!("[{}]", content_type.trim()));
            }

            let kind = map
                .get("type")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if kind.contains("image") {
                return Some("[image]".to_string());
            }
            if !kind.trim().is_empty() {
                return Some(format!("[{}]", kind.trim()));
            }

            None
        }
        serde_json::Value::Array(items) => {
            let mut pieces = Vec::new();
            for item in items.iter().take(MAX_MESSAGE_PARTS) {
                if let Some(piece) = part_to_text(item) {
                    pieces.push(piece);
                }
            }
            if pieces.is_empty() {
                None
            } else {
                Some(pieces.join("\n\n"))
            }
        }
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn clamp_message_text(value: String) -> String {
    if value.chars().count() <= MAX_MESSAGE_TEXT_CHARS {
        value
    } else {
        tracing::warn!(
            "Truncating oversized ChatGPT message text ({} chars) to {} chars",
            value.chars().count(),
            MAX_MESSAGE_TEXT_CHARS
        );
        truncate_chars(&value, MAX_MESSAGE_TEXT_CHARS)
    }
}

fn timestamp_from_unix_seconds(ts: f64) -> Option<DateTime<Utc>> {
    if !ts.is_finite() {
        return None;
    }

    let seconds_floor = ts.floor();
    let mut seconds = seconds_floor as i64;
    let mut nanos = ((ts - seconds_floor) * 1_000_000_000.0).round() as i64;

    if nanos >= 1_000_000_000 {
        seconds += 1;
        nanos -= 1_000_000_000;
    }
    if nanos < 0 {
        seconds -= 1;
        nanos += 1_000_000_000;
    }

    DateTime::<Utc>::from_timestamp(seconds, nanos as u32)
}

fn unix_epoch() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(0, 0).unwrap_or_else(Utc::now)
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

fn de_string_opt_lenient<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let out = match value {
        serde_json::Value::String(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    };
    Ok(out)
}

fn de_f64_opt_lenient<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let out = match value {
        serde_json::Value::Number(value) => value.as_f64(),
        serde_json::Value::String(value) => value.parse::<f64>().ok(),
        _ => None,
    };
    Ok(out)
}

fn de_string_vec_lenient<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let serde_json::Value::Array(items) = value else {
        return Ok(Vec::new());
    };

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item {
            serde_json::Value::String(value) => {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    out.push(trimmed.to_string());
                }
            }
            serde_json::Value::Number(value) => out.push(value.to_string()),
            serde_json::Value::Bool(value) => out.push(value.to_string()),
            _ => {}
        }
    }

    Ok(out)
}

fn de_parts_lenient<'de, D>(deserializer: D) -> Result<Vec<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let serde_json::Value::Array(items) = value else {
        return Ok(Vec::new());
    };

    Ok(items)
}

fn de_mapping_lenient<'de, D>(deserializer: D) -> Result<HashMap<String, ChatGptRawNode>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let serde_json::Value::Object(items) = value else {
        return Ok(HashMap::new());
    };

    let mut mapping = HashMap::with_capacity(items.len());
    for (id, raw_node) in items {
        match serde_json::from_value::<ChatGptRawNode>(raw_node) {
            Ok(mut node) => {
                if node.id.trim().is_empty() {
                    node.id = id.clone();
                }
                if mapping.len() >= MAX_MAPPING_NODES_PER_CONVERSATION {
                    tracing::warn!(
                        "ChatGPT mapping exceeded {} node(s); truncating",
                        MAX_MAPPING_NODES_PER_CONVERSATION
                    );
                    break;
                }
                mapping.insert(id, node);
            }
            Err(error) => {
                tracing::warn!("Skipping malformed ChatGPT mapping node {}: {}", id, error)
            }
        }
    }

    Ok(mapping)
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;

    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    use crate::cli::import::Importer;

    use super::ChatGptImporter;

    #[test]
    fn parses_linear_conversation_without_branches() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("chatgpt_export.zip");
        write_zip_with_file(
            &zip_path,
            "conversations.json",
            r#"
            [
              {
                "title": "Decorators",
                "create_time": 1705334400.0,
                "update_time": 1705334412.0,
                "conversation_id": "conv-1",
                "mapping": {
                  "root": {
                    "id": "root",
                    "message": {
                      "id": "m0",
                      "author": { "role": "system", "metadata": {} },
                      "content": { "content_type": "text", "parts": ["You are ChatGPT"] },
                      "create_time": 1705334400.0
                    },
                    "parent": null,
                    "children": ["u1"]
                  },
                  "u1": {
                    "id": "u1",
                    "message": {
                      "id": "m1",
                      "author": { "role": "user", "metadata": {} },
                      "content": { "content_type": "text", "parts": ["Explain decorators"] },
                      "create_time": 1705334405.0
                    },
                    "parent": "root",
                    "children": ["a1"]
                  },
                  "a1": {
                    "id": "a1",
                    "message": {
                      "id": "m2",
                      "author": { "role": "assistant", "metadata": {"model_slug":"gpt-4"} },
                      "content": { "content_type": "text", "parts": ["A decorator is..."] },
                      "create_time": 1705334412.0
                    },
                    "parent": "u1",
                    "children": []
                  }
                }
              }
            ]
            "#,
        );

        let importer = ChatGptImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].source_id, "conv-1");
        assert_eq!(conversations[0].title.as_deref(), Some("Decorators"));
        assert_eq!(conversations[0].messages.len(), 3);
        assert_eq!(conversations[0].messages[0].role, "system");
        assert_eq!(conversations[0].messages[1].role, "user");
        assert_eq!(conversations[0].messages[2].role, "assistant");
        assert_eq!(
            conversations[0]
                .source_metadata
                .get("model_slugs")
                .and_then(|value| value.as_array())
                .and_then(|values| values.first())
                .and_then(|value| value.as_str()),
            Some("gpt-4")
        );
    }

    #[test]
    fn follows_last_child_for_regenerated_branches() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("chatgpt_export.zip");
        write_zip_with_file(
            &zip_path,
            "conversations.json",
            r#"
            [
              {
                "conversation_id": "conv-branch",
                "mapping": {
                  "root": {
                    "id": "root",
                    "message": {
                      "id": "m0",
                      "author": { "role": "user", "metadata": {} },
                      "content": { "content_type": "text", "parts": ["question"] },
                      "create_time": 1705334400.0
                    },
                    "parent": null,
                    "children": ["a1", "a2"]
                  },
                  "a1": {
                    "id": "a1",
                    "message": {
                      "id": "m1",
                      "author": { "role": "assistant", "metadata": {} },
                      "content": { "content_type": "text", "parts": ["first answer"] },
                      "create_time": 1705334410.0
                    },
                    "parent": "root",
                    "children": []
                  },
                  "a2": {
                    "id": "a2",
                    "message": {
                      "id": "m2",
                      "author": { "role": "assistant", "metadata": {} },
                      "content": { "content_type": "text", "parts": ["final regenerated answer"] },
                      "create_time": 1705334412.0
                    },
                    "parent": "root",
                    "children": []
                  }
                }
              }
            ]
            "#,
        );

        let importer = ChatGptImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].messages.len(), 2);
        assert_eq!(
            conversations[0].messages[1].content,
            "final regenerated answer"
        );
    }

    #[test]
    fn skips_nodes_with_null_message_without_error() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("chatgpt_export.zip");
        write_zip_with_file(
            &zip_path,
            "conversations.json",
            r#"
            [
              {
                "conversation_id": "conv-null",
                "mapping": {
                  "root": {
                    "id": "root",
                    "message": {
                      "id": "m0",
                      "author": { "role": "system", "metadata": {} },
                      "content": { "content_type": "text", "parts": ["system"] },
                      "create_time": 1705334400.0
                    },
                    "parent": null,
                    "children": ["bridge"]
                  },
                  "bridge": {
                    "id": "bridge",
                    "message": null,
                    "parent": "root",
                    "children": ["u1"]
                  },
                  "u1": {
                    "id": "u1",
                    "message": {
                      "id": "m1",
                      "author": { "role": "user", "metadata": {} },
                      "content": { "content_type": "text", "parts": ["hello"] },
                      "create_time": 1705334405.0
                    },
                    "parent": "bridge",
                    "children": []
                  }
                }
              }
            ]
            "#,
        );

        let importer = ChatGptImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].messages.len(), 2);
        assert_eq!(conversations[0].messages[0].content, "system");
        assert_eq!(conversations[0].messages[1].content, "hello");
    }

    #[test]
    fn concatenates_multi_part_content_and_handles_non_string_parts() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("chatgpt_export.zip");
        write_zip_with_file(
            &zip_path,
            "conversations.json",
            r#"
            [
              {
                "conversation_id": "conv-parts",
                "mapping": {
                  "root": {
                    "id": "root",
                    "message": {
                      "id": "m0",
                      "author": { "role": "user", "metadata": {} },
                      "content": {
                        "content_type": "text",
                        "parts": [
                          "first",
                          {"content_type":"image_asset_pointer","asset_pointer":"file-1"},
                          "third"
                        ]
                      },
                      "create_time": 1705334405.0
                    },
                    "parent": null,
                    "children": []
                  }
                }
              }
            ]
            "#,
        );

        let importer = ChatGptImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].messages.len(), 1);
        assert_eq!(
            conversations[0].messages[0].content,
            "first\n\n[image]\n\nthird"
        );
    }

    #[test]
    fn skips_messages_with_empty_normalized_text() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("chatgpt_export.zip");
        write_zip_with_file(
            &zip_path,
            "conversations.json",
            r#"
            [
              {
                "conversation_id": "conv-empty-msg",
                "mapping": {
                  "root": {
                    "id": "root",
                    "message": {
                      "id": "m0",
                      "author": { "role": "user", "metadata": {} },
                      "content": { "content_type": "text", "parts": ["hello"] },
                      "create_time": 1705334400.0
                    },
                    "parent": null,
                    "children": ["a1"]
                  },
                  "a1": {
                    "id": "a1",
                    "message": {
                      "id": "m1",
                      "author": { "role": "assistant", "metadata": {} },
                      "content": { "content_type": "text", "parts": [{}] },
                      "create_time": 1705334410.0
                    },
                    "parent": "root",
                    "children": []
                  }
                }
              }
            ]
            "#,
        );

        let importer = ChatGptImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].messages.len(), 1);
        assert_eq!(
            conversations[0]
                .source_metadata
                .get("empty_text_messages")
                .and_then(|value| value.as_u64()),
            Some(1)
        );
    }

    #[test]
    fn handles_empty_mapping_and_timestamp_fallbacks() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("chatgpt_export.zip");
        write_zip_with_file(
            &zip_path,
            "conversations.json",
            r#"
            [
              {
                "conversation_id": "conv-empty",
                "title": "Empty",
                "create_time": 1705334400.5,
                "mapping": {}
              },
              {
                "conversation_id": "conv-fallback",
                "create_time": 1705334400.25,
                "mapping": {
                  "root": {
                    "id": "root",
                    "message": {
                      "id": "m0",
                      "author": { "role": "assistant", "metadata": {} },
                      "content": { "content_type": "text", "parts": ["fallback timestamp"] },
                      "create_time": null
                    },
                    "parent": null,
                    "children": []
                  }
                }
              }
            ]
            "#,
        );

        let importer = ChatGptImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 2);
        assert!(conversations[0].messages.is_empty());
        assert_eq!(conversations[1].messages.len(), 1);

        let ts = conversations[1].messages[0].created_at.clone();
        assert_eq!(ts.timestamp(), 1_705_334_400);
        assert_eq!(ts.timestamp_subsec_nanos(), 250_000_000);
    }

    #[test]
    fn parses_sharded_conversation_files_when_single_file_missing() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("chatgpt_export.zip");
        write_zip_with_files(
            &zip_path,
            &[
                (
                    "conversations-001.json",
                    r#"
                    [
                      {
                        "conversation_id": "conv-shard-2",
                        "mapping": {
                          "root": {
                            "id": "root",
                            "message": {
                              "id": "m2",
                              "author": { "role": "user", "metadata": {} },
                              "content": { "content_type": "text", "parts": ["second shard"] },
                              "create_time": 1705334500.0
                            },
                            "parent": null,
                            "children": []
                          }
                        }
                      }
                    ]
                    "#,
                ),
                (
                    "conversations-000.json",
                    r#"
                    [
                      {
                        "conversation_id": "conv-shard-1",
                        "mapping": {
                          "root": {
                            "id": "root",
                            "message": {
                              "id": "m1",
                              "author": { "role": "user", "metadata": {} },
                              "content": { "content_type": "text", "parts": ["first shard"] },
                              "create_time": 1705334400.0
                            },
                            "parent": null,
                            "children": []
                          }
                        }
                      }
                    ]
                    "#,
                ),
            ],
        );

        let importer = ChatGptImporter;
        let conversations = importer.parse(&zip_path).expect("parse");

        assert_eq!(conversations.len(), 2);
        assert_eq!(conversations[0].source_id, "conv-shard-1");
        assert_eq!(conversations[1].source_id, "conv-shard-2");
    }

    #[test]
    fn errors_when_no_conversation_json_files_exist() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("chatgpt_export.zip");
        write_zip_with_file(&zip_path, "chat.html", "<html></html>");

        let importer = ChatGptImporter;
        let err = importer.parse(&zip_path).expect_err("should fail");
        let err_text = format!("{}", err);
        assert!(err_text.contains("conversations"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn roundtrips_chatgpt_export_into_db_without_losing_messages_or_workspace_docs() {
        let temp = tempdir().expect("tempdir");
        let zip_path = temp.path().join("chatgpt_export.zip");
        write_zip_with_file(
            &zip_path,
            "conversations.json",
            r#"
            [
              {
                "title": "Roundtrip A",
                "create_time": 1705334400.0,
                "update_time": 1705334412.0,
                "conversation_id": "conv-roundtrip-a",
                "mapping": {
                  "root": {
                    "id": "root",
                    "message": {
                      "id": "m0",
                      "author": { "role": "system", "metadata": {} },
                      "content": { "content_type": "text", "parts": ["You are ChatGPT"] },
                      "create_time": 1705334400.0
                    },
                    "parent": null,
                    "children": ["u1"]
                  },
                  "u1": {
                    "id": "u1",
                    "message": {
                      "id": "m1",
                      "author": { "role": "user", "metadata": {} },
                      "content": { "content_type": "text", "parts": ["Explain decorators"] },
                      "create_time": 1705334405.0
                    },
                    "parent": "root",
                    "children": ["a1"]
                  },
                  "a1": {
                    "id": "a1",
                    "message": {
                      "id": "m2",
                      "author": { "role": "assistant", "metadata": {"model_slug":"gpt-4"} },
                      "content": { "content_type": "text", "parts": ["A decorator is..."] },
                      "create_time": 1705334412.0
                    },
                    "parent": "u1",
                    "children": []
                  }
                }
              },
              {
                "title": "Roundtrip A",
                "conversation_id": "conv-roundtrip-b",
                "mapping": {
                  "root": {
                    "id": "root",
                    "message": {
                      "id": "m10",
                      "author": { "role": "user", "metadata": {} },
                      "content": {
                        "content_type": "text",
                        "parts": ["first", {"content_type":"image_asset_pointer","asset_pointer":"file-1"}, "third"]
                      },
                      "create_time": 1705335400.0
                    },
                    "parent": null,
                    "children": []
                  }
                }
              }
            ]
            "#,
        );

        let importer = ChatGptImporter;
        let expected = importer.parse(&zip_path).expect("parse");
        let expected_count = expected.len();

        let (db, _tmp) = crate::testing::test_db().await;
        let user_id = "chatgpt-roundtrip";
        let args = crate::cli::import::HistoryImportArgs {
            path: Some(zip_path.clone()),
            user_id: user_id.to_string(),
            dedup: true,
            to_workspace: true,
            dry_run: false,
        };

        crate::cli::import::run_import_command_with_db(
            crate::cli::import::ImportSource::ChatGpt,
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
                    crate::cli::import::ImportSource::ChatGpt.source_key(),
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
            crate::cli::import::ImportSource::ChatGpt,
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
        let export_path = std::env::var("IRONCLAW_REAL_CHATGPT_EXPORT")
            .expect("IRONCLAW_REAL_CHATGPT_EXPORT must be set");
        let path = std::path::PathBuf::from(export_path);

        let importer = ChatGptImporter;
        let expected = importer.parse(&path).expect("parse");
        let expected_count = expected.len();

        let (db, _tmp) = crate::testing::test_db().await;
        let user_id = "chatgpt-real-export";
        let args = crate::cli::import::HistoryImportArgs {
            path: Some(path.clone()),
            user_id: user_id.to_string(),
            dedup: true,
            to_workspace: true,
            dry_run: false,
        };

        crate::cli::import::run_import_command_with_db(
            crate::cli::import::ImportSource::ChatGpt,
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
                    crate::cli::import::ImportSource::ChatGpt.source_key(),
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
        write_zip_with_files(path, &[(name, content)]);
    }

    fn write_zip_with_files(path: &std::path::Path, files: &[(&str, &str)]) {
        let file = File::create(path).expect("create zip");
        let mut writer = zip::ZipWriter::new(file);
        for (name, content) in files {
            writer
                .start_file(name, SimpleFileOptions::default())
                .expect("start zip file");
            writer
                .write_all(content.as_bytes())
                .expect("write zip payload");
        }
        writer.finish().expect("finish zip");
    }
}

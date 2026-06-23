//! IronClaw memory service contract for Reborn.
//!
//! This module owns the provider-neutral, host-facing IronClaw memory
//! operation shapes and the [`MemoryService`] trait. The default native
//! adapter and its storage behavior live in the `ironclaw_memory`
//! implementation crate.

use async_trait::async_trait;
use chrono_tz::Tz;
use ironclaw_host_api::{CorrelationId, ResourceScope};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::metadata::DocumentMetadata;

const MAX_LOCALE_LEN: usize = 35;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryInvocation {
    pub scope: ResourceScope,
    pub correlation_id: CorrelationId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryServiceSearchRequest {
    pub query: String,
    pub limit: usize,
}

impl MemoryServiceSearchRequest {
    pub fn from_tool_input(input: &Value) -> Result<Self, MemoryServiceError> {
        let query = search_query(input)?.to_string();
        let limit = optional_u64(input, "limit").unwrap_or(5).clamp(1, 20) as usize;
        Ok(Self { query, limit })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryServiceSearchResult {
    pub content: String,
    pub score: f32,
    pub path: String,
    pub is_hybrid_match: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryServiceSearchResponse {
    pub query: String,
    pub results: Vec<MemoryServiceSearchResult>,
}

impl MemoryServiceSearchResponse {
    pub fn result_count(&self) -> usize {
        self.results.len()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemoryServiceWriteRequest {
    pub target: String,
    pub content: String,
    pub append: bool,
    pub old_string: Option<String>,
    pub new_string: Option<String>,
    pub replace_all: bool,
    pub metadata: Option<DocumentMetadata>,
    pub timezone: Option<String>,
}

impl MemoryServiceWriteRequest {
    pub fn from_tool_input(input: &Value) -> Result<Self, MemoryServiceError> {
        let target = match input.get("target") {
            Some(Value::String(target)) => target.to_string(),
            Some(_) => return Err(MemoryServiceError::input()),
            None => "daily_log".to_string(),
        };
        let content = input
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let old_string = input
            .get("old_string")
            .and_then(Value::as_str)
            .map(str::to_string);
        let new_string = input
            .get("new_string")
            .and_then(Value::as_str)
            .map(str::to_string);
        let append = if target == "daily_log" {
            true
        } else {
            input.get("append").and_then(Value::as_bool).unwrap_or(true)
        };
        let metadata = input
            .get("metadata")
            .filter(|metadata| metadata.is_object())
            .map(DocumentMetadata::from_value);
        Ok(Self {
            target,
            content,
            append,
            old_string,
            new_string,
            replace_all: input
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            metadata,
            timezone: input
                .get("timezone")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryServiceWriteResponse {
    pub status: String,
    pub path: String,
    pub append: bool,
    pub content_length: usize,
    pub replacements: Option<usize>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryServiceReadRequest {
    pub path: String,
}

impl MemoryServiceReadRequest {
    pub fn from_tool_input(input: &Value) -> Result<Self, MemoryServiceError> {
        if input.get("version").is_some()
            || input.get("list_versions").and_then(Value::as_bool) == Some(true)
        {
            return Err(MemoryServiceError::input());
        }
        Ok(Self {
            path: required_str(input, "path")?.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryServiceReadResponse {
    pub path: String,
    pub content: String,
    pub word_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryServiceTreeRequest {
    pub path: String,
    pub depth: usize,
}

impl MemoryServiceTreeRequest {
    pub fn from_tool_input(input: &Value) -> Result<Self, MemoryServiceError> {
        let path = input
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let depth = optional_u64(input, "depth").unwrap_or(1).clamp(1, 10) as usize;
        Ok(Self { path, depth })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryServiceTreeResponse {
    pub entries: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryServiceProfileSetRequest {
    pub fields: Map<String, Value>,
}

impl MemoryServiceProfileSetRequest {
    pub fn from_tool_input(input: &Value) -> Result<Self, MemoryServiceError> {
        Ok(Self {
            fields: validated_profile_fields(input)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryServiceProfileSetResponse {
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryServiceContextRequest {
    pub query: String,
    pub max_snippets: usize,
    pub context_profile_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryServiceContextSnippet {
    pub snippet_ref: String,
    pub safe_summary: String,
    pub model_content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryServiceErrorKind {
    Input,
    Operation,
    Unavailable,
}

#[derive(Debug, thiserror::Error)]
#[error("IronClaw memory {kind:?}: {message}")]
pub struct MemoryServiceError {
    kind: MemoryServiceErrorKind,
    message: &'static str,
}

impl MemoryServiceError {
    pub fn input() -> Self {
        Self {
            kind: MemoryServiceErrorKind::Input,
            message: "invalid memory request",
        }
    }

    pub fn operation() -> Self {
        Self {
            kind: MemoryServiceErrorKind::Operation,
            message: "memory operation failed",
        }
    }

    pub fn unavailable() -> Self {
        Self {
            kind: MemoryServiceErrorKind::Unavailable,
            message: "memory provider unavailable",
        }
    }

    pub fn kind(&self) -> MemoryServiceErrorKind {
        self.kind
    }
}

#[async_trait]
pub trait MemoryService: Send + Sync {
    async fn search(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceSearchRequest,
    ) -> Result<MemoryServiceSearchResponse, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }

    async fn write(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceWriteRequest,
    ) -> Result<MemoryServiceWriteResponse, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }

    async fn read(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceReadRequest,
    ) -> Result<MemoryServiceReadResponse, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }

    async fn tree(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceTreeRequest,
    ) -> Result<MemoryServiceTreeResponse, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }

    async fn profile_set(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceProfileSetRequest,
    ) -> Result<MemoryServiceProfileSetResponse, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }

    async fn retrieve_context(
        &self,
        invocation: MemoryInvocation,
        request: MemoryServiceContextRequest,
    ) -> Result<Vec<MemoryServiceContextSnippet>, MemoryServiceError> {
        let _ = (invocation, request);
        Err(MemoryServiceError::unavailable())
    }
}

fn search_query(input: &Value) -> Result<&str, MemoryServiceError> {
    for key in ["query", "q", "text", "pattern"] {
        if let Some(value) = input.get(key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed);
            }
        }
    }
    Err(MemoryServiceError::input())
}

fn required_str<'a>(input: &'a Value, key: &'static str) -> Result<&'a str, MemoryServiceError> {
    input
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(MemoryServiceError::input)
}

fn optional_u64(input: &Value, key: &'static str) -> Option<u64> {
    input.get(key).and_then(Value::as_u64)
}

fn validated_profile_fields(input: &Value) -> Result<Map<String, Value>, MemoryServiceError> {
    let obj = input.as_object().ok_or_else(MemoryServiceError::input)?;
    let mut out = Map::new();
    for (key, value) in obj {
        match key.as_str() {
            "timezone" => {
                let value = value.as_str().ok_or_else(MemoryServiceError::input)?;
                value
                    .trim()
                    .parse::<Tz>()
                    .map_err(|_| MemoryServiceError::input())?;
                out.insert("timezone".into(), json!(value.trim()));
            }
            "locale" => {
                let value = value.as_str().ok_or_else(MemoryServiceError::input)?;
                validate_locale(value)?;
                out.insert("locale".into(), json!(value));
            }
            "location" => {
                let value = value.as_str().ok_or_else(MemoryServiceError::input)?.trim();
                if value.is_empty() || value.chars().count() > 200 || value.len() > 800 {
                    return Err(MemoryServiceError::input());
                }
                out.insert("location".into(), json!(value));
            }
            _ => return Err(MemoryServiceError::input()),
        }
    }
    if out.is_empty() {
        return Err(MemoryServiceError::input());
    }
    Ok(out)
}

fn validate_locale(value: &str) -> Result<(), MemoryServiceError> {
    if value.is_empty()
        || value.chars().count() > MAX_LOCALE_LEN
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        || value.split('-').any(str::is_empty)
    {
        return Err(MemoryServiceError::input());
    }
    Ok(())
}

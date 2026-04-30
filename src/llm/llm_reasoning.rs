//! Provider-agnostic extractor for the LLM's native reasoning channel.
//!
//! Different providers ship the model's chain-of-thought in different
//! places in the response JSON. Rather than hardcoding per-provider
//! switches, the per-provider knowledge lives in
//! [`llm_reasoning_extractors.json`] (config-as-data) and this module
//! walks the rig-core typed response — converted to a generic
//! `serde_json::Value` — applying the matching entry's rules.
//!
//! Adding support for a new provider is one new JSON entry; no Rust
//! change required.
//!
//! Three rule kinds cover every provider seen so far:
//!
//! * `field` — the reasoning is a flat string field on the assistant
//!   message (e.g. GLM/DeepSeek/Grok/Qwen `reasoning_content`,
//!   Kimi K2.6 `reasoning`).
//!
//! * `block` — the reasoning is a typed content block, identified
//!   either by `{type: "thinking"|"reasoning"}` (Anthropic, OpenAI
//!   o-series, Mistral 2509+) or by a boolean flag like
//!   `{thought: true}` (Gemini). The text lives in a sibling field.
//!   The text-field path supports a tiny `key[*].subkey` form so
//!   array-of-objects shapes (OpenAI `summary[*].text`,
//!   Mistral `thinking[*].text`) work without writing special cases.
//!
//! * `regex_in_content` — text is embedded in the assistant content
//!   string with delimiters (Mistral Magistral 2506 uses
//!   `<think>...</think>`). The pattern's first capture group is the
//!   reasoning.
//!
//! When multiple rules are listed for one provider they are tried in
//! order; the first non-empty result wins. This handles version drift
//! within a model family (e.g. Magistral 2506 vs 2509+, Kimi K2 vs
//! K2.6) without forcing a separate entry per version.

use std::sync::OnceLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const EXTRACTORS_JSON: &str = include_str!("llm_reasoning_extractors.json");

#[derive(Debug, Deserialize, Serialize)]
struct ExtractorEntryRaw {
    #[allow(dead_code)]
    name: String,
    model_match: String,
    rules: Vec<Rule>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Rule {
    /// Flat string field at any depth in the response.
    Field { field: String },
    /// Typed content block. Either:
    /// * `type_field` + `type_value` (e.g. `"type": "thinking"`), or
    /// * `flag_field` (boolean true on the block, e.g. `"thought": true`).
    Block {
        #[serde(default)]
        type_field: Option<String>,
        #[serde(default)]
        type_value: Option<String>,
        #[serde(default)]
        flag_field: Option<String>,
        /// Field on the matched block holding the reasoning text.
        /// Supports `key[*].subkey` for array-of-objects shapes.
        text_field: String,
    },
    /// Regex against any string field named `content`. Capture group 1
    /// is the reasoning text. Multiple matches are concatenated.
    RegexInContent { pattern: String },
}

struct CompiledEntry {
    model_re: Regex,
    rules: Vec<CompiledRule>,
}

enum CompiledRule {
    Field(String),
    Block {
        type_field: Option<String>,
        type_value: Option<String>,
        flag_field: Option<String>,
        text_field_outer: String,
        text_field_inner: Option<String>,
    },
    Regex(Regex),
}

fn compiled_entries() -> &'static [CompiledEntry] {
    static CACHE: OnceLock<Vec<CompiledEntry>> = OnceLock::new();
    CACHE.get_or_init(|| {
        let raw: Vec<ExtractorEntryRaw> = serde_json::from_str(EXTRACTORS_JSON)
            .expect("llm_reasoning_extractors.json is malformed; fix the file");
        raw.into_iter()
            .filter_map(|entry| {
                let model_re = match Regex::new(&entry.model_match) {
                    Ok(re) => re,
                    Err(e) => {
                        tracing::warn!(
                            entry = %entry.name,
                            pattern = %entry.model_match,
                            error = %e,
                            "skipping reasoning extractor entry: invalid model_match regex",
                        );
                        return None;
                    }
                };
                let rules = entry
                    .rules
                    .into_iter()
                    .filter_map(|r| compile_rule(r, &entry.name))
                    .collect();
                Some(CompiledEntry { model_re, rules })
            })
            .collect()
    })
}

fn compile_rule(rule: Rule, entry_name: &str) -> Option<CompiledRule> {
    match rule {
        Rule::Field { field } => Some(CompiledRule::Field(field)),
        Rule::Block {
            type_field,
            type_value,
            flag_field,
            text_field,
        } => {
            let (outer, inner) = match text_field.split_once("[*].") {
                Some((o, i)) => (o.to_string(), Some(i.to_string())),
                None => (text_field, None),
            };
            Some(CompiledRule::Block {
                type_field,
                type_value,
                flag_field,
                text_field_outer: outer,
                text_field_inner: inner,
            })
        }
        Rule::RegexInContent { pattern } => match Regex::new(&pattern) {
            Ok(re) => Some(CompiledRule::Regex(re)),
            Err(e) => {
                tracing::warn!(
                    entry = %entry_name,
                    pattern = %pattern,
                    error = %e,
                    "skipping regex_in_content rule: invalid pattern",
                );
                None
            }
        },
    }
}

/// Extract reasoning text from a provider response.
///
/// Returns `None` when no entry matches the model name, no rule
/// produces text, or the resulting text is empty.
pub fn extract_reasoning(model: &str, raw_response: &Value) -> Option<String> {
    let entry = compiled_entries()
        .iter()
        .find(|e| e.model_re.is_match(model))?;

    for rule in &entry.rules {
        let mut chunks: Vec<String> = Vec::new();
        match rule {
            CompiledRule::Field(name) => collect_field(raw_response, name, &mut chunks),
            CompiledRule::Block {
                type_field,
                type_value,
                flag_field,
                text_field_outer,
                text_field_inner,
            } => collect_block(
                raw_response,
                type_field.as_deref(),
                type_value.as_deref(),
                flag_field.as_deref(),
                text_field_outer,
                text_field_inner.as_deref(),
                &mut chunks,
            ),
            CompiledRule::Regex(re) => collect_regex(raw_response, re, &mut chunks),
        }
        let chunks: Vec<String> = chunks
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !chunks.is_empty() {
            return Some(chunks.join("\n\n"));
        }
    }
    None
}

fn collect_field(v: &Value, name: &str, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            if let Some(s) = map.get(name).and_then(|x| x.as_str()) {
                out.push(s.to_string());
            }
            for (_, child) in map {
                collect_field(child, name, out);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                collect_field(child, name, out);
            }
        }
        _ => {}
    }
}

fn collect_block(
    v: &Value,
    type_field: Option<&str>,
    type_value: Option<&str>,
    flag_field: Option<&str>,
    text_outer: &str,
    text_inner: Option<&str>,
    out: &mut Vec<String>,
) {
    match v {
        Value::Object(map) => {
            let by_type = match (type_field, type_value) {
                (Some(tf), Some(tv)) => map.get(tf).and_then(|x| x.as_str()) == Some(tv),
                _ => false,
            };
            let by_flag = match flag_field {
                Some(ff) => map.get(ff).and_then(|x| x.as_bool()) == Some(true),
                None => false,
            };
            if by_type || by_flag {
                if let Some(text_value) = map.get(text_outer) {
                    extract_text(text_value, text_inner, out);
                }
            }
            for (_, child) in map {
                collect_block(
                    child, type_field, type_value, flag_field, text_outer, text_inner, out,
                );
            }
        }
        Value::Array(arr) => {
            for child in arr {
                collect_block(
                    child, type_field, type_value, flag_field, text_outer, text_inner, out,
                );
            }
        }
        _ => {}
    }
}

fn extract_text(v: &Value, inner: Option<&str>, out: &mut Vec<String>) {
    match (v, inner) {
        (Value::String(s), None) => out.push(s.clone()),
        (Value::Array(arr), Some(inner_key)) => {
            for item in arr {
                if let Some(s) = item.get(inner_key).and_then(|x| x.as_str()) {
                    out.push(s.to_string());
                }
            }
        }
        (Value::Array(arr), None) => {
            for item in arr {
                if let Some(s) = item.as_str() {
                    out.push(s.to_string());
                }
            }
        }
        _ => {}
    }
}

fn collect_regex(v: &Value, re: &Regex, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            if let Some(s) = map.get("content").and_then(|x| x.as_str()) {
                for caps in re.captures_iter(s) {
                    if let Some(m) = caps.get(1) {
                        out.push(m.as_str().to_string());
                    }
                }
            }
            for (_, child) in map {
                collect_regex(child, re, out);
            }
        }
        Value::Array(arr) => {
            for child in arr {
                collect_regex(child, re, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn config_loads() {
        let entries = compiled_entries();
        assert!(
            entries.len() >= 8,
            "expected at least 8 provider entries, got {}",
            entries.len()
        );
    }

    #[test]
    fn glm_flat_field() {
        let raw = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "reasoning_content": "Let me think...",
                    "content": "Final answer."
                }
            }]
        });
        assert_eq!(
            extract_reasoning("glm-5", &raw),
            Some("Let me think...".to_string())
        );
    }

    #[test]
    fn anthropic_thinking_block() {
        let raw = json!({
            "content": [
                { "type": "thinking", "thinking": "Step 1, step 2." },
                { "type": "text", "text": "Done." }
            ]
        });
        assert_eq!(
            extract_reasoning("claude-sonnet-4-5", &raw),
            Some("Step 1, step 2.".to_string())
        );
    }

    #[test]
    fn openai_o_series_summary_array() {
        let raw = json!({
            "output": [
                { "type": "reasoning", "id": "r_1", "summary": [
                    { "type": "summary_text", "text": "thinking part 1" },
                    { "type": "summary_text", "text": "thinking part 2" }
                ]},
                { "type": "message", "content": [{ "type": "output_text", "text": "answer" }] }
            ]
        });
        assert_eq!(
            extract_reasoning("o3-mini", &raw),
            Some("thinking part 1\n\nthinking part 2".to_string())
        );
    }

    #[test]
    fn gemini_thought_flag() {
        let raw = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "thought": true, "text": "internal" },
                        { "text": "external" }
                    ]
                }
            }]
        });
        assert_eq!(
            extract_reasoning("gemini-2.5-pro", &raw),
            Some("internal".to_string())
        );
    }

    #[test]
    fn mistral_magistral_2509_typed() {
        let raw = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": [
                        { "type": "thinking", "thinking": [
                            { "type": "text", "text": "trace" }
                        ]},
                        { "type": "text", "text": "answer" }
                    ]
                }
            }]
        });
        assert_eq!(
            extract_reasoning("magistral-medium-1.2", &raw),
            Some("trace".to_string())
        );
    }

    #[test]
    fn mistral_magistral_2506_inline_tags() {
        let raw = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "<think>inline thoughts</think>actual answer"
                }
            }]
        });
        assert_eq!(
            extract_reasoning("magistral-medium-2506", &raw),
            Some("inline thoughts".to_string())
        );
    }

    #[test]
    fn kimi_k2_thinking_uses_reasoning_content() {
        let raw = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "reasoning_content": "kimi trace",
                    "content": "answer"
                }
            }]
        });
        assert_eq!(
            extract_reasoning("kimi-k2-thinking", &raw),
            Some("kimi trace".to_string())
        );
    }

    #[test]
    fn kimi_k2_6_uses_reasoning() {
        let raw = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "reasoning": "kimi 2.6 trace",
                    "content": "answer"
                }
            }]
        });
        assert_eq!(
            extract_reasoning("kimi-k2.6", &raw),
            Some("kimi 2.6 trace".to_string())
        );
    }

    #[test]
    fn unknown_model_returns_none() {
        let raw = json!({ "anything": "here" });
        assert_eq!(extract_reasoning("some-novel-model-9000", &raw), None);
    }

    #[test]
    fn empty_reasoning_returns_none() {
        let raw = json!({
            "choices": [{ "message": { "reasoning_content": "" }}]
        });
        assert_eq!(extract_reasoning("glm-5", &raw), None);
    }
}

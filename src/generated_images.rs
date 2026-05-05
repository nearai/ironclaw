//! Shared helpers for image-generation sentinel payloads.

use std::borrow::Cow;

pub(crate) const MAX_RECORDED_IMAGE_SENTINEL_BYTES: usize = 512 * 1024;
const MAX_EMBEDDED_JSON_STRING_LAYERS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GeneratedImageSentinel {
    pub(crate) value: serde_json::Value,
}

pub(crate) fn recorded_image_sentinel_cap_label() -> String {
    if MAX_RECORDED_IMAGE_SENTINEL_BYTES.is_multiple_of(1024 * 1024) {
        return format!("{} MiB", MAX_RECORDED_IMAGE_SENTINEL_BYTES / (1024 * 1024));
    }
    if MAX_RECORDED_IMAGE_SENTINEL_BYTES.is_multiple_of(1024) {
        return format!("{} KiB", MAX_RECORDED_IMAGE_SENTINEL_BYTES / 1024);
    }
    format!("{} bytes", MAX_RECORDED_IMAGE_SENTINEL_BYTES)
}

impl GeneratedImageSentinel {
    pub(crate) fn from_output(output: &str) -> Option<Self> {
        let parsed = serde_json::from_str::<serde_json::Value>(output).ok()?;
        Self::from_value(&parsed)
    }

    pub(crate) fn from_value(value: &serde_json::Value) -> Option<Self> {
        let value = normalize_embedded_json(value)?;
        if value.get("type").and_then(|v| v.as_str()) != Some("image_generated") {
            return None;
        }
        Some(Self {
            value: value.into_owned(),
        })
    }

    pub(crate) fn data_url(&self) -> Option<&str> {
        self.value.get("data").and_then(|v| v.as_str())
    }

    pub(crate) fn media_type(&self) -> Option<&str> {
        self.value
            .get("media_type")
            .or_else(|| self.value.get("mime_type"))
            .and_then(|v| v.as_str())
    }

    pub(crate) fn path(&self) -> Option<&str> {
        self.value.get("path").and_then(|v| v.as_str())
    }

    pub(crate) fn summary_for_context(&self) -> String {
        let media_type = self.media_type().unwrap_or("image");
        format!("Generated image ({media_type})")
    }

    pub(crate) fn compact_value_without_data_url(&self) -> serde_json::Value {
        let mut summary = serde_json::Map::new();
        summary.insert(
            "type".to_string(),
            serde_json::Value::String("image_generated".to_string()),
        );
        if let Some(media_type) = self.media_type() {
            summary.insert(
                "media_type".to_string(),
                serde_json::Value::String(media_type.to_string()),
            );
        }
        if let Some(path) = self.path()
            && !path.is_empty()
        {
            summary.insert(
                "path".to_string(),
                serde_json::Value::String(path.to_string()),
            );
        }
        summary.insert("data_omitted".to_string(), serde_json::Value::Bool(true));
        summary.insert(
            "omitted_reason".to_string(),
            serde_json::Value::String(format!(
                "exceeded the {} cap",
                recorded_image_sentinel_cap_label()
            )),
        );
        serde_json::Value::Object(summary)
    }

    fn content_with_omitted_data_url_when_oversized(&self) -> String {
        let normalized = self.value.to_string();
        if normalized.len() <= MAX_RECORDED_IMAGE_SENTINEL_BYTES {
            return normalized;
        }
        self.compact_value_without_data_url().to_string()
    }

    pub(crate) fn record_content_for_persistence(&self) -> String {
        self.content_with_omitted_data_url_when_oversized()
    }

    pub(crate) fn record_content_for_thread_state(&self) -> String {
        self.content_with_omitted_data_url_when_oversized()
    }
}

fn normalize_embedded_json(value: &serde_json::Value) -> Option<Cow<'_, serde_json::Value>> {
    let serde_json::Value::String(s) = value else {
        return Some(Cow::Borrowed(value));
    };

    let mut current = serde_json::from_str::<serde_json::Value>(s).ok()?;
    // Generated-image sentinels may be serialized more than once as they flow
    // through tool output, DB persistence, and history reconstruction. Unwrap a
    // few layers to tolerate that pipeline, but stop after a small fixed number
    // of rounds so malformed input cannot trigger unbounded reparsing.
    for _ in 1..MAX_EMBEDDED_JSON_STRING_LAYERS {
        match current {
            serde_json::Value::String(ref s) => {
                current = serde_json::from_str::<serde_json::Value>(s).ok()?;
            }
            _ => return Some(Cow::Owned(current)),
        }
    }
    if matches!(current, serde_json::Value::String(_)) {
        tracing::debug!(
            max_layers = MAX_EMBEDDED_JSON_STRING_LAYERS,
            "Generated image sentinel remained stringified after max unwrapping rounds"
        );
    }
    Some(Cow::Owned(current))
}

#[cfg(test)]
mod tests {
    use super::{GeneratedImageSentinel, MAX_RECORDED_IMAGE_SENTINEL_BYTES};

    fn double_stringified_sentinel_under_normalized_cap() -> (String, String) {
        let base = serde_json::json!({
            "type": "image_generated",
            "data": "data:image/png;base64,",
            "media_type": "image/png",
            "path": "workspace/out.png",
        })
        .to_string();
        let filler_len = MAX_RECORDED_IMAGE_SENTINEL_BYTES - base.len();
        let normalized = serde_json::json!({
            "type": "image_generated",
            "data": format!("data:image/png;base64,{}", "a".repeat(filler_len)),
            "media_type": "image/png",
            "path": "workspace/out.png",
        })
        .to_string();
        let wrapped = serde_json::to_string(&normalized).unwrap();

        assert_eq!(normalized.len(), MAX_RECORDED_IMAGE_SENTINEL_BYTES);
        assert!(wrapped.len() > MAX_RECORDED_IMAGE_SENTINEL_BYTES);

        (normalized, wrapped)
    }

    #[test]
    fn parses_double_stringified_sentinel() {
        let sentinel = serde_json::json!({
            "type": "image_generated",
            "data": "data:image/jpeg;base64,abc123",
            "media_type": "image/jpeg",
        })
        .to_string();
        let wrapped = serde_json::to_string(&sentinel).unwrap();

        let parsed = GeneratedImageSentinel::from_output(&wrapped).expect("sentinel");
        assert_eq!(parsed.data_url(), Some("data:image/jpeg;base64,abc123"));
        assert_eq!(parsed.media_type(), Some("image/jpeg"));
    }

    #[test]
    fn parses_triple_stringified_sentinel() {
        let sentinel = serde_json::json!({
            "type": "image_generated",
            "data": "data:image/jpeg;base64,abc123",
            "media_type": "image/jpeg",
        })
        .to_string();
        let wrapped = serde_json::to_string(&sentinel).unwrap();
        let triple_wrapped = serde_json::to_string(&wrapped).unwrap();

        let parsed = GeneratedImageSentinel::from_output(&triple_wrapped).expect("sentinel");
        assert_eq!(parsed.data_url(), Some("data:image/jpeg;base64,abc123"));
        assert_eq!(parsed.media_type(), Some("image/jpeg"));
    }

    #[test]
    fn summarizes_sentinel_for_context_without_data_url() {
        let sentinel = GeneratedImageSentinel::from_value(&serde_json::json!({
            "type": "image_generated",
            "data": "data:image/png;base64,abc123",
            "media_type": "image/png",
            "path": "workspace/out.png",
        }))
        .expect("sentinel");

        assert_eq!(
            sentinel.summary_for_context(),
            "Generated image (image/png)"
        );
    }

    #[test]
    fn record_content_for_thread_state_omits_large_data_url() {
        let oversized = "a".repeat(MAX_RECORDED_IMAGE_SENTINEL_BYTES);
        let sentinel = GeneratedImageSentinel::from_value(&serde_json::json!({
            "type": "image_generated",
            "data": format!("data:image/png;base64,{oversized}"),
            "media_type": "image/png",
            "path": "workspace/out.png",
        }))
        .expect("sentinel");

        let recorded = sentinel.record_content_for_thread_state();

        assert!(!recorded.contains("data:image/png;base64"));
        assert!(recorded.contains("\"type\":\"image_generated\""));
        assert!(recorded.contains("\"data_omitted\":true"));
        assert!(recorded.contains("\"path\":\"workspace/out.png\""));
    }

    #[test]
    fn record_content_for_thread_state_preserves_double_stringified_sentinel_under_cap() {
        let (normalized, wrapped) = double_stringified_sentinel_under_normalized_cap();
        let sentinel = GeneratedImageSentinel::from_output(&wrapped).expect("sentinel");

        let recorded = sentinel.record_content_for_thread_state();

        assert_eq!(recorded, normalized);
        assert!(recorded.contains("data:image/png;base64"));
        assert!(!recorded.contains("\"data_omitted\":true"));
    }
}

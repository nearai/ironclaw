// The compile-time ceiling is no longer referenced here: validation now bounds against
// `contract::effective_tool_result_read_max_bytes()`, which applies the env override.
use serde_json::{Map, Value};
use thiserror::Error;

use ironclaw_host_api::model_result_preview::{
    ModelResultJsonNextRequest, ModelResultJsonNodeType, ModelResultJsonOffsetUnit,
    ModelResultJsonOmittedArray, ModelResultJsonOmittedDescriptor, ModelResultJsonOmittedObject,
    ModelResultJsonPage, ModelResultJsonPageView, ModelResultPreview,
    json_field_name_requires_redaction,
};

use crate::{
    SessionThreadError, ToolResultRecordChunk, ToolResultRecordRead, ToolResultRecordSelection,
    ToolResultReferenceEnvelope,
};

pub(crate) const TOOL_RESULT_RECORD_MAX_BYTES: usize = 4 * 1024 * 1024;
const TOOL_RESULT_RECORD_READ_MIN_BYTES: usize = 4;

pub const TOOL_RESULT_JSON_DEFAULT_LIMIT: usize = 25;
pub const TOOL_RESULT_JSON_MAX_LIMIT: usize = 100;
/// Apply the existing model-preview redaction contract to the complete page.
/// The page is a read-time projection, so redaction never mutates durable bytes.
pub fn model_result_preview_from_json_page(
    page: &ModelResultJsonPage,
) -> Result<ModelResultPreview, String> {
    let mut page = page.clone();
    let selected_sensitive = json_pointer_is_sensitive(&page.json_pointer);
    if selected_sensitive {
        page.content = Value::String("[redacted]".to_string());
        page.node_type = ModelResultJsonNodeType::String;
        page.offset = 0;
        page.offset_unit = ModelResultJsonOffsetUnit::Bytes;
        page.omitted.clear();
        page.next_offset = None;
        page.next = None;
    } else {
        redact_json_page_content(&mut page.content)?;
        redact_omitted_descriptors(&mut page.omitted);
    }
    ModelResultPreview::from_redacted_json_page(page).map_err(|error| error.to_string())
}

fn json_pointer_is_sensitive(pointer: &str) -> bool {
    pointer.split('/').skip(1).any(|segment| {
        let decoded = segment.replace("~1", "/").replace("~0", "~");
        json_field_name_requires_redaction(&decoded)
    })
}

fn redact_json_page_content(content: &mut Value) -> Result<(), String> {
    match content {
        Value::Object(fields) => {
            for (key, value) in fields {
                if json_field_name_requires_redaction(key) {
                    *value = Value::String("[redacted]".to_string());
                } else {
                    redact_json_page_content(value)?;
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_json_page_content(value)?;
            }
        }
        Value::String(text) => {
            *text = ModelResultPreview::redacted(text.clone())
                .map_err(|error| error.to_string())?
                .into_inner();
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

fn redact_omitted_descriptors(omitted: &mut [ModelResultJsonOmittedDescriptor]) {
    for descriptor in omitted {
        match descriptor {
            ModelResultJsonOmittedDescriptor::Object(descriptor) => {
                if json_field_name_requires_redaction(&descriptor.key)
                    || json_pointer_is_sensitive(&descriptor.json_pointer)
                {
                    descriptor.key = "[redacted]".to_string();
                    descriptor.json_pointer = "[redacted]".to_string();
                }
            }
            ModelResultJsonOmittedDescriptor::Array(descriptor) => {
                if json_pointer_is_sensitive(&descriptor.json_pointer) {
                    descriptor.json_pointer = "[redacted]".to_string();
                }
            }
        }
    }
}

/// Errors in the model-selected JSON view are separate from storage failures
/// and malformed durable records so the loop host can give the model useful
/// repair guidance without hiding a backend outage.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ToolResultRecordReadError {
    #[error("invalid JSON Pointer")]
    InvalidJsonPointer { pointer: String },
    #[error("JSON Pointer does not identify a value")]
    JsonPointerNotFound { pointer: String },
    #[error("JSON view offset is outside the selected value")]
    InvalidJsonOffset { offset: u64 },
    #[error("JSON view limit must be between 1 and {max}")]
    InvalidJsonLimit { limit: usize, max: usize },
    #[error("JSON view limit is valid only for object or array selections")]
    JsonLimitRequiresCollection,
    #[error("JSON view byte budget must not exceed {max}")]
    InvalidJsonBudget { max_bytes: usize, max: usize },
    #[error("JSON view cannot fit in the requested budget")]
    JsonViewBudgetTooSmall { max_bytes: usize },
    #[error("stored tool result is malformed JSON: {reason}")]
    MalformedStoredJson { reason: String },
    #[error("stored tool result exceeds the durable storage limit")]
    StoredResultTooLarge,
}

pub(crate) fn validate_tool_result_record_ref(result_ref: &str) -> Result<(), SessionThreadError> {
    ToolResultReferenceEnvelope::validate_result_ref(result_ref)
        .map_err(SessionThreadError::Serialization)
}

pub(crate) fn validate_tool_result_record_content(
    content: &[u8],
) -> Result<(), SessionThreadError> {
    if content.len() > TOOL_RESULT_RECORD_MAX_BYTES {
        return Err(SessionThreadError::Backend(
            "tool result record exceeds the durable storage limit".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_tool_result_record_read(max_bytes: usize) -> Result<(), SessionThreadError> {
    // Upper bound is the EFFECTIVE cap, not the compile-time ceiling: 64 KiB by
    // default, retunable via IRONCLAW_TOOL_RESULT_READ_MAX_BYTES so a large file can be
    // pulled in one read instead of a paging loop (see contract.rs for the measurement).
    let ceiling = crate::contract::effective_tool_result_read_max_bytes();
    if !(TOOL_RESULT_RECORD_READ_MIN_BYTES..=ceiling).contains(&max_bytes) {
        return Err(SessionThreadError::Serialization(
            "tool result record read size is outside the supported range".to_string(),
        ));
    }
    Ok(())
}

/// Read one stored result using either the historical opaque-byte behavior or
/// a bounded JSON selected view. Both backends call this exact helper after
/// they have performed their scope-bound record lookup.
pub(crate) fn read_tool_result_record(
    content: &[u8],
    result_ref: &str,
    offset: u64,
    max_bytes: usize,
    selection: &ToolResultRecordSelection,
) -> Result<ToolResultRecordRead, SessionThreadError> {
    validate_tool_result_record_read(max_bytes)?;
    if content.len() > TOOL_RESULT_RECORD_MAX_BYTES {
        return Err(SessionThreadError::ToolResultRecordRead(
            ToolResultRecordReadError::StoredResultTooLarge,
        ));
    }
    match selection {
        ToolResultRecordSelection::Bytes => Ok(ToolResultRecordRead::Bytes(
            tool_result_record_chunk(content, offset, max_bytes),
        )),
        ToolResultRecordSelection::Json { pointer, limit } => Ok(ToolResultRecordRead::Json(
            render_json_tool_result_page(result_ref, content, pointer, offset, max_bytes, *limit)
                .map_err(SessionThreadError::ToolResultRecordRead)?,
        )),
    }
}

/// Render a shallow, bounded JSON view over a durable result.
///
/// The selected node itself is never recursively reduced. Objects and arrays
/// expose immediate children that fit and identify omitted children by their
/// exact JSON Pointer, allowing the model to select one deliberately on a
/// later call. The returned value is already the model-facing page envelope.
pub fn render_json_tool_result_page(
    result_ref: &str,
    serialized: &[u8],
    pointer: &str,
    offset: u64,
    max_bytes: usize,
    limit: Option<usize>,
) -> Result<ModelResultJsonPage, ToolResultRecordReadError> {
    validate_json_pointer(pointer)?;
    if let Some(limit) = limit
        && !(1..=TOOL_RESULT_JSON_MAX_LIMIT).contains(&limit)
    {
        return Err(ToolResultRecordReadError::InvalidJsonLimit {
            limit,
            max: TOOL_RESULT_JSON_MAX_LIMIT,
        });
    }
    let document = serde_json::from_slice::<Value>(serialized).map_err(|error| {
        ToolResultRecordReadError::MalformedStoredJson {
            reason: error.to_string(),
        }
    })?;
    let selected = document.pointer(pointer).ok_or_else(|| {
        ToolResultRecordReadError::JsonPointerNotFound {
            pointer: pointer.to_string(),
        }
    })?;
    let total_bytes = u64::try_from(serialized.len()).unwrap_or(u64::MAX);
    // JSON pages use the existing model-preview vehicle. Raw byte reads retain
    // their independently tunable ceiling, but silently truncating a JSON
    // request here would make the public continuation contract dishonest.
    let model_max = ironclaw_host_api::model_result_preview::MODEL_RESULT_PREVIEW_MAX_BYTES;
    if max_bytes > model_max {
        return Err(ToolResultRecordReadError::InvalidJsonBudget {
            max_bytes,
            max: model_max,
        });
    }
    let budget = max_bytes;

    // A credential-labeled selection is a terminal redacted view. Build it
    // before materializing collection children so a large sensitive object or
    // array cannot consume the page budget (or cross the model boundary) only
    // to be replaced during final preview redaction.
    if json_pointer_is_sensitive(pointer) {
        return ensure_page_fits(
            PageEnvelopeControls {
                result_ref,
                pointer,
                node_type: ModelResultJsonNodeType::String,
                offset: 0,
                offset_unit: ModelResultJsonOffsetUnit::Bytes,
                total_bytes,
                max_bytes: budget,
                limit: None,
            }
            .page(Value::String("[redacted]".to_string()), Vec::new(), None),
            budget,
        );
    }

    match selected {
        Value::Object(object) => render_object_page(
            result_ref,
            pointer,
            object,
            offset,
            budget,
            limit.unwrap_or(TOOL_RESULT_JSON_DEFAULT_LIMIT),
            total_bytes,
        ),
        Value::Array(array) => render_array_page(
            result_ref,
            pointer,
            array,
            offset,
            budget,
            limit.unwrap_or(TOOL_RESULT_JSON_DEFAULT_LIMIT),
            total_bytes,
        ),
        Value::String(string) => {
            if limit.is_some() {
                return Err(ToolResultRecordReadError::JsonLimitRequiresCollection);
            }
            render_string_page(result_ref, pointer, string, offset, budget, total_bytes)
        }
        scalar => {
            if limit.is_some() {
                return Err(ToolResultRecordReadError::JsonLimitRequiresCollection);
            }
            if offset != 0 {
                return Err(ToolResultRecordReadError::InvalidJsonOffset { offset });
            }
            let page = PageEnvelopeControls {
                result_ref,
                pointer,
                node_type: node_type(scalar),
                offset,
                offset_unit: ModelResultJsonOffsetUnit::Value,
                total_bytes,
                max_bytes: budget,
                limit: None,
            }
            .page(scalar.clone(), Vec::new(), None);
            ensure_page_fits(page, budget)
        }
    }
}

fn render_object_page(
    result_ref: &str,
    pointer: &str,
    object: &Map<String, Value>,
    offset: u64,
    budget: usize,
    limit: usize,
    total_bytes: u64,
) -> Result<ModelResultJsonPage, ToolResultRecordReadError> {
    let controls = PageEnvelopeControls {
        result_ref,
        pointer,
        node_type: ModelResultJsonNodeType::Object,
        offset,
        offset_unit: ModelResultJsonOffsetUnit::Items,
        total_bytes,
        max_bytes: budget,
        limit: Some(limit),
    };
    let start = checked_collection_offset(offset, object.len())?;
    let end = start.saturating_add(limit).min(object.len());
    let mut content = Map::new();
    let mut omitted = Vec::new();
    let mut next_offset = None;
    for (index, (key, value)) in object.iter().skip(start).take(end - start).enumerate() {
        let absolute_index = start + index;
        let reserved_next = (absolute_index + 1 < object.len())
            .then(|| u64::try_from(object.len()).unwrap_or(u64::MAX));
        let candidate = {
            let mut candidate_content = content.clone();
            candidate_content.insert(key.clone(), value.clone());
            controls.page(
                Value::Object(candidate_content),
                omitted.clone(),
                reserved_next,
            )
        };
        if page_fits(&candidate, budget) {
            content.insert(key.clone(), value.clone());
            continue;
        }
        let descriptor = omitted_descriptor(key_pointer(pointer, key), key, value, None);
        let omitted_candidate = controls.page(
            Value::Object(content.clone()),
            {
                let mut values = omitted.clone();
                values.push(descriptor.clone());
                values
            },
            reserved_next,
        );
        if page_fits(&omitted_candidate, budget) {
            omitted.push(descriptor);
            continue;
        }
        // The provider-controlled key itself can be larger than the entire
        // page budget, making even its omission descriptor unrepresentable.
        // Skip that one child so continuation always makes progress and later
        // object fields remain reachable.
        next_offset = Some((absolute_index + 1) as u64);
        break;
    }
    if next_offset.is_none() && end < object.len() {
        next_offset = Some(end as u64);
    }
    ensure_page_fits(
        controls.page(Value::Object(content), omitted, next_offset),
        budget,
    )
}

fn render_array_page(
    result_ref: &str,
    pointer: &str,
    array: &[Value],
    offset: u64,
    budget: usize,
    limit: usize,
    total_bytes: u64,
) -> Result<ModelResultJsonPage, ToolResultRecordReadError> {
    let controls = PageEnvelopeControls {
        result_ref,
        pointer,
        node_type: ModelResultJsonNodeType::Array,
        offset,
        offset_unit: ModelResultJsonOffsetUnit::Items,
        total_bytes,
        max_bytes: budget,
        limit: Some(limit),
    };
    let start = checked_collection_offset(offset, array.len())?;
    let end = start.saturating_add(limit).min(array.len());
    let mut content = Vec::new();
    let mut omitted = Vec::new();
    let mut next_offset = None;
    for (index, value) in array.iter().enumerate().skip(start).take(end - start) {
        let reserved_next =
            (index + 1 < array.len()).then(|| u64::try_from(array.len()).unwrap_or(u64::MAX));
        let candidate = {
            let mut candidate_content = content.clone();
            candidate_content.push(value.clone());
            controls.page(
                Value::Array(candidate_content),
                omitted.clone(),
                reserved_next,
            )
        };
        if page_fits(&candidate, budget) {
            content.push(value.clone());
            continue;
        }
        let descriptor = omitted_descriptor(
            key_pointer(pointer, &index.to_string()),
            &index.to_string(),
            value,
            Some(index),
        );
        let omitted_candidate = controls.page(
            Value::Array(content.clone()),
            {
                let mut values = omitted.clone();
                values.push(descriptor.clone());
                values
            },
            reserved_next,
        );
        if page_fits(&omitted_candidate, budget) {
            omitted.push(descriptor);
            continue;
        }
        // Keep collection continuation monotonic even when the envelope plus
        // this omission descriptor cannot fit in the caller's budget.
        next_offset = Some((index + 1) as u64);
        break;
    }
    if next_offset.is_none() && end < array.len() {
        next_offset = Some(end as u64);
    }
    ensure_page_fits(
        controls.page(Value::Array(content), omitted, next_offset),
        budget,
    )
}

fn render_string_page(
    result_ref: &str,
    pointer: &str,
    string: &str,
    offset: u64,
    budget: usize,
    total_bytes: u64,
) -> Result<ModelResultJsonPage, ToolResultRecordReadError> {
    let controls = PageEnvelopeControls {
        result_ref,
        pointer,
        node_type: ModelResultJsonNodeType::String,
        offset,
        offset_unit: ModelResultJsonOffsetUnit::Bytes,
        total_bytes,
        max_bytes: budget,
        limit: None,
    };
    let requested_start = usize::try_from(offset).map_err(|error| {
        tracing::debug!(offset, error = %error, "JSON string offset does not fit in usize");
        ToolResultRecordReadError::InvalidJsonOffset { offset }
    })?;
    if requested_start > string.len() || !string.is_char_boundary(requested_start) {
        return Err(ToolResultRecordReadError::InvalidJsonOffset { offset });
    }
    let start = requested_start;
    let candidate_ceiling = utf8_boundary_at_or_before(
        string.as_bytes(),
        start.saturating_add(budget).min(string.len()),
    );
    let mut boundaries = vec![start];
    boundaries.extend(
        string[start..candidate_ceiling]
            .char_indices()
            .map(|(relative, character)| start + relative + character.len_utf8()),
    );
    let mut lower = 0usize;
    let mut upper = boundaries.len();
    while lower + 1 < upper {
        let middle = lower + (upper - lower) / 2;
        let end = boundaries[middle];
        let page = controls.page(
            Value::String(string[start..end].to_string()),
            Vec::new(),
            (end < string.len()).then_some(end as u64),
        );
        if page_fits(&page, budget) {
            lower = middle;
        } else {
            upper = middle;
        }
    }
    let end = boundaries[lower];
    if start == string.len() {
        return ensure_page_fits(
            controls.page(Value::String(String::new()), Vec::new(), None),
            budget,
        );
    }
    if end == start {
        return Err(ToolResultRecordReadError::JsonViewBudgetTooSmall { max_bytes: budget });
    }
    ensure_page_fits(
        controls.page(
            Value::String(string[start..end].to_string()),
            Vec::new(),
            (end < string.len()).then_some(end as u64),
        ),
        budget,
    )
}

fn checked_collection_offset(
    offset: u64,
    length: usize,
) -> Result<usize, ToolResultRecordReadError> {
    let index = usize::try_from(offset).map_err(|error| {
        tracing::debug!(offset, error = %error, "JSON collection offset does not fit in usize");
        ToolResultRecordReadError::InvalidJsonOffset { offset }
    })?;
    if index > length {
        return Err(ToolResultRecordReadError::InvalidJsonOffset { offset });
    }
    Ok(index)
}

fn omitted_descriptor(
    pointer: String,
    key: &str,
    value: &Value,
    index: Option<usize>,
) -> ModelResultJsonOmittedDescriptor {
    let node_type = node_type(value);
    let item_count = collection_item_count(value);
    match index {
        Some(index) => ModelResultJsonOmittedDescriptor::Array(ModelResultJsonOmittedArray {
            index: u64::try_from(index).unwrap_or(u64::MAX),
            json_pointer: if json_pointer_is_sensitive(&pointer) {
                "[redacted]".to_string()
            } else {
                pointer
            },
            node_type,
            item_count,
        }),
        None => ModelResultJsonOmittedDescriptor::Object(ModelResultJsonOmittedObject {
            key: if json_field_name_requires_redaction(key) || json_pointer_is_sensitive(&pointer) {
                "[redacted]".to_string()
            } else {
                key.to_string()
            },
            json_pointer: if json_field_name_requires_redaction(key)
                || json_pointer_is_sensitive(&pointer)
            {
                "[redacted]".to_string()
            } else {
                pointer
            },
            node_type,
            item_count,
        }),
    }
}

fn collection_item_count(value: &Value) -> Option<u64> {
    match value {
        Value::Array(items) => u64::try_from(items.len()).ok(),
        Value::Object(items) => u64::try_from(items.len()).ok(),
        _ => None,
    }
}

fn key_pointer(parent: &str, key: &str) -> String {
    format!("{parent}/{}", escape_json_pointer_segment(key))
}

fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn validate_json_pointer(pointer: &str) -> Result<(), ToolResultRecordReadError> {
    if pointer.is_empty() {
        return Ok(());
    }
    if !pointer.starts_with('/') {
        return Err(ToolResultRecordReadError::InvalidJsonPointer {
            pointer: pointer.to_string(),
        });
    }
    let bytes = pointer.as_bytes();
    let mut index = 1;
    while index < bytes.len() {
        if bytes[index] == b'~' {
            if index + 1 >= bytes.len() || !matches!(bytes[index + 1], b'0' | b'1') {
                return Err(ToolResultRecordReadError::InvalidJsonPointer {
                    pointer: pointer.to_string(),
                });
            }
            index += 1;
        }
        index += 1;
    }
    Ok(())
}

struct PageEnvelopeControls<'a> {
    result_ref: &'a str,
    pointer: &'a str,
    node_type: ModelResultJsonNodeType,
    offset: u64,
    offset_unit: ModelResultJsonOffsetUnit,
    total_bytes: u64,
    max_bytes: usize,
    limit: Option<usize>,
}

impl PageEnvelopeControls<'_> {
    fn page(
        &self,
        content: Value,
        omitted: Vec<ModelResultJsonOmittedDescriptor>,
        next_offset: Option<u64>,
    ) -> ModelResultJsonPage {
        let next = next_offset.map(|next_offset| ModelResultJsonNextRequest {
            result_ref: self.result_ref.to_string(),
            json_pointer: self.pointer.to_string(),
            offset: next_offset,
            max_bytes: u64::try_from(self.max_bytes).unwrap_or(u64::MAX),
            limit: self
                .limit
                .map(|limit| u64::try_from(limit).unwrap_or(u64::MAX)),
        });
        ModelResultJsonPage {
            view: ModelResultJsonPageView::V1,
            result_ref: self.result_ref.to_string(),
            json_pointer: self.pointer.to_string(),
            node_type: self.node_type,
            offset: self.offset,
            offset_unit: self.offset_unit,
            content,
            omitted,
            total_bytes: self.total_bytes,
            next_offset,
            next,
        }
    }
}

fn page_fits(page: &ModelResultJsonPage, budget: usize) -> bool {
    serde_json::to_vec(page).is_ok_and(|encoded| encoded.len() <= budget)
}

fn ensure_page_fits(
    page: ModelResultJsonPage,
    budget: usize,
) -> Result<ModelResultJsonPage, ToolResultRecordReadError> {
    if page_fits(&page, budget) {
        Ok(page)
    } else {
        Err(ToolResultRecordReadError::JsonViewBudgetTooSmall { max_bytes: budget })
    }
}

fn node_type(value: &Value) -> ModelResultJsonNodeType {
    match value {
        Value::Null => ModelResultJsonNodeType::Null,
        Value::Bool(_) => ModelResultJsonNodeType::Boolean,
        Value::Number(_) => ModelResultJsonNodeType::Number,
        Value::String(_) => ModelResultJsonNodeType::String,
        Value::Array(_) => ModelResultJsonNodeType::Array,
        Value::Object(_) => ModelResultJsonNodeType::Object,
    }
}

pub(crate) fn tool_result_record_chunk(
    content: &[u8],
    offset: u64,
    max_bytes: usize,
) -> ToolResultRecordChunk {
    let total_bytes = u64::try_from(content.len()).unwrap_or(u64::MAX);
    let requested_start = usize::try_from(offset)
        .unwrap_or(usize::MAX)
        .min(content.len());
    let start = utf8_boundary_at_or_after(content, requested_start);
    let requested_end = start.saturating_add(max_bytes).min(content.len());
    let mut end = utf8_boundary_at_or_before(content, requested_end);
    if end <= start && start < content.len() {
        end = (start + 1).min(content.len());
    }
    ToolResultRecordChunk {
        content: content[start..end].to_vec(),
        total_bytes,
        next_offset: (end < content.len()).then(|| u64::try_from(end).unwrap_or(u64::MAX)),
    }
}

fn utf8_boundary_at_or_before(content: &[u8], mut index: usize) -> usize {
    while index > 0 && index < content.len() && content[index] & 0b1100_0000 == 0b1000_0000 {
        index -= 1;
    }
    index
}

fn utf8_boundary_at_or_after(content: &[u8], mut index: usize) -> usize {
    while index < content.len() && content[index] & 0b1100_0000 == 0b1000_0000 {
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::model_result_preview::MODEL_RESULT_PREVIEW_MAX_BYTES;

    use super::{
        ToolResultRecordReadError, render_json_tool_result_page, tool_result_record_chunk,
    };

    #[test]
    fn opaque_bytes_always_advance_chunk_offset() {
        let chunk = tool_result_record_chunk(&[0xC2, 0x80, 0x80, 0x80, 0x80], 0, 4);

        assert_eq!(chunk.content, vec![0xC2]);
        assert_eq!(chunk.next_offset, Some(1));
    }

    #[test]
    fn json_string_pages_are_utf8_bounded_and_reject_mid_character_offsets() {
        let serialized = serde_json::to_vec(&"é".repeat(1_000)).expect("fixture serializes");
        let page =
            render_json_tool_result_page("result:string-page", &serialized, "", 0, 512, None)
                .expect("string page renders");

        assert!(serde_json::to_vec(&page).expect("page serializes").len() <= 512);
        let next = page.next_offset.expect("large string has a continuation");
        assert_eq!(next % 2, 0, "continuation stays on a UTF-8 boundary");
        let error =
            render_json_tool_result_page("result:string-page", &serialized, "", 1, 512, None)
                .expect_err("mid-character string offset is rejected");
        assert!(matches!(
            error,
            ToolResultRecordReadError::InvalidJsonOffset { offset: 1 }
        ));
    }

    #[test]
    fn json_views_reject_invalid_pointer_escapes_and_oversized_budgets() {
        let serialized = br#"{"value":1}"#;
        assert!(matches!(
            render_json_tool_result_page(
                "result:invalid-pointer",
                serialized,
                "/bad~2escape",
                0,
                512,
                None,
            ),
            Err(ToolResultRecordReadError::InvalidJsonPointer { .. })
        ));
        assert!(matches!(
            render_json_tool_result_page(
                "result:oversized-budget",
                serialized,
                "",
                0,
                MODEL_RESULT_PREVIEW_MAX_BYTES + 1,
                None,
            ),
            Err(ToolResultRecordReadError::InvalidJsonBudget { .. })
        ));
    }

    #[test]
    fn json_scalar_rejects_collection_limit() {
        let error = render_json_tool_result_page(
            "result:scalar-limit",
            br#"{"value":1}"#,
            "/value",
            0,
            512,
            Some(2),
        )
        .expect_err("collection limit must not be silently ignored for a scalar");

        assert!(matches!(
            error,
            ToolResultRecordReadError::JsonLimitRequiresCollection
        ));
    }

    #[test]
    fn json_pages_redact_credential_labeled_omission_descriptors() {
        let mut object = serde_json::Map::new();
        object.insert("safe".to_string(), serde_json::json!("visible"));
        object.insert("api_key".to_string(), serde_json::json!("x".repeat(2_000)));
        let serialized = serde_json::to_vec(&object).expect("JSON result serializes");

        let page =
            render_json_tool_result_page("result:redacted-omission", &serialized, "", 0, 512, None)
                .expect("the safe key and redacted omission descriptor fit");

        let omitted = page
            .omitted
            .first()
            .expect("large credential field omitted");
        let descriptor = match omitted {
            ironclaw_host_api::model_result_preview::ModelResultJsonOmittedDescriptor::Object(
                descriptor,
            ) => descriptor,
            _ => panic!("object page must use an object omission descriptor"),
        };
        assert_eq!(descriptor.key, "[redacted]");
        assert_eq!(descriptor.json_pointer, "[redacted]");
        let encoded = serde_json::to_string(&page).expect("page serializes");
        assert!(!encoded.contains("api_key"));
        assert!(!encoded.contains("/api_key"));
    }

    #[test]
    fn json_pages_advance_past_an_object_key_whose_descriptor_cannot_fit() {
        let mut object = serde_json::Map::new();
        object.insert("a".to_string(), serde_json::json!("visible"));
        object.insert("m".repeat(1_000), serde_json::json!("value"));
        object.insert("z".to_string(), serde_json::json!("later"));
        let serialized = serde_json::to_vec(&object).expect("JSON result serializes");

        let first = render_json_tool_result_page(
            "result:oversized-object-key",
            &serialized,
            "",
            0,
            512,
            None,
        )
        .expect("an unrepresentable key must not block the page");
        let next_offset = first
            .next_offset
            .expect("the page advances past the unrepresentable key");
        assert_eq!(next_offset, 2);

        let second = render_json_tool_result_page(
            "result:oversized-object-key",
            &serialized,
            "",
            next_offset,
            512,
            None,
        )
        .expect("later object fields remain reachable");
        assert_eq!(second.content["z"], "later");
    }
}

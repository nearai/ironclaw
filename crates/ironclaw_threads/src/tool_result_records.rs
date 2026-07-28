use crate::{
    SessionThreadError, TOOL_RESULT_RECORD_READ_MAX_BYTES, ToolResultRecordChunk,
    ToolResultReferenceEnvelope,
};

pub(crate) const TOOL_RESULT_RECORD_MAX_BYTES: usize = 4 * 1024 * 1024;
const TOOL_RESULT_RECORD_READ_MIN_BYTES: usize = 4;

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
    if !(TOOL_RESULT_RECORD_READ_MIN_BYTES..=TOOL_RESULT_RECORD_READ_MAX_BYTES).contains(&max_bytes)
    {
        return Err(SessionThreadError::Serialization(
            "tool result record read size is outside the supported range".to_string(),
        ));
    }
    Ok(())
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
    use super::tool_result_record_chunk;

    #[test]
    fn opaque_bytes_always_advance_chunk_offset() {
        let chunk = tool_result_record_chunk(&[0xC2, 0x80, 0x80, 0x80, 0x80], 0, 4);

        assert_eq!(chunk.content, vec![0xC2]);
        assert_eq!(chunk.next_offset, Some(1));
    }
}

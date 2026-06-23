use futures::future::join_all;
use ironclaw_filesystem::{CasExpectation, ContentType, Entry, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{ScopedPath, ThreadId};
use serde::{Deserialize, Serialize};

use crate::{SessionThreadError, ThreadMessageId, ThreadMessageRecord, ThreadScope};

use super::{
    PutError, deserialize, put_with_cas, scoped_path, serialize_pretty, thread_root_string,
};

/// Conservative fan-out for indexed sequence reads.
const MESSAGE_SEQUENCE_INDEX_READ_CONCURRENCY: usize = 8;
/// Upper bound before falling back to message-directory scanning.
///
/// The index path is optimized for compact prompt-window reads. For very broad
/// or sparse ranges, probing one sequence file per possible number is worse
/// than materializing existing messages and filtering them.
const MESSAGE_SEQUENCE_INDEX_MAX_RANGE_SPAN: u64 = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct MessageSequenceIndexRecord {
    pub(super) sequence: u64,
    pub(super) message_id: ThreadMessageId,
}

pub(super) struct MessageSequenceIndexStore<'a, F>
where
    F: RootFilesystem,
{
    filesystem: &'a ScopedFilesystem<F>,
}

impl<'a, F> MessageSequenceIndexStore<'a, F>
where
    F: RootFilesystem,
{
    pub(super) fn new(filesystem: &'a ScopedFilesystem<F>) -> Self {
        Self { filesystem }
    }

    pub(super) async fn read(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        sequence: u64,
    ) -> Result<Option<MessageSequenceIndexRecord>, SessionThreadError> {
        let path = message_sequence_index_path(scope, thread_id, sequence)?;
        let Some(versioned) = self
            .filesystem
            .get(&scope.to_resource_scope(), &path)
            .await?
        else {
            return Ok(None);
        };
        let record = deserialize::<MessageSequenceIndexRecord>(&versioned.entry.body)?;
        if record.sequence != sequence {
            return Err(SessionThreadError::Backend(format!(
                "message sequence index at {} contains sequence {}",
                path.as_str(),
                record.sequence
            )));
        }
        Ok(Some(record))
    }

    pub(super) async fn read_range(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        after_sequence: u64,
        through_sequence: u64,
    ) -> Result<Option<Vec<MessageSequenceIndexRecord>>, SessionThreadError> {
        if through_sequence <= after_sequence {
            return Ok(Some(Vec::new()));
        }
        let start = after_sequence.checked_add(1).ok_or_else(|| {
            SessionThreadError::Backend("message sequence range overflowed".to_string())
        })?;
        let span = through_sequence
            .checked_sub(start)
            .and_then(|value| value.checked_add(1))
            .ok_or_else(|| {
                SessionThreadError::Backend("message sequence range overflowed".to_string())
            })?;
        if span > MESSAGE_SEQUENCE_INDEX_MAX_RANGE_SPAN {
            return Ok(None);
        }
        let mut records = Vec::with_capacity(span as usize);
        for chunk_start in
            (start..=through_sequence).step_by(MESSAGE_SEQUENCE_INDEX_READ_CONCURRENCY)
        {
            let chunk_end = through_sequence.min(
                chunk_start.saturating_add(MESSAGE_SEQUENCE_INDEX_READ_CONCURRENCY as u64 - 1),
            );
            let reads =
                (chunk_start..=chunk_end).map(|sequence| self.read(scope, thread_id, sequence));
            let results = join_all(reads).await;
            for result in results {
                let Some(record) = result? else {
                    return Ok(None);
                };
                records.push(record);
            }
        }
        Ok(Some(records))
    }

    pub(super) async fn write_new(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        message: &ThreadMessageRecord,
    ) -> Result<(), SessionThreadError> {
        let (path, entry) = message_sequence_index_entry_for_message(scope, thread_id, message)?;
        match put_with_cas(
            self.filesystem,
            &scope.to_resource_scope(),
            &path,
            entry,
            CasExpectation::Absent,
        )
        .await
        {
            Ok(()) => Ok(()),
            Err(PutError::VersionMismatch) => {
                let existing = self.read(scope, thread_id, message.sequence).await?.ok_or_else(
                    || {
                        SessionThreadError::Backend(format!(
                            "filesystem CAS Absent rejected message sequence index at {} but record is missing",
                            path.as_str()
                        ))
                    },
                )?;
                if existing.message_id == message.message_id {
                    return Ok(());
                }
                Err(SessionThreadError::Backend(format!(
                    "message sequence {} already indexes message {}, not {}",
                    message.sequence, existing.message_id, message.message_id
                )))
            }
            Err(PutError::Other(error)) => Err(error),
        }
    }
}

pub(super) fn message_sequence_index_entry_for_message(
    scope: &ThreadScope,
    thread_id: &ThreadId,
    message: &ThreadMessageRecord,
) -> Result<(ScopedPath, Entry), SessionThreadError> {
    let path = message_sequence_index_path(scope, thread_id, message.sequence)?;
    let record = MessageSequenceIndexRecord {
        sequence: message.sequence,
        message_id: message.message_id,
    };
    let entry = message_sequence_index_entry(&record)?;
    Ok((path, entry))
}

fn message_sequence_index_entry(
    record: &MessageSequenceIndexRecord,
) -> Result<Entry, SessionThreadError> {
    let body = serialize_pretty(record)?;
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
}

fn message_sequence_index_path(
    scope: &ThreadScope,
    thread_id: &ThreadId,
    sequence: u64,
) -> Result<ScopedPath, SessionThreadError> {
    scoped_path(&format!(
        "{}/messages_by_sequence/{}",
        thread_root_string(scope, thread_id),
        sequence_index_filename(sequence)
    ))
}

pub(super) fn sequence_index_filename(sequence: u64) -> String {
    format!("{sequence:020}.json")
}

#[cfg(test)]
pub(super) fn sequence_from_index_filename(name: &str) -> Option<u64> {
    let raw = name.strip_suffix(".json")?;
    if raw.len() != 20 || !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    raw.parse().ok()
}

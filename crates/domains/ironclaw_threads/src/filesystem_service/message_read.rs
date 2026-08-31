//! Bounded transcript materialization for filesystem-backed exports.

use ironclaw_filesystem::{
    Filter, IndexValue, OrderedPage, OrderedQueryCursor, Page, RootFilesystem, SortDirection,
};
use ironclaw_host_api::ids::ThreadId;
use ironclaw_host_api::turn::TurnRunId;

use crate::{MessageKind, MessageStatus, SessionThreadError, ThreadMessageRecord, ThreadScope};

use super::{
    FilesystemSessionThreadService, deserialize, fs_index_key, message_sequence_index_spec,
    message_source_binding_sequence_index_spec, message_turn_run_sequence_index_spec,
    messages_root, serde_enum_index_value, thread_partition_filter,
};

#[derive(Debug, Clone, Copy)]
pub(super) struct MessageReadBudget {
    remaining_messages: usize,
    remaining_bytes: usize,
}

impl MessageReadBudget {
    pub(super) fn new(max_messages: usize, max_bytes: usize) -> Self {
        Self {
            remaining_messages: max_messages,
            remaining_bytes: max_bytes,
        }
    }

    fn page_limit(self) -> u32 {
        self.remaining_messages
            .saturating_add(1)
            .min(Page::MAX_LIMIT as usize) as u32
    }

    fn consume(&mut self, bytes: usize) -> bool {
        if self.remaining_messages == 0 || bytes > self.remaining_bytes {
            return false;
        }
        self.remaining_messages -= 1;
        self.remaining_bytes -= bytes;
        true
    }
}

pub(super) enum MessageReadResult {
    Complete(Vec<ThreadMessageRecord>),
    LimitExceeded,
}

impl<F> FilesystemSessionThreadService<F>
where
    F: RootFilesystem + 'static,
{
    pub(super) async fn read_thread_messages(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        mut budget: Option<MessageReadBudget>,
    ) -> Result<MessageReadResult, SessionThreadError> {
        self.ensure_transcript_indexes_migrated(scope).await?;
        let root = messages_root(scope, thread_id)?;
        let index = message_sequence_index_spec()?;
        let sequence_key = fs_index_key("sequence")?;
        let message_id_key = fs_index_key("message_id")?;
        let mut messages = Vec::new();
        let mut cursor = None;

        loop {
            let page_limit = budget
                .map(MessageReadBudget::page_limit)
                .unwrap_or(Page::MAX_LIMIT)
                .max(1);
            let mut page = OrderedPage::new(
                index.name.clone(),
                sequence_key.clone(),
                message_id_key.clone(),
                SortDirection::Ascending,
                page_limit,
            );
            if let Some(after) = cursor.take() {
                page = page.after(after);
            }
            let entries = match self
                .filesystem
                .query_ordered(
                    &scope.to_resource_scope(),
                    &root,
                    &thread_partition_filter(thread_id)?,
                    &page,
                )
                .await
            {
                Ok(entries) => entries,
                Err(error) => return Err(error.into()),
            };
            let entry_count = entries.len();
            for versioned in &entries {
                if !versioned.path.as_str().ends_with(".json") {
                    continue;
                }
                if let Some(remaining) = budget.as_mut()
                    && !remaining.consume(versioned.entry.body.len())
                {
                    return Ok(MessageReadResult::LimitExceeded);
                }
                let record = deserialize::<ThreadMessageRecord>(&versioned.entry.body)?;
                if &record.thread_id == thread_id {
                    messages.push(record);
                }
            }
            cursor = entries
                .last()
                .map(|entry| {
                    let value =
                        entry
                            .entry
                            .indexed
                            .get(&sequence_key)
                            .cloned()
                            .ok_or_else(|| {
                                SessionThreadError::Backend(
                                    "ordered message row is missing sequence index".to_string(),
                                )
                            })?;
                    let tie_breaker = entry
                        .entry
                        .indexed
                        .get(&message_id_key)
                        .cloned()
                        .ok_or_else(|| {
                            SessionThreadError::Backend(
                                "ordered message row is missing message_id index".to_string(),
                            )
                        })?;
                    Ok::<_, SessionThreadError>(OrderedQueryCursor { value, tie_breaker })
                })
                .transpose()?;
            if entry_count < page_limit as usize {
                break;
            }
        }

        messages.sort_by_key(|message| message.sequence);
        Ok(MessageReadResult::Complete(messages))
    }
}

impl<F> FilesystemSessionThreadService<F>
where
    F: RootFilesystem + 'static,
{
    pub(super) async fn read_completed_run_messages(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        turn_run_id: TurnRunId,
        max_messages: usize,
        max_bytes: usize,
    ) -> Result<MessageReadResult, SessionThreadError> {
        self.ensure_transcript_indexes_migrated(scope).await?;
        let root = messages_root(scope, thread_id)?;
        let sequence_key = fs_index_key("sequence")?;
        let message_id_key = fs_index_key("message_id")?;
        let run_id = turn_run_id.to_string();
        let subagent_binding = format!("subagent-result:{run_id}");
        let page_limit = MessageReadBudget::new(max_messages, max_bytes)
            .page_limit()
            .max(1);

        let direct_page = OrderedPage::new(
            message_turn_run_sequence_index_spec()?.name,
            sequence_key.clone(),
            message_id_key.clone(),
            SortDirection::Ascending,
            page_limit,
        );
        let direct_filter = Filter::And(vec![
            thread_partition_filter(thread_id)?,
            Filter::Eq {
                key: fs_index_key("turn_run_id")?,
                value: IndexValue::Text(run_id.clone()),
            },
        ]);
        let mut entries = self
            .filesystem
            .query_ordered(
                &scope.to_resource_scope(),
                &root,
                &direct_filter,
                &direct_page,
            )
            .await?;

        let source_page = OrderedPage::new(
            message_source_binding_sequence_index_spec()?.name,
            sequence_key,
            message_id_key,
            SortDirection::Ascending,
            page_limit,
        );
        let source_filter = Filter::And(vec![
            thread_partition_filter(thread_id)?,
            Filter::Eq {
                key: fs_index_key("source_binding_id")?,
                value: IndexValue::Text(subagent_binding.clone()),
            },
            Filter::Eq {
                key: fs_index_key("message_kind")?,
                value: IndexValue::Text(serde_enum_index_value(&MessageKind::System)?),
            },
            Filter::Eq {
                key: fs_index_key("message_status")?,
                value: IndexValue::Text(serde_enum_index_value(&MessageStatus::Finalized)?),
            },
        ]);
        entries.extend(
            self.filesystem
                .query_ordered(
                    &scope.to_resource_scope(),
                    &root,
                    &source_filter,
                    &source_page,
                )
                .await?,
        );

        let mut matched = Vec::with_capacity(entries.len());
        for versioned in entries {
            if !versioned.path.as_str().ends_with(".json") {
                continue;
            }
            let bytes = versioned.entry.body.len();
            let record = deserialize::<ThreadMessageRecord>(&versioned.entry.body)?;
            let run_match = record.turn_run_id.as_deref() == Some(run_id.as_str());
            let binding_match = record.source_binding_id.as_deref()
                == Some(subagent_binding.as_str())
                && record.kind == MessageKind::System
                && record.status == MessageStatus::Finalized;
            if &record.thread_id != thread_id || (!run_match && !binding_match) {
                return Err(SessionThreadError::Backend(
                    "completed-run message index returned a mismatched row".to_string(),
                ));
            }
            matched.push((record, bytes));
        }
        matched.sort_by_key(|(record, _)| record.sequence);
        matched.dedup_by(|left, right| left.0.message_id == right.0.message_id);

        let mut budget = MessageReadBudget::new(max_messages, max_bytes);
        let mut messages = Vec::with_capacity(matched.len());
        for (record, bytes) in matched {
            if !budget.consume(bytes) {
                return Ok(MessageReadResult::LimitExceeded);
            }
            messages.push(record);
        }
        Ok(MessageReadResult::Complete(messages))
    }
}

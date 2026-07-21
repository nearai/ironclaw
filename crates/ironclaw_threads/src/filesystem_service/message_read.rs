//! Shared transcript materialization for bounded and ordinary filesystem reads.

use std::collections::HashSet;

use ironclaw_filesystem::{
    FilesystemError, FilesystemOperation, Filter, Page, RootFilesystem, SeqNo,
};
use ironclaw_host_api::ThreadId;

use crate::{SessionThreadError, ThreadMessageRecord, ThreadScope};

use super::{
    FilesystemSessionThreadService, deserialize, is_not_found, join_scoped,
    message_append_log_path, messages_root,
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

    fn consume_bytes(&mut self, bytes: usize) -> bool {
        if bytes > self.remaining_bytes {
            return false;
        }
        self.remaining_bytes -= bytes;
        true
    }

    fn consume_message(&mut self) -> bool {
        if self.remaining_messages == 0 {
            return false;
        }
        self.remaining_messages -= 1;
        true
    }
}

pub(super) enum MessageReadResult {
    Complete(Vec<ThreadMessageRecord>),
    LimitExceeded,
}

impl<F> FilesystemSessionThreadService<F>
where
    F: RootFilesystem,
{
    pub(super) async fn read_thread_messages(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        mut budget: Option<MessageReadBudget>,
    ) -> Result<MessageReadResult, SessionThreadError> {
        let root = messages_root(scope, thread_id)?;
        let mut messages = Vec::new();
        let mut offset = 0_u64;

        loop {
            let page_limit = budget
                .map(MessageReadBudget::page_limit)
                .unwrap_or(Page::MAX_LIMIT)
                .max(1);
            let entries = match self
                .filesystem
                .query(
                    &scope.to_resource_scope(),
                    &root,
                    &Filter::All,
                    Page::new(offset, page_limit),
                )
                .await
            {
                Ok(entries) => entries,
                Err(FilesystemError::Unsupported {
                    operation: FilesystemOperation::Query,
                    ..
                }) => {
                    return self
                        .read_thread_messages_by_directory(scope, thread_id, budget)
                        .await;
                }
                Err(error) => return Err(error.into()),
            };
            let entry_count = entries.len();
            for versioned in entries {
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
            if entry_count < page_limit as usize {
                break;
            }
            offset = offset.saturating_add(entry_count as u64);
        }

        self.finish_message_read(scope, thread_id, messages, budget)
            .await
    }

    async fn read_thread_messages_by_directory(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        mut budget: Option<MessageReadBudget>,
    ) -> Result<MessageReadResult, SessionThreadError> {
        let root = messages_root(scope, thread_id)?;
        let entries = match self
            .filesystem
            .list_dir(&scope.to_resource_scope(), &root)
            .await
        {
            Ok(entries) => entries,
            Err(error) if is_not_found(&error) => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        let mut messages = Vec::new();
        for entry in entries {
            if !entry.name.ends_with(".json") {
                continue;
            }
            let child = join_scoped(&root, &entry.name)?;
            let Some(versioned) = self
                .filesystem
                .get(&scope.to_resource_scope(), &child)
                .await?
            else {
                continue;
            };
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
        self.finish_message_read(scope, thread_id, messages, budget)
            .await
    }

    async fn finish_message_read(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        mut messages: Vec<ThreadMessageRecord>,
        budget: Option<MessageReadBudget>,
    ) -> Result<MessageReadResult, SessionThreadError> {
        if let Some(mut remaining) = budget {
            let existing_ids: HashSet<_> =
                messages.iter().map(|message| message.message_id).collect();
            let Some(mut event_messages) = self
                .read_message_append_events_with_budget(
                    scope,
                    thread_id,
                    &existing_ids,
                    &mut remaining,
                )
                .await?
            else {
                return Ok(MessageReadResult::LimitExceeded);
            };
            messages.append(&mut event_messages);
        } else {
            self.merge_message_append_events(scope, thread_id, &mut messages)
                .await?;
        }
        messages.sort_by_key(|message| message.sequence);
        Ok(MessageReadResult::Complete(messages))
    }

    async fn read_message_append_events_with_budget(
        &self,
        scope: &ThreadScope,
        thread_id: &ThreadId,
        existing_ids: &HashSet<crate::ThreadMessageId>,
        budget: &mut MessageReadBudget,
    ) -> Result<Option<Vec<ThreadMessageRecord>>, SessionThreadError> {
        let path = message_append_log_path(scope, thread_id)?;
        let max_events = budget.remaining_messages.saturating_add(existing_ids.len());
        let events = match self
            .filesystem
            .tail_bounded(
                &scope.to_resource_scope(),
                &path,
                SeqNo::ZERO,
                max_events.saturating_add(1),
            )
            .await
        {
            Ok(events) => events,
            Err(FilesystemError::Unsupported {
                operation: FilesystemOperation::Tail,
                ..
            })
            | Err(FilesystemError::NotFound { .. }) => return Ok(Some(Vec::new())),
            Err(error) => return Err(error.into()),
        };
        if events.len() > max_events {
            return Ok(None);
        }
        let mut messages = Vec::with_capacity(events.len().min(budget.remaining_messages));
        for event in events {
            // Charge every physical append payload against the byte budget so
            // stale shadow records cannot defeat the allocation ceiling. The
            // logical message budget is charged only after file-authoritative
            // deduplication below.
            if !budget.consume_bytes(event.payload.len()) {
                return Ok(None);
            }
            let message = deserialize::<ThreadMessageRecord>(&event.payload)?;
            if &message.thread_id != thread_id || existing_ids.contains(&message.message_id) {
                continue;
            }
            if !budget.consume_message() {
                return Ok(None);
            }
            messages.push(message);
        }
        Ok(Some(messages))
    }
}

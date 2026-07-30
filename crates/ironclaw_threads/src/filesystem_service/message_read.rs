//! Bounded transcript materialization for filesystem-backed exports.

use ironclaw_filesystem::{Filter, Page, RootFilesystem};
use ironclaw_host_api::ThreadId;

use crate::{SessionThreadError, ThreadMessageRecord, ThreadScope};

use super::{FilesystemSessionThreadService, deserialize, messages_root};

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

        messages.sort_by_key(|message| message.sequence);
        Ok(MessageReadResult::Complete(messages))
    }
}

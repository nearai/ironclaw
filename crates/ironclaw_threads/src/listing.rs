use serde::{Deserialize, Serialize};

use crate::contract::{ListThreadsForScopeResponse, SessionThreadRecord};

const LIST_THREADS_DEFAULT_LIMIT: usize = 100;
const LIST_THREADS_MAX_LIMIT: usize = 500;

#[derive(Debug, Clone)]
pub(crate) struct ThreadListEntry {
    pub(crate) record: SessionThreadRecord,
    pub(crate) updated_at_unix_ms: i64,
}

impl ThreadListEntry {
    pub(crate) fn new(record: SessionThreadRecord, updated_at_unix_ms: i64) -> Self {
        Self {
            record,
            updated_at_unix_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ThreadListCursor {
    updated_at_unix_ms: i64,
    thread_id: String,
}

pub(crate) fn paginate_thread_entries(
    mut entries: Vec<ThreadListEntry>,
    limit: Option<u32>,
    cursor: Option<&str>,
) -> ListThreadsForScopeResponse {
    entries.sort_by(|left, right| {
        right
            .updated_at_unix_ms
            .cmp(&left.updated_at_unix_ms)
            .then_with(|| {
                left.record
                    .thread_id
                    .as_str()
                    .cmp(right.record.thread_id.as_str())
            })
    });

    let limit = clamp_list_threads_limit(limit);
    let start = page_start(&entries, cursor);
    let mut page: Vec<_> = entries.into_iter().skip(start).take(limit + 1).collect();
    let next_cursor = if page.len() > limit {
        page.truncate(limit);
        page.last().and_then(encode_cursor)
    } else {
        None
    };
    ListThreadsForScopeResponse {
        threads: page.into_iter().map(|entry| entry.record).collect(),
        next_cursor,
    }
}

fn page_start(entries: &[ThreadListEntry], cursor: Option<&str>) -> usize {
    let Some(cursor) = cursor.filter(|cursor| !cursor.is_empty()) else {
        return 0;
    };
    match serde_json::from_str::<ThreadListCursor>(cursor) {
        Ok(cursor) => entries
            .iter()
            .position(|entry| is_after_cursor(entry, &cursor))
            .unwrap_or(entries.len()),
        Err(_) => entries
            .iter()
            .position(|entry| entry.record.thread_id.as_str() == cursor)
            .map(|index| index + 1)
            .unwrap_or(entries.len()),
    }
}

fn is_after_cursor(entry: &ThreadListEntry, cursor: &ThreadListCursor) -> bool {
    entry.updated_at_unix_ms < cursor.updated_at_unix_ms
        || (entry.updated_at_unix_ms == cursor.updated_at_unix_ms
            && entry.record.thread_id.as_str() > cursor.thread_id.as_str())
}

fn encode_cursor(entry: &ThreadListEntry) -> Option<String> {
    serde_json::to_string(&ThreadListCursor {
        updated_at_unix_ms: entry.updated_at_unix_ms,
        thread_id: entry.record.thread_id.as_str().to_string(),
    })
    .ok()
}

fn clamp_list_threads_limit(limit: Option<u32>) -> usize {
    limit
        .unwrap_or(LIST_THREADS_DEFAULT_LIMIT as u32)
        .clamp(1, LIST_THREADS_MAX_LIMIT as u32) as usize
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};

    use crate::{SessionThreadRecord, ThreadScope};

    use super::{ThreadListEntry, paginate_thread_entries};

    #[test]
    fn pagination_sorts_by_updated_at_before_limiting_and_cursoring() {
        let first = paginate_thread_entries(
            vec![
                entry("thread-c", 10),
                entry("thread-a", 30),
                entry("thread-b", 20),
            ],
            Some(2),
            None,
        );
        assert_eq!(thread_ids(&first.threads), vec!["thread-a", "thread-b"]);
        assert!(first.next_cursor.is_some());

        let second = paginate_thread_entries(
            vec![
                entry("thread-c", 10),
                entry("thread-a", 30),
                entry("thread-b", 20),
            ],
            Some(2),
            first.next_cursor.as_deref(),
        );
        assert_eq!(thread_ids(&second.threads), vec!["thread-c"]);
        assert!(second.next_cursor.is_none());
    }

    #[test]
    fn pagination_cursor_survives_deleted_cursor_item() {
        let first = paginate_thread_entries(
            vec![
                entry("thread-c", 10),
                entry("thread-a", 30),
                entry("thread-b", 20),
            ],
            Some(2),
            None,
        );

        let second = paginate_thread_entries(
            vec![entry("thread-c", 10), entry("thread-a", 30)],
            Some(2),
            first.next_cursor.as_deref(),
        );

        assert_eq!(thread_ids(&second.threads), vec!["thread-c"]);
        assert!(second.next_cursor.is_none());
    }

    fn entry(thread_id: &str, updated_at_unix_ms: i64) -> ThreadListEntry {
        ThreadListEntry::new(record(thread_id), updated_at_unix_ms)
    }

    fn record(thread_id: &str) -> SessionThreadRecord {
        SessionThreadRecord {
            scope: ThreadScope {
                tenant_id: TenantId::new("tenant-listing").unwrap(),
                agent_id: AgentId::new("agent-listing").unwrap(),
                project_id: Some(ProjectId::new("project-listing").unwrap()),
                owner_user_id: Some(UserId::new("user-listing").unwrap()),
                mission_id: None,
            },
            thread_id: ThreadId::new(thread_id).unwrap(),
            created_by_actor_id: "actor-a".into(),
            title: None,
            metadata_json: None,
        }
    }

    fn thread_ids(records: &[SessionThreadRecord]) -> Vec<&str> {
        records
            .iter()
            .map(|record| record.thread_id.as_str())
            .collect()
    }
}

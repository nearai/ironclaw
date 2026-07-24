//! Indexed-projection + query primitives for the durable turn-event rows.
//!
//! Event rows live under the `events` row collection keyed by a zero-padded
//! [`EventCursor`] (`event_record_key`), so their on-disk path order already
//! equals cursor order — both production backends (`libSQL`, `Postgres`) and
//! the in-memory reference backend `ORDER BY path` in
//! [`RootFilesystem::query`](ironclaw_filesystem::RootFilesystem::query). To
//! serve `read_turn_events_after` with an indexed range scan (instead of
//! listing the whole collection and reading every row body after the cursor
//! across *all* of a store's threads), each event row projects two indexed
//! values into [`Entry::indexed`](ironclaw_filesystem::Entry::indexed):
//!
//! - `scope_key` ([`IndexValue::Text`]) — the coarse
//!   `(tenant, agent, project, thread)` scope key, deliberately *ignoring*
//!   `thread_owner` (mirrors the active-run exclusivity key). A coarser key can
//!   only ever over-fetch — `project_turn_events` re-filters on the full
//!   [`TurnScope`] equality — so it never drops an in-scope event, while still
//!   pruning the cross-thread rows that dominate a multi-thread store.
//! - `cursor` ([`IndexValue::I64`]) — the event cursor, so a `cursor > after`
//!   range predicate runs in the backend.
//!
//! The read path filters `And(Eq{scope_key}, Range{cursor})`; the resulting
//! materialized bodies flow into the unchanged
//! [`project_turn_events`](crate::events::project_turn_events), preserving every
//! scope/owner/retention/rebase/pagination semantic the legacy directory scan
//! produced.

use std::collections::BTreeMap;

use ironclaw_filesystem::{Filter, IndexKey, IndexKind, IndexName, IndexSpec, IndexValue};

use crate::{EventCursor, TurnError, TurnLifecycleEvent, TurnScope};

/// Indexed key carrying the coarse scope identity of an event row.
const EVENTS_SCOPE_KEY: &str = "scope_key";
/// Indexed key carrying the event cursor of an event row.
const EVENTS_CURSOR_KEY: &str = "cursor";
/// Declared index over `scope_key`.
const EVENTS_SCOPE_INDEX_NAME: &str = "turn_events_scope";
/// Declared index over `cursor`.
const EVENTS_CURSOR_INDEX_NAME: &str = "turn_events_cursor";

fn index_key(raw: &'static str) -> Result<IndexKey, TurnError> {
    IndexKey::new(raw).map_err(|error| TurnError::Unavailable {
        reason: format!("invalid turn-events index key {raw}: {error}"),
    })
}

fn index_name(raw: &'static str) -> Result<IndexName, TurnError> {
    IndexName::new(raw).map_err(|error| TurnError::Unavailable {
        reason: format!("invalid turn-events index name {raw}: {error}"),
    })
}

pub(super) fn scope_index_key() -> Result<IndexKey, TurnError> {
    index_key(EVENTS_SCOPE_KEY)
}

pub(super) fn cursor_index_key() -> Result<IndexKey, TurnError> {
    index_key(EVENTS_CURSOR_KEY)
}

/// Coarse `(tenant, agent, project, thread)` scope key, ignoring
/// `thread_owner`. Equal-by-this-key scopes always produce byte-identical
/// strings, so the projection never under-fetches an in-scope event; distinct
/// threads produce distinct keys, which is the pruning the query relies on. A
/// `\u{1f}` (unit separator) delimiter keeps unrelated ids from concatenating
/// into a collision — but even a collision would only over-fetch (safe), never
/// drop an event, because `project_turn_events` re-filters on full scope
/// equality.
pub(super) fn event_scope_index_value(scope: &TurnScope) -> String {
    let agent = scope
        .agent_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    let project = scope
        .project_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        scope.tenant_id, agent, project, scope.thread_id
    )
}

/// Cursor values are assigned by a monotonic `u64`; a real deployment never
/// approaches `i64::MAX`, but saturate rather than wrap so an out-of-range
/// value degrades to "always the top of the range" instead of a negative
/// integer the backend would compare incorrectly.
fn cursor_to_i64(cursor: EventCursor) -> i64 {
    i64::try_from(cursor.0).unwrap_or(i64::MAX)
}

/// The indexed projection written alongside an event row body so the durable
/// read path can serve a scoped range scan.
pub(super) fn event_indexed_projection(
    event: &TurnLifecycleEvent,
) -> Result<BTreeMap<IndexKey, IndexValue>, TurnError> {
    let mut indexed = BTreeMap::new();
    indexed.insert(
        scope_index_key()?,
        IndexValue::Text(event_scope_index_value(&event.scope)),
    );
    indexed.insert(
        cursor_index_key()?,
        IndexValue::I64(cursor_to_i64(event.cursor)),
    );
    Ok(indexed)
}

/// The index specs declared once on the events row prefix. Two single-key
/// `Exact` indexes rather than one composite, so a backend that only supports
/// single-key indexes still accepts them; the `And(Eq, Range)` filter works
/// against either regardless of which the backend uses.
pub(super) fn event_index_specs() -> Result<Vec<IndexSpec>, TurnError> {
    Ok(vec![
        IndexSpec::new(
            index_name(EVENTS_SCOPE_INDEX_NAME)?,
            vec![scope_index_key()?],
            IndexKind::Exact,
        ),
        IndexSpec::new(
            index_name(EVENTS_CURSOR_INDEX_NAME)?,
            vec![cursor_index_key()?],
            IndexKind::Exact,
        ),
    ])
}

/// Filter matching this scope's events with `cursor > after` — the exact set
/// the legacy directory scan pre-filtered before `project_turn_events`.
pub(super) fn events_query_filter(
    scope: &TurnScope,
    after: Option<EventCursor>,
) -> Result<Filter, TurnError> {
    let after_cursor = after.map(|cursor| cursor.0).unwrap_or(0);
    // `cursor > after` == `cursor` in `[after + 1, i64::MAX]`.
    let lo = cursor_to_i64(EventCursor(after_cursor)).saturating_add(1);
    Ok(Filter::And(vec![
        Filter::Eq {
            key: scope_index_key()?,
            value: IndexValue::Text(event_scope_index_value(scope)),
        },
        Filter::Range {
            key: cursor_index_key()?,
            lo: IndexValue::I64(lo),
            hi: IndexValue::I64(i64::MAX),
        },
    ]))
}

/// Filter for a bounded host-internal replay of the lifecycle log across all
/// turn scopes in this already tenant/backend-scoped mount.
pub(super) fn event_log_query_filter(after: Option<EventCursor>) -> Result<Filter, TurnError> {
    let after_cursor = after.map(|cursor| cursor.0).unwrap_or(0);
    let lo = cursor_to_i64(EventCursor(after_cursor)).saturating_add(1);
    Ok(Filter::Range {
        key: cursor_index_key()?,
        lo: IndexValue::I64(lo),
        hi: IndexValue::I64(i64::MAX),
    })
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};

    use super::*;
    use crate::TurnScope;

    fn scope(thread: &str, owner: Option<&str>) -> TurnScope {
        TurnScope::new_with_owner(
            TenantId::new("tenant-a").expect("tenant"),
            Some(AgentId::new("agent-a").expect("agent")),
            Some(ProjectId::new("project-a").expect("project")),
            ThreadId::new(thread).expect("thread"),
            owner.map(|o| UserId::new(o).expect("owner")),
        )
    }

    #[test]
    fn scope_key_ignores_thread_owner_so_it_never_underfetches() {
        // Two scopes equal on (tenant, agent, project, thread) but differing on
        // owner MUST share a key — otherwise an owner change would hide events
        // that `project_turn_events` (full-equality filter) still wants.
        let a = event_scope_index_value(&scope("thread-a", Some("owner-1")));
        let b = event_scope_index_value(&scope("thread-a", Some("owner-2")));
        let c = event_scope_index_value(&scope("thread-a", None));
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn scope_key_distinguishes_threads() {
        assert_ne!(
            event_scope_index_value(&scope("thread-a", None)),
            event_scope_index_value(&scope("thread-b", None)),
        );
    }

    #[test]
    fn query_filter_encodes_cursor_strictly_greater_than_after() {
        let filter =
            events_query_filter(&scope("thread-a", None), Some(EventCursor(5))).expect("filter");
        let Filter::And(children) = filter else {
            panic!("expected And filter, got {filter:?}");
        };
        assert!(matches!(
            children.iter().find_map(|f| match f {
                Filter::Range { lo, .. } => Some(lo),
                _ => None,
            }),
            Some(IndexValue::I64(6))
        ));
    }

    #[test]
    fn query_filter_from_origin_starts_at_cursor_one() {
        let filter = events_query_filter(&scope("thread-a", None), None).expect("filter");
        let Filter::And(children) = filter else {
            panic!("expected And filter");
        };
        assert!(matches!(
            children.iter().find_map(|f| match f {
                Filter::Range { lo, .. } => Some(lo),
                _ => None,
            }),
            Some(IndexValue::I64(1))
        ));
    }
}

use serde::de::DeserializeOwned;

use crate::cursor::{EventCursor, EventLogEntry, EventReplay};
use crate::error::EventError;

/// Parse a JSONL byte slice into a vector of typed records.
///
/// Backend, mount, permission, UTF-8, or malformed JSONL failures are
/// returned as errors; the helper does not silently elide invalid lines.
/// See `events.md` §5.
pub fn parse_jsonl<T>(bytes: &[u8]) -> Result<Vec<T>, EventError>
where
    T: DeserializeOwned,
{
    let text = std::str::from_utf8(bytes).map_err(|error| EventError::Serialize {
        reason: error.to_string(),
    })?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<T>(line).map_err(|error| EventError::Serialize {
                reason: error.to_string(),
            })
        })
        .collect()
}

/// Replay a JSONL byte slice after a cursor with a bounded limit.
///
/// Used by JSONL-backed durable log adapters and fixture readers.
/// The cursor is the 1-based line index of the last consumed record.
///
/// **Assumes uncompacted JSONL.** Backends that compact entries (drop old
/// records to reclaim disk) must not use this helper directly: line index
/// will desynchronize from the logical cursor and the helper will return
/// `ReplayGap` with a meaningless `earliest` value. Compacting backends
/// should either store the cursor inline in each record and use a different
/// parser, or maintain an out-of-band file-offset → cursor map.
pub fn replay_jsonl<T>(
    bytes: &[u8],
    after: Option<EventCursor>,
    limit: usize,
) -> Result<EventReplay<T>, EventError>
where
    T: DeserializeOwned,
{
    if limit == 0 {
        return Err(EventError::InvalidReplayRequest {
            reason: "limit must be greater than zero".to_string(),
        });
    }
    let after = after.unwrap_or_default().as_u64();
    let text = std::str::from_utf8(bytes).map_err(|error| EventError::Serialize {
        reason: error.to_string(),
    })?;
    let mut entries = Vec::new();
    let mut current_cursor = 0u64;
    // Intentionally parse every non-empty line, including lines before the
    // requested cursor and beyond the returned page. The JSONL durability
    // contract treats malformed records as errors; skipping them would turn a
    // corrupted log into a partial replay.
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        current_cursor += 1;
        let record = serde_json::from_str::<T>(line).map_err(|error| EventError::Serialize {
            reason: error.to_string(),
        })?;
        if current_cursor > after && entries.len() < limit {
            entries.push(EventLogEntry {
                cursor: EventCursor::new(current_cursor),
                record,
            });
        }
    }
    // A cursor beyond the JSONL head is a foreign or stale cursor; the
    // contract requires explicit ReplayGap signaling rather than silently
    // echoing it, mirroring InMemoryDurableEventLog. Without this guard a
    // future filesystem JSONL backend would accept cursors this stream
    // never issued and hide records once new lines are appended.
    if after > current_cursor {
        return Err(EventError::ReplayGap {
            requested: EventCursor::new(after),
            earliest: EventCursor::new(current_cursor),
        });
    }
    let next_cursor = entries
        .last()
        .map(|entry| entry.cursor)
        .unwrap_or_else(|| EventCursor::new(after));
    Ok(EventReplay {
        entries,
        next_cursor,
    })
}

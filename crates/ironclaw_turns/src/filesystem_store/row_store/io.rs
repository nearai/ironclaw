use ironclaw_filesystem::{FilesystemError, SeqNo};
use ironclaw_host_api::ScopedPath;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::TurnError;

const ROW_ROOT: &str = "/turns/rows/v1";
const META_DIR: &str = "meta";
const META_FILE: &str = "state.json";
const EVENTS_INDEX_MARKER_FILE: &str = "events-index.json";
const DELTA_LOG: &str = "deltas/log";

pub(super) fn row_dir(collection: &str) -> Result<ScopedPath, TurnError> {
    scoped_row_path(format!("{ROW_ROOT}/{collection}"))
}

pub(super) fn row_path(collection: &str, key: &str) -> Result<ScopedPath, TurnError> {
    scoped_row_path(format!("{ROW_ROOT}/{collection}/{key}.json"))
}

pub(super) fn meta_path() -> Result<ScopedPath, TurnError> {
    scoped_row_path(format!("{ROW_ROOT}/{META_DIR}/{META_FILE}"))
}

/// Durable marker that the one-time backfill of `Entry::indexed` projections
/// onto pre-existing event rows has completed. Kept as its own record rather
/// than a field on [`RowStoreMeta`](super::delta::RowStoreMeta) so it stays
/// independent of the journal-seq/retention-floor forward-merge CAS logic that
/// governs the main meta record.
pub(super) fn events_index_marker_path() -> Result<ScopedPath, TurnError> {
    scoped_row_path(format!("{ROW_ROOT}/{META_DIR}/{EVENTS_INDEX_MARKER_FILE}"))
}

/// State of the durable event-index backfill migration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct EventsIndexMarker {
    /// `true` once every event row persisted before the indexed-projection
    /// change has been re-projected, so the query-backed read path will find
    /// historical events.
    #[serde(default)]
    pub(super) backfilled: bool,
}

pub(super) fn delta_log_path() -> Result<ScopedPath, TurnError> {
    scoped_row_path(format!("{ROW_ROOT}/{DELTA_LOG}"))
}

fn scoped_row_path(path: String) -> Result<ScopedPath, TurnError> {
    ScopedPath::new(path).map_err(|error| TurnError::Unavailable {
        reason: format!("invalid turn-state row path: {error}"),
    })
}

pub(super) fn deserialize_row<T>(bytes: &[u8], collection: &'static str) -> Result<T, TurnError>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(bytes).map_err(|error| TurnError::Unavailable {
        reason: format!("turn-state {collection} row deserialization failed: {error}"),
    })
}

#[derive(Serialize)]
struct MaterializedRowRef<'a, T> {
    journal_seq: SeqNo,
    value: Option<&'a T>,
}

#[derive(Deserialize)]
struct MaterializedRow<T> {
    journal_seq: SeqNo,
    value: Option<T>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredRow<T> {
    Materialized(MaterializedRow<T>),
    Raw(T),
}

pub(super) fn serialize_materialized_row<T>(
    journal_seq: SeqNo,
    value: Option<&T>,
    collection: &'static str,
) -> Result<Vec<u8>, TurnError>
where
    T: Serialize,
{
    serde_json::to_vec(&MaterializedRowRef { journal_seq, value }).map_err(|error| {
        TurnError::Unavailable {
            reason: format!("turn-state {collection} row serialization failed: {error}"),
        }
    })
}

pub(super) fn deserialize_materialized_row<T>(
    bytes: &[u8],
    collection: &'static str,
) -> Result<Option<T>, TurnError>
where
    T: DeserializeOwned,
{
    match serde_json::from_slice::<StoredRow<T>>(bytes).map_err(|error| TurnError::Unavailable {
        reason: format!("turn-state {collection} row deserialization failed: {error}"),
    })? {
        StoredRow::Materialized(row) => Ok(row.value),
        StoredRow::Raw(row) => Ok(Some(row)),
    }
}

pub(super) fn materialized_row_seq(
    bytes: &[u8],
    collection: &'static str,
) -> Result<SeqNo, TurnError> {
    match serde_json::from_slice::<StoredRow<serde_json::Value>>(bytes).map_err(|error| {
        TurnError::Unavailable {
            reason: format!("turn-state {collection} row deserialization failed: {error}"),
        }
    })? {
        StoredRow::Materialized(row) => Ok(row.journal_seq),
        StoredRow::Raw(_) => Ok(SeqNo::ZERO),
    }
}

pub(super) fn fs_error(error: FilesystemError) -> TurnError {
    tracing::debug!(%error, "turn state row-store filesystem operation failed");
    TurnError::Unavailable {
        reason: "turn state row-store persistence temporarily unavailable".to_string(),
    }
}

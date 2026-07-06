use ironclaw_filesystem::FilesystemError;
use ironclaw_host_api::ScopedPath;
use serde::de::DeserializeOwned;

use crate::TurnError;

const ROW_ROOT: &str = "/turns/rows/v1";
const META_DIR: &str = "meta";
const META_FILE: &str = "state.json";
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

pub(super) fn fs_error(error: FilesystemError) -> TurnError {
    tracing::debug!(%error, "turn state row-store filesystem operation failed");
    TurnError::Unavailable {
        reason: "turn state row-store persistence temporarily unavailable".to_string(),
    }
}

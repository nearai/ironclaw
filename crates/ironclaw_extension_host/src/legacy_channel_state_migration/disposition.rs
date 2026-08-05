use super::*;

pub(super) fn source_rows_digest(rows: &[VersionedEntry]) -> String {
    let mut ordered = rows.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.path.as_str().cmp(right.path.as_str()));
    let mut bytes = Vec::new();
    for row in ordered {
        let path = row.path.as_str().as_bytes();
        bytes.extend_from_slice(&(path.len() as u64).to_be_bytes());
        bytes.extend_from_slice(path);
        bytes.extend_from_slice(&(row.entry.body.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&row.entry.body);
    }
    sha256_digest_token(&bytes)
}

pub(super) async fn disposition_marker_matches<T>(
    filesystem: &Arc<dyn RootFilesystem>,
    path: &str,
    expected: &T,
) -> Result<bool, Rc1ChannelStateMigrationError>
where
    T: for<'de> Deserialize<'de> + PartialEq,
{
    let path = VirtualPath::new(path).map_err(log_malformed)?;
    let Some(entry) = filesystem.get(&path).await.map_err(log_unavailable)? else {
        return Ok(false);
    };
    let actual = serde_json::from_slice::<T>(&entry.entry.body).map_err(log_malformed)?;
    if &actual != expected {
        return Err(Rc1ChannelStateMigrationError::Conflict);
    }
    Ok(true)
}

pub(super) async fn commit_disposition_marker<T>(
    filesystem: &Arc<dyn RootFilesystem>,
    path: &str,
    marker: &T,
    already_complete: bool,
) -> Result<(), Rc1ChannelStateMigrationError>
where
    T: Serialize + for<'de> Deserialize<'de> + PartialEq,
{
    if already_complete {
        return Ok(());
    }
    let path = VirtualPath::new(path).map_err(log_malformed)?;
    let kind = ironclaw_filesystem::RecordKind::new("rc1_channel_state_disposition")
        .map_err(log_malformed)?;
    let value = serde_json::to_value(marker).map_err(log_unavailable)?;
    let entry = ironclaw_filesystem::Entry::record(kind, &value).map_err(log_unavailable)?;
    match filesystem
        .put(&path, entry, ironclaw_filesystem::CasExpectation::Absent)
        .await
    {
        Ok(_) => Ok(()),
        Err(FilesystemError::VersionMismatch { .. }) => {
            if disposition_marker_matches(filesystem, path.as_str(), marker).await? {
                Ok(())
            } else {
                Err(Rc1ChannelStateMigrationError::Conflict)
            }
        }
        Err(error) => Err(log_unavailable(error)),
    }
}

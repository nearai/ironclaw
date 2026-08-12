use async_trait::async_trait;
use ironclaw_host_api::path::VirtualPath;
use tokio::sync::OwnedMutexGuard;

use super::{
    State, StoredEntry, state_delete, state_get, state_put, state_reserve_sequence,
    update_materialized_indexes, with_trailing_slash,
};
use crate::{
    CasExpectation, Entry, FilesystemError, FilesystemOperation, RecordVersion, SeqNo, StorageTxn,
    VersionedEntry,
};

pub(super) struct InMemoryStorageTxn {
    pub(super) state: Option<OwnedMutexGuard<State>>,
    pub(super) undo: Vec<UndoAction>,
    pub(super) prefix: VirtualPath,
}

pub(super) enum UndoAction {
    Entry {
        path: VirtualPath,
        original: Option<StoredEntry>,
    },
    Delete {
        entries: Vec<(VirtualPath, StoredEntry)>,
        event_logs: Vec<(String, Vec<crate::EventRecord>)>,
        sequences: Vec<(String, SeqNo)>,
    },
    Sequence {
        path: String,
        original: Option<SeqNo>,
    },
}

impl InMemoryStorageTxn {
    fn check_path(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if crate::path_prefix_matches(self.prefix.as_str(), path.as_str()) {
            Ok(())
        } else {
            Err(FilesystemError::PathOutsideMount { path: path.clone() })
        }
    }

    fn state(&mut self) -> Result<&mut State, FilesystemError> {
        self.state
            .as_deref_mut()
            .ok_or_else(|| FilesystemError::Backend {
                path: self.prefix.clone(),
                operation: FilesystemOperation::BeginTxn,
                reason: "in-memory transaction already finished".to_string(),
            })
    }

    fn restore(&mut self) {
        let Some(state) = self.state.as_deref_mut() else {
            return;
        };
        for undo in self.undo.drain(..).rev() {
            match undo {
                UndoAction::Entry { path, original } => {
                    let current = state.entries.remove(&path);
                    update_materialized_indexes(
                        state,
                        &path,
                        current.as_ref().map(|stored| &stored.entry),
                        original.as_ref().map(|stored| &stored.entry),
                    );
                    if let Some(original) = original {
                        state.entries.insert(path, original);
                    }
                }
                UndoAction::Delete {
                    entries,
                    event_logs,
                    sequences,
                } => {
                    for (path, entry) in entries {
                        update_materialized_indexes(state, &path, None, Some(&entry.entry));
                        state.entries.insert(path, entry);
                    }
                    state.event_logs.extend(event_logs);
                    state.sequences.extend(sequences);
                }
                UndoAction::Sequence { path, original } => match original {
                    Some(sequence) => {
                        state.sequences.insert(path, sequence);
                    }
                    None => {
                        state.sequences.remove(&path);
                    }
                },
            }
        }
    }
}

#[async_trait]
impl StorageTxn for InMemoryStorageTxn {
    async fn put(
        &mut self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.check_path(path)?;
        let original = self.state()?.entries.get(path).cloned();
        self.undo.push(UndoAction::Entry {
            path: path.clone(),
            original,
        });
        state_put(self.state()?, path, entry, cas)
    }

    async fn get(&mut self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.check_path(path)?;
        Ok(state_get(self.state()?, path))
    }

    async fn delete(&mut self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.check_path(path)?;
        let prefix = with_trailing_slash(path.as_str());
        let state = self.state()?;
        let entries = state
            .entries
            .iter()
            .filter(|(candidate, _)| *candidate == path || candidate.as_str().starts_with(&prefix))
            .map(|(path, entry)| (path.clone(), entry.clone()))
            .collect();
        let event_logs = state
            .event_logs
            .iter()
            .filter(|(candidate, _)| {
                candidate.as_str() == path.as_str() || candidate.starts_with(&prefix)
            })
            .map(|(path, log)| (path.clone(), log.clone()))
            .collect();
        let sequences = state
            .sequences
            .iter()
            .filter(|(candidate, _)| {
                candidate.as_str() == path.as_str() || candidate.starts_with(&prefix)
            })
            .map(|(path, sequence)| (path.clone(), *sequence))
            .collect();
        self.undo.push(UndoAction::Delete {
            entries,
            event_logs,
            sequences,
        });
        state_delete(self.state()?, path)
    }

    async fn reserve_sequence(&mut self, path: &VirtualPath) -> Result<SeqNo, FilesystemError> {
        self.check_path(path)?;
        let path_key = path.as_str().to_string();
        let original = self.state()?.sequences.get(&path_key).copied();
        self.undo.push(UndoAction::Sequence {
            path: path_key,
            original,
        });
        Ok(state_reserve_sequence(self.state()?, path))
    }

    async fn reserve_sequence_range(
        &mut self,
        path: &VirtualPath,
        count: u64,
    ) -> Result<SeqNo, FilesystemError> {
        self.check_path(path)?;
        let path_key = path.as_str().to_string();
        let original = self.state()?.sequences.get(&path_key).copied();
        self.undo.push(UndoAction::Sequence {
            path: path_key,
            original,
        });
        let mut last = SeqNo::ZERO;
        for _ in 0..count {
            last = state_reserve_sequence(self.state()?, path);
        }
        Ok(last)
    }

    async fn commit(mut self: Box<Self>) -> Result<(), FilesystemError> {
        self.undo.clear();
        self.state = None;
        Ok(())
    }

    async fn rollback(mut self: Box<Self>) {
        self.restore();
        self.state = None;
    }
}

impl Drop for InMemoryStorageTxn {
    fn drop(&mut self) {
        self.restore();
    }
}

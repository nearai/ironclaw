//! Support types for the unified [`RootFilesystem`](crate::RootFilesystem) surface.
//!
//! This module deliberately does **not** define a parallel "backend trait".
//! `RootFilesystem` is the one trait that every backend implements; the
//! [`CompositeRootFilesystem`](crate::CompositeRootFilesystem) is itself a
//! `RootFilesystem` that dispatches by longest-prefix to mounted backends.
//! That keeps the codebase honest about a single dispatch fabric.
//!
//! What lives here:
//! - [`StorageTxn`] — the multi-key transactional handle that backends with
//!   `TxnCapability::MultiKey` expose via
//!   [`RootFilesystem::begin`](crate::RootFilesystem::begin). Stores must
//!   continue to work using only CAS (`put` + `CasExpectation::Version`);
//!   `StorageTxn` is a strictly stronger primitive offered as an optimisation
//!   to backends that have it natively.
//! - [`EventRecord`] — one entry emitted by the append/tail plane.

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::{
    CasExpectation, Entry, FilesystemError, FilesystemOperation, RecordVersion, SeqNo,
    VersionedEntry,
};

/// Multi-key transactional handle returned by [`RootFilesystem::begin`].
///
/// All operations scope to the prefix that produced the transaction; reaching
/// outside the prefix fails closed with [`FilesystemError::PathOutsideMount`].
/// The handle must be either [`commit`](Self::commit)-ed or
/// [`rollback`](Self::rollback)-ed exactly once; dropping the handle without
/// either is equivalent to rollback.
#[async_trait]
pub trait StorageTxn: Send {
    async fn put(
        &mut self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError>;

    async fn get(&mut self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError>;

    async fn delete(&mut self, path: &VirtualPath) -> Result<(), FilesystemError>;

    async fn reserve_sequence(&mut self, path: &VirtualPath) -> Result<SeqNo, FilesystemError> {
        Err(FilesystemError::Unsupported {
            path: path.clone(),
            operation: FilesystemOperation::ReserveSeq,
        })
    }

    async fn commit(self: Box<Self>) -> Result<(), FilesystemError>;

    async fn rollback(self: Box<Self>);
}

/// One event in the append/tail plane.
///
/// `seq` is monotonically increasing per `path` (the event log). `payload`
/// is opaque to the filesystem; consumers serialize their own event envelope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRecord {
    pub seq: SeqNo,
    pub payload: Vec<u8>,
}

impl EventRecord {
    pub fn new(seq: SeqNo, payload: Vec<u8>) -> Self {
        Self { seq, payload }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DummyTxn;

    #[async_trait]
    impl StorageTxn for DummyTxn {
        async fn put(
            &mut self,
            path: &VirtualPath,
            _entry: Entry,
            _cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
            })
        }

        async fn get(
            &mut self,
            path: &VirtualPath,
        ) -> Result<Option<VersionedEntry>, FilesystemError> {
            Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
            })
        }

        async fn delete(&mut self, path: &VirtualPath) -> Result<(), FilesystemError> {
            Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::Delete,
            })
        }

        async fn commit(self: Box<Self>) -> Result<(), FilesystemError> {
            Ok(())
        }

        async fn rollback(self: Box<Self>) {}
    }

    #[tokio::test]
    async fn storage_txn_reserve_sequence_fails_closed_by_default() {
        let path = VirtualPath::new("/events/log").unwrap();
        let mut txn = DummyTxn;

        let err = txn.reserve_sequence(&path).await.unwrap_err();

        assert!(matches!(
            err,
            FilesystemError::Unsupported { path: actual, operation: FilesystemOperation::ReserveSeq }
                if actual == path
        ));
    }
}

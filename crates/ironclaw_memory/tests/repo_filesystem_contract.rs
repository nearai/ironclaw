//! Wires [`FilesystemMemoryDocumentRepository`] (over an in-memory
//! [`RootFilesystem`]) to the shared [`MemoryDocumentRepository`]
//! contract suite.
//!
//! See `crates/ironclaw_memory/src/contract_tests.rs` for the suite
//! itself and the rationale (#3890 / .claude/rules/testing.md).
//!
//! Each contract gets its own `InMemoryBackend` — the factory closure
//! constructs both the backing filesystem and the repo per call, so
//! contracts cannot leak state into each other.

use std::sync::Arc;

use ironclaw_filesystem::InMemoryBackend;
use ironclaw_memory::{FilesystemMemoryDocumentRepository, contract_test_indexed};

// FilesystemMemoryDocumentRepository implements MemoryDocumentIndexRepository
// and serves FTS search, so it gets the chunk-seeded search-isolation
// contract via `contract_test_indexed!`.
contract_test_indexed!(filesystem, || {
    FilesystemMemoryDocumentRepository::new(Arc::new(InMemoryBackend::new()))
});

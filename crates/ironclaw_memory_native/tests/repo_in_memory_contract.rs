//! Wires [`InMemoryMemoryDocumentRepository`] to the shared
//! [`MemoryDocumentRepository`] contract suite.
//!
//! See `crates/ironclaw_memory_native/src/contract_tests.rs` for the suite
//! itself and the rationale (#3890 / .claude/rules/testing.md). One
//! `contract_test!` invocation expands to one `#[tokio::test]` per
//! contract, named `in_memory::<contract_name>` so failures attribute
//! cleanly to this impl.

use ironclaw_memory_native::{InMemoryMemoryDocumentRepository, contract_test};

contract_test!(in_memory, InMemoryMemoryDocumentRepository::new);

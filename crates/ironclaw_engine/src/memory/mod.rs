//! Memory document system.
//!
//! - [`MemoryStore`] — project-scoped document CRUD
//! - [`RetrievalEngine`] — context building from project docs via keyword search

pub mod retrieval;
pub mod store;

pub use retrieval::RetrievalEngine;
pub use store::MemoryStore;

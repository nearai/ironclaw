//! Broadcast channel for collection write events.
//!
//! Decouples the write path (event ingest HTTP handler + CollectionAddTool)
//! from the routine engine that processes CollectionWrite triggers.

use serde_json::Value;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Emitted when a record is written to a structured collection.
#[derive(Debug, Clone)]
pub struct CollectionWriteEvent {
    /// The user who owns the collection.
    pub user_id: String,
    /// The collection that was written to.
    pub collection: String,
    /// The ID of the record.
    pub record_id: Uuid,
    /// The operation that was performed: "insert", "update", or "delete".
    pub operation: String,
    /// The record data as written (Null for deletes).
    pub data: Value,
}

/// Create a broadcast channel for collection write events.
///
/// Returns a `(Sender, Receiver)` pair. The sender should be shared with write
/// paths (HTTP handlers, collection tools) and the receiver cloned into the
/// routine engine.
pub fn collection_write_channel() -> (
    broadcast::Sender<CollectionWriteEvent>,
    broadcast::Receiver<CollectionWriteEvent>,
) {
    broadcast::channel(256)
}

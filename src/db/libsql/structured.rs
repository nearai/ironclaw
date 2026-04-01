//! LibSQL implementation of StructuredStore.
//!
//! Stub implementation — returns errors for all operations since structured
//! collections are currently only used with the PostgreSQL backend.

use async_trait::async_trait;
use uuid::Uuid;

use crate::db::structured::{Aggregation, CollectionSchema, Filter, Record, StructuredStore};
use crate::error::DatabaseError;

use super::LibSqlBackend;

#[async_trait]
impl StructuredStore for LibSqlBackend {
    async fn register_collection(
        &self,
        _user_id: &str,
        _schema: &CollectionSchema,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Query(
            "Structured collections are not yet supported on the libSQL backend".to_string(),
        ))
    }

    async fn get_collection_schema(
        &self,
        _user_id: &str,
        _collection: &str,
    ) -> Result<CollectionSchema, DatabaseError> {
        Err(DatabaseError::Query(
            "Structured collections are not yet supported on the libSQL backend".to_string(),
        ))
    }

    async fn list_collections(
        &self,
        _user_id: &str,
    ) -> Result<Vec<CollectionSchema>, DatabaseError> {
        Err(DatabaseError::Query(
            "Structured collections are not yet supported on the libSQL backend".to_string(),
        ))
    }

    async fn drop_collection(
        &self,
        _user_id: &str,
        _collection: &str,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Query(
            "Structured collections are not yet supported on the libSQL backend".to_string(),
        ))
    }

    async fn insert_record(
        &self,
        _user_id: &str,
        _collection: &str,
        _data: serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        Err(DatabaseError::Query(
            "Structured collections are not yet supported on the libSQL backend".to_string(),
        ))
    }

    async fn get_record(
        &self,
        _user_id: &str,
        _record_id: Uuid,
    ) -> Result<Record, DatabaseError> {
        Err(DatabaseError::Query(
            "Structured collections are not yet supported on the libSQL backend".to_string(),
        ))
    }

    async fn update_record(
        &self,
        _user_id: &str,
        _record_id: Uuid,
        _updates: serde_json::Value,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Query(
            "Structured collections are not yet supported on the libSQL backend".to_string(),
        ))
    }

    async fn delete_record(
        &self,
        _user_id: &str,
        _record_id: Uuid,
    ) -> Result<(), DatabaseError> {
        Err(DatabaseError::Query(
            "Structured collections are not yet supported on the libSQL backend".to_string(),
        ))
    }

    async fn query_records(
        &self,
        _user_id: &str,
        _collection: &str,
        _filters: &[Filter],
        _order_by: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<Record>, DatabaseError> {
        Err(DatabaseError::Query(
            "Structured collections are not yet supported on the libSQL backend".to_string(),
        ))
    }

    async fn aggregate(
        &self,
        _user_id: &str,
        _collection: &str,
        _aggregation: &Aggregation,
    ) -> Result<serde_json::Value, DatabaseError> {
        Err(DatabaseError::Query(
            "Structured collections are not yet supported on the libSQL backend".to_string(),
        ))
    }
}

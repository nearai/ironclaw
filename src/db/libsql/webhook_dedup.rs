//! Webhook deduplication WebhookDedupStore implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;
use uuid::Uuid;

use super::LibSqlBackend;
use crate::db::WebhookDedupStore;
use crate::error::DatabaseError;

#[async_trait]
impl WebhookDedupStore for LibSqlBackend {
    async fn record_webhook_message_processed(
        &self,
        channel: &str,
        external_message_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4().to_string();
        let rows_affected = conn
            .execute(
                "INSERT INTO webhook_message_dedup (id, channel, external_message_id) \
                 VALUES (?1, ?2, ?3) \
                 ON CONFLICT(channel, external_message_id) DO NOTHING",
                params![id, channel, external_message_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(rows_affected > 0)
    }

    async fn cleanup_old_webhook_dedup_records(&self) -> Result<u64, DatabaseError> {
        let conn = self.connect().await?;
        let rows_affected = conn
            .execute(
                "DELETE FROM webhook_message_dedup \
                 WHERE datetime(processed_at) < datetime('now', '-7 days')",
                params![],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(rows_affected)
    }
}

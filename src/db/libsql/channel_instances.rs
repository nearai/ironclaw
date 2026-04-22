//! ChannelInstanceStore implementation for LibSqlBackend.

use async_trait::async_trait;
use libsql::params;

use super::{LibSqlBackend, fmt_ts, get_i64, get_json, get_opt_text, get_ts, opt_text};
use crate::db::{ChannelInstanceRecord, ChannelInstanceStore, DatabaseError};

fn row_to_channel_instance(row: &libsql::Row) -> Result<ChannelInstanceRecord, DatabaseError> {
    let id: String = row
        .get(0)
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
    Ok(ChannelInstanceRecord {
        id: uuid::Uuid::parse_str(&id).map_err(|e| DatabaseError::Query(e.to_string()))?,
        user_id: row
            .get(1)
            .map_err(|e| DatabaseError::Query(e.to_string()))?,
        channel_kind: row
            .get(2)
            .map_err(|e| DatabaseError::Query(e.to_string()))?,
        instance_key: row
            .get(3)
            .map_err(|e| DatabaseError::Query(e.to_string()))?,
        display_name: row
            .get(4)
            .map_err(|e| DatabaseError::Query(e.to_string()))?,
        is_primary: get_i64(row, 5) != 0,
        enabled: get_i64(row, 6) != 0,
        config: get_json(row, 7),
        metadata: get_json(row, 8),
        last_error: get_opt_text(row, 9),
        created_at: get_ts(row, 10),
        updated_at: get_ts(row, 11),
    })
}

#[async_trait]
impl ChannelInstanceStore for LibSqlBackend {
    async fn create_channel_instance(
        &self,
        instance: &ChannelInstanceRecord,
    ) -> Result<(), DatabaseError> {
        let channel_kind = crate::pairing::normalize_channel_name(&instance.channel_kind);
        let config_str = serde_json::to_string(&instance.config)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let metadata_str = serde_json::to_string(&instance.metadata)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO channel_instances (
                 id, user_id, channel_kind, instance_key, display_name,
                 is_primary, enabled, config, metadata, last_error, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                instance.id.to_string(),
                instance.user_id.as_str(),
                channel_kind.as_str(),
                instance.instance_key.as_str(),
                instance.display_name.as_str(),
                if instance.is_primary { 1i64 } else { 0i64 },
                if instance.enabled { 1i64 } else { 0i64 },
                config_str.as_str(),
                metadata_str.as_str(),
                opt_text(instance.last_error.as_deref()),
                fmt_ts(&instance.created_at),
                fmt_ts(&instance.updated_at),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn update_channel_instance(
        &self,
        instance: &ChannelInstanceRecord,
    ) -> Result<(), DatabaseError> {
        let channel_kind = crate::pairing::normalize_channel_name(&instance.channel_kind);
        let config_str = serde_json::to_string(&instance.config)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let metadata_str = serde_json::to_string(&instance.metadata)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let conn = self.connect().await?;
        let updated = conn
            .execute(
                "UPDATE channel_instances
                 SET channel_kind = ?1,
                     display_name = ?2,
                     is_primary = ?3,
                     enabled = ?4,
                     config = ?5,
                     metadata = ?6,
                     last_error = ?7,
                     updated_at = ?8
                 WHERE instance_key = ?9",
                params![
                    channel_kind.as_str(),
                    instance.display_name.as_str(),
                    if instance.is_primary { 1i64 } else { 0i64 },
                    if instance.enabled { 1i64 } else { 0i64 },
                    config_str.as_str(),
                    metadata_str.as_str(),
                    opt_text(instance.last_error.as_deref()),
                    fmt_ts(&instance.updated_at),
                    instance.instance_key.as_str(),
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if updated == 0 {
            return Err(DatabaseError::NotFound {
                entity: "channel_instance".to_string(),
                id: instance.instance_key.clone(),
            });
        }
        Ok(())
    }

    async fn get_channel_instance_by_key(
        &self,
        instance_key: &str,
    ) -> Result<Option<ChannelInstanceRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, channel_kind, instance_key, display_name,
                        is_primary, enabled, config, metadata, last_error,
                        created_at, updated_at
                 FROM channel_instances
                 WHERE instance_key = ?1",
                params![instance_key],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_channel_instance(&row).map(Some),
            None => Ok(None),
        }
    }

    async fn list_channel_instances_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<ChannelInstanceRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, channel_kind, instance_key, display_name,
                        is_primary, enabled, config, metadata, last_error,
                        created_at, updated_at
                 FROM channel_instances
                 WHERE user_id = ?1
                 ORDER BY channel_kind ASC, display_name ASC, instance_key ASC",
                params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            result.push(row_to_channel_instance(&row)?);
        }
        Ok(result)
    }

    async fn list_channel_instances_for_user_and_kind(
        &self,
        user_id: &str,
        channel_kind: &str,
    ) -> Result<Vec<ChannelInstanceRecord>, DatabaseError> {
        let channel_kind = crate::pairing::normalize_channel_name(channel_kind);
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, channel_kind, instance_key, display_name,
                        is_primary, enabled, config, metadata, last_error,
                        created_at, updated_at
                 FROM channel_instances
                 WHERE user_id = ?1 AND channel_kind = ?2
                 ORDER BY is_primary DESC, display_name ASC, instance_key ASC",
                params![user_id, channel_kind.as_str()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            result.push(row_to_channel_instance(&row)?);
        }
        Ok(result)
    }

    async fn get_primary_channel_instance(
        &self,
        user_id: &str,
        channel_kind: &str,
    ) -> Result<Option<ChannelInstanceRecord>, DatabaseError> {
        let channel_kind = crate::pairing::normalize_channel_name(channel_kind);
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, channel_kind, instance_key, display_name,
                        is_primary, enabled, config, metadata, last_error,
                        created_at, updated_at
                 FROM channel_instances
                 WHERE user_id = ?1 AND channel_kind = ?2 AND is_primary = 1
                 LIMIT 1",
                params![user_id, channel_kind.as_str()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => row_to_channel_instance(&row).map(Some),
            None => Ok(None),
        }
    }

    async fn list_enabled_channel_instances(
        &self,
    ) -> Result<Vec<ChannelInstanceRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, channel_kind, instance_key, display_name,
                        is_primary, enabled, config, metadata, last_error,
                        created_at, updated_at
                 FROM channel_instances
                 WHERE enabled = 1
                 ORDER BY user_id ASC, channel_kind ASC, instance_key ASC",
                (),
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            result.push(row_to_channel_instance(&row)?);
        }
        Ok(result)
    }

    async fn delete_channel_instance(&self, instance_key: &str) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM channel_instances WHERE instance_key = ?1",
                params![instance_key],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(deleted > 0)
    }
}

#[cfg(test)]
mod tests {
    use crate::db::libsql::LibSqlBackend;
    use crate::db::{ChannelInstanceRecord, ChannelInstanceStore, Database, UserRecord, UserStore};

    async fn setup_db() -> (LibSqlBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("channel_instances_test.db");
        let db = LibSqlBackend::new_local(&db_path).await.unwrap();
        db.run_migrations().await.unwrap();
        (db, dir)
    }

    async fn setup_db_with_user(user_id: &str) -> (LibSqlBackend, tempfile::TempDir) {
        let (db, dir) = setup_db().await;
        db.get_or_create_user(UserRecord {
            id: user_id.to_string(),
            role: "member".to_string(),
            display_name: user_id.to_string(),
            status: "active".to_string(),
            email: None,
            last_login_at: None,
            created_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
        (db, dir)
    }

    fn sample_instance(
        user_id: &str,
        channel_kind: &str,
        instance_key: &str,
    ) -> ChannelInstanceRecord {
        let now = chrono::Utc::now();
        ChannelInstanceRecord {
            id: uuid::Uuid::new_v4(),
            user_id: user_id.to_string(),
            channel_kind: channel_kind.to_string(),
            instance_key: instance_key.to_string(),
            display_name: format!("{} {}", user_id, channel_kind),
            is_primary: true,
            enabled: true,
            config: serde_json::json!({"polling": true}),
            metadata: serde_json::json!({"source": "test"}),
            last_error: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn test_create_and_fetch_channel_instance_roundtrip() {
        let (db, _dir) = setup_db_with_user("alice").await;
        let instance = sample_instance("alice", "TeLeGrAm", "telegram:alice:primary");

        db.create_channel_instance(&instance).await.unwrap();

        let fetched = db
            .get_channel_instance_by_key("telegram:alice:primary")
            .await
            .unwrap()
            .expect("instance should exist");
        assert_eq!(fetched.instance_key, "telegram:alice:primary");
        assert_eq!(fetched.user_id, "alice");
        assert_eq!(fetched.channel_kind, "telegram");
        assert!(fetched.enabled);
        assert!(fetched.is_primary);
        assert_eq!(fetched.config, serde_json::json!({"polling": true}));
        assert_eq!(fetched.metadata, serde_json::json!({"source": "test"}));
    }

    #[tokio::test]
    async fn test_update_channel_instance_persists_state() {
        let (db, _dir) = setup_db_with_user("alice").await;
        let mut instance = sample_instance("alice", "telegram", "telegram:alice:primary");
        db.create_channel_instance(&instance).await.unwrap();

        instance.display_name = "Alice Telegram Bot".to_string();
        instance.enabled = false;
        instance.last_error = Some("bot token invalid".to_string());
        instance.config = serde_json::json!({"polling": false});
        instance.metadata = serde_json::json!({"source": "updated"});
        instance.updated_at = chrono::Utc::now() + chrono::Duration::seconds(30);

        db.update_channel_instance(&instance).await.unwrap();

        let fetched = db
            .get_channel_instance_by_key("telegram:alice:primary")
            .await
            .unwrap()
            .expect("instance should still exist");
        assert_eq!(fetched.display_name, "Alice Telegram Bot");
        assert!(!fetched.enabled);
        assert_eq!(fetched.last_error.as_deref(), Some("bot token invalid"));
        assert_eq!(fetched.config, serde_json::json!({"polling": false}));
        assert_eq!(fetched.metadata, serde_json::json!({"source": "updated"}));
    }

    #[tokio::test]
    async fn test_get_primary_channel_instance_filters_to_primary_instance() {
        let (db, _dir) = setup_db_with_user("alice").await;
        let primary = sample_instance("alice", "telegram", "telegram:alice:primary");
        let mut secondary = sample_instance("alice", "telegram", "telegram:alice:backup");
        secondary.id = uuid::Uuid::new_v4();
        secondary.instance_key = "telegram:alice:backup".to_string();
        secondary.display_name = "Alice Telegram Backup".to_string();
        secondary.is_primary = false;

        db.create_channel_instance(&primary).await.unwrap();
        db.create_channel_instance(&secondary).await.unwrap();

        let resolved = db
            .get_primary_channel_instance("alice", "TeLeGrAm")
            .await
            .unwrap()
            .expect("primary instance should resolve");
        assert_eq!(resolved.instance_key, "telegram:alice:primary");
    }

    #[tokio::test]
    async fn test_duplicate_primary_instance_for_same_user_and_kind_is_rejected() {
        let (db, _dir) = setup_db_with_user("alice").await;
        let primary = sample_instance("alice", "telegram", "telegram:alice:primary");
        let mut duplicate_primary =
            sample_instance("alice", "telegram", "telegram:alice:second-primary");
        duplicate_primary.id = uuid::Uuid::new_v4();

        db.create_channel_instance(&primary).await.unwrap();
        let err = db.create_channel_instance(&duplicate_primary).await;
        assert!(
            err.is_err(),
            "a second primary instance for the same user+kind should fail"
        );
    }

    #[tokio::test]
    async fn test_list_enabled_channel_instances_filters_disabled_rows() {
        let (db, _dir) = setup_db().await;
        for user_id in ["alice", "bob"] {
            db.get_or_create_user(UserRecord {
                id: user_id.to_string(),
                role: "member".to_string(),
                display_name: user_id.to_string(),
                status: "active".to_string(),
                email: None,
                last_login_at: None,
                created_by: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                metadata: serde_json::json!({}),
            })
            .await
            .unwrap();
        }

        let enabled = sample_instance("alice", "telegram", "telegram:alice:primary");
        let mut disabled = sample_instance("bob", "telegram", "telegram:bob:primary");
        disabled.id = uuid::Uuid::new_v4();
        disabled.enabled = false;

        db.create_channel_instance(&enabled).await.unwrap();
        db.create_channel_instance(&disabled).await.unwrap();

        let enabled_rows = db.list_enabled_channel_instances().await.unwrap();
        assert_eq!(enabled_rows.len(), 1);
        assert_eq!(enabled_rows[0].instance_key, "telegram:alice:primary");
    }
}

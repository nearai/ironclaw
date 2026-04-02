//! WorkspaceMgmtStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::Utc;
use libsql::params;
use uuid::Uuid;

use super::{LibSqlBackend, fmt_ts, get_json, get_opt_text, get_text, get_ts, opt_text};
use crate::db::{
    DatabaseError, UserRecord, WorkspaceMemberRecord, WorkspaceMembership, WorkspaceMgmtStore,
    WorkspaceRecord,
};

fn parse_uuid_field(value: &str, field: &str) -> Result<Uuid, DatabaseError> {
    value.parse().map_err(|e| {
        DatabaseError::Serialization(format!("Failed to parse {field} UUID '{value}': {e}"))
    })
}

fn row_to_workspace(row: &libsql::Row) -> Result<WorkspaceRecord, DatabaseError> {
    Ok(WorkspaceRecord {
        id: parse_uuid_field(&get_text(row, 0), "workspaces.id")?,
        name: get_text(row, 1),
        slug: get_text(row, 2),
        description: get_text(row, 3),
        status: get_text(row, 4),
        created_at: get_ts(row, 5),
        updated_at: get_ts(row, 6),
        created_by: get_text(row, 7),
        settings: get_json(row, 8),
    })
}

fn row_to_user(row: &libsql::Row, offset: i32) -> Result<UserRecord, DatabaseError> {
    let metadata = get_json(row, offset + 9);
    Ok(UserRecord {
        id: get_text(row, offset),
        email: get_opt_text(row, offset + 1),
        display_name: get_text(row, offset + 2),
        status: get_text(row, offset + 3),
        role: get_text(row, offset + 4),
        created_at: get_ts(row, offset + 5),
        updated_at: get_ts(row, offset + 6),
        last_login_at: get_opt_text(row, offset + 7)
            .map(|s| super::parse_timestamp(&s))
            .transpose()
            .map_err(DatabaseError::Serialization)?,
        created_by: get_opt_text(row, offset + 8),
        metadata,
    })
}

#[async_trait]
impl WorkspaceMgmtStore for LibSqlBackend {
    async fn create_workspace(
        &self,
        name: &str,
        slug: &str,
        description: &str,
        created_by: &str,
        settings: &serde_json::Value,
    ) -> Result<WorkspaceRecord, DatabaseError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let id = Uuid::new_v4();
        let now = fmt_ts(&Utc::now());
        let result: Result<WorkspaceRecord, DatabaseError> = async {
            conn.execute(
                r#"
                INSERT INTO workspaces (id, name, slug, description, status, created_at, updated_at, created_by, settings)
                VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?5, ?6, ?7)
                "#,
                params![
                    id.to_string(),
                    name,
                    slug,
                    description,
                    now.as_str(),
                    created_by,
                    settings.to_string()
                ],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

            conn.execute(
                r#"
                INSERT INTO workspace_members (workspace_id, user_id, role, joined_at, invited_by)
                VALUES (?1, ?2, 'owner', ?3, ?2)
                "#,
                params![id.to_string(), created_by, now.as_str()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

            Ok(WorkspaceRecord {
                id,
                name: name.to_string(),
                slug: slug.to_string(),
                description: description.to_string(),
                status: "active".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                created_by: created_by.to_string(),
                settings: settings.clone(),
            })
        }
        .await;

        match &result {
            Ok(_) => {
                conn.execute("COMMIT", ())
                    .await
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
            }
            Err(_) => {
                let _ = conn.execute("ROLLBACK", ()).await;
            }
        }

        result
    }

    async fn get_workspace(&self, id: Uuid) -> Result<Option<WorkspaceRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, name, slug, description, status, created_at, updated_at, created_by, settings
                FROM workspaces WHERE id = ?1
                "#,
                params![id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_workspace(&row)?)),
            None => Ok(None),
        }
    }

    async fn get_workspace_by_slug(
        &self,
        slug: &str,
    ) -> Result<Option<WorkspaceRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, name, slug, description, status, created_at, updated_at, created_by, settings
                FROM workspaces WHERE slug = ?1
                "#,
                params![slug],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(row_to_workspace(&row)?)),
            None => Ok(None),
        }
    }

    async fn list_workspaces_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<WorkspaceMembership>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    w.id, w.name, w.slug, w.description, w.status, w.created_at, w.updated_at, w.created_by, w.settings,
                    wm.role
                FROM workspace_members wm
                JOIN workspaces w ON w.id = wm.workspace_id
                WHERE wm.user_id = ?1
                  AND w.status != 'archived'
                ORDER BY w.created_at DESC
                "#,
                params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut memberships = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            memberships.push(WorkspaceMembership {
                workspace: row_to_workspace(&row)?,
                role: get_text(&row, 9),
            });
        }
        Ok(memberships)
    }

    async fn update_workspace(
        &self,
        id: Uuid,
        name: &str,
        description: &str,
        settings: &serde_json::Value,
    ) -> Result<Option<WorkspaceRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let now = fmt_ts(&Utc::now());
        let updated = conn
            .execute(
                r#"
                UPDATE workspaces
                SET name = ?2, description = ?3, settings = ?4, updated_at = ?5
                WHERE id = ?1
                "#,
                params![id.to_string(), name, description, settings.to_string(), now],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        if updated == 0 {
            return Ok(None);
        }
        self.get_workspace(id).await
    }

    async fn archive_workspace(&self, id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let updated = conn
            .execute(
                "UPDATE workspaces SET status = 'archived', updated_at = ?2 WHERE id = ?1",
                params![id.to_string(), fmt_ts(&Utc::now())],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(updated > 0)
    }

    async fn add_workspace_member(
        &self,
        workspace_id: Uuid,
        user_id: &str,
        role: &str,
        invited_by: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO workspace_members (workspace_id, user_id, role, joined_at, invited_by)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT (workspace_id, user_id) DO UPDATE SET
                role = excluded.role,
                invited_by = excluded.invited_by,
                joined_at = excluded.joined_at
            "#,
            params![
                workspace_id.to_string(),
                user_id,
                role,
                fmt_ts(&Utc::now()),
                opt_text(invited_by)
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(())
    }

    async fn remove_workspace_member(
        &self,
        workspace_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM workspace_members WHERE workspace_id = ?1 AND user_id = ?2",
                params![workspace_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(deleted > 0)
    }

    async fn list_workspace_members(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<(UserRecord, WorkspaceMemberRecord)>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    u.id, u.email, u.display_name, u.status, u.role, u.created_at, u.updated_at, u.last_login_at, u.created_by, u.metadata,
                    wm.workspace_id, wm.user_id, wm.role, wm.joined_at, wm.invited_by
                FROM workspace_members wm
                JOIN users u ON u.id = wm.user_id
                WHERE wm.workspace_id = ?1
                ORDER BY wm.joined_at ASC
                "#,
                params![workspace_id.to_string()],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut members = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let user = row_to_user(&row, 0)?;
            let membership = WorkspaceMemberRecord {
                workspace_id: parse_uuid_field(
                    &get_text(&row, 10),
                    "workspace_members.workspace_id",
                )?,
                user_id: get_text(&row, 11),
                role: get_text(&row, 12),
                joined_at: get_ts(&row, 13),
                invited_by: get_opt_text(&row, 14),
            };
            members.push((user, membership));
        }
        Ok(members)
    }

    async fn get_member_role(
        &self,
        workspace_id: Uuid,
        user_id: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT role FROM workspace_members WHERE workspace_id = ?1 AND user_id = ?2",
                params![workspace_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => Ok(Some(get_text(&row, 0))),
            None => Ok(None),
        }
    }

    async fn is_last_workspace_owner(
        &self,
        workspace_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT
                    SUM(CASE WHEN user_id = ?2 AND role = 'owner' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN role = 'owner' THEN 1 ELSE 0 END)
                FROM workspace_members
                WHERE workspace_id = ?1
                "#,
                params![workspace_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        match rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            Some(row) => {
                let target_is_owner = row.get::<i64>(0).unwrap_or(0) > 0;
                let owner_count = row.get::<i64>(1).unwrap_or(0);
                Ok(target_is_owner && owner_count <= 1)
            }
            None => Ok(false),
        }
    }

    async fn update_member_role(
        &self,
        workspace_id: Uuid,
        user_id: &str,
        role: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let updated = conn
            .execute(
                "UPDATE workspace_members SET role = ?3 WHERE workspace_id = ?1 AND user_id = ?2",
                params![workspace_id.to_string(), user_id, role],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(updated > 0)
    }

    async fn is_workspace_member(
        &self,
        workspace_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT 1 FROM workspace_members WHERE workspace_id = ?1 AND user_id = ?2",
                params![workspace_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        Ok(rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .is_some())
    }
}

//! Storage trait + libSQL implementation for the legal harness.
//!
//! The trait surface is intentionally narrow: the only operations the
//! Stream B chat handlers perform are project lookup (to confirm the
//! project still exists and isn't soft-deleted), document-text retrieval
//! (for RAG context assembly), chat CRUD, and message append/list.
//! Stream A's foundation layer owns the project+document writes; Stream C
//! reads chat messages for DOCX export. This trait covers Stream B's
//! reads + chat writes only.
//!
//! IDs are stored as `TEXT`. The schema spec calls them ULIDs; this
//! crate ships v4 UUIDs in their hyphenated form to avoid pulling a new
//! dependency. The schema accepts any `TEXT` value, so this is a private
//! implementation detail that a downstream stream can swap out without a
//! migration.

use async_trait::async_trait;

use crate::error::DatabaseError;

/// Role on a chat message. Mirrors the CHECK constraint in the
/// `legal_chat_messages` table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegalRole {
    User,
    Assistant,
    System,
    Tool,
}

impl LegalRole {
    /// Stable lower-case identifier matching the CHECK constraint.
    pub fn as_str(self) -> &'static str {
        match self {
            LegalRole::User => "user",
            LegalRole::Assistant => "assistant",
            LegalRole::System => "system",
            LegalRole::Tool => "tool",
        }
    }

    /// Parse a role string from the database. Unknown values are an error
    /// because the CHECK constraint should prevent them upstream.
    pub fn parse(raw: &str) -> Result<Self, DatabaseError> {
        Ok(match raw {
            "user" => LegalRole::User,
            "assistant" => LegalRole::Assistant,
            "system" => LegalRole::System,
            "tool" => LegalRole::Tool,
            other => {
                return Err(DatabaseError::Query(format!(
                    "legal_chat_messages.role: unknown role {other:?}"
                )));
            }
        })
    }
}

/// Minimal project metadata used by chat handlers — enough to confirm
/// the project still exists, surface a name in the response payload, and
/// reject writes against soft-deleted projects.
#[derive(Debug, Clone)]
pub struct LegalProjectMeta {
    pub id: String,
    pub name: String,
    pub deleted_at: Option<i64>,
    pub created_at: i64,
}

/// A chat thread within a project.
#[derive(Debug, Clone)]
pub struct LegalChat {
    pub id: String,
    pub project_id: String,
    pub title: Option<String>,
    pub created_at: i64,
}

/// One message on a chat thread.
#[derive(Debug, Clone)]
pub struct LegalChatMessage {
    pub id: String,
    pub chat_id: String,
    pub role: LegalRole,
    pub content: String,
    /// JSON-encoded array of referenced document ids, or `None` when the
    /// caller did not attach any document refs to the message.
    pub document_refs: Option<String>,
    pub created_at: i64,
}

/// Just enough document text to assemble a RAG prompt without dragging
/// blob bytes or page counts through the pipeline.
#[derive(Debug, Clone)]
pub struct LegalDocumentText {
    pub id: String,
    pub filename: String,
    /// `extracted_text` from the row. `None` here represents "extraction
    /// not yet complete or returned NULL" — handlers must skip these
    /// rather than serialize "null" into the prompt.
    pub extracted_text: Option<String>,
}

/// Storage operations Stream B's chat handlers need. Implemented for the
/// libSQL backend below; the abstraction exists so tests can plug in a
/// stub for the LLM-call boundary while still exercising the real DB.
#[async_trait]
pub trait LegalStore: Send + Sync {
    /// Look up an active or deleted project by id. Returns `None` when no
    /// row matches. Callers decide whether `deleted_at.is_some()` is an
    /// error condition.
    async fn project_meta(
        &self,
        project_id: &str,
    ) -> Result<Option<LegalProjectMeta>, DatabaseError>;

    /// Pull every document's `extracted_text` for a project, in upload
    /// order. Caller is responsible for skipping rows whose extraction
    /// hasn't completed (`extracted_text` is `None`).
    async fn project_document_texts(
        &self,
        project_id: &str,
    ) -> Result<Vec<LegalDocumentText>, DatabaseError>;

    /// Insert a new chat row and return it.
    async fn create_chat(
        &self,
        project_id: &str,
        title: Option<&str>,
    ) -> Result<LegalChat, DatabaseError>;

    /// Look up a chat by id. Returns `None` when no row matches.
    async fn get_chat(&self, chat_id: &str) -> Result<Option<LegalChat>, DatabaseError>;

    /// List chats in a project (oldest first). Returns an empty vector if
    /// the project has no chats yet.
    async fn list_chats_for_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<LegalChat>, DatabaseError>;

    /// List messages on a chat (oldest first).
    async fn list_messages_for_chat(
        &self,
        chat_id: &str,
    ) -> Result<Vec<LegalChatMessage>, DatabaseError>;

    /// Insert a message on a chat. Returns the persisted row, including
    /// the server-assigned `id` and `created_at`.
    async fn append_message(
        &self,
        chat_id: &str,
        role: LegalRole,
        content: &str,
        document_refs: Option<&str>,
    ) -> Result<LegalChatMessage, DatabaseError>;
}

#[cfg(feature = "libsql")]
mod libsql_impl {
    use std::sync::Arc;

    use async_trait::async_trait;
    use libsql::{Database as LibSqlDatabase, params};

    use super::{
        LegalChat, LegalChatMessage, LegalDocumentText, LegalProjectMeta, LegalRole, LegalStore,
    };
    use crate::error::DatabaseError;

    /// libSQL-backed [`LegalStore`].
    ///
    /// Constructed from the same shared `LibSqlDatabase` handle the rest
    /// of ironclaw threads through `LibSqlBackend`. Each method opens a
    /// fresh connection rather than holding one — matches the pattern
    /// used by `secrets`/`wasm` stores in the same crate.
    pub struct LibSqlLegalStore {
        db: Arc<LibSqlDatabase>,
    }

    impl LibSqlLegalStore {
        pub fn new(db: Arc<LibSqlDatabase>) -> Self {
            Self { db }
        }

        async fn connect(&self) -> Result<libsql::Connection, DatabaseError> {
            self.db
                .connect()
                .map_err(|e| DatabaseError::Pool(format!("legal store connect: {e}")))
        }
    }

    fn map_row_err(context: &str) -> impl Fn(libsql::Error) -> DatabaseError + '_ {
        move |e| DatabaseError::Query(format!("{context}: {e}"))
    }

    fn now_epoch() -> i64 {
        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(d) => d.as_secs() as i64,
            Err(_) => 0,
        }
    }

    fn new_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    #[async_trait]
    impl LegalStore for LibSqlLegalStore {
        async fn project_meta(
            &self,
            project_id: &str,
        ) -> Result<Option<LegalProjectMeta>, DatabaseError> {
            let conn = self.connect().await?;
            let mut rows = conn
                .query(
                    "SELECT id, name, deleted_at, created_at \
                     FROM legal_projects WHERE id = ?1",
                    params![project_id.to_string()],
                )
                .await
                .map_err(map_row_err("legal_projects select"))?;

            let Some(row) = rows
                .next()
                .await
                .map_err(map_row_err("legal_projects next"))?
            else {
                return Ok(None);
            };

            let id: String = row
                .get::<String>(0)
                .map_err(map_row_err("legal_projects.id"))?;
            let name: String = row
                .get::<String>(1)
                .map_err(map_row_err("legal_projects.name"))?;
            let deleted_at: Option<i64> = row
                .get::<Option<i64>>(2)
                .map_err(map_row_err("legal_projects.deleted_at"))?;
            let created_at: i64 = row
                .get::<i64>(3)
                .map_err(map_row_err("legal_projects.created_at"))?;

            Ok(Some(LegalProjectMeta {
                id,
                name,
                deleted_at,
                created_at,
            }))
        }

        async fn project_document_texts(
            &self,
            project_id: &str,
        ) -> Result<Vec<LegalDocumentText>, DatabaseError> {
            let conn = self.connect().await?;
            let mut rows = conn
                .query(
                    "SELECT id, filename, extracted_text \
                     FROM legal_documents \
                     WHERE project_id = ?1 \
                     ORDER BY uploaded_at ASC, id ASC",
                    params![project_id.to_string()],
                )
                .await
                .map_err(map_row_err("legal_documents select"))?;

            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(map_row_err("legal_documents next"))?
            {
                let id: String = row.get::<String>(0).map_err(map_row_err("documents.id"))?;
                let filename: String = row
                    .get::<String>(1)
                    .map_err(map_row_err("documents.filename"))?;
                let extracted_text: Option<String> = row
                    .get::<Option<String>>(2)
                    .map_err(map_row_err("documents.extracted_text"))?;
                out.push(LegalDocumentText {
                    id,
                    filename,
                    extracted_text,
                });
            }
            Ok(out)
        }

        async fn create_chat(
            &self,
            project_id: &str,
            title: Option<&str>,
        ) -> Result<LegalChat, DatabaseError> {
            let id = new_id();
            let created_at = now_epoch();
            let title_owned: Option<String> = title.map(|s| s.to_string());
            let conn = self.connect().await?;
            conn.execute(
                "INSERT INTO legal_chats (id, project_id, title, created_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    id.clone(),
                    project_id.to_string(),
                    title_owned.clone(),
                    created_at,
                ],
            )
            .await
            .map_err(map_row_err("legal_chats insert"))?;
            Ok(LegalChat {
                id,
                project_id: project_id.to_string(),
                title: title_owned,
                created_at,
            })
        }

        async fn get_chat(&self, chat_id: &str) -> Result<Option<LegalChat>, DatabaseError> {
            let conn = self.connect().await?;
            let mut rows = conn
                .query(
                    "SELECT id, project_id, title, created_at \
                     FROM legal_chats WHERE id = ?1",
                    params![chat_id.to_string()],
                )
                .await
                .map_err(map_row_err("legal_chats select"))?;
            let Some(row) = rows.next().await.map_err(map_row_err("legal_chats next"))? else {
                return Ok(None);
            };
            Ok(Some(row_to_chat(&row)?))
        }

        async fn list_chats_for_project(
            &self,
            project_id: &str,
        ) -> Result<Vec<LegalChat>, DatabaseError> {
            let conn = self.connect().await?;
            let mut rows = conn
                .query(
                    "SELECT id, project_id, title, created_at \
                     FROM legal_chats \
                     WHERE project_id = ?1 \
                     ORDER BY created_at ASC, id ASC",
                    params![project_id.to_string()],
                )
                .await
                .map_err(map_row_err("legal_chats list"))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(map_row_err("legal_chats list next"))?
            {
                out.push(row_to_chat(&row)?);
            }
            Ok(out)
        }

        async fn list_messages_for_chat(
            &self,
            chat_id: &str,
        ) -> Result<Vec<LegalChatMessage>, DatabaseError> {
            let conn = self.connect().await?;
            let mut rows = conn
                .query(
                    "SELECT id, chat_id, role, content, document_refs, created_at \
                     FROM legal_chat_messages \
                     WHERE chat_id = ?1 \
                     ORDER BY created_at ASC, id ASC",
                    params![chat_id.to_string()],
                )
                .await
                .map_err(map_row_err("legal_chat_messages list"))?;
            let mut out = Vec::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(map_row_err("legal_chat_messages list next"))?
            {
                out.push(row_to_message(&row)?);
            }
            Ok(out)
        }

        async fn append_message(
            &self,
            chat_id: &str,
            role: LegalRole,
            content: &str,
            document_refs: Option<&str>,
        ) -> Result<LegalChatMessage, DatabaseError> {
            let id = new_id();
            let created_at = now_epoch();
            let role_str = role.as_str().to_string();
            let conn = self.connect().await?;
            let document_refs_owned: Option<String> = document_refs.map(|s| s.to_string());
            conn.execute(
                "INSERT INTO legal_chat_messages \
                    (id, chat_id, role, content, document_refs, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id.clone(),
                    chat_id.to_string(),
                    role_str,
                    content.to_string(),
                    document_refs_owned.clone(),
                    created_at,
                ],
            )
            .await
            .map_err(map_row_err("legal_chat_messages insert"))?;
            Ok(LegalChatMessage {
                id,
                chat_id: chat_id.to_string(),
                role,
                content: content.to_string(),
                document_refs: document_refs_owned,
                created_at,
            })
        }
    }

    fn row_to_chat(row: &libsql::Row) -> Result<LegalChat, DatabaseError> {
        let id: String = row.get::<String>(0).map_err(map_row_err("chats.id"))?;
        let project_id: String = row
            .get::<String>(1)
            .map_err(map_row_err("chats.project_id"))?;
        let title: Option<String> = row
            .get::<Option<String>>(2)
            .map_err(map_row_err("chats.title"))?;
        let created_at: i64 = row.get::<i64>(3).map_err(map_row_err("chats.created_at"))?;
        Ok(LegalChat {
            id,
            project_id,
            title,
            created_at,
        })
    }

    fn row_to_message(row: &libsql::Row) -> Result<LegalChatMessage, DatabaseError> {
        let id: String = row.get::<String>(0).map_err(map_row_err("messages.id"))?;
        let chat_id: String = row
            .get::<String>(1)
            .map_err(map_row_err("messages.chat_id"))?;
        let role_raw: String = row.get::<String>(2).map_err(map_row_err("messages.role"))?;
        let role = LegalRole::parse(&role_raw)?;
        let content: String = row
            .get::<String>(3)
            .map_err(map_row_err("messages.content"))?;
        let document_refs: Option<String> = row
            .get::<Option<String>>(4)
            .map_err(map_row_err("messages.document_refs"))?;
        let created_at: i64 = row
            .get::<i64>(5)
            .map_err(map_row_err("messages.created_at"))?;
        Ok(LegalChatMessage {
            id,
            chat_id,
            role,
            content,
            document_refs,
            created_at,
        })
    }
}

#[cfg(feature = "libsql")]
pub use libsql_impl::LibSqlLegalStore;

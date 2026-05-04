//! libSQL implementation of `XBookmarkStore`.
//!
//! Each operation opens a fresh connection per the libSQL backend pattern.
//! Inserts use `INSERT OR IGNORE` for per-`(user_id, tweet_id)` dedupe and
//! batch all rows into a single transaction so a 500-item ingest is one
//! commit, not 500.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::Utc;
use libsql::params;
use uuid::Uuid;

use super::{LibSqlBackend, fmt_opt_ts, fmt_ts, get_i64, get_opt_text, get_text, get_ts};
use crate::db::{ResolvedTriageDecision, XBookmarkStore};
use crate::error::DatabaseError;
use crate::x_bookmarks::{Bookmark, BookmarkStatus, NormalizedIngestItem};

#[async_trait]
impl XBookmarkStore for LibSqlBackend {
    async fn insert_x_bookmarks(
        &self,
        user_id: &str,
        items: &[NormalizedIngestItem],
    ) -> Result<(u64, u64), DatabaseError> {
        if items.is_empty() {
            return Ok((0, 0));
        }
        let conn = self.connect().await?;
        let tx = conn
            .transaction()
            .await
            .map_err(|e| DatabaseError::Query(format!("begin: {e}")))?;

        let now = fmt_ts(&Utc::now());
        let mut inserted: u64 = 0;
        for item in items {
            let id = Uuid::new_v4().to_string();
            let media_urls = serde_json::to_string(&item.media_urls)
                .map_err(|e| DatabaseError::Query(format!("media_urls json: {e}")))?;
            let posted_at = fmt_opt_ts(&item.posted_at);
            // execute() returns rows-changed; INSERT OR IGNORE returns 0 on
            // conflict, so this is the canonical way to count dedupes.
            let changed = tx
                .execute(
                    r#"
INSERT OR IGNORE INTO x_bookmarks (
    id, user_id, tweet_id, author_handle, author_name,
    text, url, media_urls, quoted_tweet, thread_id,
    posted_at, scraped_at, status, tags
) VALUES (?1, ?2, ?3, ?4, ?5,
          ?6, ?7, ?8, ?9, ?10,
          ?11, ?12, 'untriaged', '[]')
"#,
                    params![
                        id,
                        user_id,
                        item.tweet_id.as_str(),
                        item.author_handle.as_deref(),
                        item.author_name.as_deref(),
                        item.text.as_str(),
                        item.url.as_str(),
                        media_urls,
                        item.quoted_tweet.as_deref(),
                        item.thread_id.as_deref(),
                        posted_at,
                        now.as_str(),
                    ],
                )
                .await
                .map_err(|e| DatabaseError::Query(format!("insert x_bookmark: {e}")))?;
            inserted += changed;
        }

        tx.commit()
            .await
            .map_err(|e| DatabaseError::Query(format!("commit: {e}")))?;

        let total = items.len() as u64;
        Ok((inserted, total - inserted))
    }

    async fn list_untriaged_x_bookmarks(
        &self,
        user_id: &str,
        limit: u32,
    ) -> Result<Vec<Bookmark>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
SELECT id, user_id, tweet_id, author_handle, author_name, text, url,
       media_urls, quoted_tweet, thread_id, posted_at, scraped_at,
       status, rationale, project_slug, tags, triaged_at, triage_model
FROM x_bookmarks
WHERE user_id = ?1 AND status = 'untriaged'
ORDER BY COALESCE(posted_at, scraped_at) DESC
LIMIT ?2
"#,
                params![user_id, limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            out.push(row_to_bookmark(&row)?);
        }
        Ok(out)
    }

    async fn apply_x_bookmark_triage(
        &self,
        user_id: &str,
        decisions: &[(Uuid, ResolvedTriageDecision)],
        triage_model: &str,
    ) -> Result<u64, DatabaseError> {
        if decisions.is_empty() {
            return Ok(0);
        }
        let conn = self.connect().await?;
        let tx = conn
            .transaction()
            .await
            .map_err(|e| DatabaseError::Query(format!("begin: {e}")))?;
        let now = fmt_ts(&Utc::now());

        let mut updated: u64 = 0;
        for (id, decision) in decisions {
            let tags = serde_json::to_string(&decision.tags)
                .map_err(|e| DatabaseError::Query(format!("tags json: {e}")))?;
            let id_str = id.to_string();
            let changed = tx
                .execute(
                    // Codex review fix: only overwrite rows that are still
                    // `untriaged`. If a concurrent triage request already
                    // committed, the second writer affects zero rows and
                    // the existing decision is preserved.
                    r#"
UPDATE x_bookmarks
SET status        = ?1,
    rationale     = ?2,
    project_slug  = ?3,
    tags          = ?4,
    triaged_at    = ?5,
    triage_model  = ?6
WHERE id = ?7 AND user_id = ?8 AND status = 'untriaged'
"#,
                    params![
                        decision.status.as_str(),
                        decision.rationale.as_deref(),
                        decision.project_slug.as_deref(),
                        tags,
                        now.as_str(),
                        triage_model,
                        id_str,
                        user_id,
                    ],
                )
                .await
                .map_err(|e| DatabaseError::Query(format!("update triage: {e}")))?;
            updated += changed;
        }

        tx.commit()
            .await
            .map_err(|e| DatabaseError::Query(format!("commit: {e}")))?;
        Ok(updated)
    }

    async fn list_x_bookmarks_by_status(
        &self,
        user_id: &str,
        status: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Bookmark>, DatabaseError> {
        let conn = self.connect().await?;
        // Validate status against the canonical vocabulary so a caller cannot
        // pull rows with arbitrary status filtering past the API layer.
        let normalized_status = status
            .map(|s| s.to_ascii_lowercase())
            .filter(|s| BookmarkStatus::parse(s).is_some());

        let mut rows = if let Some(s) = normalized_status.as_deref() {
            conn.query(
                r#"
SELECT id, user_id, tweet_id, author_handle, author_name, text, url,
       media_urls, quoted_tweet, thread_id, posted_at, scraped_at,
       status, rationale, project_slug, tags, triaged_at, triage_model
FROM x_bookmarks
WHERE user_id = ?1 AND status = ?2
ORDER BY COALESCE(posted_at, scraped_at) DESC
LIMIT ?3
"#,
                params![user_id, s, limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        } else {
            conn.query(
                r#"
SELECT id, user_id, tweet_id, author_handle, author_name, text, url,
       media_urls, quoted_tweet, thread_id, posted_at, scraped_at,
       status, rationale, project_slug, tags, triaged_at, triage_model
FROM x_bookmarks
WHERE user_id = ?1
ORDER BY COALESCE(posted_at, scraped_at) DESC
LIMIT ?2
"#,
                params![user_id, limit as i64],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        };

        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            out.push(row_to_bookmark(&row)?);
        }
        Ok(out)
    }

    async fn x_bookmark_counts_by_status(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, u64>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
SELECT status, COUNT(*) AS cnt
FROM x_bookmarks
WHERE user_id = ?1
GROUP BY status
"#,
                params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut map = HashMap::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let status = get_text(&row, 0);
            let count = get_i64(&row, 1);
            map.insert(status, count.max(0) as u64);
        }
        Ok(map)
    }
}

fn row_to_bookmark(row: &libsql::Row) -> Result<Bookmark, DatabaseError> {
    let id_str = get_text(row, 0);
    let id = Uuid::parse_str(&id_str)
        .map_err(|e| DatabaseError::Query(format!("invalid x_bookmark id {id_str:?}: {e}")))?;
    let media_urls_raw = get_text(row, 7);
    let media_urls: Vec<String> = serde_json::from_str(&media_urls_raw).unwrap_or_default();
    let tags_raw = get_text(row, 15);
    let tags: Vec<String> = serde_json::from_str(&tags_raw).unwrap_or_default();
    let status_raw = get_text(row, 12);
    let status = BookmarkStatus::parse(&status_raw).unwrap_or(BookmarkStatus::Untriaged);
    Ok(Bookmark {
        id,
        user_id: get_text(row, 1),
        tweet_id: get_text(row, 2),
        author_handle: get_opt_text(row, 3),
        author_name: get_opt_text(row, 4),
        text: get_text(row, 5),
        url: get_opt_text(row, 6),
        media_urls,
        quoted_tweet: get_opt_text(row, 8),
        thread_id: get_opt_text(row, 9),
        posted_at: row.get::<String>(10).ok().and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        }),
        scraped_at: get_ts(row, 11),
        status,
        rationale: get_opt_text(row, 13),
        project_slug: get_opt_text(row, 14),
        tags,
        triaged_at: row.get::<String>(16).ok().and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .ok()
                .map(|dt| dt.with_timezone(&Utc))
        }),
        triage_model: get_opt_text(row, 17),
    })
}

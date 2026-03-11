//! FactStore implementation for LibSqlBackend.
//!
//! Provides structured fact storage with hybrid FTS5 + vector search,
//! following the same patterns as workspace.rs.

use async_trait::async_trait;
use libsql::params;
use uuid::Uuid;

use super::{LibSqlBackend, get_opt_text, get_opt_ts, get_text, get_ts};
use crate::db::{ExtractionLogEntry, Fact, FactSearchResult, FactStore};
use crate::error::WorkspaceError;

/// Parse a Fact from a row with columns:
/// id, user_id, agent_id, content, category, confidence,
/// source_session_id, created_at, updated_at, expires_at, metadata
fn row_to_fact(row: &libsql::Row) -> Fact {
    Fact {
        id: get_text(row, 0).parse().unwrap_or_default(),
        user_id: get_text(row, 1),
        agent_id: get_opt_text(row, 2).and_then(|s| s.parse().ok()),
        content: get_text(row, 3),
        category: get_text(row, 4),
        confidence: row.get::<f64>(5).unwrap_or(1.0) as f32,
        source_session_id: get_opt_text(row, 6),
        created_at: get_ts(row, 7),
        updated_at: get_ts(row, 8),
        expires_at: get_opt_ts(row, 9),
        metadata: get_opt_text(row, 10)
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
    }
}

#[async_trait]
impl FactStore for LibSqlBackend {
    async fn upsert_fact(
        &self,
        id: Uuid,
        user_id: &str,
        agent_id: Option<Uuid>,
        content: &str,
        category: &str,
        confidence: f32,
        source_session_id: Option<&str>,
        embedding: Option<&[f32]>,
        metadata: Option<&str>,
    ) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;

        let agent_id_str = agent_id.map(|id| id.to_string());
        let embedding_blob = embedding.map(|e| {
            let bytes: Vec<u8> = e.iter().flat_map(|f| f.to_le_bytes()).collect();
            bytes
        });

        conn.execute(
            r#"
            INSERT INTO memory_facts (id, user_id, agent_id, content, category,
                                      confidence, source_session_id, embedding, metadata)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(id) DO UPDATE SET
                content = excluded.content,
                category = excluded.category,
                confidence = excluded.confidence,
                embedding = excluded.embedding,
                metadata = excluded.metadata,
                updated_at = datetime('now')
            "#,
            params![
                id.to_string(),
                user_id,
                agent_id_str.as_deref(),
                content,
                category,
                confidence as f64,
                source_session_id,
                embedding_blob.map(libsql::Value::Blob),
                metadata.unwrap_or("{}"),
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Upsert fact failed: {}", e),
        })?;

        Ok(())
    }

    async fn search_facts(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        limit: usize,
    ) -> Result<Vec<FactSearchResult>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let pre_limit = 50i64;

        // Phase 1: FTS search
        let mut fts_ids: Vec<(String, u32)> = Vec::new();
        {
            let mut rows = conn
                .query(
                    r#"
                    SELECT f.id
                    FROM memory_facts_fts fts
                    JOIN memory_facts f ON f._rowid = fts.rowid
                    WHERE f.user_id = ?1 AND f.agent_id IS ?2
                      AND memory_facts_fts MATCH ?3
                    ORDER BY rank
                    LIMIT ?4
                    "#,
                    params![user_id, agent_id_str.as_deref(), query, pre_limit],
                )
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("FTS fact query failed: {}", e),
                })?;

            let mut rank = 1u32;
            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("FTS row fetch failed: {}", e),
                })?
            {
                fts_ids.push((get_text(&row, 0), rank));
                rank += 1;
            }
        }

        // Phase 2: Vector search
        let mut vec_ids: Vec<(String, u32)> = Vec::new();
        if let Some(emb) = embedding {
            let vector_json = format!(
                "[{}]",
                emb.iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );

            let mut rows = conn
                .query(
                    r#"
                    SELECT f.id
                    FROM vector_top_k('idx_memory_facts_embedding', vector(?1), ?2) AS top_k
                    JOIN memory_facts f ON f._rowid = top_k.id
                    WHERE f.user_id = ?3 AND f.agent_id IS ?4
                    "#,
                    params![vector_json, pre_limit, user_id, agent_id_str.as_deref()],
                )
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("Vector fact query failed: {}", e),
                })?;

            let mut rank = 1u32;
            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("Vector row fetch failed: {}", e),
                })?
            {
                vec_ids.push((get_text(&row, 0), rank));
                rank += 1;
            }
        }

        // Phase 3: RRF fusion
        let k = 60u32;
        let mut scores: std::collections::HashMap<String, f32> = std::collections::HashMap::new();

        for (id, rank) in &fts_ids {
            *scores.entry(id.clone()).or_default() += 1.0 / (k + rank) as f32;
        }
        for (id, rank) in &vec_ids {
            *scores.entry(id.clone()).or_default() += 1.0 / (k + rank) as f32;
        }

        // Sort by score descending
        let mut scored: Vec<(String, f32)> = scores.into_iter().collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        if scored.is_empty() {
            return Ok(Vec::new());
        }

        // Normalize scores
        let max_score = scored.first().map(|(_, s)| *s).unwrap_or(1.0);
        if max_score > 0.0 {
            for (_, s) in &mut scored {
                *s /= max_score;
            }
        }

        // Phase 4: Fetch full facts for top results
        let mut results = Vec::with_capacity(scored.len());
        for (fact_id, score) in &scored {
            let mut rows = conn
                .query(
                    r#"
                    SELECT id, user_id, agent_id, content, category, confidence,
                           source_session_id, created_at, updated_at, expires_at, metadata
                    FROM memory_facts
                    WHERE id = ?1
                    "#,
                    params![fact_id.as_str()],
                )
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("Fact fetch failed: {}", e),
                })?;

            if let Some(row) = rows
                .next()
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("Fact row fetch failed: {}", e),
                })?
            {
                results.push(FactSearchResult {
                    fact: row_to_fact(&row),
                    score: *score,
                });
            }
        }

        Ok(results)
    }

    async fn get_facts_by_category(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        category: &str,
        limit: usize,
    ) -> Result<Vec<Fact>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());

        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, content, category, confidence,
                       source_session_id, created_at, updated_at, expires_at, metadata
                FROM memory_facts
                WHERE user_id = ?1 AND agent_id IS ?2 AND category = ?3
                ORDER BY updated_at DESC
                LIMIT ?4
                "#,
                params![user_id, agent_id_str.as_deref(), category, limit as i64],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Category query failed: {}", e),
            })?;

        let mut facts = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Row fetch failed: {}", e),
            })?
        {
            facts.push(row_to_fact(&row));
        }
        Ok(facts)
    }

    async fn get_recent_facts(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<Fact>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());

        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, content, category, confidence,
                       source_session_id, created_at, updated_at, expires_at, metadata
                FROM memory_facts
                WHERE user_id = ?1 AND agent_id IS ?2
                  AND (expires_at IS NULL OR expires_at > datetime('now'))
                ORDER BY updated_at DESC
                LIMIT ?3
                "#,
                params![user_id, agent_id_str.as_deref(), limit as i64],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Recent facts query failed: {}", e),
            })?;

        let mut facts = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Row fetch failed: {}", e),
            })?
        {
            facts.push(row_to_fact(&row));
        }
        Ok(facts)
    }

    async fn get_fact(&self, id: Uuid) -> Result<Option<Fact>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;

        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, content, category, confidence,
                       source_session_id, created_at, updated_at, expires_at, metadata
                FROM memory_facts
                WHERE id = ?1
                "#,
                params![id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Get fact failed: {}", e),
            })?;

        match rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Row fetch failed: {}", e),
            })? {
            Some(row) => Ok(Some(row_to_fact(&row))),
            None => Ok(None),
        }
    }

    async fn update_fact(
        &self,
        id: Uuid,
        content: &str,
        confidence: f32,
    ) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;

        conn.execute(
            r#"
            UPDATE memory_facts
            SET content = ?2, confidence = ?3, updated_at = datetime('now')
            WHERE id = ?1
            "#,
            params![id.to_string(), content, confidence as f64],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Update fact failed: {}", e),
        })?;

        Ok(())
    }

    async fn delete_fact(&self, id: Uuid) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;

        conn.execute(
            "DELETE FROM memory_facts WHERE id = ?1",
            params![id.to_string()],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Delete fact failed: {}", e),
        })?;

        Ok(())
    }

    async fn delete_expired_facts(&self) -> Result<u64, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;

        let count = conn
            .execute(
                "DELETE FROM memory_facts WHERE expires_at IS NOT NULL AND expires_at < datetime('now')",
                (),
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Delete expired failed: {}", e),
            })?;

        Ok(count)
    }

    async fn log_extraction(
        &self,
        entry: &ExtractionLogEntry,
    ) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;

        let agent_id_str = entry.agent_id.map(|id| id.to_string());

        conn.execute(
            r#"
            INSERT INTO memory_extraction_log
                (id, session_id, user_id, agent_id, facts_added, facts_updated,
                 facts_skipped, duration_ms, model_used)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                entry.id.to_string(),
                entry.session_id.as_str(),
                entry.user_id.as_str(),
                agent_id_str.as_deref(),
                entry.facts_added as i64,
                entry.facts_updated as i64,
                entry.facts_skipped as i64,
                entry.duration_ms,
                entry.model_used.as_deref(),
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Log extraction failed: {}", e),
        })?;

        Ok(())
    }

    async fn find_similar_facts(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        embedding: &[f32],
        _threshold: f32,
        limit: usize,
    ) -> Result<Vec<FactSearchResult>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());

        let vector_json = format!(
            "[{}]",
            embedding
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );

        // Use vector_top_k to find similar facts; distance is returned
        let mut rows = conn
            .query(
                r#"
                SELECT f.id, f.user_id, f.agent_id, f.content, f.category, f.confidence,
                       f.source_session_id, f.created_at, f.updated_at, f.expires_at, f.metadata,
                       top_k.distance
                FROM vector_top_k('idx_memory_facts_embedding', vector(?1), ?2) AS top_k
                JOIN memory_facts f ON f._rowid = top_k.id
                WHERE f.user_id = ?3 AND f.agent_id IS ?4
                "#,
                params![vector_json, limit as i64, user_id, agent_id_str.as_deref()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Similar facts query failed: {}", e),
            })?;

        let mut results = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Row fetch failed: {}", e),
            })?
        {
            let distance: f64 = row.get::<f64>(11).unwrap_or(f64::MAX);
            // Convert distance to similarity (cosine distance: lower = more similar)
            // libsql vector_top_k returns cosine distance, similarity = 1 - distance
            let similarity = 1.0 - distance as f32;

            if similarity >= _threshold {
                results.push(FactSearchResult {
                    fact: row_to_fact(&row),
                    score: similarity,
                });
            }
        }

        Ok(results)
    }

    async fn get_recent_transcript_for_user(
        &self,
        user_id: &str,
        max_messages: usize,
    ) -> Result<Vec<(String, String)>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;

        // Find the most recently active conversation for this user, then get
        // the latest messages from it. We use a subquery to avoid scanning all
        // conversations — just pick the one updated most recently.
        let mut rows = conn
            .query(
                r#"
                SELECT cm.role, cm.content
                FROM conversation_messages cm
                WHERE cm.conversation_id = (
                    SELECT c.id
                    FROM conversations c
                    WHERE c.user_id = ?1
                    ORDER BY c.updated_at DESC
                    LIMIT 1
                )
                ORDER BY cm.created_at DESC
                LIMIT ?2
                "#,
                params![user_id, max_messages as i64],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Transcript query failed: {}", e),
            })?;

        let mut messages = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Row fetch failed: {}", e),
            })?
        {
            let role: String = row.get::<String>(0).unwrap_or_default();
            let content: String = row.get::<String>(1).unwrap_or_default();
            messages.push((role, content));
        }

        // Reverse to get chronological order (query returned DESC)
        messages.reverse();
        Ok(messages)
    }
}

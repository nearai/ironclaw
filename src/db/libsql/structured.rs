//! LibSQL implementation of StructuredStore.
//!
//! Ports the PostgreSQL structured collections logic to SQLite dialect.
//! Uses TEXT columns for JSON (instead of JSONB) and `json_extract()` for
//! field access (instead of `->>` / `jsonb_extract_path_text()`).

use async_trait::async_trait;
use uuid::Uuid;

use crate::db::structured::{self, Aggregation, CollectionSchema, Filter, Record, StructuredStore};
use crate::error::DatabaseError;

use super::{LibSqlBackend, fmt_ts, get_json, get_text, get_ts};

// ==================== Helpers ====================

/// Parse a structured record from a libsql Row.
///
/// Expected column order: id(0), user_id(1), collection(2), data(3),
/// created_at(4), updated_at(5).
fn row_to_record(row: &libsql::Row) -> Result<Record, DatabaseError> {
    let id_str = get_text(row, 0);
    let id: Uuid = id_str
        .parse()
        .map_err(|e| DatabaseError::Serialization(format!("invalid UUID: {e}")))?;
    let user_id = get_text(row, 1);
    let collection = get_text(row, 2);
    let data = get_json(row, 3);
    let created_at = get_ts(row, 4);
    let updated_at = get_ts(row, 5);

    Ok(Record {
        id,
        user_id,
        collection,
        data,
        created_at,
        updated_at,
    })
}

/// Resolve a filter field name to its SQLite expression.
///
/// Special fields:
/// - `created_at` / `updated_at` -> use the DB column directly
/// - Dot-notation (e.g. `_lineage.source`) -> nested JSON access
///   (`json_extract(data, '$._lineage.source')`)
/// - Everything else -> `json_extract(data, '$.field')`
fn resolve_filter_field(field: &str) -> Result<String, DatabaseError> {
    // DB column fields.
    if field == "created_at" || field == "updated_at" {
        return Ok(field.to_string());
    }

    // Dot-notation for nested JSON: `parent.child` -> `json_extract(data, '$.parent.child')`.
    if let Some((parent, child)) = field.split_once('.') {
        validate_filter_field_segment(parent)?;
        validate_filter_field_segment(child)?;
        return Ok(format!("json_extract(data, '$.{parent}.{child}')"));
    }

    // Regular data field.
    structured::validate_field_name(field).map_err(|e| DatabaseError::Query(e.to_string()))?;
    Ok(format!("json_extract(data, '$.{field}')"))
}

/// Validate a single segment of a filter field name (for dot-notation).
///
/// Allows system-field prefixes (starting with `_`) in addition to regular identifiers.
fn validate_filter_field_segment(name: &str) -> Result<(), DatabaseError> {
    if name.is_empty() || name.len() > 64 {
        return Err(DatabaseError::Query(format!(
            "filter field segment '{name}' must be 1-64 characters"
        )));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(DatabaseError::Query(format!(
            "filter field segment '{name}' contains invalid characters"
        )));
    }
    Ok(())
}

/// Convert a JSON filter value to the appropriate `libsql::Value`.
///
/// SQLite's `json_extract()` returns native types (integer for bools/ints,
/// real for floats, text for strings). Filter parameters must use matching
/// types for correct comparison — unlike PostgreSQL's `->>` which always
/// returns text.
fn json_to_libsql_value(value: &serde_json::Value) -> libsql::Value {
    match value {
        serde_json::Value::Bool(b) => {
            // json_extract returns 1/0 for booleans in SQLite
            libsql::Value::Integer(if *b { 1 } else { 0 })
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                libsql::Value::Integer(i)
            } else if let Some(f) = n.as_f64() {
                libsql::Value::Real(f)
            } else {
                libsql::Value::Text(n.to_string())
            }
        }
        serde_json::Value::String(s) => libsql::Value::Text(s.clone()),
        serde_json::Value::Null => libsql::Value::Null,
        // Arrays and objects: serialize to text for comparison
        other => libsql::Value::Text(other.to_string()),
    }
}

/// Build filter WHERE clauses and collect parameters for a set of filters.
///
/// Returns (where_clauses, params) where where_clauses is a Vec of SQL fragments
/// and params is the collected parameter values. Uses `?N` placeholders.
fn build_filters(
    filters: &[Filter],
    start_idx: i32,
) -> Result<(Vec<String>, Vec<libsql::Value>), DatabaseError> {
    let mut clauses = Vec::new();
    let mut params: Vec<libsql::Value> = Vec::new();
    let mut idx = start_idx;

    for filter in filters {
        let sql_field = resolve_filter_field(&filter.field)?;

        let make_compare =
            |op: &str, idx: &mut i32| -> (String, Vec<libsql::Value>) {
                let val = json_to_libsql_value(&filter.value);
                let clause = format!("{sql_field} {op} ?{}", *idx);
                *idx += 1;
                (clause, vec![val])
            };

        match filter.op {
            structured::FilterOp::IsNull => {
                clauses.push(format!("{sql_field} IS NULL"));
            }
            structured::FilterOp::IsNotNull => {
                clauses.push(format!("{sql_field} IS NOT NULL"));
            }
            structured::FilterOp::Eq => {
                let (clause, p) = make_compare("=", &mut idx);
                clauses.push(clause);
                params.extend(p);
            }
            structured::FilterOp::Neq => {
                let (clause, p) = make_compare("!=", &mut idx);
                clauses.push(clause);
                params.extend(p);
            }
            structured::FilterOp::Gt => {
                let (clause, p) = make_compare(">", &mut idx);
                clauses.push(clause);
                params.extend(p);
            }
            structured::FilterOp::Gte => {
                let (clause, p) = make_compare(">=", &mut idx);
                clauses.push(clause);
                params.extend(p);
            }
            structured::FilterOp::Lt => {
                let (clause, p) = make_compare("<", &mut idx);
                clauses.push(clause);
                params.extend(p);
            }
            structured::FilterOp::Lte => {
                let (clause, p) = make_compare("<=", &mut idx);
                clauses.push(clause);
                params.extend(p);
            }
            structured::FilterOp::Between => {
                let arr = filter.value.as_array().ok_or_else(|| {
                    DatabaseError::Query("Between filter requires an array of [lo, hi]".to_string())
                })?;
                if arr.len() != 2 {
                    return Err(DatabaseError::Query(
                        "Between filter requires exactly 2 elements".to_string(),
                    ));
                }
                clauses.push(format!("{sql_field} BETWEEN ?{idx} AND ?{}", idx + 1));
                params.push(json_to_libsql_value(&arr[0]));
                params.push(json_to_libsql_value(&arr[1]));
                idx += 2;
            }
            structured::FilterOp::In => {
                let arr = filter.value.as_array().ok_or_else(|| {
                    DatabaseError::Query("In filter requires an array value".to_string())
                })?;
                if arr.is_empty() {
                    clauses.push("0".to_string()); // FALSE equivalent in SQLite
                } else {
                    let placeholders: Vec<String> = arr
                        .iter()
                        .enumerate()
                        .map(|(i, _)| format!("?{}", idx + i as i32))
                        .collect();
                    clauses.push(format!("{sql_field} IN ({})", placeholders.join(", ")));
                    for item in arr {
                        params.push(json_to_libsql_value(item));
                    }
                    idx += arr.len() as i32;
                }
            }
        }
    }

    Ok((clauses, params))
}

// ==================== Rust-side aggregation for SUM/AVG ====================
//
// libSQL SDK has a bug where `row.get::<f64>()` on a CAST result returns 0
// for integer JSON values.  We work around this by fetching raw records and
// computing SUM/AVG in Rust.

async fn aggregate_in_rust(
    backend: &LibSqlBackend,
    user_id: &str,
    collection: &str,
    aggregation: &Aggregation,
) -> Result<serde_json::Value, DatabaseError> {
    let field = aggregation
        .field
        .as_deref()
        .ok_or_else(|| DatabaseError::Query("SUM/AVG requires a field".to_string()))?;

    // Fetch all matching records (applying filters).
    let records = backend
        .query_records(user_id, collection, &aggregation.filters, None, usize::MAX)
        .await?;

    let group_by = aggregation.group_by.as_deref();

    if let Some(group_field) = group_by {
        // Grouped aggregation: { "group_key": value, ... }
        let mut groups: std::collections::BTreeMap<String, (f64, usize)> =
            std::collections::BTreeMap::new();
        for record in &records {
            let group_key = record
                .data
                .get(group_field)
                .and_then(|v| match v {
                    serde_json::Value::String(s) => Some(s.clone()),
                    other => Some(other.to_string()),
                })
                .unwrap_or_default();
            if let Some(val) = record.data.get(field).and_then(json_to_f64) {
                let entry = groups.entry(group_key).or_insert((0.0, 0));
                entry.0 += val;
                entry.1 += 1;
            }
        }
        let mut result = serde_json::Map::new();
        for (key, (sum, count)) in &groups {
            let value = match aggregation.operation {
                structured::AggOp::Sum => *sum,
                structured::AggOp::Avg => {
                    if *count > 0 {
                        sum / *count as f64
                    } else {
                        0.0
                    }
                }
                _ => unreachable!(),
            };
            result.insert(key.clone(), serde_json::json!(value));
        }
        Ok(serde_json::Value::Object(result))
    } else {
        // Non-grouped: return bare value (matching SQL-path format).
        let mut sum = 0.0;
        let mut count = 0usize;
        for record in &records {
            if let Some(val) = record.data.get(field).and_then(json_to_f64) {
                sum += val;
                count += 1;
            }
        }
        match aggregation.operation {
            structured::AggOp::Sum => Ok(serde_json::json!(sum)),
            structured::AggOp::Avg => {
                if count > 0 {
                    Ok(serde_json::json!(sum / count as f64))
                } else {
                    Ok(serde_json::Value::Null)
                }
            }
            _ => unreachable!(),
        }
    }
}

/// Extract a numeric value from a JSON value.
fn json_to_f64(v: &serde_json::Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_i64().map(|i| i as f64))
}

// ==================== StructuredStore Implementation ====================

#[async_trait]
impl StructuredStore for LibSqlBackend {
    async fn register_collection(
        &self,
        user_id: &str,
        schema: &CollectionSchema,
    ) -> Result<(), DatabaseError> {
        CollectionSchema::validate_name(&schema.collection)
            .map_err(|e| DatabaseError::Query(e.to_string()))?;
        schema
            .validate_source_scope()
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let schema_json = serde_json::to_string(schema)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        let description = schema
            .description
            .as_deref()
            .map(|s| libsql::Value::Text(s.to_string()))
            .unwrap_or(libsql::Value::Null);

        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO structured_schemas (user_id, collection, schema, description)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT (user_id, collection) DO UPDATE SET
                schema = excluded.schema,
                description = excluded.description
            "#,
            libsql::params![user_id, schema.collection.as_str(), schema_json, description],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("register_collection: {e}")))?;

        Ok(())
    }

    async fn get_collection_schema(
        &self,
        user_id: &str,
        collection: &str,
    ) -> Result<CollectionSchema, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT schema FROM structured_schemas WHERE user_id = ?1 AND collection = ?2",
                libsql::params![user_id, collection],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("get_collection_schema: {e}")))?;

        let row = rows.next().await.map_err(|e| {
            DatabaseError::Query(format!("get_collection_schema next: {e}"))
        })?.ok_or_else(|| DatabaseError::NotFound {
            entity: "collection".to_string(),
            id: collection.to_string(),
        })?;

        let schema_str = get_text(&row, 0);
        serde_json::from_str(&schema_str)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))
    }

    async fn list_collections(
        &self,
        user_id: &str,
    ) -> Result<Vec<CollectionSchema>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT schema FROM structured_schemas WHERE user_id = ?1 ORDER BY collection",
                libsql::params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("list_collections: {e}")))?;

        let mut schemas = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("list_collections next: {e}")))?
        {
            let schema_str = get_text(&row, 0);
            let schema: CollectionSchema = serde_json::from_str(&schema_str)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            schemas.push(schema);
        }
        Ok(schemas)
    }

    async fn drop_collection(&self, user_id: &str, collection: &str) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;

        // Delete records first (foreign key CASCADE may not be enabled).
        conn.execute(
            "DELETE FROM structured_records WHERE user_id = ?1 AND collection = ?2",
            libsql::params![user_id, collection],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("drop_collection records: {e}")))?;

        let n = conn
            .execute(
                "DELETE FROM structured_schemas WHERE user_id = ?1 AND collection = ?2",
                libsql::params![user_id, collection],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("drop_collection schema: {e}")))?;

        if n == 0 {
            return Err(DatabaseError::NotFound {
                entity: "collection".to_string(),
                id: collection.to_string(),
            });
        }
        Ok(())
    }

    async fn insert_record(
        &self,
        user_id: &str,
        collection: &str,
        data: serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        let schema = self.get_collection_schema(user_id, collection).await?;
        let validated = schema
            .validate_record(&data)
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let id = Uuid::new_v4();
        let now = chrono::Utc::now();
        let now_str = fmt_ts(&now);
        let data_str = serde_json::to_string(&validated)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO structured_records (id, user_id, collection, data, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            libsql::params![
                id.to_string(),
                user_id,
                collection,
                data_str,
                now_str.clone(),
                now_str
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(format!("insert_record: {e}")))?;

        Ok(id)
    }

    async fn get_record(&self, user_id: &str, record_id: Uuid) -> Result<Record, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, collection, data, created_at, updated_at
                FROM structured_records
                WHERE id = ?1 AND user_id = ?2
                "#,
                libsql::params![record_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("get_record: {e}")))?;

        let row = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("get_record next: {e}")))?
            .ok_or_else(|| DatabaseError::NotFound {
                entity: "record".to_string(),
                id: record_id.to_string(),
            })?;

        row_to_record(&row)
    }

    async fn update_record(
        &self,
        user_id: &str,
        record_id: Uuid,
        updates: serde_json::Value,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;

        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(|e| DatabaseError::Query(format!("update_record BEGIN: {e}")))?;

        let result = async {
            // Fetch existing record within the transaction.
            let mut rows = conn
                .query(
                    r#"
                    SELECT id, user_id, collection, data, created_at, updated_at
                    FROM structured_records
                    WHERE id = ?1 AND user_id = ?2
                    "#,
                    libsql::params![record_id.to_string(), user_id],
                )
                .await
                .map_err(|e| DatabaseError::Query(format!("update_record select: {e}")))?;

            let row = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(format!("update_record next: {e}")))?
                .ok_or_else(|| DatabaseError::NotFound {
                    entity: "record".to_string(),
                    id: record_id.to_string(),
                })?;
            let existing = row_to_record(&row)?;

            // Fetch schema within the transaction.
            let mut schema_rows = conn
                .query(
                    "SELECT schema FROM structured_schemas WHERE user_id = ?1 AND collection = ?2",
                    libsql::params![user_id, existing.collection.as_str()],
                )
                .await
                .map_err(|e| DatabaseError::Query(format!("update_record schema: {e}")))?;

            let schema_row = schema_rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(format!("update_record schema next: {e}")))?
                .ok_or_else(|| DatabaseError::NotFound {
                    entity: "collection".to_string(),
                    id: existing.collection.clone(),
                })?;
            let schema_str = get_text(&schema_row, 0);
            let schema: CollectionSchema = serde_json::from_str(&schema_str)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

            // Validate the partial update.
            let validated_updates = schema
                .validate_partial(&updates)
                .map_err(|e| DatabaseError::Query(e.to_string()))?;

            // Merge updates into existing data.
            let mut merged = existing.data.clone();
            if let (Some(base), Some(patch)) =
                (merged.as_object_mut(), validated_updates.as_object())
            {
                for (k, v) in patch {
                    base.insert(k.clone(), v.clone());
                }
            }

            let now = chrono::Utc::now();
            let now_str = fmt_ts(&now);
            let merged_str = serde_json::to_string(&merged)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

            let n = conn
                .execute(
                    r#"
                    UPDATE structured_records
                    SET data = ?1, updated_at = ?2
                    WHERE id = ?3 AND user_id = ?4
                    "#,
                    libsql::params![merged_str, now_str, record_id.to_string(), user_id],
                )
                .await
                .map_err(|e| DatabaseError::Query(format!("update_record: {e}")))?;

            if n == 0 {
                return Err(DatabaseError::NotFound {
                    entity: "record".to_string(),
                    id: record_id.to_string(),
                });
            }

            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                conn.execute("COMMIT", ())
                    .await
                    .map_err(|e| DatabaseError::Query(format!("update_record COMMIT: {e}")))?;
                Ok(())
            }
            Err(e) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(e)
            }
        }
    }

    async fn delete_record(&self, user_id: &str, record_id: Uuid) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let n = conn
            .execute(
                "DELETE FROM structured_records WHERE id = ?1 AND user_id = ?2",
                libsql::params![record_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(format!("delete_record: {e}")))?;

        if n == 0 {
            return Err(DatabaseError::NotFound {
                entity: "record".to_string(),
                id: record_id.to_string(),
            });
        }
        Ok(())
    }

    async fn query_records(
        &self,
        user_id: &str,
        collection: &str,
        filters: &[Filter],
        order_by: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Record>, DatabaseError> {
        // Cap at 1000 for normal queries, but allow uncapped for internal
        // callers (e.g., aggregate_in_rust) that pass usize::MAX.
        let capped_limit = if limit == usize::MAX {
            i64::MAX
        } else {
            limit.min(1000) as i64
        };

        // Start building the query. Params ?1 = user_id, ?2 = collection.
        let mut sql = String::from(
            "SELECT id, user_id, collection, data, created_at, updated_at \
             FROM structured_records WHERE user_id = ?1 AND collection = ?2",
        );
        let mut params: Vec<libsql::Value> = Vec::new();
        params.push(libsql::Value::Text(user_id.to_string()));
        params.push(libsql::Value::Text(collection.to_string()));

        // Build filter clauses starting at ?3.
        let (filter_clauses, filter_params) = build_filters(filters, 3)?;
        for clause in &filter_clauses {
            sql.push_str(" AND ");
            sql.push_str(clause);
        }
        params.extend(filter_params);

        // ORDER BY
        match order_by {
            Some(field) => {
                structured::validate_field_name(field)
                    .map_err(|e| DatabaseError::Query(e.to_string()))?;
                // Top-level columns are used directly, not extracted from JSON.
                if field == "created_at" || field == "updated_at" {
                    sql.push_str(&format!(" ORDER BY {field}"));
                } else {
                    sql.push_str(&format!(" ORDER BY json_extract(data, '$.{field}')"));
                }
            }
            None => {
                sql.push_str(" ORDER BY created_at DESC");
            }
        }

        // LIMIT
        let limit_idx = params.len() as i32 + 1;
        sql.push_str(&format!(" LIMIT ?{limit_idx}"));
        params.push(libsql::Value::Integer(capped_limit));

        let conn = self.connect().await?;
        let mut rows = conn
            .query(&sql, params)
            .await
            .map_err(|e| DatabaseError::Query(format!("query_records: {e}")))?;

        let mut records = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(format!("query_records next: {e}")))?
        {
            records.push(row_to_record(&row)?);
        }
        Ok(records)
    }

    async fn aggregate(
        &self,
        user_id: &str,
        collection: &str,
        aggregation: &Aggregation,
    ) -> Result<serde_json::Value, DatabaseError> {
        let group_by = &aggregation.group_by;

        // Validate field names to prevent SQL injection (they are interpolated
        // directly into query strings).
        if let Some(field) = &aggregation.field {
            structured::validate_field_name(field)
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
        }
        if let Some(group_field) = group_by {
            structured::validate_field_name(group_field)
                .map_err(|e| DatabaseError::Query(e.to_string()))?;
        }

        // SUM and AVG are computed in Rust to work around a libSQL SDK bug
        // where CAST(json_extract(integer) AS REAL) returns 0 through the
        // row.get::<f64>() API.  We fetch the raw JSON values and aggregate
        // in application code.  COUNT/MIN/MAX are still done in SQL.
        if matches!(
            aggregation.operation,
            structured::AggOp::Sum | structured::AggOp::Avg
        ) {
            return aggregate_in_rust(self, user_id, collection, aggregation).await;
        }

        // Build the aggregation expression for COUNT/MIN/MAX.
        let agg_expr = match aggregation.operation {
            structured::AggOp::Count => "COUNT(*)".to_string(),
            structured::AggOp::Min => {
                let field = aggregation
                    .field
                    .as_deref()
                    .ok_or_else(|| DatabaseError::Query("Min requires a field".to_string()))?;
                format!("MIN(json_extract(data, '$.{field}'))")
            }
            structured::AggOp::Max => {
                let field = aggregation
                    .field
                    .as_deref()
                    .ok_or_else(|| DatabaseError::Query("Max requires a field".to_string()))?;
                format!("MAX(json_extract(data, '$.{field}'))")
            }
            // SUM/AVG handled above.
            _ => unreachable!(),
        };

        // Start building query. ?1 = user_id, ?2 = collection.
        let mut sql = if let Some(group_field) = group_by {
            format!(
                "SELECT json_extract(data, '$.{group_field}') AS group_key, {agg_expr} AS result \
                 FROM structured_records WHERE user_id = ?1 AND collection = ?2"
            )
        } else {
            format!(
                "SELECT {agg_expr} AS result \
                 FROM structured_records WHERE user_id = ?1 AND collection = ?2"
            )
        };

        let mut params: Vec<libsql::Value> = Vec::new();
        params.push(libsql::Value::Text(user_id.to_string()));
        params.push(libsql::Value::Text(collection.to_string()));

        // Apply filters.
        let (filter_clauses, filter_params) = build_filters(&aggregation.filters, 3)?;
        for clause in &filter_clauses {
            sql.push_str(" AND ");
            sql.push_str(clause);
        }
        params.extend(filter_params);

        // GROUP BY
        if let Some(group_field) = group_by {
            sql.push_str(&format!(
                " GROUP BY json_extract(data, '$.{group_field}')"
            ));
        }

        let conn = self.connect().await?;
        let mut rows = conn
            .query(&sql, params)
            .await
            .map_err(|e| DatabaseError::Query(format!("aggregate: {e}")))?;

        if group_by.is_some() {
            // Grouped result: return an object { "group_key": result, ... }
            // Columns: 0 = group_key, 1 = result
            let mut result_map = serde_json::Map::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(format!("aggregate next: {e}")))?
            {
                let key = get_text(&row, 0);
                let value = extract_agg_value(&row, 1, &aggregation.operation)?;
                result_map.insert(key, value);
            }
            Ok(serde_json::Value::Object(result_map))
        } else {
            // Single result. Column: 0 = result
            let row = rows
                .next()
                .await
                .map_err(|e| DatabaseError::Query(format!("aggregate next: {e}")))?
                .ok_or_else(|| {
                    DatabaseError::Query("Aggregation returned no rows".to_string())
                })?;
            extract_agg_value(&row, 0, &aggregation.operation)
        }
    }
}

/// Extract the aggregation result value from a libSQL row.
///
/// `result_idx` is the column index containing the aggregation result:
/// 0 for non-grouped queries, 1 for grouped (where column 0 is group_key).
fn extract_agg_value(
    row: &libsql::Row,
    result_idx: i32,
    op: &structured::AggOp,
) -> Result<serde_json::Value, DatabaseError> {
    match op {
        structured::AggOp::Count => {
            // COUNT(*) returns an integer in SQLite.
            let count = row.get::<i64>(result_idx).unwrap_or(0);
            Ok(serde_json::json!(count))
        }
        structured::AggOp::Sum | structured::AggOp::Avg => {
            // SUM/AVG returns REAL in standard SQLite, but libSQL may return
            // integer when all aggregated values are integers.  Try f64 first,
            // then i64, then treat as NULL.
            if let Ok(f) = row.get::<f64>(result_idx) {
                Ok(serde_json::json!(f))
            } else if let Ok(i) = row.get::<i64>(result_idx) {
                Ok(serde_json::json!(i as f64))
            } else {
                Ok(serde_json::Value::Null)
            }
        }
        structured::AggOp::Min | structured::AggOp::Max => {
            // MIN/MAX can return text or numeric depending on the data.
            // Try as f64 first (numeric fields), then fall back to string.
            if let Ok(f) = row.get::<f64>(result_idx) {
                Ok(serde_json::json!(f))
            } else if let Ok(s) = row.get::<String>(result_idx) {
                // Try to parse as number for consistent behavior with PG backend.
                if let Ok(n) = s.parse::<f64>() {
                    Ok(serde_json::json!(n))
                } else {
                    Ok(serde_json::json!(s))
                }
            } else {
                Ok(serde_json::Value::Null)
            }
        }
    }
}

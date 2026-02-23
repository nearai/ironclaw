//! Structured collections StructuredStore implementation for LibSqlBackend.

use async_trait::async_trait;
use chrono::Utc;
use libsql::params;
use uuid::Uuid;

use crate::db::libsql::{LibSqlBackend, fmt_ts, get_text, get_ts};
use crate::db::structured::{
    AggOp, Aggregation, CollectionSchema, Filter, FilterOp, Record, StructuredStore, json_to_text,
};
use crate::error::DatabaseError;

/// Convert a libsql Row to a Record struct.
///
/// Column order: id(0), user_id(1), collection(2), data(3), created_at(4), updated_at(5).
fn row_to_record(row: &libsql::Row) -> Result<Record, DatabaseError> {
    let id_str = get_text(row, 0);
    let id: Uuid = id_str
        .parse()
        .map_err(|e| DatabaseError::Serialization(format!("invalid record id: {e}")))?;
    let user_id = get_text(row, 1);
    let collection = get_text(row, 2);
    let data_str = get_text(row, 3);
    let data: serde_json::Value = serde_json::from_str(&data_str)
        .map_err(|e| DatabaseError::Serialization(format!("invalid record data: {e}")))?;
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

/// Check whether a single record's field value matches a filter.
///
/// Returns true if the record passes the filter.
fn matches_filter(data: &serde_json::Value, filter: &Filter) -> Result<bool, DatabaseError> {
    let field_val = data.get(&filter.field);

    match filter.op {
        FilterOp::IsNull => Ok(field_val.is_none() || field_val == Some(&serde_json::Value::Null)),
        FilterOp::IsNotNull => {
            Ok(field_val.is_some() && field_val != Some(&serde_json::Value::Null))
        }
        FilterOp::Eq => {
            let fv = match field_val {
                Some(v) if !v.is_null() => v,
                _ => return Ok(false),
            };
            Ok(json_to_text(fv) == json_to_text(&filter.value))
        }
        FilterOp::Neq => {
            let fv = match field_val {
                Some(v) if !v.is_null() => v,
                _ => return Ok(true),
            };
            Ok(json_to_text(fv) != json_to_text(&filter.value))
        }
        FilterOp::Gt => Ok(compare_fields(field_val, &filter.value) == Some(std::cmp::Ordering::Greater)),
        FilterOp::Gte => {
            let ord = compare_fields(field_val, &filter.value);
            Ok(ord == Some(std::cmp::Ordering::Greater) || ord == Some(std::cmp::Ordering::Equal))
        }
        FilterOp::Lt => Ok(compare_fields(field_val, &filter.value) == Some(std::cmp::Ordering::Less)),
        FilterOp::Lte => {
            let ord = compare_fields(field_val, &filter.value);
            Ok(ord == Some(std::cmp::Ordering::Less) || ord == Some(std::cmp::Ordering::Equal))
        }
        FilterOp::Between => {
            let arr = filter.value.as_array().ok_or_else(|| {
                DatabaseError::Query("Between filter requires an array of [lo, hi]".to_string())
            })?;
            if arr.len() != 2 {
                return Err(DatabaseError::Query(
                    "Between filter requires exactly 2 elements".to_string(),
                ));
            }
            let gte = {
                let ord = compare_fields(field_val, &arr[0]);
                ord == Some(std::cmp::Ordering::Greater) || ord == Some(std::cmp::Ordering::Equal)
            };
            let lte = {
                let ord = compare_fields(field_val, &arr[1]);
                ord == Some(std::cmp::Ordering::Less) || ord == Some(std::cmp::Ordering::Equal)
            };
            Ok(gte && lte)
        }
        FilterOp::In => {
            let arr = filter.value.as_array().ok_or_else(|| {
                DatabaseError::Query("In filter requires an array value".to_string())
            })?;
            let fv = match field_val {
                Some(v) if !v.is_null() => v,
                _ => return Ok(false),
            };
            let fv_text = json_to_text(fv);
            Ok(arr.iter().any(|item| json_to_text(item) == fv_text))
        }
    }
}

/// Compare a record field value against a filter value, returning ordering.
///
/// Both values are compared as text strings (matching PostgreSQL `data->>'field'`
/// semantics which always returns text).
fn compare_fields(
    field_val: Option<&serde_json::Value>,
    filter_val: &serde_json::Value,
) -> Option<std::cmp::Ordering> {
    let fv = match field_val {
        Some(v) if !v.is_null() => v,
        _ => return None,
    };
    let a = json_to_text(fv);
    let b = json_to_text(filter_val);

    // Try numeric comparison first for consistent behavior with PostgreSQL.
    if let (Ok(na), Ok(nb)) = (a.parse::<f64>(), b.parse::<f64>()) {
        return na.partial_cmp(&nb);
    }

    Some(a.cmp(&b))
}


#[async_trait]
impl StructuredStore for LibSqlBackend {
    async fn register_collection(
        &self,
        user_id: &str,
        schema: &CollectionSchema,
    ) -> Result<(), DatabaseError> {
        CollectionSchema::validate_name(&schema.collection)
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let schema_json = serde_json::to_string(schema)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO structured_schemas (user_id, collection, schema, description)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT (user_id, collection) DO UPDATE SET
                schema = excluded.schema,
                description = excluded.description
            "#,
            params![
                user_id,
                schema.collection.as_str(),
                schema_json,
                schema.description.as_deref(),
            ],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

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
                params![user_id, collection],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let row = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
            .ok_or_else(|| DatabaseError::NotFound {
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
                params![user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut schemas = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            let schema_str = get_text(&row, 0);
            let schema: CollectionSchema = serde_json::from_str(&schema_str)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
            schemas.push(schema);
        }
        Ok(schemas)
    }

    async fn drop_collection(
        &self,
        user_id: &str,
        collection: &str,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;

        // Delete records first (FK cascade may not be enforced by default in SQLite).
        conn.execute(
            "DELETE FROM structured_records WHERE user_id = ?1 AND collection = ?2",
            params![user_id, collection],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let n = conn
            .execute(
                "DELETE FROM structured_schemas WHERE user_id = ?1 AND collection = ?2",
                params![user_id, collection],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

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
        let data_str = serde_json::to_string(&validated)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let now = fmt_ts(&Utc::now());

        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO structured_records (id, user_id, collection, data, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            params![id.to_string(), user_id, collection, data_str, now.clone(), now],
        )
        .await
        .map_err(|e| DatabaseError::Query(e.to_string()))?;

        Ok(id)
    }

    async fn get_record(
        &self,
        user_id: &str,
        record_id: Uuid,
    ) -> Result<Record, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, collection, data, created_at, updated_at
                FROM structured_records
                WHERE id = ?1 AND user_id = ?2
                "#,
                params![record_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let row = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
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
        // Fetch existing record to get its collection and current data.
        let existing = self.get_record(user_id, record_id).await?;
        let schema = self
            .get_collection_schema(user_id, &existing.collection)
            .await?;

        // Validate the partial update.
        let validated_updates = schema
            .validate_partial(&updates)
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        // Merge updates into existing data.
        let mut merged = existing.data.clone();
        if let (Some(base), Some(patch)) = (merged.as_object_mut(), validated_updates.as_object())
        {
            for (k, v) in patch {
                base.insert(k.clone(), v.clone());
            }
        }

        let merged_str = serde_json::to_string(&merged)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let now = fmt_ts(&Utc::now());

        let conn = self.connect().await?;
        let n = conn
            .execute(
                r#"
                UPDATE structured_records
                SET data = ?1, updated_at = ?2
                WHERE id = ?3 AND user_id = ?4
                "#,
                params![merged_str, now, record_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        if n == 0 {
            return Err(DatabaseError::NotFound {
                entity: "record".to_string(),
                id: record_id.to_string(),
            });
        }
        Ok(())
    }

    async fn delete_record(
        &self,
        user_id: &str,
        record_id: Uuid,
    ) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        let n = conn
            .execute(
                "DELETE FROM structured_records WHERE id = ?1 AND user_id = ?2",
                params![record_id.to_string(), user_id],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

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
        let capped_limit = limit.min(1000);

        // Fetch all records for the collection, then filter in Rust.
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, collection, data, created_at, updated_at
                FROM structured_records
                WHERE user_id = ?1 AND collection = ?2
                ORDER BY created_at DESC
                "#,
                params![user_id, collection],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut all_records = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            all_records.push(row_to_record(&row)?);
        }

        // Apply filters.
        let mut filtered: Vec<Record> = Vec::new();
        for record in all_records {
            let mut passes = true;
            for filter in filters {
                if !matches_filter(&record.data, filter)? {
                    passes = false;
                    break;
                }
            }
            if passes {
                filtered.push(record);
            }
        }

        // Sort.
        match order_by {
            Some(field) => {
                let field_owned = field.to_string();
                filtered.sort_by(|a, b| {
                    let va = a.data.get(&field_owned).map(json_to_text).unwrap_or_default();
                    let vb = b.data.get(&field_owned).map(json_to_text).unwrap_or_default();
                    // Try numeric sort first.
                    if let (Ok(na), Ok(nb)) = (va.parse::<f64>(), vb.parse::<f64>()) {
                        return na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal);
                    }
                    va.cmp(&vb)
                });
            }
            None => {
                // Default: created_at DESC (already sorted from SQL query).
            }
        }

        // Apply limit.
        filtered.truncate(capped_limit);
        Ok(filtered)
    }

    async fn aggregate(
        &self,
        user_id: &str,
        collection: &str,
        aggregation: &Aggregation,
    ) -> Result<serde_json::Value, DatabaseError> {
        // Fetch all records, filter, then compute aggregation in Rust.
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, collection, data, created_at, updated_at
                FROM structured_records
                WHERE user_id = ?1 AND collection = ?2
                "#,
                params![user_id, collection],
            )
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?;

        let mut all_records = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| DatabaseError::Query(e.to_string()))?
        {
            all_records.push(row_to_record(&row)?);
        }

        // Apply filters.
        let mut filtered = Vec::new();
        for record in &all_records {
            let mut passes = true;
            for filter in &aggregation.filters {
                if !matches_filter(&record.data, filter)? {
                    passes = false;
                    break;
                }
            }
            if passes {
                filtered.push(record);
            }
        }

        if let Some(ref group_field) = aggregation.group_by {
            // Grouped aggregation.
            let mut groups: std::collections::BTreeMap<String, Vec<&Record>> =
                std::collections::BTreeMap::new();
            for record in filtered.iter().copied() {
                let key = record
                    .data
                    .get(group_field)
                    .map(json_to_text)
                    .unwrap_or_else(|| "null".to_string());
                groups.entry(key).or_default().push(record);
            }

            let mut result_map = serde_json::Map::new();
            for (key, records) in &groups {
                let value = compute_agg(&aggregation.operation, aggregation.field.as_deref(), records)?;
                result_map.insert(key.clone(), value);
            }
            Ok(serde_json::Value::Object(result_map))
        } else {
            // Single (ungrouped) aggregation.
            compute_agg(&aggregation.operation, aggregation.field.as_deref(), &filtered)
        }
    }
}

/// Compute an aggregation over a set of records.
fn compute_agg(
    op: &AggOp,
    field: Option<&str>,
    records: &[&Record],
) -> Result<serde_json::Value, DatabaseError> {
    match op {
        AggOp::Count => Ok(serde_json::json!(records.len() as i64)),
        AggOp::Sum => {
            let field = field.ok_or_else(|| {
                DatabaseError::Query("Sum requires a field".to_string())
            })?;
            let mut sum: f64 = 0.0;
            let mut has_value = false;
            for record in records {
                if let Some(val) = record.data.get(field)
                    && !val.is_null()
                {
                    let text = json_to_text(val);
                    let n: f64 = text.parse().map_err(|_| {
                        DatabaseError::Query(format!(
                            "Cannot parse '{text}' as number for Sum on field '{field}'"
                        ))
                    })?;
                    sum += n;
                    has_value = true;
                }
            }
            if has_value {
                Ok(serde_json::json!(sum))
            } else {
                Ok(serde_json::Value::Null)
            }
        }
        AggOp::Avg => {
            let field = field.ok_or_else(|| {
                DatabaseError::Query("Avg requires a field".to_string())
            })?;
            let mut sum: f64 = 0.0;
            let mut count: usize = 0;
            for record in records {
                if let Some(val) = record.data.get(field)
                    && !val.is_null()
                {
                    let text = json_to_text(val);
                    let n: f64 = text.parse().map_err(|_| {
                        DatabaseError::Query(format!(
                            "Cannot parse '{text}' as number for Avg on field '{field}'"
                        ))
                    })?;
                    sum += n;
                    count += 1;
                }
            }
            if count > 0 {
                Ok(serde_json::json!(sum / count as f64))
            } else {
                Ok(serde_json::Value::Null)
            }
        }
        AggOp::Min => {
            let field = field.ok_or_else(|| {
                DatabaseError::Query("Min requires a field".to_string())
            })?;
            let mut min_val: Option<String> = None;
            for record in records {
                if let Some(val) = record.data.get(field)
                    && !val.is_null()
                {
                    let text = json_to_text(val);
                    match &min_val {
                        None => min_val = Some(text),
                        Some(current) => {
                            // Try numeric comparison.
                            if let (Ok(nc), Ok(nt)) =
                                (current.parse::<f64>(), text.parse::<f64>())
                            {
                                if nt < nc {
                                    min_val = Some(text);
                                }
                            } else if text < *current {
                                min_val = Some(text);
                            }
                        }
                    }
                }
            }
            match min_val {
                Some(s) => {
                    if let Ok(n) = s.parse::<f64>() {
                        Ok(serde_json::json!(n))
                    } else {
                        Ok(serde_json::json!(s))
                    }
                }
                None => Ok(serde_json::Value::Null),
            }
        }
        AggOp::Max => {
            let field = field.ok_or_else(|| {
                DatabaseError::Query("Max requires a field".to_string())
            })?;
            let mut max_val: Option<String> = None;
            for record in records {
                if let Some(val) = record.data.get(field)
                    && !val.is_null()
                {
                    let text = json_to_text(val);
                    match &max_val {
                        None => max_val = Some(text),
                        Some(current) => {
                            // Try numeric comparison.
                            if let (Ok(nc), Ok(nt)) =
                                (current.parse::<f64>(), text.parse::<f64>())
                            {
                                if nt > nc {
                                    max_val = Some(text);
                                }
                            } else if text > *current {
                                max_val = Some(text);
                            }
                        }
                    }
                }
            }
            match max_val {
                Some(s) => {
                    if let Ok(n) = s.parse::<f64>() {
                        Ok(serde_json::json!(n))
                    } else {
                        Ok(serde_json::json!(s))
                    }
                }
                None => Ok(serde_json::Value::Null),
            }
        }
    }
}

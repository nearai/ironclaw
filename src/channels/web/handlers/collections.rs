//! REST API handlers for structured collections.
//!
//! Provides full CRUD access to structured collections with cross-lens support.
//! Cross-lens access uses `workspace_read_scopes` from the authenticated user's
//! identity — if a user has a read scope for another user, they can read
//! that user's collections.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::server::GatewayState;
use crate::db::structured::{
    CollectionSchema, Filter, FilterOp, StructuredStore, append_history, init_history,
};

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct CollectionListResponse {
    collections: Vec<CollectionInfo>,
}

#[derive(Debug, Serialize)]
struct CollectionInfo {
    collection: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    user_id: String,
    fields: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct CollectionQueryResponse {
    collection: String,
    records: Vec<RecordInfo>,
    count: usize,
}

#[derive(Debug, Serialize)]
struct RecordInfo {
    id: Uuid,
    data: serde_json::Value,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CollectionInsertRequest {
    pub data: serde_json::Value,
    #[serde(default = "default_source")]
    pub source: String,
    pub context: Option<String>,
}

fn default_source() -> String {
    "rest_api".to_string()
}

#[derive(Debug, Deserialize)]
pub struct CollectionUpdateRequest {
    pub data: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct CollectionMutationResponse {
    status: String,
    record_id: String,
    collection: String,
}

// ---------------------------------------------------------------------------
// Cross-lens resolution
// ---------------------------------------------------------------------------

/// Resolve which user_id owns a collection, checking the caller's own scope
/// first, then each workspace_read_scope in order.
async fn resolve_collection_owner(
    db: &(dyn StructuredStore + Send + Sync),
    caller_user_id: &str,
    scopes: &[String],
    collection: &str,
) -> Option<String> {
    // Try caller's own collections first.
    if db
        .get_collection_schema(caller_user_id, collection)
        .await
        .is_ok()
    {
        return Some(caller_user_id.to_string());
    }
    // Try each scope.
    for scope in scopes {
        if db.get_collection_schema(scope, collection).await.is_ok() {
            return Some(scope.clone());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/collections
///
/// List all collection schemas visible to the authenticated user (own + scopes).
pub async fn collections_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> impl IntoResponse {
    let db = match &state.store {
        Some(db) => Arc::clone(db),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "database not available"})),
            )
                .into_response();
        }
    };

    let mut all_collections = Vec::new();

    // Collect user IDs to query: own + scopes.
    let mut user_ids = vec![user.user_id.clone()];
    user_ids.extend(user.workspace_read_scopes.iter().cloned());

    for uid in &user_ids {
        match db.list_collections(uid).await {
            Ok(schemas) => {
                for schema in schemas {
                    // Serialize fields to JSON value.
                    let fields = serde_json::to_value(&schema.fields)
                        .unwrap_or(serde_json::Value::Object(Default::default()));
                    all_collections.push(CollectionInfo {
                        collection: schema.collection,
                        description: schema.description,
                        user_id: uid.to_string(),
                        fields,
                    });
                }
            }
            Err(e) => {
                tracing::warn!(user_id = %uid, error = %e, "failed to list collections");
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!(CollectionListResponse {
            collections: all_collections,
        })),
    )
        .into_response()
}

/// GET /api/collections/{name}?field=value&...
///
/// Query records from a collection. Supports equality filtering via query params.
pub async fn collections_query_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let db = match &state.store {
        Some(db) => Arc::clone(db),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "database not available"})),
            )
                .into_response();
        }
    };

    let owner = match resolve_collection_owner(
        db.as_ref(),
        &user.user_id,
        &user.workspace_read_scopes,
        &name,
    )
    .await
    {
        Some(uid) => uid,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("collection '{}' not found", name)})),
            )
                .into_response();
        }
    };

    // Build filters from query params (skip reserved params).
    let filters: Vec<Filter> = params
        .iter()
        .filter(|(k, _)| !matches!(k.as_str(), "limit" | "order_by"))
        .map(|(k, v)| Filter {
            field: k.clone(),
            op: FilterOp::Eq,
            value: serde_json::Value::String(v.clone()),
        })
        .collect();

    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(200);
    let order_by = params.get("order_by").map(|s| s.as_str());

    match db
        .query_records(&owner, &name, &filters, order_by, limit)
        .await
    {
        Ok(records) => {
            let count = records.len();
            let records: Vec<RecordInfo> = records
                .into_iter()
                .map(|r| RecordInfo {
                    id: r.id,
                    data: r.data,
                    created_at: r.created_at.to_rfc3339(),
                    updated_at: r.updated_at.to_rfc3339(),
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!(CollectionQueryResponse {
                    collection: name,
                    records,
                    count,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// POST /api/collections/{name}
///
/// Insert a new record. Injects `_lineage` for provenance. Fires `CollectionWriteEvent`.
pub async fn collections_insert_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
    Json(req): Json<CollectionInsertRequest>,
) -> impl IntoResponse {
    let db = match &state.store {
        Some(db) => Arc::clone(db),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "database not available"})),
            )
                .into_response();
        }
    };

    let owner = match resolve_collection_owner(
        db.as_ref(),
        &user.user_id,
        &user.workspace_read_scopes,
        &name,
    )
    .await
    {
        Some(uid) => uid,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("collection '{}' not found", name)})),
            )
                .into_response();
        }
    };

    // Inject _lineage.
    let mut data = req.data.clone();
    match data.as_object_mut() {
        Some(obj) => {
            let event_id = Uuid::new_v4();
            let timestamp = chrono::Utc::now().to_rfc3339();
            obj.insert(
                "_lineage".to_string(),
                serde_json::json!({
                    "source": req.source,
                    "source_id": event_id.to_string(),
                    "created_by": req.source,
                    "context": req.context,
                    "timestamp": timestamp
                }),
            );
        }
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "data must be a JSON object"})),
            )
                .into_response();
        }
    }

    // Inject _history for audit trail.
    init_history(&mut data, &req.source);



    match db.insert_record(&owner, &name, data).await {
        Ok(id) => {
            // Fire collection write event for SSE broadcast.
            if let Some(ref tx) = state.collection_write_tx {
                let _ = tx.send(crate::agent::collection_events::CollectionWriteEvent {
                    user_id: user.user_id.clone(),
                    collection: name.clone(),
                    record_id: id,
                    operation: "insert".to_string(),
                    data: req.data,
                });
            }
            (
                StatusCode::CREATED,
                Json(serde_json::json!(CollectionMutationResponse {
                    status: "created".to_string(),
                    record_id: id.to_string(),
                    collection: name,
                })),
            )
                .into_response()
        }
        Err(e) => {
            let status = if e.to_string().contains("NotFound") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            (status, Json(serde_json::json!({"error": e.to_string()}))).into_response()
        }
    }
}

/// PATCH /api/collections/{name}/{id}
///
/// Update fields on an existing record by ID.
pub async fn collections_update_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path((name, id)): Path<(String, Uuid)>,
    Json(req): Json<CollectionUpdateRequest>,
) -> impl IntoResponse {
    let db = match &state.store {
        Some(db) => Arc::clone(db),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "database not available"})),
            )
                .into_response();
        }
    };

    let owner = match resolve_collection_owner(
        db.as_ref(),
        &user.user_id,
        &user.workspace_read_scopes,
        &name,
    )
    .await
    {
        Some(uid) => uid,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("collection '{}' not found", name)})),
            )
                .into_response();
        }
    };

    // Fetch existing record to append _history before the update.
    let event_data = req.data.clone();
    let mut update_data = req.data;
    match db.get_record(&owner, id).await {
        Ok(existing) => {
            let mut existing_data = existing.data;
            append_history(&mut existing_data, &update_data, "rest_api");
            // Carry the updated _history into the update payload so the DB merge
            // replaces the old _history with the appended version.
            if let Some(history) = existing_data.get("_history")
                && let Some(obj) = update_data.as_object_mut()
            {
                obj.insert("_history".to_string(), history.clone());
            }
        }
        Err(e) => {
            let status = if e.to_string().contains("NotFound") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            return (status, Json(serde_json::json!({"error": e.to_string()}))).into_response();
        }
    }

    match db.update_record(&owner, id, update_data).await {
        Ok(()) => {
            // Fire collection write event for SSE broadcast.
            if let Some(ref tx) = state.collection_write_tx {
                let _ = tx.send(crate::agent::collection_events::CollectionWriteEvent {
                    user_id: user.user_id.clone(),
                    collection: name.clone(),
                    record_id: id,
                    operation: "update".to_string(),
                    data: event_data,
                });
            }
            (
                StatusCode::OK,
                Json(serde_json::json!(CollectionMutationResponse {
                    status: "updated".to_string(),
                    record_id: id.to_string(),
                    collection: name,
                })),
            )
                .into_response()
        }
        Err(e) => {
            let status = if e.to_string().contains("NotFound") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_REQUEST
            };
            (status, Json(serde_json::json!({"error": e.to_string()}))).into_response()
        }
    }
}

/// DELETE /api/collections/{name}/{id}
///
/// Delete a record by ID.
pub async fn collections_delete_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path((name, id)): Path<(String, Uuid)>,
) -> impl IntoResponse {
    let db = match &state.store {
        Some(db) => Arc::clone(db),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "database not available"})),
            )
                .into_response();
        }
    };

    let owner = match resolve_collection_owner(
        db.as_ref(),
        &user.user_id,
        &user.workspace_read_scopes,
        &name,
    )
    .await
    {
        Some(uid) => uid,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("collection '{}' not found", name)})),
            )
                .into_response();
        }
    };

    match db.delete_record(&owner, id).await {
        Ok(()) => {
            // Fire collection write event for SSE broadcast.
            if let Some(ref tx) = state.collection_write_tx {
                let _ = tx.send(crate::agent::collection_events::CollectionWriteEvent {
                    user_id: user.user_id.clone(),
                    collection: name.clone(),
                    record_id: id,
                    operation: "delete".to_string(),
                    data: serde_json::Value::Null,
                });
            }
            (
                StatusCode::OK,
                Json(serde_json::json!(CollectionMutationResponse {
                    status: "deleted".to_string(),
                    record_id: id.to_string(),
                    collection: name,
                })),
            )
                .into_response()
        }
        Err(e) => {
            let status = if e.to_string().contains("NotFound") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(serde_json::json!({"error": e.to_string()}))).into_response()
        }
    }
}

/// POST /api/collections
///
/// Register (or update) a collection schema.
pub async fn collections_register_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(mut schema): Json<CollectionSchema>,
) -> impl IntoResponse {
    // Only trusted seeding paths can set source_scope
    schema.source_scope = None;

    let db = match &state.store {
        Some(db) => Arc::clone(db),
        None => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error": "database not available"})),
            )
                .into_response();
        }
    };

    match db.register_collection(&user.user_id, &schema).await {
        Ok(()) => {
            // Generate and register per-collection dynamic tools (best-effort).
            // Without this, tools like {collection}_query won't appear in the registry.
            let mut tool_names = Vec::new();
            if let Some(ref registry) = state.tool_registry {
                // Build workspace resolver from pool or single workspace.
                let ws_resolver: Option<Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>> =
                    if let Some(ref pool) = state.workspace_pool {
                        Some(Arc::clone(pool) as Arc<dyn crate::tools::builtin::memory::WorkspaceResolver>)
                    } else {
                        None
                    };
                tool_names =
                    crate::tools::builtin::collections::refresh_collection_tools(
                        &schema,
                        &db,
                        registry,
                        state.skills_dir.as_deref(),
                        state.skill_registry.as_ref(),
                        &user.user_id,
                        state.collection_write_tx.as_ref(),
                        ws_resolver.as_ref(),
                    )
                    .await;
            }

            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "status": "registered",
                    "collection": schema.collection,
                    "tools_created": tool_names,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_request_default_source() {
        let json = r#"{"data": {"name": "test"}}"#;
        let req: CollectionInsertRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.source, "rest_api");
        assert!(req.context.is_none());
    }

    #[test]
    fn test_insert_request_custom_source() {
        let json = r#"{"data": {"name": "test"}, "source": "webhook", "context": "HA event"}"#;
        let req: CollectionInsertRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.source, "webhook");
        assert_eq!(req.context.as_deref(), Some("HA event"));
    }

    #[test]
    fn test_update_request_deserialize() {
        let json = r#"{"data": {"end_time": "17:00"}}"#;
        let req: CollectionUpdateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.data["end_time"], "17:00");
    }

    #[test]
    fn test_mutation_response_serialize() {
        let resp = CollectionMutationResponse {
            status: "created".to_string(),
            record_id: "abc-123".to_string(),
            collection: "test".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("created"));
        assert!(json.contains("abc-123"));
    }
}

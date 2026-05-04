//! Pilot invitation API handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
};
use chrono::{Duration, Utc};
use rand::RngCore;
use rand::rngs::OsRng;
use serde::Deserialize;
use uuid::Uuid;

use crate::channels::web::auth::AdminUser;
use crate::channels::web::platform::state::GatewayState;
use crate::channels::web::types::{AdminInvitationCreateResponse, InvitationPreviewResponse};
use crate::db::InvitationRecord;

const DEFAULT_EXPIRY_HOURS: u64 = 72;
const MAX_EXPIRY_HOURS: u64 = 14 * 24;

#[derive(Debug, Deserialize)]
pub struct AdminInvitationCreateRequest {
    pub target_email: Option<String>,
    pub target_role: Option<String>,
    pub scopes: Option<serde_json::Value>,
    pub expires_in_hours: Option<u64>,
}

/// POST /api/admin/invitations — create a one-time pilot invitation.
pub async fn invitations_create_handler(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    AdminUser(admin): AdminUser,
    Json(body): Json<AdminInvitationCreateRequest>,
) -> Result<Json<AdminInvitationCreateResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let target_email = normalize_email(body.target_email)?;
    let target_role = normalize_target_role(body.target_role)?;
    let scopes = normalize_scopes(body.scopes)?;
    let expires_in_hours = body.expires_in_hours.unwrap_or(DEFAULT_EXPIRY_HOURS);
    if expires_in_hours == 0 || expires_in_hours > MAX_EXPIRY_HOURS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("expires_in_hours must be between 1 and {MAX_EXPIRY_HOURS}"),
        ));
    }

    let mut token_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut token_bytes);
    let plaintext_token = hex::encode(token_bytes);
    let token_hash = token_hash_hex(&plaintext_token);
    let token_prefix = plaintext_token[..8].to_string();

    let now = Utc::now();
    let invitation = InvitationRecord {
        id: Uuid::new_v4(),
        token_prefix,
        token_hash,
        created_by_admin: admin.user_id.clone(),
        target_email: target_email.clone(),
        target_role: target_role.clone(),
        scopes,
        expires_at: now + Duration::hours(expires_in_hours as i64),
        claimed_at: None,
        claimed_by_user_id: None,
        revoked_at: None,
        metadata: serde_json::json!({}),
        created_at: now,
    };

    store.create_invitation(&invitation).await.map_err(|e| {
        let msg = e.to_string();
        let lower = msg.to_ascii_lowercase();
        if lower.contains("unique") || lower.contains("duplicate") || lower.contains("constraint") {
            (StatusCode::CONFLICT, msg)
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, msg)
        }
    })?;

    tracing::info!(
        invitation_id = %invitation.id,
        created_by_admin = %invitation.created_by_admin,
        target_email_present = invitation.target_email.is_some(),
        target_role = %invitation.target_role,
        expires_at = %invitation.expires_at.to_rfc3339(),
        "invitation created"
    );

    let invite_url = format!(
        "{}/invite/{}",
        request_base_url(&state, &headers),
        plaintext_token
    );

    Ok(Json(AdminInvitationCreateResponse {
        id: invitation.id.to_string(),
        invite_url,
        target_email,
        target_role,
        expires_at: invitation.expires_at.to_rfc3339(),
    }))
}

/// GET /api/invitations/{token} — preview a pending invitation without claiming it.
pub async fn invitations_preview_handler(
    State(state): State<Arc<GatewayState>>,
    Path(token): Path<String>,
) -> Result<Json<InvitationPreviewResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    if !is_hex_token(&token) {
        return Err((StatusCode::NOT_FOUND, "Invitation not found".to_string()));
    }

    let token_hash = token_hash_hex(&token);
    let invitation = store
        .get_invitation_by_token_hash(&token_hash)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Invitation not found".to_string()))?;

    if invitation.revoked_at.is_some() {
        return Err((StatusCode::GONE, "Invitation revoked".to_string()));
    }
    if invitation.claimed_at.is_some() {
        return Err((StatusCode::GONE, "Invitation already claimed".to_string()));
    }
    if invitation.expires_at <= Utc::now() {
        return Err((StatusCode::GONE, "Invitation expired".to_string()));
    }

    let inviter_handle = match store.get_user(&invitation.created_by_admin).await {
        Ok(Some(user)) => user.display_name,
        Ok(None) => invitation.created_by_admin.clone(),
        Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    };

    Ok(Json(InvitationPreviewResponse {
        org_name: "IronClaw".to_string(),
        inviter_handle,
        target_role: invitation.target_role,
        expires_at: invitation.expires_at.to_rfc3339(),
    }))
}

fn normalize_email(email: Option<String>) -> Result<Option<String>, (StatusCode, String)> {
    let Some(email) = email else {
        return Ok(None);
    };
    let email = email.trim().to_string();
    if email.is_empty() {
        return Ok(None);
    }
    if !email.contains('@') || email.len() < 3 {
        return Err((
            StatusCode::BAD_REQUEST,
            "target_email must be a valid email address".to_string(),
        ));
    }
    Ok(Some(email))
}

fn normalize_target_role(role: Option<String>) -> Result<String, (StatusCode, String)> {
    let role = role
        .as_deref()
        .unwrap_or("user")
        .trim()
        .to_ascii_lowercase();
    match role.as_str() {
        "user" | "regular" | "member" | "admin" => Ok(role),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "target_role must be 'user', 'regular', 'member', or 'admin'".to_string(),
        )),
    }
}

fn normalize_scopes(
    scopes: Option<serde_json::Value>,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let scopes = scopes.unwrap_or_else(|| serde_json::json!({}));
    if !scopes.is_object() {
        return Err((
            StatusCode::BAD_REQUEST,
            "scopes must be a JSON object".to_string(),
        ));
    }
    Ok(scopes)
}

fn request_base_url(state: &GatewayState, headers: &HeaderMap) -> String {
    if let Some(base_url) = state.oauth_base_url.as_deref() {
        return base_url.trim_end_matches('/').to_string();
    }

    let proto = header_value(headers, "x-forwarded-proto").unwrap_or("http");
    let host = header_value(headers, "x-forwarded-host")
        .or_else(|| header_value(headers, header::HOST.as_str()))
        .unwrap_or("localhost");
    format!("{proto}://{host}")
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn token_hash_hex(token: &str) -> String {
    hex::encode(crate::channels::web::auth::hash_token(token))
}

fn is_hex_token(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        Router,
        body::Body,
        http::{Method, Request, StatusCode},
        middleware,
        routing::{get, post},
    };
    use tower::ServiceExt;

    use super::*;
    use crate::channels::web::auth::{MultiAuthState, UserIdentity, auth_middleware};
    use crate::channels::web::test_helpers::{
        insert_test_user, test_gateway_state_with_dependencies,
    };

    fn auth() -> MultiAuthState {
        let mut tokens = std::collections::HashMap::new();
        tokens.insert(
            "admin-token".to_string(),
            UserIdentity {
                user_id: "admin-1".to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: vec![],
            },
        );
        tokens.insert(
            "member-token".to_string(),
            UserIdentity {
                user_id: "member-1".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: vec![],
            },
        );
        MultiAuthState::multi(tokens)
    }

    fn app(state: Arc<GatewayState>) -> Router {
        let protected = Router::new()
            .route("/api/admin/invitations", post(invitations_create_handler))
            .route_layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth()),
                auth_middleware,
            ));

        Router::new()
            .route("/api/invitations/{token}", get(invitations_preview_handler))
            .merge(protected)
            .with_state(state)
    }

    async fn parse_json<T: serde::de::DeserializeOwned>(resp: axum::response::Response) -> T {
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn invitation_record(token: &str, expires_at: chrono::DateTime<Utc>) -> InvitationRecord {
        let now = Utc::now();
        InvitationRecord {
            id: Uuid::new_v4(),
            token_prefix: token[..8].to_string(),
            token_hash: token_hash_hex(token),
            created_by_admin: "admin-1".to_string(),
            target_email: Some("pilot@example.com".to_string()),
            target_role: "user".to_string(),
            scopes: serde_json::json!({}),
            expires_at,
            claimed_at: None,
            claimed_by_user_id: None,
            revoked_at: None,
            metadata: serde_json::json!({}),
            created_at: now,
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn admin_post_creates_invitation_and_returns_one_time_url() {
        let (db, _tmp) = crate::testing::test_db().await;
        insert_test_user(&db, "admin-1", "admin").await;
        let state = test_gateway_state_with_dependencies(None, Some(Arc::clone(&db)), None, None);
        let app = app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/admin/invitations")
                    .header("Authorization", "Bearer admin-token")
                    .header("Content-Type", "application/json")
                    .header("Host", "internal.local")
                    .header("X-Forwarded-Proto", "https")
                    .header("X-Forwarded-Host", "pilots.example.com")
                    .body(Body::from(
                        r#"{"target_email":"pilot@example.com","target_role":"user","scopes":{"oauth":"limited"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: AdminInvitationCreateResponse = parse_json(resp).await;
        assert!(
            body.invite_url
                .starts_with("https://pilots.example.com/invite/")
        );

        let token = body.invite_url.rsplit('/').next().unwrap();
        assert!(is_hex_token(token));
        let hash = token_hash_hex(token);
        let stored = db
            .get_invitation_by_token_hash(&hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.token_hash, hash);
        assert_eq!(stored.token_prefix, token[..8]);
        assert_eq!(stored.target_email.as_deref(), Some("pilot@example.com"));
        assert_eq!(stored.scopes, serde_json::json!({"oauth":"limited"}));
        assert!(
            db.get_invitation_by_token_hash(token)
                .await
                .unwrap()
                .is_none(),
            "plaintext token must not be stored as the lookup key"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn preview_pending_invitation_does_not_claim_it() {
        let (db, _tmp) = crate::testing::test_db().await;
        insert_test_user(&db, "admin-1", "admin").await;
        let token = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let invitation = invitation_record(token, Utc::now() + Duration::hours(1));
        db.create_invitation(&invitation).await.unwrap();
        let state = test_gateway_state_with_dependencies(None, Some(Arc::clone(&db)), None, None);
        let app = app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/invitations/{token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: InvitationPreviewResponse = parse_json(resp).await;
        assert_eq!(body.org_name, "IronClaw");
        assert_eq!(body.inviter_handle, "admin-1");
        assert_eq!(body.target_role, "user");

        let stored = db
            .get_invitation_by_token_hash(&token_hash_hex(token))
            .await
            .unwrap()
            .unwrap();
        assert!(stored.claimed_at.is_none());
        assert!(stored.claimed_by_user_id.is_none());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn preview_expired_invitation_returns_gone() {
        let (db, _tmp) = crate::testing::test_db().await;
        insert_test_user(&db, "admin-1", "admin").await;
        let token = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let invitation = invitation_record(token, Utc::now() - Duration::hours(1));
        db.create_invitation(&invitation).await.unwrap();
        let state = test_gateway_state_with_dependencies(None, Some(Arc::clone(&db)), None, None);
        let app = app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/api/invitations/{token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::GONE);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn non_admin_cannot_create_invitation() {
        let (db, _tmp) = crate::testing::test_db().await;
        insert_test_user(&db, "member-1", "member").await;
        let state = test_gateway_state_with_dependencies(None, Some(db), None, None);
        let app = app(state);

        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/admin/invitations")
                    .header("Authorization", "Bearer member-token")
                    .header("Content-Type", "application/json")
                    .body(Body::from(r#"{"target_email":"pilot@example.com"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}

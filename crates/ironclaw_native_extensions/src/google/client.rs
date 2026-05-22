use std::sync::Arc;

use ironclaw_host_api::{NetworkMethod, ResourceScope};
use ironclaw_network::{NetworkHttpEgress, NetworkHttpRequest, NetworkHttpResponse};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

use crate::google::network::google_api_network_policy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoogleApiErrorKind {
    Network,
    Unauthorized,
    InsufficientScope,
    RefreshRequired,
    InvalidGrant,
    HttpStatus,
    InvalidJson,
}

#[derive(Debug, Error)]
#[error("Google API request failed: {kind:?}")]
pub struct GoogleApiError {
    pub kind: GoogleApiErrorKind,
    pub status: Option<u16>,
}

impl GoogleApiError {
    pub fn should_refresh(&self) -> bool {
        matches!(
            self.kind,
            GoogleApiErrorKind::Unauthorized | GoogleApiErrorKind::RefreshRequired
        )
    }

    pub fn requires_auth_prompt(&self) -> bool {
        matches!(
            self.kind,
            GoogleApiErrorKind::InsufficientScope | GoogleApiErrorKind::InvalidGrant
        )
    }
}

#[derive(Clone)]
pub struct GoogleHttpClient {
    egress: Arc<dyn NetworkHttpEgress>,
}

impl GoogleHttpClient {
    pub fn new(egress: Arc<dyn NetworkHttpEgress>) -> Self {
        Self { egress }
    }

    pub async fn get_json(
        &self,
        scope: &ResourceScope,
        url: impl Into<String>,
        access_token: &SecretString,
    ) -> Result<serde_json::Value, GoogleApiError> {
        self.request_json(
            scope,
            NetworkMethod::Get,
            url.into(),
            Vec::new(),
            access_token,
        )
        .await
    }

    pub async fn post_json(
        &self,
        scope: &ResourceScope,
        url: impl Into<String>,
        body: serde_json::Value,
        access_token: &SecretString,
    ) -> Result<serde_json::Value, GoogleApiError> {
        let body = serde_json::to_vec(&body).map_err(|_| GoogleApiError {
            kind: GoogleApiErrorKind::InvalidJson,
            status: None,
        })?;
        self.request_json(scope, NetworkMethod::Post, url.into(), body, access_token)
            .await
    }

    async fn request_json(
        &self,
        scope: &ResourceScope,
        method: NetworkMethod,
        url: String,
        body: Vec<u8>,
        access_token: &SecretString,
    ) -> Result<serde_json::Value, GoogleApiError> {
        let response = self
            .egress
            .execute(NetworkHttpRequest {
                scope: scope.clone(),
                method,
                url,
                headers: vec![
                    (
                        "authorization".to_string(),
                        format!("Bearer {}", access_token.expose_secret()),
                    ),
                    ("content-type".to_string(), "application/json".to_string()),
                ],
                body,
                policy: google_api_network_policy(),
                response_body_limit: Some(1024 * 1024),
                timeout_ms: Some(30_000),
            })
            .map_err(|_| GoogleApiError {
                kind: GoogleApiErrorKind::Network,
                status: None,
            })?;
        map_google_response(response)
    }
}

pub fn map_google_response(
    response: NetworkHttpResponse,
) -> Result<serde_json::Value, GoogleApiError> {
    if (200..300).contains(&response.status) {
        return serde_json::from_slice(&response.body).map_err(|_| GoogleApiError {
            kind: GoogleApiErrorKind::InvalidJson,
            status: Some(response.status),
        });
    }
    let kind = google_error_kind(response.status, &response.body);
    Err(GoogleApiError {
        kind,
        status: Some(response.status),
    })
}

fn google_error_kind(status: u16, body: &[u8]) -> GoogleApiErrorKind {
    let value = serde_json::from_slice::<serde_json::Value>(body).ok();
    let reasons = value.as_ref().map(google_error_reasons).unwrap_or_default();
    let has_reason = |expected: &str| reasons.iter().any(|reason| reason == expected);
    match status {
        _ if has_reason("invalid_grant") => GoogleApiErrorKind::InvalidGrant,
        _ if has_reason("invalid_token") => GoogleApiErrorKind::RefreshRequired,
        401 => GoogleApiErrorKind::Unauthorized,
        403 if has_reason("insufficient_scope") => GoogleApiErrorKind::InsufficientScope,
        _ => GoogleApiErrorKind::HttpStatus,
    }
}

fn google_error_reasons(value: &serde_json::Value) -> Vec<String> {
    let mut reasons = Vec::new();
    push_reason(&mut reasons, value.pointer("/error"));
    push_reason(&mut reasons, value.pointer("/error/status"));
    push_reason(&mut reasons, value.pointer("/error/reason"));
    push_reason(&mut reasons, value.pointer("/error/errorCode"));
    push_array_reasons(&mut reasons, value.pointer("/error/errors"));
    push_array_reasons(&mut reasons, value.pointer("/error/details"));
    reasons
}

fn push_array_reasons(reasons: &mut Vec<String>, value: Option<&serde_json::Value>) {
    if let Some(values) = value.and_then(serde_json::Value::as_array) {
        for value in values {
            push_reason(reasons, value.get("reason"));
            push_reason(reasons, value.get("errorCode"));
            push_reason(reasons, value.get("status"));
        }
    }
}

fn push_reason(reasons: &mut Vec<String>, value: Option<&serde_json::Value>) {
    if let Some(reason) = value.and_then(serde_json::Value::as_str) {
        reasons.push(reason.to_ascii_lowercase());
    }
}

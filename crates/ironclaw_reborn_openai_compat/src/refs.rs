use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_product_adapters::ProductInboundAck;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const CHAT_COMPLETION_PREFIX: &str = "chatcmpl-";
const RESPONSE_PREFIX: &str = "resp_";
const MAX_PUBLIC_REF_BYTES: usize = 96;
const MAX_INTERNAL_REF_BYTES: usize = 256;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;
const DEFAULT_IN_MEMORY_REF_CAPACITY: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OpenAiCompatRefError {
    #[error("invalid {kind}: {reason}")]
    InvalidIdentifier {
        kind: &'static str,
        reason: &'static str,
    },
    #[error("OpenAI-compatible ref store is unavailable")]
    StoreUnavailable,
    #[error("OpenAI-compatible ref store mapping is inconsistent")]
    CorruptMapping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiCompatResourceKind {
    ChatCompletion,
    Response,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiCompatRouteSurface {
    ChatCompletions,
    ResponsesApi,
    ResponsesV1,
}

impl OpenAiCompatRouteSurface {
    pub fn resource_kind(self) -> OpenAiCompatResourceKind {
        match self {
            Self::ChatCompletions => OpenAiCompatResourceKind::ChatCompletion,
            Self::ResponsesApi | Self::ResponsesV1 => OpenAiCompatResourceKind::Response,
        }
    }
}

macro_rules! public_ref {
    ($name:ident, $prefix:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, OpenAiCompatRefError> {
                let value = value.into();
                validate_public_ref($kind, &value, $prefix)?;
                Ok(Self(value))
            }

            pub fn generate() -> Self {
                let suffix = Uuid::new_v4().simple().to_string();
                Self(format!("{}{}", $prefix, suffix))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = OpenAiCompatRefError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

public_ref!(
    OpenAiChatCompletionId,
    CHAT_COMPLETION_PREFIX,
    "chat_completion_id"
);
public_ref!(OpenAiResponseId, RESPONSE_PREFIX, "response_id");

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum OpenAiCompatPublicId {
    ChatCompletion(OpenAiChatCompletionId),
    Response(OpenAiResponseId),
}

impl OpenAiCompatPublicId {
    pub fn generate_for(surface: OpenAiCompatRouteSurface) -> Self {
        match surface.resource_kind() {
            OpenAiCompatResourceKind::ChatCompletion => {
                Self::ChatCompletion(OpenAiChatCompletionId::generate())
            }
            OpenAiCompatResourceKind::Response => Self::Response(OpenAiResponseId::generate()),
        }
    }

    pub fn resource_kind(&self) -> OpenAiCompatResourceKind {
        match self {
            Self::ChatCompletion(_) => OpenAiCompatResourceKind::ChatCompletion,
            Self::Response(_) => OpenAiCompatResourceKind::Response,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::ChatCompletion(id) => id.as_str(),
            Self::Response(id) => id.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct OpenAiCompatIdempotencyKey(String);

impl OpenAiCompatIdempotencyKey {
    pub fn new(value: impl Into<String>) -> Result<Self, OpenAiCompatRefError> {
        let value = value.into();
        validate_bounded_clean_ref("idempotency_key", &value, MAX_IDEMPOTENCY_KEY_BYTES, false)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for OpenAiCompatIdempotencyKey {
    type Error = OpenAiCompatRefError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct OpenAiCompatRequestFingerprint(String);

impl OpenAiCompatRequestFingerprint {
    pub fn from_body_bytes(body: &[u8]) -> Self {
        let digest = Sha256::digest(body);
        Self(format!("sha256:{}", hex::encode(digest)))
    }

    pub fn from_json(value: &impl Serialize) -> Result<Self, OpenAiCompatRefError> {
        let bytes =
            serde_json::to_vec(value).map_err(|_| OpenAiCompatRefError::InvalidIdentifier {
                kind: "request_fingerprint",
                reason: "request body is not serializable",
            })?;
        Ok(Self::from_body_bytes(&bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for OpenAiCompatRequestFingerprint {
    type Error = OpenAiCompatRefError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_fingerprint(&value)?;
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiCompatActorScope {
    tenant_id: TenantId,
    user_id: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_id: Option<ProjectId>,
}

impl OpenAiCompatActorScope {
    pub fn new(
        tenant_id: TenantId,
        user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
    ) -> Self {
        Self {
            tenant_id,
            user_id,
            agent_id,
            project_id,
        }
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn user_id(&self) -> &UserId {
        &self.user_id
    }

    pub fn agent_id(&self) -> Option<&AgentId> {
        self.agent_id.as_ref()
    }

    pub fn project_id(&self) -> Option<&ProjectId> {
        self.project_id.as_ref()
    }
}

macro_rules! internal_ref {
    ($name:ident, $kind:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, OpenAiCompatRefError> {
                let value = value.into();
                validate_bounded_clean_ref($kind, &value, MAX_INTERNAL_REF_BYTES, true)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = OpenAiCompatRefError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }
    };
}

internal_ref!(OpenAiCompatProductActionRef, "product_action_ref");
internal_ref!(OpenAiCompatTurnRunRef, "turn_run_ref");
internal_ref!(OpenAiCompatProjectionRef, "projection_ref");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiCompatInternalRefs {
    pub product_action_ref: OpenAiCompatProductActionRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_run_ref: Option<OpenAiCompatTurnRunRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_ref: Option<OpenAiCompatProjectionRef>,
}

impl OpenAiCompatInternalRefs {
    pub fn new(product_action_ref: OpenAiCompatProductActionRef) -> Self {
        Self {
            product_action_ref,
            turn_run_ref: None,
            projection_ref: None,
        }
    }

    pub fn with_turn_run_ref(mut self, turn_run_ref: OpenAiCompatTurnRunRef) -> Self {
        self.turn_run_ref = Some(turn_run_ref);
        self
    }

    pub fn with_projection_ref(mut self, projection_ref: OpenAiCompatProjectionRef) -> Self {
        self.projection_ref = Some(projection_ref);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OpenAiCompatResourceBinding {
    Pending,
    Bound {
        internal_refs: OpenAiCompatInternalRefs,
    },
}

impl OpenAiCompatResourceBinding {
    pub fn internal_refs(&self) -> Option<&OpenAiCompatInternalRefs> {
        match self {
            Self::Pending => None,
            Self::Bound { internal_refs } => Some(internal_refs),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiCompatResourceMapping {
    pub public_id: OpenAiCompatPublicId,
    pub owner: OpenAiCompatActorScope,
    pub surface: OpenAiCompatRouteSurface,
    pub request_fingerprint: OpenAiCompatRequestFingerprint,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<OpenAiCompatIdempotencyKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_ack: Option<ProductInboundAck>,
    pub binding: OpenAiCompatResourceBinding,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenAiCompatResourceMappingFields {
    public_id: OpenAiCompatPublicId,
    owner: OpenAiCompatActorScope,
    surface: OpenAiCompatRouteSurface,
    request_fingerprint: OpenAiCompatRequestFingerprint,
    #[serde(default)]
    created_at: Option<u64>,
    idempotency_key: Option<OpenAiCompatIdempotencyKey>,
    #[serde(default)]
    accepted_ack: Option<ProductInboundAck>,
    binding: OpenAiCompatResourceBinding,
}

impl<'de> Deserialize<'de> for OpenAiCompatResourceMapping {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = OpenAiCompatResourceMappingFields::deserialize(deserializer)?;
        let mapping = Self {
            public_id: fields.public_id,
            owner: fields.owner,
            surface: fields.surface,
            request_fingerprint: fields.request_fingerprint,
            created_at: fields.created_at.unwrap_or_else(unix_timestamp_now),
            idempotency_key: fields.idempotency_key,
            accepted_ack: fields.accepted_ack,
            binding: fields.binding,
        };
        mapping.validate().map_err(serde::de::Error::custom)?;
        Ok(mapping)
    }
}

impl OpenAiCompatResourceMapping {
    pub fn resource_kind(&self) -> OpenAiCompatResourceKind {
        self.public_id.resource_kind()
    }

    pub fn validate(&self) -> Result<(), OpenAiCompatRefError> {
        if self.public_id.resource_kind() != self.surface.resource_kind() {
            return Err(OpenAiCompatRefError::CorruptMapping);
        }
        Ok(())
    }

    pub fn is_authorized_for(&self, scope: &OpenAiCompatActorScope) -> bool {
        &self.owner == scope
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiCompatRefReservation {
    pub owner: OpenAiCompatActorScope,
    pub surface: OpenAiCompatRouteSurface,
    pub request_fingerprint: OpenAiCompatRequestFingerprint,
    pub idempotency_key: Option<OpenAiCompatIdempotencyKey>,
}

impl OpenAiCompatRefReservation {
    pub fn new(
        owner: OpenAiCompatActorScope,
        surface: OpenAiCompatRouteSurface,
        request_fingerprint: OpenAiCompatRequestFingerprint,
        idempotency_key: Option<OpenAiCompatIdempotencyKey>,
    ) -> Self {
        Self {
            owner,
            surface,
            request_fingerprint,
            idempotency_key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenAiCompatRefReservationOutcome {
    Created(OpenAiCompatResourceMapping),
    Replayed(OpenAiCompatResourceMapping),
    Conflict(OpenAiCompatIdempotencyConflict),
}

impl OpenAiCompatRefReservationOutcome {
    pub fn mapping(&self) -> Option<&OpenAiCompatResourceMapping> {
        match self {
            Self::Created(mapping) | Self::Replayed(mapping) => Some(mapping),
            Self::Conflict(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiCompatIdempotencyConflict {
    pub surface: OpenAiCompatRouteSurface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiCompatRefOperation {
    Retrieve,
    StreamResume,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiCompatRefLookup {
    pub requester: OpenAiCompatActorScope,
    pub public_id: OpenAiCompatPublicId,
    /// Caller intent for future operation-specific policy/audit. Current lookup
    /// authorization is owner-scoped and intentionally identical for all values.
    pub operation: OpenAiCompatRefOperation,
}

impl OpenAiCompatRefLookup {
    pub fn new(
        requester: OpenAiCompatActorScope,
        public_id: OpenAiCompatPublicId,
        operation: OpenAiCompatRefOperation,
    ) -> Self {
        Self {
            requester,
            public_id,
            operation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiCompatBindInternalRefs {
    pub owner: OpenAiCompatActorScope,
    pub public_id: OpenAiCompatPublicId,
    pub internal_refs: OpenAiCompatInternalRefs,
}

impl OpenAiCompatBindInternalRefs {
    pub fn new(
        owner: OpenAiCompatActorScope,
        public_id: OpenAiCompatPublicId,
        internal_refs: OpenAiCompatInternalRefs,
    ) -> Self {
        Self {
            owner,
            public_id,
            internal_refs,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiCompatRecordAcceptedAck {
    pub owner: OpenAiCompatActorScope,
    pub public_id: OpenAiCompatPublicId,
    pub accepted_ack: ProductInboundAck,
}

impl OpenAiCompatRecordAcceptedAck {
    pub fn new(
        owner: OpenAiCompatActorScope,
        public_id: OpenAiCompatPublicId,
        accepted_ack: ProductInboundAck,
    ) -> Self {
        Self {
            owner,
            public_id,
            accepted_ack,
        }
    }
}

#[async_trait]
pub trait OpenAiCompatRefStore: Send + Sync {
    async fn reserve(
        &self,
        request: OpenAiCompatRefReservation,
    ) -> Result<OpenAiCompatRefReservationOutcome, OpenAiCompatRefError>;

    async fn bind_internal_refs(
        &self,
        request: OpenAiCompatBindInternalRefs,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError>;

    async fn record_accepted_ack(
        &self,
        request: OpenAiCompatRecordAcceptedAck,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError>;

    async fn lookup_authorized(
        &self,
        request: OpenAiCompatRefLookup,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError>;
}

#[derive(Clone)]
pub struct InMemoryOpenAiCompatRefStore {
    state: Arc<Mutex<InMemoryOpenAiCompatRefState>>,
    max_mappings: usize,
}

#[derive(Default)]
struct InMemoryOpenAiCompatRefState {
    by_public_id: HashMap<OpenAiCompatPublicId, OpenAiCompatResourceMapping>,
    by_idempotency: HashMap<IdempotencyIndexKey, OpenAiCompatPublicId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IdempotencyIndexKey {
    owner: OpenAiCompatActorScope,
    surface: OpenAiCompatRouteSurface,
    key: OpenAiCompatIdempotencyKey,
}

impl Default for InMemoryOpenAiCompatRefStore {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryOpenAiCompatRefStore {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_IN_MEMORY_REF_CAPACITY)
    }

    pub fn with_capacity(max_mappings: usize) -> Self {
        Self {
            state: Arc::new(Mutex::new(InMemoryOpenAiCompatRefState::default())),
            max_mappings: max_mappings.max(1),
        }
    }

    async fn lock_state(&self) -> MutexGuard<'_, InMemoryOpenAiCompatRefState> {
        self.state.lock().await
    }
}

#[async_trait]
impl OpenAiCompatRefStore for InMemoryOpenAiCompatRefStore {
    async fn reserve(
        &self,
        request: OpenAiCompatRefReservation,
    ) -> Result<OpenAiCompatRefReservationOutcome, OpenAiCompatRefError> {
        let mut state = self.lock_state().await;
        evict_oldest_if_needed(&mut state, self.max_mappings);
        if let Some(key) = request.idempotency_key.clone() {
            let index = IdempotencyIndexKey {
                owner: request.owner.clone(),
                surface: request.surface,
                key,
            };
            if let Some(public_id) = state.by_idempotency.get(&index) {
                let mapping = state
                    .by_public_id
                    .get(public_id)
                    .cloned()
                    .ok_or(OpenAiCompatRefError::CorruptMapping)?;
                mapping.validate()?;
                if mapping.request_fingerprint == request.request_fingerprint {
                    return Ok(OpenAiCompatRefReservationOutcome::Replayed(mapping));
                }
                return Ok(OpenAiCompatRefReservationOutcome::Conflict(
                    OpenAiCompatIdempotencyConflict {
                        surface: request.surface,
                    },
                ));
            }

            let mapping = new_pending_mapping(request);
            state
                .by_idempotency
                .insert(index, mapping.public_id.clone());
            state
                .by_public_id
                .insert(mapping.public_id.clone(), mapping.clone());
            return Ok(OpenAiCompatRefReservationOutcome::Created(mapping));
        }

        let mapping = new_pending_mapping(request);
        state
            .by_public_id
            .insert(mapping.public_id.clone(), mapping.clone());
        Ok(OpenAiCompatRefReservationOutcome::Created(mapping))
    }

    async fn bind_internal_refs(
        &self,
        request: OpenAiCompatBindInternalRefs,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        let mut state = self.lock_state().await;
        let Some(mapping) = state.by_public_id.get_mut(&request.public_id) else {
            return Ok(None);
        };
        mapping.validate()?;
        if !mapping.is_authorized_for(&request.owner) {
            return Ok(None);
        }
        mapping.binding = OpenAiCompatResourceBinding::Bound {
            internal_refs: request.internal_refs,
        };
        Ok(Some(mapping.clone()))
    }

    async fn record_accepted_ack(
        &self,
        request: OpenAiCompatRecordAcceptedAck,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        let mut state = self.lock_state().await;
        let Some(mapping) = state.by_public_id.get_mut(&request.public_id) else {
            return Ok(None);
        };
        mapping.validate()?;
        if !mapping.is_authorized_for(&request.owner) {
            return Ok(None);
        }
        mapping.accepted_ack = Some(request.accepted_ack);
        Ok(Some(mapping.clone()))
    }

    async fn lookup_authorized(
        &self,
        request: OpenAiCompatRefLookup,
    ) -> Result<Option<OpenAiCompatResourceMapping>, OpenAiCompatRefError> {
        let state = self.lock_state().await;
        let Some(mapping) = state.by_public_id.get(&request.public_id) else {
            return Ok(None);
        };
        mapping.validate()?;
        if !mapping.is_authorized_for(&request.requester) {
            return Ok(None);
        }
        Ok(Some(mapping.clone()))
    }
}

fn new_pending_mapping(request: OpenAiCompatRefReservation) -> OpenAiCompatResourceMapping {
    let mapping = OpenAiCompatResourceMapping {
        public_id: OpenAiCompatPublicId::generate_for(request.surface),
        owner: request.owner,
        surface: request.surface,
        request_fingerprint: request.request_fingerprint,
        created_at: unix_timestamp_now(),
        idempotency_key: request.idempotency_key,
        accepted_ack: None,
        binding: OpenAiCompatResourceBinding::Pending,
    };
    debug_assert!(mapping.validate().is_ok());
    mapping
}

pub fn unix_timestamp_now() -> u64 {
    Utc::now().timestamp().try_into().unwrap_or(0)
}

fn evict_oldest_if_needed(state: &mut InMemoryOpenAiCompatRefState, max_mappings: usize) {
    if state.by_public_id.len() < max_mappings {
        return;
    }
    let Some(public_id) = state
        .by_public_id
        .iter()
        .min_by_key(|(_, mapping)| mapping.created_at)
        .map(|(public_id, _)| public_id.clone())
    else {
        return;
    };
    state.by_public_id.remove(&public_id);
    state
        .by_idempotency
        .retain(|_, mapped_public_id| mapped_public_id != &public_id);
}

fn validate_public_ref(
    kind: &'static str,
    value: &str,
    prefix: &'static str,
) -> Result<(), OpenAiCompatRefError> {
    validate_bounded_clean_ref(kind, value, MAX_PUBLIC_REF_BYTES, false)?;
    let Some(suffix) = value.strip_prefix(prefix) else {
        return Err(OpenAiCompatRefError::InvalidIdentifier {
            kind,
            reason: "must use the expected OpenAI-compatible prefix",
        });
    };
    if suffix.is_empty() {
        return Err(OpenAiCompatRefError::InvalidIdentifier {
            kind,
            reason: "must include an opaque suffix",
        });
    }
    if suffix
        .bytes()
        .any(|byte| !byte.is_ascii_alphanumeric() && byte != b'_' && byte != b'-')
    {
        return Err(OpenAiCompatRefError::InvalidIdentifier {
            kind,
            reason: "opaque suffix must contain only ASCII letters, digits, '_', or '-'",
        });
    }
    Ok(())
}

/// Validates a reference string against byte-length and cleanliness limits.
///
/// The length limit is checked in bytes with `str::len()` because these refs
/// are ASCII-shaped identifiers rather than user-visible text.
fn validate_bounded_clean_ref(
    kind: &'static str,
    value: &str,
    max_bytes: usize,
    allow_colon: bool,
) -> Result<(), OpenAiCompatRefError> {
    if value.is_empty() {
        return Err(OpenAiCompatRefError::InvalidIdentifier {
            kind,
            reason: "must not be empty",
        });
    }
    if value.trim() != value {
        return Err(OpenAiCompatRefError::InvalidIdentifier {
            kind,
            reason: "must not contain leading or trailing whitespace",
        });
    }
    if value.len() > max_bytes {
        return Err(OpenAiCompatRefError::InvalidIdentifier {
            kind,
            reason: "is too long",
        });
    }
    if value.chars().any(|ch| ch == '\0' || ch.is_control()) {
        return Err(OpenAiCompatRefError::InvalidIdentifier {
            kind,
            reason: "must not contain NUL/control characters",
        });
    }
    if value.contains('/') || value.contains('\\') {
        return Err(OpenAiCompatRefError::InvalidIdentifier {
            kind,
            reason: "must not contain path separators",
        });
    }
    if !allow_colon && value.contains(':') {
        return Err(OpenAiCompatRefError::InvalidIdentifier {
            kind,
            reason: "must not contain ':'",
        });
    }
    if contains_no_exposure_sentinel(value) {
        return Err(OpenAiCompatRefError::InvalidIdentifier {
            kind,
            reason: "must not contain no-exposure sentinels",
        });
    }
    Ok(())
}

fn validate_fingerprint(value: &str) -> Result<(), OpenAiCompatRefError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(OpenAiCompatRefError::InvalidIdentifier {
            kind: "request_fingerprint",
            reason: "must use sha256 prefix",
        });
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(OpenAiCompatRefError::InvalidIdentifier {
            kind: "request_fingerprint",
            reason: "must be a SHA-256 hex digest",
        });
    }
    Ok(())
}

fn contains_no_exposure_sentinel(value: &str) -> bool {
    const NO_EXPOSURE_SENTINELS: &[&str] = &[
        "RAW_PROMPT_SENTINEL",
        "SECRET_SENTINEL",
        "secret-token",
        "sk-live",
        "/host/path",
        "/Users/",
    ];
    NO_EXPOSURE_SENTINELS
        .iter()
        .any(|sentinel| value.contains(sentinel))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_turns::{AcceptedMessageRef, TurnRunId};

    fn scope(user: &str) -> OpenAiCompatActorScope {
        OpenAiCompatActorScope::new(
            TenantId::new("tenant-a").expect("tenant"),
            UserId::new(user).expect("user"),
            None,
            None,
        )
    }

    fn fingerprint(label: &str) -> OpenAiCompatRequestFingerprint {
        OpenAiCompatRequestFingerprint::from_body_bytes(label.as_bytes())
    }

    fn idempotency_key() -> OpenAiCompatIdempotencyKey {
        OpenAiCompatIdempotencyKey::new("same-key").expect("key")
    }

    fn accepted_ack() -> ProductInboundAck {
        ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("msg:test").expect("message ref"),
            submitted_run_id: TurnRunId::new(),
        }
    }

    #[tokio::test]
    async fn in_memory_store_record_accepted_ack_wrong_owner_is_none() {
        let store = InMemoryOpenAiCompatRefStore::new();
        let alice = scope("alice");
        let bob = scope("bob");
        let created = store
            .reserve(OpenAiCompatRefReservation::new(
                alice.clone(),
                OpenAiCompatRouteSurface::ChatCompletions,
                fingerprint("body"),
                Some(idempotency_key()),
            ))
            .await
            .expect("reserve");
        let mapping = created.mapping().expect("created mapping").clone();

        let wrong_owner = store
            .record_accepted_ack(OpenAiCompatRecordAcceptedAck::new(
                bob,
                mapping.public_id.clone(),
                accepted_ack(),
            ))
            .await
            .expect("record ack");
        assert!(wrong_owner.is_none());

        let alice_lookup = store
            .lookup_authorized(OpenAiCompatRefLookup::new(
                alice,
                mapping.public_id,
                OpenAiCompatRefOperation::Retrieve,
            ))
            .await
            .expect("lookup")
            .expect("alice mapping");
        assert!(alice_lookup.accepted_ack.is_none());
    }

    #[tokio::test]
    async fn in_memory_store_same_key_distinct_actors_create_distinct_refs() {
        let store = InMemoryOpenAiCompatRefStore::new();
        let first = store
            .reserve(OpenAiCompatRefReservation::new(
                scope("alice"),
                OpenAiCompatRouteSurface::ChatCompletions,
                fingerprint("body"),
                Some(idempotency_key()),
            ))
            .await
            .expect("first reserve");
        let second = store
            .reserve(OpenAiCompatRefReservation::new(
                scope("bob"),
                OpenAiCompatRouteSurface::ChatCompletions,
                fingerprint("body"),
                Some(idempotency_key()),
            ))
            .await
            .expect("second reserve");

        let first_id = first.mapping().expect("first mapping").public_id.clone();
        let second_id = second.mapping().expect("second mapping").public_id.clone();
        assert_ne!(first_id, second_id);
    }

    #[tokio::test]
    async fn in_memory_store_evicts_oldest_mapping_when_capacity_is_reached() {
        let store = InMemoryOpenAiCompatRefStore::with_capacity(1);
        let owner = scope("alice");
        let first = store
            .reserve(OpenAiCompatRefReservation::new(
                owner.clone(),
                OpenAiCompatRouteSurface::ChatCompletions,
                fingerprint("one"),
                Some(OpenAiCompatIdempotencyKey::new("key-one").expect("key")),
            ))
            .await
            .expect("first reserve")
            .mapping()
            .expect("first mapping")
            .public_id
            .clone();
        let second = store
            .reserve(OpenAiCompatRefReservation::new(
                owner.clone(),
                OpenAiCompatRouteSurface::ChatCompletions,
                fingerprint("two"),
                Some(OpenAiCompatIdempotencyKey::new("key-two").expect("key")),
            ))
            .await
            .expect("second reserve")
            .mapping()
            .expect("second mapping")
            .public_id
            .clone();

        assert!(
            store
                .lookup_authorized(OpenAiCompatRefLookup::new(
                    owner.clone(),
                    first,
                    OpenAiCompatRefOperation::Retrieve,
                ))
                .await
                .expect("lookup first")
                .is_none()
        );
        assert!(
            store
                .lookup_authorized(OpenAiCompatRefLookup::new(
                    owner,
                    second,
                    OpenAiCompatRefOperation::Retrieve,
                ))
                .await
                .expect("lookup second")
                .is_some()
        );
    }
}

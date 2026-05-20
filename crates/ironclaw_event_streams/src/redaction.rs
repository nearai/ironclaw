use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, Weak},
};

use ironclaw_host_api::sha256_digest_token;

use crate::{
    error::ProjectionStreamError,
    keys::{ProjectionScopeKey, projection_scope_key},
    types::ProductProjectionEnvelope,
};

pub trait ProjectionRedactionValidator: Send + Sync {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError>;
}

#[derive(Default)]
pub struct NoExposureProjectionRedactionValidator;

impl ProjectionRedactionValidator for NoExposureProjectionRedactionValidator {
    fn validate(&self, envelope: &ProductProjectionEnvelope) -> Result<(), ProjectionStreamError> {
        let rendered =
            serde_json::to_string(envelope).map_err(|_| ProjectionStreamError::Source)?;
        if NO_EXPOSURE_SENTINELS
            .iter()
            .any(|sentinel| rendered.contains(sentinel))
        {
            return Err(ProjectionStreamError::Redaction);
        }
        Ok(())
    }
}

const NO_EXPOSURE_SENTINELS: &[&str] = &[
    "RAW_PROMPT_SENTINEL",
    "TOOL_INPUT_SENTINEL",
    "TOOL_OUTPUT_SENTINEL",
    "SECRET_SENTINEL",
    "HOST_PATH_SENTINEL",
    "RAW_RUNTIME_OUTPUT_SENTINEL",
    "BACKEND_DIAGNOSTIC_SENTINEL",
    "RAW_PROVIDER_ERROR_SENTINEL",
    "INVOCATION_FINGERPRINT_SENTINEL",
    "APPROVAL_REASON_SENTINEL",
    "LEASE_MATERIAL_SENTINEL",
];

const MAX_VALIDATION_CACHE_ENTRIES: usize = 1024;

#[derive(Clone, Default)]
pub(crate) struct ProjectionValidationCache {
    allowed: Arc<Mutex<HashSet<ProjectionValidationCacheKey>>>,
    live_allowed:
        Arc<Mutex<HashMap<ProjectionValidationLiveCacheKey, Weak<ProductProjectionEnvelope>>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProjectionValidationCacheKey {
    variant: ProjectionEnvelopeKind,
    scope: ProjectionScopeKey,
    cursor: u64,
    payload_len: usize,
    payload_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProjectionValidationLiveCacheKey {
    variant: ProjectionEnvelopeKind,
    scope: ProjectionScopeKey,
    cursor: u64,
    pointer: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ProjectionEnvelopeKind {
    ThreadSnapshot,
    ThreadUpdates,
    DeliveryStatus,
    Debug,
}

impl ProjectionValidationCache {
    pub(crate) fn validate(
        &self,
        validator: &dyn ProjectionRedactionValidator,
        envelope: &ProductProjectionEnvelope,
    ) -> Result<(), ProjectionStreamError> {
        let key = validation_cache_key(envelope)?;
        if self
            .allowed
            .lock()
            .map_err(|_| ProjectionStreamError::Source)?
            .contains(&key)
        {
            return Ok(());
        }

        validator.validate(envelope)?;
        let mut allowed = self
            .allowed
            .lock()
            .map_err(|_| ProjectionStreamError::Source)?;
        if allowed.len() >= MAX_VALIDATION_CACHE_ENTRIES {
            allowed.clear();
        }
        allowed.insert(key);
        Ok(())
    }

    pub(crate) fn validate_shared(
        &self,
        validator: &dyn ProjectionRedactionValidator,
        envelope: &Arc<ProductProjectionEnvelope>,
    ) -> Result<(), ProjectionStreamError> {
        let key = validation_live_cache_key(envelope);
        {
            let mut live_allowed = self
                .live_allowed
                .lock()
                .map_err(|_| ProjectionStreamError::Source)?;
            if let Some(cached) = live_allowed.get(&key) {
                if let Some(cached) = cached.upgrade()
                    && Arc::ptr_eq(&cached, envelope)
                {
                    return Ok(());
                }
                live_allowed.remove(&key);
            }
        }

        self.validate(validator, envelope.as_ref())?;

        let mut live_allowed = self
            .live_allowed
            .lock()
            .map_err(|_| ProjectionStreamError::Source)?;
        if live_allowed.len() >= MAX_VALIDATION_CACHE_ENTRIES {
            live_allowed.retain(|_, cached| cached.strong_count() > 0);
            if live_allowed.len() >= MAX_VALIDATION_CACHE_ENTRIES {
                live_allowed.clear();
            }
        }
        live_allowed.insert(key, Arc::downgrade(envelope));
        Ok(())
    }
}

fn validation_cache_key(
    envelope: &ProductProjectionEnvelope,
) -> Result<ProjectionValidationCacheKey, ProjectionStreamError> {
    let payload = serde_json::to_vec(envelope).map_err(|_| ProjectionStreamError::Source)?;
    let payload_len = payload.len();
    let payload_digest = sha256_digest_token(&payload);
    Ok(ProjectionValidationCacheKey {
        variant: envelope_kind(envelope),
        scope: projection_scope_key(envelope.scope()),
        cursor: envelope.cursor().runtime.as_u64(),
        payload_len,
        payload_digest,
    })
}

fn validation_live_cache_key(
    envelope: &Arc<ProductProjectionEnvelope>,
) -> ProjectionValidationLiveCacheKey {
    ProjectionValidationLiveCacheKey {
        variant: envelope_kind(envelope.as_ref()),
        scope: projection_scope_key(envelope.scope()),
        cursor: envelope.cursor().runtime.as_u64(),
        pointer: Arc::as_ptr(envelope) as usize,
    }
}

fn envelope_kind(envelope: &ProductProjectionEnvelope) -> ProjectionEnvelopeKind {
    match envelope {
        ProductProjectionEnvelope::ThreadSnapshot(_) => ProjectionEnvelopeKind::ThreadSnapshot,
        ProductProjectionEnvelope::ThreadUpdates(_) => ProjectionEnvelopeKind::ThreadUpdates,
        ProductProjectionEnvelope::DeliveryStatus(_) => ProjectionEnvelopeKind::DeliveryStatus,
        ProductProjectionEnvelope::Debug(_) => ProjectionEnvelopeKind::Debug,
    }
}

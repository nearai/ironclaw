//! Host-owned FirstParty capability handler registry.
//!
//! First-party handlers are registered by host composition, not by extension
//! manifests. They receive already-authorized, scoped dispatch input from the
//! Reborn runtime adapter path and return normalized JSON output plus resource
//! usage. Authority decisions remain in `CapabilityHost`/authorization and the
//! runtime-policy/planning layers.

use std::{collections::HashMap, fmt, sync::Arc};

use async_trait::async_trait;
use ironclaw_host_api::{
    CapabilityId, MountView, ResourceEstimate, ResourceScope, ResourceUsage,
    RuntimeDispatchErrorKind,
};
use serde_json::Value;

use crate::InvocationServices;

/// Already-authorized first-party capability dispatch input.
///
/// This is host-composed first-party surface, so the struct is `non_exhaustive`:
/// external crates may inspect fields in custom handlers but must not construct
/// it with a struct literal.
#[derive(Clone)]
#[non_exhaustive]
pub struct FirstPartyCapabilityRequest {
    pub capability_id: CapabilityId,
    pub scope: ResourceScope,
    pub estimate: ResourceEstimate,
    pub mounts: Option<MountView>,
    pub services: InvocationServices,
    pub input: Value,
}

impl fmt::Debug for FirstPartyCapabilityRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FirstPartyCapabilityRequest")
            .field("capability_id", &self.capability_id)
            .field("scope", &self.scope)
            .field("estimate", &self.estimate)
            .field("mounts", &self.mounts)
            .field("services", &self.services)
            .field("input", &self.input)
            .finish()
    }
}

impl PartialEq for FirstPartyCapabilityRequest {
    fn eq(&self, other: &Self) -> bool {
        self.capability_id == other.capability_id
            && self.scope == other.scope
            && self.estimate == other.estimate
            && self.mounts == other.mounts
            && self.input == other.input
    }
}

/// Normalized first-party capability output before resource reconciliation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FirstPartyCapabilityResult {
    pub output: Value,
    pub usage: ResourceUsage,
}

impl FirstPartyCapabilityResult {
    pub fn new(output: Value, usage: ResourceUsage) -> Self {
        Self { output, usage }
    }
}

/// Stable redacted first-party handler failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("first-party capability dispatch failed: {kind}")]
pub struct FirstPartyCapabilityError {
    kind: RuntimeDispatchErrorKind,
    usage: Option<ResourceUsage>,
}

impl FirstPartyCapabilityError {
    pub fn new(kind: RuntimeDispatchErrorKind) -> Self {
        Self { kind, usage: None }
    }

    pub fn with_usage(mut self, usage: ResourceUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    pub fn kind(&self) -> RuntimeDispatchErrorKind {
        self.kind
    }

    pub fn usage(&self) -> Option<&ResourceUsage> {
        self.usage.as_ref()
    }
}

/// Host-owned first-party capability implementation.
#[async_trait]
pub trait FirstPartyCapabilityHandler: Send + Sync {
    async fn dispatch(
        &self,
        request: FirstPartyCapabilityRequest,
    ) -> Result<FirstPartyCapabilityResult, FirstPartyCapabilityError>;
}

/// Host-owned registry keyed by declared [`CapabilityId`].
#[derive(Clone, Default)]
pub struct FirstPartyCapabilityRegistry {
    handlers: HashMap<CapabilityId, Arc<dyn FirstPartyCapabilityHandler>>,
}

impl FirstPartyCapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_handler<T>(mut self, capability_id: CapabilityId, handler: Arc<T>) -> Self
    where
        T: FirstPartyCapabilityHandler + 'static,
    {
        self.insert_handler(capability_id, handler);
        self
    }

    pub fn insert_handler<T>(&mut self, capability_id: CapabilityId, handler: Arc<T>)
    where
        T: FirstPartyCapabilityHandler + 'static,
    {
        let handler: Arc<dyn FirstPartyCapabilityHandler> = handler;
        self.handlers.insert(capability_id, handler);
    }

    pub fn get(
        &self,
        capability_id: &CapabilityId,
    ) -> Option<Arc<dyn FirstPartyCapabilityHandler>> {
        self.handlers.get(capability_id).cloned()
    }

    pub fn contains_handler(&self, capability_id: &CapabilityId) -> bool {
        self.handlers.contains_key(capability_id)
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

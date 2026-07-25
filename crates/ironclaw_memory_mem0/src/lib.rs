//! mem0-backed memory provider for IronClaw Reborn.
//!
//! This crate is a second implementation of the provider-neutral
//! [`ironclaw_memory::MemoryService`] contract (the first being
//! `ironclaw_memory_native`). It proves the Reborn memory layer (issue #3537 /
//! #5264) is genuinely swappable: it slots in behind the same
//! `memory.document_store.v1` capability-profile binding the native provider
//! uses, resolved by `ironclaw_host_runtime`'s `MemoryServiceResolver` and
//! constructed by the composition-layer provider factory.
//!
//! ## Shape
//!
//! - [`Mem0Transport`] is the HTTP seam. The provider logic owns no HTTP client
//!   directly; it speaks to mem0 through this trait. The production
//!   implementation [`Mem0HttpTransport`] is a real `reqwest` client (built the
//!   same way the embedding providers build theirs), guarded by a
//!   `check_base_url`-style SSRF check, with a bounded request timeout and
//!   redirects disabled, and *optionally* authenticated with an
//!   `Authorization: Token <key>` header — the key is omitted for a self-hosted
//!   mem0 OSS server running with `AUTH_DISABLED=true`. Tests substitute a mock.
//!   This keeps the `MemoryService` mapping unit-testable without network and
//!   keeps the crate inside its narrow internal-dependency boundary
//!   (`ironclaw_memory` + `ironclaw_host_api` only).
//! - [`Mem0MemoryService`] maps the IronClaw memory operations onto mem0's
//!   `add` / `search` / `list` REST endpoints. See its docs for the per-op
//!   mapping-fidelity table; non-clean mappings are marked `MAPPING GAP` in
//!   source.
//!
//! ## mem0 REST surface targeted
//!
//! This adapter targets the **self-hosted mem0 OSS** REST surface — `POST
//! /memories` (add), `POST /search` (semantic search), and `GET /memories?user_id=…`
//! (list) — which keys memories by a top-level `user_id`. This is the surface a
//! `mem0.Memory` engine exposes behind a thin FastAPI (mem0's own `server/`
//! shape), run fully locally against a self-hosted embedder/LLM (e.g. Ollama) and
//! vector store (e.g. Qdrant). There is no default base URL: mem0 stays off
//! unless a deployment both binds it AND supplies a base URL (via config or the
//! `MEMORY_MEM0_BASE_URL` env override); a bound-but-unset mem0 fails closed. No
//! API key is required for a server running with `AUTH_DISABLED=true`. Every add
//! sets `infer=false` so content is stored verbatim (document-store semantics),
//! needing only the embedder. The hosted cloud's `/v1/memories/…` paths and
//! bearer key are *not* used. The endpoint
//! paths and request shaping are isolated to a handful of constants and helpers
//! in [`service`], so retargeting another surface is a localized change. The
//! tolerant response parsing accepts a bare array or the `results` / `memories` /
//! `data` envelopes.
//!
//! ## Boundary
//!
//! Provider-neutral-contract-conformant: this crate depends only on the contract
//! crate and the host-api id/scope substrate among internal IronClaw crates. It
//! never reaches into host composition, dispatch, filesystem, or runtime.

mod config;
mod error;
mod service;
mod transport;
mod url_check;

/// Reserved extension id under which a deployment binds the mem0 provider to the
/// `memory.document_store.v1` profile (third-party; in production-shaped
/// deployments this binding requires an admin override). Valid against the
/// host-api `ExtensionId` grammar (lowercase, dot-segmented). `local` because
/// this provider targets a self-hosted mem0 OSS server, not the hosted cloud.
pub const MEM0_MEMORY_EXTENSION_ID: &str = "mem0.local.memory";

pub use config::Mem0Config;
pub use error::Mem0Error;
pub use service::Mem0MemoryService;
pub use transport::{
    Mem0HttpMethod, Mem0HttpRequest, Mem0HttpResponse, Mem0HttpTransport, Mem0Transport,
    Mem0TransportError,
};

#[cfg(any(test, feature = "test-support"))]
pub use transport::{Mem0MockHandler, MockMem0Transport};

// Re-export the contract surface so downstream consumers and the provider-swap
// integration test can drive the provider through this crate alone, mirroring
// `ironclaw_memory_native`'s re-export convenience.
pub use ironclaw_memory::{
    MemoryInvocation, MemoryProfileSetStatus, MemoryService, MemoryServiceContextRequest,
    MemoryServiceContextSnippet, MemoryServiceError, MemoryServiceErrorKind,
    MemoryServiceProfileReadResponse, MemoryServiceProfileSetRequest,
    MemoryServiceProfileSetResponse, MemoryServiceReadRequest, MemoryServiceReadResponse,
    MemoryServiceSearchRequest, MemoryServiceSearchResponse, MemoryServiceSearchResult,
    MemoryServiceTreeRequest, MemoryServiceTreeResponse, MemoryServiceWriteRequest,
    MemoryServiceWriteResponse, MemoryWriteStatus,
};

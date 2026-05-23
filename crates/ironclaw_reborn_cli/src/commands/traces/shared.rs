//! Shared helpers for the `traces` CLI surface.
//!
//! Today these helpers live alongside the dispatcher in `super` (mod.rs) and
//! are re-exported here for forward compatibility with finer-grained splits.
//! Audience modules currently use `super::*` so they pick the helpers up via
//! the parent module; this file documents the shared surface explicitly.

#![allow(unused_imports)]

pub(super) use super::{
    TraceCommonsApiResponse, compact_response_body, join_url_paths, normalize_url_path,
    resolve_runtime_owner_scope, trace_commons_api_request, trace_commons_api_url,
    trace_commons_endpoint_prefix,
};

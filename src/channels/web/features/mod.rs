//! Feature slices for the web gateway.
//!
//! Each submodule under `features/` owns a vertical slice of
//! browser-facing behavior end-to-end: request/response types (shared
//! ones still live in [`super::types`] for now), handler functions, and
//! any slice-local helpers. Feature modules depend on `super::platform`
//! for shared state and extractors; they must not depend on one another.
//!
//! The older `handlers/` folder is a transitional fallback. Handlers
//! will migrate into `features/<slice>/` incrementally — see
//! `src/channels/web/CLAUDE.md` for the staged plan tracked in
//! ironclaw#2599.

pub(crate) mod chat;
pub(crate) mod extensions;
pub(crate) mod jobs;
pub(crate) mod logs;
pub(crate) mod oauth;
pub(crate) mod pairing;
pub(crate) mod routines;
pub(crate) mod settings;
pub(crate) mod status;

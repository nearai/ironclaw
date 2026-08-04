//! Domain-free cross-cutting primitives with persisted-compatibility
//! guarantees, shared across the IronClaw workspace.
//!
//! Target-architecture contract: PROPOSAL §6.1.5. This crate owns `identity`,
//! `pkce`, `hashing`, `paths`, `timezone`, `util`, `env_helpers`, and the
//! `attachment` pair — and nothing that carries a domain. It has, and must
//! keep, **no internal dependency**: every consumer sits above it, so a domain
//! type here would invert the graph for the whole workspace.
//!
//! Three LLM-data modules survive the WS1.6 narrowing (`llm_costs`,
//! `model_selection`, `provider_transcript`). Each is blocked from §6.1.5's
//! assigned home by a *pinned rule*, not by preference — the measurements are
//! in `AGENTS.md`. Do not add a fourth, and do not treat their presence as
//! precedent for new domain data.
#![warn(unreachable_pub)]

mod attachment;
pub mod attachment_format;
pub mod env_helpers;
pub mod hashing;
mod identity;
pub mod llm_costs;
pub mod model_selection;
pub mod paths;
pub mod pkce;
pub mod provider_transcript;
mod timezone;
mod util;

pub use attachment::{AttachmentKind, AttachmentRef, IncomingAttachment, normalize_mime_type};
// `attachment_format` is also a `pub mod`, but the registry query functions are
// re-exported at the crate root because the whole attachment pipeline consumes
// them as `ironclaw_common::is_supported_mime` / `kind_for_mime` / etc.
pub use attachment_format::{
    AttachmentFormat, ExtractorId, accept_attribute, accept_tokens, all_formats,
    canonical_extension, extractor_for_mime, is_supported_mime, kind_for_mime, lookup,
    mime_for_extension,
};
pub use identity::{
    CredentialName, ExtensionName, ExternalThreadId, ExternalThreadIdError,
    MAX_MCP_SERVER_NAME_LEN, MAX_NAME_LEN, McpServerName,
};
pub use paths::ironclaw_base_dir;
pub use timezone::{ValidTimezone, deserialize_option_lenient};
pub use util::{truncate_for_preview, truncate_preview};

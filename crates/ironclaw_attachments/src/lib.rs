//! Channel-agnostic attachment landing for IronClaw Reborn.
//!
//! Inbound attachments must not be transient turn context: their bytes are
//! written into the agent-accessible project filesystem so an agent can
//! `file_read` / `list_dir` them in this turn or any later turn of the same
//! project. This crate owns the single landing routine every channel converges
//! on, so there is no per-channel persistence path to drift.
//!
//! The bytes are written **through the project-scoped [`ScopedFilesystem`]
//! authority** — the same authority the agent's file tools resolve through — so
//! the writer and the reader share one `MountView`. That is the whole point:
//! resolving through the authority (rather than a self-computed host path)
//! guarantees the attachment is reachable at the same virtual path the agent
//! reads from, and that a write requires a [`MountPermissions`] write grant
//! (a read-only mount fails closed).
//!
//! [`ScopedFilesystem`]: ironclaw_filesystem::ScopedFilesystem
//! [`MountPermissions`]: ironclaw_host_api::MountPermissions

mod inbound;
mod landing;

pub use inbound::{InboundAttachment, land_inbound_attachments};
pub use landing::{
    ATTACHMENTS_DIR, AttachmentLanding, AttachmentLandingError, DEFAULT_MAX_ATTACHMENT_BYTES,
    attachment_scoped_path, land_attachment,
};

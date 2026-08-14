//! Scoped project-filesystem adapters over the `RootFilesystem` mount plane.
//!
//! These implement this crate's own `ProjectFilesystemReader` port and
//! `ironclaw_attachments`' `InboundAttachmentReader`: alias confinement,
//! sensitive-filename omission, the two-stage size guard, extension→MIME
//! derivation, and the substrate→port error sanitization table. That is
//! contract policy owned by the port, not deployment shape — composition still
//! chooses the backend and hands these a `ScopedFilesystem`.
//!
//! The write half (`ProjectScopedAttachmentLander`) moved to
//! `ironclaw_attachments` with the WS5 widening; the reader stays because it
//! also implements `ironclaw_loop_host`'s `LoopAttachmentReadPort`.

pub mod attachment_reader;
pub mod project_filesystem_reader;

pub use attachment_reader::ProjectScopedAttachmentReader;
pub use project_filesystem_reader::{
    ProjectScopedFilesystemReader, file_name_of, guard_readable_file, map_filesystem_error,
    map_kind, mime_for_path,
};

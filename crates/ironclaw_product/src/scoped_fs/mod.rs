//! Scoped project-filesystem adapters over the `RootFilesystem` mount plane.
//!
//! These implement this crate's own `ProjectFilesystemReader` /
//! `InboundAttachmentLander` ports: alias confinement, sensitive-filename
//! omission, the two-stage size guard, extension→MIME derivation, and the
//! substrate→port error sanitization table. That is contract policy owned by
//! the port, not deployment shape — composition still chooses the backend and
//! hands these a `ScopedFilesystem`.

pub mod attachment_landing;
pub mod project_filesystem_reader;

pub use attachment_landing::{ProjectScopedAttachmentLander, ProjectScopedAttachmentReader};
pub use project_filesystem_reader::{
    ProjectScopedFilesystemReader, file_name_of, guard_readable_file, map_filesystem_error,
    map_kind, mime_for_path,
};
